# Reorg Ring Buffer & Log Type Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Address Fraser's PR #98 review: move the reorg ring buffer from `FilterManagerInner` to `ChainNotifier`, and change `RemovedBlock.logs` to `Vec<alloy::rpc::types::Log>` for Ethereum spec compliance.

**Architecture:** Two independent changes. Change 2 (log type) is implemented first because it's simpler and Change 1's tests must agree on the log type. Change 1 moves the ring buffer upstream into `ChainNotifier`, deletes `FilterReorgTask`, and has `FilterManager` delegate `reorgs_since` to `ChainNotifier`.

**Tech Stack:** Rust, alloy, tokio broadcast, signet-rpc, signet-node

**Spec:** `docs/superpowers/specs/2026-03-17-reorg-ring-buffer-and-log-type-design.md`

---

## Chunk 1: Preserve rpc::types::Log in RemovedBlock

### Task 1: Change RemovedBlock.logs type and update producer

**Files:**
- Modify: `crates/rpc/src/interest/mod.rs:87`
- Modify: `crates/node/src/node.rs:367-381`

- [ ] **Step 1: Change `RemovedBlock.logs` type**

In `crates/rpc/src/interest/mod.rs`, change:

```rust
    /// Logs emitted in the removed block.
    pub logs: Vec<alloy::primitives::Log>,
```

To:

```rust
    /// Logs emitted in the removed block.
    ///
    /// Uses the RPC log type to preserve `transaction_hash` and
    /// `log_index` from the original receipts, as required by the
    /// Ethereum JSON-RPC spec for removed logs.
    pub logs: Vec<alloy::rpc::types::Log>,
```

- [ ] **Step 2: Update the producer in node.rs**

In `crates/node/src/node.rs`, the `notify_reorg` method currently strips the RPC wrapper. Change:

```rust
                let logs =
                    d.receipts.into_iter().flat_map(|r| r.receipt.logs).map(|l| l.inner).collect();
```

To:

```rust
                let logs = d.receipts.into_iter().flat_map(|r| r.receipt.logs).collect();
```

- [ ] **Step 3: Verify compilation**

Run: `cargo clippy -p signet-node --all-features --all-targets 2>&1 | head -30`

Expected: Compilation errors in `filters.rs` and `kind.rs` because they
still construct `Log` from `primitives::Log` — these are fixed in Tasks 2
and 3.

### Task 2: Update compute_removed_logs consumer (filters.rs)

**Files:**
- Modify: `crates/rpc/src/interest/filters.rs:117-151`
- Modify: `crates/rpc/src/interest/filters.rs:388-403` (test helpers)

- [ ] **Step 1: Update `compute_removed_logs`**

In `crates/rpc/src/interest/filters.rs`, replace the body of
`compute_removed_logs` (lines 117-151). The key changes:
- `filter.matches(log)` becomes `filter.matches(&log.inner)` (rpc Log wraps primitives Log)
- Instead of constructing a new `Log`, clone the existing one and set `removed: true`

```rust
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
                    if !filter.matches(&log.inner) {
                        continue;
                    }
                    let mut log = log.clone();
                    log.removed = true;
                    removed.push(log);
                }
            }
        }
        removed
    }
```

- [ ] **Step 2: Update test helpers**

In the `mod tests` block of `filters.rs`, update `test_log` and
`removed_block` to use `alloy::rpc::types::Log`:

Replace the `test_log` helper:

```rust
    fn test_log(addr: Address, number: u64, hash: B256) -> Log {
        Log {
            inner: alloy::primitives::Log {
                address: addr,
                data: LogData::new_unchecked(vec![], Bytes::new()),
            },
            block_hash: Some(hash),
            block_number: Some(number),
            block_timestamp: Some(1_000_000 + number),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(0),
            removed: false,
        }
    }
```

Replace the `removed_block` helper:

```rust
    fn removed_block(number: u64, logs: Vec<Log>) -> RemovedBlock {
        RemovedBlock {
            number,
            hash: b256!("0x0000000000000000000000000000000000000001"),
            timestamp: 1_000_000 + number,
            logs,
        }
    }
```

Add `B256` to the test imports (already available via
`alloy::primitives`). The `Log` type comes from `super::*`.

Update all test call sites that use `test_log` to pass block number and
hash. For example, `test_log(addr)` becomes
`test_log(addr, 9, b256!("0x...0001"))` where the number and hash match
the `removed_block` they appear in. Each `removed_block` call already
passes a block number — the logs inside should use the same number and
the same hash constant.

- [ ] **Step 3: Run tests**

