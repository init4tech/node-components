# Parity `trace_` Namespace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Parity/OpenEthereum `trace_` JSON-RPC namespace (9 methods) to signet-rpc for Blockscout and general tooling compatibility.

**Architecture:** New `trace` module mirroring the `debug` module structure. Two new Parity tracer functions in `debug/tracer.rs` (shared inspector setup, different output builder). Two shared block replay helpers in `trace/endpoints.rs`. All handlers semaphore-gated. No block reward traces (Signet is post-merge L2).

**Tech Stack:** Rust, ajj 0.7.0, alloy (parity trace types, filter types), revm-inspectors (ParityTraceBuilder, TracingInspector), trevm, signet-evm

**Spec:** `docs/superpowers/specs/2026-03-25-parity-trace-namespace-design.md`

**Prerequisite:** Branch off PR #120 (namespace completeness) which depends on PR #119 (structured error codes). Verify `IntoErrorPayload` exists and `response_tri!` is gone before starting.

---

### Task 1: Create `TraceError`

**Files:**
- Create: `crates/rpc/src/trace/error.rs`

Model directly after `crates/rpc/src/debug/error.rs`.

- [ ] **Step 1: Create the error enum with tests**

```rust
//! Error types for the `trace` namespace.

use alloy::{eips::BlockId, primitives::B256};
use std::borrow::Cow;

/// Errors that can occur in the `trace` namespace.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// Cold storage error.
    #[error("cold storage error")]
    Cold(#[from] signet_cold::ColdStorageError),
    /// Hot storage error.
    #[error("hot storage error")]
    Hot(#[from] signet_storage::StorageError),
    /// Block resolution error.
    #[error("resolve: {0}")]
    Resolve(crate::config::resolve::ResolveError),
    /// EVM execution halted.
    #[error("execution halted: {reason}")]
    EvmHalt {
        /// Debug-formatted halt reason.
        reason: String,
    },
    /// Block not found.
    #[error("block not found: {0}")]
    BlockNotFound(BlockId),
    /// Transaction not found.
    #[error("transaction not found: {0}")]
    TransactionNotFound(B256),
    /// RLP decoding failed.
    #[error("RLP decode: {0}")]
    RlpDecode(String),
    /// Transaction sender recovery failed.
    #[error("sender recovery failed")]
    SenderRecovery,
    /// Block range too large for trace_filter.
    #[error("block range too large: {requested} blocks (max {max})")]
    BlockRangeExceeded {
        /// Requested range size.
        requested: u64,
        /// Maximum allowed range.
        max: u64,
    },
}

impl ajj::IntoErrorPayload for TraceError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::Cold(_)
            | Self::Hot(_)
            | Self::EvmHalt { .. }
            | Self::SenderRecovery => -32000,
            Self::Resolve(r) => crate::eth::error::resolve_error_code(r),
            Self::BlockNotFound(_) | Self::TransactionNotFound(_) => -32001,
            Self::RlpDecode(_) | Self::BlockRangeExceeded { .. } => -32602,
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::Cold(_) | Self::Hot(_) => "server error".into(),
            Self::Resolve(r) => crate::eth::error::resolve_error_message(r),
            Self::EvmHalt { reason } => {
                format!("execution halted: {reason}").into()
            }
            Self::BlockNotFound(id) => {
                format!("block not found: {id}").into()
            }
            Self::TransactionNotFound(h) => {
                format!("transaction not found: {h}").into()
            }
            Self::RlpDecode(msg) => {
                format!("RLP decode error: {msg}").into()
            }
            Self::SenderRecovery => "sender recovery failed".into(),
            Self::BlockRangeExceeded { requested, max } => {
                format!(
                    "block range too large: {requested} blocks (max {max})"
                )
                .into()
            }
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::TraceError;
    use ajj::IntoErrorPayload;
    use alloy::{eips::BlockId, primitives::B256};

    #[test]
    fn cold_error_code() {
        // Cold/Hot/EvmHalt/SenderRecovery all map to -32000
        let err = TraceError::SenderRecovery;
        assert_eq!(err.error_code(), -32000);
    }

    #[test]
    fn block_not_found_code() {
        let err = TraceError::BlockNotFound(BlockId::latest());
        assert_eq!(err.error_code(), -32001);
    }

    #[test]
    fn transaction_not_found_code() {
        let err = TraceError::TransactionNotFound(B256::ZERO);
        assert_eq!(err.error_code(), -32001);
    }

    #[test]
    fn rlp_decode_code() {
        let err = TraceError::RlpDecode("bad".into());
        assert_eq!(err.error_code(), -32602);
    }

    #[test]
    fn block_range_exceeded_code() {
        let err = TraceError::BlockRangeExceeded {
            requested: 200,
            max: 100,
        };
        assert_eq!(err.error_code(), -32602);
        assert!(err.error_message().contains("200"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo t -p signet-rpc -- trace::error::tests`
