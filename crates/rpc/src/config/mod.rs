//! Configuration, context, and block tag resolution.
//!
//! This module groups the crate's configuration types, the RPC context
//! that wraps [`signet_storage::UnifiedStorage`], gas oracle helpers,
//! and block tag / block ID resolution logic.

use std::sync::atomic::{AtomicU64, Ordering};

use alloy::primitives::U256;

mod rpc_config;
pub use rpc_config::StorageRpcConfig;

mod ctx;
pub(crate) use ctx::EvmBlockContext;
pub use ctx::StorageRpcCtx;

pub(crate) mod gas_oracle;

pub(crate) mod resolve;
pub use resolve::{BlockTags, SyncStatus};

/// Lock-free cache for gas oracle tip suggestions.
/// Invalidates automatically when block number changes.
#[derive(Debug)]
pub struct GasOracleCache {
    /// Block number this cached value corresponds to
    block: AtomicU64,
    /// Cached tip value
    tip: AtomicU64,
}

impl GasOracleCache {
    /// Create a new cache with no valid cached value.
    pub const fn new() -> Self {
        Self {
            block: AtomicU64::new(u64::MAX), // sentinel - no valid cache
            tip: AtomicU64::new(0),
        }
    }

    /// Returns cached tip if still valid for current block
    pub fn get(&self, current_block: u64) -> Option<U256> {
        let cached_block = self.block.load(Ordering::Acquire);
        if cached_block == current_block {
            Some(U256::from(self.tip.load(Ordering::Acquire)))
        } else {
            None
        }
    }

    /// Update cache for new block
    pub fn set(&self, block: u64, tip: U256) {
        let tip_u64: u64 = tip.try_into().unwrap_or(u64::MAX);
        self.tip.store(tip_u64, Ordering::Release);
        self.block.store(block, Ordering::Release);
    }
}

impl Default for GasOracleCache {
    fn default() -> Self {
        Self::new()
    }
}
