# signet-rpc

Ethereum JSON-RPC server backed by `signet-storage`'s unified storage backend.

This crate provides a standalone ETH RPC implementation that uses hot storage
for state queries and cold storage for block, transaction, and receipt data.

## Supported Methods

- Block queries: `eth_blockNumber`, `eth_getBlockByHash`, `eth_getBlockByNumber`, etc.
- Transaction queries: `eth_getTransactionByHash`, `eth_getTransactionReceipt`, etc.
- Account state: `eth_getBalance`, `eth_getStorageAt`, `eth_getCode`, `eth_getTransactionCount`
- EVM execution: `eth_call`, `eth_estimateGas`
- Logs: `eth_getLogs`
- Transaction submission: `eth_sendRawTransaction` (optional, via `TxCache`)
