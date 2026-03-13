# RPC Host Notifier: HostNotifier over Alloy RPC Provider

**Date:** 2026-03-13
**Status:** Draft

## Prerequisite

This spec builds on the **Host Context Adapter** design
(`docs/superpowers/specs/2026-03-13-host-context-adapter-design.md`) and its
implementation plan. That work introduces:

- The `HostNotifier` trait and `HostNotification`/`HostNotificationKind` types
  in `signet-node-types`
- The `signet-host-reth` crate (reth ExEx implementation)
- The refactor of `signet-node` to be generic over `HostNotifier`

This spec assumes that work is complete. It modifies the trait types
introduced there and adds a second backend implementation.

## Goal

Implement `HostNotifier` backed by an alloy RPC provider over WebSocket,
enabling signet-node to follow a host chain without embedding a reth node.
This enables lightweight standalone deployment and multi-host flexibility
(any EL client with WS support).

## Scope

This design covers:

1. **`signet-host-rpc`** — a new crate implementing `HostNotifier` over an
   alloy WS provider
2. **Trait changes to `signet-node-types`** — modifying `HostNotificationKind`
   to remove the outer `Arc` and replace reverted chain data with a lightweight
   `RevertedRange`
3. **Trait changes to `signet-extract`** — replacing the tuple return in
   `blocks_and_receipts` with a named struct, and `&Vec<R>` with `&[R]`

Out of scope:
- Alias oracle for the RPC backend (sync trait would need async redesign)
- Changes to `signet-host-reth` beyond adapting to the trait changes
- Transaction pool or blob cacher integration

## Architecture

```
signet-extract             signet-node-types          signet-host-reth
(BlockAndReceipts struct)  (RevertedRange, no Arc)    (adapted to trait changes)
       \                        |                        /
        \                       |                       /
         v                      v                      v
                         signet-node
                   (generic over HostNotifier)
                              ^
                              |
                       signet-host-rpc
                   (alloy WS provider impl)
```

## Design

### Trait Changes

#### `signet-extract`: `BlockAndReceipts` struct

The `Extractable` trait's `blocks_and_receipts` method currently returns
`impl Iterator<Item = (&Self::Block, &Vec<Self::Receipt>)>`. This changes
to a named struct with a slice reference:

```rust
/// A block with its associated receipts.
pub struct BlockAndReceipts<'a, B, R> {
    /// The block.
    pub block: &'a B,
    /// The receipts for this block's transactions.
    pub receipts: &'a [R],
}

pub trait Extractable: Debug + Sync {
    type Block: BlockHeader + HasTxns + Debug + Sync;
    type Receipt: TxReceipt<Log = Log> + Debug + Sync;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>>;

    // provided methods unchanged
}
```

This affects the reth shim in `signet-blobber` and the `SealedBlock` impl —
both must return `BlockAndReceipts` instead of tuples.

#### `signet-node-types`: `HostNotificationKind` changes

Two changes:

1. **Remove outer `Arc`** from chain variants. Each backend handles sharing
   internally (reth wraps its chain in `Arc` inside `RethChain`; the RPC
   backend uses `Arc<RpcBlock>` inside `RpcChainSegment`).

2. **Replace reverted chain data with `RevertedRange`.** The RPC backend
   cannot cheaply provide full reverted block data. Since `signet-node`'s
   revert handling only uses block number ranges, this is sufficient.

```rust
/// A range of reverted block numbers.
#[derive(Debug, Clone, Copy)]
pub struct RevertedRange {
    /// The first (lowest) reverted block number.
    pub first: u64,
    /// The last (highest) reverted block number.
    pub last: u64,
}

pub enum HostNotificationKind<C> {
    /// A new chain segment was committed.
    ChainCommitted { new: C },
    /// A chain segment was reverted.
    ChainReverted { reverted: RevertedRange },
    /// A reorg: one segment was reverted, another committed.
    ChainReorged { reverted: RevertedRange, new: C },
}
```

Accessor methods:
- `committed_chain() -> Option<&C>` — returns `Some` for `ChainCommitted`
  and `ChainReorged`
- `reverted_range() -> Option<&RevertedRange>` — returns `Some` for
  `ChainReverted` and `ChainReorged`

The reth backend populates `RevertedRange` from the reverted chain's
`first_number()` and `tip_number()` before discarding the full chain data.