Run: `cargo t -p signet-rpc -- filters`

Expected: All `compute_removed_logs_*` and `push_reorg_*` and
`reorgs_since_*` tests pass.

- [ ] **Step 4: Lint**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`

Expected: Clean.

### Task 3: Update filter_reorg_for_sub consumer (kind.rs)

**Files:**
- Modify: `crates/rpc/src/interest/kind.rs:105-133`
- Modify: `crates/rpc/src/interest/kind.rs:141-158` (test helpers)

- [ ] **Step 1: Update `filter_reorg_for_sub`**

In `crates/rpc/src/interest/kind.rs`, replace the body of
`filter_reorg_for_sub`:

```rust
    pub(crate) fn filter_reorg_for_sub(&self, reorg: ReorgNotification) -> SubscriptionBuffer {
        let Some(filter) = self.as_filter() else {
            return self.empty_sub_buffer();
        };

        let logs: VecDeque<Log> = reorg
            .removed_blocks
            .iter()
            .flat_map(|block| block.logs.iter())
            .filter(|log| filter.matches(&log.inner))
            .map(|log| {
                let mut log = log.clone();
                log.removed = true;
                log
            })
            .collect();

        SubscriptionBuffer::Log(logs)
    }
```

- [ ] **Step 2: Update test helpers**

Replace the `test_log` helper:

```rust
    fn test_log(addr: Address, topic: B256, number: u64, hash: B256) -> Log {
        Log {
            inner: alloy::primitives::Log {
                address: addr,
                data: LogData::new_unchecked(vec![topic], Bytes::new()),
            },
            block_hash: Some(hash),
            block_number: Some(number),
            block_timestamp: Some(1_000_000 + number),
            transaction_hash: Some(B256::ZERO),
            transaction_index: Some(0),
            log_index: Some(0),
            removed: false,
        }
    }
```

Replace the `test_removed_block` helper:

```rust
    fn test_removed_block(
        number: u64,
        hash: B256,
        logs: Vec<Log>,
    ) -> crate::interest::RemovedBlock {
        crate::interest::RemovedBlock { number, hash, timestamp: 1_000_000 + number, logs }
    }
```

Update all test call sites: `test_log(addr, topic)` becomes
`test_log(addr, topic, number, hash)` where number and hash match the
`test_removed_block` the log appears in. The `Log` type comes from
`use super::*`. Existing imports stay the same.

- [ ] **Step 3: Run tests**

Run: `cargo t -p signet-rpc -- kind`

Expected: All `filter_reorg_for_sub_*` tests pass.

- [ ] **Step 4: Lint both crates**

Run: `cargo clippy -p signet-rpc --all-features --all-targets && cargo clippy -p signet-node --all-features --all-targets`

Expected: Clean.

- [ ] **Step 5: Format**

Run: `cargo +nightly fmt`

- [ ] **Step 6: Commit**

```bash
git add crates/rpc/src/interest/mod.rs crates/rpc/src/interest/filters.rs crates/rpc/src/interest/kind.rs crates/node/src/node.rs
git commit -m "fix: preserve tx_hash and log_index on removed logs

Change RemovedBlock.logs from Vec<alloy::primitives::Log> to
Vec<alloy::rpc::types::Log>. The producer (notify_reorg) now passes
through the full RPC log from ColdReceipt instead of stripping the
wrapper. Consumers (compute_removed_logs, filter_reorg_for_sub) clone
and set removed=true instead of reconstructing with None fields.

This satisfies the Ethereum JSON-RPC spec requirement that removed
logs include transaction_hash and log_index."
```

---

## Chunk 2: Move ring buffer into ChainNotifier

### Task 4: Add ring buffer and reorgs_since to ChainNotifier

**Files:**
- Modify: `crates/rpc/src/config/chain_notifier.rs`

- [ ] **Step 1: Add imports and constant**

In `crates/rpc/src/config/chain_notifier.rs`, add imports and constant.
Replace the existing imports:

```rust
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
```

Add constant after imports:

```rust
/// Maximum number of reorg notifications retained in the ring buffer.
/// At most one reorg per 12 seconds and a 5-minute stale filter TTL
/// gives a worst case of 25 entries.
const MAX_REORG_ENTRIES: usize = 25;
```

- [ ] **Step 2: Add `reorgs` field to `ChainNotifier`**

```rust
pub struct ChainNotifier {
    tags: BlockTags,
    notif_tx: broadcast::Sender<ChainEvent>,
    reorgs: Arc<RwLock<VecDeque<(Instant, Arc<ReorgNotification>)>>>,
}
```

- [ ] **Step 3: Update `new` to initialize the field**

```rust
    pub fn new(channel_capacity: usize) -> Self {
        let tags = BlockTags::new(0, 0, 0);
        let (notif_tx, _) = broadcast::channel(channel_capacity);
        Self { tags, notif_tx, reorgs: Arc::new(RwLock::new(VecDeque::new())) }
    }
