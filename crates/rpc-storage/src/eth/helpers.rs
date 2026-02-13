//! Parameter types, macros, and utility helpers for ETH RPC endpoints.

use alloy::{
    consensus::{
        ReceiptEnvelope, ReceiptWithBloom, Transaction, TxReceipt, transaction::Recovered,
    },
    eips::BlockId,
    primitives::{Address, TxKind, U256},
    rpc::types::{
        BlockOverrides, Log, TransactionReceipt, TransactionRequest, state::StateOverride,
    },
};
use serde::Deserialize;
use signet_cold::ColdReceipt;
use signet_storage_types::ConfirmationMeta;
use trevm::MIN_TRANSACTION_GAS;

/// Args for `eth_call` and `eth_estimateGas`.
#[derive(Debug, Deserialize)]
pub(crate) struct TxParams(
    pub TransactionRequest,
    #[serde(default)] pub Option<BlockId>,
    #[serde(default)] pub Option<StateOverride>,
    #[serde(default)] pub Option<Box<BlockOverrides>>,
);

/// Args for `eth_getBlockByHash` and `eth_getBlockByNumber`.
#[derive(Debug, Deserialize)]
pub(crate) struct BlockParams<T>(pub T, #[serde(default)] pub Option<bool>);

/// Args for `eth_getStorageAt`.
#[derive(Debug, Deserialize)]
pub(crate) struct StorageAtArgs(pub Address, pub U256, #[serde(default)] pub Option<BlockId>);

/// Args for `eth_getBalance`, `eth_getTransactionCount`, and `eth_getCode`.
#[derive(Debug, Deserialize)]
pub(crate) struct AddrWithBlock(pub Address, #[serde(default)] pub Option<BlockId>);

/// Args for `eth_feeHistory`.
#[derive(Debug, Deserialize)]
pub(crate) struct FeeHistoryArgs(
    pub alloy::primitives::U64,
    pub alloy::eips::BlockNumberOrTag,
    #[serde(default)] pub Option<Vec<f64>>,
);

/// Args for `eth_subscribe`.
#[derive(Debug, Deserialize)]
pub(crate) struct SubscribeArgs(
    pub alloy::rpc::types::pubsub::SubscriptionKind,
    #[serde(default)] pub Option<Box<alloy::rpc::types::Filter>>,
);

/// Normalize transaction request gas without making DB reads.
///
/// - If the gas is below `MIN_TRANSACTION_GAS`, set it to `None`
/// - If the gas is above the `rpc_gas_cap`, set it to the `rpc_gas_cap`
pub(crate) const fn normalize_gas_stateless(request: &mut TransactionRequest, max_gas: u64) {
    match request.gas {
        Some(..MIN_TRANSACTION_GAS) => request.gas = None,
        Some(val) if val > max_gas => request.gas = Some(max_gas),
        _ => {}
    }
}

/// Await a handler task, returning an error string on panic/cancel.
macro_rules! await_handler {
    ($h:expr) => {
        match $h.await {
            Ok(res) => res,
            Err(_) => return Err("task panicked or cancelled".to_string()),
        }
    };

    (@option $h:expr) => {
        match $h.await {
            Ok(Some(res)) => res,
            _ => return Err("task panicked or cancelled".to_string()),
        }
    };

    (@response_option $h:expr) => {
        match $h.await {
            Ok(Some(res)) => res,
            _ => {
                return ajj::ResponsePayload::internal_error_message(std::borrow::Cow::Borrowed(
                    "task panicked or cancelled",
                ))
            }
        }
    };
}
pub(crate) use await_handler;

/// Try-operator for `ResponsePayload`.
macro_rules! response_tri {
    ($h:expr) => {
        match $h {
            Ok(res) => res,
            Err(err) => return ajj::ResponsePayload::internal_error_message(err.to_string().into()),
        }
    };
}
pub(crate) use response_tri;

/// Small wrapper implementing [`trevm::Cfg`] to set the chain ID.
pub(crate) struct CfgFiller(pub u64);

impl trevm::Cfg for CfgFiller {
    fn fill_cfg_env(&self, cfg: &mut trevm::revm::context::CfgEnv) {
        cfg.chain_id = self.0;
    }
}

/// Build an [`alloy::rpc::types::Transaction`] from cold storage types.
pub(crate) fn build_rpc_transaction(
    tx: signet_storage_types::RecoveredTx,
    meta: &ConfirmationMeta,
    base_fee: Option<u64>,
) -> alloy::rpc::types::Transaction {
    let signer = tx.signer();
    let tx_envelope: alloy::consensus::TxEnvelope = tx.into_inner().into();
    let inner = Recovered::new_unchecked(tx_envelope, signer);

    let egp = base_fee
        .map(|bf| inner.effective_tip_per_gas(bf).unwrap_or_default() as u64 + bf)
        .unwrap_or_else(|| inner.max_fee_per_gas() as u64);

    alloy::rpc::types::Transaction {
        inner,
        block_hash: Some(meta.block_hash()),
        block_number: Some(meta.block_number()),
        transaction_index: Some(meta.transaction_index()),
        effective_gas_price: Some(egp as u128),
    }
}

/// Build a [`TransactionReceipt`] from a [`ColdReceipt`] and its transaction.
///
/// The transaction is needed for `to`, `contract_address`, and
/// `effective_gas_price` which are not stored on the receipt.
pub(crate) fn build_receipt(
    cr: ColdReceipt,
    tx: &signet_storage_types::RecoveredTx,
    base_fee: Option<u64>,
) -> TransactionReceipt<ReceiptEnvelope<Log>> {
    let logs_bloom = cr.receipt.bloom();
    let status = cr.receipt.status;
    let cumulative_gas_used = cr.receipt.cumulative_gas_used;

    let rpc_receipt =
        alloy::rpc::types::eth::Receipt { status, cumulative_gas_used, logs: cr.receipt.logs };

    let (contract_address, to) = match tx.kind() {
        TxKind::Create => (Some(cr.from.create(tx.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    let egp = base_fee
        .map(|bf| tx.effective_tip_per_gas(bf).unwrap_or_default() as u64 + bf)
        .unwrap_or_else(|| tx.max_fee_per_gas() as u64);

    TransactionReceipt {
        inner: build_receipt_envelope(
            ReceiptWithBloom { receipt: rpc_receipt, logs_bloom },
            cr.tx_type,
        ),
        transaction_hash: cr.tx_hash,
        transaction_index: Some(cr.transaction_index),
        block_hash: Some(cr.block_hash),
        block_number: Some(cr.block_number),
        from: cr.from,
        to,
        gas_used: cr.gas_used,
        contract_address,
        effective_gas_price: egp as u128,
        blob_gas_price: None,
        blob_gas_used: None,
    }
}

/// Wrap a receipt in the appropriate [`ReceiptEnvelope`] variant.
const fn build_receipt_envelope(
    receipt: ReceiptWithBloom<alloy::consensus::Receipt<Log>>,
    tx_type: alloy::consensus::TxType,
) -> ReceiptEnvelope<Log> {
    match tx_type {
        alloy::consensus::TxType::Legacy => ReceiptEnvelope::Legacy(receipt),
        alloy::consensus::TxType::Eip2930 => ReceiptEnvelope::Eip2930(receipt),
        alloy::consensus::TxType::Eip1559 => ReceiptEnvelope::Eip1559(receipt),
        alloy::consensus::TxType::Eip4844 => ReceiptEnvelope::Eip4844(receipt),
        alloy::consensus::TxType::Eip7702 => ReceiptEnvelope::Eip7702(receipt),
    }
}
