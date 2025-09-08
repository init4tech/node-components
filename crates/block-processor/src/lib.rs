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

mod v1;
pub use v1::SignetBlockProcessor as SignetBlockProcessorV1;

/// Primitives used by the host.
pub type PrimitivesOf<Host> =
    <<Host as reth_node_api::FullNodeTypes>::Types as reth_node_api::NodeTypes>::Primitives;

/// A [`reth::providers::Chain`] using the host primitives.
pub type Chain<Host> = reth::providers::Chain<PrimitivesOf<Host>>;

/// A [`reth_exex::ExExNotification`] using the host primitives.
pub type ExExNotification<Host> = reth_exex::ExExNotification<PrimitivesOf<Host>>;