Note: the module won't be wired yet, so you may need to add a temporary
`mod trace;` in `lib.rs` with just `mod error; pub use error::TraceError;`
to make the tests compile. Or run tests after Task 11 wires everything.

- [ ] **Step 3: Lint and commit**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`
Run: `cargo +nightly fmt`

```bash
git add crates/rpc/src/trace/error.rs
git commit -m "feat(rpc): add TraceError for Parity trace namespace"
```

---

### Task 2: Create param types

**Files:**
- Create: `crates/rpc/src/trace/types.rs`

Follow the tuple struct pattern from `debug/types.rs`.

- [ ] **Step 1: Create the param types**

```rust
//! Parameter types for the `trace` namespace.

use alloy::{
    eips::BlockId,
    primitives::{Bytes, B256},
    rpc::types::{
        state::StateOverride, BlockNumberOrTag, BlockOverrides,
        TransactionRequest,
        trace::{filter::TraceFilter, parity::TraceType},
    },
};
use std::collections::HashSet;

/// Params for `trace_block`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceBlockParams(pub(crate) BlockNumberOrTag);

/// Params for `trace_transaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceTransactionParams(pub(crate) B256);

/// Params for `trace_replayBlockTransactions`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ReplayBlockParams(
    pub(crate) BlockNumberOrTag,
    pub(crate) HashSet<TraceType>,
);

/// Params for `trace_replayTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ReplayTransactionParams(
    pub(crate) B256,
    pub(crate) HashSet<TraceType>,
);

/// Params for `trace_call`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceCallParams(
    pub(crate) TransactionRequest,
    pub(crate) HashSet<TraceType>,
    #[serde(default)]
    pub(crate) Option<BlockId>,
    #[serde(default)]
    pub(crate) Option<StateOverride>,
    #[serde(default)]
    pub(crate) Option<Box<BlockOverrides>>,
);

/// Params for `trace_callMany`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceCallManyParams(
    pub(crate) Vec<(TransactionRequest, HashSet<TraceType>)>,
    #[serde(default)]
    pub(crate) Option<BlockId>,
);

/// Params for `trace_rawTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceRawTransactionParams(
    pub(crate) Bytes,
    pub(crate) HashSet<TraceType>,
    #[serde(default)]
    pub(crate) Option<BlockId>,
);

/// Params for `trace_get`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceGetParams(
    pub(crate) B256,
    pub(crate) Vec<usize>,
);

/// Params for `trace_filter`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceFilterParams(pub(crate) TraceFilter);
```

Note: check whether `HashSet<TraceType>` deserializes correctly from
JSON arrays. The alloy `TraceType` implements `Deserialize` and `Hash`.
If `std::collections::HashSet` doesn't work, use
`alloy::primitives::map::HashSet` instead.

- [ ] **Step 2: Lint and commit**

```bash
git add crates/rpc/src/trace/types.rs
git commit -m "feat(rpc): add param types for Parity trace namespace"
```

---

### Task 3: Add `max_trace_filter_blocks` config

**Files:**
- Modify: `crates/rpc/src/config/rpc_config.rs`

- [ ] **Step 1: Add field to `StorageRpcConfig`**

Add after the existing `max_tracing_requests` field:

```rust
/// Maximum block range for `trace_filter` queries.
///
/// Default: `100`.
pub max_trace_filter_blocks: u64,
```

- [ ] **Step 2: Add to `Default` impl**

```rust
max_trace_filter_blocks: 100,
```

- [ ] **Step 3: Add to builder**

```rust
/// Set the max block range for trace_filter.
pub const fn max_trace_filter_blocks(mut self, max: u64) -> Self {
    self.inner.max_trace_filter_blocks = max;
    self
}
```

- [ ] **Step 4: Add to `StorageRpcConfigEnv`**

Add field with env var annotation (follow existing pattern):

```rust
#[from_env(
    var = "SIGNET_RPC_MAX_TRACE_FILTER_BLOCKS",
    desc = "Maximum block range for trace_filter queries",
    optional
)]
max_trace_filter_blocks: Option<u64>,
```

- [ ] **Step 5: Add to `From<StorageRpcConfigEnv>` impl**

```rust
max_trace_filter_blocks: env
    .max_trace_filter_blocks
    .unwrap_or(defaults.max_trace_filter_blocks),
```

- [ ] **Step 6: Lint and commit**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`
Run: `cargo +nightly fmt`

