//! Error types for the signet namespace.

use crate::config::resolve::ResolveError;
use std::borrow::Cow;

/// Errors that can occur in the `signet` namespace.
#[derive(Debug, thiserror::Error)]
pub enum SignetError {
    /// The transaction cache was not provided.
    #[error("transaction cache not provided")]
    TxCacheNotProvided,
    /// Block resolution failed.
    #[error("resolve: {0}")]
    Resolve(#[from] ResolveError),
    /// EVM execution halted for a non-revert reason.
    #[error("execution halted: {reason}")]
    EvmHalt {
        /// The halt reason.
        reason: String,
    },
    /// Bundle simulation timed out.
    #[error("timeout during bundle simulation")]
    Timeout,
    /// Internal server error.
    #[error("{0}")]
    Internal(String),
}

impl From<crate::eth::EthError> for SignetError {
    fn from(e: crate::eth::EthError) -> Self {
        match e {
            crate::eth::EthError::Resolve(r) => Self::Resolve(r),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl ajj::IntoErrorPayload for SignetError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::TxCacheNotProvided | Self::EvmHalt { .. } | Self::Timeout | Self::Internal(_) => {
                -32000
            }
            Self::Resolve(r) => crate::eth::error::resolve_error_code(r),
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::TxCacheNotProvided => "transaction cache not provided".into(),
            Self::Resolve(r) => crate::eth::error::resolve_error_message(r),
            Self::EvmHalt { reason } => format!("execution halted: {reason}").into(),
            Self::Timeout => "timeout during bundle simulation".into(),
            Self::Internal(msg) => Cow::Owned(msg.clone()),
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
    use alloy::primitives::B256;

    #[test]
    fn tx_cache_not_provided() {
        let err = SignetError::TxCacheNotProvided;
        assert_eq!(err.error_code(), -32000);
        assert_eq!(err.error_message(), "transaction cache not provided");
    }

    #[test]
    fn resolve_hash_not_found() {
        let err = SignetError::Resolve(ResolveError::HashNotFound(B256::ZERO));
        assert_eq!(err.error_code(), -32001);
        assert!(err.error_message().contains("block hash not found"));
    }

    #[test]
    fn evm_halt() {
        let err = SignetError::EvmHalt { reason: "OutOfGas".into() };
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("OutOfGas"));
    }

    #[test]
    fn timeout() {
        let err = SignetError::Timeout;
        assert_eq!(err.error_code(), -32000);
        assert_eq!(err.error_message(), "timeout during bundle simulation");
    }

    #[test]
    fn internal() {
        let err = SignetError::Internal("task panicked or cancelled".into());
        assert_eq!(err.error_code(), -32000);
        assert!(err.error_message().contains("task panicked"));
    }

    #[test]
    fn from_eth_error_resolve() {
        let eth = crate::eth::EthError::Resolve(ResolveError::HashNotFound(B256::ZERO));
        let signet = SignetError::from(eth);
        assert_eq!(signet.error_code(), -32001);
    }

    #[test]
    fn from_eth_error_other() {
        let eth = crate::eth::EthError::Internal("boom".into());
        let signet = SignetError::from(eth);
        assert_eq!(signet.error_code(), -32000);
        assert!(signet.error_message().contains("boom"));
    }
}
