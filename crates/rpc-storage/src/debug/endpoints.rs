//! Debug namespace RPC endpoint implementations.

use crate::{
    ctx::StorageRpcCtx,
    debug::DebugError,
    eth::helpers::{CfgFiller, await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{
    consensus::BlockHeader,
    eips::BlockId,
    primitives::B256,
    rpc::types::trace::geth::{GethDebugTracingOptions, GethTrace, TraceResult},
};
use itertools::Itertools;
use signet_evm::EvmErrored;
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use signet_types::MagicSig;
use tracing::Instrument;
use trevm::revm::database::DBErrorMarker;

/// Params for `debug_traceBlockByNumber` and `debug_traceBlockByHash`.
#[derive(Debug, serde::Deserialize)]
pub(super) struct TraceBlockParams<T>(T, #[serde(default)] Option<GethDebugTracingOptions>);

/// Params for `debug_traceTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(super) struct TraceTransactionParams(B256, #[serde(default)] Option<GethDebugTracingOptions>);

/// `debug_traceBlockByNumber` and `debug_traceBlockByHash` handler.
pub(super) async fn trace_block<T, H>(
    hctx: HandlerCtx,
    TraceBlockParams(id, opts): TraceBlockParams<T>,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<Vec<TraceResult>, DebugError>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = response_tri!(opts.ok_or(DebugError::InvalidTracerConfig));

    let _permit = ctx.acquire_tracing_permit().await;

    let id = id.into();
    let span = tracing::debug_span!("traceBlock", ?id, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        let cold = ctx.cold();
        let block_num = response_tri!(
            ctx.resolve_block_id(id)
                .await
                .map_err(|e| { DebugError::BlockNotFound(e.to_string()) })
        );

        let (header, txs) = response_tri!(
            tokio::try_join!(
                cold.get_header_by_number(block_num),
                cold.get_transactions_in_block(block_num),
            )
            .map_err(|e| DebugError::Cold(e.to_string()))
        );

        let Some(header) = header else {
            return ResponsePayload::internal_error_message(
                format!("block not found: {id}").into(),
            );
        };

        let block_hash = header.hash_slow();

        tracing::debug!(number = header.number, "Loaded block");

        let mut frames = Vec::with_capacity(txs.len());

        // State BEFORE this block
        let db = response_tri!(
            ctx.revm_state_at_height(header.number.saturating_sub(1))
                .map_err(|e| DebugError::Hot(e.to_string()))
        );

        let mut trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        let mut txns = txs.iter().enumerate().peekable();
        for (idx, tx) in txns
            .by_ref()
            .peeking_take_while(|(_, t)| MagicSig::try_from_signature(t.signature()).is_none())
        {
            let tx_info = alloy::rpc::types::TransactionInfo {
                hash: Some(*tx.tx_hash()),
                index: Some(idx as u64),
                block_hash: Some(block_hash),
                block_number: Some(header.number),
                base_fee: header.base_fee_per_gas(),
            };

            let t = trevm.fill_tx(tx);
            let frame;
            (frame, trevm) = response_tri!(crate::debug::tracer::trace(t, &opts, tx_info));
            frames.push(TraceResult::Success { result: frame, tx_hash: Some(*tx.tx_hash()) });

            tracing::debug!(tx_index = idx, tx_hash = ?tx.tx_hash(), "Traced transaction");
        }

        ResponsePayload::Success(frames)
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(fut))
}

/// `debug_traceTransaction` handler.
pub(super) async fn trace_transaction<H>(
    hctx: HandlerCtx,
    TraceTransactionParams(tx_hash, opts): TraceTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<GethTrace, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = response_tri!(opts.ok_or(DebugError::InvalidTracerConfig));

    let _permit = ctx.acquire_tracing_permit().await;

    let span = tracing::debug_span!("traceTransaction", %tx_hash, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        let cold = ctx.cold();

        // Look up the transaction and its containing block
        let confirmed = response_tri!(
            cold.get_tx_by_hash(tx_hash).await.map_err(|e| DebugError::Cold(e.to_string()))
        );

        let confirmed = response_tri!(confirmed.ok_or(DebugError::TransactionNotFound));
        let (_tx, meta) = confirmed.into_parts();

        let block_num = meta.block_number();
        let block_hash = meta.block_hash();

        let (header, txs) = response_tri!(
            tokio::try_join!(
                cold.get_header_by_number(block_num),
                cold.get_transactions_in_block(block_num),
            )
            .map_err(|e| DebugError::Cold(e.to_string()))
        );

        let header =
            response_tri!(header.ok_or(DebugError::BlockNotFound(format!("block {block_num}"))));

        tracing::debug!(number = block_num, "Loaded containing block");

        // State BEFORE this block
        let db = response_tri!(
            ctx.revm_state_at_height(block_num.saturating_sub(1))
                .map_err(|e| DebugError::Hot(e.to_string()))
        );

        let mut trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        // Replay all transactions up to (but not including) the target
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns.by_ref().peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash) {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return ResponsePayload::internal_error_message(
                    DebugError::TransactionNotFound.to_string().into(),
                );
            }

            trevm = response_tri!(trevm.run_tx(tx).map_err(EvmErrored::into_error)).accept_state();
        }

        let (index, tx) = response_tri!(txns.next().ok_or(DebugError::TransactionNotFound));

        let trevm = trevm.fill_tx(tx);

        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(index as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let res = response_tri!(crate::debug::tracer::trace(trevm, &opts, tx_info)).0;

        ResponsePayload::Success(res)
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(fut))
}
