use crate::metrics;
use alloy::{
    consensus::BlockHeader,
    eips::NumHash,
    primitives::{B256, BlockNumber, b256},
};
use eyre::Context;
use futures_util::StreamExt;
use reth::{
    primitives::EthPrimitives,
    providers::{
        BlockIdReader, BlockNumReader, BlockReader, CanonChainTracker, CanonStateNotification,
        CanonStateNotifications, CanonStateSubscriptions, HeaderProvider, NodePrimitivesProvider,
        ProviderFactory, StateProviderFactory, providers::BlockchainProvider,
    },
    rpc::types::engine::ForkchoiceState,
};
use reth_chainspec::EthChainSpec;
use reth_exex::{ExExContext, ExExEvent, ExExHead, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, FullNodeTypes, NodeTypes};
use signet_blobber::BlobFetcher;
use signet_block_processor::{AliasOracleFactory, SignetBlockProcessorV1};
use signet_db::{DbProviderExt, ProviderConsistencyExt, RuChain, RuWriter};
use signet_node_config::SignetNodeConfig;
use signet_node_types::{NodeStatus, NodeTypesDbTrait, SignetNodeTypes};
use signet_rpc::RpcServerGuard;
use signet_types::{PairedHeights, constants::SignetSystemConstants};
use std::{fmt, mem::MaybeUninit, sync::Arc};
use tokio::sync::watch;
use tracing::{debug, info, instrument};

/// The genesis journal hash for the signet chain.
pub const GENESIS_JOURNAL_HASH: B256 =
    b256!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

/// Make it easier to write some args
type PrimitivesOf<Host> = <<Host as FullNodeTypes>::Types as NodeTypes>::Primitives;
type ExExNotification<Host> = reth_exex::ExExNotification<PrimitivesOf<Host>>;
type Chain<Host> = reth::providers::Chain<PrimitivesOf<Host>>;

/// Signet context and configuration.
pub struct SignetNode<Host, Db, AliasOracle = Box<dyn StateProviderFactory>>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
{
    /// The host context, which manages provider access and notifications.
    pub(crate) host: ExExContext<Host>,

    /// Signet node configuration.
    pub(crate) config: Arc<SignetNodeConfig>,

    /// A [`ProviderFactory`] instance to allow RU database access.
    pub(crate) ru_provider: ProviderFactory<SignetNodeTypes<Db>>,

    /// A [`BlockchainProvider`] instance. Used to notify the RPC server of
    /// changes to the canonical/safe/finalized head.
    pub(crate) bp: BlockchainProvider<SignetNodeTypes<Db>>,

    /// The join handle for the RPC server. None if the RPC server is not
    /// yet running.
    pub(crate) rpc_handle: Option<RpcServerGuard>,

    /// Chain configuration constants.
    pub(crate) constants: SignetSystemConstants,

    /// Status channel, currently used only for testing
    pub(crate) status: watch::Sender<NodeStatus>,

    /// The block processor
    pub(crate) processor: SignetBlockProcessorV1<Db, AliasOracle>,

    /// A reqwest client, used by the blob fetch and the tx cache forwarder.
    pub(crate) client: reqwest::Client,
}

impl<Host, Db, AliasOracle> fmt::Debug for SignetNode<Host, Db, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignetNode").field("config", &self.config).finish_non_exhaustive()
    }
}

impl<Host, Db, AliasOracle> NodePrimitivesProvider for SignetNode<Host, Db, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
{
    type Primitives = EthPrimitives;
}

impl<Host, Db, AliasOracle> CanonStateSubscriptions for SignetNode<Host, Db, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
    AliasOracle: AliasOracleFactory,
{
    fn subscribe_to_canonical_state(&self) -> CanonStateNotifications<Self::Primitives> {
        self.bp.subscribe_to_canonical_state()
    }
}

