//! Filter management for `eth_newFilter` / `eth_getFilterChanges`.

use crate::interest::{ChainEvent, InterestKind, buffer::EventBuffer};
use alloy::{
    primitives::{B256, U64},
    rpc::types::Filter,
};
use dashmap::{DashMap, mapref::one::RefMut};
use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::trace;

type FilterId = U64;

/// Output of a polled filter: log entries or block hashes.
pub(crate) type FilterOutput = EventBuffer<B256>;

/// An active filter.
///
/// Records the filter details, the [`Instant`] at which the filter was last
/// polled, and the first block whose contents should be considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveFilter {
    next_start_block: u64,
    last_poll_time: Instant,
    kind: InterestKind,
    reorg_watermark: Option<u64>,
}

impl core::fmt::Display for ActiveFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ActiveFilter {{ next_start_block: {}, ms_since_last_poll: {}, kind: {:?}",
            self.next_start_block,
            self.last_poll_time.elapsed().as_millis(),
            self.kind
        )?;
        if let Some(w) = self.reorg_watermark {
            write!(f, ", reorg_watermark: {w}")?;
        }
        write!(f, " }}")
    }
}

impl ActiveFilter {
    /// True if this is a block filter.
    pub(crate) const fn is_block(&self) -> bool {
        self.kind.is_block()
    }

    /// Fallible cast to a filter.
    pub(crate) const fn as_filter(&self) -> Option<&Filter> {
        self.kind.as_filter()
    }

    /// Mark the filter as having been polled at the given block.
    pub(crate) fn mark_polled(&mut self, current_block: u64) {
        self.next_start_block = current_block + 1;
        self.last_poll_time = Instant::now();
    }

    /// Get the next start block for the filter.
    pub(crate) const fn next_start_block(&self) -> u64 {
        self.next_start_block
    }

    /// Get the duration since the filter was last polled.
    pub(crate) fn time_since_last_poll(&self) -> Duration {
        self.last_poll_time.elapsed()
    }

    /// Return an empty output of the same kind as this filter.
    pub(crate) const fn empty_output(&self) -> FilterOutput {
        self.kind.empty_output()
    }

    /// Record that a reorg occurred back to this ancestor block.
    ///
    /// If multiple reorgs arrive before the filter is polled, the lowest
    /// (most conservative) watermark is kept.
    pub(crate) fn set_reorg_watermark(&mut self, common_ancestor: u64) {
        self.reorg_watermark =
            Some(self.reorg_watermark.map_or(common_ancestor, |w| w.min(common_ancestor)));
    }

    /// Reset filter state if a pending reorg affected this filter's window.
    ///
    /// Takes and clears the watermark. If the watermark is below
    /// `next_start_block`, rewinds the start block so the next poll
    /// re-fetches from just after the common ancestor. Returns the
    /// watermark value when a reset occurred, `None` otherwise.
    pub(crate) fn handle_reorg(&mut self) -> Option<u64> {
        let watermark = self.reorg_watermark.take()?;
        if watermark < self.next_start_block {
            self.next_start_block = watermark + 1;
            Some(watermark)
        } else {
            None
        }
    }
}

/// Inner logic for [`FilterManager`].
#[derive(Debug)]
pub(crate) struct FilterManagerInner {
    current_id: AtomicU64,
    filters: DashMap<FilterId, ActiveFilter>,
}

impl FilterManagerInner {
    /// Create a new filter manager.
    fn new() -> Self {
        // Start from 1, as 0 is weird in quantity encoding.
        Self { current_id: AtomicU64::new(1), filters: DashMap::new() }
    }

    /// Get the next filter ID.
    fn next_id(&self) -> FilterId {
        FilterId::from(self.current_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Get a filter by ID.
    pub(crate) fn get_mut(&self, id: FilterId) -> Option<RefMut<'_, U64, ActiveFilter>> {
        self.filters.get_mut(&id)
    }

    fn install(&self, current_block: u64, kind: InterestKind) -> FilterId {
        let id = self.next_id();
        let next_start_block = current_block + 1;
        let _ = self.filters.insert(
            id,
            ActiveFilter {
                next_start_block,
                last_poll_time: Instant::now(),
                kind,
                reorg_watermark: None,
            },
        );
        id
    }

    /// Install a new log filter.
    pub(crate) fn install_log_filter(&self, current_block: u64, filter: Filter) -> FilterId {
        self.install(current_block, InterestKind::Log(Box::new(filter)))
    }

    /// Install a new block filter.
    pub(crate) fn install_block_filter(&self, current_block: u64) -> FilterId {
        self.install(current_block, InterestKind::Block)
    }

    /// Uninstall a filter, returning the kind of filter that was uninstalled.
    pub(crate) fn uninstall(&self, id: FilterId) -> Option<(U64, ActiveFilter)> {
        self.filters.remove(&id)
    }

    /// Set a reorg watermark on all active filters.
    pub(crate) fn set_reorg_watermark_all(&self, common_ancestor: u64) {
        self.filters
            .iter_mut()
            .for_each(|mut entry| entry.value_mut().set_reorg_watermark(common_ancestor));
    }

    /// Clean stale filters that have not been polled in a while.
    fn clean_stale(&self, older_than: Duration) {
        self.filters.retain(|_, filter| filter.time_since_last_poll() < older_than);
    }
}

/// Manager for filters.
///
/// The manager tracks active filters, and periodically cleans stale filters.
/// Filters are stored in a [`DashMap`] that maps filter IDs to active filters.
/// Filter IDs are assigned sequentially, starting from 1.
///
/// Calling [`Self::new`] spawns a task that periodically cleans stale filters.
/// This task runs on a separate thread to avoid [`DashMap::retain`] deadlock.
/// See [`DashMap`] documentation for more information.
#[derive(Debug, Clone)]
pub(crate) struct FilterManager {
    inner: Arc<FilterManagerInner>,
}

impl FilterManager {
    /// Create a new filter manager.
    ///
    /// Spawns a cleanup thread for stale filters and a tokio task that
    /// listens for [`ChainEvent::Reorg`] events and propagates watermarks
    /// to all active filters.
    pub(crate) fn new(
        chain_events: &broadcast::Sender<ChainEvent>,
        clean_interval: Duration,
        age_limit: Duration,
    ) -> Self {
        let inner = Arc::new(FilterManagerInner::new());
        let manager = Self { inner };
        FilterCleanTask::new(Arc::downgrade(&manager.inner), clean_interval, age_limit).spawn();
        FilterReorgTask::new(Arc::downgrade(&manager.inner), chain_events.subscribe()).spawn();
        manager
    }
}

impl std::ops::Deref for FilterManager {
    type Target = FilterManagerInner;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

/// Task to clean up unpolled filters.
///
/// This task runs on a separate thread to avoid [`DashMap::retain`] deadlocks.
#[derive(Debug)]
struct FilterCleanTask {
    manager: Weak<FilterManagerInner>,
    sleep: Duration,
    age_limit: Duration,
}

impl FilterCleanTask {
    /// Create a new filter cleaner task.
    const fn new(manager: Weak<FilterManagerInner>, sleep: Duration, age_limit: Duration) -> Self {
        Self { manager, sleep, age_limit }
    }

