//! Filter management for `eth_newFilter` / `eth_getFilterChanges`.

use crate::interest::{ChainEvent, InterestKind, ReorgNotification, buffer::EventBuffer};
use alloy::{
    primitives::{B256, U64},
    rpc::types::{Filter, Log},
};
use dashmap::{DashMap, mapref::one::RefMut};
use std::{
    collections::VecDeque,
    sync::{
        Arc, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::broadcast;
use tracing::trace;

type FilterId = U64;

/// Output of a polled filter: log entries or block hashes.
pub(crate) type FilterOutput = EventBuffer<B256>;

/// Maximum number of reorg notifications retained in the global ring
/// buffer. At most one reorg per 12 seconds and a 5-minute stale filter
/// TTL gives a worst case of 25 entries.
const MAX_REORG_ENTRIES: usize = 25;

/// An active filter.
///
/// Records the filter details, the [`Instant`] at which the filter was last
/// polled, and the first block whose contents should be considered.
///
/// `last_poll_time` doubles as the cursor into the global reorg ring
/// buffer — at poll time the filter scans for reorgs received after
/// this instant.
#[derive(Debug, Clone)]
pub(crate) struct ActiveFilter {
    next_start_block: u64,
    last_poll_time: Instant,
    kind: InterestKind,
}

impl core::fmt::Display for ActiveFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ActiveFilter {{ next_start_block: {}, ms_since_last_poll: {}, kind: {:?} }}",
            self.next_start_block,
            self.last_poll_time.elapsed().as_millis(),
            self.kind,
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
    }

    /// Update the poll timestamp without advancing `next_start_block`.
    ///
    /// Used on early-return paths (implicit reorg, no new blocks) where
    /// `compute_removed_logs` has already rewound `next_start_block` and
    /// we must preserve that position for the next forward scan.
    pub(crate) fn touch_poll_time(&mut self) {
        self.last_poll_time = Instant::now();
    }

    /// Get the next start block for the filter.
    pub(crate) const fn next_start_block(&self) -> u64 {
        self.next_start_block
    }

    /// Get the instant at which the filter was last polled (or created).
    pub(crate) const fn last_poll_time(&self) -> Instant {
        self.last_poll_time
    }

    /// Get the duration since the filter was last polled.
    pub(crate) fn time_since_last_poll(&self) -> Duration {
        self.last_poll_time.elapsed()
    }

    /// Return an empty output of the same kind as this filter.
    pub(crate) const fn empty_output(&self) -> FilterOutput {
        self.kind.empty_output()
    }

    /// Compute removed logs from a sequence of reorg notifications.
    ///
    /// Walks the reorgs in order, computing snapshots lazily from the
    /// filter's current [`next_start_block`]. For each reorg, only logs
    /// from blocks the filter had already delivered (block number below
    /// the snapshot) are included.
    ///
    /// Updates `next_start_block` to the rewound value so the subsequent
    /// forward scan starts from the correct position.
    ///
    /// Block filters return an empty vec — the Ethereum JSON-RPC spec
    /// does not define `removed` semantics for block filters.
    ///
    /// [`next_start_block`]: Self::next_start_block
    pub(crate) fn compute_removed_logs(&mut self, reorgs: &[Arc<ReorgNotification>]) -> Vec<Log> {
        let Some(filter) = self.kind.as_filter() else {
            for reorg in reorgs {
                self.next_start_block = self.next_start_block.min(reorg.common_ancestor + 1);
            }
            return Vec::new();
        };

        let mut removed = Vec::new();
        for notification in reorgs {
            let snapshot = self.next_start_block;
            self.next_start_block = self.next_start_block.min(notification.common_ancestor + 1);
            for block in &notification.removed_blocks {
                if block.number >= snapshot {
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
                        block_timestamp: Some(block.timestamp),
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
    reorgs: RwLock<VecDeque<(Instant, Arc<ReorgNotification>)>>,
}

impl FilterManagerInner {
    /// Create a new filter manager.
    fn new() -> Self {
        // Start from 1, as 0 is weird in quantity encoding.
        Self {
            current_id: AtomicU64::new(1),
            filters: DashMap::new(),
            reorgs: RwLock::new(VecDeque::new()),
        }
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
        let _ = self.filters.insert(
            id,
            ActiveFilter {
                next_start_block: current_block + 1,
                last_poll_time: Instant::now(),
                kind,
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

    /// Append a reorg notification to the global ring buffer.
    ///
    /// Evicts the oldest entry when the buffer is full. The
    /// [`Arc<ReorgNotification>`] is shared across all poll-time readers.
    pub(crate) fn push_reorg(&self, reorg: ReorgNotification) {
        let entry = (Instant::now(), Arc::new(reorg));
        let mut buf = self.reorgs.write().unwrap();
        if buf.len() >= MAX_REORG_ENTRIES {
            buf.pop_front();
        }
        buf.push_back(entry);
    }

    /// Return all reorg notifications received after `since`.
    ///
    /// Clones the [`Arc`]s under a brief read lock and returns them.
    pub(crate) fn reorgs_since(&self, since: Instant) -> Vec<Arc<ReorgNotification>> {
        self.reorgs
            .read()
            .unwrap()
            .iter()
            .filter(|(received_at, _)| *received_at > since)
            .map(|(_, reorg)| Arc::clone(reorg))
            .collect()
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
/// Reorg notifications are stored in a global ring buffer (capped at
/// [`MAX_REORG_ENTRIES`]). Filters compute their removed logs lazily at
/// poll time by scanning the buffer for entries received since the last
/// poll.
///
/// Calling [`Self::new`] spawns two background workers:
/// - An OS thread that periodically cleans stale filters (using a separate
///   thread to avoid [`DashMap::retain`] deadlock).
/// - A tokio task that listens for reorg broadcasts and appends them to
///   the global ring buffer.
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
    /// listens for [`ChainEvent::Reorg`] events and appends them to the
    /// global ring buffer.
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

/// Task that listens for reorg events and appends them to the global
/// ring buffer.
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
            manager.push_reorg(reorg);
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
            kind: InterestKind::Block,
        }
    }

    fn log_filter(start: u64, addr: Address) -> ActiveFilter {
        ActiveFilter {
            next_start_block: start,
            last_poll_time: Instant::now(),
            kind: InterestKind::Log(Box::new(Filter::new().address(addr))),
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
            timestamp: 1_000_000 + number,
            logs,
        }
    }

    #[test]
    fn compute_removed_logs_rewinds_start_block() {
        let mut f = block_filter(10);
        let reorg = Arc::new(reorg_notification(7, vec![]));

        f.compute_removed_logs(&[reorg]);

        assert_eq!(f.next_start_block, 8);
    }

    #[test]
    fn compute_removed_logs_matches_removed() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        let mut f = log_filter(11, addr);

        let reorg = Arc::new(reorg_notification(
            8,
            vec![removed_block(9, vec![test_log(addr)]), removed_block(10, vec![test_log(addr)])],
        ));

        let removed = f.compute_removed_logs(&[reorg]);

        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|l| l.removed));
        assert!(removed.iter().all(|l| l.inner.address == addr));
    }

    #[test]
    fn compute_removed_logs_skips_undelivered_blocks() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        // Filter has only delivered up to block 10 (next_start = 11).
        let mut f = log_filter(11, addr);

        // Reorg removes blocks 10, 11, 12. Only block 10 was delivered.
        let reorg = Arc::new(reorg_notification(
            9,
            vec![
                removed_block(10, vec![test_log(addr)]),
                removed_block(11, vec![test_log(addr)]),
                removed_block(12, vec![test_log(addr)]),
            ],
        ));

        let removed = f.compute_removed_logs(&[reorg]);

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].block_number, Some(10));
    }

    #[test]
    fn compute_removed_logs_cascading() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        // Filter has delivered up to block 100 (next_start = 101).
        let mut f = log_filter(101, addr);

        // Reorg A: rewinds to 98, removes 99-100.
        let reorg_a = Arc::new(reorg_notification(
            98,
            vec![removed_block(99, vec![test_log(addr)]), removed_block(100, vec![test_log(addr)])],
        ));

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

        let removed = f.compute_removed_logs(&[reorg_a, reorg_b]);

        // Reorg A: snapshot=101, blocks 99-100 < 101 → 2 logs.
        // Reorg B: snapshot=99, blocks 96-98 < 99 → 3 logs.
        // Blocks 99-103 from reorg B are >= 99 → skipped.
        assert_eq!(removed.len(), 5);
        assert_eq!(f.next_start_block, 96);
    }

    #[test]
    fn compute_removed_logs_block_filter_empty() {
        let mut f = block_filter(10);
        let reorg = Arc::new(reorg_notification(5, vec![removed_block(6, vec![])]));

        let removed = f.compute_removed_logs(&[reorg]);

        assert!(removed.is_empty());
        // But the rewind still happened.
        assert_eq!(f.next_start_block, 6);
    }

    #[test]
    fn push_reorg_evicts_oldest() {
        let inner = FilterManagerInner::new();
        for i in 0..MAX_REORG_ENTRIES + 5 {
            inner.push_reorg(reorg_notification(i as u64, vec![]));
        }
        let buf = inner.reorgs.read().unwrap();
        assert_eq!(buf.len(), MAX_REORG_ENTRIES);
        // Oldest surviving entry should be the 6th push (index 5).
        assert_eq!(buf[0].1.common_ancestor, 5);
    }

    #[test]
    fn reorgs_since_filters_by_time() {
        let inner = FilterManagerInner::new();
        let before = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        inner.push_reorg(reorg_notification(10, vec![]));
        let mid = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        inner.push_reorg(reorg_notification(8, vec![]));

        let all = inner.reorgs_since(before);
        assert_eq!(all.len(), 2);

        let recent = inner.reorgs_since(mid);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].common_ancestor, 8);
    }

    #[test]
    fn reorgs_since_skips_pre_creation_reorgs() {
        let inner = FilterManagerInner::new();
        inner.push_reorg(reorg_notification(5, vec![]));
        std::thread::sleep(Duration::from_millis(5));

        let id = inner.install_block_filter(20);
        let filter = inner.filters.get(&id).unwrap();

        let reorgs = inner.reorgs_since(filter.last_poll_time);
        assert!(reorgs.is_empty());
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
