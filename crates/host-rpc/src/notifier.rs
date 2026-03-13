use crate::{RpcBlock, RpcChainSegment, RpcHostError};
use alloy::{
    consensus::{BlockHeader, transaction::Recovered},
    eips::BlockNumberOrTag,
    network::BlockResponse,
    primitives::{B256, Sealed},
    providers::Provider,
    pubsub::SubscriptionStream,
    rpc::types::Header as RpcHeader,
};
use futures_util::StreamExt;
use signet_extract::Extractable;
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier};
use signet_types::primitives::{RecoveredBlock, SealedBlock, TransactionSigned};
use std::{collections::VecDeque, sync::Arc};
use tracing::debug;

/// Seconds per Ethereum slot.
const SLOT_SECONDS: u64 = 12;
/// Slots per Ethereum epoch.
const SLOTS_PER_EPOCH: u64 = 32;

/// RPC-based implementation of [`HostNotifier`].
///
/// Follows a host chain via WebSocket `newHeads` subscription, fetching full
/// blocks and receipts on demand. Detects reorgs via a ring buffer of recent
/// block hashes.
///
/// Generic over `P`: any alloy provider that supports subscriptions.
pub struct RpcHostNotifier<P> {
    /// The alloy provider.
    pub(crate) provider: P,

    /// Subscription stream of new block headers.
    pub(crate) header_sub: SubscriptionStream<RpcHeader>,

    /// Recent blocks for reorg detection and caching.
    pub(crate) block_buffer: VecDeque<Arc<RpcBlock>>,

    /// Maximum entries in the block buffer.
    pub(crate) buffer_capacity: usize,

    /// Cached safe block number, refreshed at epoch boundaries.
    pub(crate) cached_safe: Option<u64>,

    /// Cached finalized block number, refreshed at epoch boundaries.
    pub(crate) cached_finalized: Option<u64>,

    /// Last epoch number for which safe/finalized were fetched.
    pub(crate) last_tag_epoch: Option<u64>,

    /// If set, backfill from this block number before processing
    /// subscription events.
    pub(crate) backfill_from: Option<u64>,

    /// Max blocks per backfill batch.
    pub(crate) backfill_batch_size: u64,

    /// Genesis timestamp, used for epoch calculation.
    pub(crate) genesis_timestamp: u64,
}

impl<P> core::fmt::Debug for RpcHostNotifier<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcHostNotifier")
            .field("buffer_len", &self.block_buffer.len())
            .field("buffer_capacity", &self.buffer_capacity)
            .field("backfill_from", &self.backfill_from)
            .finish_non_exhaustive()
    }
}