    /// Run the task. This task runs on a separate thread, which ensures that
    /// [`DashMap::retain`]'s deadlock condition is not met. See [`DashMap`]
    /// documentation for more information.
    fn spawn(self) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(self.sleep);
                trace!("cleaning stale filters");
                match self.manager.upgrade() {
                    Some(manager) => manager.clean_stale(self.age_limit),
                    None => break,
                }
            }
        });
    }
}

/// Task that listens for reorg events and propagates watermarks to all
/// active filters.
///
/// Uses a [`Weak`] reference to self-terminate when the [`FilterManager`]
/// is dropped.
struct FilterReorgTask {
    manager: Weak<FilterManagerInner>,
    rx: broadcast::Receiver<ChainEvent>,
}

impl FilterReorgTask {
    const fn new(manager: Weak<FilterManagerInner>, rx: broadcast::Receiver<ChainEvent>) -> Self {
        Self { manager, rx }
    }

    /// Spawn the listener as a tokio task.
    fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(mut self) {
        loop {
            let event = match self.rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    trace!(skipped, "filter reorg listener missed notifications");
                    continue;
                }
                Err(_) => break,
            };

            let ChainEvent::Reorg(reorg) = event else { continue };

            let Some(manager) = self.manager.upgrade() else { break };
            manager.set_reorg_watermark_all(reorg.common_ancestor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::InterestKind;

    fn block_filter(start: u64) -> ActiveFilter {
        ActiveFilter {
            next_start_block: start,
            last_poll_time: Instant::now(),
            kind: InterestKind::Block,
            reorg_watermark: None,
        }
    }

    #[test]
    fn set_reorg_watermark_keeps_minimum() {
        let mut f = block_filter(10);
        f.set_reorg_watermark(8);
        assert_eq!(f.reorg_watermark, Some(8));

        // A higher watermark does not overwrite the lower one.
        f.set_reorg_watermark(9);
        assert_eq!(f.reorg_watermark, Some(8));

        // A lower watermark replaces the current one.
        f.set_reorg_watermark(5);
        assert_eq!(f.reorg_watermark, Some(5));
    }

    #[test]
    fn handle_reorg_resets_start_block() {
        let mut f = block_filter(10);
        f.set_reorg_watermark(7);

        let result = f.handle_reorg();
        assert_eq!(result, Some(7));
        assert_eq!(f.next_start_block, 8);
        assert!(f.reorg_watermark.is_none());
    }

    #[test]
    fn handle_reorg_noop_when_watermark_at_or_above_start() {
        let mut f = block_filter(10);
        f.set_reorg_watermark(10);

        let result = f.handle_reorg();
        assert!(result.is_none());
        // next_start_block unchanged.
        assert_eq!(f.next_start_block, 10);
        assert!(f.reorg_watermark.is_none());
    }

    #[test]
    fn handle_reorg_clears_watermark() {
        let mut f = block_filter(10);
        f.set_reorg_watermark(5);
        f.handle_reorg();

        // Second call returns None — watermark already consumed.
        assert!(f.handle_reorg().is_none());
    }

    #[test]
    fn set_reorg_watermark_all_propagates() {
        let inner = FilterManagerInner::new();
        inner.install_block_filter(20);
        inner.install_block_filter(30);

        inner.set_reorg_watermark_all(15);

        inner.filters.iter().for_each(|entry| {
            assert_eq!(entry.value().reorg_watermark, Some(15));
        });
    }
}

// Some code in this file has been copied and modified from reth
// <https://github.com/paradigmxyz/reth>
// The original license is included below:
//
// The MIT License (MIT)
//
// Copyright (c) 2022-2025 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