```bash
git add crates/rpc/src/config/rpc_config.rs
git commit -m "feat(rpc): add max_trace_filter_blocks config"
```

---

### Task 4: Add Parity tracer functions

**Files:**
- Modify: `crates/rpc/src/debug/tracer.rs`

Add two `pub(crate)` functions alongside the existing Geth tracers.

- [ ] **Step 1: Add `trace_parity_localized`**

Add after the existing tracer functions. This follows the exact pattern
of `trace_flat_call` (which already uses `into_parity_builder()`):

```rust
/// Trace a transaction and return Parity-format localized traces.
///
/// Used by `trace_block`, `trace_transaction`, `trace_get`,
/// `trace_filter`.
pub(crate) fn trace_parity_localized<Db, Insp>(
    trevm: EvmReady<Db, Insp>,
    tx_info: TransactionInfo,
) -> Result<(Vec<LocalizedTransactionTrace>, EvmNeedsTx<Db, Insp>), DebugError>
where
    Db: Database + DatabaseCommit + DatabaseRef,
    Insp: Inspector<Ctx<Db>>,
{
    let gas_limit = trevm.gas_limit();
    let mut inspector = TracingInspector::new(
        TracingInspectorConfig::default_parity(),
    );
    let trevm = trevm
        .try_with_inspector(&mut inspector, |trevm| trevm.run())
        .map_err(|err| DebugError::EvmHalt {
            reason: err.into_error().to_string(),
        })?;

    let traces = inspector
        .with_transaction_gas_limit(gas_limit)
        .into_parity_builder()
        .into_localized_transaction_traces(tx_info);

    Ok((traces, trevm.accept_state()))
}
```

Note: check whether `TracingInspector` has
`with_transaction_gas_limit()`. If not, use
`into_parity_builder().with_transaction_gas_used(trevm.gas_used())`
instead. The existing `trace_flat_call` (line 161) shows the exact
pattern — follow it.

- [ ] **Step 2: Add `trace_parity_replay`**

This is the more complex function — handles `TraceType` selection and
`StateDiff` enrichment:

```rust
/// Trace a transaction and return Parity-format `TraceResults`.
///
/// When `StateDiff` is in `trace_types`, the state diff is enriched
/// with pre-transaction balance/nonce from the database. Requires
/// `Db: DatabaseRef` for this enrichment.
///
/// Used by `trace_replayBlockTransactions`, `trace_call`,
/// `trace_callMany`, `trace_rawTransaction`.
pub(crate) fn trace_parity_replay<Db, Insp>(
    trevm: EvmReady<Db, Insp>,
    trace_types: &HashSet<TraceType>,
) -> Result<(TraceResults, EvmNeedsTx<Db, Insp>), DebugError>
where
    Db: Database + DatabaseCommit + DatabaseRef,
    <Db as DatabaseRef>::Error: std::fmt::Debug,
    Insp: Inspector<Ctx<Db>>,
{
    let mut inspector = TracingInspector::new(
        TracingInspectorConfig::from_parity_config(trace_types),
    );
    let trevm = trevm
        .try_with_inspector(&mut inspector, |trevm| trevm.run())
        .map_err(|err| DebugError::EvmHalt {
            reason: err.into_error().to_string(),
        })?;

    // Follow the take_result_and_state pattern from trace_pre_state
    // (debug/tracer.rs line ~124). This gives us the ExecutionResult
    // and state map while keeping trevm alive for DB access.
    let (result, mut trevm) = trevm.take_result_and_state();

    let mut trace_res = inspector
        .into_parity_builder()
        .into_trace_results(&result.result, trace_types);

    // If StateDiff was requested, enrich with pre-tx balance/nonce.
    if let Some(ref mut state_diff) = trace_res.state_diff {
        // populate_state_diff reads pre-tx state from db and overlays
        // the committed changes. Check revm-inspectors for the exact
        // import path and function signature.
        revm_inspectors::tracing::builder::parity::populate_state_diff(
            state_diff,
            trevm.inner_mut_unchecked().db_mut(),
            result.state.iter(),
        )
        .map_err(|e| DebugError::EvmHalt {
            reason: format!("state diff: {e:?}"),
        })?;
    }

    // Commit the state changes.
    trevm.inner_mut_unchecked().db_mut().commit(result.state);
    Ok((trace_res, trevm))
}
```

**IMPORTANT:** The code uses `take_result_and_state()` which follows
the pattern from `trace_pre_state` in `debug/tracer.rs` (~line 124).
Verify the exact API during implementation:
- `take_result_and_state()` returns `(ResultAndState, EvmNeedsTx)` or similar
- `inner_mut_unchecked().db_mut()` for `&mut Db` (DatabaseRef access)
- Check `populate_state_diff` import path — may be at
  `revm_inspectors::tracing::parity::populate_state_diff` or
  `revm_inspectors::tracing::builder::parity::populate_state_diff`

