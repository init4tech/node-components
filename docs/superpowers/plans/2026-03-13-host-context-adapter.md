# Host Context Adapter Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decouple signet-node from reth's ExExContext by introducing a HostNotifier trait, isolating all reth-specific code in a dedicated signet-host-reth crate.

**Architecture:** A single `HostNotifier` trait (in `signet-node-types`) abstracts over the host chain. Notifications bundle safe/finalized block numbers. Hash resolution is the backend's responsibility. The reth implementation lives in `signet-host-reth`. `signet-node` becomes reth-free.

**Tech Stack:** Rust, alloy, reth (isolated in signet-host-reth only), tokio, tracing

**Spec:** `docs/superpowers/specs/2026-03-13-host-context-adapter-design.md`

---

## Chunk 1: Extend Extractable and Create signet-node-types

### Task 1: Extend `Extractable` in the SDK

The `Extractable` trait lives in the external SDK repo at `../sdk/crates/extract/src/trait.rs`. We add provided methods for chain segment metadata (`first_number`, `tip_number`, `len`, `is_empty`) and patch the components workspace to use the local copy.

**Files:**
- Modify: `../sdk/crates/extract/src/trait.rs`
- Modify: `Cargo.toml` (workspace root — uncomment path patches)

- [ ] **Step 1: Add provided methods to `Extractable`**

In `../sdk/crates/extract/src/trait.rs`, add after the `blocks_and_receipts` method:

```rust
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
```

Note: `number()` comes from the `BlockHeader` bound on `Self::Block`.

- [ ] **Step 2: Add `BlockHeader` import**

The trait file needs `use alloy::consensus::BlockHeader;` for the `.number()` call in default impls. Add this import at the top of `../sdk/crates/extract/src/trait.rs`.

- [ ] **Step 3: Verify SDK builds**

Run: `cargo clippy -p signet-extract --all-features --all-targets` (from `../sdk/`)
Expected: Clean pass. The new methods are provided with defaults, so no existing impls break.

- [ ] **Step 4: Uncomment path patches in components workspace**

In `/Users/james/devel/init4/components/Cargo.toml`, uncomment the SDK path overrides (lines 122–130):

```toml
signet-bundle = { path = "../sdk/crates/bundle"}
signet-constants = { path = "../sdk/crates/constants"}
signet-evm = { path = "../sdk/crates/evm"}
signet-extract = { path = "../sdk/crates/extract"}
signet-journal = { path = "../sdk/crates/journal"}
signet-test-utils = { path = "../sdk/crates/test-utils"}
signet-tx-cache = { path = "../sdk/crates/tx-cache"}
signet-types = { path = "../sdk/crates/types"}
signet-zenith = { path = "../sdk/crates/zenith"}
```

- [ ] **Step 5: Verify components workspace builds with patched SDK**

Run: `cargo clippy -p signet-node --all-features --all-targets`
Expected: Clean pass. Nothing in components uses the new methods yet.

- [ ] **Step 6: Commit SDK change**

```bash
cd ../sdk
git add crates/extract/src/trait.rs
git commit -m "feat: add chain segment metadata methods to Extractable"
```

- [ ] **Step 7: Commit components workspace patch**

```bash
cd /Users/james/devel/init4/components
git add Cargo.toml
git commit -m "chore: enable local SDK path overrides for Extractable changes"
```

---

### Task 2: Create `signet-node-types` crate

New crate with the `HostNotifier` trait, `HostNotification`, and `HostNotificationKind`. No reth dependencies.

**Files:**
- Create: `crates/node-types/Cargo.toml`
- Create: `crates/node-types/src/lib.rs`
- Create: `crates/node-types/src/notifier.rs`
- Create: `crates/node-types/src/notification.rs`
- Create: `crates/node-types/README.md`
- Modify: `Cargo.toml` (workspace root — add to dependencies)

- [ ] **Step 1: Create crate directory and README**

```bash
mkdir -p crates/node-types/src
```

Write `crates/node-types/README.md`:
```markdown
# signet-node-types

Trait abstractions for the signet node's host chain interface.
```

- [ ] **Step 2: Write `Cargo.toml`**

Create `crates/node-types/Cargo.toml`:

```toml
[package]
name = "signet-node-types"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
homepage.workspace = true
repository.workspace = true

[dependencies]
signet-extract.workspace = true

alloy.workspace = true
```

- [ ] **Step 3: Add workspace dependency**

In the root `Cargo.toml`, add to `[workspace.dependencies]` near the other local crates:

```toml
signet-node-types = { version = "0.16.0-rc.7", path = "crates/node-types" }
```

- [ ] **Step 4: Write `HostNotificationKind` and `HostNotification`**

Create `crates/node-types/src/notification.rs`:

