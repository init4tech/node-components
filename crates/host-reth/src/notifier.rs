use crate::{
    RethChain,
    config::{rpc_config_from_args, serve_config_from_args},
    error::RethHostError,
};
use alloy::{consensus::BlockHeader, eips::BlockNumHash};
use futures_util::StreamExt;
use reth::{
    chainspec::EthChainSpec,
    primitives::EthPrimitives,
    providers::{BlockIdReader, HeaderProvider},
};
use reth_exex::{ExExContext, ExExEvent, ExExNotifications, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_stages_types::ExecutionStageThresholds;
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier, RevertRange};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use std::sync::Arc;
use tracing::{debug, error};

/// Reth ExEx implementation of [`HostNotifier`].
///
/// Wraps reth's notification stream, provider, and event sender. All hash
/// resolution happens internally — consumers only work with block numbers.
pub struct RethHostNotifier<Host: FullNodeComponents> {
    notifications: ExExNotifications<Host::Provider, Host::Evm>,
    provider: Host::Provider,
    events: tokio::sync::mpsc::UnboundedSender<ExExEvent>,
}

impl<Host: FullNodeComponents> core::fmt::Debug for RethHostNotifier<Host> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RethHostNotifier").finish_non_exhaustive()
    }
}

/// The output of [`decompose_exex_context`].
pub struct DecomposedContext<Host: FullNodeComponents> {
    /// The host notifier adapter.
    pub notifier: RethHostNotifier<Host>,
    /// Plain RPC serve config.
    pub serve_config: ServeConfig,
    /// Plain RPC storage config.
    pub rpc_config: StorageRpcConfig,
    /// The transaction pool, for blob cacher construction.
    pub pool: Host::Pool,
    /// The chain name, for tracing.
    pub chain_name: String,
}

impl<Host: FullNodeComponents> core::fmt::Debug for DecomposedContext<Host> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecomposedContext")
            .field("chain_name", &self.chain_name)
            .finish_non_exhaustive()
    }
}

/// Decompose a reth [`ExExContext`] into a [`RethHostNotifier`] and
/// associated configuration values.
///
/// This is the primary entry point for integrating with reth's ExEx
/// framework. Typical usage in an ExEx `init` function:
///
/// ```ignore
/// # // Requires ExExContext — shown for API illustration only.
/// use signet_host_reth::decompose_exex_context;
///
/// async fn init(ctx: ExExContext<Node>) -> eyre::Result<()> {
///     let decomposed = decompose_exex_context(ctx);
///     // decomposed.notifier implements HostNotifier
///     // decomposed.serve_config / rpc_config are reth-free
///     // decomposed.pool is the transaction pool handle
///     Ok(())
/// }
/// ```
///
/// This splits the ExEx context into:
/// - A [`RethHostNotifier`] (implements [`HostNotifier`])
/// - A [`ServeConfig`] (plain RPC server config)
/// - A [`StorageRpcConfig`] (gas oracle settings)
/// - The transaction pool handle
/// - A chain name for tracing
pub fn decompose_exex_context<Host>(ctx: ExExContext<Host>) -> DecomposedContext<Host>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
{
    let chain_name = ctx.config.chain.chain().to_string();
    let serve_config = serve_config_from_args(&ctx.config.rpc);
    let rpc_config = rpc_config_from_args(&ctx.config.rpc);
    let pool = ctx.pool().clone();
    let provider = ctx.provider().clone();

    let notifier =
        RethHostNotifier { notifications: ctx.notifications, provider, events: ctx.events };

    DecomposedContext { notifier, serve_config, rpc_config, pool, chain_name }
}

impl<Host> HostNotifier for RethHostNotifier<Host>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
{
    type Chain = RethChain;
    type Error = RethHostError;

    async fn next_notification(
        &mut self,
    ) -> Option<Result<HostNotification<Self::Chain>, Self::Error>> {
        let notification = self.notifications.next().await?;
        let notification = match notification {
            Ok(n) => n,
            Err(e) => return Some(Err(RethHostError::notification(e))),
        };

        // Read safe/finalized from the provider at notification time.
        let safe_block_number = self
            .provider
            .safe_block_number()
            .inspect_err(|e| {
                debug!(%e, "failed to read safe block number from provider");
            })
            .ok()
            .flatten();
        let finalized_block_number = self
            .provider
            .finalized_block_number()
            .inspect_err(|e| {
                debug!(%e, "failed to read finalized block number from provider");
            })
            .ok()
            .flatten();

        let kind = match notification {
            reth_exex::ExExNotification::ChainCommitted { new } => {
                HostNotificationKind::ChainCommitted { new: Arc::new(RethChain::new(new)) }
            }
            reth_exex::ExExNotification::ChainReverted { old } => {
                let old = RevertRange::new(old.first().number(), old.tip().number());
                HostNotificationKind::ChainReverted { old }
            }
            reth_exex::ExExNotification::ChainReorged { old, new } => {
                let old = RevertRange::new(old.first().number(), old.tip().number());
                HostNotificationKind::ChainReorged { old, new: Arc::new(RethChain::new(new)) }
            }
        };

        Some(Ok(HostNotification { kind, safe_block_number, finalized_block_number }))
    }

    fn set_head(&mut self, block_number: u64) {
        let head = self
            .provider
            .sealed_header(block_number)
            .inspect_err(|e| error!(block_number, %e, "failed to look up header for set_head"))
            .expect("failed to look up header for set_head")
            .map(|h| BlockNumHash { number: block_number, hash: h.hash() })
            .unwrap_or_else(|| {
                debug!(block_number, "header not found for set_head, falling back to genesis");
                let genesis = self
                    .provider
                    .sealed_header(0)
                    .inspect_err(|e| error!(%e, "failed to look up genesis header"))
                    .expect("failed to look up genesis header")
                    .expect("genesis header missing");
                BlockNumHash { number: 0, hash: genesis.hash() }
            });

        self.notifications.set_with_head(reth_exex::ExExHead { block: head });
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        let thresholds = match max_blocks {
            Some(max_blocks) => {
                debug!(max_blocks, "configured backfill thresholds");
                ExecutionStageThresholds { max_blocks: Some(max_blocks), ..Default::default() }
            }
            None => {
                debug!("reset backfill thresholds to defaults");
                ExecutionStageThresholds::default()
            }
        };
        self.notifications.set_backfill_thresholds(thresholds);
    }

    fn send_finished_height(&self, block_number: u64) -> Result<(), Self::Error> {
        let header = self
            .provider
            .sealed_header(block_number)?
            .ok_or(RethHostError::MissingHeader(block_number))?;

        let hash = header.hash();
        self.events.send(ExExEvent::FinishedHeight(BlockNumHash { number: block_number, hash }))?;
        Ok(())
    }
}