```

- [ ] **Step 4: Update `send_reorg` to write ring buffer first**

Replace `send_reorg`:

```rust
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
```

- [ ] **Step 5: Add `reorgs_since` method**

Add after the `notif_sender` method:

```rust
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
```

- [ ] **Step 6: Update the doctest**

The construction doctest needs no changes — `ChainNotifier::new(128)`
still works. But verify:

Run: `cargo t -p signet-rpc --doc`

Expected: Doctest passes.

- [ ] **Step 7: Lint**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`

Expected: Errors in `filters.rs` (still references old ring buffer code).
That's fine — fixed in Task 5.

### Task 5: Remove ring buffer and FilterReorgTask from FilterManager

**Files:**
- Modify: `crates/rpc/src/interest/filters.rs`

- [ ] **Step 1: Add ChainNotifier import, remove broadcast import**

Replace the imports at the top of `filters.rs`:

```rust
use crate::{
    config::ChainNotifier,
    interest::{InterestKind, ReorgNotification, buffer::EventBuffer},
};
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
use tracing::trace;
```

Removed vs. current: `tokio::sync::broadcast` gone, `RwLock` gone,
`VecDeque` gone, `ChainEvent` gone from crate import. Added
`ChainNotifier`. Kept `ReorgNotification` (used by `compute_removed_logs`).

- [ ] **Step 2: Remove `MAX_REORG_ENTRIES` constant**

Delete:

```rust
/// Maximum number of reorg notifications retained in the global ring
/// buffer. At most one reorg per 12 seconds and a 5-minute stale filter
/// TTL gives a worst case of 25 entries.
const MAX_REORG_ENTRIES: usize = 25;
```

- [ ] **Step 3: Remove reorg fields from `FilterManagerInner`**

Update the struct:

```rust
/// Inner logic for [`FilterManager`].
#[derive(Debug)]
pub(crate) struct FilterManagerInner {
    current_id: AtomicU64,
    filters: DashMap<FilterId, ActiveFilter>,
}
```

Update `new`:

```rust
    fn new() -> Self {
        // Start from 1, as 0 is weird in quantity encoding.
        Self {
            current_id: AtomicU64::new(1),
            filters: DashMap::new(),
        }
    }
```

- [ ] **Step 4: Remove `push_reorg` and `reorgs_since` from `FilterManagerInner`**

Delete these two methods entirely (lines 211-235 in the current file).

- [ ] **Step 5: Update `FilterManager` struct and `new`**

Replace the `FilterManager` struct, its doc comment, and `new`:

```rust
/// Manager for filters.
///
/// The manager tracks active filters, and periodically cleans stale filters.
/// Filters are stored in a [`DashMap`] that maps filter IDs to active filters.
/// Filter IDs are assigned sequentially, starting from 1.
///
/// Reorg notifications are read from the [`ChainNotifier`]'s ring buffer
/// at poll time. Filters compute their removed logs lazily by scanning
/// for entries received since the last poll.
///
/// Calling [`Self::new`] spawns an OS thread that periodically cleans
/// stale filters (using a separate thread to avoid [`DashMap::retain`]
/// deadlock). The worker holds a [`Weak`] reference and self-terminates
/// when the manager is dropped.
#[derive(Debug, Clone)]
pub(crate) struct FilterManager {
    inner: Arc<FilterManagerInner>,
    chain: ChainNotifier,
}

impl FilterManager {
    /// Create a new filter manager.
    ///
    /// Spawns a cleanup thread for stale filters. Reorg notifications
    /// are read directly from the [`ChainNotifier`]'s ring buffer at
    /// poll time — no background listener is needed.
    pub(crate) fn new(
        chain: ChainNotifier,
        clean_interval: Duration,
        age_limit: Duration,
    ) -> Self {
        let inner = Arc::new(FilterManagerInner::new());
        let manager = Self { inner, chain };
        FilterCleanTask::new(Arc::downgrade(&manager.inner), clean_interval, age_limit).spawn();
        manager
    }

    /// Return all reorg notifications received after `since`.
    ///
    /// Delegates to [`ChainNotifier::reorgs_since`].
    pub(crate) fn reorgs_since(&self, since: Instant) -> Vec<Arc<ReorgNotification>> {
        self.chain.reorgs_since(since)
    }
}

impl std::ops::Deref for FilterManager {
    type Target = FilterManagerInner;

    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}
```

