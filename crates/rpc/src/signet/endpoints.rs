//! Signet namespace RPC endpoint implementations.

use crate::{
    config::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{CfgFiller, await_handler},
    signet::error::SignetError,
};
use ajj::HandlerCtx;
use alloy::eips::BlockId;
use signet_bundle::{SignetBundleDriver, SignetCallBundle, SignetCallBundleResponse};
use signet_hot::{HotKv, model::HotKvRead};
use signet_types::SignedOrder;
use std::time::Duration;
use tokio::select;
use trevm::revm::database::DBErrorMarker;

/// `signet_sendOrder` handler.
///
/// Forwards the order to the transaction cache asynchronously. The
/// response is returned immediately — forwarding errors are logged
/// but not propagated to the caller (fire-and-forget).
pub(super) async fn send_order<H>(
    hctx: HandlerCtx,
    order: SignedOrder,
    ctx: StorageRpcCtx<H>,
) -> Result<(), SignetError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let Some(tx_cache) = ctx.tx_cache().cloned() else {
        return Err(SignetError::TxCacheNotProvided);
    };

    let task = |hctx: HandlerCtx| async move {
        hctx.spawn(async move {
            if let Err(e) = tx_cache.forward_order(order).await {
                tracing::warn!(error = %e, "failed to forward order");
            }
        });
        Ok(())
    };

    await_handler!(hctx.spawn_with_ctx(task), SignetError::Timeout)
}

/// `signet_callBundle` handler.
pub(super) async fn call_bundle<H>(
    hctx: HandlerCtx,
    bundle: SignetCallBundle,
    ctx: StorageRpcCtx<H>,
) -> Result<SignetCallBundleResponse, SignetError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let timeout = bundle.bundle.timeout.unwrap_or(ctx.config().default_bundle_timeout_ms);

    let task = async move {
        let id = bundle.state_block_number();
        let block_id: BlockId = id.into();

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(block_id).map_err(|e| {
                tracing::warn!(error = %e, ?block_id, "block resolution failed for bundle");
                SignetError::Resolve(e.to_string())
            })?;

        let mut driver = SignetBundleDriver::from(&bundle);

        let mut trevm = signet_evm::signet_evm(db, ctx.constants().clone());
        trevm.set_spec_id(spec_id);
        let trevm = trevm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        trevm.drive_bundle(&mut driver).map_err(|e| {
            let e = e.into_error();
            tracing::warn!(error = %e, "evm error during bundle simulation");
            SignetError::EvmHalt { reason: e.to_string() }
        })?;

        Ok(driver.into_response())
    };

    let task = async move {
        select! {
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {
                Err(SignetError::Timeout)
            }
            result = task => {
                result
            }
        }
    };

    await_handler!(hctx.spawn(task), SignetError::Timeout)
}
