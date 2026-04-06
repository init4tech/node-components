use crate::{RpcBlock, RpcChainSegment, RpcHostError, latest::Latest};
use alloy::{
    consensus::{BlockHeader, transaction::Recovered},
    eips::{BlockId, BlockNumberOrTag},
    network::BlockResponse,
    primitives::{B256, Sealed},
    providers::Provider,
    pubsub::SubscriptionStream,
    rpc::types::Header as RpcHeader,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier, RevertRange};
use signet_types::primitives::{RecoveredBlock, SealedBlock, TransactionSigned};
use std::{collections::VecDeque, sync::Arc, time::Instant};
use tracing::{debug, info, warn};

/// Default seconds per slot (Ethereum mainnet).
pub(crate) const DEFAULT_SLOT_SECONDS: u64 = 12;
/// Slots per Ethereum epoch.
const SLOTS_PER_EPOCH: u64 = 32;

/// Result of walking the chain backward to find overlap with local state.
enum WalkResult {
    /// The hint hash is already our tip — nothing new.
    AlreadySeen,
    /// Normal advance — new blocks extend our tip.
    Advance {
        /// `(number, hash)` pairs for new blocks, ascending by number.
        new_chain: Vec<(u64, B256)>,
    },
    /// Reorg detected — fork point is below our tip.
    Reorg {
        /// First divergent block number.
        fork_number: u64,
        /// Our old tip number (for the revert range).
        old_tip: u64,
        /// `(number, hash)` pairs for the new chain from fork_number, ascending.
        new_chain: Vec<(u64, B256)>,
    },
    /// Walk exhausted the buffer without finding overlap.
    /// Could be a deep reorg or a stale buffer after extended disconnection.
    /// The notifier should clear the buffer and re-enter backfill.
    Exhausted,
}

/// RPC-based implementation of [`HostNotifier`].
///
/// Follows a host chain via WebSocket `newHeads` subscription. On each
/// event, walks the chain backward by hash to find overlap with a local
/// ring buffer, then fetches full blocks for the new segment only.
///
/// This design is TOCTOU-free: every block fetch is anchored to a specific
/// hash, so reorgs between RPC calls cannot produce inconsistent data.
pub struct RpcHostNotifier<P> {
    /// The alloy provider.
    provider: P,

    /// Subscription stream of new block headers (used as wake-up signal).
    /// Wrapped in [`Latest`] to coalesce stale buffered headers.
    header_sub: Latest<SubscriptionStream<RpcHeader>>,

    /// Local chain view — lightweight ring buffer of (number, hash).
    chain_view: VecDeque<(u64, B256)>,

    /// Maximum entries in the chain view.
    buffer_capacity: usize,

    /// Cached safe block number, refreshed at epoch boundaries.
    cached_safe: Option<u64>,

    /// Cached finalized block number, refreshed at epoch boundaries.
    cached_finalized: Option<u64>,

    /// Last epoch number for which safe/finalized were fetched.
    last_tag_epoch: Option<u64>,

    /// If set, backfill from this block number before processing
    /// subscription events.
    backfill_from: Option<u64>,

    /// Max blocks per backfill batch.
    backfill_batch_size: u64,

    /// Maximum number of concurrent RPC block fetches.
    max_rpc_concurrency: usize,

    /// The last block emitted in a notification, tracked as (number, hash).
    /// Used as the authoritative delivery position for exhaustion recovery.
    last_emitted: Option<(u64, B256)>,

    /// Seconds per slot, used for epoch calculation.
    slot_seconds: u64,

    /// Genesis timestamp, used for epoch calculation.
    genesis_timestamp: u64,
}

impl<P> core::fmt::Debug for RpcHostNotifier<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcHostNotifier")
            .field("chain_view_len", &self.chain_view.len())
            .field("buffer_capacity", &self.buffer_capacity)
            .field("max_rpc_concurrency", &self.max_rpc_concurrency)
            .field("backfill_from", &self.backfill_from)
            .field("last_emitted", &self.last_emitted)
            .finish_non_exhaustive()
    }
}

