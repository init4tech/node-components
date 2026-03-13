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

/// A chain event broadcast to subscribers.
///
/// Wraps block notifications and reorg notifications into a single
/// enum so consumers can handle both through one channel.
#[derive(Debug, Clone)]
pub enum ChainEvent {
    /// A new block has been added to the chain.
    NewBlock(Box<NewBlockNotification>),
    /// A chain reorganization has occurred.
    Reorg(ReorgNotification),
}

/// A block that was removed during a chain reorganization.
#[derive(Debug, Clone)]
pub struct RemovedBlock {
    /// The block number.
    pub number: u64,
    /// The block hash.
    pub hash: alloy::primitives::B256,
    /// Logs emitted in the removed block.
    pub logs: Vec<alloy::primitives::Log>,
}

/// Notification sent when a chain reorganization is detected.
#[derive(Debug, Clone)]
pub struct ReorgNotification {
    /// The block number of the common ancestor (last block still valid).
    pub common_ancestor: u64,
    /// The blocks that were removed, ordered by block number.
    pub removed_blocks: Vec<RemovedBlock>,
}