```rust
use signet_extract::Extractable;
use std::sync::Arc;

/// A notification from the host chain, bundling a chain event with
/// point-in-time block tag data.
#[derive(Debug, Clone)]
pub struct HostNotification<C> {
    /// The chain event (commit, revert, or reorg).
    pub kind: HostNotificationKind<C>,
    /// The host chain "safe" block number at the time of this notification.
    pub safe_block_number: Option<u64>,
    /// The host chain "finalized" block number at the time of this
    /// notification.
    pub finalized_block_number: Option<u64>,
}

/// The kind of chain event in a [`HostNotification`].
#[derive(Debug, Clone)]
pub enum HostNotificationKind<C> {
    /// A new chain segment was committed.
    ChainCommitted {
        /// The newly committed chain segment.
        new: Arc<C>,
    },
    /// A chain segment was reverted.
    ChainReverted {
        /// The reverted chain segment.
        old: Arc<C>,
    },
    /// A chain reorg occurred: one segment was reverted and replaced by
    /// another.
    ChainReorged {
        /// The reverted chain segment.
        old: Arc<C>,
        /// The newly committed chain segment.
        new: Arc<C>,
    },
}

impl<C: Extractable> HostNotificationKind<C> {
    /// Returns the committed chain, if any.
    ///
    /// Returns `Some` for [`ChainCommitted`] and [`ChainReorged`], `None`
    /// for [`ChainReverted`].
    ///
    /// [`ChainCommitted`]: HostNotificationKind::ChainCommitted
    /// [`ChainReorged`]: HostNotificationKind::ChainReorged
    /// [`ChainReverted`]: HostNotificationKind::ChainReverted
    pub fn committed_chain(&self) -> Option<&Arc<C>> {
        match self {
            Self::ChainCommitted { new } | Self::ChainReorged { new, .. } => Some(new),
            Self::ChainReverted { .. } => None,
        }
    }

    /// Returns the reverted chain, if any.
    ///
    /// Returns `Some` for [`ChainReverted`] and [`ChainReorged`], `None`
    /// for [`ChainCommitted`].
    ///
    /// [`ChainReverted`]: HostNotificationKind::ChainReverted
    /// [`ChainReorged`]: HostNotificationKind::ChainReorged
    /// [`ChainCommitted`]: HostNotificationKind::ChainCommitted
    pub fn reverted_chain(&self) -> Option<&Arc<C>> {
        match self {
            Self::ChainReverted { old } | Self::ChainReorged { old, .. } => Some(old),
            Self::ChainCommitted { .. } => None,
        }
    }
}
```

- [ ] **Step 5: Write `HostNotifier` trait**

Create `crates/node-types/src/notifier.rs`:

```rust
use crate::HostNotification;
use core::future::Future;
use signet_extract::Extractable;

/// Abstraction over a host chain notification source.
///
/// Drives the signet node's main loop: yielding chain events, controlling
/// backfill, and sending feedback. All block data comes from notifications;
/// the backend handles hash resolution internally.
///
/// # Implementors
///
/// - `signet-host-reth`: wraps reth's `ExExContext`
pub trait HostNotifier {
    /// A chain segment — contiguous blocks with receipts.
    type Chain: Extractable;

    /// The error type for fallible operations.
    type Error: core::error::Error + Send + Sync + 'static;

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

- [ ] **Step 6: Write `lib.rs`**

Create `crates/node-types/src/lib.rs`:

```rust
#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod notification;
pub use notification::{HostNotification, HostNotificationKind};

mod notifier;
pub use notifier::HostNotifier;
```

- [ ] **Step 7: Lint the new crate**

Run: `cargo clippy -p signet-node-types --all-features --all-targets`
Expected: Clean pass.

Run: `cargo +nightly fmt`
Expected: Clean.

- [ ] **Step 8: Commit**

```bash
git add crates/node-types/ Cargo.toml
git commit -m "feat: add signet-node-types crate with HostNotifier trait"
```

---

## Chunk 2: Create signet-host-reth

### Task 3: Create `signet-host-reth` crate scaffold

New crate that wraps reth's ExEx types behind the `HostNotifier` trait.

**Files:**
- Create: `crates/host-reth/Cargo.toml`
- Create: `crates/host-reth/src/lib.rs`
- Create: `crates/host-reth/src/chain.rs`
- Create: `crates/host-reth/src/notifier.rs`
- Create: `crates/host-reth/src/alias.rs`
- Create: `crates/host-reth/src/config.rs`
- Create: `crates/host-reth/README.md`
- Modify: `Cargo.toml` (workspace root — add dependency)

- [ ] **Step 1: Create crate directory and README**

```bash
mkdir -p crates/host-reth/src
```

Write `crates/host-reth/README.md`:
```markdown
# signet-host-reth

