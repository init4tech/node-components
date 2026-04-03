use crate::{NodeStatus, metrics};
use alloy::consensus::BlockHeader;
use eyre::{Context, OptionExt};
use signet_blobber::CacheHandle;
use signet_block_processor::{AliasOracleFactory, SignetBlockProcessorV1};
use signet_evm::EthereumHardfork;
use signet_extract::{Extractable, Extractor};
use signet_node_config::SignetNodeConfig;
use signet_node_types::{HostNotification, HostNotifier, RevertRange};
use signet_rpc::{
    ChainNotifier, NewBlockNotification, RemovedBlock, ReorgNotification, RpcServerGuard,
    ServeConfig, StorageRpcConfig,
};
use signet_storage::{DrainedBlock, HistoryRead, HotKv, HotKvRead, UnifiedStorage};
use signet_types::{PairedHeights, constants::SignetSystemConstants};
use std::{fmt, sync::Arc};
use tokio::sync::watch;
use tracing::{debug, info, instrument};
use trevm::revm::database::DBErrorMarker;

/// Signet context and configuration.
pub struct SignetNode<N, H, AliasOracle>
where
    N: HostNotifier,
    H: HotKv,
{
    /// The host notifier, which yields chain notifications.
    pub(crate) notifier: N,

    /// Signet node configuration.
    pub(crate) config: Arc<SignetNodeConfig>,

    /// Unified hot + cold storage backend.
    pub(crate) storage: Arc<UnifiedStorage<H>>,

    /// Shared chain state (block tags + notification sender).
    /// Cloned to the RPC context on startup.
    pub(crate) chain: ChainNotifier,

    /// The join handle for the RPC server. None if the RPC server is not
    /// yet running.
    pub(crate) rpc_handle: Option<RpcServerGuard>,

    /// Chain configuration constants.
    pub(crate) constants: SignetSystemConstants,

    /// Status channel, currently used only for testing.
    pub(crate) status: watch::Sender<NodeStatus>,

    /// An oracle for determining whether addresses should be aliased.
    pub(crate) alias_oracle: Arc<AliasOracle>,

    /// A handle to the blob cacher.
    pub(crate) blob_cacher: CacheHandle,

    /// A reqwest client, used by the blob fetch and the tx cache forwarder.
    pub(crate) client: reqwest::Client,

    /// RPC transport configuration.
    pub(crate) serve_config: ServeConfig,

    /// RPC behaviour configuration.
    pub(crate) rpc_config: StorageRpcConfig,
}

impl<N, H, AliasOracle> fmt::Debug for SignetNode<N, H, AliasOracle>
where
    N: HostNotifier,
    H: HotKv,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignetNode").field("config", &self.config).finish_non_exhaustive()
    }
}

