//! Filter management for `eth_newFilter` / `eth_getFilterChanges`.

use crate::interest::{InterestKind, buffer::EventBuffer};
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
}

impl core::fmt::Display for ActiveFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ActiveFilter {{ next_start_block: {}, ms_since_last_poll: {}, kind: {:?} }}",
            self.next_start_block,
            self.last_poll_time.elapsed().as_millis(),
            self.kind
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
        let _ = self
            .filters
            .insert(id, ActiveFilter { next_start_block, last_poll_time: Instant::now(), kind });
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
    /// Create a new filter manager. Spawn a task to clean stale filters.
    pub(crate) fn new(clean_interval: Duration, age_limit: Duration) -> Self {
        let inner = Arc::new(FilterManagerInner::new());
        let manager = Self { inner };
        FilterCleanTask::new(Arc::downgrade(&manager.inner), clean_interval, age_limit).spawn();
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
