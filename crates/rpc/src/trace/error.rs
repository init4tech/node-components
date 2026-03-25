//! Error types for the `trace` namespace.

use alloy::{eips::BlockId, primitives::B256};
use std::borrow::Cow;

/// Errors that can occur in the `trace` namespace.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// Cold storage error.
    #[error("cold storage error")]
    Cold(#[from] signet_cold::ColdStorageError),
    /// Hot storage error.
    #[error("hot storage error")]
    Hot(#[from] signet_storage::StorageError),
    /// Block resolution error.
    #[error("resolve: {0}")]
    Resolve(crate::config::resolve::ResolveError),
    /// EVM execution halted.
    #[error("execution halted: {reason}")]
    EvmHalt {
        /// Debug-formatted halt reason.
        reason: String,
    },
    /// Block not found.
    #[error("block not found: {0}")]
    BlockNotFound(BlockId),
    /// Transaction not found.
    #[error("transaction not found: {0}")]
    TransactionNotFound(B256),
    /// RLP decoding failed.
    #[error("RLP decode: {0}")]
    RlpDecode(String),
    /// Transaction sender recovery failed.
    #[error("sender recovery failed")]
    SenderRecovery,
    /// Block range too large for trace_filter.
    #[error("block range too large: {requested} blocks (max {max})")]
    BlockRangeExceeded {
        /// Requested range size.
        requested: u64,
        /// Maximum allowed range.
        max: u64,
    },
}

impl ajj::IntoErrorPayload for TraceError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::Cold(_) | Self::Hot(_) | Self::EvmHalt { .. } | Self::SenderRecovery => -32000,
            Self::Resolve(r) => crate::eth::error::resolve_error_code(r),
            Self::BlockNotFound(_) | Self::TransactionNotFound(_) => -32001,
            Self::RlpDecode(_) | Self::BlockRangeExceeded { .. } => -32602,
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::Cold(_) | Self::Hot(_) => "server error".into(),
            Self::Resolve(r) => crate::eth::error::resolve_error_message(r),
            Self::EvmHalt { reason } => format!("execution halted: {reason}").into(),
            Self::BlockNotFound(id) => format!("block not found: {id}").into(),
            Self::TransactionNotFound(h) => format!("transaction not found: {h}").into(),
            Self::RlpDecode(msg) => format!("RLP decode error: {msg}").into(),
            Self::SenderRecovery => "sender recovery failed".into(),
            Self::BlockRangeExceeded { requested, max } => {
                format!("block range too large: {requested} blocks (max {max})").into()
            }
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::TraceError;
    use ajj::IntoErrorPayload;
    use alloy::{eips::BlockId, primitives::B256};

    #[test]
    fn cold_error_code() {
        // Cold/Hot/EvmHalt/SenderRecovery all map to -32000
        let err = TraceError::SenderRecovery;
        assert_eq!(err.error_code(), -32000);
    }

    #[test]
    fn block_not_found_code() {
        let err = TraceError::BlockNotFound(BlockId::latest());
        assert_eq!(err.error_code(), -32001);
    }

    #[test]
    fn transaction_not_found_code() {
        let err = TraceError::TransactionNotFound(B256::ZERO);
        assert_eq!(err.error_code(), -32001);
    }

    #[test]
    fn rlp_decode_code() {
        let err = TraceError::RlpDecode("bad".into());
        assert_eq!(err.error_code(), -32602);
    }

    #[test]
    fn block_range_exceeded_code() {
        let err = TraceError::BlockRangeExceeded { requested: 200, max: 100 };
        assert_eq!(err.error_code(), -32602);
        assert!(err.error_message().contains("200"));
    }
}
