//! Shared chain state between the node and RPC layer.

use crate::{
    config::resolve::BlockTags,
    interest::{ChainEvent, NewBlockNotification, ReorgNotification},
};
use tokio::sync::broadcast;

/// Shared chain state between the node and RPC layer.
///
/// Combines block tag tracking and chain event notification into a single
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
    notif_tx: broadcast::Sender<ChainEvent>,
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
    pub fn send_new_block(
        &self,
        notif: NewBlockNotification,
    ) -> Result<usize, broadcast::error::SendError<ChainEvent>> {
        self.send_event(ChainEvent::NewBlock(Box::new(notif)))
    }

    /// Send a reorg notification.
    ///
    /// Returns `Ok(receiver_count)` or `Err` if there are no active
    /// receivers (which is not usually an error condition).
    #[allow(clippy::result_large_err)]
    pub fn send_reorg(
        &self,
        notif: ReorgNotification,
    ) -> Result<usize, broadcast::error::SendError<ChainEvent>> {
        self.send_event(ChainEvent::Reorg(notif))
    }

    /// Send a chain event to subscribers.
    #[allow(clippy::result_large_err)]
    fn send_event(
        &self,
        event: ChainEvent,
    ) -> Result<usize, broadcast::error::SendError<ChainEvent>> {
        self.notif_tx.send(event)
    }

    /// Subscribe to chain events.
    pub fn subscribe(&self) -> broadcast::Receiver<ChainEvent> {
        self.notif_tx.subscribe()
    }

    /// Get a clone of the broadcast sender.
    ///
    /// Used by the subscription manager to create its own receiver.
    pub fn notif_sender(&self) -> broadcast::Sender<ChainEvent> {
        self.notif_tx.clone()
    }
}