impl<P> RpcHostNotifier<P>
where
    P: Provider + Clone,
{
    /// Current tip block number from the buffer.
    fn tip(&self) -> Option<u64> {
        self.block_buffer.back().map(|b| b.number())
    }

    /// Look up a block hash in the buffer by block number.
    fn buffered_hash(&self, number: u64) -> Option<B256> {
        self.block_buffer.iter().rev().find(|b| b.number() == number).map(|b| b.hash())
    }

    /// Fetch a single block with its receipts from the provider.
    async fn fetch_block(&self, number: u64) -> Result<RpcBlock, RpcHostError> {
        let rpc_block = self
            .provider
            .get_block_by_number(number.into())
            .full()
            .await?
            .ok_or(RpcHostError::MissingBlock(number))?;

        let rpc_receipts =
            self.provider.get_block_receipts(number.into()).await?.unwrap_or_default();

        // Convert RPC block to our RecoveredBlock type.
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

        // Convert RPC receipts to consensus receipts.
        let receipts =
            rpc_receipts.into_iter().map(|r| r.inner.into_primitives_receipt()).collect();

        Ok(RpcBlock { block, receipts })
    }

    /// Fetch a range of blocks concurrently.
    async fn fetch_range(&self, from: u64, to: u64) -> Result<Vec<Arc<RpcBlock>>, RpcHostError> {
        let mut blocks = Vec::with_capacity((to - from + 1) as usize);
        // Fetch sequentially for now; can be parallelized later with
        // futures::stream::FuturesOrdered.
        for number in from..=to {
            let block = self.fetch_block(number).await?;
            blocks.push(Arc::new(block));
        }
        Ok(blocks)
    }

    /// Derive the epoch number from a block timestamp.
    const fn epoch_of(&self, timestamp: u64) -> u64 {
        timestamp.saturating_sub(self.genesis_timestamp) / (SLOT_SECONDS * SLOTS_PER_EPOCH)
    }

    /// Refresh safe/finalized block numbers if an epoch boundary was crossed.
    async fn maybe_refresh_tags(&mut self, timestamp: u64) -> Result<(), RpcHostError> {
        let epoch = self.epoch_of(timestamp);
        if self.last_tag_epoch == Some(epoch) {
            return Ok(());
        }

        let safe = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Safe)
            .await?
            .map(|b| b.header().number());
        let finalized = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Finalized)
            .await?
            .map(|b| b.header().number());

        self.cached_safe = safe;
        self.cached_finalized = finalized;
        self.last_tag_epoch = Some(epoch);

        debug!(epoch, safe, finalized, "refreshed block tags at epoch boundary");
        Ok(())
    }

    /// Remove invalidated entries from the buffer on reorg, then add new
    /// blocks.
    fn update_buffer_reorg(&mut self, fork_number: u64, new_blocks: &[Arc<RpcBlock>]) {
        // Remove all blocks at or above the fork point.
        while self.block_buffer.back().is_some_and(|b| b.number() >= fork_number) {
            self.block_buffer.pop_back();
        }
        self.push_to_buffer(new_blocks);
    }

    /// Push blocks to the buffer, evicting oldest if over capacity.
    fn push_to_buffer(&mut self, blocks: &[Arc<RpcBlock>]) {
        for block in blocks {
            self.block_buffer.push_back(Arc::clone(block));
            if self.block_buffer.len() > self.buffer_capacity {
                self.block_buffer.pop_front();
            }
        }
    }

    /// Find the fork point by walking backward through the buffer.
    ///
    /// Returns the block number where the chain diverged (the first block
    /// that differs), or `None` if the fork is deeper than the buffer.
    ///
    /// **Limitation:** This queries the RPC node for blocks on the new chain.
    /// If the node hasn't fully switched to the new chain, it may return
    /// stale blocks from the old chain, producing an incorrect fork point.
    /// This is an inherent limitation of RPC-based reorg detection.
    async fn find_fork_point(&self, new_header: &RpcHeader) -> Result<Option<u64>, RpcHostError> {
        // Walk the new chain backward from the new header's parent.
        let mut check_hash = new_header.parent_hash();
        let mut check_number = new_header.number().saturating_sub(1);

        loop {
            match self.buffered_hash(check_number) {
                Some(buffered) if buffered == check_hash => {
                    // Found the common ancestor at check_number.
                    // The fork point is the next block (first divergence).
                    return Ok(Some(check_number + 1));
                }
                Some(_) => {
                    // Mismatch — keep walking backward.
                    if check_number == 0 {
                        return Ok(None);
                    }
                    // Fetch the parent of this block on the new chain.
                    let parent = self
                        .provider
                        .get_block_by_number(check_number.into())
                        .await?
                        .ok_or(RpcHostError::MissingBlock(check_number))?;
                    check_hash = parent.header().parent_hash();
                    check_number = check_number.saturating_sub(1);
                }
                None => {
                    // Beyond our buffer — can't determine fork point.
                    return Ok(None);
                }
            }
        }
    }

    /// Process a backfill batch if pending.
    ///
    /// Returns `Some(notification)` if a batch was emitted, `None` if no
    /// backfill is pending.
    async fn drain_backfill(
        &mut self,
    ) -> Option<Result<HostNotification<RpcChainSegment>, RpcHostError>> {
        let from = self.backfill_from?;
        let tip = match self.provider.get_block_number().await {
            Ok(n) => n,
            Err(e) => return Some(Err(e.into())),
        };

        if from > tip {
            self.backfill_from = None;
            return None;
        }

        let to = tip.min(from + self.backfill_batch_size - 1);

        let blocks = match self.fetch_range(from, to).await {
            Ok(b) => b,
            Err(e) => return Some(Err(e)),
        };

        self.push_to_buffer(&blocks);

        // Advance or clear backfill.
        if to >= tip {
            self.backfill_from = None;
        } else {
            self.backfill_from = Some(to + 1);
        }

        // Refresh tags using the last block's timestamp.
        if let Some(last) = blocks.last()
            && let Err(e) = self.maybe_refresh_tags(last.block.timestamp()).await
        {
            return Some(Err(e));
        }

        let segment = Arc::new(RpcChainSegment::new(blocks));
        Some(Ok(HostNotification {
            kind: HostNotificationKind::ChainCommitted { new: segment },
            safe_block_number: self.cached_safe,
            finalized_block_number: self.cached_finalized,
        }))
    }

    /// Handle a new header from the subscription stream.
    async fn handle_new_head(
        &mut self,
        header: RpcHeader,
    ) -> Result<HostNotification<RpcChainSegment>, RpcHostError> {
        let new_number = header.number();
        let new_parent = header.parent_hash();

        // Check parent hash continuity.
        let is_reorg = self.tip().is_some_and(|tip_num| {
            self.buffered_hash(tip_num).is_some_and(|tip_hash| {
                // Parent should point to our tip, and the new block
                // should be exactly one ahead.
                new_parent != tip_hash || new_number != tip_num + 1
            })
        });

        let kind = if is_reorg {
            self.handle_reorg(header).await?
        } else {
            self.handle_advance(header).await?
        };

        // Refresh block tags.
        let timestamp = match &kind {
            HostNotificationKind::ChainCommitted { new } => {
                new.blocks_and_receipts().last().map(|bar| bar.block.timestamp()).unwrap_or(0)
            }
            HostNotificationKind::ChainReorged { new, .. } => {
                new.blocks_and_receipts().last().map(|bar| bar.block.timestamp()).unwrap_or(0)
            }
            HostNotificationKind::ChainReverted { .. } => 0,
        };
        if timestamp > 0 {
            self.maybe_refresh_tags(timestamp).await?;
        }

        Ok(HostNotification {
            kind,
            safe_block_number: self.cached_safe,
            finalized_block_number: self.cached_finalized,
        })
    }

    /// Handle a normal chain advance (no reorg).
    async fn handle_advance(
        &mut self,
        header: RpcHeader,
    ) -> Result<HostNotificationKind<RpcChainSegment>, RpcHostError> {
        let new_number = header.number();
        let from = self.tip().map_or(new_number, |t| t + 1);

        let blocks = self.fetch_range(from, new_number).await?;
        self.push_to_buffer(&blocks);

        Ok(HostNotificationKind::ChainCommitted { new: Arc::new(RpcChainSegment::new(blocks)) })
    }

    /// Handle a reorg: find fork point, emit `ChainReorged`.
    async fn handle_reorg(
        &mut self,
        header: RpcHeader,
    ) -> Result<HostNotificationKind<RpcChainSegment>, RpcHostError> {
        let new_number = header.number();

        let fork_number = self.find_fork_point(&header).await?.ok_or_else(|| {
            let depth = self
                .tip()
                .unwrap_or(0)
                .saturating_sub(self.block_buffer.front().map_or(0, |b| b.number()));
            RpcHostError::ReorgTooDeep { depth, capacity: self.buffer_capacity }
        })?;

        // Collect reverted blocks from the buffer before removing them.
        let old_blocks: Vec<Arc<RpcBlock>> = self
            .block_buffer
            .iter()
            .filter(|b| b.number() >= fork_number)
            .map(Arc::clone)
            .collect();

        // Fetch new chain from fork point to new head.
        let blocks = self.fetch_range(fork_number, new_number).await?;
        self.update_buffer_reorg(fork_number, &blocks);

        Ok(HostNotificationKind::ChainReorged {
            old: Arc::new(RpcChainSegment::new(old_blocks)),
            new: Arc::new(RpcChainSegment::new(blocks)),
        })
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
        // Drain pending backfill first.
        if let Some(result) = self.drain_backfill().await {
            return Some(result);
        }

        // Await next header from subscription.
        let header = self.header_sub.next().await?;
        Some(self.handle_new_head(header).await)
    }

    fn set_head(&mut self, block_number: u64) {
        self.backfill_from = Some(block_number);
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        if let Some(max) = max_blocks {
            self.backfill_batch_size = max.max(1);
        }
    }

    fn send_finished_height(&self, _block_number: u64) -> Result<(), Self::Error> {
        // No-op: no ExEx to notify for an RPC follower.
        Ok(())
    }
}
