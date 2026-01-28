use crate::{
    DebugError, RpcCtx,
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{consensus::BlockHeader, eips::BlockId, primitives::B256};
use itertools::Itertools;
use reth::rpc::{
    server_types::eth::EthApiError,
    types::{
        TransactionInfo,
        trace::geth::{GethDebugTracingOptions, GethTrace, TraceResult},
    },
};
use reth_node_api::FullNodeComponents;
use signet_evm::EvmErrored;
use signet_node_types::Pnt;
use signet_types::MagicSig;
use tracing::Instrument;

/// Params for the `debug_traceBlockByNumber` and `debug_traceBlockByHash`
/// endpoints.
#[derive(Debug, serde::Deserialize)]
pub(super) struct TraceBlockParams<T>(T, #[serde(default)] Option<GethDebugTracingOptions>);

/// Params type for `debug_traceTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(super) struct TraceTransactionParams(B256, #[serde(default)] Option<GethDebugTracingOptions>);

/// `debug_traceBlockByNumber` and `debug_traceBlockByHash` endpoint handler.
pub(super) async fn trace_block<T, Host, Signet>(
    hctx: HandlerCtx,
    TraceBlockParams(id, opts): TraceBlockParams<T>,
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<Vec<TraceResult>, DebugError>
where
    T: Into<BlockId>,
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let opts = response_tri!(opts.ok_or(DebugError::from(EthApiError::InvalidTracerConfig)));

    let _permit = response_tri!(
        ctx.acquire_tracing_permit()
            .await
            .map_err(|_| DebugError::rpc_error("Failed to acquire tracing permit".into()))
    );

    let id = id.into();
    let span = tracing::debug_span!("traceBlock", ?id, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        // Fetch the block by ID
        let Some((hash, block)) = response_tri!(ctx.signet().raw_block(id).await) else {
            return ResponsePayload::internal_error_message(
                EthApiError::HeaderNotFound(id).to_string().into(),
            );
        };

        tracing::debug!(number = block.number(), "Loaded block");

        // Allocate space for the frames
        let mut frames = Vec::with_capacity(block.transaction_count());

        // Instantiate the EVM with the block
        let mut trevm = response_tri!(ctx.trevm(crate::LoadState::Before, block.header()));

        // Apply all transactions in the block up, tracing each one
        tracing::trace!(?opts, "Tracing block transactions");

        let mut txns = block.body().transactions().enumerate().peekable();
        for (idx, tx) in txns
            .by_ref()
            .peeking_take_while(|(_, t)| MagicSig::try_from_signature(t.signature()).is_none())
        {
            let tx_info = TransactionInfo {
                hash: Some(*tx.hash()),
                index: Some(idx as u64),
                block_hash: Some(hash),
                block_number: Some(block.header().number()),
                base_fee: block.header().base_fee_per_gas(),
            };

            let t = trevm.fill_tx(tx);

            let frame;
            (frame, trevm) = response_tri!(crate::debug::tracer::trace(t, &opts, tx_info));
            frames.push(TraceResult::Success { result: frame, tx_hash: Some(*tx.hash()) });

            tracing::debug!(tx_index = idx, tx_hash = ?tx.hash(), "Traced transaction");
        }

        ResponsePayload::Success(frames)
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(fut))
}

/// Handle for `debug_traceTransaction`.
pub(super) async fn trace_transaction<Host, Signet>(
    hctx: HandlerCtx,
    TraceTransactionParams(tx_hash, opts): TraceTransactionParams,
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<GethTrace, DebugError>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let opts = response_tri!(opts.ok_or(DebugError::from(EthApiError::InvalidTracerConfig)));

    let _permit = response_tri!(
        ctx.acquire_tracing_permit()
            .await
            .map_err(|_| DebugError::rpc_error("Failed to acquire tracing permit".into()))
    );

    let span = tracing::debug_span!("traceTransaction", %tx_hash, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        // Load the transaction by hash
        let (tx, meta) = response_tri!(
            response_tri!(ctx.signet().raw_transaction_by_hash(tx_hash))
                .ok_or(EthApiError::TransactionNotFound)
        );

        tracing::debug!("Loaded transaction metadata");

        // Load the block containing the transaction
        let res = response_tri!(ctx.signet().raw_block(meta.block_hash).await);
        let (_, block) =
            response_tri!(res.ok_or_else(|| EthApiError::HeaderNotFound(meta.block_hash.into())));

        tracing::debug!(number = block.number(), "Loaded containing block");

        // Load trevm at the start of the block (i.e. before any transactions are applied)
        let mut trevm = response_tri!(ctx.trevm(crate::LoadState::Before, block.header()));

        // Apply all transactions in the block up to (but not including) the
        // target one
        let mut txns = block.body().transactions().enumerate().peekable();
        for (_idx, tx) in txns.by_ref().peeking_take_while(|(_, t)| t.hash() != tx.hash()) {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return ResponsePayload::internal_error_message(
                    EthApiError::TransactionNotFound.to_string().into(),
                );
            }

            trevm = response_tri!(trevm.run_tx(tx).map_err(EvmErrored::into_error)).accept_state();
        }

        let (index, tx) = response_tri!(txns.next().ok_or(EthApiError::TransactionNotFound));

        let trevm = trevm.fill_tx(tx);

        let tx_info = TransactionInfo {
            hash: Some(*tx.hash()),
            index: Some(index as u64),
            block_hash: Some(block.hash()),
            block_number: Some(block.header().number()),
            base_fee: block.header().base_fee_per_gas(),
        };

        let res = response_tri!(crate::debug::tracer::trace(trevm, &opts, tx_info)).0;

        ResponsePayload::Success(res)
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(fut))
}
