//! Signet namespace RPC endpoint implementations.

use crate::{
    ctx::StorageRpcCtx,
    eth::helpers::{CfgFiller, await_handler, response_tri},
    signet::error::SignetError,
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::eips::{BlockId, eip1559::BaseFeeParams};
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
        let mut block_id: BlockId = id.into();

        let pending = block_id.is_pending();
        if pending {
            block_id = BlockId::latest();
        }

        let cold = ctx.cold();
        let block_num = response_tri!(ctx.resolve_block_id(block_id));

        let sealed_header =
            response_tri!(cold.get_header_by_number(block_num).await.map_err(|e| e.to_string()));

        let sealed_header =
            response_tri!(sealed_header.ok_or_else(|| format!("block not found: {block_id}")));

        let parent_hash = sealed_header.hash();
        let mut header = sealed_header.into_inner();

        // For pending blocks, synthesize the next-block header.
        if pending {
            header.parent_hash = parent_hash;
            header.number += 1;
            header.timestamp += 12;
            header.base_fee_per_gas = header.next_block_base_fee(BaseFeeParams::ethereum());
            header.gas_limit = ctx.config().rpc_gas_cap;
        }

        // State at the resolved block number (before any pending header mutation).
        let db = response_tri!(ctx.revm_state_at_height(block_num).map_err(|e| e.to_string()));

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
