use crate::{NodeStatus, metrics, serve::RpcServerGuard};
use alloy::consensus::BlockHeader;
use eyre::Context;
use futures_util::StreamExt;
use reth::{
    chainspec::EthChainSpec,
    primitives::EthPrimitives,
    providers::{BlockIdReader, BlockReader, HeaderProvider, StateProviderFactory},
};
use reth_exex::{ExExContext, ExExEvent, ExExHead, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, FullNodeTypes, NodeTypes};
use signet_blobber::ExtractableChainShim;
use signet_block_processor::{AliasOracleFactory, SignetBlockProcessorV1};
use signet_extract::Extractor;
use signet_node_config::SignetNodeConfig;
use signet_rpc::{BlockTags, NewBlockNotification};
use signet_storage::{HistoryRead, HotKv, HotKvRead, UnifiedStorage};
use signet_types::{PairedHeights, constants::SignetSystemConstants};
use std::{fmt, sync::Arc};
use tokio::sync::{broadcast, watch};
use tracing::{debug, info, instrument};
use trevm::revm::database::DBErrorMarker;

/// Type alias for the host primitives.
type PrimitivesOf<Host> = <<Host as FullNodeTypes>::Types as NodeTypes>::Primitives;
type ExExNotification<Host> = reth_exex::ExExNotification<PrimitivesOf<Host>>;
type Chain<Host> = reth::providers::Chain<PrimitivesOf<Host>>;

/// Signet context and configuration.
pub struct SignetNode<Host, H, AliasOracle = Box<dyn StateProviderFactory>>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv,
{
    /// The host context, which manages provider access and notifications.
    pub(crate) host: ExExContext<Host>,

    /// Signet node configuration.
    pub(crate) config: Arc<SignetNodeConfig>,

    /// Unified hot + cold storage backend.
    pub(crate) storage: Arc<UnifiedStorage<H>>,

    /// Atomic block tag tracking (latest/safe/finalized).
    /// Shared with the RPC context via `Clone` (backed by `Arc<AtomicU64>`).
    pub(crate) tags: BlockTags,

    /// Notification sender for new blocks.
    /// Shared with the RPC context's `SubscriptionManager` via `Clone`.
    pub(crate) notif_tx: broadcast::Sender<NewBlockNotification>,

    /// The join handle for the RPC server. None if the RPC server is not
    /// yet running.
    pub(crate) rpc_handle: Option<RpcServerGuard>,

    /// Chain configuration constants.
    pub(crate) constants: SignetSystemConstants,

    /// Status channel, currently used only for testing.
    pub(crate) status: watch::Sender<NodeStatus>,

    /// The block processor.
    pub(crate) processor: SignetBlockProcessorV1<H, AliasOracle>,

    /// A reqwest client, used by the blob fetch and the tx cache forwarder.
    pub(crate) client: reqwest::Client,
}

impl<Host, H, AliasOracle> fmt::Debug for SignetNode<Host, H, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignetNode").field("config", &self.config).finish_non_exhaustive()
    }
}

