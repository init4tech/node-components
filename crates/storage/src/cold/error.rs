//! Error types for cold storage operations.

/// Result type alias for cold storage operations.
pub type ColdResult<T, E = ColdStorageError> = Result<T, E>;

/// Error type for cold storage operations.
#[derive(Debug, thiserror::Error)]
pub enum ColdStorageError {
    /// An error occurred in the storage backend.
    #[error("Backend error: {0}")]
    Backend(#[from] Box<dyn core::error::Error + Send + Sync + 'static>),

    /// The requested resource was not found.
    #[error("Not found: {0}")]
    NotFound(String),

    /// The storage task was cancelled.
    #[error("Task cancelled")]
    Cancelled,
}

impl ColdStorageError {
    /// Create a new backend error from any error type.
    pub fn backend<E>(error: E) -> Self
    where
        E: core::error::Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(error))
    }
}
