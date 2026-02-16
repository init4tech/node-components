//! Error types for the storage-backed ETH RPC.

use alloy::{eips::BlockId, primitives::Bytes};
use serde::Serialize;

/// Errors from the storage-backed ETH RPC.
#[derive(Debug, thiserror::Error)]
pub enum EthError {
    /// Cold storage error.
    #[error("cold storage: {0}")]
    Cold(#[from] signet_cold::ColdStorageError),
    /// Hot storage error.
    #[error("hot storage: {0}")]
    Hot(#[from] signet_storage::StorageError),
    /// Block resolution error.
    #[error("resolve: {0}")]
    Resolve(#[from] crate::config::resolve::ResolveError),
    /// Invalid transaction signature.
    #[error("invalid transaction signature")]
    InvalidSignature,
    /// Block not found.
    #[error("block not found: {0}")]
    BlockNotFound(BlockId),
    /// Receipt found but the corresponding transaction is missing.
    #[error("receipt found but transaction missing")]
    TransactionMissing,
    /// EVM execution error.
    #[error("evm: {0}")]
    Evm(String),
}

impl EthError {
    /// Convert the error to a string for JSON-RPC responses.
    pub fn into_string(self) -> String {
        self.to_string()
    }
}

/// Error data for `eth_call` and `eth_estimateGas` responses.
///
/// Serialized as JSON in the error response `data` field.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum CallErrorData {
    /// Revert data bytes.
    Bytes(Bytes),
    /// Error message string.
    String(String),
}

impl From<Bytes> for CallErrorData {
    fn from(b: Bytes) -> Self {
        Self::Bytes(b)
    }
}

impl From<String> for CallErrorData {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}
