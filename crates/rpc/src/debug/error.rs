//! Error types for the debug namespace.

use alloy::eips::BlockId;

/// Errors that can occur in the `debug` namespace.
///
/// The [`serde::Serialize`] impl emits sanitized messages suitable for
/// API responses — internal storage details are not exposed to callers.
/// Use [`tracing`] to log the full error chain before constructing the
/// variant.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DebugError {
    /// Cold storage error.
    #[error("cold storage error")]
    Cold(String),
    /// Hot storage error.
    #[error("hot storage error")]
    Hot(String),
    /// Invalid tracer configuration.
    #[error("invalid tracer config")]
    InvalidTracerConfig,
    /// Unsupported tracer type.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// EVM execution error.
    #[error("evm execution error")]
    Evm(String),
    /// Block not found.
    #[error("block not found: {0}")]
    BlockNotFound(BlockId),
    /// Transaction not found.
    #[error("transaction not found")]
    TransactionNotFound,
}

impl DebugError {
    /// Convert to a string by value.
    pub fn into_string(self) -> String {
        self.to_string()
    }
}

impl serde::Serialize for DebugError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
