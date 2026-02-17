# Future Work: signet-rpc Architectural Improvements

Deferred from PR #75 review. These should be follow-up PRs after the initial fixes land.

---

## Structured JSON-RPC Error Codes

**Problem**: Most endpoints return `Result<T, String>`, losing structured error info. All errors become `-32603 Internal error`. Clients can't programmatically distinguish error types.

**Approach**:
- Add error code constants to `eth/error.rs`: `-32000` (server error), `-32001` (resource not found), `3` (execution reverted)
- Add `Into<ErrorPayload>` impl on `EthError` that maps variants to codes
- Construct `ErrorPayload` directly with custom codes (all fields are `pub` in ajj 0.6)
- Progressively migrate endpoints from `Result<T, String>` to `ResponsePayload<T, EthError>`
- Start with EVM execution endpoints (call/estimate/accessList), then query endpoints

**Files**: `crates/rpc/src/eth/error.rs`, `crates/rpc/src/eth/endpoints.rs`, `crates/rpc/src/debug/error.rs`, `crates/rpc/src/signet/error.rs`

---

## Reorg Tracking in Filter/Subscription Managers

**Problem**: No mechanism to notify RPC layer about reorgs. Filters/subscriptions can't emit `removed: true` logs per Ethereum spec.

**Approach**:
- Add `ReorgNotification { common_ancestor: u64, removed_hashes: Vec<B256> }` type
- Create `ChainEvent` enum wrapping `NewBlock` and `Reorg` variants
- Change `ChainNotifier` broadcast channel from `NewBlockNotification` to `ChainEvent`
- Node sends `ChainEvent::Reorg` during `on_host_revert()` before updating block tags
- `SubscriptionTask` handles `ChainEvent::Reorg` by emitting logs with `removed: true`
- `get_filter_changes` detects `latest < next_start_block` as reorg indicator, resets filter

**Files**: `crates/rpc/src/interest/mod.rs`, `crates/rpc/src/config/chain_notifier.rs`, `crates/rpc/src/interest/subs.rs`, `crates/rpc/src/interest/filters.rs`, `crates/rpc/src/eth/endpoints.rs`, `crates/node/src/node.rs`

---

## Best-Effort Read Consistency

**Problem**: RPC reads can span hot and cold storage at different points in time. Reorgs can make resolved block numbers stale between hot and cold queries.

**Approach**:
- Document the consistency model in `StorageRpcCtx` rustdoc
- Verify all cold queries use explicitly-resolved block numbers (already mostly the pattern)
- Detect reorg-induced staleness in `get_filter_changes` (coupled with reorg tracking above)
- Consider hash-based verification: resolve number from hot, fetch header hash, pass hash to cold for consistency check

**Files**: `crates/rpc/src/config/ctx.rs` (documentation), `crates/rpc/src/eth/endpoints.rs` (filter changes)
