//! Debug namespace RPC router backed by storage.

mod endpoints;
use endpoints::{trace_block, trace_transaction};
mod error;
pub use error::DebugError;
pub(crate) mod tracer;

use crate::ctx::StorageRpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `debug` API router backed by storage.
pub(crate) fn debug<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("traceBlockByNumber", trace_block::<BlockNumberOrTag, H>)
        .route("traceBlockByHash", trace_block::<B256, H>)
        .route("traceTransaction", trace_transaction::<H>)
}
