# Future Evaluation Notes

## Lazy Serialization (ajj 0.5.0)

ajj 0.5.0 relaxed `RpcSend` bounds so that custom `Serialize` impls can be
returned from handlers without collecting into a `Vec`. Two endpoints now use
this:

- **`eth_getBlockReceipts`** — `LazyReceipts` serializes receipts inline from
  raw `ColdReceipt` + `RecoveredTx` data without an intermediate
  `Vec<RpcReceipt>`.
- **`eth_getBlockByHash` / `eth_getBlockByNumber`** — in-housed
  `BlockTransactions` and `RpcBlock` types serialize full transactions or
  hashes lazily from `Vec<RecoveredTx>`.

## Endpoints that cannot benefit from lazy serialization

- **`debug_traceBlockByNumber`** — EVM state is sequential and destructively
  consumed per transaction. Computation must be eager; the `Vec<TraceResult>`
  confirms all traces succeed before returning. Converting `DebugError` to
  `serde::ser::Error` would lose variant information.
- **`eth_feeHistory`** — Vecs feed into `FeeHistory` (alloy type) which uses
  `alloy_serde::quantity` custom serializers. In-housing is moderate effort
  for low payoff.
- **`eth_getLogs`** — `Vec<Log>` comes directly from cold storage with no
  transformation at the API boundary. Lazy serialization does not apply, but
  this endpoint now uses `stream_logs` (see below).
- **Other sites** — Vecs are needed for sorting (`calculate_reward_percentiles`,
  `suggest_tip_cap`), poll buffering (`FilterOutput`), or feed into alloy
  types we don't control.

## Channel-based cold storage streaming (adopted)

`eth_getLogs` and `eth_getFilterChanges` now use `stream_logs()`, which
returns a `ReceiverStream<ColdResult<RpcLog>>` over a bounded MPSC channel
(256 buffer). The stream is collected into a `Vec<Log>` at the handler level
because:

1. `serde::Serialize` is synchronous — there is no way to `.await` a channel
   receiver inside `serialize()`.
2. ajj buffers the entire response via `serde_json::to_raw_value` before
   sending over HTTP or WebSocket. There is no chunked or streaming response
   support.

Despite the full collection, streaming provides:

- **Deadline enforcement** — queries exceeding the configured wall-clock limit
  are terminated early, freeing cold storage resources.
- **Dedicated concurrency** — log queries use a separate semaphore (max 8
  streams), preventing starvation of other read operations.
- **Reorg detection** — anchor hash check prevents returning stale/incorrect
  data from reorganized blocks.
- **Progressive memory** — cold storage does not hold the entire result set
  simultaneously (only 256 items in the channel buffer at a time).

True async streaming to the HTTP response requires ajj to support a streaming
response API (e.g. `AsyncSerialize` or HTTP chunked encoding with per-item
flushing).
