use alloy::{eips::eip2718::Eip2718Error, primitives::B256};

/// Result using [`DecodeError`] as the default error type.
pub type DecodeResult<T> = Result<T, DecodeError>;

/// Ignorable blob fetching errors. These result in the block being skipped.
#[derive(Debug, thiserror::Error, Copy, Clone)]
pub enum DecodeError {
    /// Incorrect transaction type error
    #[error("Non-4844 transaction")]
    Non4844Transaction,
    /// Decoding error from the internal [`SimpleCoder`]. This indicates the
    /// blobs are not formatted in the simple coder format.
    ///
    /// [`SimpleCoder`]: alloy::consensus::SimpleCoder
    #[error("Decoding failed")]
    BlobDecodeError,
    /// Block data not found in decoded blob
    #[error("Block data not found in decoded blob. Expected block hash: {0}")]
    BlockDataNotFound(B256),
    /// Error while decoding block from blob
    #[error("Block decode error: {0}")]
    BlockDecodeError(#[from] Eip2718Error),
}
