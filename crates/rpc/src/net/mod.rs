//! `net` namespace RPC handlers.

use crate::config::StorageRpcCtx;
use signet_cold::ColdStorageBackend;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate the `net` API router.
pub(crate) fn net<H, B>() -> ajj::Router<StorageRpcCtx<H, B>>
where
    H: HotKv + Send + Sync + 'static,
    B: ColdStorageBackend,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new().route("version", version::<H, B>).route("listening", listening)
}

/// `net_version` — returns the chain ID as a decimal string.
pub(crate) async fn version<H: HotKv, B: ColdStorageBackend>(
    ctx: StorageRpcCtx<H, B>,
) -> Result<String, ()> {
    Ok(ctx.chain_id().to_string())
}

/// `net_listening` — always returns true (the server is listening).
pub(crate) async fn listening() -> Result<bool, ()> {
    Ok(true)
}
