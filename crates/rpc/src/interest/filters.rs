//! Filter management for `eth_newFilter` / `eth_getFilterChanges`.

use crate::interest::{ChainEvent, InterestKind, ReorgNotification, buffer::EventBuffer};
use alloy::{
    primitives::{B256, U64},
    rpc::types::{Filter, Log},
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

/// A pending reorg notification paired with the filter's
/// `next_start_block` at the time the reorg was received.
///
/// The snapshot records which blocks the filter had already delivered,
/// so [`ActiveFilter::drain_reorgs`] can determine which removed logs
/// are relevant.
type PendingReorg = (Arc<ReorgNotification>, u64);

/// An active filter.
///
/// Records the filter details, the [`Instant`] at which the filter was last
/// polled, and the first block whose contents should be considered. Tracks
/// a `created_at` timestamp used to guard against stale reorg notifications.
#[derive(Debug, Clone)]
pub(crate) struct ActiveFilter {
    next_start_block: u64,
    last_poll_time: Instant,
    created_at: Instant,
    kind: InterestKind,
    pending_reorgs: Vec<PendingReorg>,
}

impl core::fmt::Display for ActiveFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ActiveFilter {{ next_start_block: {}, ms_since_last_poll: {}, kind: {:?}, \
             pending_reorgs: {} }}",
            self.next_start_block,
            self.last_poll_time.elapsed().as_millis(),
            self.kind,
            self.pending_reorgs.len(),
        )
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
        self.pending_reorgs.clear();
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

    /// Record a reorg notification, eagerly rewinding `next_start_block`.
    ///
    /// The notification is stored (behind an [`Arc`]) alongside a snapshot
    /// of the filter's `next_start_block` at the time of the reorg, so
    /// that [`drain_reorgs`] can later determine which removed logs the
    /// client has already seen.
    ///
    /// If `received_at` is before the filter's creation time, the reorg
    /// is silently skipped — the filter was installed after the reorg
    /// occurred and its `next_start_block` already reflects the post-reorg
    /// chain state.
    ///
    /// [`drain_reorgs`]: Self::drain_reorgs
    fn push_reorg(&mut self, reorg: Arc<ReorgNotification>, received_at: Instant) {
        if self.created_at > received_at {
            return;
        }

        let snapshot = self.next_start_block;
        self.next_start_block = self.next_start_block.min(reorg.common_ancestor + 1);
        self.pending_reorgs.push((reorg, snapshot));
    }

    /// Drain pending reorgs, returning matched removed logs with
    /// `removed: true`.
    ///
    /// For each pending reorg, only logs from blocks the filter had
    /// already delivered (block number below the snapshot) are included.
    /// Block filters return an empty vec — the Ethereum JSON-RPC spec
    /// does not define `removed` semantics for block filters.
    pub(crate) fn drain_reorgs(&mut self) -> Vec<Log> {
        let reorgs = std::mem::take(&mut self.pending_reorgs);

        let Some(filter) = self.kind.as_filter() else {
            return Vec::new();
        };

        let mut removed = Vec::new();
        for (notification, snapshot_start) in reorgs {
            for block in &notification.removed_blocks {
                if block.number >= snapshot_start {
                    continue;
                }
                for log in &block.logs {
                    if !filter.matches(log) {
                        continue;
                    }
                    removed.push(Log {
                        inner: log.clone(),
                        block_hash: Some(block.hash),
                        block_number: Some(block.number),
                        block_timestamp: None,
                        transaction_hash: None,
                        transaction_index: None,
                        log_index: None,
                        removed: true,
                    });
                }
            }
        }
        removed
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
        let now = Instant::now();
        let _ = self.filters.insert(
            id,
            ActiveFilter {
                next_start_block: current_block + 1,
                last_poll_time: now,
                created_at: now,
                kind,
                pending_reorgs: Vec::new(),
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

    /// Apply a reorg notification to all active filters.
    ///
    /// Each filter records the shared `Arc<ReorgNotification>` alongside
    /// a snapshot of its current `next_start_block`, then eagerly rewinds.
    /// Filters created after `received_at` are skipped (race guard).
    pub(crate) fn apply_reorg(&self, reorg: Arc<ReorgNotification>, received_at: Instant) {
        self.filters
            .iter_mut()
            .for_each(|mut entry| entry.value_mut().push_reorg(Arc::clone(&reorg), received_at));
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
/// Calling [`Self::new`] spawns two background workers:
/// - An OS thread that periodically cleans stale filters (using a separate
///   thread to avoid [`DashMap::retain`] deadlock).
/// - A tokio task that listens for reorg broadcasts and eagerly propagates
///   them to all active filters.
///
/// Both workers hold [`Weak`] references and self-terminate when the
/// manager is dropped.
#[derive(Debug, Clone)]
pub(crate) struct FilterManager {
    inner: Arc<FilterManagerInner>,
}

impl FilterManager {
    /// Create a new filter manager.
    ///
    /// Spawns a cleanup thread for stale filters and a tokio task that
    /// listens for [`ChainEvent::Reorg`] events and propagates reorg
    /// notifications to all active filters.
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

/// Task that listens for reorg events and propagates them to all active
/// filters.
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

            let received_at = Instant::now();
            let reorg = Arc::new(reorg);
            let Some(manager) = self.manager.upgrade() else { break };
            manager.apply_reorg(reorg, received_at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::{InterestKind, RemovedBlock};
    use alloy::primitives::{Address, Bytes, LogData, address, b256};

    fn block_filter(start: u64) -> ActiveFilter {
        ActiveFilter {
            next_start_block: start,
            last_poll_time: Instant::now(),
            created_at: Instant::now(),
            kind: InterestKind::Block,
            pending_reorgs: Vec::new(),
        }
    }

    fn log_filter(start: u64, addr: Address) -> ActiveFilter {
        ActiveFilter {
            next_start_block: start,
            last_poll_time: Instant::now(),
            created_at: Instant::now(),
            kind: InterestKind::Log(Box::new(Filter::new().address(addr))),
            pending_reorgs: Vec::new(),
        }
    }

    fn test_log(addr: Address) -> alloy::primitives::Log {
        alloy::primitives::Log { address: addr, data: LogData::new_unchecked(vec![], Bytes::new()) }
    }

    fn reorg_notification(ancestor: u64, removed: Vec<RemovedBlock>) -> ReorgNotification {
        ReorgNotification { common_ancestor: ancestor, removed_blocks: removed }
    }

    fn removed_block(number: u64, logs: Vec<alloy::primitives::Log>) -> RemovedBlock {
        RemovedBlock {
            number,
            hash: b256!("0x0000000000000000000000000000000000000000000000000000000000000001"),
            logs,
        }
    }

    #[test]
    fn push_reorg_skips_future_filters() {
        let mut f = block_filter(10);
        // received_at is before the filter was created.
        let received_at = f.created_at - Duration::from_secs(1);
        let reorg = Arc::new(reorg_notification(5, vec![]));

        f.push_reorg(reorg, received_at);

        assert!(f.pending_reorgs.is_empty());
        assert_eq!(f.next_start_block, 10);
    }

    #[test]
    fn push_reorg_rewinds_start_block() {
        let mut f = block_filter(10);
        let received_at = Instant::now();
        let reorg = Arc::new(reorg_notification(7, vec![]));

        f.push_reorg(reorg, received_at);

        assert_eq!(f.next_start_block, 8);
        assert_eq!(f.pending_reorgs.len(), 1);
    }

    #[test]
    fn drain_reorgs_matches_removed_logs() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        let mut f = log_filter(11, addr);
        let received_at = Instant::now();

        let reorg = Arc::new(reorg_notification(
            8,
            vec![removed_block(9, vec![test_log(addr)]), removed_block(10, vec![test_log(addr)])],
        ));

        f.push_reorg(reorg, received_at);
        let removed = f.drain_reorgs();

        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|l| l.removed));
        assert!(removed.iter().all(|l| l.inner.address == addr));
    }

    #[test]
    fn drain_reorgs_skips_undelivered_blocks() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        // Filter has only delivered up to block 10 (next_start = 11).
        let mut f = log_filter(11, addr);
        let received_at = Instant::now();

        // Reorg removes blocks 10, 11, 12. Only block 10 was delivered.
        let reorg = Arc::new(reorg_notification(
            9,
            vec![
                removed_block(10, vec![test_log(addr)]),
                removed_block(11, vec![test_log(addr)]),
                removed_block(12, vec![test_log(addr)]),
            ],
        ));

        f.push_reorg(reorg, received_at);
        let removed = f.drain_reorgs();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].block_number, Some(10));
    }

    #[test]
    fn drain_reorgs_cascading() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        // Filter has delivered up to block 100 (next_start = 101).
        let mut f = log_filter(101, addr);
        let received_at = Instant::now();

        // Reorg A: rewinds to 98, removes 99-100.
        let reorg_a = Arc::new(reorg_notification(
            98,
            vec![removed_block(99, vec![test_log(addr)]), removed_block(100, vec![test_log(addr)])],
        ));
        f.push_reorg(reorg_a, received_at);
        assert_eq!(f.next_start_block, 99);

        // Reorg B: rewinds to 95, removes 96-103.
        let reorg_b = Arc::new(reorg_notification(
            95,
            vec![
                removed_block(96, vec![test_log(addr)]),
                removed_block(97, vec![test_log(addr)]),
                removed_block(98, vec![test_log(addr)]),
                removed_block(99, vec![test_log(addr)]),
                removed_block(100, vec![test_log(addr)]),
                removed_block(101, vec![test_log(addr)]),
                removed_block(102, vec![test_log(addr)]),
                removed_block(103, vec![test_log(addr)]),
            ],
        ));
        f.push_reorg(reorg_b, received_at);
        assert_eq!(f.next_start_block, 96);

        let removed = f.drain_reorgs();

        // Reorg A: snapshot=101, blocks 99-100 < 101 → 2 logs.
        // Reorg B: snapshot=99, blocks 96-98 < 99 → 3 logs.
        // Blocks 99-103 from reorg B are >= 99 → skipped.
        assert_eq!(removed.len(), 5);
    }

    #[test]
    fn drain_reorgs_block_filter_empty() {
        let mut f = block_filter(10);
        let received_at = Instant::now();
        let reorg = Arc::new(reorg_notification(5, vec![removed_block(6, vec![])]));

        f.push_reorg(reorg, received_at);
        let removed = f.drain_reorgs();

        assert!(removed.is_empty());
        // But the rewind still happened.
        assert_eq!(f.next_start_block, 6);
    }

    #[test]
    fn drain_reorgs_clears_pending() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        let mut f = log_filter(11, addr);
        let received_at = Instant::now();
        let reorg = Arc::new(reorg_notification(8, vec![removed_block(9, vec![test_log(addr)])]));

        f.push_reorg(reorg, received_at);
        let first = f.drain_reorgs();
        assert_eq!(first.len(), 1);

        let second = f.drain_reorgs();
        assert!(second.is_empty());
    }

    #[test]
    fn apply_reorg_propagates_with_race_guard() {
        let inner = FilterManagerInner::new();
        inner.install_block_filter(20);
        inner.install_block_filter(30);

        let received_at = Instant::now();
        let reorg = Arc::new(reorg_notification(15, vec![]));
        inner.apply_reorg(reorg, received_at);

        inner.filters.iter().for_each(|entry| {
            assert_eq!(entry.value().pending_reorgs.len(), 1);
            assert_eq!(entry.value().next_start_block, 16);
        });

        // A filter installed after the reorg should not have it.
        let late_id = inner.install_block_filter(50);
        // Re-apply same reorg with the old timestamp.
        let reorg2 = Arc::new(reorg_notification(15, vec![]));
        inner.apply_reorg(reorg2, received_at);

        let late = inner.filters.get(&late_id).unwrap();
        // The late filter was created after received_at, so it should
        // have no pending reorgs from this event.
        assert!(late.pending_reorgs.is_empty());
        assert_eq!(late.next_start_block, 51);
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
