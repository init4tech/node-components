use crate::ser::DeserError;

/// Trait for hot storage read/write errors.
#[derive(thiserror::Error, Debug)]
pub enum HotKvError {
    /// Boxed error. Indicates an issue with the DB backend.
    #[error(transparent)]
    Inner(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),

    /// Deserialization error. Indicates an issue deserializing a key or value.
    #[error("Deserialization error: {0}")]
    Deser(#[from] crate::ser::DeserError),

    /// Indicates that a write transaction is already in progress.
    #[error("A write transaction is already in progress")]
    WriteLocked,
}

impl HotKvError {
    /// Internal helper to create a `HotKvError::Inner` from any error.
    pub fn from_err<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        HotKvError::Inner(Box::new(err))
    }
}

/// Trait to convert specific read errors into `HotKvError`.
pub trait HotKvReadError: std::error::Error + From<DeserError> + Send + Sync + 'static {
    /// Convert the error into a `HotKvError`.
    fn into_hot_kv_error(self) -> HotKvError;
}

impl HotKvReadError for HotKvError {
    fn into_hot_kv_error(self) -> HotKvError {
        self
    }
}

/// Result type for hot storage operations.
pub type HotKvResult<T> = Result<T, HotKvError>;
