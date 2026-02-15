//! Response and serialization types for ETH RPC endpoints.

use super::helpers::{build_receipt, build_rpc_transaction};
use alloy::{
    network::{Ethereum, Network},
    primitives::B256,
};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use signet_cold::ColdReceipt;

/// RPC header type for the Ethereum network.
pub(crate) type RpcHeader = <Ethereum as Network>::HeaderResponse;
/// RPC transaction type for the Ethereum network.
pub(crate) type RpcTransaction = <Ethereum as Network>::TransactionResponse;
/// RPC receipt type for the Ethereum network.
pub(crate) type RpcReceipt = <Ethereum as Network>::ReceiptResponse;

/// Serializes as an empty JSON array `[]` without allocating.
pub(crate) struct EmptyArray;

impl Serialize for EmptyArray {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_seq(Some(0))?.end()
    }
}

/// Block transactions with lazy serialization.
///
/// In both variants the raw `RecoveredTx` list is kept and transformed
/// during serialization — either to full RPC transaction objects or to bare
/// hashes — avoiding an intermediate `Vec` allocation.
pub(crate) enum BlockTransactions {
    Full {
        txs: Vec<signet_storage_types::RecoveredTx>,
        block_num: u64,
        block_hash: B256,
        base_fee: Option<u64>,
    },
    Hashes(Vec<signet_storage_types::RecoveredTx>),
}

impl Serialize for BlockTransactions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Full { txs, block_num, block_hash, base_fee } => {
                let mut seq = serializer.serialize_seq(Some(txs.len()))?;
                for (i, tx) in txs.iter().enumerate() {
                    let meta = signet_storage_types::ConfirmationMeta::new(
                        *block_num,
                        *block_hash,
                        i as u64,
                    );
                    seq.serialize_element(&build_rpc_transaction(tx, &meta, *base_fee))?;
                }
                seq.end()
            }
            Self::Hashes(txs) => {
                let mut seq = serializer.serialize_seq(Some(txs.len()))?;
                for tx in txs {
                    seq.serialize_element(tx.tx_hash())?;
                }
                seq.end()
            }
        }
    }
}

/// RPC block response with lazy transaction serialization.
///
/// Replaces the alloy `Block` type so that transactions are serialized
/// inline from raw storage data. Signet has no uncles or withdrawals, so
/// those are hardcoded as empty/absent to avoid allocations.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RpcBlock {
    #[serde(flatten)]
    pub(crate) header: alloy::rpc::types::Header,
    pub(crate) transactions: BlockTransactions,
    pub(crate) uncles: EmptyArray,
}

/// Lazily serialized receipt list. Each receipt is built and serialized
/// inline without allocating an intermediate `Vec<RpcReceipt>`.
pub(crate) struct LazyReceipts {
    pub(crate) txs: Vec<signet_storage_types::RecoveredTx>,
    pub(crate) receipts: Vec<ColdReceipt>,
    pub(crate) base_fee: Option<u64>,
}

impl Serialize for LazyReceipts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.txs.len()))?;
        for (tx, cr) in self.txs.iter().zip(&self.receipts) {
            seq.serialize_element(&build_receipt(cr, tx, self.base_fee))?;
        }
        seq.end()
    }
}