Reth ExEx implementation of the `HostNotifier` trait for signet-node.
```

- [ ] **Step 2: Write `Cargo.toml`**

Create `crates/host-reth/Cargo.toml`:

```toml
[package]
name = "signet-host-reth"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
homepage.workspace = true
repository.workspace = true

[dependencies]
signet-node-types.workspace = true
signet-blobber.workspace = true
signet-extract.workspace = true
signet-rpc.workspace = true
signet-block-processor.workspace = true

alloy.workspace = true
reth.workspace = true
reth-exex.workspace = true
reth-node-api.workspace = true
reth-stages-types.workspace = true

eyre.workspace = true
futures-util.workspace = true
tracing.workspace = true
```

- [ ] **Step 3: Add workspace dependency**

In root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
signet-host-reth = { version = "0.16.0-rc.7", path = "crates/host-reth" }
```

- [ ] **Step 4: Commit scaffold**

```bash
git add crates/host-reth/Cargo.toml crates/host-reth/README.md Cargo.toml
git commit -m "chore: scaffold signet-host-reth crate"
```

---

### Task 4: Implement the reth chain wrapper

An owning wrapper around `reth::providers::Chain` that implements `Extractable` with O(1) overrides for `first_number`, `tip_number`, `len`.

**Files:**
- Create: `crates/host-reth/src/chain.rs`

**Reference:** `crates/blobber/src/shim.rs` for the existing `ExtractableChainShim` pattern.

- [ ] **Step 1: Write the chain wrapper**

Create `crates/host-reth/src/chain.rs`:

```rust
use alloy::consensus::BlockHeader;
use reth::primitives::EthPrimitives;
use reth::providers::Chain;
use signet_blobber::{ExtractableChainShim, RecoveredBlockShim};
use signet_extract::Extractable;
use std::sync::Arc;

/// An owning wrapper around reth's [`Chain`] that implements [`Extractable`]
/// with O(1) metadata accessors.
#[derive(Debug)]
pub struct RethChain {
    inner: Arc<Chain<EthPrimitives>>,
}

impl RethChain {
    /// Wrap a reth chain.
    pub fn new(chain: Arc<Chain<EthPrimitives>>) -> Self {
        Self { inner: chain }
    }
}

impl Extractable for RethChain {
    type Block = RecoveredBlockShim;
    type Receipt = reth::primitives::Receipt;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = (&Self::Block, &Vec<Self::Receipt>)> {
        ExtractableChainShim::new(&self.inner).blocks_and_receipts()
    }

    fn first_number(&self) -> u64 {
        self.inner.first().number()
    }

    fn tip_number(&self) -> u64 {
        self.inner.tip().number()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
```

Note: The `blocks_and_receipts` impl delegates to `ExtractableChainShim` which already handles the `repr(transparent)` transmute. The lifetime on the returned iterator borrows from `self.inner` via the shim. If the borrow checker complains because `ExtractableChainShim` holds a temporary, the implementation may need to inline the shim logic. Verify during implementation.

- [ ] **Step 2: Verify it compiles**

Add `mod chain;` to a temporary `lib.rs` and run:
`cargo clippy -p signet-host-reth --all-features --all-targets`

- [ ] **Step 3: Commit**

```bash
git add crates/host-reth/src/chain.rs
git commit -m "feat(host-reth): add RethChain wrapper with O(1) Extractable overrides"
```

---

### Task 5: Move alias oracle to signet-host-reth

Move `RethAliasOracle` and `RethAliasOracleFactory` from `signet-node` to `signet-host-reth`.

**Files:**
- Create: `crates/host-reth/src/alias.rs`
- Reference: `crates/node/src/alias.rs` (copy and adapt)

- [ ] **Step 1: Copy alias.rs to host-reth**

Copy `/Users/james/devel/init4/components/crates/node/src/alias.rs` to `crates/host-reth/src/alias.rs`. The contents are identical — no changes needed to the code itself.

- [ ] **Step 2: Verify it compiles**

Add `mod alias; pub use alias::{RethAliasOracle, RethAliasOracleFactory};` to lib.rs.
Run: `cargo clippy -p signet-host-reth --all-features --all-targets`

- [ ] **Step 3: Commit**

```bash
git add crates/host-reth/src/alias.rs
git commit -m "feat(host-reth): move RethAliasOracle from signet-node"
```

---

### Task 6: Move RPC config helpers to signet-host-reth

Move `rpc_config_from_args` and `serve_config_from_args` from `signet-node/src/rpc.rs`.

**Files:**
- Create: `crates/host-reth/src/config.rs`
- Reference: `crates/node/src/rpc.rs:49-72`

- [ ] **Step 1: Write config.rs**

Create `crates/host-reth/src/config.rs` with the two functions from `crates/node/src/rpc.rs`:

