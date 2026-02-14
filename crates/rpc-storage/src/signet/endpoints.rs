//! Signet namespace RPC endpoint implementations.

use crate::{
    ctx::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{CfgFiller, await_handler, response_tri},
    signet::error::SignetError,
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::eips::BlockId;
use signet_bundle::{SignetBundleDriver, SignetCallBundle, SignetCallBundleResponse};
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use signet_types::SignedOrder;
use std::time::Duration;
use tokio::select;
use trevm::revm::database::DBErrorMarker;

/// `signet_sendOrder` handler.
pub(super) async fn send_order<H>(
    hctx: HandlerCtx,
    order: SignedOrder,
    ctx: StorageRpcCtx<H>,
) -> Result<(), String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let Some(tx_cache) = ctx.tx_cache().cloned() else {
        return Err(SignetError::TxCacheNotProvided.into_string());
    };

    let task = |hctx: HandlerCtx| async move {
        hctx.spawn(async move { tx_cache.forward_order(order).await.map_err(|e| e.to_string()) });
        Ok(())
    };

    await_handler!(@option hctx.spawn_blocking_with_ctx(task))
}

/// `signet_callBundle` handler.
pub(super) async fn call_bundle<H>(
    hctx: HandlerCtx,
    bundle: SignetCallBundle,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<SignetCallBundleResponse, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let timeout = bundle.bundle.timeout.unwrap_or(1000);

    let task = async move {
        let id = bundle.state_block_number();
        let block_id: BlockId = id.into();

        let EvmBlockContext { header, db } = response_tri!(ctx.resolve_evm_block(block_id));

        let mut driver = SignetBundleDriver::from(&bundle);

        let trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        response_tri!(trevm.drive_bundle(&mut driver).map_err(|e| e.into_error()));

        ResponsePayload::Success(driver.into_response())
    };

    let task = async move {
        select! {
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {
                ResponsePayload::internal_error_message(
                    "timeout during bundle simulation".into(),
                )
            }
            result = task => {
                result
            }
        }
    };

    await_handler!(@response_option hctx.spawn_blocking(task))
}
