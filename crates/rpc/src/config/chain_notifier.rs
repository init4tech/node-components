//! Shared chain state between the node and RPC layer.

use crate::{
    config::resolve::BlockTags,
    interest::{ChainEvent, NewBlockNotification, ReorgNotification},
};
use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
    time::Instant,
};
use tokio::sync::broadcast;

/// Maximum number of reorg notifications retained in the ring buffer.
/// At most one reorg per 12 seconds and a 5-minute stale filter TTL
/// gives a worst case of 25 entries.
const MAX_REORG_ENTRIES: usize = 25;

/// Timestamped reorg ring buffer.
type ReorgBuffer = VecDeque<(Instant, Arc<ReorgNotification>)>;

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
    reorgs: Arc<RwLock<ReorgBuffer>>,
}

impl ChainNotifier {
    /// Create a new [`ChainNotifier`] with zeroed tags and a broadcast
    /// channel of the given capacity.
    pub fn new(channel_capacity: usize) -> Self {
        let tags = BlockTags::new(0, 0, 0);
        let (notif_tx, _) = broadcast::channel(channel_capacity);
        Self { tags, notif_tx, reorgs: Arc::new(RwLock::new(VecDeque::new())) }
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
    /// Writes the notification to the authoritative ring buffer before
    /// broadcasting to push subscribers. The broadcast `Err` only means
    /// "no active push subscribers" — the ring buffer is unaffected.
    #[allow(clippy::result_large_err)]
    pub fn send_reorg(
        &self,
        notif: ReorgNotification,
    ) -> Result<usize, broadcast::error::SendError<ChainEvent>> {
        {
            let mut buf = self.reorgs.write().unwrap();
            if buf.len() >= MAX_REORG_ENTRIES {
                buf.pop_front();
            }
            buf.push_back((Instant::now(), Arc::new(notif.clone())));
        }
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

    /// Return all reorg notifications received after `since`.
    ///
    /// Clones the [`Arc`]s under a brief read lock. Used by polling
    /// filters at poll time to compute removed logs.
    pub fn reorgs_since(&self, since: Instant) -> Vec<Arc<ReorgNotification>> {
        self.reorgs
            .read()
            .unwrap()
            .iter()
            .filter(|(received_at, _)| *received_at > since)
            .map(|(_, reorg)| Arc::clone(reorg))
            .collect()
    }

    /// Get a clone of the broadcast sender.
    ///
    /// Used by the subscription manager to create its own receiver.
    pub fn notif_sender(&self) -> broadcast::Sender<ChainEvent> {
        self.notif_tx.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::ReorgNotification;
    use std::time::Duration;

    fn reorg_notification(ancestor: u64) -> ReorgNotification {
        ReorgNotification { common_ancestor: ancestor, removed_blocks: vec![] }
    }

    #[test]
    fn push_reorg_evicts_oldest() {
        let notifier = ChainNotifier::new(16);
        for i in 0..MAX_REORG_ENTRIES + 5 {
            notifier.send_reorg(reorg_notification(i as u64)).ok();
        }
        let buf = notifier.reorgs.read().unwrap();
        assert_eq!(buf.len(), MAX_REORG_ENTRIES);
        assert_eq!(buf[0].1.common_ancestor, 5);
    }

    #[test]
    fn reorgs_since_filters_by_time() {
        let notifier = ChainNotifier::new(16);
        let before = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        notifier.send_reorg(reorg_notification(10)).ok();
        let mid = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        notifier.send_reorg(reorg_notification(8)).ok();

        let all = notifier.reorgs_since(before);
        assert_eq!(all.len(), 2);

        let recent = notifier.reorgs_since(mid);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].common_ancestor, 8);
    }

    #[test]
    fn reorgs_since_skips_pre_creation_reorgs() {
        let notifier = ChainNotifier::new(16);
        notifier.send_reorg(reorg_notification(5)).ok();
        std::thread::sleep(Duration::from_millis(5));

        let after = Instant::now();
        let reorgs = notifier.reorgs_since(after);
        assert!(reorgs.is_empty());
    }
}