```rust
use reth::args::RpcServerArgs;
use signet_rpc::{ServeConfig, StorageRpcConfig};
use std::net::SocketAddr;

/// Extract [`StorageRpcConfig`] values from reth's host RPC settings.
pub fn rpc_config_from_args(args: &RpcServerArgs) -> StorageRpcConfig {
    let gpo = &args.gas_price_oracle;
    StorageRpcConfig::builder()
        .rpc_gas_cap(args.rpc_gas_cap)
        .max_tracing_requests(args.rpc_max_tracing_requests)
        .gas_oracle_block_count(gpo.blocks as u64)
        .gas_oracle_percentile(gpo.percentile as f64)
        .ignore_price(Some(gpo.ignore_price as u128))
        .max_price(Some(gpo.max_price as u128))
        .build()
}

/// Convert reth [`RpcServerArgs`] into a reth-free [`ServeConfig`].
pub fn serve_config_from_args(args: &RpcServerArgs) -> ServeConfig {
    let http = if args.http {
        vec![SocketAddr::from((args.http_addr, args.http_port))]
    } else {
        vec![]
    };
    let ws = if args.ws {
        vec![SocketAddr::from((args.ws_addr, args.ws_port))]
    } else {
        vec![]
    };
    let ipc = if !args.ipcdisable {
        Some(args.ipcpath.clone())
    } else {
        None
    };
    ServeConfig {
        http,
        http_cors: args.http_corsdomain.clone(),
        ws,
        ws_cors: args.ws_allowed_origins.clone(),
        ipc,
    }
}
```

- [ ] **Step 2: Verify it compiles**

Add `mod config; pub use config::{rpc_config_from_args, serve_config_from_args};` to lib.rs.
Run: `cargo clippy -p signet-host-reth --all-features --all-targets`

- [ ] **Step 3: Commit**

```bash
git add crates/host-reth/src/config.rs
git commit -m "feat(host-reth): move RPC config helpers from signet-node"
```

---

### Task 7: Implement `RethHostNotifier` and convenience constructor

The core adapter: wraps `ExExNotificationsStream`, provider, and events sender behind `HostNotifier`.

**Files:**
- Create: `crates/host-reth/src/notifier.rs`
- Modify: `crates/host-reth/src/lib.rs` (final version)

- [ ] **Step 1: Write `RethHostNotifier`**

Create `crates/host-reth/src/notifier.rs`:

