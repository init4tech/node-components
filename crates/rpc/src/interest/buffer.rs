//! Unified event buffer for filters and subscriptions.
//!
//! [`EventBuffer`] is a generic buffer over the block representation.
//! Filters use `EventBuffer<B256>` (block hashes), while subscriptions
//! use `EventBuffer<Header>` (full headers). Both variants share a
//! common log-event arm.

use alloy::{
    primitives::B256,
    rpc::types::{Header, Log},
};
use serde::Serialize;
use std::collections::VecDeque;

/// Buffer of chain events, parameterized by the block representation.
///
/// Filters use `EventBuffer<B256>` (block hashes), while subscriptions
/// use `EventBuffer<Header>` (full headers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum EventBuffer<B> {
    /// Log entries.
    Log(VecDeque<Log>),
    /// Block events.
    Block(VecDeque<B>),
}

impl<B> EventBuffer<B> {
    /// True if the buffer contains no events.
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of events in the buffer.
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Log(logs) => logs.len(),
            Self::Block(blocks) => blocks.len(),
        }
    }

    /// Extend this buffer with events from another buffer of the same kind.
    ///
    /// # Panics
    ///
    /// Panics if the buffers are of different kinds (log vs. block).
    pub(crate) fn extend(&mut self, other: Self) {
        match (self, other) {
            (Self::Log(a), Self::Log(b)) => a.extend(b),
            (Self::Block(a), Self::Block(b)) => a.extend(b),
            _ => panic!("attempted to extend with mismatched buffer kinds"),
        }
    }

    /// Pop the next event from the front of the buffer.
    pub(crate) fn pop_front(&mut self) -> Option<EventItem<B>> {
        match self {
            Self::Log(logs) => logs.pop_front().map(|l| EventItem::Log(Box::new(l))),
            Self::Block(blocks) => blocks.pop_front().map(EventItem::Block),
        }
    }
}

/// A single event popped from an [`EventBuffer`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum EventItem<B> {
    /// A log entry.
    Log(Box<Log>),
    /// A block event.
    Block(B),
}

// --- FilterOutput (EventBuffer<B256>) conversions ---

impl From<Vec<B256>> for EventBuffer<B256> {
    fn from(hashes: Vec<B256>) -> Self {
        Self::Block(hashes.into())
    }
}

impl From<Vec<Log>> for EventBuffer<B256> {
    fn from(logs: Vec<Log>) -> Self {
        Self::Log(logs.into())
    }
}

impl FromIterator<Log> for EventBuffer<B256> {
    fn from_iter<T: IntoIterator<Item = Log>>(iter: T) -> Self {
        Self::Log(iter.into_iter().collect())
    }
}

impl FromIterator<B256> for EventBuffer<B256> {
    fn from_iter<T: IntoIterator<Item = B256>>(iter: T) -> Self {
        Self::Block(iter.into_iter().collect())
    }
}

// --- SubscriptionBuffer (EventBuffer<Header>) conversions ---

impl From<Vec<Log>> for EventBuffer<Header> {
    fn from(logs: Vec<Log>) -> Self {
        Self::Log(logs.into())
    }
}

impl From<Vec<Header>> for EventBuffer<Header> {
    fn from(headers: Vec<Header>) -> Self {
        Self::Block(headers.into())
    }
}

impl FromIterator<Log> for EventBuffer<Header> {
    fn from_iter<T: IntoIterator<Item = Log>>(iter: T) -> Self {
        Self::Log(iter.into_iter().collect())
    }
}

impl FromIterator<Header> for EventBuffer<Header> {
    fn from_iter<T: IntoIterator<Item = Header>>(iter: T) -> Self {
        Self::Block(iter.into_iter().collect())
    }
}