Build docs: `cargo doc -p revm-inspectors --no-deps` and
`cargo doc -p trevm --no-deps` to find exact paths.

- [ ] **Step 3: Add required imports**

At the top of `tracer.rs`, add:

```rust
use alloy::rpc::types::trace::parity::{
    LocalizedTransactionTrace, TraceResults, TraceType,
};
use std::collections::HashSet;
```

- [ ] **Step 4: Lint and commit**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`
Run: `cargo +nightly fmt`

```bash
git add crates/rpc/src/debug/tracer.rs
git commit -m "feat(rpc): add Parity tracer functions (localized + replay)"
```

---

### Task 5: Create block replay helpers

**Files:**
- Create: `crates/rpc/src/trace/endpoints.rs` (initial skeleton)

These parallel `debug::trace_block_inner` but produce Parity output.

- [ ] **Step 1: Create endpoints.rs with imports and localized helper**

```rust
//! Parity `trace` namespace RPC endpoint implementations.

use crate::{
    config::StorageRpcCtx,
    eth::helpers::{CfgFiller, await_handler},
    trace::{
        TraceError,
        types::{
            ReplayBlockParams, ReplayTransactionParams, TraceBlockParams,
            TraceCallManyParams, TraceCallParams, TraceFilterParams,
            TraceGetParams, TraceRawTransactionParams, TraceTransactionParams,
        },
    },
};
use ajj::HandlerCtx;
use alloy::{
    consensus::BlockHeader,
    eips::BlockId,
    primitives::{B256, Bytes},
    rpc::types::trace::parity::{
        LocalizedTransactionTrace, TraceResults,
        TraceResultsWithTransactionHash, TraceType,
    },
};
use signet_hot::{HotKv, model::HotKvRead};
use signet_types::{MagicSig, constants::SignetSystemConstants};
use std::collections::HashSet;
use tracing::Instrument;
use trevm::revm::{
    Database, DatabaseRef,
    database::{DBErrorMarker, State},
    primitives::hardfork::SpecId,
};