```rust
use crate::{RethChain, config::{rpc_config_from_args, serve_config_from_args}};
use alloy::consensus::BlockHeader;
use alloy::eips::NumHash;
use futures_util::StreamExt;
use reth::{
    chainspec::EthChainSpec,
    primitives::EthPrimitives,
    providers::{BlockIdReader, BlockReader, HeaderProvider},
};
use reth_exex::{ExExContext, ExExEvent, ExExNotificationsStream};
use reth_node_api::{FullNodeComponents, FullNodeTypes, NodeTypes};
use reth_stages_types::ExecutionStageThresholds;
use signet_node_types::{HostNotification, HostNotificationKind, HostNotifier};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use std::sync::Arc;
use tracing::debug;

/// Reth ExEx implementation of [`HostNotifier`].
///
/// Wraps reth's notification stream, provider, and event sender. All hash
/// resolution happens internally — consumers only work with block numbers.
pub struct RethHostNotifier<Host: FullNodeComponents> {
    notifications: ExExNotificationsStream<Host>,
    provider: Host::Provider,
    events: tokio::sync::mpsc::UnboundedSender<ExExEvent>,
}

impl<Host: FullNodeComponents> core::fmt::Debug for RethHostNotifier<Host> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RethHostNotifier").finish_non_exhaustive()
    }
}

/// The output of [`decompose_exex_context`].
pub struct DecomposedContext<Host: FullNodeComponents> {
    /// The host notifier adapter.
    pub notifier: RethHostNotifier<Host>,
    /// Plain RPC serve config.
    pub serve_config: ServeConfig,
    /// Plain RPC storage config.
    pub rpc_config: StorageRpcConfig,
    /// The transaction pool, for blob cacher construction.
    pub pool: Host::Pool,
    /// The chain name, for tracing.
    pub chain_name: String,
}

impl<Host: FullNodeComponents> core::fmt::Debug for DecomposedContext<Host> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecomposedContext")
            .field("chain_name", &self.chain_name)
            .finish_non_exhaustive()
    }
}

/// Decompose a reth [`ExExContext`] into a [`RethHostNotifier`] and
/// associated configuration values.
///
/// This splits the ExEx context into:
/// - A [`RethHostNotifier`] (implements [`HostNotifier`])
/// - A [`ServeConfig`] (plain RPC server config)
/// - A [`StorageRpcConfig`] (gas oracle settings)
/// - The transaction pool handle
/// - A chain name for tracing
pub fn decompose_exex_context<Host>(ctx: ExExContext<Host>) -> DecomposedContext<Host>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
{
    let chain_name = ctx.config.chain.chain().to_string();
    let serve_config = serve_config_from_args(&ctx.config.rpc);
    let rpc_config = rpc_config_from_args(&ctx.config.rpc);
    let pool = ctx.pool().clone();
    let provider = ctx.provider().clone();

    let notifier = RethHostNotifier {
        notifications: ctx.notifications,
        provider,
        events: ctx.events,
    };

    DecomposedContext { notifier, serve_config, rpc_config, pool, chain_name }
}

impl<Host> HostNotifier for RethHostNotifier<Host>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
{
    type Chain = RethChain;
    type Error = eyre::Report;

    async fn next_notification(
        &mut self,
    ) -> Option<Result<HostNotification<Self::Chain>, Self::Error>> {
        let notification = self.notifications.next().await?;
        let notification = match notification {
            Ok(n) => n,
            Err(e) => return Some(Err(e.into())),
        };

        // Read safe/finalized from the provider at notification time.
        let safe_block_number = self
            .provider
            .safe_block_number()
            .ok()
            .flatten();
        let finalized_block_number = self
            .provider
            .finalized_block_number()
            .ok()
            .flatten();

        let kind = match notification {
            reth_exex::ExExNotification::ChainCommitted { new } => {
                HostNotificationKind::ChainCommitted {
                    new: Arc::new(RethChain::new(new)),
                }
            }
            reth_exex::ExExNotification::ChainReverted { old } => {
                HostNotificationKind::ChainReverted {
                    old: Arc::new(RethChain::new(old)),
                }
            }
            reth_exex::ExExNotification::ChainReorged { old, new } => {
                HostNotificationKind::ChainReorged {
                    old: Arc::new(RethChain::new(old)),
                    new: Arc::new(RethChain::new(new)),
                }
            }
        };

        Some(Ok(HostNotification { kind, safe_block_number, finalized_block_number }))
    }

    fn set_head(&mut self, block_number: u64) {
        let block = self
            .provider
            .block_by_number(block_number)
            .expect("failed to look up block for set_head");

        let head = match block {
            Some(b) => b.num_hash_slow(),
            None => {
                debug!(block_number, "block not found for set_head, falling back to genesis");
                let genesis = self
                    .provider
                    .block_by_number(0)
                    .expect("failed to look up genesis block")
                    .expect("genesis block missing");
                genesis.num_hash_slow()
            }
        };

        let exex_head = reth_exex::ExExHead { block: head };
        self.notifications.set_with_head(exex_head);
    }

    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>) {
        if let Some(max_blocks) = max_blocks {
            self.notifications
                .set_backfill_thresholds(ExecutionStageThresholds {
                    max_blocks: Some(max_blocks),
                    ..Default::default()
                });
            debug!(max_blocks, "configured backfill thresholds");
        }
    }

    fn send_finished_height(&self, block_number: u64) -> Result<(), Self::Error> {
        let header = self
            .provider
            .sealed_header(block_number)?
            .ok_or_else(|| {
                eyre::eyre!(
                    "no host header for finished height {block_number}"
                )
            })?;

        let hash = header.hash();
        self.events
            .send(ExExEvent::FinishedHeight(NumHash {
                number: block_number,
                hash,
            }))?;
        Ok(())
    }
}
```

- [ ] **Step 2: Write final `lib.rs`**

Create `crates/host-reth/src/lib.rs`:

```rust
#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod alias;
pub use alias::{RethAliasOracle, RethAliasOracleFactory};

mod chain;
pub use chain::RethChain;

mod config;
pub use config::{rpc_config_from_args, serve_config_from_args};

mod notifier;
pub use notifier::{DecomposedContext, RethHostNotifier, decompose_exex_context};
```

- [ ] **Step 3: Lint**

Run: `cargo clippy -p signet-host-reth --all-features --all-targets`
Expected: Clean pass.

Run: `cargo +nightly fmt`

- [ ] **Step 4: Commit**

```bash
git add crates/host-reth/src/
git commit -m "feat(host-reth): implement RethHostNotifier and decompose_exex_context"
```

---

## Chunk 3: Refactor signet-node

### Task 8: Add signet-node-types dependency to signet-node

Add the new dependency alongside reth (temporarily). Reth deps are removed in Task 11 after all code is updated.

**Files:**
- Modify: `crates/node/Cargo.toml`

- [ ] **Step 1: Add signet-node-types to Cargo.toml**

In `crates/node/Cargo.toml`, add:
```toml
signet-node-types.workspace = true
```

Keep the reth deps for now — they'll be removed after the code changes.

