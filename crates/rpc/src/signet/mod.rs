//! Signet RPC methods and related code.

mod endpoints;
use endpoints::{call_bundle, send_order};
pub(crate) mod error;

use crate::config::StorageRpcCtx;
use signet_cold::ColdStorageBackend;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `signet` API router backed by storage.
pub(crate) fn signet<H, B>() -> ajj::Router<StorageRpcCtx<H, B>>
where
    H: HotKv + Send + Sync + 'static,
    B: ColdStorageBackend,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("sendOrder", send_order::<H, B>)
        .route("callBundle", call_bundle::<H, B>)
}
