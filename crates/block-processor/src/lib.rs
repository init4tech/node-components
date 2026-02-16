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

pub(crate) mod metrics;

mod alias;
pub use alias::{AliasOracle, AliasOracleFactory};

mod utils;
pub use utils::revm_spec;

mod v1;
pub use v1::SignetBlockProcessor as SignetBlockProcessorV1;
