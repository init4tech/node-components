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
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `trace` API router backed by storage.
pub(crate) fn trace<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("block", trace_block::<H>)
        .route("transaction", trace_transaction::<H>)
        .route("replayBlockTransactions", replay_block_transactions::<H>)
        .route("replayTransaction", replay_transaction::<H>)
        .route("call", trace_call::<H>)
        .route("callMany", trace_call_many::<H>)
        .route("rawTransaction", trace_raw_transaction::<H>)
        .route("get", trace_get::<H>)
        .route("filter", trace_filter::<H>)
}