- [ ] **Step 2: Verify it compiles**

Run: `cargo clippy -p signet-node --all-features --all-targets`
Expected: Clean pass (nothing changed yet).

- [ ] **Step 3: Commit**

```bash
git add crates/node/Cargo.toml
git commit -m "chore(node): add signet-node-types dependency"
```

---

### Task 9: Refactor `SignetNode` struct and notification loop

Replace `ExExContext<Host>` with `N: HostNotifier`. Update `start`, `start_inner`, `on_notification`, and related methods.

**Files:**
- Modify: `crates/node/src/node.rs`
- Modify: `crates/node/src/lib.rs`

- [ ] **Step 1: Rewrite `node.rs`**

Replace the entire file. Key changes:
- `SignetNode<Host, H, AliasOracle>` → `SignetNode<N, H, AliasOracle>` where `N: HostNotifier`
- Remove `Host: FullNodeComponents` bounds
- `host: ExExContext<Host>` → `notifier: N`
- Add `chain_name: String` field
- Remove `type PrimitivesOf`, `type ExExNotification`, `type Chain` aliases
- `new_unsafe` takes `notifier: N` instead of `ctx: ExExContext<Host>`, plus `chain_name: String` and `blob_cacher: CacheHandle`
- `start` instrument field: `host = ?self.host.config.chain.chain()` → `host = %self.chain_name`
- `start` error handler (`.inspect_err`): replace `self.set_exex_head(h)` with `self.notifier.set_head(host_height)` — the error recovery path just logs the block number now
- `start_inner`: replace `set_exex_head` with `self.notifier.set_head(host_height)` and `self.notifier.set_backfill_thresholds(self.config.backfill_max_blocks())`
- Notification loop: `self.host.notifications.next().await` → `self.notifier.next_notification().await`
- `on_notification` takes `&HostNotification<N::Chain>` by reference (currently takes `ExExNotification<Host>` by value). The `Arc<C>` inside notifications makes borrowing cheap.
- `notification.reverted_chain()` → `notification.kind.reverted_chain()`
- `notification.committed_chain()` → `notification.kind.committed_chain()`
- `process_committed_chain` takes `&Arc<N::Chain>` where `N::Chain: Extractable`. Remove the `ExtractableChainShim::new(chain)` call — the chain already implements `Extractable` directly. Use the chain as-is with `Extractor::extract_signet`.
- `on_host_revert` takes `&Arc<N::Chain>`, uses `chain.first_number()` and `chain.tip_number()`
- **Decompose `update_status`:** Split into two parts:
  - `update_status_channel(ru_height)` — just updates `self.status.send_modify(...)`. Called from `on_notification` as before.
  - `update_block_tags(safe_block_number, finalized_block_number)` — takes the bundled values from the notification. Called from `start_inner` AFTER `on_notification` returns.
  This ensures block tags are updated exactly once per notification, using the values bundled in the notification.
- `on_notification` returns `(bool, u64)` — whether anything changed AND the current rollup height (for the status channel update). Or: `on_notification` still calls `self.update_status_channel()` internally for the status/height update, and returns `bool` for whether `update_block_tags` should run.
- `load_safe_block_heights` takes `safe_block_number: Option<u64>` param
- `load_finalized_block_heights` takes `finalized_block_number: Option<u64>` param
- `update_highest_processed_height` calls `self.notifier.send_finished_height(adjusted_height)`
- Remove `set_exex_head` entirely
- Remove `set_backfill_thresholds` method (logic moves to `start_inner` directly)
- Remove `signet_blobber::ExtractableChainShim` from imports (no longer needed)

The `start_inner` notification loop becomes:
```rust
while let Some(notification) = self.notifier.next_notification().await {
    let notification =
        notification.wrap_err("error in host notifications stream")?;
    let changed = self.on_notification(&notification).await?;
    if changed {
        // safe/finalized come from the notification, not from lookups
        self.update_block_tags(
            notification.safe_block_number,
            notification.finalized_block_number,
        )?;
    }
}
```

- [ ] **Step 2: Update `lib.rs`**

Remove the `RethAliasOracle` and `RethAliasOracleFactory` re-exports (they moved to `signet-host-reth`):

```rust
// Remove:
mod alias;
pub use alias::{RethAliasOracle, RethAliasOracleFactory};
```

- [ ] **Step 3: Attempt compilation**

Run: `cargo clippy -p signet-node --all-features --all-targets 2>&1 | head -40`
Expected: Errors in builder.rs, rpc.rs, metrics.rs — addressed in next tasks.

- [ ] **Step 4: Commit**

```bash
git add crates/node/src/node.rs crates/node/src/lib.rs
git commit -m "refactor(node): replace ExExContext with HostNotifier trait"
```

---

### Task 10: Refactor `SignetNodeBuilder`