impl<Host, Db, AliasOracle> SignetNode<Host, Db, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
    AliasOracle: AliasOracleFactory,
{
    /// Create a new Signet instance. It is strongly recommend that you use the
    /// [`SignetNodeBuilder`] instead of this function.
    ///
    /// This function does NOT initialize the genesis state. As such it is NOT
    /// safe to use directly. The genesis state in the `factory` MUST be
    /// initialized BEFORE calling this function.
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
        factory: ProviderFactory<SignetNodeTypes<Db>>,
        alias_oracle: AliasOracle,
        client: reqwest::Client,
    ) -> eyre::Result<(Self, tokio::sync::watch::Receiver<NodeStatus>)> {
        let constants =
            config.constants().wrap_err("failed to load signet constants from genesis")?;

        let bp: BlockchainProvider<SignetNodeTypes<Db>> = BlockchainProvider::new(factory.clone())?;

        let (status, receiver) = tokio::sync::watch::channel(NodeStatus::Booting);

        let blob_cacher = BlobFetcher::builder()
            .with_config(config.block_extractor())?
            .with_pool(ctx.pool().clone())
            .with_client(client.clone())
            .build_cache()
            .wrap_err("failed to create blob cacher")?
            .spawn();

        let processor = SignetBlockProcessorV1::new(
            constants.clone(),
            config.chain_spec().clone(),
            factory.clone(),
            alias_oracle,
            config.slot_calculator(),
            blob_cacher,
        );

        let this = Self {
            config: config.into(),
            host: ctx,
            ru_provider: factory.clone(),
            bp,

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
        if let Some(height) = self.ru_provider.ru_check_consistency()? {
            self.unwind_to(height).wrap_err("failed to unwind RU database to consistent state")?;
        }

        // This exists only to bypass the `tracing::instrument(err)` macro to
        // ensure that full sources get reported.
        self.start_inner().await.inspect_err(|err| {
            // using `:#` invokes the alternate formatter, which for eyre
            // includes cause reporting.
            let err = format!("{err:#}");

            let last_block = self.ru_provider.last_block_number().ok();
            let exex_head = last_block.and_then(|h| self.set_exex_head(h).ok());

            tracing::error!(err, last_block, ?exex_head, "Signet node crashed");
        })
    }

    /// Start the Signet instance, listening for ExEx notifications.
    async fn start_inner(&mut self) -> eyre::Result<()> {
        debug!(constants = ?self.constants, "signet starting");

        self.start_rpc().await?;

        // Determine the last block written to the database for backfill
        let last_rollup_block: u64 = self.ru_provider.last_block_number()?;

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

    /// Sets the head of the Exex chain from the last rollup block, handling genesis conditions if necessary.
    fn set_exex_head(&mut self, last_rollup_block: u64) -> eyre::Result<ExExHead> {
        // If the last rollup block is 0, we can shortcut and just set the head to the host rollup deployment block.
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
                    let genesis_block = self.host.provider().block_by_number(0)?;
                    match genesis_block {
                        Some(genesis_block) => {
                            let exex_head = ExExHead { block: genesis_block.num_hash_slow() };
                            self.host.notifications.set_with_head(exex_head);
                            return Ok(exex_head);
                        }
                        None => panic!("failed to find genesis block"),
                    }
                }
            }
        }

        // If the last rollup block is not 0, we need to find the corresponding host block.
        // We do this by looking up the host block number for the rollup block number, and then
        // looking up the host block for that number.
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
                let genesis_block = self.host.provider().block_by_number(0)?;
                match genesis_block {
                    Some(genesis_block) => {
                        let exex_head = ExExHead { block: genesis_block.num_hash_slow() };
                        self.host.notifications.set_with_head(exex_head);
                        Ok(exex_head)
                    }
                    None => panic!("failed to find genesis block"),
                }
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

        // NB: REVERTS MUST RUN FIRST
        let mut reverted = None;
        if let Some(chain) = notification.reverted_chain() {
            reverted = self.on_host_revert(&chain).wrap_err("error encountered during revert")?;
        }

        let mut committed = None;
        if let Some(chain) = notification.committed_chain() {
            committed = self
                .processor
                .on_host_commit::<Host>(&chain)
                .await
                .wrap_err("error encountered during commit")?;
        }

        if committed.is_some() || reverted.is_some() {
            // Update the status channel and canon heights, etc.
            self.update_status(committed, reverted)?;
        }

        metrics::record_notification_processed(&notification);
        Ok(())
    }

    /// Update the status channel and the latest block info. This is necessary
    /// to keep the RPC node in sync with the latest block information.
    fn update_status(
        &self,
        committed: Option<RuChain>,
        reverted: Option<RuChain>,
    ) -> eyre::Result<()> {
        let ru_height = self.ru_provider.last_block_number()?;

        // Update the RPC's block information
        self.update_canon_heights(ru_height)?;

        // We'll also emit the new chains as notifications on our canonstate
        // notification channel, provided anyone is listening
        self.update_canon_state(committed, reverted);

        // Update the status channel. This is used by the test-utils to watch
        // notification processing, and may be removed in the future.
        self.status.send_modify(|s| *s = NodeStatus::AtHeight(ru_height));

        Ok(())
    }

    /// Update the canonical heights of the chain. This does two main things
    /// - Update the RPC server's view of the forkchoice rule, setting the
    ///   tip and block labels
    /// - Update the reth node that the ExEx has finished processing blocks up
    ///   to the finalized block.
    ///
    /// This is used by the RPC to resolve block tags including "latest",
    /// "safe", and "finalized", as well as the number returned by
    /// `eth_blockNumber`.
    fn update_canon_heights(&self, ru_height: u64) -> eyre::Result<()> {
        // Set the canonical head ("latest" label)
        let latest_ru_block_header = self
            .ru_provider
            .sealed_header(ru_height)?
            .expect("ru db inconsistent. no header for height");
        let latest_ru_block_hash = latest_ru_block_header.hash();
        self.bp.set_canonical_head(latest_ru_block_header);

        // This is our fallback safe and finalized, in case the host chain
        // hasn't finalized more recent blocks
        let genesis_ru_hash = self
            .ru_provider
            .sealed_header(0)?
            .expect("ru db inconsistent. no header for height")
            .hash();

        // Load the safe block hash for both the host and the rollup.
        // The safe block height of the rollup CANNOT be higher than the latest ru height,
        // as we've already processed all the blocks up to the latest ru height.
        let PairedHeights { host: _, rollup: safe_ru_height } =
            self.load_safe_block_heights(ru_height)?;
        let safe_ru_block_header = self
            .ru_provider
            .sealed_header(safe_ru_height)?
            .expect("ru db inconsistent. no header for height");
        let safe_ru_block_hash = safe_ru_block_header.hash();

        debug!(safe_ru_height, "calculated safe ru height");

        // Update the safe rollup block hash iff it's not the genesis rollup block.
        if safe_ru_block_hash != genesis_ru_hash {
            self.bp.set_safe(safe_ru_block_header);
        }

        // Load the finalized rollup block hash.
        // The finalized rollup block height CANNOT be higher than the latest ru height,
        // as we've already processed all the blocks up to the latest ru height.
        let finalized_heights = self.load_finalized_block_heights(ru_height)?;

        debug!(
            finalized_host_height = finalized_heights.host,
            finalized_ru_height = finalized_heights.rollup,
            "calculated finalized heights"
        );

        // Load the finalized RU block hash. It's the genesis hash if the host
        // and rollup finalized heights are both 0. Otherwise, we load the finalized
        // RU header and set the finalized block hash.
        let finalized_ru_block_hash =
            self.set_finalized_ru_block_hash(finalized_heights, genesis_ru_hash)?;

        // NB:
        // We also need to notify the reth node that we are totally
        // finished processing the host block before the finalized block now.
        // We want to keep the finalized host block in case we reorg to the block
        // immediately on top of it, and we need some state from the parent.
        //
        // If this errors, it means that the reth node has shut down and we
        // should stop processing blocks.
        //
        // To do this, we grab the finalized host header to get its height and hash,
        // so we can send the corresponding [`ExExEvent`].
        if finalized_ru_block_hash != genesis_ru_hash {
            self.update_highest_processed_height(finalized_heights.host)?;
        }

        // Update the RPC's forkchoice timestamp.
        self.bp.on_forkchoice_update_received(&ForkchoiceState {
            head_block_hash: latest_ru_block_hash,
            safe_block_hash: safe_ru_block_hash,
            finalized_block_hash: finalized_ru_block_hash,
        });
        debug!(
            %latest_ru_block_hash, %safe_ru_block_hash, %finalized_ru_block_hash,
            "updated RPC block producer"
        );
        Ok(())
    }

    /// Update the ExEx head to the finalized host block.
    ///
    /// If this errors, it means that the reth node has shut down and we
    /// should stop processing blocks.
    fn update_exex_head(
        &self,
        finalized_host_height: u64,
        finalized_host_hash: B256,
    ) -> eyre::Result<()> {
        debug!(finalized_host_height, "Sending FinishedHeight notification");
        self.host.events.send(ExExEvent::FinishedHeight(NumHash {
            number: finalized_host_height,
            hash: finalized_host_hash,
        }))?;
        Ok(())
    }

    /// Send a canon state notification via the channel.
    fn update_canon_state(&self, committed: Option<RuChain>, reverted: Option<RuChain>) {
        let commit_count = committed.as_ref().map(|c| c.len()).unwrap_or_default();
        let revert_count = reverted.as_ref().map(|r| r.len()).unwrap_or_default();

        let notif = match (committed, reverted) {
            (None, None) => None,
            (None, Some(r)) => Some(CanonStateNotification::Reorg {
                old: Arc::new(r.inner),
                new: Arc::new(Default::default()),
            }),
            (Some(c), None) => Some(CanonStateNotification::Commit { new: Arc::new(c.inner) }),
            (Some(c), Some(r)) => Some(CanonStateNotification::Reorg {
                old: Arc::new(r.inner),
                new: Arc::new(c.inner),
            }),
        };
        if let Some(notif) = notif {
            tracing::debug!(commit_count, revert_count, "sending canon state notification");
            // we don't care if it fails, we just want to send it
            self.bp.canonical_in_memory_state().notify_canon_state(notif);
        }
    }

    /// Load the host chain "safe" block number and determine the rollup "safe"
    /// block number. There are three cases:
    ///
    /// 1. The host chain "safe" block number is below the rollup genesis.
    ///    In this case, we'll use the host genesis block number as the "safe"
    ///    block number. This can happen if the rollup starts syncing while the
    ///    host still hasn't seen the rollup genesis block.
    /// 2. The host safe "block", when converted to the equivalent rollup block,
    ///    is beyond the current rollup block. In this case, we'll use the current
    ///    rollup block as safe block. This can happen if the host chain is
    ///    synced beyond the current rollup block, but the rollup is still syncing
    ///    and catching up with the host head and therefore hasn't seen the host
    ///    safe block.
    /// 3. The host safe block number is below the current rollup block. In this
    ///    case, we can use the safe host block number, converted to its rollup
    ///    equivalent, as the safe rollup block number. This is the expected case
    ///    when the rollup and host are both caught up and in live sync.
    fn load_safe_block_heights(&self, ru_height: u64) -> eyre::Result<PairedHeights> {
        // Load the host safe block number
        let safe_host_height = self.host.provider().safe_block_number()?;

        // Convert the host safe block number to the rollup safe block number.
        // If the host safe block number is below the rollup genesis,
        // this will return None.
        let safe_heights = safe_host_height
            .and_then(|safe_host_height| self.constants.pair_host(safe_host_height));

        // If we successfully converted the host safe block number to the rollup safe block number,
        // then we'll compare it to the current rollup block height and use the smaller of the two.
        if let Some(safe_heights) = safe_heights {
            // We compare the safe ru height to the current ru height. If the safe ru height is
            // beyond the current ru height, we're in case 2.
            if safe_heights.rollup > ru_height {
                // We are in case 2.
                Ok(PairedHeights {
                    host: self.constants.rollup_block_to_host_block_num(ru_height),
                    rollup: ru_height,
                })
            } else {
                // If the safe ru height is below the current ru height, we're in case 3.
                Ok(safe_heights)
            }
        } else {
            // If the host safe block number is below the rollup genesis,
            // we'll use the host genesis block number as the "safe" block number.
            Ok(PairedHeights { host: 0, rollup: 0 })
        }
    }

    /// Set the finalized RU block hash.
    ///
    /// Depending on the current rollup sync status, there are two cases:
    /// 1. If we're syncing from scratch, we'll set the finalized RU block hash to the genesis hash.
    /// 2. If we're syncing, or following the tip, we'll set the finalized RU block hash to the current RU block hash.
    fn set_finalized_ru_block_hash(
        &self,
        finalized_heights: PairedHeights,
        genesis_hash: B256,
    ) -> eyre::Result<B256> {
        // If both heights are 0, return genesis hash
        if finalized_heights.host == 0 && finalized_heights.rollup == 0 {
            return Ok(genesis_hash);
        }

        // Load and set finalized RU header
        let finalized_ru_header = self
            .ru_provider
            .sealed_header(finalized_heights.rollup)?
            .expect("ru db inconsistent. no header for height");
        let finalized_ru_block_hash = finalized_ru_header.hash();
        self.bp.set_finalized(finalized_ru_header);

        Ok(finalized_ru_block_hash)
    }

    /// Update the host node with the highest processed host height for the exex.
    fn update_highest_processed_height(&self, finalized_host_height: u64) -> eyre::Result<()> {
        let finalized_host_header = self
            .host
            .provider()
            .sealed_header(finalized_host_height)?
            .expect("db inconsistent. no host header for finalized height");

        let adjusted_height = finalized_host_header.number.saturating_sub(1);
        let hash = finalized_host_header.hash();

        debug!(finalized_host_height = adjusted_height, "Sending FinishedHeight notification");
        self.update_exex_head(adjusted_height, hash)
    }

    /// Load the host chain "finalized" block number and determine the rollup
    /// "finalized" block number. If the host chain "finalized" block number is below the
    /// rollup genesis, we'll use the genesis hash as the "finalized" block.
    /// If the host chain "finalized" block number is beyond the current rollup block,
    /// we'll use the current rollup block and its host equivalent as the "finalized" blocks.
    ///
    /// This returns a tuple of the host and rollup "finalized" block numbers.
    ///
    /// There are three cases:
    /// 1. The host chain "finalized" block number is below the rollup genesis (and therefore the current rollup block).
    ///    In this case, we'll use the host genesis block number as the "finalized" block number, with the rollup syncing from scratch.
    ///    This can happen if the rollup starts syncing while the host still hasn't seen the rollup genesis block.
    /// 2. The host chain "finalized" block number is beyond the current rollup block.
    ///    In this case, we'll use the current rollup block number as the "finalized" block number.
    ///    This can happen if the host chain is synced beyond the current rollup block, but the rollup is still syncing
    ///    and catching up with the host head and therefore hasn't seen the host finalized block.
    /// 3. The host chain "finalized" block number is below the current rollup block.
    ///    In this case, we'll use the host chain "finalized" block number, converted to its rollup equivalent, as the "finalized" block number.
    ///    This is the expected case when the rollup and host are both caught up and in live sync.
    fn load_finalized_block_heights(&self, ru_height: u64) -> eyre::Result<PairedHeights> {
        // Load the host chain "finalized" block number
        let finalized_host_block_number = self.host.provider().finalized_block_number()?;

        // Convert the host chain "finalized" block number to the rollup "finalized" block number.
        // If the host chain "finalized" block number is below the rollup genesis,
        // this will return None.
        let finalized_ru_block_number =
            finalized_host_block_number.and_then(|finalized_host_block_number| {
                self.constants.host_block_to_rollup_block_num(finalized_host_block_number)
            });

        // If we successfully converted the host chain "finalized" block number to the rollup "finalized" block number,
        // then we'll figure out which case we're in and return the appropriate heights.
        if let Some(finalized_ru_block_number) = finalized_ru_block_number {
            // We compare the finalized ru height to the current ru height. If the finalized ru height is
            // beyond the current ru height, we're in case 2 (rollup is behind host).
            if finalized_ru_block_number > ru_height {
                Ok(self.constants.pair_ru(ru_height))
            } else {
                // If the finalized ru height is below the current ru height, we're in case 3 (rollup is near or in sync with the host head).
                Ok(self.constants.pair_ru(finalized_ru_block_number))
            }
        } else {
            // If we failed to convert the host chain "finalized" block number to the rollup "finalized" block number,
            // then this means the host chain "finalized" block number is below the rollup genesis (and therefore the current rollup block).
            // We'll use the genesis block number as the "finalized" block number.
            Ok(PairedHeights { host: 0, rollup: 0 })
        }
    }

    /// Unwind the RU chain DB to the target block number.
    fn unwind_to(&self, target: BlockNumber) -> eyre::Result<RuChain> {
        let mut reverted = MaybeUninit::uninit();
        self.ru_provider
            .provider_rw()?
            .update(|writer| {
                reverted.write(writer.ru_take_blocks_and_execution_above(target)?);
                Ok(())
            })
            // SAFETY: if the closure above returns Ok, reverted is initialized.
            .map(|_| unsafe { reverted.assume_init() })
            .map_err(Into::into)
    }

    /// Called when the host chain has reverted a block or set of blocks.
    #[instrument(skip_all, fields(first = chain.first().number(), tip = chain.tip().number()))]
    pub fn on_host_revert(&self, chain: &Arc<Chain<Host>>) -> eyre::Result<Option<RuChain>> {
        // If the end is before the RU genesis, we don't need to do anything at
        // all.
        if chain.tip().number() <= self.constants.host_deploy_height() {
            return Ok(None);
        }

        // The target is
        // - the block BEFORE the first block in the chain
        // - or block 0, if the first block is before the rollup deploy height
        let target = self
            .constants
            .host_block_to_rollup_block_num(chain.first().number())
            .unwrap_or_default() // 0 if the block is before the deploy height
            .saturating_sub(1); // still 0 if 0, otherwise the block BEFORE.

        self.unwind_to(target).map(Some)
    }
}
