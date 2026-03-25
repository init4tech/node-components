# Parity `trace_` Namespace

Add the Parity/OpenEthereum `trace_` JSON-RPC namespace to signet-rpc for
Blockscout and general tooling compatibility. Driven by ENG-1064 and ENG-1065.

## Scope

9 methods in a new `trace` namespace. No block/uncle reward traces — Signet
is a post-merge L2. No `debug_storageRangeAt` — Geth's hashed-key pagination
format doesn't match Signet's plain-key storage, and reth hasn't implemented
it either.

## Methods

| Method | Input | Output |
|--------|-------|--------|
| `trace_block` | `BlockNumberOrTag` | `Option<Vec<LocalizedTransactionTrace>>` |
| `trace_transaction` | `B256` | `Option<Vec<LocalizedTransactionTrace>>` |
| `trace_replayBlockTransactions` | `BlockNumberOrTag, HashSet<TraceType>` | `Option<Vec<TraceResultsWithTransactionHash>>` |
| `trace_replayTransaction` | `B256, HashSet<TraceType>` | `TraceResults` |
| `trace_call` | `TransactionRequest, HashSet<TraceType>, Option<BlockId>` | `TraceResults` |
| `trace_callMany` | `Vec<(TransactionRequest, HashSet<TraceType>)>, Option<BlockId>` | `Vec<TraceResults>` |
| `trace_rawTransaction` | `Bytes, HashSet<TraceType>, Option<BlockId>` | `TraceResults` |
| `trace_get` | `B256, Vec<usize>` | `Option<LocalizedTransactionTrace>` |
| `trace_filter` | `TraceFilter` | `Vec<LocalizedTransactionTrace>` |

All types from `alloy::rpc::types::trace::parity` and
`alloy::rpc::types::trace::filter`.

## Architecture

### New files

- `crates/rpc/src/trace/mod.rs` — router (9 routes) + re-exports
- `crates/rpc/src/trace/endpoints.rs` — 9 handlers + 2 shared replay helpers
- `crates/rpc/src/trace/error.rs` — `TraceError` with `IntoErrorPayload`
- `crates/rpc/src/trace/types.rs` — param tuple structs for ajj positional
  param deserialization (following `debug/types.rs` pattern)

### Modified files

- `crates/rpc/src/debug/tracer.rs` — add 2 `pub(crate)` Parity tracer
  functions
- `crates/rpc/src/config/rpc_config.rs` — add `max_trace_filter_blocks`
- `crates/rpc/src/lib.rs` — add `mod trace`, export `TraceError`, nest router

### No new dependencies

`revm-inspectors` 0.34.2 already has `ParityTraceBuilder`. `alloy` 1.7.3
already has all Parity trace types. The existing `trace_flat_call` in
`debug/tracer.rs` already uses `inspector.into_parity_builder()`.

## Param Types

Tuple structs in `trace/types.rs` for ajj positional param deserialization,
following the pattern in `debug/types.rs`:

```
TraceBlockParams(BlockNumberOrTag)
TraceTransactionParams(B256)
ReplayBlockParams(BlockNumberOrTag, HashSet<TraceType>)
ReplayTransactionParams(B256, HashSet<TraceType>)
TraceCallParams(TransactionRequest, HashSet<TraceType>, Option<BlockId>,
                Option<StateOverride>, Option<Box<BlockOverrides>>)
TraceCallManyParams(Vec<(TransactionRequest, HashSet<TraceType>)>,
                    Option<BlockId>)
TraceRawTransactionParams(Bytes, HashSet<TraceType>, Option<BlockId>)
TraceGetParams(B256, Vec<usize>)
TraceFilterParams(TraceFilter)
```

`trace_call` includes state and block override fields to support reth's
`TraceCallRequest` semantics via positional params.

## Parity Tracer Functions

Two new `pub(crate)` functions in `debug/tracer.rs`:

**`trace_parity_localized(trevm, tx_info)`**
Returns `(Vec<LocalizedTransactionTrace>, EvmNeedsTx)`. Creates
`TracingInspector` with `TracingInspectorConfig::default_parity()`, runs the
tx via `try_with_inspector`, extracts `gas_used` from the result (matching
`trace_flat_call` pattern), converts via
`into_parity_builder().with_transaction_gas_used(gas).into_localized_transaction_traces(tx_info)`.

Used by: `trace_block`, `trace_transaction`, `trace_get`, `trace_filter`.

**`trace_parity_replay(trevm, trace_types)`**
Returns `(TraceResults, EvmNeedsTx)`. Requires `Db: Database + DatabaseRef`
(the `DatabaseRef` bound is needed for `populate_state_diff`). Creates
`TracingInspector` with
`TracingInspectorConfig::from_parity_config(&trace_types)`. Runs the tx,
then:

1. Takes result WITHOUT committing state (holds uncommitted state).
2. Converts via `into_parity_builder().into_trace_results(&result, &trace_types)`.
3. If `StateDiff` is requested: calls `populate_state_diff(&mut state_diff, &db, state.iter())` to enrich with pre-tx balance/nonce from the DB.
4. Then commits state.

This matches reth's `replayBlockTransactions` pattern.

