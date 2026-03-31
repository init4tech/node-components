//! `net` namespace RPC handlers.

use crate::config::StorageRpcCtx;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate the `net` API router.
pub(crate) fn net<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new().route("version", version::<H>).route("listening", listening)
}

/// `net_version` — returns the chain ID as a decimal string.
pub(crate) async fn version<H: HotKv>(ctx: StorageRpcCtx<H>) -> Result<String, ()> {
    Ok(ctx.chain_id().to_string())
}

/// `net_listening` — always returns true (the server is listening).
pub(crate) async fn listening() -> Result<bool, ()> {
    Ok(true)
}
