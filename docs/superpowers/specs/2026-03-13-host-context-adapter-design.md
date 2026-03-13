# Host Context Adapter: Decoupling signet-node from reth ExEx

**Date:** 2026-03-13
**Status:** Approved

## Goal

Replace `signet-node`'s direct dependency on reth's `ExExContext<Host>` with a
trait abstraction, enabling alternative host backends (e.g. RPC-backed) without
reth in the dependency graph of `signet-node`.

## Scope

This design covers **`signet-node` only**. Other crates with reth dependencies
(`signet-blobber`, `signet-node-config`) are out of scope.

The abstraction is a single `HostNotifier` trait covering:
1. **Notifications stream** — committed, reverted, and reorged chain segments,
   with safe/finalized block numbers bundled into each notification
2. **Backfill** — head positioning and batch size thresholds
3. **Event feedback** — notifying the host of processed height

Out of scope for the trait: transaction pool access, RPC config inheritance,
host block lookups (eliminated — see design notes).

## Architecture

Three crates are modified or created:

```
signet-extract             signet-node-types          signet-host-reth
(Extractable extended)     (HostNotifier trait)        (reth ExEx impl)
       \                        |                        /
        \                       |                       /
         v                      v                      v
                         signet-node
                   (generic over HostNotifier)
```

### `signet-extract` (modified)

The `Extractable` trait gains provided methods for chain segment metadata.
These are used during revert handling and notification instrumentation.

```rust
pub trait Extractable: Debug + Sync {
    type Block: BlockHeader + HasTxns + Debug + Sync;
    type Receipt: TxReceipt<Log = Log> + Debug + Sync;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = (&Self::Block, &Vec<Self::Receipt>)>;

    /// Block number of the first block in the segment.
    ///
    /// # Panics
    ///
    /// Panics if the chain segment is empty.
    fn first_number(&self) -> u64 {
        self.blocks_and_receipts()
            .next()
            .expect("chain segment is empty")
            .0
            .number()
    }

    /// Block number of the tip (last block) in the segment.
    ///
    /// # Panics
    ///
    /// Panics if the chain segment is empty.
    fn tip_number(&self) -> u64 {
        self.blocks_and_receipts()
            .last()
            .expect("chain segment is empty")
            .0
            .number()
    }

    /// Number of blocks in the segment.
    fn len(&self) -> usize {
        self.blocks_and_receipts().count()
    }

    /// Whether the segment is empty.
    fn is_empty(&self) -> bool {
        self.blocks_and_receipts().next().is_none()
    }
}
```

