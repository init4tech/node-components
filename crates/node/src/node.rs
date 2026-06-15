use crate::{
    NodeStatus,
    journal_sync::{
        JOURNAL_SYNC_BACKPRESSURE_CAPACITY, JournalExitKind, JournalIngestor, RunningJournalSync,
        SyncOutcome, build_journal_client, collapse_sync_failure, is_caught_up_to_host,
        journal_sync_loop, journal_task_result, seed_journal_checkpoints,
    },
    metrics,
};
use alloy::{
    consensus::BlockHeader,
    primitives::{B256, keccak256},
};
use bytes::Bytes;
use eyre::{Context, OptionExt, Report, eyre};
use signet_blobber::CacheHandle;
use signet_block_processor::{AliasOracleFactory, SignetBlockProcessorV1};
use signet_evm::EthereumHardfork;
use signet_extract::{Extractable, Extractor};
use signet_journal::{GENESIS_JOURNAL_HASH, HostJournal, Journal, JournalMeta};
use signet_journal_chain::{
    JournalChainBuilder, JournalChainConfig, JournalChainError, JournalChainEvent,
    JournalChainHandle, JournalChainParts, RingBufferConfig, extract_signet_metadata,
};
use signet_node_config::{JournalConfig, SignetNodeConfig, SyncStrategy};
use signet_node_types::{HostNotification, HostNotifier, RevertRange};
use signet_rpc::{
    ChainNotifier, NewBlockNotification, RemovedBlock, ReorgNotification, RpcServerGuard,
    ServeConfig, StorageRpcConfig,
};
use signet_storage::{
    DrainedBlock, ExecutedBlock, HistoryRead, HotDbRead, HotKv, HotKvRead, UnifiedStorage,
};
use signet_types::{PairedHeights, constants::SignetSystemConstants};
use std::{borrow::Cow, fmt, sync::Arc};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};
use trevm::{
    journal::{BundleStateIndex, JournalEncode},
    revm::database::DBErrorMarker,
};

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

    /// Handle to the journal chain, used by the RPC layer to mount the
    /// `/journal` WebSocket endpoint.
    pub(crate) journal_chain_handle: JournalChainHandle,

    /// Sender into the journal chain's bounded input channel. The block
    /// processing loop pushes serialized journal bytes here after each block
    /// is committed. It's dropped during shutdown; that closes the channel
    /// and lets the journal chain's ingestion task drain and exit cleanly.
    journal_sender: mpsc::Sender<Bytes>,

    /// Join handle for the journal chain's ingestion task.
    journal_task: Option<JoinHandle<Result<(), JournalChainError>>>,

    /// The configured sync strategy. Selects the startup path in
    /// [`Self::start_inner`]: block execution or journal application.
    sync_strategy: SyncStrategy,

    /// Receiver end of the journal chain's backpressured event channel, present only under
    /// [`SyncStrategy::Journals`].
    backpressured_receiver: Option<mpsc::Receiver<JournalChainEvent>>,

    /// Cancellation token for graceful shutdown.
    cancellation_token: CancellationToken,
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
    pub(crate) fn new_unsafe(
        notifier: N,
        config: SignetNodeConfig,
        storage: Arc<UnifiedStorage<H>>,
        alias_oracle: AliasOracle,
        client: reqwest::Client,
        blob_cacher: CacheHandle,
        serve_config: ServeConfig,
        rpc_config: StorageRpcConfig,
        cancellation_token: CancellationToken,
    ) -> eyre::Result<(Self, watch::Receiver<NodeStatus>)> {
        let constants =
            config.constants().wrap_err("failed to load signet constants from genesis")?;

        let sync_strategy = config.journal().sync_strategy();
        config.journal().warn_on_misconfiguration();
        config.journal().validate().wrap_err("invalid journal configuration")?;

        let (status, receiver) = watch::channel(NodeStatus::Booting);
        let chain = ChainNotifier::new(128);

        // Under the journals strategy the journal chain feeds an ingestor through a backpressured
        // sender; under the blocks strategy there is no such consumer and the producer drives the
        // chain.
        let backpressured = match sync_strategy {
            SyncStrategy::Journals => Some(mpsc::channel(JOURNAL_SYNC_BACKPRESSURE_CAPACITY)),
            SyncStrategy::Blocks => None,
        };
        let (backpressured_sender, backpressured_receiver) = match backpressured {
            Some((sender, receiver)) => (Some(sender), Some(receiver)),
            None => (None, None),
        };

        let JournalChainParts {
            chain: journal_chain,
            handle: journal_chain_handle,
            journal_sender,
        } = build_journal_chain(config.journal(), backpressured_sender)?;
        let journal_task = journal_chain.run();

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
            journal_chain_handle,
            journal_sender,
            journal_task: Some(journal_task),
            sync_strategy,
            backpressured_receiver,
            cancellation_token,
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
    pub async fn start(self) -> eyre::Result<()> {
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
                self.storage.unwind_above(target).await?;
            }
        }

        let storage = Arc::clone(&self.storage);

        // This exists only to bypass the `tracing::instrument(err)` macro to
        // ensure that full sources get reported.
        self.start_inner().await.inspect_err(|err| {
            // using `:#` invokes the alternate formatter, which for eyre
            // includes cause reporting.
            let err = format!("{err:#}");

            let last_block =
                storage.reader().ok().and_then(|r| r.last_block_number().ok().flatten());

            error!(err, last_block, "Signet node crashed");
        })
    }

    /// Start the Signet instance, listening for host notifications.
    async fn start_inner(mut self) -> eyre::Result<()> {
        debug!(constants = ?self.constants, "signet starting");

        self.start_rpc().await?;

        // Determine the last block written to storage for backfill
        let last_rollup_block = self.last_rollup_block()?;

        info!(last_rollup_block, "resuming execution from last rollup block found");

        // Update the node status channel with last block height
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(last_rollup_block));

        match self.sync_strategy {
            SyncStrategy::Journals => self.run_journal_sync().await,
            SyncStrategy::Blocks => self.run_block_sync(last_rollup_block).await,
        }
    }

    /// Drive the node by executing host blocks (the `blocks` strategy).
    async fn run_block_sync(mut self, last_rollup_block: u64) -> eyre::Result<()> {
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

        let mut journal_task = self
            .journal_task
            .take()
            .expect("journal task should be set by new_unsafe and only taken here");

        // Handle incoming host notifications. Also observe the journal
        // chain's ingestion task: an unexpected exit there means new
        // journals cannot be emitted, so the node must shut down.
        let main_result: eyre::Result<()> = loop {
            tokio::select! {
                biased;
                () = self.cancellation_token.cancelled() => {
                    info!("cancellation requested, shutting down block sync");
                    break Ok(());
                }
                result = &mut journal_task => {
                    return journal_task_result(result, JournalExitKind::Unexpected);
                },
                notification = self.notifier.next_notification() => {
                    let Some(notification) = notification else { break Ok(()) };
                    match notification.wrap_err("error in host notifications stream") {
                        Ok(notification) => {
                            if let Err(error) = self
                                .on_notification(&notification)
                                .await
                                .wrap_err("error while processing notification")
                            {
                                break Err(error);
                            }
                        }
                        Err(error) => break Err(error),
                    }
                }
            }
        };

        info!("signet shutting down, awaiting journal chain");
        // Always close the sender and await the chain task on the way out so
        // its result is observed rather than dropped along with the join
        // handle. The main-loop error takes precedence; the journal task's
        // result only surfaces if the main loop succeeded.
        drop(self.journal_sender);
        let journal_result =
            journal_task_result((&mut journal_task).await, JournalExitKind::Expected);
        main_result.and(journal_result)
    }

    /// Drive the node by applying journals from upstream sources (the `journals` strategy): spawn
    /// the client and ingestor alongside the chain task, then either hand off to block execution
    /// once the applied state reaches the host tip or wind down on shutdown. A node that boots
    /// already at the host tip short-circuits straight to block execution without connecting
    /// upstream.
    ///
    /// Application failures are fatal - the node bails, with no source failover. A bad-content
    /// fault (undecodable journal, unsupported version, or a header that does not chain onto
    /// storage) recurs against any honest source since journals are deterministic; a local storage
    /// fault (database/IO error) may clear on restart, which re-seeds the client checkpoint from
    /// the persisted `JournalHashes` tip (see [`seed_journal_checkpoints`]) and resubscribes.
    async fn run_journal_sync(mut self) -> eyre::Result<()> {
        let mut journal_task = self
            .journal_task
            .take()
            .expect("journal task should be set by new_unsafe and only taken here");

        let event_rx = self
            .backpressured_receiver
            .take()
            .expect("backpressured receiver must be set under the journals strategy");

        // Read the tip after `start()`'s hot/cold reconciliation, so the catch-up decision and the
        // client checkpoint reflect the reconciled tip rather than the pre-reconciliation one seen
        // when the node was built (an unwind there can move it).
        let startup_tip = self.last_rollup_block()?;
        if is_caught_up_to_host(&self.notifier, startup_tip, &self.constants).await {
            info!("already caught up to host tip at startup, starting block execution");
            // Release the receiver before block execution: with it dropped the journal chain's
            // first post-handoff `blocking_send` fails and the chain clears its sender, so a
            // producing chain never stalls on a full, unconsumed channel.
            drop(event_rx);
            self.journal_task = Some(journal_task);
            return self.run_block_sync(startup_tip).await;
        }

        // Seed the client checkpoint from the reconciled storage tip and assemble the sync
        // machinery (deliberately deferred from `new_unsafe`; see `backpressured_receiver`).
        let checkpoints = seed_journal_checkpoints(self.storage.as_ref())?;
        let client = build_journal_client(self.config.journal(), checkpoints)?;
        let sync_token = self.cancellation_token.child_token();
        // Seed the progress channel with the storage tip; the ingestor overwrites it after every
        // applied event.
        let (height_tx, applied_rollup_height) = watch::channel(checkpoints.primary.height);
        let ingestor = JournalIngestor::new(
            Arc::clone(&self.storage),
            self.chain.clone(),
            self.status.clone(),
            event_rx,
            sync_token.clone(),
            height_tx,
        );

        let mut sync = RunningJournalSync::start(
            client,
            self.journal_sender.clone(),
            ingestor,
            sync_token,
            applied_rollup_height,
        );
        info!(
            primary_height = checkpoints.primary.height,
            primary_hash = %checkpoints.primary.hash,
            fallback_height = checkpoints.fallback.height,
            fallback_hash = %checkpoints.fallback.hash,
            sources = ?self.config.journal().sources(),
            "journal sync started",
        );

        let outcome = journal_sync_loop(
            &mut self.notifier,
            &self.constants,
            &self.cancellation_token,
            &mut journal_task,
            &mut sync,
        )
        .await;

        match outcome {
            SyncOutcome::CaughtUp => {
                info!("journal sync caught up to host tip, transitioning to block execution");
                // Stop the client and ingestor cooperatively; keep the journal chain alive so the
                // producer path can keep pushing through `self.journal_sender` after the handoff.
                // `cancel_and_drain` lets the ingestor apply any in-flight journals first, so
                // storage catches up to the chain's tip. Any residual lead the chain still holds
                // (from input buffered beyond what the ingestor drained, bounded by how far the
                // upstream ran past storage at catch-up - at most the transition margin) is
                // reconciled when block execution re-derives those heights: the chain sees
                // identical journals and treats them as duplicates.
                sync.cancel_and_drain().await;
                let rollup_tip = self.last_rollup_block()?;
                self.journal_task = Some(journal_task);
                self.run_block_sync(rollup_tip).await
            }
            SyncOutcome::Shutdown => {
                info!("cancellation requested, shutting down journal sync");
                // Closing the journal chain input lets the chain finish; `cancel_and_drain`
                // finishes off the sync tasks. The parent token already cascaded, so the cancel
                // inside the drain is just defensive.
                drop(self.journal_sender);
                sync.cancel_and_drain().await;
                journal_task_result(journal_task.await, JournalExitKind::Expected)
            }
            SyncOutcome::Failure(failure) => {
                drop(self.journal_sender);
                sync.cancel_and_drain().await;
                collapse_sync_failure(failure, journal_task).await
            }
        }
    }

    /// Runs on any notification received from the host.
    ///
    /// Drives the full per-notification pipeline: revert (if any), committed chain (if any),
    /// status-channel refresh, and the safe / finalized tag and `FinishedHeight` update. When
    /// the revert step requests a shutdown - e.g. because the journal chain's ring buffer no
    /// longer holds the post-revert tip - the local tag refresh and the host-bound
    /// `FinishedHeight` still run so reth can prune to the post-revert finalized height before
    /// the bail error propagates out of the main loop. A tag-update failure on the shutdown
    /// path is logged but does not override the shutdown error.
    #[instrument(parent = None, skip_all, fields(
        reverted = notification.revert_range().map(|r| r.len()).unwrap_or_default(),
        committed = notification.committed_chain().map(|c| c.len()).unwrap_or_default(),
    ))]
    pub async fn on_notification(
        &self,
        notification: &HostNotification<N::Chain>,
    ) -> eyre::Result<()> {
        metrics::record_notification_received(notification);

        let mut changed = false;
        let mut shutdown: Option<Report> = None;

        // NB: REVERTS MUST RUN FIRST
        if let Some(range) = notification.revert_range() {
            let outcome =
                self.on_host_revert(range).await.wrap_err("error encountered during revert")?;
            changed |= outcome.changed;
            shutdown = outcome.shutdown;
        }

        // Skip committed-chain processing when a shutdown is pending: storage has been drained
        // to a point the in-process journal chain can no longer anchor, so emitting a fresh
        // journal would just fail validation downstream.
        if shutdown.is_none()
            && let Some(chain) = notification.committed_chain()
        {
            changed |= self
                .process_committed_chain(chain)
                .await
                .wrap_err("error encountered during commit")?;
        }

        if changed {
            let tag_result = self
                .update_status_channel()
                .and_then(|()| self.last_rollup_block())
                .and_then(|ru_height| {
                    self.update_block_tags(
                        ru_height,
                        notification.safe_block_number,
                        notification.finalized_block_number,
                    )
                });
            match (tag_result, shutdown.is_some()) {
                (Err(tag_error), true) => {
                    // Shutdown error is the root cause; log the secondary failure so it's
                    // not lost, then let the shutdown error propagate below.
                    error!(error = ?tag_error, "tag refresh failed during shutdown bail");
                }
                (Err(tag_error), false) => return Err(tag_error),
                (Ok(()), _) => {}
            }
        }

        metrics::record_notification_processed(notification);

        match shutdown {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Process a committed chain by extracting and executing blocks.
    ///
    /// Returns `true` if any rollup blocks were processed.
    async fn process_committed_chain(&self, chain: &Arc<N::Chain>) -> eyre::Result<bool> {
        let extractor = Extractor::new(self.constants.clone());
        let extracts: Vec<_> = extractor.extract_signet(chain.as_ref()).collect();

        let last_height = self.last_rollup_block()?;

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
            let previous_hash = self.previous_journal_hash()?;
            let (executed, journal_bytes) =
                encode_journal(previous_hash, block_extracts.host_block.number(), executed);
            // Order: emit -> append -> notify.
            //
            // `emit` first so any post-emit failure (append error, hard crash) leaves storage
            // at N-1; the host re-delivers N on restart and the producer re-emits
            // deterministically against a freshly-built chain (`tip = None`), avoiding the
            // permanent "block persisted, journal lost" gap that the reverse order leaves
            // behind. `notify` last so `eth_subscribe("newHeads")` clients querying storage
            // immediately after the broadcast see block N already indexed; `send_new_block`
            // only errors when there are no subscribers, which is safe to ignore.
            //
            // `/journal` consumers may see the journal before storage has indexed the block,
            // but they reconstruct state from the journal itself, not from storage RPC, so
            // they are unaffected by that ordering.
            //
            // The broadcast payload is built before `append_blocks` consumes `executed`.
            let notification = NewBlockNotification {
                header: executed.header.inner().clone(),
                transactions: executed.transactions.iter().map(|tx| tx.inner().clone()).collect(),
                receipts: executed.receipts.clone(),
            };
            self.emit_journal(block_extracts.ru_height, journal_bytes).await?;
            self.storage.append_blocks(vec![executed]).await?;
            let _ = self.chain.send_new_block(notification);
            processed = true;
        }
        Ok(processed)
    }

    /// Read the rolling `previous_journal_hash` for the next produced block from storage.
    ///
    /// Returns [`GENESIS_JOURNAL_HASH`] when the database is empty or only contains the genesis
    /// block, and also when the storage tip has no recorded journal hash - the persistence-off
    /// startup path, also covering an upgrade from a pre-`JournalHashes` build. In that fallback
    /// the next emit presents as the initial journal of a fresh chain; downstream `/journal`
    /// consumers with cached checkpoints will fail validation and must re-bootstrap.
    ///
    /// The fallback is only sound while the in-process journal chain is itself fresh
    /// (`tip = None`); if the chain already holds a tip, emitting with `previous_hash =
    /// GENESIS_JOURNAL_HASH` would be rejected as `PreviousHashMismatch` and the journal task
    /// would exit with a generic error. That combination indicates storage corruption or a
    /// block appended without going through [`encode_journal`], so surface the real cause here.
    fn previous_journal_hash(&self) -> eyre::Result<B256> {
        let reader = self.storage.reader()?;
        let storage_tip = reader.last_block_number()?.unwrap_or(0);

        if storage_tip == 0 {
            return Ok(GENESIS_JOURNAL_HASH);
        }

        if let Some(hash) = reader.get_journal_hash(storage_tip)? {
            return Ok(hash);
        }

        if self.journal_chain_handle.tip().is_some() {
            return Err(eyre!(
                "storage tip {storage_tip} has no recorded journal hash but the in-process \
                 journal chain already holds a tip; emitting a fresh-chain initial here would \
                 be rejected as `PreviousHashMismatch`. This indicates storage corruption or a \
                 block appended without going through `encode_journal`."
            ));
        }

        warn!(
            storage_tip,
            "no journal hash recorded for storage tip; presenting next journal as a \
             fresh-chain initial. Downstream `/journal` consumers must re-bootstrap."
        );
        Ok(GENESIS_JOURNAL_HASH)
    }

    /// Push the encoded journal bytes into the journal chain. Awaits if
    /// the chain's input channel is full so the producer naturally
    /// backpressures during backfill rather than crashing the node. The
    /// only failure mode is the receiver being closed, which means the
    /// ingestion task has exited and the node cannot continue.
    #[instrument(skip(self, bytes), fields(len = bytes.len()))]
    async fn emit_journal(&self, ru_height: u64, bytes: Bytes) -> eyre::Result<()> {
        self.journal_sender
            .send(bytes)
            .await
            .map_err(|_| eyre!("journal chain ingestion task exited unexpectedly"))
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
    /// Returns a [`RevertOutcome`] describing whether any rollup state was unwound and whether
    /// the caller must shut the node down after running its post-notification work.
    ///
    /// # Errors
    ///
    /// Returns an error if the revert range is inconsistent with stored
    /// state — i.e. the range tip does not cover the node's current
    /// rollup tip.
    #[instrument(skip_all, fields(first = range.first(), tip = range.tip()))]
    async fn on_host_revert(&self, range: RevertRange) -> eyre::Result<RevertOutcome> {
        let tip = range.tip();
        let first = range.first();

        // If the end is before the RU genesis, nothing to do.
        if tip <= self.constants.host_deploy_height() {
            return Ok(RevertOutcome::unchanged());
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

        let chain_tip = self.journal_chain_handle.tip();
        let shutdown_for_chain_reset = revert_forces_shutdown(
            target,
            rollup_tip,
            chain_tip.map(|checkpoint| checkpoint.height),
            self.journal_chain_handle.contains(target),
        );

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

        let shutdown = shutdown_for_chain_reset.then(|| {
            eyre!(
                "rollup reverted to height {target} but the in-process journal chain's ring \
                 buffer no longer holds that height (either it is genesis, or it has been \
                 evicted by ring-buffer rotation); the chain cannot validate the post-revert \
                 reorg replacement. Restart the node to rebuild the chain in lockstep with \
                 storage."
            )
        });

        Ok(RevertOutcome { changed: true, shutdown })
    }
}

/// Outcome of [`SignetNode::on_host_revert`].
#[derive(Debug)]
struct RevertOutcome {
    /// Whether any rollup state was unwound.
    changed: bool,
    /// Set when the in-process journal chain can no longer validate the post-revert tip and
    /// the node must shut down. Storage and tags have already been drained; the caller is
    /// responsible for running the per-notification tag refresh before propagating this.
    shutdown: Option<Report>,
}

impl RevertOutcome {
    /// A revert that performed no work and requires no shutdown.
    const fn unchanged() -> Self {
        Self { changed: false, shutdown: None }
    }
}

/// Build a store-less journal chain.
///
/// [`extract_signet_metadata`] is the parser the chain calls on every
/// incoming journal to pull out the version tag, previous-journal hash,
/// and block height it needs to validate continuity and index the entry.
///
/// `backpressured_sender` is supplied only by a syncing node, so the chain
/// throttles ingestion to the ingestor's pace; a producing node leaves it
/// `None` and the chain drives only the broadcast channel.
fn build_journal_chain(
    config: &JournalConfig,
    backpressured_sender: Option<mpsc::Sender<JournalChainEvent>>,
) -> eyre::Result<JournalChainParts> {
    let chain_config = JournalChainConfig {
        ring_buffer: RingBufferConfig {
            max_bytes: config.ring_buffer_max_bytes(),
            max_count: config.ring_buffer_max_count(),
        },
        max_subscriber_lag: config.max_subscriber_lag(),
    };

    let mut builder = JournalChainBuilder::new(chain_config, extract_signet_metadata);
    if let Some(sender) = backpressured_sender {
        builder = builder.with_backpressured_sender(sender);
    }
    builder.build().wrap_err("failed to build journal chain")
}

/// Build the serialized journal for a freshly executed block, stamping the resulting keccak256
/// of the wire-encoded `Journal::V1` onto [`ExecutedBlock::journal_hash`] so `append_blocks`
/// persists it into the `JournalHashes` table. Returns the modified block plus the encoded wire
/// bytes; the hash on the block becomes the `previous_hash` for the next block.
#[instrument(skip(executed), fields(ru_height = executed.header.number()))]
fn encode_journal(
    previous_hash: B256,
    host_height: u64,
    mut executed: ExecutedBlock,
) -> (ExecutedBlock, Bytes) {
    let host_journal = HostJournal::new(
        JournalMeta::new(host_height, previous_hash, Cow::Borrowed(executed.header.inner())),
        BundleStateIndex::from(&executed.bundle),
    );
    let encoded: Bytes = Journal::V1(host_journal).encoded().into();
    executed.journal_hash = Some(keccak256(&encoded));
    (executed, encoded)
}

/// Decide whether a host revert that rewinds the rollup to `target` must shut the node down
/// because the in-process journal chain can no longer anchor the post-revert replacement at
/// `target + 1`.
///
/// Inputs are the revert `target`, the node's current stored `rollup_tip`, the journal
/// chain's tip height (`chain_tip_height`, `None` when the chain is fresh), and whether its
/// ring buffer still holds `target` (`chain_contains_target`).
///
/// Two situations force a shutdown:
///
///   * `target == 0`: genesis is never stored in the ring buffer. The producer has already
///     emitted journals for every block being reverted, so the chain holds - or, once it
///     drains its queued journals, will hold - a tip `>= 1` that it cannot rewind past
///     genesis. This is independent of how far the chain's asynchronous ingestion has
///     progressed, so it must NOT be gated on the live tip: doing so races ingestion and
///     makes the bail non-deterministic. The `rollup_tip > 0` guard keeps a no-op revert of
///     a genesis-only chain (nothing emitted, tip never set) from spuriously bailing.
///   * `target > 0` but the journal for `target` has been evicted by ring-buffer rotation
///     (`chain_tip_height >= target && !chain_contains_target`).
///
/// A tip *behind* a non-zero `target` is fine: `emit` precedes `append`, so the missing
/// journals are queued ahead of the chain and will be ingested before any replacement
/// arrives.
const fn revert_forces_shutdown(
    target: u64,
    rollup_tip: u64,
    chain_tip_height: Option<u64>,
    chain_contains_target: bool,
) -> bool {
    if target == 0 {
        rollup_tip > 0
    } else {
        match chain_tip_height {
            Some(tip_height) => tip_height >= target && !chain_contains_target,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::revert_forces_shutdown;

    // Revert-to-genesis of a non-empty chain must bail regardless of how far the journal
    // chain's asynchronous ingestion has progressed. The `None` case is the exact boundary
    // that previously raced ingestion and made `test_revert_to_genesis_bails` flaky: the
    // revert arrived before the chain ingested the only block's journal, the bail was gated
    // on the (still unset) live tip, and the node failed to shut down.
    #[test]
    fn revert_to_genesis_bails_independent_of_ingestion() {
        // Chain has not yet ingested the journal (tip unset) - must still bail.
        assert!(revert_forces_shutdown(0, 1, None, false));
        // Chain has ingested the journal (tip set) - bails for the same reason. `contains` is
        // irrelevant at genesis, so it must not change the outcome either way.
        assert!(revert_forces_shutdown(0, 1, Some(1), false));
        assert!(revert_forces_shutdown(0, 5, Some(5), true));
    }

    // A revert that targets genesis on a chain that only ever held genesis is a no-op: nothing
    // was emitted, so there is no replacement to anchor and the node must not bail.
    #[test]
    fn revert_to_genesis_of_empty_chain_does_not_bail() {
        assert!(!revert_forces_shutdown(0, 0, None, false));
    }

    // For `target > 0`, a chain that has not yet caught up to `target` (or sits below it) is
    // fine: the missing journals are queued ahead of the chain and will be ingested before any
    // replacement arrives.
    #[test]
    fn revert_above_genesis_with_chain_behind_does_not_bail() {
        assert!(!revert_forces_shutdown(2, 3, None, false));
        assert!(!revert_forces_shutdown(2, 3, Some(1), false));
    }

    // For `target > 0`, the chain can anchor the replacement as long as its ring buffer still
    // holds `target`, even after it has advanced past it.
    #[test]
    fn revert_above_genesis_with_target_retained_does_not_bail() {
        assert!(!revert_forces_shutdown(2, 3, Some(3), true));
        assert!(!revert_forces_shutdown(2, 2, Some(2), true));
    }

    // For `target > 0`, only an evicted `target` (advanced past it, ring buffer no longer holds
    // it) is fatal: the chain cannot anchor the replacement and the node must bail.
    #[test]
    fn revert_above_genesis_with_target_evicted_bails() {
        assert!(revert_forces_shutdown(2, 3, Some(3), false));
    }
}
