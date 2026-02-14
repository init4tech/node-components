//! Error types for the signet namespace.

/// Errors that can occur in the `signet` namespace.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SignetError {
    /// The transaction cache was not provided.
    #[error("transaction cache not provided")]
    TxCacheNotProvided,
    /// Block resolution failed.
    #[error("block resolution error")]
    Resolve(String),
    /// EVM execution error.
    #[error("evm execution error")]
    Evm(String),
    /// Bundle simulation timed out.
    #[error("timeout during bundle simulation")]
    Timeout,
}

impl serde::Serialize for SignetError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
