//! Parameter types, macros, and utility helpers for ETH RPC endpoints.

use crate::eth::error::EthError;
use alloy::{
    consensus::{
        BlockHeader, ReceiptEnvelope, ReceiptWithBloom, Transaction, TxReceipt,
        transaction::{Recovered, SignerRecoverable},
    },
    eips::BlockId,
    primitives::{Address, TxKind, U256},
    rpc::types::{
        BlockOverrides, Log, TransactionReceipt, TransactionRequest, state::StateOverride,
    },
};
use serde::Deserialize;
use signet_cold::ReceiptContext;
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

/// An iterator that yields inclusive block ranges of a given step size.
#[derive(Debug)]
pub(crate) struct BlockRangeInclusiveIter {
    iter: std::iter::StepBy<std::ops::RangeInclusive<u64>>,
    step: u64,
    end: u64,
}

impl BlockRangeInclusiveIter {
    pub(crate) fn new(range: std::ops::RangeInclusive<u64>, step: u64) -> Self {
        Self { end: *range.end(), iter: range.step_by(step as usize + 1), step }
    }
}

impl Iterator for BlockRangeInclusiveIter {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.iter.next()?;
        let end = (start + self.step).min(self.end);
        if start > end {
            return None;
        }
        Some((start, end))
    }
}

/// Small wrapper implementing [`trevm::Cfg`] to set the chain ID.
pub(crate) struct CfgFiller(pub u64);

impl trevm::Cfg for CfgFiller {
    fn fill_cfg_env(&self, cfg: &mut trevm::revm::context::CfgEnv) {
        cfg.chain_id = self.0;
    }
}

/// Recover the sender of a transaction, falling back to [`MagicSig`].
///
/// [`MagicSig`]: signet_types::MagicSig
pub(crate) fn recover_sender(
    tx: &signet_storage_types::TransactionSigned,
) -> Result<Address, EthError> {
    signet_types::MagicSig::try_from_signature(tx.signature())
        .map(|s| s.rollup_sender())
        .or_else(|| SignerRecoverable::recover_signer_unchecked(tx).ok())
        .ok_or(EthError::InvalidSignature)
}

/// Build an [`alloy::rpc::types::Transaction`] from cold storage types.
pub(crate) fn build_rpc_transaction(
    tx: signet_storage_types::TransactionSigned,
    meta: &ConfirmationMeta,
    base_fee: Option<u64>,
) -> Result<alloy::rpc::types::Transaction, EthError> {
    let sender = recover_sender(&tx)?;

    // Convert EthereumTxEnvelope<TxEip4844> → TxEnvelope (EthereumTxEnvelope<TxEip4844Variant>)
    let tx_envelope: alloy::consensus::TxEnvelope = tx.into();
    let inner = Recovered::new_unchecked(tx_envelope, sender);

    let egp = base_fee
        .map(|bf| inner.effective_tip_per_gas(bf).unwrap_or_default() as u64 + bf)
        .unwrap_or_else(|| inner.max_fee_per_gas() as u64);

    Ok(alloy::rpc::types::Transaction {
        inner,
        block_hash: Some(meta.block_hash()),
        block_number: Some(meta.block_number()),
        transaction_index: Some(meta.transaction_index()),
        effective_gas_price: Some(egp as u128),
    })
}

/// Build a [`TransactionReceipt`] from a [`ReceiptContext`].
pub(crate) fn build_receipt(
    ctx: ReceiptContext,
) -> Result<TransactionReceipt<ReceiptEnvelope<Log>>, EthError> {
    let (receipt, meta) = ctx.receipt.into_parts();
    let gas_used = receipt.inner.cumulative_gas_used() - ctx.prior_cumulative_gas;

    build_receipt_inner(
        ctx.transaction,
        &ctx.header,
        &meta,
        receipt,
        gas_used,
        0, // log_index_offset: single receipt, no prior logs
    )
}

/// Build a [`TransactionReceipt`] from individual components.
///
/// Used by `eth_getBlockReceipts` where all receipts in the block are available.
pub(crate) fn build_receipt_from_parts(
    tx: signet_storage_types::TransactionSigned,
    header: &alloy::consensus::Header,
    block_hash: alloy::primitives::B256,
    tx_index: u64,
    receipt: signet_storage_types::Receipt,
    gas_used: u64,
    log_index_offset: u64,
) -> Result<TransactionReceipt<ReceiptEnvelope<Log>>, EthError> {
    let meta = ConfirmationMeta::new(header.number, block_hash, tx_index);
    build_receipt_inner(tx, header, &meta, receipt, gas_used, log_index_offset)
}

/// Shared receipt builder.
fn build_receipt_inner(
    tx: signet_storage_types::TransactionSigned,
    header: &alloy::consensus::Header,
    meta: &ConfirmationMeta,
    receipt: signet_storage_types::Receipt,
    gas_used: u64,
    log_index_offset: u64,
) -> Result<TransactionReceipt<ReceiptEnvelope<Log>>, EthError> {
    let sender = recover_sender(&tx)?;
    let tx_hash = *tx.tx_hash();

    let logs_bloom = receipt.inner.bloom();
    let status = receipt.inner.status_or_post_state();
    let cumulative_gas_used = receipt.inner.cumulative_gas_used();

    let logs: Vec<Log> = receipt
        .inner
        .logs
        .into_iter()
        .enumerate()
        .map(|(i, log)| Log {
            inner: log,
            block_hash: Some(meta.block_hash()),
            block_number: Some(meta.block_number()),
            block_timestamp: Some(header.timestamp),
            transaction_hash: Some(tx_hash),
            transaction_index: Some(meta.transaction_index()),
            log_index: Some(log_index_offset + i as u64),
            removed: false,
        })
        .collect();

    let rpc_receipt = alloy::rpc::types::eth::Receipt { status, cumulative_gas_used, logs };

    let (contract_address, to) = match tx.kind() {
        TxKind::Create => (Some(sender.create(tx.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    let base_fee = header.base_fee_per_gas();
    let egp = base_fee
        .map(|bf| tx.effective_tip_per_gas(bf).unwrap_or_default() as u64 + bf)
        .unwrap_or_else(|| tx.max_fee_per_gas() as u64);

    Ok(TransactionReceipt {
        inner: build_receipt_envelope(
            ReceiptWithBloom { receipt: rpc_receipt, logs_bloom },
            receipt.tx_type,
        ),
        transaction_hash: tx_hash,
        transaction_index: Some(meta.transaction_index()),
        block_hash: Some(meta.block_hash()),
        block_number: Some(meta.block_number()),
        from: sender,
        to,
        gas_used,
        contract_address,
        effective_gas_price: egp as u128,
        blob_gas_price: None,
        blob_gas_used: None,
    })
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
