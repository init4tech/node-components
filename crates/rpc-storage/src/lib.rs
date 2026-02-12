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

mod config;
pub use config::StorageRpcConfig;
mod ctx;
pub use ctx::StorageRpcCtx;
mod resolve;
pub use resolve::BlockTags;
mod eth;
pub use eth::EthError;
mod gas_oracle;
mod interest;
pub use interest::NewBlockNotification;
mod debug;
pub use debug::DebugError;
mod signet;
pub use signet::error::SignetError;

/// Instantiate the `eth` API router.
pub fn eth<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    eth::eth()
}

/// Instantiate the `debug` API router.
pub fn debug<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    debug::debug()
}

/// Instantiate the `signet` API router.
pub fn signet<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    signet::signet()
}

/// Instantiate a combined router with `eth`, `debug`, and `signet`
/// namespaces.
pub fn router<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    ajj::Router::new().merge(eth::eth()).merge(debug::debug()).merge(signet::signet())
}
