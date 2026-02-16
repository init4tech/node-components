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
pub use config::{BlockTags, ChainNotifier, StorageRpcConfig, StorageRpcCtx, SyncStatus};

mod eth;
pub use eth::EthError;

mod interest;
pub use interest::NewBlockNotification;

mod debug;
pub use debug::DebugError;

mod signet;
pub use signet::error::SignetError;

/// Instantiate a combined router with `eth`, `debug`, and `signet`
/// namespaces.
pub fn router<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    ajj::Router::new()
        .nest("eth", eth::eth())
        .nest("debug", debug::debug())
        .nest("signet", signet::signet())
}
