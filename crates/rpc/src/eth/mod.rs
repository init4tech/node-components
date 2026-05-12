//! ETH namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    addr_tx_count, balance, block, block_number, block_receipts, block_tx_count, call, chain_id,
    code_at, create_access_list, estimate_gas, fee_history, gas_price, get_filter_changes,
    get_logs, header_by, max_priority_fee_per_gas, new_block_filter, new_filter, protocol_version,
    raw_transaction_by_block_and_index, raw_transaction_by_hash, send_raw_transaction, storage_at,
    subscribe, syncing, transaction_by_block_and_index, transaction_by_hash, transaction_receipt,
    uncle_block, uncle_count, uninstall_filter, unsubscribe,
};

pub(crate) mod error;
pub use error::EthError;

pub(crate) mod helpers;
pub(crate) mod types;

use crate::config::StorageRpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_cold::ColdStorageBackend;
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate the `eth` API router backed by storage.
pub(crate) fn eth<H, B>() -> ajj::Router<StorageRpcCtx<H, B>>
where
    H: HotKv + Send + Sync + 'static,
    B: ColdStorageBackend,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new()
        .route("blockNumber", block_number::<H, B>)
        .route("chainId", chain_id::<H, B>)
        .route("getBlockByHash", block::<B256, H, B>)
        .route("getBlockByNumber", block::<BlockNumberOrTag, H, B>)
        .route("getBlockTransactionCountByHash", block_tx_count::<B256, H, B>)
        .route("getBlockTransactionCountByNumber", block_tx_count::<BlockNumberOrTag, H, B>)
        .route("getBlockReceipts", block_receipts::<H, B>)
        .route("getRawTransactionByHash", raw_transaction_by_hash::<H, B>)
        .route("getTransactionByHash", transaction_by_hash::<H, B>)
        .route(
            "getRawTransactionByBlockHashAndIndex",
            raw_transaction_by_block_and_index::<B256, H, B>,
        )
        .route(
            "getRawTransactionByBlockNumberAndIndex",
            raw_transaction_by_block_and_index::<BlockNumberOrTag, H, B>,
        )
        .route("getTransactionByBlockHashAndIndex", transaction_by_block_and_index::<B256, H, B>)
        .route(
            "getTransactionByBlockNumberAndIndex",
            transaction_by_block_and_index::<BlockNumberOrTag, H, B>,
        )
        .route("getTransactionReceipt", transaction_receipt::<H, B>)
        .route("getBlockHeaderByHash", header_by::<B256, H, B>)
        .route("getBlockHeaderByNumber", header_by::<BlockNumberOrTag, H, B>)
        .route("getBalance", balance::<H, B>)
        .route("getStorageAt", storage_at::<H, B>)
        .route("getTransactionCount", addr_tx_count::<H, B>)
        .route("getCode", code_at::<H, B>)
        .route("call", call::<H, B>)
        .route("estimateGas", estimate_gas::<H, B>)
        .route("sendRawTransaction", send_raw_transaction::<H, B>)
        .route("getLogs", get_logs::<H, B>)
        .route("syncing", syncing::<H, B>)
        .route("gasPrice", gas_price::<H, B>)
        .route("maxPriorityFeePerGas", max_priority_fee_per_gas::<H, B>)
        .route("feeHistory", fee_history::<H, B>)
        .route("createAccessList", create_access_list::<H, B>)
        .route("newFilter", new_filter::<H, B>)
        .route("newBlockFilter", new_block_filter::<H, B>)
        .route("uninstallFilter", uninstall_filter::<H, B>)
        .route("getFilterChanges", get_filter_changes::<H, B>)
        .route("getFilterLogs", get_filter_changes::<H, B>)
        .route("subscribe", subscribe::<H, B>)
        .route("unsubscribe", unsubscribe::<H, B>)
        // Uncle queries return semantically correct values (0 / null)
        // because Signet has no uncle blocks.
        .route("getUncleCountByBlockHash", uncle_count)
        .route("getUncleCountByBlockNumber", uncle_count)
        .route("getUncleByBlockHashAndIndex", uncle_block)
        .route("getUncleByBlockNumberAndIndex", uncle_block)
        .route("protocolVersion", protocol_version)
    // Unsupported methods (return method_not_found by default):
    // - coinbase, accounts, blobBaseFee
    // - getWork, hashrate, mining, submitHashrate, submitWork
    // - sendTransaction, sign, signTransaction, signTypedData
    // - getProof, newPendingTransactionFilter
}
