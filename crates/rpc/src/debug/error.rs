//! Error types for the debug namespace.

use alloy::{eips::BlockId, primitives::B256};
use std::borrow::Cow;

/// Errors that can occur in the `debug` namespace.
#[derive(Debug, thiserror::Error)]
pub enum DebugError {
    /// Cold storage error.
    #[error("cold storage error")]
    Cold(#[from] signet_cold::ColdStorageError),
    /// Hot storage error.
    #[error("hot storage error")]
    Hot(#[from] signet_storage::StorageError),
    /// Block resolution error.
    #[error("resolve: {0}")]
    Resolve(crate::config::resolve::ResolveError),
    /// Invalid tracer configuration.
    #[error("invalid tracer config")]
    InvalidTracerConfig,
    /// Unsupported tracer type.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
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
    /// Internal server error.
    #[error("{0}")]
    Internal(String),
    /// RLP decoding failed (malformed input).
    #[error("RLP decode: {0}")]
    RlpDecode(String),
    /// Transaction sender recovery failed.
    #[error("sender recovery failed")]
    SenderRecovery,
}

impl ajj::IntoErrorPayload for DebugError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::Cold(_) | Self::Hot(_) | Self::EvmHalt { .. } | Self::Internal(_) => -32000,
            Self::Resolve(r) => crate::eth::error::resolve_error_code(r),
            Self::InvalidTracerConfig | Self::RlpDecode(_) => -32602,
            Self::Unsupported(_) => -32601,
            Self::BlockNotFound(_) | Self::TransactionNotFound(_) => -32001,
            Self::SenderRecovery => -32000,
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::Cold(_) | Self::Hot(_) => "server error".into(),
            Self::Internal(msg) => Cow::Owned(msg.clone()),
            Self::Resolve(r) => crate::eth::error::resolve_error_message(r),
            Self::InvalidTracerConfig => "invalid tracer config".into(),
            Self::Unsupported(msg) => format!("unsupported: {msg}").into(),
            Self::EvmHalt { reason } => format!("execution halted: {reason}").into(),
            Self::BlockNotFound(id) => format!("block not found: {id}").into(),
            Self::TransactionNotFound(h) => format!("transaction not found: {h}").into(),
            Self::RlpDecode(msg) => format!("RLP decode error: {msg}").into(),
            Self::SenderRecovery => "sender recovery failed".into(),
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve::ResolveError;
    use ajj::IntoErrorPayload;

    #[test]
    fn cold_storage_error() {
        let err = DebugError::Cold(signet_cold::ColdStorageError::NotFound("test".into()));
        assert_eq!(err.error_code(), -32000);
        assert_eq!(err.error_message(), "server error");
    }

    #[test]
    fn resolve_hash_not_found() {
        let err = DebugError::Resolve(ResolveError::HashNotFound(B256::ZERO));
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("block hash not found"));
    }

    #[test]
    fn invalid_tracer_config() {
        let err = DebugError::InvalidTracerConfig;
        assert_eq!(err.error_code(), -32602);
        assert_eq!(err.error_message(), "invalid tracer config");
    }

    #[test]
    fn unsupported() {
        let err = DebugError::Unsupported("flatCallTracer");
        assert_eq!(err.error_code(), -32601);
        assert!(err.error_message().contains("flatCallTracer"));
    }

    #[test]
    fn evm_halt() {
        let err = DebugError::EvmHalt { reason: "OutOfGas".into() };
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("OutOfGas"));
    }

    #[test]
    fn block_not_found() {
        let err = DebugError::BlockNotFound(BlockId::latest());
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("block not found"));
    }

    #[test]
    fn transaction_not_found() {
        let err = DebugError::TransactionNotFound(B256::ZERO);
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("transaction not found"));
    }

    #[test]
    fn internal() {
        let err = DebugError::Internal("task panicked or cancelled".into());
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("task panicked"));
    }

    #[test]
    fn rlp_decode_error_code() {
        let err = DebugError::RlpDecode("invalid block RLP".into());
        assert_eq!(err.error_code(), -32602);
        assert!(err.error_message().contains("RLP"));
    }

    #[test]
    fn sender_recovery_error_code() {
        let err = DebugError::SenderRecovery;
        assert_eq!(err.error_code(), -32000);
    }
}