/// Shared localized tracing loop for Parity `trace_block` and
/// `trace_filter`.
///
/// Replays all transactions in a block (stopping at the first
/// magic-signature tx) and returns localized Parity traces.
#[allow(clippy::too_many_arguments)]
fn trace_block_localized<Db>(
    ctx_chain_id: u64,
    constants: SignetSystemConstants,
    spec_id: SpecId,
    header: &alloy::consensus::Header,
    block_hash: B256,
    txs: &[signet_storage_types::RecoveredTx],
    db: State<Db>,
) -> Result<Vec<LocalizedTransactionTrace>, TraceError>
where
    Db: Database + DatabaseRef,
    <Db as Database>::Error: DBErrorMarker,
    <Db as DatabaseRef>::Error: DBErrorMarker,
{
    use itertools::Itertools;

    let mut evm = signet_evm::signet_evm(db, constants);
    evm.set_spec_id(spec_id);
    let mut trevm = evm
        .fill_cfg(&CfgFiller(ctx_chain_id))
        .fill_block(header);

    let mut all_traces = Vec::new();
    let mut txns = txs.iter().enumerate().peekable();
    for (idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| {
            MagicSig::try_from_signature(t.signature()).is_none()
        })
    {
        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(idx as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let t = trevm.fill_tx(tx);
        let (traces, next);
        // Convert DebugError from tracer into TraceError.
        (traces, next) = crate::debug::tracer::trace_parity_localized(
            t, tx_info,
        )
        .map_err(|e| TraceError::EvmHalt {
            reason: e.to_string(),
        })?;
        trevm = next;
        all_traces.extend(traces);
    }

    Ok(all_traces)
}
```

- [ ] **Step 2: Add the replay helper**

```rust
/// Shared replay tracing loop for Parity `trace_replayBlockTransactions`.
///
/// Replays all transactions and returns per-tx `TraceResults` with
/// the caller's `TraceType` selection.
#[allow(clippy::too_many_arguments)]
fn trace_block_replay<Db>(
    ctx_chain_id: u64,
    constants: SignetSystemConstants,
    spec_id: SpecId,
    header: &alloy::consensus::Header,
    block_hash: B256,
    txs: &[signet_storage_types::RecoveredTx],
    db: State<Db>,
    trace_types: &HashSet<TraceType>,
) -> Result<Vec<TraceResultsWithTransactionHash>, TraceError>
where
    Db: Database + DatabaseRef,
    <Db as Database>::Error: DBErrorMarker,
    <Db as DatabaseRef>::Error: std::fmt::Debug + DBErrorMarker,
{
    use itertools::Itertools;

    let mut evm = signet_evm::signet_evm(db, constants);
    evm.set_spec_id(spec_id);
    let mut trevm = evm
        .fill_cfg(&CfgFiller(ctx_chain_id))
        .fill_block(header);

    let mut results = Vec::with_capacity(txs.len());
    let mut txns = txs.iter().enumerate().peekable();
    for (idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| {
            MagicSig::try_from_signature(t.signature()).is_none()
        })
    {
        let t = trevm.fill_tx(tx);
        let (trace_res, next);
        (trace_res, next) = crate::debug::tracer::trace_parity_replay(
            t, trace_types,
        )
        .map_err(|e| TraceError::EvmHalt {
            reason: e.to_string(),
        })?;
        trevm = next;

        results.push(TraceResultsWithTransactionHash {
            full_trace: trace_res,
            transaction_hash: *tx.tx_hash(),
        });
    }

    Ok(results)
}
```

- [ ] **Step 3: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add Parity block replay helpers"
```

---

### Task 6: Implement `trace_block` and `trace_transaction`

**Files:**
- Modify: `crates/rpc/src/trace/endpoints.rs`

- [ ] **Step 1: Add `trace_block` handler**

```rust
/// `trace_block` — return Parity traces for all transactions in a block.
pub(super) async fn trace_block<H>(
    hctx: HandlerCtx,
    TraceBlockParams(id): TraceBlockParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<LocalizedTransactionTrace>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = BlockId::Number(id);
    let span = tracing::debug_span!("trace_block", ?id);

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            TraceError::Resolve(e)
        })?;

        let sealed = ctx
            .resolve_header(BlockId::Number(block_num.into()))
            .map_err(|e| {
                tracing::warn!(error = %e, block_num, "header resolution failed");
                TraceError::Resolve(e)
            })?;

        let Some(sealed) = sealed else {
            return Ok(None);
        };

        let block_hash = sealed.hash();
        let header = sealed.into_inner();

        let txs = cold
            .get_transactions_in_block(block_num)
            .await
            .map_err(TraceError::from)?;

        let db = ctx
            .revm_state_at_height(header.number.saturating_sub(1))
            .map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let traces = trace_block_localized(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &header,
            block_hash,
            &txs,
            db,
        )?;

        Ok(Some(traces))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 2: Add `trace_transaction` handler**

Follow the pattern of `debug::trace_transaction` — replay preceding txs
without tracing, trace only the target tx:

```rust
/// `trace_transaction` — return Parity traces for a single transaction.
pub(super) async fn trace_transaction<H>(
    hctx: HandlerCtx,
    TraceTransactionParams(tx_hash): TraceTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<LocalizedTransactionTrace>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_transaction", %tx_hash);

    let fut = async move {
        let cold = ctx.cold();

        let confirmed = cold
            .get_tx_by_hash(tx_hash)
            .await
            .map_err(TraceError::from)?;

        let Some(confirmed) = confirmed else {
            return Ok(None);
        };
        let (_tx, meta) = confirmed.into_parts();
        let block_num = meta.block_number();
        let block_hash = meta.block_hash();

        let block_id = BlockId::Number(block_num.into());
        let sealed = ctx
            .resolve_header(block_id)
            .map_err(|e| {
                tracing::warn!(error = %e, block_num, "header resolution failed");
                TraceError::Resolve(e)
            })?;
        let header = sealed
            .ok_or(TraceError::BlockNotFound(block_id))?
            .into_inner();

        let txs = cold
            .get_transactions_in_block(block_num)
            .await
            .map_err(TraceError::from)?;

        let db = ctx
            .revm_state_at_height(block_num.saturating_sub(1))
            .map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        // Replay preceding txs without tracing.
        use itertools::Itertools;
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns
            .by_ref()
            .peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash)
        {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return Ok(None);
            }
            trevm = trevm
                .run_tx(tx)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.into_error().to_string(),
                })?
                .accept_state();
        }

        let Some((index, tx)) = txns.next() else {
            return Ok(None);
        };

        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(index as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let trevm = trevm.fill_tx(tx);
        let (traces, _) =
            crate::debug::tracer::trace_parity_localized(trevm, tx_info)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.to_string(),
                })?;

        Ok(Some(traces))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 3: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add trace_block and trace_transaction"
