use crate::{
    DebugError, RpcCtx,
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{consensus::BlockHeader, primitives::B256};
use itertools::Itertools;
use reth::rpc::{
    server_types::eth::EthApiError,
    types::{
        TransactionInfo,
        trace::geth::{GethDebugTracingOptions, GethTrace},
    },
};
use reth_node_api::FullNodeComponents;
use signet_evm::EvmErrored;
use signet_node_types::Pnt;
use signet_types::MagicSig;

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
        // Load the transaction by hash
        let (tx, meta) = response_tri!(
            response_tri!(ctx.signet().raw_transaction_by_hash(tx_hash))
                .ok_or(EthApiError::TransactionNotFound)
        );

        // Load the block containing the transaction
        let res = response_tri!(ctx.signet().raw_block(meta.block_hash).await);
        let (_, block) =
            response_tri!(res.ok_or_else(|| EthApiError::HeaderNotFound(meta.block_hash.into())));

        // Load trevm at the start of the block (i.e. before any transactions are applied)
        let mut trevm = response_tri!(
            ctx.trevm(crate::LoadState::Before, block.header()).map_err(EthApiError::from)
        );

        // Apply all transactions in the block up to (but not including) the
        // target one
        // TODO: check if the tx signature is a magic sig, and abort if so.
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

        let res =
            response_tri!(crate::debug::tracer::trace(trevm, &opts.unwrap_or_default(), tx_info)).0;

        ResponsePayload::Success(res)
    };

    await_handler!(@response_option hctx.spawn_blocking(fut))
}
