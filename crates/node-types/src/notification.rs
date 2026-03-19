use std::sync::Arc;

/// The range of host blocks that were reverted by the host chain.
///
/// # Panics
///
/// [`RevertRange::new`] panics if `first > tip` or `first == 0`.
///
/// # Examples
///
/// ```
/// # use signet_node_types::RevertRange;
/// let range = RevertRange::new(10, 15);
/// assert_eq!(range.first(), 10);
/// assert_eq!(range.tip(), 15);
/// assert_eq!(range.len(), 6);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevertRange {
    /// Block number of the first reverted block.
    first: u64,
    /// Block number of the last reverted block (tip).
    tip: u64,
}

impl RevertRange {
    /// Create a new revert range. Panics if `first > tip` or `first == 0`.
    pub fn new(first: u64, tip: u64) -> Self {
        assert!(first > 0, "RevertRange: first block number must be > 0");
        assert!(first <= tip, "RevertRange: first ({first}) must be <= tip ({tip})");
        Self { first, tip }
    }

    /// The first (lowest) reverted block number.
    pub const fn first(&self) -> u64 {
        self.first
    }

    /// The tip (highest) reverted block number.
    pub const fn tip(&self) -> u64 {
        self.tip
    }

    /// The number of blocks in this range. Always >= 1.
    pub const fn len(&self) -> u64 {
        self.tip - self.first + 1
    }

    /// Returns `false`. A valid `RevertRange` is never empty.
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// A notification from the host chain, bundling a chain event with
/// point-in-time block tag data. The safe/finalized block numbers are
/// intentionally snapshotted at notification creation time rather than
/// fetched live, because rollup safe/finalized tags are only updated
/// after block processing completes.
///
/// # Examples
///
/// ```
/// # use std::sync::Arc;
/// # use signet_node_types::{HostNotification, HostNotificationKind};
/// # fn example<C: core::fmt::Debug>(chain: Arc<C>) {
/// let notification = HostNotification {
///     kind: HostNotificationKind::ChainCommitted { new: chain },
///     safe_block_number: Some(100),
///     finalized_block_number: Some(90),
/// };
///
/// // Access the committed chain via the shortcut method.
/// assert!(notification.committed_chain().is_some());
/// assert!(notification.revert_range().is_none());
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct HostNotification<C> {
    /// The chain event (commit, revert, or reorg).
    pub kind: HostNotificationKind<C>,
    /// The host chain "safe" block number at the time of this notification.
    pub safe_block_number: Option<u64>,
    /// The host chain "finalized" block number at the time of this
    /// notification.
    pub finalized_block_number: Option<u64>,
}

impl<C> HostNotification<C> {
    /// Returns the committed chain, if any. Shortcut for
    /// `self.kind.committed_chain()`.
    pub const fn committed_chain(&self) -> Option<&Arc<C>> {
        self.kind.committed_chain()
    }

    /// Returns the revert range, if any. Shortcut for
    /// `self.kind.revert_range()`.
    pub const fn revert_range(&self) -> Option<RevertRange> {
        self.kind.revert_range()
    }
}

/// The kind of chain event in a [`HostNotification`].
///
/// Only committed chain segments carry full block and receipt data.
/// Reverted segments are represented as a [`RevertRange`] containing
/// only the first and tip block numbers.
///
/// # Examples
///
/// ```
/// # use std::sync::Arc;
/// # use signet_node_types::{HostNotificationKind, RevertRange};
/// # fn example<C: core::fmt::Debug>(new: Arc<C>) {
/// let kind = HostNotificationKind::ChainReorged {
///     old: RevertRange::new(5, 10),
///     new,
/// };
/// # }
/// ```
#[derive(Debug, Clone)]
pub enum HostNotificationKind<C> {
    /// A new chain segment was committed.
    ChainCommitted {
        /// The newly committed chain segment.
        new: Arc<C>,
    },
    /// A chain segment was reverted.
    ChainReverted {
        /// The range of reverted host blocks.
        old: RevertRange,
    },
    /// A chain reorg occurred: one segment was reverted and replaced by
    /// another.
    ChainReorged {
        /// The range of reverted host blocks.
        old: RevertRange,
        /// The newly committed chain segment.
        new: Arc<C>,
    },
}

impl<C> HostNotificationKind<C> {
    /// Returns the committed chain, if any.
    ///
    /// Returns `Some` for [`ChainCommitted`] and [`ChainReorged`], `None`
    /// for [`ChainReverted`].
    ///
    /// [`ChainCommitted`]: HostNotificationKind::ChainCommitted
    /// [`ChainReorged`]: HostNotificationKind::ChainReorged
    /// [`ChainReverted`]: HostNotificationKind::ChainReverted
    pub const fn committed_chain(&self) -> Option<&Arc<C>> {
        match self {
            Self::ChainCommitted { new } | Self::ChainReorged { new, .. } => Some(new),
            Self::ChainReverted { .. } => None,
        }
    }

    /// Returns the revert range, if any.
    ///
    /// Returns `Some` for [`ChainReverted`] and [`ChainReorged`], `None`
    /// for [`ChainCommitted`].
    ///
    /// [`ChainReverted`]: HostNotificationKind::ChainReverted
    /// [`ChainReorged`]: HostNotificationKind::ChainReorged
    /// [`ChainCommitted`]: HostNotificationKind::ChainCommitted
    pub const fn revert_range(&self) -> Option<RevertRange> {
        match self {
            Self::ChainReverted { old } | Self::ChainReorged { old, .. } => Some(*old),
            Self::ChainCommitted { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_range_valid() {
        let range = RevertRange::new(1, 1);
        assert_eq!(range.first(), 1);
        assert_eq!(range.tip(), 1);
        assert_eq!(range.len(), 1);

        let range = RevertRange::new(5, 10);
        assert_eq!(range.len(), 6);
    }

    #[test]
    #[should_panic(expected = "first block number must be > 0")]
    fn revert_range_zero_first() {
        RevertRange::new(0, 5);
    }

    #[test]
    #[should_panic(expected = "first (10) must be <= tip (5)")]
    fn revert_range_inverted() {
        RevertRange::new(10, 5);
    }
}