Update the builder to accept `N: HostNotifier` instead of `ExExContext<Host>`.

**Files:**
- Modify: `crates/node/src/builder.rs`

- [ ] **Step 1: Rewrite builder.rs**

Key changes:
- Remove all `reth` / `reth_exex` / `reth_node_api` imports
- `SignetNodeBuilder<Host, Storage, Aof>` → `SignetNodeBuilder<Notifier, Storage, Aof>`
- `with_ctx` → `with_notifier`, takes `N: HostNotifier`
- Add `with_chain_name(String)`, `with_blob_cacher(CacheHandle)`, `with_serve_config(ServeConfig)` methods
- Add `chain_name: Option<String>`, `blob_cacher: Option<CacheHandle>`, `serve_config: Option<ServeConfig>` fields
- Remove the `build()` impl that creates `RethAliasOracleFactory` from provider (this moves to signet-host-reth or is done by the caller)
- The remaining `build()` requires an explicit `AliasOracleFactory`
- `prebuild` no longer needs `ExExContext` — it only does storage genesis checks

- [ ] **Step 2: Attempt compilation**

Run: `cargo clippy -p signet-node --all-features --all-targets 2>&1 | head -40`
Expected: Errors in rpc.rs and metrics.rs still.

- [ ] **Step 3: Commit**

```bash
git add crates/node/src/builder.rs
git commit -m "refactor(node): update SignetNodeBuilder for HostNotifier"
```

---

### Task 11: Refactor metrics and RPC modules

Update metrics to use `HostNotification` and RPC to use pre-built configs.

**Files:**
- Modify: `crates/node/src/metrics.rs`
- Modify: `crates/node/src/rpc.rs`

- [ ] **Step 1: Update metrics.rs**

Replace reth notification types with `HostNotificationKind`:

```rust
use signet_extract::Extractable;
use signet_node_types::HostNotification;
```

Change `record_notification_received` and `record_notification_processed` to take `&HostNotification<C>` where `C: Extractable`:

```rust
pub(crate) fn record_notification_received<C: Extractable>(
    notification: &HostNotification<C>,
) {
    inc_notifications_received();
    if notification.kind.reverted_chain().is_some() {
        inc_reorgs_received();
    }
}

pub(crate) fn record_notification_processed<C: Extractable>(
    notification: &HostNotification<C>,
) {
    inc_notifications_processed();
    if notification.kind.reverted_chain().is_some() {
        inc_reorgs_processed();
    }
}
```

Remove the `reth` and `reth_exex` imports.

- [ ] **Step 2: Update rpc.rs**

Remove all reth imports. The RPC module now receives pre-built `ServeConfig` and `StorageRpcConfig`. The `SignetNode` struct holds these as fields (set via builder).

The `launch_rpc` method no longer calls `self.config.merge_rpc_configs(&self.host)`. Instead it uses `self.serve_config` and `self.rpc_config` directly.

Remove `rpc_config_from_args` and `serve_config_from_args` (moved to `signet-host-reth`).

- [ ] **Step 3: Remove alias.rs from signet-node**

Delete `crates/node/src/alias.rs` — it now lives in `signet-host-reth`.

- [ ] **Step 4: Lint**

Run: `cargo clippy -p signet-node --all-features --all-targets`
Expected: Clean pass. signet-node should have zero reth imports.

Run: `cargo +nightly fmt`

- [ ] **Step 5: Verify no reth imports remain in source**

Run: `grep -r "reth" crates/node/src/`
Expected: No matches (except possibly comments).

- [ ] **Step 6: Remove reth deps from Cargo.toml**

In `crates/node/Cargo.toml`, remove:
```toml
reth.workspace = true
reth-exex.workspace = true
reth-node-api.workspace = true
reth-stages-types.workspace = true
```

- [ ] **Step 7: Final lint with reth deps removed**

Run: `cargo clippy -p signet-node --all-features --all-targets`
Expected: Clean pass.

- [ ] **Step 8: Commit**

```bash
git add crates/node/
git commit -m "refactor(node): remove all reth dependencies from signet-node"
```

---

### Task 12: Remove ExEx dependency from signet-node-config

Remove the `merge_rpc_configs` method and its reth ExEx/node-api deps. Note: `reth` and `reth-chainspec` deps remain — they are used by `core.rs` for `ChainSpec`, `StaticFileProvider`, etc. Full reth removal from node-config is out of scope.

**Files:**
- Delete: `crates/node-config/src/rpc.rs`
- Modify: `crates/node-config/src/lib.rs` (remove `mod rpc;`)
- Modify: `crates/node-config/Cargo.toml`

- [ ] **Step 1: Delete `crates/node-config/src/rpc.rs`**

