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

mod alias;
pub use alias::{RethAliasOracle, RethAliasOracleFactory};

mod blob_source;
pub use blob_source::RethBlobSource;

mod error;
pub use error::RethHostError;

mod chain;
pub use chain::RethChain;

mod config;
pub use config::{rpc_config_from_args, serve_config_from_args};

mod notifier;
pub use notifier::{DecomposedContext, RethHostNotifier, decompose_exex_context};

mod shim;
pub use shim::RecoveredBlockShim;
