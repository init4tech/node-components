//! Error types for the signet namespace.

/// Errors that can occur in the `signet` namespace.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum SignetError {
    /// The transaction cache was not provided.
    #[error("transaction cache not provided")]
    TxCacheNotProvided,
}

impl SignetError {
    /// Convert to a string by value.
    pub fn into_string(self) -> String {
        self.to_string()
    }
}
