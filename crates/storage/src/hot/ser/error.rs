/// Error type for deserialization errors.
///
/// Erases the underlying error type to a boxed trait object or a string
/// message, for convenience.
#[derive(thiserror::Error, Debug)]
pub enum DeserError {
    /// Boxed error.
    #[error(transparent)]
    Boxed(Box<dyn std::error::Error + Send + Sync + 'static>),

    /// String error message.
    #[error("{0}")]
    String(String),

    /// Deserialization ended with extra bytes remaining.
    #[error("inexact deserialization: {extra_bytes} extra bytes remaining")]
    InexactDeser {
        /// Number of extra bytes remaining after deserialization.
        extra_bytes: usize,
    },

    /// Not enough data to complete deserialization.
    #[error("insufficient data: needed {needed} bytes, but only {available} available")]
    InsufficientData {
        /// Number of bytes needed.
        needed: usize,
        /// Number of bytes available.
        available: usize,
    },
}

impl From<&str> for DeserError {
    fn from(err: &str) -> Self {
        DeserError::String(err.to_string())
    }
}

impl DeserError {
    /// Box an error into a `DeserError`.
    pub fn from<E>(err: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        DeserError::Boxed(Box::new(err))
    }
}
