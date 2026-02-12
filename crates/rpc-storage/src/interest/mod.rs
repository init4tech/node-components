//! Filter and subscription management for block/log notifications.

mod filters;
pub(crate) use filters::{FilterManager, FilterOutput};
mod kind;
pub(crate) use kind::InterestKind;
mod subs;
pub(crate) use subs::SubscriptionManager;

/// Notification sent when a new block is available in storage.
///
/// The caller constructs and sends these via a
/// [`tokio::sync::broadcast::Sender`].
#[derive(Debug, Clone)]
pub struct NewBlockNotification {
    /// The block header.
    pub header: alloy::consensus::Header,
    /// Transactions in the block.
    pub transactions: Vec<signet_storage_types::TransactionSigned>,
    /// Receipts for the block.
    pub receipts: Vec<signet_storage_types::Receipt>,
}
