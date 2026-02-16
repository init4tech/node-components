//! ETH namespace RPC router backed by storage.

mod endpoints;
use endpoints::{
    addr_tx_count, balance, block, block_number, block_receipts, block_tx_count, call, chain_id,
    code_at, create_access_list, estimate_gas, fee_history, gas_price, get_filter_changes,
    get_logs, header_by, max_priority_fee_per_gas, new_block_filter, new_filter,
    raw_transaction_by_block_and_index, raw_transaction_by_hash, send_raw_transaction, storage_at,
    subscribe, syncing, transaction_by_block_and_index, transaction_by_hash, transaction_receipt,
    uncle_block, uncle_count, uninstall_filter, unsubscribe,
};

mod error;
pub use error::EthError;

pub(crate) mod helpers;
pub(crate) mod types;

use crate::config::StorageRpcCtx;
use alloy::{eips::BlockNumberOrTag, primitives::B256};
use signet_hot::{HotKv, model::HotKvRead};
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
        .route(
            "getRawTransactionByBlockHashAndIndex",
            raw_transaction_by_block_and_index::<B256, H>,
        )
        .route(
            "getRawTransactionByBlockNumberAndIndex",
            raw_transaction_by_block_and_index::<BlockNumberOrTag, H>,
        )
        .route("getTransactionByBlockHashAndIndex", transaction_by_block_and_index::<B256, H>)
        .route(
            "getTransactionByBlockNumberAndIndex",
            transaction_by_block_and_index::<BlockNumberOrTag, H>,
        )
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
        .route("syncing", syncing::<H>)
        .route("gasPrice", gas_price::<H>)
        .route("maxPriorityFeePerGas", max_priority_fee_per_gas::<H>)
        .route("feeHistory", fee_history::<H>)
        .route("createAccessList", create_access_list::<H>)
        .route("newFilter", new_filter::<H>)
        .route("newBlockFilter", new_block_filter::<H>)
        .route("uninstallFilter", uninstall_filter::<H>)
        .route("getFilterChanges", get_filter_changes::<H>)
        .route("getFilterLogs", get_filter_changes::<H>)
        .route("subscribe", subscribe::<H>)
        .route("unsubscribe", unsubscribe::<H>)
        // Uncle queries return semantically correct values (0 / null)
        // because Signet has no uncle blocks.
        .route("getUncleCountByBlockHash", uncle_count)
        .route("getUncleCountByBlockNumber", uncle_count)
        .route("getUncleByBlockHashAndIndex", uncle_block)
        .route("getUncleByBlockNumberAndIndex", uncle_block)
    // Unsupported methods (return method_not_found by default):
    // - protocolVersion, coinbase, accounts, blobBaseFee
    // - getWork, hashrate, mining, submitHashrate, submitWork
    // - sendTransaction, sign, signTransaction, signTypedData
    // - getProof, newPendingTransactionFilter
}