impl<P> RpcHostNotifier<P>
where
    P: Provider + Clone,
{
    /// Create a new notifier. Used by the builder.
    pub(crate) fn new(
        provider: P,
        header_sub: SubscriptionStream<RpcHeader>,
        buffer_capacity: usize,
        backfill_batch_size: u64,
        max_rpc_concurrency: usize,
        slot_seconds: u64,
        genesis_timestamp: u64,
    ) -> Self {
        Self {
            provider,
            header_sub: Latest::new(header_sub),
            chain_view: VecDeque::with_capacity(buffer_capacity),
            buffer_capacity,
            cached_safe: None,
            cached_finalized: None,
            last_tag_epoch: None,
            backfill_from: None,
            backfill_batch_size,
            max_rpc_concurrency,
            last_emitted: None,
            slot_seconds,
            genesis_timestamp,
        }
    }

    // ── Chain view helpers ──────────────────────────────────────────

    /// Current tip from the chain view.
    fn tip(&self) -> Option<(u64, B256)> {
        self.chain_view.back().copied()
    }

    /// Look up a hash in the chain view by block number (O(1)).
    fn view_hash(&self, number: u64) -> Option<B256> {
        let &(front_number, _) = self.chain_view.front()?;
        let index = number.checked_sub(front_number)? as usize;
        let &(found_number, hash) = self.chain_view.get(index)?;
        debug_assert_eq!(
            found_number, number,
            "chain_view contiguity invariant violated: expected block {number} at index {index}, \
             found {found_number}"
        );
        Some(hash)
    }

    /// Append entries to the chain view, evicting oldest if over capacity.
    fn extend_view(&mut self, entries: &[(u64, B256)]) {
        for &entry in entries {
            self.chain_view.push_back(entry);
            if self.chain_view.len() > self.buffer_capacity {
                self.chain_view.pop_front();
            }
        }
    }

    /// Truncate the chain view at `fork_number` (remove entries >= fork_number),
    /// then append new entries.
    fn reorg_view(&mut self, fork_number: u64, new_entries: &[(u64, B256)]) {
        while self.chain_view.back().is_some_and(|(n, _)| *n >= fork_number) {
            self.chain_view.pop_back();
        }
        self.extend_view(new_entries);
    }

    // ── Block fetching ─────────────────────────────────────────────

    /// Convert an RPC block response and its receipts into an [`RpcBlock`].
    fn convert_rpc_block(
        rpc_block: alloy::rpc::types::Block,
        rpc_receipts: Option<Vec<alloy::rpc::types::TransactionReceipt>>,
    ) -> RpcBlock {
        let hash = rpc_block.header.hash;
        let block = rpc_block
            .map_transactions(|tx| {
                let recovered = tx.inner;
                let signer = recovered.signer();
                let tx: TransactionSigned = recovered.into_inner().into();
                Recovered::new_unchecked(tx, signer)
            })
            .into_consensus();
        let sealed_header = Sealed::new_unchecked(block.header, hash);
        let block: RecoveredBlock = SealedBlock::new(sealed_header, block.body.transactions);
        let receipts = rpc_receipts
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.inner.into_primitives_receipt())
            .collect();
        RpcBlock { block, receipts }
    }

    /// Fetch a single block with receipts, anchored by hash.
    ///
    /// The block and receipt fetches are concurrent via [`tokio::try_join!`].
    #[tracing::instrument(level = "debug", skip_all, fields(%hash))]
    async fn fetch_block_by_hash(&self, hash: B256) -> Result<RpcBlock, RpcHostError> {
        let start = Instant::now();
        let (rpc_block, rpc_receipts) = tokio::try_join!(
            async { Ok::<_, RpcHostError>(self.provider.get_block_by_hash(hash).full().await?) },
            async {
                Ok::<_, RpcHostError>(
                    self.provider.get_block_receipts(BlockId::Hash(hash.into())).await?,
                )
            },
        )?;

        let rpc_block = rpc_block.ok_or(RpcHostError::MissingBlockByHash(hash))?;
        crate::metrics::record_fetch_block_duration(start.elapsed());
        Ok(Self::convert_rpc_block(rpc_block, rpc_receipts))
    }

    /// Fetch full blocks+receipts for a list of hashes, concurrently.
    ///
    /// Hashes must be in ascending block-number order. Results preserve
    /// that order. Concurrency is bounded by [`Self::max_rpc_concurrency`].
    #[tracing::instrument(level = "debug", skip_all, fields(count = hashes.len()))]
    async fn fetch_blocks_by_hash(
        &self,
        hashes: &[(u64, B256)],
    ) -> Result<Vec<Arc<RpcBlock>>, RpcHostError> {
        stream::iter(hashes.iter().copied().map(|(_, hash)| self.fetch_block_by_hash(hash)))
            .buffered(self.max_rpc_concurrency)
            .map_ok(Arc::new)
            .try_collect()
            .await
    }

    /// Fetch a single block with receipts by number (used for backfill only).
    ///
    /// The block and receipt fetches are concurrent.
    #[tracing::instrument(level = "debug", skip_all, fields(number))]
    async fn fetch_block_by_number(&self, number: u64) -> Result<RpcBlock, RpcHostError> {
        let start = Instant::now();
        let tag = BlockNumberOrTag::Number(number);
        let (rpc_block, rpc_receipts) = tokio::try_join!(
            async { Ok::<_, RpcHostError>(self.provider.get_block_by_number(tag).full().await?) },
            async { Ok::<_, RpcHostError>(self.provider.get_block_receipts(tag.into()).await?) },
        )?;

        let rpc_block = rpc_block.ok_or(RpcHostError::MissingBlock(tag))?;
        crate::metrics::record_fetch_block_duration(start.elapsed());
        Ok(Self::convert_rpc_block(rpc_block, rpc_receipts))
    }

    /// Fetch a range of blocks by number concurrently (used for backfill only).
    ///
    /// Returns an empty `Vec` if `from > to`. Concurrency is bounded by
    /// [`Self::max_rpc_concurrency`].
    #[tracing::instrument(level = "debug", skip_all, fields(from, to))]
    async fn fetch_range(&self, from: u64, to: u64) -> Result<Vec<Arc<RpcBlock>>, RpcHostError> {
        if from > to {
            return Ok(Vec::new());
        }

        stream::iter((from..=to).map(|number| self.fetch_block_by_number(number)))
            .buffered(self.max_rpc_concurrency)
            .map_ok(Arc::new)
            .try_collect()
            .await
    }

    // ── Epoch / tag helpers ────────────────────────────────────────

    /// Derive the epoch number from a block timestamp.
    const fn epoch_of(&self, timestamp: u64) -> u64 {
        timestamp.saturating_sub(self.genesis_timestamp) / (self.slot_seconds * SLOTS_PER_EPOCH)
    }

    /// Refresh safe/finalized block numbers if an epoch boundary was crossed.
    #[tracing::instrument(level = "debug", skip_all, fields(epoch = tracing::field::Empty))]
    async fn maybe_refresh_tags(&mut self, timestamp: u64) -> Result<(), RpcHostError> {
        let epoch = self.epoch_of(timestamp);
        tracing::Span::current().record("epoch", epoch);
        if self.last_tag_epoch == Some(epoch) {
            return Ok(());
        }

        let (safe, finalized) = tokio::try_join!(
            self.provider.get_block_by_number(BlockNumberOrTag::Safe),
            self.provider.get_block_by_number(BlockNumberOrTag::Finalized),
        )?;
        let safe = safe.map(|b| b.header().number());
        let finalized = finalized.map(|b| b.header().number());

        self.cached_safe = safe;
        self.cached_finalized = finalized;
        self.last_tag_epoch = Some(epoch);

        crate::metrics::inc_tag_refreshes();
        debug!(epoch, safe, finalized, "refreshed block tags at epoch boundary");
        Ok(())
    }

    // ── Walk algorithm ─────────────────────────────────────────────

    /// Walk the chain backward from `start_hash` to find overlap with
    /// our local chain view.
    ///
    /// Returns `Ok(WalkResult)` describing advance, reorg, or already-seen.
    #[tracing::instrument(level = "debug", skip_all, fields(%start_hash))]
    async fn walk_chain(&self, start_hash: B256) -> Result<WalkResult, RpcHostError> {
        let start = Instant::now();

        // Quick check: is the hint hash already our tip?
        if self.tip().is_some_and(|(_, h)| h == start_hash) {
            crate::metrics::record_walk_duration(start.elapsed());
            return Ok(WalkResult::AlreadySeen);
        }

        let mut current_hash = start_hash;
        let mut walked: Vec<(u64, B256)> = Vec::new();

        loop {
            // Fetch header by hash (no .full() — lightweight).
            let block = self
                .provider
                .get_block_by_hash(current_hash)
                .await?
                .ok_or(RpcHostError::MissingBlockByHash(current_hash))?;

            let number = block.header().number();
            walked.push((number, current_hash));

            // Check against our chain view.
            match self.view_hash(number) {
                Some(local_hash) if local_hash == current_hash => {
                    // This block is in our view — it's the common ancestor.
                    // Remove it from walked (it's not new).
                    walked.pop();

                    if walked.is_empty() {
                        crate::metrics::record_walk_duration(start.elapsed());
                        return Ok(WalkResult::AlreadySeen);
                    }

                    // Reverse so blocks are ascending by number.
                    walked.reverse();

                    let tip_num = self.tip().map(|(n, _)| n).unwrap_or(0);
                    if number == tip_num {
                        crate::metrics::record_walk_duration(start.elapsed());
                        return Ok(WalkResult::Advance { new_chain: walked });
                    }

                    crate::metrics::record_walk_duration(start.elapsed());
                    return Ok(WalkResult::Reorg {
                        fork_number: number + 1,
                        old_tip: tip_num,
                        new_chain: walked,
                    });
                }
                Some(_) => {
                    // Number exists in view but hash differs — keep walking.
                }
                None if self.chain_view.is_empty() => {
                    // Buffer is empty — no backfill was performed, or this is
                    // the very first event. Treat all walked blocks as new.
                    // The buffer fills incrementally over subsequent events.
                    // (When backfill WAS performed, the buffer is pre-populated
                    // and this arm does not fire — overlap is found normally.)
                    walked.reverse();
                    crate::metrics::record_walk_duration(start.elapsed());
                    return Ok(WalkResult::Advance { new_chain: walked });
                }
                None => {
                    // Below our buffer — check if we're at the front.
                    if self.chain_view.front().is_some_and(|(front_num, _)| number < *front_num) {
                        crate::metrics::inc_walk_exhausted();
                        crate::metrics::record_walk_duration(start.elapsed());
                        return Ok(WalkResult::Exhausted);
                    }
                    // Number is within our range but not in buffer (gap).
                    // Keep walking.
                }
            }

            // Depth limit.
            if walked.len() > self.buffer_capacity {
                crate::metrics::inc_walk_exhausted();
                crate::metrics::record_walk_duration(start.elapsed());
                return Ok(WalkResult::Exhausted);
            }

            // Walk to parent.
            current_hash = block.header().parent_hash();
        }
    }

    // ── Notification handling ──────────────────────────────────────

    /// Handle a new subscription header.
    ///
    /// Uses the header hash as a hint to walk the chain. Returns `Ok(None)`
    /// if the header was already processed (no-op walk).
    #[tracing::instrument(level = "info", skip_all, fields(hint_number = header.number(), hint_hash = %header.hash))]
    async fn handle_new_head(
        &mut self,
        header: RpcHeader,
    ) -> Result<Option<HostNotification<RpcChainSegment>>, RpcHostError> {
        let start = Instant::now();
        let timestamp = header.timestamp();
        let hint_hash = header.hash;

        let walk = match self.walk_chain(hint_hash).await {
            Ok(walk) => walk,
            Err(RpcHostError::MissingBlockByHash(_)) => {
                debug!("subscription hint stale, falling back to latest");
                crate::metrics::inc_stale_hints();
                let latest = self
                    .provider
                    .get_block_by_number(BlockNumberOrTag::Latest)
                    .await?
                    .ok_or(RpcHostError::MissingBlock(BlockNumberOrTag::Latest))?;
                self.walk_chain(latest.header.hash).await.inspect_err(|_| {
                    crate::metrics::inc_rpc_errors();
                })?
            }
            Err(e) => {
                crate::metrics::inc_rpc_errors();
                return Err(e);
            }
        };

        let kind = match walk {
            WalkResult::AlreadySeen => {
                info!("already seen");
                crate::metrics::record_handle_new_head_duration(start.elapsed());
                return Ok(None);
            }
            WalkResult::Exhausted => {
                let resume_from = self.last_emitted.map(|(n, _)| n + 1);
                warn!(
                    buffer_capacity = self.buffer_capacity,
                    ?resume_from,
                    "walk exhausted buffer, resetting to backfill from last emitted"
                );
                self.chain_view.clear();
                self.backfill_from = resume_from;
                self.last_tag_epoch = None;
                crate::metrics::record_handle_new_head_duration(start.elapsed());
                return Ok(None);
            }
            WalkResult::Advance { new_chain } => {
                let count = new_chain.len();
                let blocks = self.fetch_blocks_by_hash(&new_chain).await?;
                self.extend_view(&new_chain);
                self.last_emitted = new_chain.last().copied();
                info!(blocks = count, "chain advanced");
                crate::metrics::inc_blocks_fetched(count as u64, "frontfill");
                HostNotificationKind::ChainCommitted { new: Arc::new(RpcChainSegment::new(blocks)) }
            }
            WalkResult::Reorg { fork_number, old_tip, new_chain } => {
                let count = new_chain.len();
                let depth = old_tip.saturating_sub(fork_number) + 1;
                let blocks = self.fetch_blocks_by_hash(&new_chain).await?;
                self.reorg_view(fork_number, &new_chain);
                self.last_emitted = new_chain.last().copied();
                info!(depth, fork_number, new_blocks = count, "chain reorged");
                crate::metrics::inc_blocks_fetched(count as u64, "frontfill");
                crate::metrics::inc_reorgs(depth);
                HostNotificationKind::ChainReorged {
                    old: RevertRange::new(fork_number, old_tip),
                    new: Arc::new(RpcChainSegment::new(blocks)),
                }
            }
        };

        self.maybe_refresh_tags(timestamp).await?;

        // Update gauges.
        if let Some((tip_num, _)) = self.tip() {
            crate::metrics::set_tip(tip_num);
        }
        crate::metrics::set_chain_view_len(self.chain_view.len());
        crate::metrics::record_handle_new_head_duration(start.elapsed());

        Ok(Some(HostNotification {
            kind,
            safe_block_number: self.cached_safe,
            finalized_block_number: self.cached_finalized,
        }))
    }

    // ── Backfill ───────────────────────────────────────────────────

    /// Process a backfill batch if pending.
    ///
    /// Backfills by number up to `(latest - buffer_capacity / 2)` to leave
    /// half the buffer depth for hash-based frontfill of recent blocks.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            from = tracing::field::Empty,
            to = tracing::field::Empty,
            batch_size = tracing::field::Empty,
        )
    )]
    async fn drain_backfill(
        &mut self,
    ) -> Option<Result<HostNotification<RpcChainSegment>, RpcHostError>> {
        let from = self.backfill_from?;
        let start = Instant::now();

        let span = tracing::Span::current();
        span.record("from", from);
        span.record("batch_size", self.backfill_batch_size);

        let tip = match self.provider.get_block_number().await {
            Ok(n) => n,
            Err(e) => {
                crate::metrics::inc_rpc_errors();
                return Some(Err(e.into()));
            }
        };

        let backfill_ceiling = tip.saturating_sub(self.buffer_capacity as u64 / 2);
        if from > backfill_ceiling {
            self.backfill_from = None;
            info!("backfill complete, switching to frontfill");
            return None;
        }

        let to = backfill_ceiling.min(from + self.backfill_batch_size - 1);
        span.record("to", to);

        let blocks = match self.fetch_range(from, to).await {
            Ok(b) => b,
            Err(e) => {
                crate::metrics::inc_rpc_errors();
                crate::metrics::record_backfill_batch(start.elapsed());
                return Some(Err(e));
            }
        };

        // Verify parent-hash continuity with last emitted block.
        if let Some((_, expected_parent)) = self.last_emitted
            && let Some(first) = blocks.first()
            && first.parent_hash() != expected_parent
        {
            warn!(
                expected = %expected_parent,
                actual = %first.parent_hash(),
                "parent hash mismatch after exhaustion recovery"
            );
            self.chain_view.clear();
            crate::metrics::record_backfill_batch(start.elapsed());
            return Some(Err(RpcHostError::BackfillContinuityBreak));
        }

        let view_entries: Vec<(u64, B256)> =
            blocks.iter().map(|b| (b.number(), b.hash())).collect();
        self.extend_view(&view_entries);

        // Update delivery high-water mark.
        if let Some(&(n, h)) = view_entries.last() {
            self.last_emitted = Some((n, h));
        }

        let backfill_done = to >= backfill_ceiling;
        if backfill_done {
            self.backfill_from = None;
        } else {
            self.backfill_from = Some(to + 1);
        }

        if let Some(last) = blocks.last()
            && let Err(e) = self.maybe_refresh_tags(last.block.timestamp()).await
        {
            crate::metrics::record_backfill_batch(start.elapsed());
            return Some(Err(e));
        }

        // Metrics.
        let count = blocks.len() as u64;
        crate::metrics::inc_blocks_fetched(count, "backfill");
        crate::metrics::record_backfill_batch(start.elapsed());
        if let Some((tip_num, _)) = self.tip() {
            crate::metrics::set_tip(tip_num);
        }
        crate::metrics::set_chain_view_len(self.chain_view.len());

        if backfill_done {
            info!("backfill complete, switching to frontfill");
        } else {
            debug!(blocks_fetched = count, next_from = to + 1, "backfill batch complete");
        }

        let segment = Arc::new(RpcChainSegment::new(blocks));
        Some(Ok(HostNotification {
            kind: HostNotificationKind::ChainCommitted { new: segment },
            safe_block_number: self.cached_safe,
            finalized_block_number: self.cached_finalized,
        }))
    }
}

