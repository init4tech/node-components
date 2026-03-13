use crate::{
    RethChain,
    config::{rpc_config_from_args, serve_config_from_args},
    error::RethHostError,
};
use alloy::eips::BlockNumHash;
use futures_util::StreamExt;
use reth::{
    chainspec::EthChainSpec,
    primitives::EthPrimitives,
    providers::{BlockIdReader, BlockReader, HeaderProvider},
};
use reth_exex::{ExExContext, ExExEvent, ExExNotifications, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, NodeTypes};
use reth_stages_types::ExecutionStageThresholds;
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use std::sync::Arc;
use tracing::debug;

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
            Err(e) => return Some(Err(e.into())),
        };

        // Read safe/finalized from the provider at notification time.
        let safe_block_number = self.provider.safe_block_number().ok().flatten();
        let finalized_block_number = self.provider.finalized_block_number().ok().flatten();

        let kind = match notification {
            reth_exex::ExExNotification::ChainCommitted { new } => {
                HostNotificationKind::ChainCommitted { new: Arc::new(RethChain::new(new)) }
            }
            reth_exex::ExExNotification::ChainReverted { old } => {
                HostNotificationKind::ChainReverted { old: Arc::new(RethChain::new(old)) }
            }
            reth_exex::ExExNotification::ChainReorged { old, new } => {
                HostNotificationKind::ChainReorged {
                    old: Arc::new(RethChain::new(old)),
                    new: Arc::new(RethChain::new(new)),
                }
            }
        };

        Some(Ok(HostNotification { kind, safe_block_number, finalized_block_number }))
    }

    fn set_head(&mut self, block_number: u64) {
        let block = self
            .provider
            .block_by_number(block_number)
            .expect("failed to look up block for set_head");

        let head = match block {
            Some(b) => b.num_hash_slow(),
            None => {
                debug!(block_number, "block not found for set_head, falling back to genesis");
                let genesis = self
                    .provider
                    .block_by_number(0)
                    .expect("failed to look up genesis block")
                    .expect("genesis block missing");
                genesis.num_hash_slow()
            }
        };

        let exex_head = reth_exex::ExExHead { block: head };
        self.notifications.set_with_head(exex_head);
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        if let Some(max_blocks) = max_blocks {
            self.notifications.set_backfill_thresholds(ExecutionStageThresholds {
                max_blocks: Some(max_blocks),
                ..Default::default()
            });
            debug!(max_blocks, "configured backfill thresholds");
        }
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