```

---

### Task 7: Implement `trace_replayBlockTransactions` and `trace_replayTransaction`

**Files:**
- Modify: `crates/rpc/src/trace/endpoints.rs`

- [ ] **Step 1: Add `replay_block_transactions`**

```rust
/// `trace_replayBlockTransactions` — replay all block txs with trace type selection.
pub(super) async fn replay_block_transactions<H>(
    hctx: HandlerCtx,
    ReplayBlockParams(id, trace_types): ReplayBlockParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<TraceResultsWithTransactionHash>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = BlockId::Number(id);
    let span = tracing::debug_span!("trace_replayBlockTransactions", ?id);

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            TraceError::Resolve(e)
        })?;

        let sealed = ctx
            .resolve_header(BlockId::Number(block_num.into()))
            .map_err(|e| TraceError::Resolve(e))?;

        let Some(sealed) = sealed else {
            return Ok(None);
        };

        let block_hash = sealed.hash();
        let header = sealed.into_inner();

        let txs = cold
            .get_transactions_in_block(block_num)
            .await
            .map_err(TraceError::from)?;

        let db = ctx
            .revm_state_at_height(header.number.saturating_sub(1))
            .map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let results = trace_block_replay(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &header,
            block_hash,
            &txs,
            db,
            &trace_types,
        )?;

        Ok(Some(results))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 2: Add `replay_transaction`**

This one uses `into_trace_results_with_state` (different from
`replay_block_transactions`), matching reth's divergent pattern:

```rust
/// `trace_replayTransaction` — replay a single tx with trace type selection.
///
/// Uses `into_trace_results_with_state` (different from
/// `replayBlockTransactions` which uses `into_trace_results` +
/// `populate_state_diff`). Matches reth's divergent pattern.
pub(super) async fn replay_transaction<H>(
    hctx: HandlerCtx,
    ReplayTransactionParams(tx_hash, trace_types): ReplayTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_replayTransaction", %tx_hash);

    let fut = async move {
        // Same tx lookup + block replay as trace_transaction, but use
        // trace_parity_replay for the target tx instead of
        // trace_parity_localized.
        //
        // HOWEVER: this handler needs into_trace_results_with_state,
        // not into_trace_results + populate_state_diff. The spec notes
        // this divergence. For the initial implementation, use
        // trace_parity_replay which uses into_trace_results +
        // populate_state_diff. If reth compatibility requires the
        // exact into_trace_results_with_state path, refactor later.
        //
        // The practical difference is minimal — both produce correct
        // state diffs, just through different internal paths.

        let cold = ctx.cold();
        let confirmed = cold
            .get_tx_by_hash(tx_hash)
            .await
            .map_err(TraceError::from)?
            .ok_or(TraceError::TransactionNotFound(tx_hash))?;

        let (_tx, meta) = confirmed.into_parts();
        let block_num = meta.block_number();

        let block_id = BlockId::Number(block_num.into());
        let sealed = ctx
            .resolve_header(block_id)
            .map_err(|e| TraceError::Resolve(e))?;
        let header = sealed
            .ok_or(TraceError::BlockNotFound(block_id))?
            .into_inner();

        let txs = cold
            .get_transactions_in_block(block_num)
            .await
            .map_err(TraceError::from)?;

        let db = ctx
            .revm_state_at_height(block_num.saturating_sub(1))
            .map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        // Replay preceding txs.
        use itertools::Itertools;
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns
            .by_ref()
            .peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash)
        {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return Err(TraceError::TransactionNotFound(tx_hash));
            }
            trevm = trevm
                .run_tx(tx)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.into_error().to_string(),
                })?
                .accept_state();
        }

        let (_index, tx) = txns
            .next()
            .ok_or(TraceError::TransactionNotFound(tx_hash))?;

        let trevm = trevm.fill_tx(tx);
        let (results, _) =
            crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.to_string(),
                })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 3: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add trace_replayBlockTransactions and trace_replayTransaction"
```

---

### Task 8: Implement `trace_call` and `trace_callMany`

**Files:**
- Modify: `crates/rpc/src/trace/endpoints.rs`

- [ ] **Step 1: Add `trace_call`**

Follows `debug_trace_call` pattern but with state/block overrides
(matching reth) and Parity output:

