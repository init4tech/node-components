//! Shared chain state between the node and RPC layer.

use crate::{config::resolve::BlockTags, interest::NewBlockNotification};
use tokio::sync::broadcast;

/// Shared chain state between the node and RPC layer.
///
/// Combines block tag tracking and new-block notification into a single
/// unit that both the node and RPC context hold. Cloning is cheap — all
/// fields are backed by `Arc`.
///
/// # Construction
///
/// ```
/// use signet_rpc::ChainNotifier;
///
/// let notifier = ChainNotifier::new(128);
/// assert_eq!(notifier.tags().latest(), 0);
///
/// notifier.tags().set_latest(42);
/// assert_eq!(notifier.tags().latest(), 42);
/// ```
#[derive(Debug, Clone)]
pub struct ChainNotifier {
    tags: BlockTags,
    notif_tx: broadcast::Sender<NewBlockNotification>,
}

impl ChainNotifier {
    /// Create a new [`ChainNotifier`] with zeroed tags and a broadcast
    /// channel of the given capacity.
    pub fn new(channel_capacity: usize) -> Self {
        let tags = BlockTags::new(0, 0, 0);
        let (notif_tx, _) = broadcast::channel(channel_capacity);
        Self { tags, notif_tx }
    }

    /// Access the block tags.
    pub const fn tags(&self) -> &BlockTags {
        &self.tags
    }

    /// Send a new block notification.
    ///
    /// Returns `Ok(receiver_count)` or `Err` if there are no active
    /// receivers (which is not usually an error condition).
    #[allow(clippy::result_large_err)]
    pub fn send_notification(
        &self,
        notif: NewBlockNotification,
    ) -> Result<usize, broadcast::error::SendError<NewBlockNotification>> {
        self.notif_tx.send(notif)
    }

    /// Subscribe to new block notifications.
    pub fn subscribe(&self) -> broadcast::Receiver<NewBlockNotification> {
        self.notif_tx.subscribe()
    }

    /// Get a clone of the broadcast sender.
    ///
    /// Used by the subscription manager to create its own receiver.
    pub fn notif_sender(&self) -> broadcast::Sender<NewBlockNotification> {
        self.notif_tx.clone()
    }
}
