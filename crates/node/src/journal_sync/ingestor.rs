use crate::NodeStatus;
use alloy::{
    consensus::BlockHeader,
    primitives::{Sealable, keccak256},
};
use bytes::Bytes;
use eyre::{Context, bail, eyre};
use signet_journal::Journal;
use signet_journal_chain::JournalChainEvent;
use signet_rpc::{ChainNotifier, NewBlockNotification, RemovedBlock, ReorgNotification};
use signet_storage::{DrainedBlock, ExecutedBlockBuilder, HotKv, HotKvRead, UnifiedStorage};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument};
use trevm::{
    journal::JournalDecode,
    revm::database::{BundleState, DBErrorMarker},
};

/// Applies [`JournalChainEvent`]s from a local journal chain to storage on a
/// syncing node.
///
/// The ingestor is the consumer end of the chain's backpressured sender: the
/// chain awaits on the sender after each event, so a slow ingestor throttles
/// ingestion all the way back to the upstream source. Each event is applied
/// synchronously before the next is taken, keeping hot storage's tip in
/// lockstep with the chain's.
///
/// - [`JournalChainEvent::Reorg`] drains storage above the fork point, rewinds
///   the block tags, and broadcasts a reorg notification.
/// - [`JournalChainEvent::Journal`] decodes the journal, applies its state diff
///   via `append_blocks` (cold storage receives the header with empty
///   transaction/receipt/event lists), and broadcasts a new-block notification.
///
/// Only the `latest` block tag is advanced during journal sync (`set_latest` on each applied
/// journal, capped by `rewind_to` on reorg). `safe` and `finalized` stay at genesis until the node
/// hands off to block execution and processes its first committed chain: the journal feed carries
/// no host safe/finalized markers, and since journal sync can itself reorg, advancing `finalized`
/// here could later un-finalize a block. RPC consumers therefore read genesis for `safe` and
/// `finalized` until shortly after that handoff.
#[derive(Debug)]
pub(crate) struct JournalIngestor<H: HotKv> {
    storage: Arc<UnifiedStorage<H>>,
    chain: ChainNotifier,
    status: watch::Sender<NodeStatus>,
    event_rx: mpsc::Receiver<JournalChainEvent>,
    cancellation_token: CancellationToken,
    /// Latest applied rollup height, published after every event so the
    /// run-loop driver can query `host_tip` exactly when progress occurs.
    applied_rollup_height: watch::Sender<u64>,
}