Backends (e.g. reth's chain wrapper) can override these for O(1) performance.
The default implementations iterate `blocks_and_receipts()`.

### `signet-node-types` (new crate)

Minimal crate defining the host abstraction. No reth dependencies.

**Dependencies:** `alloy`, `signet-extract`, `std`/`core`.

**Contents:**

#### `HostNotifier` trait

```rust
pub trait HostNotifier {
    /// A chain segment — contiguous blocks with receipts.
    type Chain: Extractable;

    /// The error type for fallible operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Yield the next notification. `None` signals host shutdown.
    fn next_notification(
        &mut self,
    ) -> impl Future<Output = Option<Result<HostNotification<Self::Chain>, Self::Error>>> + Send;

    /// Set the head position, requesting backfill from this block number.
    /// The backend resolves the block number to a block hash internally.
    fn set_head(&mut self, block_number: u64);

    /// Configure backfill batch size limits.
    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>);

    /// Signal that processing is complete up to this host block number.
    /// The backend resolves the block number to a block hash internally.
    fn send_finished_height(&self, block_number: u64) -> Result<(), Self::Error>;
}
```

**Design notes — elimination of host block lookups:**

The previous design had a `HostReader` trait for point-in-time lookups
(`sealed_header`, `safe_block_number`, `finalized_block_number`). These
lookups served two purposes:

1. **Per-notification:** Reading safe/finalized block numbers for tag updates,
   and resolving a header hash for `FinishedHeight` feedback.
2. **Startup:** Resolving a block number to a `NumHash` for `set_head`.

Both are eliminated:

- **Per-notification lookups** are replaced by bundling `safe_block_number`
  and `finalized_block_number` into `HostNotification`. The header hash for
  `FinishedHeight` feedback is resolved by the backend — `send_finished_height`
  takes a `u64` block number, and the backend looks up the hash internally.
- **Startup lookups** are eliminated by having `set_head` take a `u64` block
  number instead of a `NumHash`. The backend resolves the hash internally.

This means `signet-node` never performs host chain lookups. All block data
comes from notifications, and all hash resolution is the backend's
responsibility. This structurally prevents inconsistent reads during block
processing — there are no lookup methods to misuse.

#### `HostNotification` types

```rust
pub struct HostNotification<C> {
    /// The chain event (commit, revert, or reorg).
    pub kind: HostNotificationKind<C>,

    /// The host chain "safe" block number at the time of this notification.
    pub safe_block_number: Option<u64>,

    /// The host chain "finalized" block number at the time of this notification.
    pub finalized_block_number: Option<u64>,
}

pub enum HostNotificationKind<C> {
    ChainCommitted { new: Arc<C> },
    ChainReverted { old: Arc<C> },
    ChainReorged { old: Arc<C>, new: Arc<C> },
}
```

Accessor semantics on `HostNotificationKind`:
- `committed_chain()` returns `Some(&new)` for `ChainCommitted` and
  `ChainReorged`, `None` for `ChainReverted`.
- `reverted_chain()` returns `Some(&old)` for `ChainReverted` and
  `ChainReorged`, `None` for `ChainCommitted`.

This matches the existing `ExExNotification` behavior and ensures the
"reverts run first" pattern in `on_notification` works correctly.

### `signet-host-reth` (new crate)

Implements `HostNotifier` for reth's ExEx. Owns all reth dependencies.

**Dependencies:** `signet-node-types`, `signet-blobber` (for chain shim),
`reth`, `reth-exex`, `reth-node-api`, `reth-stages-types`, `alloy`.

**Contents:**

#### `RethHostNotifier`

```rust
pub struct RethHostNotifier<Host: FullNodeComponents> {
    notifications: ExExNotificationsStream<Host>,
    provider: Host::Provider,
    events: UnboundedSender<ExExEvent>,
}
```

- `HostNotifier::Chain` → an owning wrapper around `reth::providers::Chain`
  that implements `Extractable` (by providing a borrowed
  `ExtractableChainShim` internally), overriding `first_number`,
  `tip_number`, `len`, and `is_empty` for O(1) performance
- `HostNotifier::Error` → reth error type or `eyre::Report`
- `next_notification` wraps the reth notification stream, reads
  `safe_block_number` and `finalized_block_number` from the provider, and
  bundles them into `HostNotification`
- `set_head` resolves the block number to a `NumHash` via the provider's
  `BlockReader`, then calls `set_with_head` on the notifications stream
- `send_finished_height` resolves the block number to a `NumHash` via the
  provider's `HeaderProvider`, then sends `ExExEvent::FinishedHeight`

The provider is held by the notifier (not exposed to `signet-node`) so that
hash resolution stays internal to the backend.

#### Moved from `signet-node`

- `RethAliasOracleFactory` / `RethAliasOracle` — uses `StateProviderFactory`
  to query host chain state for alias decisions
- The owning chain wrapper with `Extractable` impl (O(1) overrides)

#### Convenience constructor

Takes an `ExExContext<Host>` and returns:
1. A `RethHostNotifier` (the adapter)
2. A `ServeConfig` (plain RPC config extracted from `ExExContext::config.rpc`)
3. A `StorageRpcConfig` (gas oracle settings extracted from reth config)
4. The transaction pool handle (`Host::Pool`) for blob cacher construction
5. A chain ID / chain name for tracing

This cleanly splits the `ExExContext` into the parts that flow through the
trait and the parts that are consumed at construction time.

### `signet-node` (modified)

Drops all reth dependencies. Replaces them with `signet-node-types`.

#### `SignetNode`

```rust
pub struct SignetNode<N, H, AliasOracle>
where
    N: HostNotifier,
    H: HotKv,
{
    notifier: N,
    config: Arc<SignetNodeConfig>,
    storage: Arc<UnifiedStorage<H>>,
    chain_name: String,
    // ... rest unchanged
}
```

The notification loop becomes:

```rust
async fn start_inner(&mut self) -> eyre::Result<()> {
    // Startup: tell the backend where we are
    self.notifier.set_head(last_rollup_block_host_height);

    // Notification loop
    while let Some(notification) = self.notifier.next_notification().await {
        let notification = notification?;
        let changed = self.on_notification(&notification).await?;

        if changed {
            // safe/finalized come from the notification itself
            self.update_block_tags(
                notification.safe_block_number,
                notification.finalized_block_number,
            )?;
        }
    }
}
```

No host block lookups anywhere in `signet-node`. All block data comes from
the notification's chain, and all hash resolution is the backend's job.

**Handling `ctx.pool()` (blob cacher):** The `BlobFetcher` construction that
currently calls `ctx.pool().clone()` moves out of `SignetNode::new_unsafe`. The
builder accepts a pre-built `CacheHandle` instead. The caller (assembly crate)
constructs the `BlobFetcher` using the pool returned from the
`signet-host-reth` convenience constructor, then passes the `CacheHandle` to
the builder.

**Handling chain ID for tracing:** The `#[instrument]` attribute on `start()`
currently reads `self.host.config.chain.chain()`. This is replaced by a
`chain_name: String` field on `SignetNode`, set during construction from the
chain ID returned by the `signet-host-reth` convenience constructor.

**Handling `set_exex_head`:** The current method resolves block numbers to
hashes via provider lookups, constructs an `ExExHead`, and logs it. After the
change, `set_head(u64)` delegates hash resolution to the backend. Logging
uses the block number directly. The `ExExHead` type is no longer needed in
`signet-node`.

**`num_hash_slow()` elimination:** All `num_hash_slow()` calls disappear.
Hash resolution is fully internal to the backend.

Method translations:
- `self.host.notifications.next().await` →
  `self.notifier.next_notification().await`
- `self.host.provider().block_by_number(n)` → eliminated (backend resolves)
- `self.host.provider().sealed_header(n)` → eliminated (backend resolves)
- `self.host.provider().safe_block_number()` →
  `notification.safe_block_number` (bundled)
- `self.host.provider().finalized_block_number()` →
  `notification.finalized_block_number` (bundled)
- `self.host.notifications.set_with_head(head)` →
  `self.notifier.set_head(block_number)`
- `self.host.notifications.set_backfill_thresholds(t)` →
  `self.notifier.set_backfill_thresholds(max_blocks)`
- `self.host.events.send(ExExEvent::FinishedHeight(h))` →
  `self.notifier.send_finished_height(block_number)`

#### `SignetNodeBuilder`

```rust
pub fn with_notifier<N: HostNotifier>(self, n: N) -> SignetNodeBuilder<N, Storage, Aof>
```

New builder methods:
- `with_notifier(N)` — accepts the notification adapter
- `with_blob_cacher(CacheHandle)` — accepts pre-built blob cacher
- `with_serve_config(ServeConfig)` — accepts plain RPC config
- `with_chain_name(String)` — accepts chain name for tracing

The `build()` variant that creates a default `RethAliasOracleFactory` from the
provider moves to `signet-host-reth` (extension trait or convenience function).

#### Metrics

`record_notification_received` and `record_notification_processed` change from
`&ExExNotification<N>` to `&HostNotification<C>`. Same logic, different types.

#### RPC config

The `merge_rpc_configs` method is removed from `signet-node-config`. The
`signet-host-reth` convenience constructor extracts `RpcServerArgs` from the
`ExExContext` and converts them to a `ServeConfig` and `StorageRpcConfig`,
which are passed to the builder. The `rpc_config_from_args` and
`serve_config_from_args` helpers (currently in `signet-node/src/rpc.rs`) move
to `signet-host-reth` since they operate on reth's `RpcServerArgs` type.
The `signet-node` RPC module receives pre-built config values only.

## What does NOT change

- `signet-blobber` retains its reth dependencies (transaction pool, `Chain`)
- `signet-node-tests` continues to use `reth-exex-test-utils` — tests construct
  a `RethHostNotifier` from the test ExEx context
- All other workspace crates are unaffected
- The binary assembly crate pulls in both `signet-node` and `signet-host-reth`

## Testing strategy

- **`signet-node` unit tests:** A mock `HostNotifier` backed by channels/vecs
  for notifications. No lookup mocks needed — there are no lookup methods.
  This replaces the need for `reth-exex-test-utils` in unit tests.
- **`signet-node-tests` integration tests:** Continue using
  `reth-exex-test-utils` via `RethHostNotifier` from `signet-host-reth`.

## Implementation order

1. Extend `Extractable` in `signet-extract` with `first_number`, `tip_number`,
   `len`, `is_empty` as provided methods
2. Create `signet-node-types` with `HostNotifier`, `HostNotification`,
   `HostNotificationKind`
3. Create `signet-host-reth` with `RethHostNotifier` and moved reth-specific
   code (alias oracle, chain wrapper with O(1) overrides)
4. Refactor `signet-node` to depend on `signet-node-types` instead of reth,
   update `SignetNode`/`SignetNodeBuilder`, move blob cacher construction to
   caller
5. Update `signet-node-tests` to construct `RethHostNotifier`
6. Remove reth deps from `signet-node/Cargo.toml`

## Design decisions

| Decision | Rationale |
|----------|-----------|
| Single `HostNotifier` trait | All host interaction flows through one interface; no separate reader needed |
| Lookups bundled into notification | Safe/finalized numbers travel with the notification; hash resolution is the backend's job. Structurally prevents inconsistent reads during processing |
| `set_head` and `send_finished_height` take `u64` | Backend resolves block hashes internally; signet-node never queries the host chain |
| Associated types for `Chain` | Flexibility for backends with different chain representations |
| Chain metadata merged into `Extractable` | `first_number`, `tip_number`, `len` are derivable from `blocks_and_receipts`; eliminates a separate `HostChain` trait. Panics on empty match current reth behavior |
| Reth impl in dedicated crate | Keeps all reth deps isolated; signet-node is reth-free |
| RPC config extracted at call site | Simpler than threading config through the trait; eliminates reth dep from signet-node-config |
| `CacheHandle` passed into builder | Moves pool dependency out of signet-node; caller constructs blob cacher |
| Chain name as construction param | Avoids threading host config type through the trait for a single tracing field |
