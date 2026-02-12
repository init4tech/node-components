//! ETH namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    addr_tx_count, balance, block, block_number, block_receipts, block_tx_count, call, chain_id,
    code_at, estimate_gas, get_logs, header_by, not_supported, raw_transaction_by_hash,
    raw_tx_by_block_and_index, send_raw_transaction, storage_at, transaction_by_hash,
    transaction_receipt, tx_by_block_and_index,
};

mod error;
pub use error::EthError;

mod helpers;

use crate::StorageRpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_hot::HotKv;
use signet_hot::model::HotKvRead;
use trevm::revm::database::DBErrorMarker;

/// Instantiate the `eth` API router backed by storage.
pub(crate) fn eth<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("blockNumber", block_number::<H>)
        .route("chainId", chain_id::<H>)
        .route("getBlockByHash", block::<B256, H>)
        .route("getBlockByNumber", block::<BlockNumberOrTag, H>)
        .route("getBlockTransactionCountByHash", block_tx_count::<B256, H>)
        .route("getBlockTransactionCountByNumber", block_tx_count::<BlockNumberOrTag, H>)
        .route("getBlockReceipts", block_receipts::<H>)
        .route("getRawTransactionByHash", raw_transaction_by_hash::<H>)
        .route("getTransactionByHash", transaction_by_hash::<H>)
        .route("getRawTransactionByBlockHashAndIndex", raw_tx_by_block_and_index::<B256, H>)
        .route(
            "getRawTransactionByBlockNumberAndIndex",
            raw_tx_by_block_and_index::<BlockNumberOrTag, H>,
        )
        .route("getTransactionByBlockHashAndIndex", tx_by_block_and_index::<B256, H>)
        .route("getTransactionByBlockNumberAndIndex", tx_by_block_and_index::<BlockNumberOrTag, H>)
        .route("getTransactionReceipt", transaction_receipt::<H>)
        .route("getBlockHeaderByHash", header_by::<B256, H>)
        .route("getBlockHeaderByNumber", header_by::<BlockNumberOrTag, H>)
        .route("getBalance", balance::<H>)
        .route("getStorageAt", storage_at::<H>)
        .route("getTransactionCount", addr_tx_count::<H>)
        .route("getCode", code_at::<H>)
        .route("call", call::<H>)
        .route("estimateGas", estimate_gas::<H>)
        .route("sendRawTransaction", send_raw_transaction::<H>)
        .route("getLogs", get_logs::<H>)
        // ---
        // Unsupported methods
        // ---
        .route("protocolVersion", not_supported)
        .route("syncing", not_supported)
        .route("gasPrice", not_supported)
        .route("maxPriorityFeePerGas", not_supported)
        .route("feeHistory", not_supported)
        .route("coinbase", not_supported)
        .route("accounts", not_supported)
        .route("blobBaseFee", not_supported)
        .route("getUncleCountByBlockHash", not_supported)
        .route("getUncleCountByBlockNumber", not_supported)
        .route("getUncleByBlockHashAndIndex", not_supported)
        .route("getUncleByBlockNumberAndIndex", not_supported)
        .route("getWork", not_supported)
        .route("hashrate", not_supported)
        .route("mining", not_supported)
        .route("submitHashrate", not_supported)
        .route("submitWork", not_supported)
        .route("sendTransaction", not_supported)
        .route("sign", not_supported)
        .route("signTransaction", not_supported)
        .route("signTypedData", not_supported)
        .route("getProof", not_supported)
        .route("createAccessList", not_supported)
        .route("newFilter", not_supported)
        .route("newBlockFilter", not_supported)
        .route("newPendingTransactionFilter", not_supported)
        .route("uninstallFilter", not_supported)
        .route("getFilterChanges", not_supported)
        .route("getFilterLogs", not_supported)
        .route("subscribe", not_supported)
        .route("unsubscribe", not_supported)
}
