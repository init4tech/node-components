use crate::{
    DebugError, RpcCtx,
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::primitives::B256;
use itertools::Itertools;
use reth::rpc::{
    server_types::eth::EthApiError,
    types::trace::geth::{GethDebugTracingOptions, GethTrace},
};
use reth_node_api::FullNodeComponents;
use signet_evm::EvmErrored;
use signet_node_types::Pnt;

// /// Params for the `debug_traceBlockByNumber` and `debug_traceBlockByHash`
// /// endpoints.
// #[derive(Debug, serde::Deserialize)]
// pub(super) struct TraceBlockParams<T>(T, #[serde(default)] Option<GethDebugTracingOptions>);

// /// `debug_traceBlockByNumber` and `debug_traceBlockByHash` endpoint handler.
// pub(super) async fn trace_block<T, Host, Signet>(
//     hctx: HandlerCtx,
//     TraceBlockParams(id, opts): TraceBlockParams<T>,
//     ctx: RpcCtx<Host, Signet>,
// ) -> ResponsePayload<Vec<TraceResult>, TraceError>
// where
//     T: Into<BlockId>,
//     Host: FullNodeComponents,
//     Signet: Pnt,
// {
//     let id = id.into();

//     let fut = async move {
//         // Fetch the block by ID
//         let Some((hash, block)) = response_tri!(ctx.signet().raw_block(id).await) else {
//             return ResponsePayload::internal_error_message(
//                 EthApiError::HeaderNotFound(id).to_string().into(),
//             );
//         };
//         // Instantiate the EVM with the block
//         let evm = response_tri!(ctx.trevm(id, block.header()));

//         todo!()

//         // ResponsePayload::Success(vec![])
//     };

//     await_jh_option_response!(hctx.spawn_blocking(fut))
// }

/// Handle for `debug_traceTransaction`.
pub(super) async fn trace_transaction<Host, Signet>(
    hctx: HandlerCtx,
    (tx_hash, opts): (B256, Option<GethDebugTracingOptions>),
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<GethTrace, DebugError>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let fut = async move {
        let (tx, meta) = response_tri!(
            response_tri!(ctx.signet().raw_transaction_by_hash(tx_hash))
                .ok_or(EthApiError::TransactionNotFound)
        );

        let res = response_tri!(ctx.signet().raw_block(meta.block_hash).await);
        let (_, block) =
            response_tri!(res.ok_or_else(|| EthApiError::HeaderNotFound(meta.block_hash.into())));

        // Load trevm at the start of the block (i.e. before any transactions are applied)
        let mut trevm = response_tri!(
            ctx.trevm(crate::LoadState::Before, block.header()).map_err(EthApiError::from)
        );

        let mut txns = block.body().transactions().peekable();

        for tx in txns.by_ref().peeking_take_while(|t| t.hash() != tx.hash()) {
            // Apply all transactions before the target one
            trevm = response_tri!(trevm.run_tx(tx).map_err(EvmErrored::into_error)).accept_state();
        }

        let tx = response_tri!(txns.next().ok_or(EthApiError::TransactionNotFound));
        let trevm = trevm.fill_tx(tx);

        let res = response_tri!(crate::debug::tracer::trace(trevm, &opts.unwrap_or_default()));

        ResponsePayload::Success(res)
    };

    await_handler!(@response_option hctx.spawn_blocking(fut))
}
