//! Block tag tracking and [`BlockId`] resolution.
//!
//! [`BlockTags`] holds externally-updated atomic values for Latest, Safe,
//! and Finalized block numbers. The RPC context owner is responsible for
//! updating these as the chain progresses.
//!
//! # Atomic Ordering Guarantees
//!
//! All tag loads use [`Acquire`] ordering and all stores use [`Release`]
//! ordering. This establishes a happens-before relationship: any data
//! written before a tag update is visible to readers who observe the new
//! tag value.
//!
//! Multi-tag updates ([`BlockTags::update_all`]) store in order:
//! finalized → safe → latest. This ensures readers never observe a
//! state where `latest` is behind `finalized` — they may see a
//! slightly stale view (old latest with new finalized), but never an
//! inverted one.
//!
//! During reorgs, [`BlockTags::rewind_to`] uses [`fetch_min`] with the
//! same finalized → safe → latest order, atomically capping each tag
//! without risk of a reader seeing `latest > finalized` while values
//! are being decreased.
//!
//! # Race Window
//!
//! Between [`StorageRpcCtx::resolve_block_tag`] and the subsequent
//! storage query, tags can advance. The caller gets a point-in-time
//! snapshot of the tag values — not a guarantee that those values are
//! still current by the time the query executes. For hot-only queries,
//! the MDBX read transaction provides snapshot isolation. For cold
//! queries, the resolved number may reference a block that cold storage
//! has not yet received (returning `None`) or that a reorg has since
//! replaced (returning stale data).
//!
//! [`Acquire`]: std::sync::atomic::Ordering::Acquire
//! [`Release`]: std::sync::atomic::Ordering::Release
//! [`fetch_min`]: std::sync::atomic::AtomicU64::fetch_min
//! [`StorageRpcCtx::resolve_block_tag`]: crate::StorageRpcCtx::resolve_block_tag

use alloy::primitives::B256;
use signet_storage::StorageError;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};

/// Snapshot of the node's syncing progress.
///
/// When the node is still catching up to the network, this struct
/// describes the sync window. Once fully synced, the context owner
/// should call [`BlockTags::clear_sync_status`] to indicate that
/// syncing is complete.
#[derive(Debug, Clone, Copy)]
pub struct SyncStatus {
    /// Block number the node started syncing from.
    pub starting_block: u64,
    /// Current block the node has synced to.
    pub current_block: u64,
    /// Highest known block number on the network.
    pub highest_block: u64,
}

/// Externally-updated block tag tracker.
///
/// Each tag is an [`Arc<AtomicU64>`] that the caller updates as the
/// chain progresses. The RPC layer reads these atomically for tag
/// resolution.
///
/// Tags are updated in a specific order to maintain consistency:
/// - **Writes** ([`update_all`](Self::update_all)): finalized → safe →
///   latest, so readers never see `latest` behind `finalized`.
/// - **Rewinds** ([`rewind_to`](Self::rewind_to)): same order, using
///   [`fetch_min`](AtomicU64::fetch_min) for atomic capping.
/// - **Reads**: each tag is loaded independently with [`Acquire`]
///   ordering. A reader may see a slightly stale combination (e.g.,
///   new `finalized` but old `latest`) but never an inverted one.
///
/// [`Acquire`]: std::sync::atomic::Ordering::Acquire
///
/// # Example
///
/// ```
/// use signet_rpc::BlockTags;
///
/// let tags = BlockTags::new(100, 95, 90);
/// assert_eq!(tags.latest(), 100);
///
/// tags.set_latest(101);
/// assert_eq!(tags.latest(), 101);
///
/// // Update all tags at once.
/// tags.update_all(200, 195, 190);
/// assert_eq!(tags.latest(), 200);
/// assert_eq!(tags.safe(), 195);
/// assert_eq!(tags.finalized(), 190);
/// ```
#[derive(Debug, Clone)]
pub struct BlockTags {
    latest: Arc<AtomicU64>,
    safe: Arc<AtomicU64>,
    finalized: Arc<AtomicU64>,
    sync_status: Arc<RwLock<Option<SyncStatus>>>,
}

