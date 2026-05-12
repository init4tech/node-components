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

mod core;
pub use core::SignetNodeConfig;

// NB: RPC config merging (previously `merge_rpc_configs`) is now the
// responsibility of the host adapter crate (e.g. `signet-host-reth`).

mod storage;
pub use storage::{NodeColdBackend, StorageConfig};

// `signet-cold-mdbx` is referenced via `NodeColdBackend` only when the SQL
// features are disabled. Keep the dependency satisfied for the SQL-enabled
// build so callers can still type-name the backend uniformly.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use signet_cold_mdbx as _;

/// Test configuration for Signet Nodes.
#[cfg(feature = "test_utils")]
pub mod test_utils;

#[cfg(test)]
mod test {
    use init4_bin_base::utils::from_env::FromEnv;

    use crate::SignetNodeConfig;

    #[test]
    fn print_inventory() {
        let inventory = SignetNodeConfig::inventory();
        for config in inventory {
            println!("{config:?}");
        }
    }
}