impl<H> JournalIngestor<H>
where
    H: HotKv + Clone + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    /// Create a new ingestor consuming from the chain's backpressured receiver.
    pub(crate) const fn new(
        storage: Arc<UnifiedStorage<H>>,
        chain: ChainNotifier,
        status: watch::Sender<NodeStatus>,
        event_rx: mpsc::Receiver<JournalChainEvent>,
        cancellation_token: CancellationToken,
        applied_rollup_height: watch::Sender<u64>,
    ) -> Self {
        Self { storage, chain, status, event_rx, cancellation_token, applied_rollup_height }
    }

    /// Run the ingestion loop until the cancellation token fires, the chain
    /// drops its sender, or a fatal error occurs. The cancellation check only
    /// runs between events, so an in-flight `apply_journal` always runs to
    /// completion before the loop exits.
    ///
    /// On cancellation, any events the chain has already emitted are applied
    /// before returning. At a journals->blocks handoff this lets hot storage
    /// catch up to the chain's tip, so the chain does not lead storage when the
    /// node starts executing blocks (see [`crate::SignetNode::run_journal_sync`]).
    pub(crate) async fn run(mut self) -> eyre::Result<()> {
        loop {
            tokio::select! {
                biased;
                () = self.cancellation_token.cancelled() => break,
                event = self.event_rx.recv() => match event {
                    Some(event) => self.apply_event(event).await?,
                    None => return Ok(()),
                },
            }
        }
        // Cancelled. Drain events the chain has already emitted so storage reaches the chain's
        // emitted tip. Best-effort: `try_recv` takes what is buffered now. The chain may emit a
        // little more from input it had not yet processed; block execution reconciles that on
        // re-derivation (the chain treats the re-derived journals as duplicates).
        while let Ok(event) = self.event_rx.try_recv() {
            self.apply_event(event).await?;
        }
        Ok(())
    }

    /// Apply a single chain event to storage.
    async fn apply_event(&self, event: JournalChainEvent) -> eyre::Result<()> {
        match event {
            JournalChainEvent::Reorg(height) => self.apply_reorg(height).await,
            JournalChainEvent::Journal { height, data } => self.apply_journal(height, data).await,
        }
    }

    /// Unwind storage and tags in response to a reorg at `height`. All state at
    /// heights `>= height` is removed, so the surviving tip is `height - 1`.
    #[instrument(skip(self))]
    async fn apply_reorg(&self, height: u64) -> eyre::Result<()> {
        let common_ancestor = height.saturating_sub(1);
        let drained = self
            .storage
            .drain_above(common_ancestor)
            .await
            .wrap_err("failed to drain storage during journal-sync reorg")?;

        if drained.is_empty() {
            debug!(
                height,
                common_ancestor,
                "journal-sync reorg drained nothing; storage already at or below fork point"
            );
            return Ok(());
        }

        // Cap tags before broadcasting so RPC consumers never observe a tag
        // pointing above the post-reorg tip.
        self.chain.tags().rewind_to(common_ancestor);
        self.notify_reorg(drained, common_ancestor);
        self.status.send_modify(|status| *status = NodeStatus::AtHeight(common_ancestor));
        self.applied_rollup_height.send_replace(common_ancestor);
        debug!(height, common_ancestor, "applied journal-sync reorg");
        Ok(())
    }

    /// Decode a journal, apply its state diff to storage, and broadcast the new
    /// block. The decoded header is re-sealed (recomputing the canonical block
    /// hash) and the keccak256 of the wire bytes is stamped onto the block so
    /// `append_blocks` records it in the `JournalHashes` table for restart-safe
    /// checkpoint seeding.
    #[instrument(skip(self, data), fields(len = data.len()))]
    async fn apply_journal(&self, height: u64, data: Bytes) -> eyre::Result<()> {
        let journal_hash = keccak256(&data);
        let mut slice = data.as_ref();
        // The decode/version/header-continuity checks below reject bad upstream content; an
        // `append_blocks` database error is instead a local storage fault. All are fatal - see
        // `SignetNode::run_journal_sync` for the failure taxonomy and restart recovery.
        let journal = Journal::decode(&mut slice).map_err(|error| {
            eyre!("invalid upstream journal at height {height}: failed to decode: {error}")
        })?;
        let Journal::V1(host_journal) = journal else {
            bail!("invalid upstream journal at height {height}: unsupported journal version");
        };

        let (meta, bundle_index) = host_journal.into_parts();
        let (_host_height, _prev_journal_hash, header) = meta.into_parts();
        let bundle = BundleState::from(bundle_index);

        let executed = ExecutedBlockBuilder::new()
            .header(header.seal_slow())
            .bundle(bundle)
            .journal_hash(journal_hash)
            .build()
            .wrap_err_with(|| {
                format!("invalid upstream journal at height {height}: failed to build block")
            })?;

        // The journal chain indexes by rollup height (the header number), so a
        // mismatch here means the chain and the decoded payload disagree - a
        // bug in the chain or the metadata extractor, not bad upstream data.
        let block_number = executed.block_number();
        if block_number != height {
            bail!("journal-sync height {height} does not match header height {block_number}");
        }

        // Build the broadcast payload before `append_blocks` consumes the block.
        // Journal sync carries no transactions or receipts, so both lists are
        // empty; `newHeads` subscribers still see the header.
        let notification = NewBlockNotification {
            header: executed.header.inner().clone(),
            transactions: Vec::new(),
            receipts: Vec::new(),
        };

        // A non-contiguous / parent-hash error here means bad upstream content (the journal does
        // not chain onto storage); any other (database) error is a local storage fault.
        self.storage.append_blocks(vec![executed]).await.wrap_err_with(|| {
            format!("failed to append journal-synced block at height {height} to storage")
        })?;

        // Only `latest` advances during journal sync; see the type-level docs on tag handling.
        self.chain.tags().set_latest(height);
        // Best-effort broadcast to RPC push-subscribers; an error just means none are connected.
        let _ = self.chain.send_new_block(notification);
        self.status.send_modify(|status| *status = NodeStatus::AtHeight(height));
        self.applied_rollup_height.send_replace(height);
        info!(height, "applied journal");
        Ok(())
    }

    /// Broadcast a reorg notification built from the drained blocks.
    fn notify_reorg(&self, drained: Vec<DrainedBlock>, common_ancestor: u64) {
        let removed_blocks = drained
            .into_iter()
            .map(|block| {
                let number = block.header.number();
                let hash = block.header.hash();
                let timestamp = block.header.timestamp();
                let logs = block.receipts.into_iter().flat_map(|receipt| receipt.receipt.logs);
                RemovedBlock { number, hash, timestamp, logs: logs.collect() }
            })
            .collect();
        // `send_reorg` records the reorg in the authoritative ring buffer before broadcasting,
        // so an error here just means no RPC push-subscribers are connected.
        let _ = self.chain.send_reorg(ReorgNotification { common_ancestor, removed_blocks });
    }
}