The `Deref` impl is preserved from the existing code — call sites like
`fm.get_mut(id)`, `fm.install_log_filter(...)`, etc. in `endpoints.rs`
depend on it to reach `FilterManagerInner` methods.

- [ ] **Step 6: Delete `FilterReorgTask`**

Delete the entire `FilterReorgTask` struct and impl block (lines 327-364
in the current file).

- [ ] **Step 7: Update test module**

The tests `push_reorg_evicts_oldest`, `reorgs_since_filters_by_time`,
and `reorgs_since_skips_pre_creation_reorgs` test ring buffer behavior
that now lives on `ChainNotifier`. Move them to a new `#[cfg(test)] mod
tests` block in `chain_notifier.rs` (Task 6). For now, delete them from
`filters.rs`.

Remove these three tests and update the test imports. The remaining tests
only need:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::{InterestKind, RemovedBlock};
    use alloy::primitives::{Address, Bytes, LogData, address, b256};
```

(Same as before — the `RemovedBlock` import stays; the `Log` type comes
from `super::*`.)

Also update `removed_block` test helper to match the new log type from
Task 2 (should already be done if Task 2 was completed first).

- [ ] **Step 8: Lint**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`

Expected: Errors in `ctx.rs` (wrong `FilterManager::new` signature).
Fixed in Task 7.

### Task 6: Add ring buffer tests to ChainNotifier

**Files:**
- Modify: `crates/rpc/src/config/chain_notifier.rs`

- [ ] **Step 1: Add test module**

Add at the bottom of `chain_notifier.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::ReorgNotification;

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
```

Add `Duration` to the existing `std` import in the file's main imports
(it's already brought in by the test module via `super::*` if we add it
to the main imports). Alternatively, add `use std::time::Duration;`
inside the test module.

- [ ] **Step 2: Run tests**

Run: `cargo t -p signet-rpc -- chain_notifier`

Expected: All three new tests pass.

### Task 7: Update call site in ctx.rs

**Files:**
- Modify: `crates/rpc/src/config/ctx.rs:104-108`

- [ ] **Step 1: Update `FilterManager::new` call**

Replace:

```rust
        let filter_manager = FilterManager::new(
            &chain.notif_sender(),
            config.stale_filter_ttl,
            config.stale_filter_ttl,
        );
```

With:

```rust
        let filter_manager = FilterManager::new(
            chain.clone(),
            config.stale_filter_ttl,
            config.stale_filter_ttl,
        );
```

- [ ] **Step 2: Lint**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`

Expected: Clean.

### Task 8: Update module-level docs

**Files:**
- Modify: `crates/rpc/src/interest/mod.rs:34-37`

- [ ] **Step 1: Remove FilterReorgTask mention from module docs**

Replace lines 34-37:

```rust
//! [`FilterManager`] additionally spawns a tokio task that listens
//! for [`ChainEvent::Reorg`] broadcasts and eagerly propagates reorg
//! notifications to all active filters. This task does not call
//! `retain`, so it is safe to run in an async context.
```

With:

```rust
//! [`FilterManager`] reads reorg notifications directly from the
//! [`ChainNotifier`]'s ring buffer at poll time — no background
//! listener task is needed.
//!
//! [`ChainNotifier`]: crate::ChainNotifier
```

- [ ] **Step 2: Full lint, test, format**

Run:

```bash
cargo clippy -p signet-rpc --all-features --all-targets
cargo clippy -p signet-node --all-features --all-targets
cargo t -p signet-rpc
cargo +nightly fmt
```

Expected: All clean, all tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/rpc/src/config/chain_notifier.rs crates/rpc/src/interest/filters.rs crates/rpc/src/interest/mod.rs crates/rpc/src/config/ctx.rs
git commit -m "refactor: move reorg ring buffer from FilterManager to ChainNotifier

ChainNotifier now writes to an authoritative ring buffer before
broadcasting reorg events. Polling filters read it directly via
reorgs_since(), eliminating the broadcast::Receiver lag bug where
FilterReorgTask could silently drop reorg notifications.

Deletes FilterReorgTask entirely. FilterManager::new now takes a
ChainNotifier instead of a broadcast::Sender."
```

- [ ] **Step 4: Final verification**

Run full test suite:

```bash
cargo t -p signet-rpc
cargo t -p signet-node
```

Expected: All tests pass.
