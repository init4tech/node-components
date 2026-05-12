//! Parity `trace` namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    replay_block_transactions, replay_transaction, trace_block, trace_call, trace_call_many,
    trace_filter, trace_get, trace_raw_transaction, trace_transaction,
};
mod error;
pub use error::TraceError;
mod types;

use crate::config::StorageRpcCtx;
use signet_cold::ColdStorageBackend;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `trace` API router backed by storage.
pub(crate) fn trace<H, B>() -> ajj::Router<StorageRpcCtx<H, B>>
where
    H: HotKv + Send + Sync + 'static,
    B: ColdStorageBackend,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("block", trace_block::<H, B>)
        .route("transaction", trace_transaction::<H, B>)
        .route("replayBlockTransactions", replay_block_transactions::<H, B>)
        .route("replayTransaction", replay_transaction::<H, B>)
        .route("call", trace_call::<H, B>)
        .route("callMany", trace_call_many::<H, B>)
        .route("rawTransaction", trace_raw_transaction::<H, B>)
        .route("get", trace_get::<H, B>)
        .route("filter", trace_filter::<H, B>)
}