/// [`HostNotifier`] implementation for [`RpcHostNotifier`].
///
/// Note: this implementation never emits
/// [`HostNotificationKind::ChainReverted`]. The `newHeads` WebSocket
/// subscription only fires when a new block appears — a pure revert
/// (blocks removed without a replacement chain) produces no new header and
/// is therefore invisible to the subscription. Only
/// [`HostNotificationKind::ChainCommitted`] and
/// [`HostNotificationKind::ChainReorged`] are produced.
impl<P> HostNotifier for RpcHostNotifier<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    type Chain = RpcChainSegment;
    type Error = RpcHostError;

    async fn next_notification(
        &mut self,
    ) -> Option<Result<HostNotification<Self::Chain>, Self::Error>> {
        loop {
            // Drain backfill on every iteration — handles both initial
            // backfill and re-entry after buffer exhaustion recovery.
            if let Some(result) = self.drain_backfill().await {
                return Some(result);
            }

            let header = self.header_sub.next().await?;
            match self.handle_new_head(header).await {
                Ok(Some(notification)) => return Some(Ok(notification)),
                Ok(None) => continue,
                Err(e) => return Some(Err(e)),
            }
        }
    }

    fn set_head(&mut self, block_number: u64) {
        self.backfill_from = Some(block_number);
        self.last_emitted = None;
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        self.backfill_batch_size =
            max_blocks.map_or(crate::DEFAULT_BACKFILL_BATCH_SIZE, |m| m.max(1));
    }

    fn send_finished_height(&self, _block_number: u64) -> Result<(), Self::Error> {
        // No-op: no ExEx to notify for an RPC follower.
        Ok(())
    }
}
