//! Eth RPC namespace endpoints.
//!
//! Endpoint implementations are provided in Plan 3.

use crate::ctx::StorageRpcCtx;
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use trevm::revm::database::DBErrorMarker;

/// Errors returned by `eth_*` RPC methods.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum EthError {
    /// Placeholder — additional variants added in Plan 3.
    #[error("not yet implemented")]
    NotImplemented,
}

/// Instantiate the `eth` API router.
pub(crate) fn eth<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
}