`signet-node`'s `on_host_revert` uses these values for:
- Early return when `reverted.last <= host_deploy_height` (revert
  is entirely before the rollup's deployment)
- Determining the block range to revert in storage
- Logging the reverted range

### `signet-host-rpc` Crate

#### Chain Segment Types

```rust
/// A block with its receipts, fetched via RPC.
#[derive(Debug)]
pub struct RpcBlock {
    /// The sealed block.
    pub block: SealedBlock<TransactionSigned>,
    /// The receipts for this block's transactions.
    pub receipts: Vec<TransactionReceipt>,
}

/// A chain segment fetched via RPC.
#[derive(Debug)]
pub struct RpcChainSegment {
    /// Blocks and their receipts, ordered by block number ascending.
    blocks: Vec<Arc<RpcBlock>>,
}
```

`RpcChainSegment` implements `Extractable` with O(1) overrides for
`first_number`, `tip_number`, `len`, and `is_empty`. The `Block` associated
type is `SealedBlock<TransactionSigned>` (already satisfies `BlockHeader +
HasTxns`). The `Receipt` associated type should be whichever alloy receipt
type satisfies `TxReceipt<Log = Log>` — likely `alloy::consensus::Receipt`
rather than the RPC wrapper `TransactionReceipt`. The exact type must be
verified during implementation; RPC responses will be converted at fetch
time.

Blocks are wrapped in `Arc<RpcBlock>` to allow cheap sharing between the
notifier's internal buffer and emitted chain segments.

#### `RpcHostNotifier`

```rust
pub struct RpcHostNotifier<P> {
    /// The alloy provider (must support subscriptions).
    provider: P,

    /// Subscription stream of new block headers.
    header_sub: SubscriptionStream<Header>,

    /// Recent blocks for reorg detection and cache.
    block_buffer: VecDeque<Arc<RpcBlock>>,

    /// Maximum entries in the block buffer.
    buffer_capacity: usize,

    /// Cached safe block number, refreshed at epoch boundaries.
    cached_safe: Option<u64>,

    /// Cached finalized block number, refreshed at epoch boundaries.
    cached_finalized: Option<u64>,

    /// Last epoch number for which safe/finalized were fetched.
    last_tag_epoch: Option<u64>,

    /// If set, backfill from this block number before processing
    /// subscription events.
    backfill_from: Option<u64>,

    /// Max blocks per backfill batch.
    backfill_batch_size: u64,
}
```

Generic over `P` with bounds requiring alloy provider + subscription
support. The exact trait bounds depend on alloy's trait hierarchy and will
be determined during implementation.

##### `next_notification` Flow

1. **Drain backfill.** If `backfill_from` is set, fetch a batch of blocks
   (concurrently) from that point toward the current tip. Build an
   `RpcChainSegment`, emit `ChainCommitted`. Advance `backfill_from`.
   Repeat until backfill is drained.

2. **Await subscription.** Wait for the next `newHeads` header.

3. **Check parent hash continuity.** Compare the new block's `parent_hash`
   against the hash of our tip block in `block_buffer`.

4. **Normal case (parent matches).** Fetch the full block + receipts for
   any blocks between our tip and the new head (handles small gaps from
   missed subscription events). Build `RpcChainSegment`, emit
   `ChainCommitted`.

5. **Reorg case (parent mismatch).** Walk backward through `block_buffer`
   to find the fork point — the highest block number where our buffered
   hash matches the actual chain. If the fork point is not in the buffer,
   return `RpcHostError::ReorgTooDeep`. Otherwise, build `RevertedRange`
   from the fork point to our old tip, fetch the new blocks from the fork
   point to the new head, and emit `ChainReorged`.

6. **Update block tags.** Derive the epoch number from the block's
   timestamp (`(timestamp - genesis_timestamp) / (12 * 32)`). If the epoch
   changed since `last_tag_epoch`, fetch safe and finalized block numbers
   via `eth_getBlockByNumber("safe")` and
   `eth_getBlockByNumber("finalized")`.

7. **Update buffer.** Push newly fetched blocks (as `Arc<RpcBlock>`) into
   `block_buffer`. On reorg, remove invalidated entries first. Evict
   oldest entries if over `buffer_capacity`.

##### `set_head`

Sets `backfill_from` to the given block number. The next call to
`next_notification` will drain the backfill before processing subscription
events. Blocks are fetched concurrently in batches of `backfill_batch_size`.

##### `set_backfill_thresholds`

Sets `backfill_batch_size`.

##### `send_finished_height`

No-op. There is no ExEx to notify. Returns `Ok(())`.

#### Builder

```rust
pub struct RpcHostNotifierBuilder<P> { .. }

impl<P> RpcHostNotifierBuilder<P> {
    pub fn new(provider: P) -> Self;
    pub fn with_buffer_capacity(self, capacity: usize) -> Self;     // default: 64
    pub fn with_backfill_batch_size(self, batch_size: u64) -> Self;  // default: 32
    pub async fn build(self) -> Result<RpcHostNotifier<P>, RpcHostError>;
}
```

`build()` is async — it establishes the `newHeads` WebSocket subscription.

#### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum RpcHostError {
    /// The WebSocket subscription was dropped unexpectedly.
    #[error("subscription closed")]
    SubscriptionClosed,

    /// An RPC call failed.
    #[error("rpc error: {0}")]
    Rpc(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),

    /// Reorg deeper than the block buffer.
    #[error("reorg depth {depth} exceeds buffer capacity {capacity}")]
    ReorgTooDeep { depth: u64, capacity: usize },
}
```

#### Dependencies

```toml
[dependencies]
signet-node-types.workspace = true
signet-extract.workspace = true
signet-types.workspace = true