Used by: `trace_replayBlockTransactions`, `trace_call`, `trace_callMany`,
`trace_rawTransaction`.

**Exception:** `trace_replayTransaction` uses
`into_trace_results_with_state(&res, &trace_types, &db)` instead. This
method takes `&ResultAndState` (a different type) and handles state diff
population internally. Matches reth's divergent pattern for single-tx replay.
The `DB::Error` from this call must be mapped into `TraceError`.

## Shared Block Replay Helpers

Two inner functions in `trace/endpoints.rs`:

**`trace_block_localized()`** — replays block txs, calls
`trace_parity_localized` for each. Stops at the first magic-sig tx (using
`peeking_take_while`, same as `debug::trace_block_inner`). Returns
`Vec<LocalizedTransactionTrace>`. No reward traces (Signet is post-merge).

**`trace_block_replay()`** — same replay loop but calls
`trace_parity_replay` with the caller's `HashSet<TraceType>`. Returns
`Vec<TraceResultsWithTransactionHash>`.

Both follow the same EVM setup pattern as `debug::trace_block_inner`:
`signet_evm::signet_evm()`, `fill_cfg`, `fill_block`, iterate txs with
`peeking_take_while`. All handlers use `tracing::debug_span!` +
`.instrument(span)` for instrumentation (matching existing debug endpoints).

## Method Details

### `trace_block`

Semaphore-gated. Resolves block, delegates to `trace_block_localized`.
Returns `None` if block not found. No reward traces.

### `trace_transaction`

Semaphore-gated. Finds tx by hash in cold storage, resolves containing block,
replays preceding txs without tracing (using `run_tx` + `accept_state`),
traces target tx with `trace_parity_localized`. Returns `None` if tx not
found.

### `trace_replayBlockTransactions`

Semaphore-gated. Resolves block, delegates to `trace_block_replay` with the
caller's `HashSet<TraceType>`. For each tx, wraps result in
`TraceResultsWithTransactionHash`. Returns `None` if block not found.

### `trace_replayTransaction`

Semaphore-gated. Replays preceding txs, traces target tx. Uses
`into_trace_results_with_state(&res, &trace_types, &db)` — different from
`replayBlockTransactions` which uses `into_trace_results()` +
`populate_state_diff()`. This matches reth's divergent pattern. Returns error
(not `None`) if tx not found.

### `trace_call`

Semaphore-gated. Resolves EVM state at block via `resolve_evm_block`.
Supports state overrides and block overrides (matching reth). Fills tx from
`TransactionRequest`, traces with `trace_parity_replay`. Defaults to latest
block if unspecified.

### `trace_callMany`

Semaphore-gated. Resolves EVM state once, then processes calls sequentially.
Each call can have different `HashSet<TraceType>`. State is committed between
calls via `db.commit(res.state)` — each subsequent call sees prior state
changes. Last call's state is not committed. Defaults to `BlockId::pending()`
if unspecified (matching reth).

### `trace_rawTransaction`

Semaphore-gated. Decodes RLP bytes into a transaction, recovers sender. Takes
optional `block_id` (defaults to latest). Traces with `trace_parity_replay`.

### `trace_get`

Semaphore-gated. Returns `None` if `indices.len() != 1` (Erigon
compatibility, matching reth). Delegates to `trace_transaction` to get all
traces, then selects the trace at `indices[0]`. Returns `None` if index is
out of bounds or tx not found.

### `trace_filter`

Semaphore-gated. Validates block range:
- `from_block` defaults to 0, `to_block` defaults to latest
- Both must be <= latest block
- `from_block` must be <= `to_block`
- Range must be <= `max_trace_filter_blocks` (default 100, configurable)

Processes blocks sequentially. For each block, calls
`trace_block_localized()`, filters results with
`TraceFilter::matcher().matches(&trace.trace)`. Applies `after` (skip) and
`count` (limit) pagination. No reward traces.

## Error Type

`TraceError` in `trace/error.rs`:

```
Cold(ColdStorageError)       — -32000
Hot(StorageError)            — -32000
Resolve(ResolveError)        — via resolve_error_code()
EvmHalt { reason: String }   — -32000
BlockNotFound(BlockId)       — -32001
TransactionNotFound(B256)    — -32001
RlpDecode(String)            — -32602
SenderRecovery               — -32000
BlockRangeExceeded           — -32602
```

Implements `IntoErrorPayload`. Reuses `resolve_error_code` and
`resolve_error_message` from `crate::eth::error`. Error messages are
sanitized — storage/DB errors return `"server error"`, no internals leaked.

## Configuration

New field in `StorageRpcConfig`:

- `max_trace_filter_blocks: u64` — default 100
- Env var: `SIGNET_RPC_MAX_TRACE_FILTER_BLOCKS`
- Builder setter: `max_trace_filter_blocks(u64)`

## Router

```
router()
  |- eth::eth()       (41 methods)
  |- debug::debug()   (9 methods)
  |- signet::signet() (2 methods)
  |- web3::web3()     (2 methods)
  |- net::net()       (2 methods)
  +- trace::trace()   (9 methods)
```

65 total routes.

## Ordering

This work depends on PR #120 (namespace completeness) which depends on
PR #119 (structured error codes). Branch off PR #120's head.
