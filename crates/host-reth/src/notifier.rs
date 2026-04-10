use crate::{
    backfill::DbBackfill,
    chain::HostChain,
    config::{rpc_config_from_args, serve_config_from_args},
    error::RethHostError,
};
use alloy::{consensus::BlockHeader, eips::BlockNumHash};
use futures_util::StreamExt;
use reth::{
    chainspec::EthChainSpec,
    primitives::{EthPrimitives, Receipt},
    providers::{BlockIdReader, BlockReader, HeaderProvider, ReceiptProvider},
};
use reth_exex::{ExExContext, ExExEvent, ExExNotifications, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier, RevertRange};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use signet_types::primitives::TransactionSigned;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Reth ExEx implementation of [`HostNotifier`].
///
/// Wraps reth's notification stream, provider, and event sender. All hash
/// resolution happens internally — consumers only work with block numbers.
pub struct RethHostNotifier<Host: FullNodeComponents> {
    notifications: ExExNotifications<Host::Provider, Host::Evm>,
    provider: Host::Provider,
    events: tokio::sync::mpsc::UnboundedSender<ExExEvent>,
    backfill: Option<DbBackfill<Host::Provider>>,
    head_set: bool,
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

    let notifier = RethHostNotifier {
        notifications: ctx.notifications,
        provider,
        events: ctx.events,
        backfill: None,
        head_set: false,
    };

    DecomposedContext { notifier, serve_config, rpc_config, pool, chain_name }
}

impl<Host> HostNotifier for RethHostNotifier<Host>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Host::Provider: BlockReader<Block = alloy::consensus::Block<TransactionSigned>>
        + ReceiptProvider<Receipt = Receipt>,
{
    type Chain = HostChain;
    type Error = RethHostError;

    async fn next_notification(
        &mut self,
    ) -> Option<Result<HostNotification<Self::Chain>, Self::Error>> {
        // Phase 1: DB backfill — drain batches until complete.
        if let Some(backfill) = &mut self.backfill {
            match backfill.next_batch().await {
                Ok(Some(segment)) => {
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

                    return Some(Ok(HostNotification {
                        kind: HostNotificationKind::ChainCommitted {
                            new: Arc::new(HostChain::Backfill(segment)),
                        },
                        safe_block_number,
                        finalized_block_number,
                    }));
                }
                Ok(None) => {
                    // Backfill complete. The cursor points to the next block
                    // to read, so the last backfilled block is cursor - 1.
                    let backfill = self.backfill.take().expect("backfill was Some");
                    let last_backfilled = backfill.cursor().saturating_sub(1);

                    let head = self
                        .provider
                        .sealed_header(last_backfilled)
                        .inspect_err(|e| {
                            error!(last_backfilled, %e, "failed to look up header after backfill");
                        })
                        .expect("failed to look up header after backfill")
                        .map(|h| BlockNumHash { number: last_backfilled, hash: h.hash() })
                        .unwrap_or_else(|| {
                            debug!(
                                last_backfilled,
                                "header not found after backfill, falling back to genesis"
                            );
                            let genesis = self
                                .provider
                                .sealed_header(0)
                                .inspect_err(|e| error!(%e, "failed to look up genesis header"))
                                .expect("failed to look up genesis header")
                                .expect("genesis header missing");
                            BlockNumHash { number: 0, hash: genesis.hash() }
                        });

                    info!(
                        last_backfilled,
                        head_number = head.number,
                        "DB backfill complete, switching to live ExEx notifications"
                    );

                    self.notifications.set_with_head(reth_exex::ExExHead { block: head });
                    // Fall through to phase 2.
                }
                Err(e) => return Some(Err(e)),
            }
        }

        // Phase 2: live ExEx notifications.
        let notification = self.notifications.next().await?;
        let notification = match notification {
            Ok(n) => n,
            Err(e) => return Some(Err(RethHostError::notification(e))),
        };

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
                HostNotificationKind::ChainCommitted {
                    new: Arc::new(HostChain::Live(crate::RethChain::new(new))),
                }
            }
            reth_exex::ExExNotification::ChainReverted { old } => {
                let old = RevertRange::new(old.first().number(), old.tip().number());
                HostNotificationKind::ChainReverted { old }
            }
            reth_exex::ExExNotification::ChainReorged { old, new } => {
                let old = RevertRange::new(old.first().number(), old.tip().number());
                HostNotificationKind::ChainReorged {
                    old,
                    new: Arc::new(HostChain::Live(crate::RethChain::new(new))),
                }
            }
        };

        Some(Ok(HostNotification { kind, safe_block_number, finalized_block_number }))
    }

    fn set_head(&mut self, block_number: u64) {
        if self.head_set {
            warn!(block_number, "set_head called more than once, ignoring");
            return;
        }
        self.head_set = true;
        info!(block_number, "initiating DB backfill");
        self.backfill = Some(DbBackfill::new(self.provider.clone(), block_number));
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        if let Some(backfill) = &mut self.backfill
            && let Some(max_blocks) = max_blocks
        {
            debug!(max_blocks, "configured DB backfill batch size");
            backfill.set_batch_size(max_blocks);
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