alloy.workspace = true
tokio.workspace = true
tracing.workspace = true
thiserror.workspace = true
```

No reth dependencies.

### Adaptations to `signet-host-reth`

The existing reth backend adapts to the trait changes:

- `HostNotificationKind::ChainCommitted { new }` — pass `RethChain`
  directly instead of `Arc<RethChain>`. `RethChain` already holds
  `Arc<Chain<EthPrimitives>>` internally.
- `HostNotificationKind::ChainReverted` — build `RevertedRange` from the
  reverted chain's `first().number()` and `tip().number()`.
- `HostNotificationKind::ChainReorged` — same: `RevertedRange` from the
  old chain, `RethChain` directly for the new chain.
- `Extractable` impl returns `BlockAndReceipts` struct instead of tuple.

## What Does NOT Change

- `signet-node` — generic over `HostNotifier` (per prerequisite refactor);
  only adapts to `RevertedRange` and `BlockAndReceipts` changes
- `signet-blobber` — adapts `ExtractableChainShim` to return
  `BlockAndReceipts` (mechanical)
- `signet-node-tests` — continue using reth test utilities via
  `signet-host-reth`
- Alias oracle — out of scope; RPC deployments can use `HashSet<Address>`
  or a custom `AliasOracleFactory`

## Testing Strategy

- **Unit tests:** Mock provider that returns canned blocks and receipts
  for subscription events, verifying normal advance, gap filling, reorg
  detection, backfill, and epoch-boundary tag refresh
- **Reorg tests:** Construct sequences where parent hash mismatches trigger
  reorg detection at various depths, including `ReorgTooDeep`
- **Integration tests:** Connect to a local dev node (anvil or similar)
  with WS support to verify end-to-end subscription and block fetching

## Implementation Notes

**Receipt type verification.** The `Extractable` trait requires
`Receipt: TxReceipt<Log = Log>`. Alloy's RPC `TransactionReceipt` wraps a
consensus receipt with additional metadata. The implementor should verify
which alloy type satisfies the bound and convert at fetch time if needed.

**Subscription `Send` bound.** `RpcHostNotifier` will likely need to be
`Send` for `tokio::spawn`. The implementor should verify that alloy's
`SubscriptionStream<Header>` with the WS transport is `Send`.

**Backfill and subscription buffer.** Long backfills may cause the WS
subscription buffer to overflow. The gap-filling logic in step 4 of the
notification flow handles missed subscription events, so this is
self-healing. If the gap is larger than the block buffer, the notifier
should re-subscribe.

**Epoch calculation is approximate.** The formula
`(timestamp - genesis_timestamp) / (12 * 32)` assumes aligned genesis and
exact 12s slots. This is sufficient for reducing RPC call frequency — an
off-by-one epoch is harmless.

**Reconnection.** The spec does not define a reconnection strategy for
dropped WebSocket connections. `SubscriptionClosed` is surfaced to the
caller, who is responsible for reconstructing the notifier. A reconnection
wrapper could be added later.

## Design Decisions

| Decision | Rationale |
|----------|-----------|
| WebSocket required | Production chain follower needs low-latency push notifications; polling adds unacceptable delay |
| Generic over provider | Supports any alloy transport with pubsub; not locked to a specific WS implementation |
| Minimal reorg signal (`RevertedRange`) | Avoids expensive RPC fetches for reverted blocks; signet-node only needs block number ranges for revert handling |
| No outer `Arc` in `HostNotificationKind` | Backends handle sharing internally; avoids double-Arc for RPC backend |
| Individual block buffer (`VecDeque<Arc<RpcBlock>>`) | Simple reorg detection via hash lookup; shares blocks cheaply with emitted segments |
| Epoch-aware safe/finalized caching | Safe and finalized only advance at epoch boundaries (~6.4 min); avoids unnecessary RPC calls every block |
| Batched concurrent backfill | Parallel `eth_getBlockByNumber` + `eth_getBlockReceipts` calls for throughput; batch size is configurable |
| `send_finished_height` is a no-op | No ExEx feedback mechanism for a standalone RPC follower |
| Alias oracle out of scope | Requires async trait redesign; orthogonal to the notifier |
| Async builder | Subscription establishment is inherently async; clean API |
| `BlockAndReceipts` struct in `Extractable` | Named fields over tuple; `&[R]` over `&Vec<R>` |
