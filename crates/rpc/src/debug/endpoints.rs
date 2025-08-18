use crate::{
    RpcCtx, TraceError,
    utils::{await_jh_option_response, response_tri},
};
use ajj::HandlerCtx;
use ajj::ResponsePayload;
use alloy::eips::BlockId;
use reth::rpc::{
    server_types::eth::EthApiError,
    types::trace::geth::{GethDebugTracingOptions, TraceResult},
};
use reth_node_api::FullNodeComponents;
use signet_node_types::Pnt;

#[derive(Debug, serde::Deserialize)]
struct TraceBlockParams<T>(T, #[serde(default)] Option<GethDebugTracingOptions>);

pub(super) async fn trace_block<T, Host, Signet>(
    hctx: HandlerCtx,
    TraceBlockParams(id, opts): TraceBlockParams<T>,
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<Vec<TraceResult>, TraceError>
where
    T: Into<BlockId>,
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let id = id.into();

    let fut = async move {
        // // Fetch the block by ID
        // let Some((hash, block)) = response_tri!(ctx.signet().raw_block(id).await) else {
        //     return ResponsePayload::internal_error_message(
        //         EthApiError::HeaderNotFound(id).to_string().into(),
        //     );
        // };
        // // Instantiate the EVM with the block
        // let evm = response_tri!(ctx.trevm(id, &block.header()));

        ResponsePayload::Success(vec![])
    };

    await_jh_option_response!(hctx.spawn_blocking(fut))
}
