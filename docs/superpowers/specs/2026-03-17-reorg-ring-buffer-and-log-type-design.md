# Reorg Ring Buffer and Removed Log Type

**Date:** 2026-03-17
**PR:** #98 (james/eng-1971)
**Reviewer feedback:** Fraser

## Context

PR #98 adds reorg handling to `eth_getFilterChanges`. Fraser's review
identified two improvements:

1. The reorg ring buffer lives in `FilterManagerInner` and is fed by
   `FilterReorgTask` via a `broadcast::Receiver`. If the receiver lags,
   reorg notifications are silently dropped — a correctness bug.

2. `RemovedBlock.logs` uses `alloy::primitives::Log`, discarding
   `transaction_hash` and `log_index` that the Ethereum spec requires on
   removed logs returned to clients.

## Change 1: Move ring buffer into ChainNotifier

### Current flow

```
ChainNotifier::send_reorg()
  -> broadcast::Sender<ChainEvent>
    -> FilterReorgTask (broadcast::Receiver)
      -> FilterManagerInner::push_reorg() (ring buffer)
```

`FilterReorgTask` listens on a `broadcast::Receiver<ChainEvent>`, filters
for `ChainEvent::Reorg`, and appends to a `VecDeque` on
`FilterManagerInner`. If the receiver lags, reorgs are lost.

### New flow

```
ChainNotifier::send_reorg()
  -> write to ring buffer (authoritative)
  -> broadcast to subscribers (push-based only)
```

Polling filters read the ring buffer directly from `ChainNotifier` at
poll time. The broadcast channel remains for push subscriptions only.

### ChainNotifier changes

Add fields:

- `reorgs: Arc<RwLock<VecDeque<(Instant, Arc<ReorgNotification>)>>>`

The `Arc` wrapper is required because `ChainNotifier` derives `Clone`
and all existing fields are `Arc`-backed.

Add constant:

- `MAX_REORG_ENTRIES: usize = 25`

Modify `send_reorg`:

- Write `Arc<ReorgNotification>` to the ring buffer (evicting oldest if
  full) **before** broadcasting on the channel. The ring buffer write is
  the authoritative store; the broadcast `Err` only means "no push
  subscribers" and does not affect the buffer. Return type stays the same
  for backward compatibility.

Add method:

- `reorgs_since(since: Instant) -> Vec<Arc<ReorgNotification>>` — scans
  the buffer under a brief read lock. Moved from `FilterManagerInner`.

### FilterManager changes

- `FilterManager::new` takes `ChainNotifier` instead of
  `&broadcast::Sender<ChainEvent>`.
- Store `ChainNotifier` (cheap clone) alongside the `Arc<FilterManagerInner>`.
- Remove `reorgs` field, `push_reorg`, and `reorgs_since` from
  `FilterManagerInner`.
- Delete `FilterReorgTask` entirely.
- Poll-time reorg reads call `chain_notifier.reorgs_since(...)` instead
  of `self.inner.reorgs_since(...)`.

### Call site (ctx.rs)

`FilterManager::new` call changes from `&chain.notif_sender()` to
`chain.clone()`.

### SubscriptionManager

No changes. Continues using the broadcast channel for push delivery.

## Change 2: Preserve rpc::types::Log in RemovedBlock

### RemovedBlock type change

```rust
// Before
pub logs: Vec<alloy::primitives::Log>

// After
pub logs: Vec<alloy::rpc::types::Log>
```

### Producer (node.rs::notify_reorg)

Stop stripping the rpc wrapper. Currently:

```rust
d.receipts.into_iter().flat_map(|r| r.receipt.logs).map(|l| l.inner).collect()
```

Becomes:

```rust
d.receipts.into_iter().flat_map(|r| r.receipt.logs).collect()
```

The `ColdReceipt` already stores `Receipt<RpcLog>` (where `RpcLog` is
re-exported from alloy as `alloy::rpc::types::Log`) with
`transaction_hash` and `log_index` populated.

### Consumer: compute_removed_logs (filters.rs)

Currently reconstructs `alloy::rpc::types::Log` with `transaction_hash:
None`. Instead, clone the log from `RemovedBlock`, set `removed: true`,
and preserve the existing fields. Filter matching calls
`filter.matches(&log.inner)` since `Filter::matches` takes
`&alloy::primitives::Log`.

### Consumer: filter_reorg_for_sub (kind.rs)

Same change — use the log directly instead of reconstructing with `None`
fields. Set `removed: true` on the cloned log.

### Tests

Update test helpers (`removed_block` in filters.rs, `test_removed_block`
in kind.rs) to construct `alloy::rpc::types::Log` instead of
`alloy::primitives::Log`.

## Files changed

| File | Change |
|------|--------|
| `crates/rpc/src/config/chain_notifier.rs` | Add ring buffer, `reorgs_since`, modify `send_reorg` |
| `crates/rpc/src/interest/filters.rs` | Remove `FilterReorgTask`, reorg fields; store `ChainNotifier`; update `compute_removed_logs`; update `FilterManager` doc comment (now one background worker, not two) |
| `crates/rpc/src/interest/mod.rs` | Change `RemovedBlock.logs` type; update module-level docs (remove `FilterReorgTask` description) |
| `crates/rpc/src/interest/kind.rs` | Update `filter_reorg_for_sub` to use rpc log directly |
| `crates/rpc/src/config/ctx.rs` | Pass `ChainNotifier` to `FilterManager::new` |
| `crates/node/src/node.rs` | Stop stripping log wrapper in `notify_reorg` |

## Ordering

Change 2 (log type) is independent of Change 1 (ring buffer move) and
can be implemented first. Change 1 depends on Change 2 only in that
test helpers need to agree on the log type.
