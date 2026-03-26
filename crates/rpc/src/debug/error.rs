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
}

impl ajj::IntoErrorPayload for DebugError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::Cold(_) | Self::Hot(_) | Self::EvmHalt { .. } => -32000,
            Self::Resolve(r) => crate::eth::error::resolve_error_code(r),
            Self::InvalidTracerConfig => -32602,
            Self::Unsupported(_) => -32601,
            Self::BlockNotFound(_) | Self::TransactionNotFound(_) => -32001,
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::Cold(_) | Self::Hot(_) => "server error".into(),
            Self::Resolve(r) => crate::eth::error::resolve_error_message(r),
            Self::InvalidTracerConfig => "invalid tracer config".into(),
            Self::Unsupported(msg) => format!("unsupported: {msg}").into(),
            Self::EvmHalt { reason } => format!("execution halted: {reason}").into(),
            Self::BlockNotFound(id) => format!("block not found: {id}").into(),
            Self::TransactionNotFound(h) => format!("transaction not found: {h}").into(),
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        None
    }
}
