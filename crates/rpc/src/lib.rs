#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

pub(crate) mod config;
pub use config::{
    BlockTags, ChainNotifier, StorageRpcConfig, StorageRpcConfigEnv, StorageRpcCtx, SyncStatus,
};

mod eth;
pub use eth::EthError;

mod interest;
pub use interest::{ChainEvent, NewBlockNotification, RemovedBlock, ReorgNotification};

mod debug;
pub use debug::DebugError;

mod trace;
pub use trace::TraceError;

mod signet;
pub use signet::error::SignetError;

mod net;
mod web3;

pub mod serve;
pub use serve::{RpcServerGuard, ServeConfig, ServeConfigEnv, ServeError};

// Concrete cold backend chosen at compile time. The node and RPC layer use
// this alias instead of propagating a `B: ColdStorageBackend` generic.
// Selected at compile time:
//   - `test-utils` → in-memory backend for unit/integration tests
//   - `postgres` or `sqlite` → runtime-selectable MDBX-or-SQL via `EitherCold`
//   - default → MDBX cold backend
/// Concrete cold storage backend used by the node.
#[cfg(feature = "test-utils")]
pub type NodeColdBackend = signet_cold::mem::MemColdBackend;

/// Concrete cold storage backend used by the node.
#[cfg(all(not(feature = "test-utils"), any(feature = "postgres", feature = "sqlite")))]
pub type NodeColdBackend = signet_storage::either::EitherCold;

/// Concrete cold storage backend used by the node.
#[cfg(all(not(feature = "test-utils"), not(any(feature = "postgres", feature = "sqlite"))))]
pub type NodeColdBackend = signet_cold_mdbx::MdbxColdBackend;

// `signet-cold-mdbx` is referenced via `NodeColdBackend` only on the default
// (non-test, non-SQL) path. Keep the dep satisfied on the other paths so
// `unused_crate_dependencies` does not fire — `MdbxColdBackend` is still
// reachable transitively through `EitherCold`.
#[cfg(any(feature = "test-utils", any(feature = "postgres", feature = "sqlite")))]
use signet_cold_mdbx as _;

/// Instantiate a combined router with `eth`, `debug`, `trace`, `signet`,
/// `web3`, and `net` namespaces.
pub fn router<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    ajj::Router::new()
        .nest("eth", eth::eth())
        .nest("debug", debug::debug())
        .nest("trace", trace::trace())
        .nest("signet", signet::signet())
        .nest("web3", web3::web3())
        .nest("net", net::net())
}