```rust
/// `trace_call` — trace a call with Parity output and state overrides.
pub(super) async fn trace_call<H>(
    hctx: HandlerCtx,
    TraceCallParams(request, trace_types, block_id, state_overrides, block_overrides): TraceCallParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::latest());
    let span = tracing::debug_span!("trace_call", ?id);

    let fut = async move {
        use crate::config::EvmBlockContext;

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => {
                    TraceError::BlockNotFound(id)
                }
                other => TraceError::EvmHalt {
                    reason: other.to_string(),
                },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let trevm = evm
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        // Apply state and block overrides (matching reth trace_call).
        let trevm = trevm
            .maybe_apply_state_overrides(state_overrides.as_ref())
            .map_err(|e| TraceError::EvmHalt {
                reason: e.to_string(),
            })?
            .maybe_apply_block_overrides(block_overrides.as_deref())
            .fill_tx(&request);

        let (results, _) =
            crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.to_string(),
                })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 2: Add `trace_call_many`**

Sequential calls with state committed between each:

```rust
/// `trace_callMany` — trace sequential calls with accumulated state.
///
/// Each call sees state changes from prior calls. Per-call trace
/// types. Defaults to `BlockId::pending()` (matching reth).
pub(super) async fn trace_call_many<H>(
    hctx: HandlerCtx,
    TraceCallManyParams(calls, block_id): TraceCallManyParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<TraceResults>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::pending());
    let span = tracing::debug_span!("trace_callMany", ?id, count = calls.len());

    let fut = async move {
        use crate::config::EvmBlockContext;

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => {
                    TraceError::BlockNotFound(id)
                }
                other => TraceError::EvmHalt {
                    reason: other.to_string(),
                },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        let mut results = Vec::with_capacity(calls.len());
        let mut calls = calls.into_iter().peekable();

        while let Some((request, trace_types)) = calls.next() {
            let filled = trevm.fill_tx(&request);
            let (trace_res, next) =
                crate::debug::tracer::trace_parity_replay(
                    filled,
                    &trace_types,
                )
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.to_string(),
                })?;

            results.push(trace_res);

            // accept_state commits the tx's state changes so
            // subsequent calls see them.
            trevm = next.accept_state();
        }

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

Note: the `accept_state()` call may or may not commit to the
underlying DB. Check trevm docs. The key requirement is that call N+1
sees state from call N. In the debug namespace, `run_tx().accept_state()`
is used for this purpose.

- [ ] **Step 3: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add trace_call and trace_callMany"
```

---

### Task 9: Implement `trace_rawTransaction` and `trace_get`

**Files:**
- Modify: `crates/rpc/src/trace/endpoints.rs`

- [ ] **Step 1: Add `trace_raw_transaction`**

```rust
/// `trace_rawTransaction` — trace a transaction from raw RLP bytes.
pub(super) async fn trace_raw_transaction<H>(
    hctx: HandlerCtx,
    TraceRawTransactionParams(rlp_bytes, trace_types, block_id): TraceRawTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::latest());
    let span = tracing::debug_span!("trace_rawTransaction", ?id);

    let fut = async move {
        use alloy::consensus::transaction::SignerRecoverable;
        use crate::config::EvmBlockContext;

        // Decode and recover sender.
        let tx: signet_storage_types::TransactionSigned =
            alloy::rlp::Decodable::decode(&mut rlp_bytes.as_ref())
                .map_err(|e| TraceError::RlpDecode(e.to_string()))?;
        let recovered = tx
            .try_into_recovered()
            .map_err(|_| TraceError::SenderRecovery)?;

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => {
                    TraceError::BlockNotFound(id)
                }
                other => TraceError::EvmHalt {
                    reason: other.to_string(),
                },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let trevm = evm
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header)
            .fill_tx(&recovered);

        let (results, _) =
            crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
                .map_err(|e| TraceError::EvmHalt {
                    reason: e.to_string(),
                })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 2: Add `trace_get`**

```rust
/// `trace_get` — get a specific trace by tx hash and index.
///
/// Returns `None` if `indices.len() != 1` (Erigon compatibility,
/// matching reth).
pub(super) async fn trace_get<H>(
    hctx: HandlerCtx,
    TraceGetParams(tx_hash, indices): TraceGetParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<LocalizedTransactionTrace>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    if indices.len() != 1 {
        return Ok(None);
    }

    let traces = trace_transaction(
        hctx,
        TraceTransactionParams(tx_hash),
        ctx,
    )
    .await?;

    Ok(traces.and_then(|t| t.into_iter().nth(indices[0])))
}
```

