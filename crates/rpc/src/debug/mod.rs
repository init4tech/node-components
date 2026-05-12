//! Debug namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    debug_trace_call, get_raw_block, get_raw_header, get_raw_receipts, get_raw_transaction,
    trace_block, trace_block_rlp, trace_transaction,
};
mod error;
pub use error::DebugError;
pub(crate) mod tracer;
mod types;

use crate::config::StorageRpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_cold::ColdStorageBackend;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `debug` API router backed by storage.
pub(crate) fn debug<H, B>() -> ajj::Router<StorageRpcCtx<H, B>>
where
    H: HotKv + Send + Sync + 'static,
    B: ColdStorageBackend,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("traceBlockByNumber", trace_block::<BlockNumberOrTag, H, B>)
        .route("traceBlockByHash", trace_block::<B256, H, B>)
        .route("traceTransaction", trace_transaction::<H, B>)
        .route("traceBlock", trace_block_rlp::<H, B>)
        .route("getRawBlock", get_raw_block::<H, B>)
        .route("getRawHeader", get_raw_header::<H, B>)
        .route("getRawReceipts", get_raw_receipts::<H, B>)
        .route("getRawTransaction", get_raw_transaction::<H, B>)
        .route("traceCall", debug_trace_call::<H, B>)
}
