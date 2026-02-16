//! Signet namespace RPC endpoint implementations.

use crate::{
    config::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{CfgFiller, await_handler, response_tri},
    signet::error::SignetError,
};
use ajj::{HandlerCtx, ResponsePayload};
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
) -> ResponsePayload<(), SignetError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let Some(tx_cache) = ctx.tx_cache().cloned() else {
        return ResponsePayload(Err(SignetError::TxCacheNotProvided.into()));
    };

    let task = |hctx: HandlerCtx| async move {
        hctx.spawn(async move {
            if let Err(e) = tx_cache.forward_order(order).await {
                tracing::warn!(error = %e, "failed to forward order");
            }
        });
        ResponsePayload(Ok(()))
    };

    await_handler!(@response_option hctx.spawn_blocking_with_ctx(task))
}

/// `signet_callBundle` handler.
pub(super) async fn call_bundle<H>(
    hctx: HandlerCtx,
    bundle: SignetCallBundle,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<SignetCallBundleResponse, SignetError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let timeout = bundle.bundle.timeout.unwrap_or(ctx.config().default_bundle_timeout_ms);

    let task = async move {
        let id = bundle.state_block_number();
        let block_id: BlockId = id.into();

        let EvmBlockContext { header, db } =
            response_tri!(ctx.resolve_evm_block(block_id).map_err(|e| {
                tracing::warn!(error = %e, ?block_id, "block resolution failed for bundle");
                SignetError::Resolve(e.to_string())
            }));

        let mut driver = SignetBundleDriver::from(&bundle);

        let trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        response_tri!(trevm.drive_bundle(&mut driver).map_err(|e| {
            let e = e.into_error();
            tracing::warn!(error = %e, "evm error during bundle simulation");
            SignetError::Evm(e.to_string())
        }));

        ResponsePayload(Ok(driver.into_response()))
    };

    let task = async move {
        select! {
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {
                ResponsePayload::internal_error_message(
                    SignetError::Timeout.to_string().into(),
                )
            }
            result = task => {
                result
            }
        }
    };

    await_handler!(@response_option hctx.spawn(task))
}