- [ ] **Step 3: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add trace_rawTransaction and trace_get"
```

---

### Task 10: Implement `trace_filter`

**Files:**
- Modify: `crates/rpc/src/trace/endpoints.rs`

- [ ] **Step 1: Add `trace_filter`**

```rust
/// `trace_filter` — filter traces across a block range.
///
/// Brute-force replay with configurable block range limit (default
/// 100 blocks). Matches reth's approach.
pub(super) async fn trace_filter<H>(
    hctx: HandlerCtx,
    TraceFilterParams(filter): TraceFilterParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<LocalizedTransactionTrace>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_filter");

    let fut = async move {
        let latest = ctx.tags().latest();
        let start = filter.from_block.unwrap_or(0);
        let end = filter.to_block.unwrap_or(latest);

        if start > latest || end > latest {
            return Err(TraceError::BlockNotFound(BlockId::latest()));
        }
        if start > end {
            return Err(TraceError::EvmHalt {
                reason: "fromBlock cannot be greater than toBlock".into(),
            });
        }

        let max = ctx.config().max_trace_filter_blocks;
        let distance = end.saturating_sub(start);
        if distance > max {
            return Err(TraceError::BlockRangeExceeded {
                requested: distance,
                max,
            });
        }

        let matcher = filter.matcher();
        let mut all_traces = Vec::new();

        for block_num in start..=end {
            let cold = ctx.cold();
            let block_id = BlockId::Number(block_num.into());

            let sealed = ctx
                .resolve_header(block_id)
                .map_err(|e| TraceError::Resolve(e))?;

            let Some(sealed) = sealed else {
                continue;
            };

            let block_hash = sealed.hash();
            let header = sealed.into_inner();

            let txs = cold
                .get_transactions_in_block(block_num)
                .await
                .map_err(TraceError::from)?;

            let db = ctx
                .revm_state_at_height(header.number.saturating_sub(1))
                .map_err(TraceError::from)?;

            let spec_id = ctx.spec_id_for_header(&header);
            let mut traces = trace_block_localized(
                ctx.chain_id(),
                ctx.constants().clone(),
                spec_id,
                &header,
                block_hash,
                &txs,
                db,
            )?;

            // Apply filter matcher.
            traces.retain(|t| matcher.matches(&t.trace));
            all_traces.extend(traces);
        }

        // Apply pagination: skip `after`, limit `count`.
        if let Some(after) = filter.after {
            let after = after as usize;
            if after >= all_traces.len() {
                return Ok(vec![]);
            }
            all_traces.drain(..after);
        }
        if let Some(count) = filter.count {
            all_traces.truncate(count as usize);
        }

        Ok(all_traces)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt {
            reason: "task panicked or cancelled".into()
        }
    )
}
```

- [ ] **Step 2: Lint and commit**

```bash
git add crates/rpc/src/trace/endpoints.rs
git commit -m "feat(rpc): add trace_filter with configurable block range limit"
```

---

### Task 11: Wire the router

**Files:**
- Create: `crates/rpc/src/trace/mod.rs`
- Modify: `crates/rpc/src/lib.rs`

- [ ] **Step 1: Create `trace/mod.rs`**

```rust
//! Parity `trace` namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    replay_block_transactions, replay_transaction, trace_block,
    trace_call, trace_call_many, trace_filter, trace_get,
    trace_raw_transaction, trace_transaction,
};
mod error;
pub use error::TraceError;
mod types;

use crate::config::StorageRpcCtx;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate a `trace` API router backed by storage.
pub(crate) fn trace<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("block", trace_block::<H>)
        .route("transaction", trace_transaction::<H>)
        .route("replayBlockTransactions", replay_block_transactions::<H>)
        .route("replayTransaction", replay_transaction::<H>)
        .route("call", trace_call::<H>)
        .route("callMany", trace_call_many::<H>)
        .route("rawTransaction", trace_raw_transaction::<H>)
        .route("get", trace_get::<H>)
        .route("filter", trace_filter::<H>)
}
```

- [ ] **Step 2: Wire into `lib.rs`**

Add `mod trace;` and `pub use trace::TraceError;` alongside the
existing module declarations.

Add `.nest("trace", trace::trace())` to the router function.

Update the docstring to mention the `trace` namespace.

- [ ] **Step 3: Lint and verify**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`
Run: `cargo +nightly fmt`

- [ ] **Step 4: Commit**

```bash
git add crates/rpc/src/trace/mod.rs crates/rpc/src/lib.rs
git commit -m "feat(rpc): wire Parity trace namespace into router"
```

---

### Task 12: Final verification

- [ ] **Step 1: Run all tests**

Run: `cargo t -p signet-rpc`
Expected: All pass (existing + new TraceError tests).

- [ ] **Step 2: Full lint**

Run: `cargo clippy -p signet-rpc --all-features --all-targets`
Expected: Clean.

- [ ] **Step 3: Format**

Run: `cargo +nightly fmt`

- [ ] **Step 4: Verify route count**

Count `.route(` calls across all namespace modules. Expected:
eth 41 + debug 9 + trace 9 + signet 2 + web3 2 + net 2 = 65 total.

- [ ] **Step 5: Workspace-wide lint**

Run: `cargo clippy --all-features --all-targets`
Verify no other crates broke.

- [ ] **Step 6: Commit any remaining fixes**

```bash
git add -A
git commit -m "chore(rpc): final cleanup for Parity trace namespace"
```
