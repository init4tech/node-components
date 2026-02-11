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

mod ctx;
pub use ctx::StorageRpcCtx;

mod resolve;
pub use resolve::BlockTags;

mod eth;
pub use eth::EthError;

/// Instantiate the `eth` API router.
pub fn eth<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: signet_hot::HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as signet_hot::model::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
{
    eth::eth()
}
