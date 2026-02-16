# signet-rpc

Ethereum JSON-RPC server backed by `signet-storage`'s unified storage backend.

This crate provides a standalone RPC implementation that uses hot storage
for state queries and cold storage for block, transaction, and receipt data.

## Namespaces

### `eth`

Standard Ethereum JSON-RPC methods:

- Block queries: `blockNumber`, `getBlockByHash`, `getBlockByNumber`,
  `getBlockTransactionCount*`, `getBlockReceipts`, `getBlockHeader*`
- Transaction queries: `getTransactionByHash`, `getTransactionReceipt`,
  `getTransactionByBlock*AndIndex`, `getRawTransaction*`
- Account state: `getBalance`, `getStorageAt`, `getCode`, `getTransactionCount`
- EVM execution: `call`, `estimateGas`, `createAccessList`
- Gas/fees: `gasPrice`, `maxPriorityFeePerGas`, `feeHistory`
- Logs & filters: `getLogs`, `newFilter`, `newBlockFilter`,
  `getFilterChanges`, `getFilterLogs`, `uninstallFilter`
- Subscriptions: `subscribe`, `unsubscribe`
- Transaction submission: `sendRawTransaction` (optional, via `TxCache`)
- Uncle queries: `getUncleCountByBlock*`, `getUncleByBlock*AndIndex`
  (always return 0 / null — Signet has no uncle blocks)
- Misc: `chainId`, `syncing`

### `debug`

- `traceBlockByNumber`, `traceBlockByHash` — trace all transactions in a block
- `traceTransaction` — trace a single transaction by hash

### `signet`

- `sendOrder` — forward a signed order to the transaction cache
- `callBundle` — simulate a bundle against a specific block

## Unsupported Methods

The following `eth` methods are **not supported** and return
`method_not_found`:

- **Mining**: `getWork`, `hashrate`, `mining`, `submitHashrate`, `submitWork`
  — Signet does not use proof-of-work.
- **Account management**: `accounts`, `sign`, `signTransaction`,
  `signTypedData`, `sendTransaction` — the RPC server does not hold keys.
  Use `sendRawTransaction` with a pre-signed transaction instead.
- **Blob transactions**: `blobBaseFee` — Signet does not support EIP-4844
  blob transactions.
- **Other**: `protocolVersion`, `getProof`, `newPendingTransactionFilter`,
  `coinbase`.