impl<Host, H, AliasOracle> SignetNode<Host, H, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
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
    pub fn new_unsafe(
        ctx: ExExContext<Host>,
        config: SignetNodeConfig,
        storage: Arc<UnifiedStorage<H>>,
        alias_oracle: AliasOracle,
        client: reqwest::Client,
    ) -> eyre::Result<(Self, watch::Receiver<NodeStatus>)> {
        let constants =
            config.constants().wrap_err("failed to load signet constants from genesis")?;

        let (status, receiver) = watch::channel(NodeStatus::Booting);
        let tags = BlockTags::new(0, 0, 0);
        let (notif_tx, _) = broadcast::channel(128);

        let blob_cacher = signet_blobber::BlobFetcher::builder()
            .with_config(config.block_extractor())?
            .with_pool(ctx.pool().clone())
            .with_client(client.clone())
            .build_cache()
            .wrap_err("failed to create blob cacher")?
            .spawn();

        let processor = SignetBlockProcessorV1::new(
            constants.clone(),
            config.chain_spec().clone(),
            storage.hot().clone(),
            alias_oracle,
            config.slot_calculator(),
            blob_cacher,
        );

        let this = Self {
            config: config.into(),
            host: ctx,
            storage,
            tags,
            notif_tx,
            rpc_handle: None,
            constants,
            status,
            processor,
            client,
        };
        Ok((this, receiver))
    }

    /// Start the Signet instance, listening for ExEx notifications. Trace any
    /// errors.
    #[instrument(skip(self), fields(host = ?self.host.config.chain.chain()))]
    pub async fn start(mut self) -> eyre::Result<()> {
        if let Some(lag_start) = self.storage.cold_lag().await? {
            info!(%lag_start, "cold storage behind hot, will catch up asynchronously");
        }

        // This exists only to bypass the `tracing::instrument(err)` macro to
        // ensure that full sources get reported.
        self.start_inner().await.inspect_err(|err| {
            // using `:#` invokes the alternate formatter, which for eyre
            // includes cause reporting.
            let err = format!("{err:#}");

            let last_block = self
                .storage
                .reader()
                .ok()
                .and_then(|r| HistoryRead::last_block_number(&r).ok().flatten());
            let exex_head = last_block.and_then(|h| self.set_exex_head(h).ok());

            tracing::error!(err, last_block, ?exex_head, "Signet node crashed");
        })
    }

    /// Start the Signet instance, listening for ExEx notifications.
    async fn start_inner(&mut self) -> eyre::Result<()> {
        debug!(constants = ?self.constants, "signet starting");

        self.start_rpc().await?;

        // Determine the last block written to storage for backfill
        let reader = self.storage.reader()?;
        let last_rollup_block = HistoryRead::last_block_number(&reader)?.unwrap_or(0);
        drop(reader);

        info!(last_rollup_block, "resuming execution from last rollup block found");

        // Update the node status channel with last block height
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(last_rollup_block));

        // Sets the ExEx head position relative to that last block
        let exex_head = self.set_exex_head(last_rollup_block)?;
        info!(
            host_head = exex_head.block.number,
            host_hash = %exex_head.block.hash,
            rollup_head_height = last_rollup_block,
            "signet listening for notifications"
        );

        // Handle incoming ExEx notifications
        while let Some(notification) = self.host.notifications.next().await {
            let notification = notification.wrap_err("error in reth host notifications stream")?;
            self.on_notification(notification)
                .await
                .wrap_err("error while processing notification")?;
        }

        info!("signet shutting down");
        Ok(())
    }

    /// Sets the head of the Exex chain from the last rollup block, handling
    /// genesis conditions if necessary.
    fn set_exex_head(&mut self, last_rollup_block: u64) -> eyre::Result<ExExHead> {
        // If the last rollup block is 0, shortcut to the host rollup
        // deployment block.
        if last_rollup_block == 0 {
            let host_deployment_block =
                self.host.provider().block_by_number(self.constants.host_deploy_height())?;
            match host_deployment_block {
                Some(genesis_block) => {
                    let exex_head = ExExHead { block: genesis_block.num_hash_slow() };
                    self.host.notifications.set_with_head(exex_head);
                    return Ok(exex_head);
                }
                None => {
                    let host_ru_deploy_block = self.constants.host_deploy_height();
                    debug!(
                        host_ru_deploy_block,
                        "Host deploy height not found. Falling back to genesis block"
                    );
                    let genesis_block = self
                        .host
                        .provider()
                        .block_by_number(0)?
                        .expect("failed to find genesis block");
                    let exex_head = ExExHead { block: genesis_block.num_hash_slow() };
                    self.host.notifications.set_with_head(exex_head);
                    return Ok(exex_head);
                }
            }
        }

        // Find the corresponding host block for the rollup block number.
        let host_height = self.constants.pair_ru(last_rollup_block).host;

        match self.host.provider().block_by_number(host_height)? {
            Some(host_block) => {
                debug!(host_height, "found host block for height");
                let exex_head = ExExHead { block: host_block.num_hash_slow() };
                self.host.notifications.set_with_head(exex_head);
                Ok(exex_head)
            }
            None => {
                debug!(host_height, "no host block found for host height");
                let genesis_block =
                    self.host.provider().block_by_number(0)?.expect("failed to find genesis block");
                let exex_head = ExExHead { block: genesis_block.num_hash_slow() };
                self.host.notifications.set_with_head(exex_head);
                Ok(exex_head)
            }
        }
    }

    /// Runs on any notification received from the ExEx context.
    #[instrument(parent = None, skip_all, fields(
        reverted = notification.reverted_chain().map(|c| c.len()).unwrap_or_default(),
        committed = notification.committed_chain().map(|c| c.len()).unwrap_or_default(),
    ))]
    pub async fn on_notification(&self, notification: ExExNotification<Host>) -> eyre::Result<()> {
        metrics::record_notification_received(&notification);

        let mut changed = false;

        // NB: REVERTS MUST RUN FIRST
        if let Some(chain) = notification.reverted_chain() {
            self.on_host_revert(&chain).wrap_err("error encountered during revert")?;
            changed = true;
        }

        if let Some(chain) = notification.committed_chain() {
            self.process_committed_chain(&chain)
                .await
                .wrap_err("error encountered during commit")?;
            changed = true;
        }

        if changed {
            self.update_status()?;
        }

        metrics::record_notification_processed(&notification);
        Ok(())
    }

    /// Process a committed chain by extracting and executing blocks.
    async fn process_committed_chain(&self, chain: &Arc<Chain<Host>>) -> eyre::Result<()> {
        let shim = ExtractableChainShim::new(chain);
        let extractor = Extractor::new(self.constants.clone());
        let extracts: Vec<_> = extractor.extract_signet(&shim).collect();

        let reader = self.storage.reader()?;
        let last_height = HistoryRead::last_block_number(&reader)?.unwrap_or(0);
        drop(reader);

        for block_extracts in extracts.iter().filter(|e| e.ru_height > last_height) {
            let executed = self.processor.process_block(block_extracts).await?;
            self.notify_new_block(&executed);
            self.storage.append_blocks(vec![executed])?;
        }
        Ok(())
    }

    /// Send a new block notification on the broadcast channel.
    fn notify_new_block(&self, block: &signet_storage::ExecutedBlock) {
        let notif = NewBlockNotification {
            header: block.header.inner().clone(),
            transactions: block.transactions.iter().map(|tx| tx.inner().clone()).collect(),
            receipts: block.receipts.clone(),
        };
        // Ignore send errors — no subscribers is fine.
        let _ = self.notif_tx.send(notif);
    }

    /// Update the status channel and block tags. This keeps the RPC node
    /// in sync with the latest block information.
    fn update_status(&self) -> eyre::Result<()> {
        let reader = self.storage.reader()?;
        let ru_height = HistoryRead::last_block_number(&reader)?.unwrap_or(0);
        drop(reader);

        self.update_block_tags(ru_height)?;
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(ru_height));
        Ok(())
    }

    /// Update block tags (latest/safe/finalized) and notify reth of processed
    /// height.
    fn update_block_tags(&self, ru_height: u64) -> eyre::Result<()> {
        // Safe height
        let safe_heights = self.load_safe_block_heights(ru_height)?;
        let safe_ru_height = safe_heights.rollup;
        debug!(safe_ru_height, "calculated safe ru height");

        // Finalized height
        let finalized_heights = self.load_finalized_block_heights(ru_height)?;
        debug!(
            finalized_host_height = finalized_heights.host,
            finalized_ru_height = finalized_heights.rollup,
            "calculated finalized heights"
        );

        // Atomically update all three tags
        self.tags.update_all(ru_height, safe_ru_height, finalized_heights.rollup);

        // Notify reth that we've finished processing up to the finalized
        // height. Skip if finalized is still at genesis.
        if finalized_heights.host != 0 || finalized_heights.rollup != 0 {
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

    /// Load the host chain "safe" block number and determine the rollup "safe"
    /// block number.
    ///
    /// There are three cases:
    /// 1. The host chain "safe" block number is below the rollup genesis.
    /// 2. The safe rollup equivalent is beyond the current rollup height.
    /// 3. The safe rollup equivalent is below the current rollup height (normal
    ///    case).
    fn load_safe_block_heights(&self, ru_height: u64) -> eyre::Result<PairedHeights> {
        let safe_host_height = self.host.provider().safe_block_number()?;

        let safe_heights = safe_host_height
            .and_then(|safe_host_height| self.constants.pair_host(safe_host_height));

        safe_heights.map_or(Ok(PairedHeights { host: 0, rollup: 0 }), |safe_heights| {
            if safe_heights.rollup > ru_height {
                Ok(PairedHeights {
                    host: self.constants.rollup_block_to_host_block_num(ru_height),
                    rollup: ru_height,
                })
            } else {
                Ok(safe_heights)
            }
        })
    }

    /// Load the host chain "finalized" block number and determine the rollup
    /// "finalized" block number.
    ///
    /// There are three cases:
    /// 1. The host chain "finalized" block is below the rollup genesis.
    /// 2. The finalized rollup equivalent is beyond the current rollup height.
    /// 3. The finalized rollup equivalent is below the current rollup height
    ///    (normal case).
    fn load_finalized_block_heights(&self, ru_height: u64) -> eyre::Result<PairedHeights> {
        let finalized_host_block_number = self.host.provider().finalized_block_number()?;

        let finalized_ru_block_number =
            finalized_host_block_number.and_then(|finalized_host_block_number| {
                self.constants.host_block_to_rollup_block_num(finalized_host_block_number)
            });

        finalized_ru_block_number.map_or(
            Ok(PairedHeights { host: 0, rollup: 0 }),
            |finalized_ru_block_number| {
                if finalized_ru_block_number > ru_height {
                    Ok(self.constants.pair_ru(ru_height))
                } else {
                    Ok(self.constants.pair_ru(finalized_ru_block_number))
                }
            },
        )
    }

    /// Update the host node with the highest processed host height for the
    /// ExEx.
    fn update_highest_processed_height(&self, finalized_host_height: u64) -> eyre::Result<()> {
        let adjusted_height = finalized_host_height.saturating_sub(1);
        let adjusted_header = self
            .host
            .provider()
            .sealed_header(adjusted_height)?
            .expect("db inconsistent. no host header for adjusted height");

        let hash = adjusted_header.hash();

        debug!(finalized_host_height = adjusted_height, "Sending FinishedHeight notification");
        self.host.events.send(ExExEvent::FinishedHeight(alloy::eips::NumHash {
            number: adjusted_height,
            hash,
        }))?;
        Ok(())
    }

    /// Called when the host chain has reverted a block or set of blocks.
    #[instrument(skip_all, fields(first = chain.first().number(), tip = chain.tip().number()))]
    pub fn on_host_revert(&self, chain: &Arc<Chain<Host>>) -> eyre::Result<()> {
        // If the end is before the RU genesis, nothing to do.
        if chain.tip().number() <= self.constants.host_deploy_height() {
            return Ok(());
        }

        // Target is the block BEFORE the first block in the chain, or 0.
        let target = self
            .constants
            .host_block_to_rollup_block_num(chain.first().number())
            .unwrap_or_default()
            .saturating_sub(1);

        self.storage.unwind_above(target)?;
        Ok(())
    }
}