impl BlockTags {
    /// Create new block tags with initial values.
    pub fn new(latest: u64, safe: u64, finalized: u64) -> Self {
        Self {
            latest: Arc::new(AtomicU64::new(latest)),
            safe: Arc::new(AtomicU64::new(safe)),
            finalized: Arc::new(AtomicU64::new(finalized)),
            sync_status: Arc::new(RwLock::new(None)),
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

    /// Update all three tags in one call.
    ///
    /// Stores are ordered finalized → safe → latest so that readers
    /// always observe a consistent or slightly-stale view (never a
    /// latest that is behind the finalized it was published with).
    pub fn update_all(&self, latest: u64, safe: u64, finalized: u64) {
        self.finalized.store(finalized, Ordering::Release);
        self.safe.store(safe, Ordering::Release);
        self.latest.store(latest, Ordering::Release);
    }

    /// Cap all three tags to at most `ancestor`.
    ///
    /// Used during reorgs to ensure tags never reference blocks that
    /// have been removed from storage. Stores are ordered
    /// finalized → safe → latest (the same as [`update_all`]) so
    /// that readers never observe `latest < finalized` while the
    /// values are being decreased.
    ///
    /// [`update_all`]: Self::update_all
    ///
    /// # Example
    ///
    /// ```
    /// use signet_rpc::BlockTags;
    ///
    /// let tags = BlockTags::new(100, 95, 90);
    /// tags.rewind_to(92);
    /// assert_eq!(tags.latest(), 92);
    /// assert_eq!(tags.safe(), 92);
    /// assert_eq!(tags.finalized(), 90); // already below ancestor
    /// ```
    pub fn rewind_to(&self, ancestor: u64) {
        self.finalized.fetch_min(ancestor, Ordering::Release);
        self.safe.fetch_min(ancestor, Ordering::Release);
        self.latest.fetch_min(ancestor, Ordering::Release);
    }

    /// Returns `true` if the node is currently syncing.
    pub fn is_syncing(&self) -> bool {
        self.sync_status.read().expect("sync status lock poisoned").is_some()
    }

    /// Returns the current sync status, if the node is syncing.
    pub fn sync_status(&self) -> Option<SyncStatus> {
        *self.sync_status.read().expect("sync status lock poisoned")
    }

    /// Update the sync status to indicate the node is syncing.
    pub fn set_sync_status(&self, status: SyncStatus) {
        *self.sync_status.write().expect("sync status lock poisoned") = Some(status);
    }

    /// Clear the sync status, indicating the node is fully synced.
    pub fn clear_sync_status(&self) {
        *self.sync_status.write().expect("sync status lock poisoned") = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewind_to_caps_all_tags() {
        let tags = BlockTags::new(100, 95, 90);
        tags.rewind_to(92);
        assert_eq!(tags.latest(), 92);
        assert_eq!(tags.safe(), 92);
        assert_eq!(tags.finalized(), 90);
    }

    #[test]
    fn rewind_to_caps_all_above_ancestor() {
        let tags = BlockTags::new(100, 95, 90);
        tags.rewind_to(50);
        assert_eq!(tags.latest(), 50);
        assert_eq!(tags.safe(), 50);
        assert_eq!(tags.finalized(), 50);
    }

    #[test]
    fn rewind_to_noop_when_all_below() {
        let tags = BlockTags::new(100, 95, 90);
        tags.rewind_to(200);
        assert_eq!(tags.latest(), 100);
        assert_eq!(tags.safe(), 95);
        assert_eq!(tags.finalized(), 90);
    }
}

/// Error resolving a block identifier.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Storage error (e.g. failed to open a read transaction).
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Database read error.
    #[error("{0}")]
    Db(Box<dyn std::error::Error + Send + Sync>),
    /// Block hash not found.
    #[error("block hash not found: {0}")]
    HashNotFound(B256),
}
