//! Filter and subscription management for storage-backed RPC.
//!
//! This module implements two managers that track client-registered
//! interests in chain events:
//!
//! - **[`FilterManager`]** — manages poll-based filters created via
//!   `eth_newFilter` and `eth_newBlockFilter`. Clients poll with
//!   `eth_getFilterChanges` to retrieve accumulated results.
//!
//! - **[`SubscriptionManager`]** — manages push-based subscriptions
//!   created via `eth_subscribe`. Matching events are forwarded to
//!   the client over the notification channel (WebSocket / SSE).
//!
//! # Architecture
//!
//! Both managers wrap a shared `Arc<Inner>` containing a [`DashMap`]
//! that maps client-assigned IDs to their active state. This makes
//! both types cheaply clonable — cloning just increments an `Arc`
//! reference count.
//!
//! # Resource lifecycle
//!
//! Each manager spawns a **background OS thread** that periodically
//! cleans up stale entries. The cleanup threads hold a [`Weak`]
//! reference to the `Arc<Inner>`, so they self-terminate once all
//! strong references are dropped.
//!
//! OS threads are used (rather than tokio tasks) because
//! [`DashMap::retain`] can deadlock if called from an async context
//! that also holds a `DashMap` read guard on the same shard. Running
//! cleanup on a dedicated OS thread ensures the retain lock is never
//! contended with an in-flight async handler.
//!
//! [`Weak`]: std::sync::Weak
//! [`DashMap`]: dashmap::DashMap
//! [`DashMap::retain`]: dashmap::DashMap::retain

mod buffer;
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
