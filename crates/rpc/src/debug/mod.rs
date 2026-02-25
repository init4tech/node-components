mod endpoints;
use endpoints::*;

mod error;
pub use error::DebugError;

mod tracer;

use crate::ctx::RpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_node_types::Pnt;

/// Instantiate a `debug` API router.
pub fn debug<Host, Signet>() -> ajj::Router<RpcCtx<Host, Signet>>
where
    Host: Send + Sync + 'static,
    Signet: Pnt,
{
    ajj::Router::new()
        .route("traceBlockByNumber", trace_block::<BlockNumberOrTag, _, _>)
        .route("traceBlockByHash", trace_block::<B256, _, _>)
        .route("traceTransaction", trace_transaction)
}
