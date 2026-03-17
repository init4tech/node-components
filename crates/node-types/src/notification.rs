use signet_extract::Extractable;
use std::sync::Arc;

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
/// # fn example<C: signet_extract::Extractable>(chain: Arc<C>) {
/// let notification = HostNotification {
///     kind: HostNotificationKind::ChainCommitted { new: chain },
///     safe_block_number: Some(100),
///     finalized_block_number: Some(90),
/// };
///
/// // Access the committed chain via the shortcut method.
/// assert!(notification.committed_chain().is_some());
/// assert!(notification.reverted_chain().is_none());
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

impl<C: Extractable> HostNotification<C> {
    /// Returns the committed chain, if any. Shortcut for
    /// `self.kind.committed_chain()`.
    pub const fn committed_chain(&self) -> Option<&Arc<C>> {
        self.kind.committed_chain()
    }

    /// Returns the reverted chain, if any. Shortcut for
    /// `self.kind.reverted_chain()`.
    pub const fn reverted_chain(&self) -> Option<&Arc<C>> {
        self.kind.reverted_chain()
    }
}

/// The kind of chain event in a [`HostNotification`].
///
/// # Examples
///
/// ```
/// # use std::sync::Arc;
/// # use signet_node_types::HostNotificationKind;
/// # fn example<C: signet_extract::Extractable>(old: Arc<C>, new: Arc<C>) {
/// let kind = HostNotificationKind::ChainReorged {
///     old: old.clone(),
///     new: new.clone(),
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
        /// The reverted chain segment.
        old: Arc<C>,
    },
    /// A chain reorg occurred: one segment was reverted and replaced by
    /// another.
    ChainReorged {
        /// The reverted chain segment.
        old: Arc<C>,
        /// The newly committed chain segment.
        new: Arc<C>,
    },
}

impl<C: Extractable> HostNotificationKind<C> {
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

    /// Returns the reverted chain, if any.
    ///
    /// Returns `Some` for [`ChainReverted`] and [`ChainReorged`], `None`
    /// for [`ChainCommitted`].
    ///
    /// [`ChainReverted`]: HostNotificationKind::ChainReverted
    /// [`ChainReorged`]: HostNotificationKind::ChainReorged
    /// [`ChainCommitted`]: HostNotificationKind::ChainCommitted
    pub const fn reverted_chain(&self) -> Option<&Arc<C>> {
        match self {
            Self::ChainReverted { old } | Self::ChainReorged { old, .. } => Some(old),
            Self::ChainCommitted { .. } => None,
        }
    }
}