impl<N, H, AliasOracle> SignetNode<N, H, AliasOracle>
where
    N: HostNotifier,
    H: HotKv + Clone + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    AliasOracle: AliasOracleFactory,
{
    /// Create a new Signet instance. It is strongly recommend that you use the
    /// [`SignetNodeBuilder`] instead of this function.
    ///
    /// This function does NOT initialize the genesis state. As such it is NOT
    /// safe to use directly. The genesis state in storage MUST be initialized
    /// BEFORE calling this function.
    ///
    /// # Panics
    ///
    /// If invoked outside a tokio runtime.
    ///
    /// [`SignetNodeBuilder`]: crate::builder::SignetNodeBuilder
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn new_unsafe(
        notifier: N,
        config: SignetNodeConfig,
        storage: Arc<UnifiedStorage<H>>,
        alias_oracle: AliasOracle,
        client: reqwest::Client,
        blob_cacher: CacheHandle,
        serve_config: ServeConfig,
        rpc_config: StorageRpcConfig,
    ) -> eyre::Result<(Self, watch::Receiver<NodeStatus>)> {
        let constants =
            config.constants().wrap_err("failed to load signet constants from genesis")?;

        let (status, receiver) = watch::channel(NodeStatus::Booting);
        let chain = ChainNotifier::new(128);

        let this = Self {
            config: config.into(),
            notifier,
            storage,
            chain,
            rpc_handle: None,
            constants,
            status,
            alias_oracle: Arc::new(alias_oracle),
            blob_cacher,
            client,
            serve_config,
            rpc_config,
        };
        Ok((this, receiver))
    }

    /// Get the last rollup block number from hot storage.
    fn last_rollup_block(&self) -> eyre::Result<u64> {
        let reader = self.storage.reader()?;
        Ok(reader.last_block_number()?.unwrap_or(0))
    }

    /// Start the Signet instance, listening for host notifications. Trace any
    /// errors.
    #[instrument(skip(self))]
    pub async fn start(mut self) -> eyre::Result<()> {
        // Ensure hot and cold storage are at the same height. If either
        // is ahead, unwind to the minimum so the host re-delivers blocks.
        {
            let hot_tip = self.last_rollup_block()?;
            let cold_tip = self.storage.cold_reader().get_latest_block().await?.unwrap_or(0);

            let target = hot_tip.min(cold_tip);
            if target < hot_tip || target < cold_tip {
                info!(
                    hot_tip,
                    cold_tip,
                    unwind_to = target,
                    "storage layers inconsistent, reconciling"
                );
                self.storage.unwind_above(target)?;
            }
        }

        // This exists only to bypass the `tracing::instrument(err)` macro to
        // ensure that full sources get reported.
        self.start_inner().await.inspect_err(|err| {
            // using `:#` invokes the alternate formatter, which for eyre
            // includes cause reporting.
            let err = format!("{err:#}");

            let last_block =
                self.storage.reader().ok().and_then(|r| r.last_block_number().ok().flatten());

            tracing::error!(err, last_block, "Signet node crashed");
        })
    }

    /// Start the Signet instance, listening for host notifications.
    async fn start_inner(&mut self) -> eyre::Result<()> {
        debug!(constants = ?self.constants, "signet starting");

        self.start_rpc().await?;

        // Determine the last block written to storage for backfill
        let last_rollup_block = self.last_rollup_block()?;

        info!(last_rollup_block, "resuming execution from last rollup block found");

        // Update the node status channel with last block height
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(last_rollup_block));

        // Set the head position and backfill thresholds on the notifier
        let host_height = match last_rollup_block {
            0 => self.constants.host_deploy_height(),
            n => self.constants.pair_ru(n).host,
        };
        self.notifier.set_head(host_height);
        self.notifier.set_backfill_thresholds(self.config.backfill_max_blocks());

        info!(
            host_height,
            rollup_head_height = last_rollup_block,
            "signet listening for notifications"
        );

        // Handle incoming host notifications
        while let Some(notification) = self.notifier.next_notification().await {
            let notification = notification.wrap_err("error in host notifications stream")?;
            let changed = self
                .on_notification(&notification)
                .await
                .wrap_err("error while processing notification")?;
            if changed {
                let ru_height = self.last_rollup_block()?;
                self.update_block_tags(
                    ru_height,
                    notification.safe_block_number,
                    notification.finalized_block_number,
                )?;
            }
        }

        info!("signet shutting down");
        Ok(())
    }

    /// Runs on any notification received from the host.
    ///
    /// Returns `true` if any rollup state changed.
    #[instrument(parent = None, skip_all, fields(
        reverted = notification.revert_range().map(|r| r.len()).unwrap_or_default(),
        committed = notification.committed_chain().map(|c| c.len()).unwrap_or_default(),
    ))]
    pub async fn on_notification(
        &self,
        notification: &HostNotification<N::Chain>,
    ) -> eyre::Result<bool> {
        metrics::record_notification_received(notification);

        let mut changed = false;

        // NB: REVERTS MUST RUN FIRST
        if let Some(range) = notification.revert_range() {
            changed |=
                self.on_host_revert(range).await.wrap_err("error encountered during revert")?;
        }

        if let Some(chain) = notification.committed_chain() {
            changed |= self
                .process_committed_chain(chain)
                .await
                .wrap_err("error encountered during commit")?;
        }

        if changed {
            self.update_status_channel()?;
        }

        metrics::record_notification_processed(notification);
        Ok(changed)
    }

    /// Process a committed chain by extracting and executing blocks.
    ///
    /// Returns `true` if any rollup blocks were processed.
    async fn process_committed_chain(&self, chain: &Arc<N::Chain>) -> eyre::Result<bool> {
        let extractor = Extractor::new(self.constants.clone());
        let extracts: Vec<_> = extractor.extract_signet(chain.as_ref()).collect();

        let last_height = self.last_rollup_block()?;

        // Detect gaps: if the first block to process is not contiguous with
        // our last stored block, bail early so the notifier can re-backfill.
        if let Some(first) = extracts.iter().find(|e| e.ru_height > last_height) {
            let expected_next = last_height + 1;
            eyre::ensure!(
                first.ru_height == expected_next,
                "notification gap: expected ru_height {expected_next}, got {}. \
                 Last stored block is {last_height}.",
                first.ru_height,
            );
        }

        let mut processed = false;
        for block_extracts in extracts.iter().filter(|e| e.ru_height > last_height) {
            // Constructed per-block: hardforks must be rechecked each block,
            // and the remaining fields are cheap (Arcs / Copy types).
            let hardforks = EthereumHardfork::active_hardforks(
                &self.config.genesis().config,
                block_extracts.host_block.number(),
                block_extracts.host_block.timestamp(),
            );
            let processor = SignetBlockProcessorV1::new(
                self.constants.clone(),
                hardforks,
                self.storage.hot().clone(),
                self.alias_oracle.clone(),
                self.config.slot_calculator(),
                self.blob_cacher.clone(),
            );
            let executed = processor.process_block(block_extracts).await?;
            self.notify_new_block(&executed);
            self.storage.append_blocks(vec![executed])?;
            processed = true;
        }
        Ok(processed)
    }

    /// Send a new block notification on the broadcast channel.
    fn notify_new_block(&self, block: &signet_storage::ExecutedBlock) {
        let notif = NewBlockNotification {
            header: block.header.inner().clone(),
            transactions: block.transactions.iter().map(|tx| tx.inner().clone()).collect(),
            receipts: block.receipts.clone(),
        };
        // Ignore send errors — no subscribers is fine.
        let _ = self.chain.send_new_block(notif);
    }

    /// Send a reorg notification on the broadcast channel.
    fn notify_reorg(&self, drained: Vec<DrainedBlock>, common_ancestor: u64) {
        let removed_blocks = drained
            .into_iter()
            .map(|d| {
                let number = d.header.number();
                let hash = d.header.hash();
                let timestamp = d.header.timestamp();
                let logs = d.receipts.into_iter().flat_map(|r| r.receipt.logs).collect();
                RemovedBlock { number, hash, timestamp, logs }
            })
            .collect();
        let notif = ReorgNotification { common_ancestor, removed_blocks };
        // Ignore send errors — no subscribers is fine.
        let _ = self.chain.send_reorg(notif);
    }

    /// Update the status channel with the current rollup height.
    fn update_status_channel(&self) -> eyre::Result<()> {
        let ru_height = self.last_rollup_block()?;
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(ru_height));
        Ok(())
    }

    /// Update block tags (latest/safe/finalized) and notify the host of
    /// processed height.
    fn update_block_tags(
        &self,
        ru_height: u64,
        safe_block_number: Option<u64>,
        finalized_block_number: Option<u64>,
    ) -> eyre::Result<()> {
        // Safe height
        let safe_heights = self.clamp_host_heights(ru_height, safe_block_number);
        let safe_ru_height = safe_heights.rollup;
        debug!(safe_ru_height, "calculated safe ru height");

        // Finalized height
        let finalized_heights = self.clamp_host_heights(ru_height, finalized_block_number);
        debug!(
            finalized_host_height = finalized_heights.host,
            finalized_ru_height = finalized_heights.rollup,
            "calculated finalized heights"
        );

        // Atomically update all three tags
        self.chain.tags().update_all(ru_height, safe_ru_height, finalized_heights.rollup);

        // Notify the host that we've finished processing up to the finalized
        // height. Skip if finalized rollup height is still at genesis.
        if finalized_heights.rollup > 0 {
            self.update_highest_processed_height(finalized_heights.host)?;
        }

        debug!(
            latest = ru_height,
            safe = safe_ru_height,
            finalized = finalized_heights.rollup,
            "updated block tags"
        );
        Ok(())
    }

    /// Map a host block number to a [`PairedHeights`], clamping to the
    /// current rollup height. Returns genesis heights when the host block
    /// is below the rollup deploy height.
    fn clamp_host_heights(&self, ru_height: u64, host_block_number: Option<u64>) -> PairedHeights {
        let Some(heights) = host_block_number.and_then(|h| self.constants.pair_host(h)) else {
            return PairedHeights { host: self.constants.host_deploy_height(), rollup: 0 };
        };

        // Clamp to current rollup height if ahead.
        if heights.rollup > ru_height { self.constants.pair_ru(ru_height) } else { heights }
    }

    /// Update the host node with the highest processed host height.
    fn update_highest_processed_height(&self, finalized_host_height: u64) -> eyre::Result<()> {
        let adjusted_height = finalized_host_height.saturating_sub(1);
        debug!(finalized_host_height = adjusted_height, "Sending FinishedHeight notification");
        self.notifier.send_finished_height(adjusted_height).map_err(|e| eyre::eyre!(e))?;
        Ok(())
    }

    /// Called when the host chain has reverted a block or set of blocks.
    ///
    /// Returns `true` if any rollup state was unwound.
    ///
    /// # Errors
    ///
    /// Returns an error if the revert range is inconsistent with stored
    /// state — i.e. the range tip does not cover the node's current
    /// rollup tip.
    #[instrument(skip_all, fields(
        first = range.first(),
        tip = range.tip(),
    ))]
    pub async fn on_host_revert(&self, range: RevertRange) -> eyre::Result<bool> {
        let tip = range.tip();
        let first = range.first();

        // If the end is before the RU genesis, nothing to do.
        if tip <= self.constants.host_deploy_height() {
            return Ok(false);
        }

        // Validate that the revert range is consistent with our stored
        // state: the range tip must be at or above the host block that
        // produced our current rollup tip.
        let rollup_tip = self.last_rollup_block()?;
        let range_tip_ru = self
            .constants
            .host_block_to_rollup_block_num(tip)
            .ok_or_eyre("revert range tip does not map to a rollup block number")?;
        eyre::ensure!(
            range_tip_ru >= rollup_tip,
            "revert range tip (host {tip}, rollup {range_tip_ru}) \
             does not cover stored rollup tip ({rollup_tip})"
        );

        // Target is the block BEFORE the first block in the chain, or 0.
        let target = self
            .constants
            .host_block_to_rollup_block_num(first)
            .unwrap_or_default()
            .saturating_sub(1);

        let drained = self.storage.drain_above(target).await?;

        // Immediately cap block tags to the common ancestor so that
        // `latest` never references a block that no longer exists in
        // storage. This must happen before the reorg notification so
        // that RPC consumers see consistent tags.
        self.chain.tags().rewind_to(target);

        // The early return above guards against no-op reverts, so drained
        // should always contain at least one block. Guard defensively.
        debug_assert!(!drained.is_empty(), "drain_above returned empty after host revert");
        if !drained.is_empty() {
            self.notify_reorg(drained, target);
        }

        Ok(true)
    }
}
