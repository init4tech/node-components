# signet-rpc

HTTP, WebSocket, and IPC serving layer for a Signet node. Mounts the JSON-RPC
namespaces backed by `signet-storage` plus the journal streaming endpoint.

## Transports

`ServeConfig` binds three optional transports:

- **HTTP** — JSON-RPC at `/`, plus the `/journal` WebSocket and
  `/healthcheck` when a `signet_journal_chain::JournalChainHandle` is passed
  to `serve`.
- **WebSocket** — JSON-RPC at `/rpc` (used by `eth_subscribe`), plus the
  `/journal` WebSocket and `/healthcheck` when a `JournalChainHandle` is
  passed to `serve`.
- **IPC** — JSON-RPC over local socket.

CORS, bind addresses, and the IPC socket path are all configured via
`ServeConfig` / `ServeConfigEnv`.

## JSON-RPC Namespaces

- **`eth`** — block / transaction / receipt / state queries, `call`,
  `estimateGas`, `createAccessList`, fee history, logs, filters,
  `subscribe`/`unsubscribe`, `sendRawTransaction` (optional, via `TxCache`),
  `chainId`, `syncing`. Uncle methods return 0 / null.
- **`debug`** — `traceBlockByNumber`, `traceBlockByHash`,
  `traceTransaction`.
- **`trace`** — parity-style block and transaction traces.
- **`signet`** — `sendOrder`, `callBundle`.
- **`web3`**, **`net`** — `clientVersion`, `sha3`, `version`, `listening`,
  `peerCount`.

## Streaming

- `GET /journal?from_height=N` — binary WebSocket. Streams encoded journals
  from the given height (catch-up via ring buffer) then live.
- `GET /healthcheck` — `200` once the journal chain has a tip, `503`
  otherwise.

Both routes are only mounted when a `JournalChainHandle` is supplied, and
are exposed on every enabled HTTP-shaped transport (HTTP and WS) so an
operator running only one of the two still gets the journal endpoints.

## Unsupported `eth` Methods

Return `method_not_found`:

- **Mining**: `getWork`, `hashrate`, `mining`, `submitHashrate`,
  `submitWork` — no PoW.
- **Account management**: `accounts`, `sign`, `signTransaction`,
  `signTypedData`, `sendTransaction` — the server holds no keys; use
  `sendRawTransaction`.
- **Blob**: `blobBaseFee` — no EIP-4844.
- **Other**: `protocolVersion`, `getProof`, `newPendingTransactionFilter`,
  `coinbase`.
