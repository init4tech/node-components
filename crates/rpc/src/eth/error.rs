//! Error types for the storage-backed ETH RPC.

use ajj::IntoErrorPayload;
use alloy::{
    eips::BlockId,
    primitives::{B256, Bytes},
};
use std::borrow::Cow;

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
    /// Block not found.
    #[error("block not found: {0}")]
    BlockNotFound(BlockId),
    /// Receipt found but the corresponding transaction is missing.
    #[error("transaction not found: {0}")]
    TransactionMissing(B256),
    /// EVM execution reverted with output data.
    #[error("execution reverted")]
    EvmRevert {
        /// ABI-encoded revert data.
        output: Bytes,
    },
    /// EVM execution halted with a non-revert reason.
    #[error("execution halted: {reason}")]
    EvmHalt {
        /// Human-readable halt reason.
        reason: String,
    },
    /// Invalid RPC parameters.
    #[error("{0}")]
    InvalidParams(String),
    /// Internal server error.
    #[error("{0}")]
    Internal(String),
}

impl EthError {
    /// Construct the standard error for a spawned task that panicked or
    /// was cancelled before producing a result.
    pub(crate) fn task_panic() -> Self {
        Self::Internal("task panicked or cancelled".into())
    }
}

/// Returns the JSON-RPC error code for a [`ResolveError`].
///
/// [`ResolveError`]: crate::config::resolve::ResolveError
pub(crate) const fn resolve_error_code(e: &crate::config::resolve::ResolveError) -> i64 {
    use crate::config::resolve::ResolveError;
    match e {
        ResolveError::HashNotFound(_) => -32001,
        ResolveError::Storage(_) | ResolveError::Db(_) => -32000,
    }
}

/// Returns the JSON-RPC error message for a [`ResolveError`].
///
/// [`ResolveError`]: crate::config::resolve::ResolveError
pub(crate) fn resolve_error_message(e: &crate::config::resolve::ResolveError) -> Cow<'static, str> {
    use crate::config::resolve::ResolveError;
    match e {
        ResolveError::HashNotFound(hash) => Cow::Owned(format!("block hash not found: {hash}")),
        ResolveError::Storage(_) | ResolveError::Db(_) => Cow::Borrowed("server error"),
    }
}

impl IntoErrorPayload for EthError {
    type ErrData = Bytes;

    fn error_code(&self) -> i64 {
        match self {
            Self::Cold(..) | Self::Hot(..) => -32000,
            Self::Resolve(r) => resolve_error_code(r),
            Self::BlockNotFound(_) | Self::TransactionMissing(_) => -32001,
            Self::EvmRevert { .. } => 3,
            Self::EvmHalt { .. } => -32000,
            Self::InvalidParams(_) => -32602,
            Self::Internal(_) => -32000,
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::Cold(..) | Self::Hot(..) => Cow::Borrowed("server error"),
            Self::Resolve(r) => resolve_error_message(r),
            Self::BlockNotFound(id) => Cow::Owned(format!("block not found: {id}")),
            Self::TransactionMissing(h) => Cow::Owned(format!("transaction not found: {h}")),
            Self::EvmRevert { .. } => Cow::Borrowed("execution reverted"),
            Self::EvmHalt { reason } => Cow::Owned(format!("execution halted: {reason}")),
            Self::InvalidParams(msg) => Cow::Owned(msg.clone()),
            Self::Internal(msg) => Cow::Owned(msg.clone()),
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        match self {
            Self::EvmRevert { output } => Some(output),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve::ResolveError;
    use alloy::primitives::B256;

    #[test]
    fn cold_storage_error() {
        let err = EthError::Cold(signet_cold::ColdStorageError::NotFound("test".into()));
        assert_eq!(err.error_code(), -32000);
        assert_eq!(err.error_message(), "server error");
        assert_eq!(err.error_data(), None);
    }

    #[test]
    fn resolve_hash_not_found() {
        let err = EthError::Resolve(ResolveError::HashNotFound(B256::ZERO));
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("block hash not found"));
    }

    #[test]
    fn block_not_found() {
        let err = EthError::BlockNotFound(BlockId::latest());
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("block not found"));
    }

    #[test]
    fn transaction_missing() {
        let err = EthError::TransactionMissing(B256::ZERO);
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("transaction not found"));
    }

    #[test]
    fn evm_revert() {
        let output = Bytes::from(vec![0xde, 0xad]);
        let err = EthError::EvmRevert { output: output.clone() };
        assert_eq!(err.error_code(), 3);
        assert_eq!(err.error_message(), "execution reverted");
        assert_eq!(err.error_data(), Some(output));
    }

    #[test]
    fn evm_halt() {
        let err = EthError::EvmHalt { reason: "OutOfGas".into() };
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("OutOfGas"));
        assert_eq!(err.error_data(), None);
    }

    #[test]
    fn invalid_params() {
        let err = EthError::InvalidParams("bad param".into());
        assert_eq!(err.error_code(), -32602);
        assert_eq!(err.error_message(), "bad param");
    }

    #[test]
    fn internal() {
        let err = EthError::Internal("something broke".into());
        assert_eq!(err.error_code(), -32000);
        assert_eq!(err.error_message(), "something broke");
    }

    #[test]
    fn task_panic_constructor() {
        let err = EthError::task_panic();
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("task panicked"));
    }
}
