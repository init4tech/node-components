//! Configuration, context, and block tag resolution.
//!
//! This module groups the crate's configuration types, the RPC context
//! that wraps [`signet_storage::UnifiedStorage`], gas oracle helpers,
//! and block tag / block ID resolution logic.

mod chain_notifier;
pub use chain_notifier::ChainNotifier;
mod rpc_config;
pub use rpc_config::StorageRpcConfig;

mod ctx;
pub(crate) use ctx::EvmBlockContext;
pub use ctx::StorageRpcCtx;

pub(crate) mod gas_oracle;

pub(crate) mod resolve;
pub use resolve::{BlockTags, SyncStatus};
