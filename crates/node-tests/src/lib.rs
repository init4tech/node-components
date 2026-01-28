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

/// Test constants.
pub mod constants;

/// Test context.
mod context;
pub use context::{BalanceChecks, NonceChecks, SignetTestContext};

/// Bespoke conversion utilities for converting between alloy and reth types.
pub mod convert;

/// Signet node RPC server utilities and helpers.
pub mod rpc;
pub use rpc::rpc_test;

/// Test aliases and type definitions.
pub mod types;

/// Utility functions and test harnesses.
pub mod utils;
pub use utils::run_test;

pub use reth_exex_test_utils::{Adapter, TestExExContext, TmpDB as TmpDb};
pub use signet_test_utils::specs::{
    HostBlockSpec, NotificationSpec, NotificationWithSidecars, RuBlockSpec,
};