The file only contains `modify_args` and `merge_rpc_configs` — both depend on `ExExContext` and `RpcServerArgs`. Delete the entire file.

- [ ] **Step 2: Remove `mod rpc;` from lib.rs**

In `crates/node-config/src/lib.rs`, remove the `mod rpc;` line.

- [ ] **Step 3: Remove ExEx deps from Cargo.toml**

In `crates/node-config/Cargo.toml`, remove only:
```toml
reth-exex.workspace = true
reth-node-api.workspace = true
```

Keep `reth.workspace = true` and `reth-chainspec.workspace = true` — they are still used by `core.rs`.

- [ ] **Step 4: Lint**

Run: `cargo clippy -p signet-node-config --all-features --all-targets`
Run: `cargo clippy -p signet-node-config --no-default-features --all-targets`
Expected: Clean pass.

- [ ] **Step 5: Commit**

```bash
git add crates/node-config/
git commit -m "refactor(node-config): remove ExEx dependency"
```

---

## Chunk 4: Update tests

### Task 13: Update signet-node-tests

The test harness needs to construct `RethHostNotifier` from the test ExEx context and use the new builder API.

**Files:**
- Modify: `crates/node-tests/Cargo.toml`
- Modify: `crates/node-tests/src/context.rs`
- Modify: `crates/node-tests/src/lib.rs`

- [ ] **Step 1: Add signet-host-reth dependency**

In `crates/node-tests/Cargo.toml`, add:
```toml
signet-host-reth.workspace = true
```

- [ ] **Step 2: Update context.rs**

In `crates/node-tests/src/context.rs`, the `SignetTestContext::new()` method currently:
1. Creates a test ExEx context via `reth_exex_test_utils::test_exex_context()`
2. Passes it to `SignetNodeBuilder::new(config).with_ctx(ctx)...build()`

Update to:
1. Create test ExEx context (same as before)
2. Call `decompose_exex_context(ctx)` to get `DecomposedContext`
3. Build blob cacher from the pool
4. Pass `notifier`, `blob_cacher`, `serve_config`, `chain_name` to the new builder API

The `send_notification` method currently sends via `self.handle.notifications_tx`. This should still work — the `RethHostNotifier` wraps the same notification stream that the test handle writes to.

- [ ] **Step 3: Update lib.rs re-exports if needed**

Ensure `signet-host-reth` types are available if needed by tests.

- [ ] **Step 4: Run tests**

Run: `cargo t -p signet-node-tests`
Expected: All tests pass.

- [ ] **Step 5: Lint**

Run: `cargo clippy -p signet-node-tests --all-features --all-targets`

- [ ] **Step 6: Commit**

```bash
git add crates/node-tests/
git commit -m "test: update signet-node-tests for HostNotifier API"
```

---

### Task 14: Full workspace verification

- [ ] **Step 1: Run full workspace clippy**

Run: `cargo clippy --workspace --all-features --all-targets`
Expected: Clean pass.

- [ ] **Step 2: Run full workspace tests**

Run: `cargo t --workspace`
Expected: All tests pass.

- [ ] **Step 3: Format**

Run: `cargo +nightly fmt`
Expected: No changes (already formatted).

- [ ] **Step 4: Final commit if any formatting changes**

```bash
git add -A
git commit -m "chore: final formatting pass"
```

---

## File Map Summary

| Action | Path | Purpose |
|--------|------|---------|
| Modify | `../sdk/crates/extract/src/trait.rs` | Add `first_number`, `tip_number`, `len`, `is_empty` to `Extractable` |
| Modify | `Cargo.toml` (workspace) | Uncomment SDK path patches, add new crate deps |
| Create | `crates/node-types/` | New crate: `HostNotifier`, `HostNotification`, `HostNotificationKind` |
| Create | `crates/host-reth/` | New crate: `RethHostNotifier`, `RethChain`, alias oracle, config helpers |
| Modify | `crates/node/Cargo.toml` | Remove reth deps, add signet-node-types |
| Modify | `crates/node/src/node.rs` | Replace `ExExContext` with `HostNotifier` |
| Modify | `crates/node/src/builder.rs` | Update builder for new API |
| Modify | `crates/node/src/metrics.rs` | Use `HostNotification` types |
| Modify | `crates/node/src/rpc.rs` | Use pre-built configs |
| Delete | `crates/node/src/alias.rs` | Moved to signet-host-reth |
| Modify | `crates/node/src/lib.rs` | Remove alias re-exports |
| Modify | `crates/node-config/src/rpc.rs` | Remove reth-dependent methods |
| Modify | `crates/node-config/Cargo.toml` | Remove reth deps |
| Modify | `crates/node-tests/Cargo.toml` | Add signet-host-reth dep |
| Modify | `crates/node-tests/src/context.rs` | Use `decompose_exex_context` |
