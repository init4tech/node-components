//! Signet RPC methods and related code.

mod endpoints;
use endpoints::{call_bundle, send_order};
pub(crate) mod error;

use crate::ctx::StorageRpcCtx;
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `signet` API router backed by storage.
pub(crate) fn signet<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new().route("sendOrder", send_order::<H>).route("callBundle", call_bundle::<H>)
}
