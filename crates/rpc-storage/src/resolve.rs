//! Block tag tracking and BlockId resolution.
//!
//! [`BlockTags`] holds externally-updated atomic values for Latest, Safe,
//! and Finalized block numbers. The RPC context owner is responsible for
//! updating these as the chain progresses.

use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    primitives::B256,
};
use signet_cold::ColdStorageReadHandle;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Externally-updated block tag tracker.
///
/// Each tag is an `Arc<AtomicU64>` that the caller updates as the chain
/// progresses. The RPC layer reads these atomically for tag resolution.
///
/// # Example
///
/// ```
/// use signet_rpc_storage::BlockTags;
///
/// let tags = BlockTags::new(100, 95, 90);
/// assert_eq!(tags.latest(), 100);
///
/// tags.set_latest(101);
/// assert_eq!(tags.latest(), 101);
/// ```
#[derive(Debug, Clone)]
pub struct BlockTags {
    latest: Arc<AtomicU64>,
    safe: Arc<AtomicU64>,
    finalized: Arc<AtomicU64>,
}

impl BlockTags {
    /// Create new block tags with initial values.
    pub fn new(latest: u64, safe: u64, finalized: u64) -> Self {
        Self {
            latest: Arc::new(AtomicU64::new(latest)),
            safe: Arc::new(AtomicU64::new(safe)),
            finalized: Arc::new(AtomicU64::new(finalized)),
        }
    }

    /// Get the latest block number.
    pub fn latest(&self) -> u64 {
        self.latest.load(Ordering::Acquire)
    }

    /// Get the safe block number.
    pub fn safe(&self) -> u64 {
        self.safe.load(Ordering::Acquire)
    }

    /// Get the finalized block number.
    pub fn finalized(&self) -> u64 {
        self.finalized.load(Ordering::Acquire)
    }

    /// Set the latest block number.
    pub fn set_latest(&self, n: u64) {
        self.latest.store(n, Ordering::Release);
    }

    /// Set the safe block number.
    pub fn set_safe(&self, n: u64) {
        self.safe.store(n, Ordering::Release);
    }

    /// Set the finalized block number.
    pub fn set_finalized(&self, n: u64) {
        self.finalized.store(n, Ordering::Release);
    }
}

/// Error resolving a block identifier.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ResolveError {
    /// Cold storage error.
    #[error(transparent)]
    Cold(#[from] signet_cold::ColdStorageError),
    /// Block hash not found.
    #[error("block hash not found: {0}")]
    HashNotFound(B256),
}

/// Resolve a [`BlockId`] to a block number.
///
/// - `Latest` / `Pending` → `tags.latest()`
/// - `Safe` → `tags.safe()`
/// - `Finalized` → `tags.finalized()`
/// - `Earliest` → `0`
/// - `Number(n)` → `n`
/// - `Hash(h)` → cold storage header lookup → `header.number`
pub(crate) async fn resolve_block_id(
    id: BlockId,
    tags: &BlockTags,
    cold: &ColdStorageReadHandle,
) -> Result<u64, ResolveError> {
    match id {
        BlockId::Number(tag) => resolve_block_number_or_tag(tag, tags),
        BlockId::Hash(h) => {
            let header = cold
                .get_header_by_hash(h.block_hash)
                .await?
                .ok_or(ResolveError::HashNotFound(h.block_hash))?;
            Ok(header.number)
        }
    }
}

/// Resolve a [`BlockNumberOrTag`] to a block number (sync, no cold lookup needed).
pub(crate) fn resolve_block_number_or_tag(
    tag: BlockNumberOrTag,
    tags: &BlockTags,
) -> Result<u64, ResolveError> {
    Ok(match tag {
        BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => tags.latest(),
        BlockNumberOrTag::Safe => tags.safe(),
        BlockNumberOrTag::Finalized => tags.finalized(),
        BlockNumberOrTag::Earliest => 0,
        BlockNumberOrTag::Number(n) => n,
    })
}
