use crate::{DecodeError, FetchError};
use alloy::{eips::eip2718::Eip2718Error, primitives::B256};
use reth::transaction_pool::BlobStoreError;

/// Result using [`BlobberError`] as the default error type.
pub type BlobberResult<T, E = BlobberError> = std::result::Result<T, E>;

/// Blob fetching errors
#[derive(Debug, thiserror::Error)]
pub enum BlobberError {
    /// Unrecoverable blob fetching error
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Ignorable blob fetching error
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

impl BlobberError {
    /// Returns true if the error is ignorable
    pub const fn is_decode(&self) -> bool {
        matches!(self, Self::Decode(_))
    }

    /// Returns true if the error is unrecoverable
    pub const fn is_fetch(&self) -> bool {
        matches!(self, Self::Fetch(_))
    }

    /// Non-4844 transaction error
    pub fn non_4844_transaction() -> Self {
        DecodeError::Non4844Transaction.into()
    }

    /// Blob decode error
    pub fn blob_decode_error() -> Self {
        DecodeError::BlobDecodeError.into()
    }

    /// Blob decode error
    pub fn block_decode_error(err: Eip2718Error) -> Self {
        DecodeError::BlockDecodeError(err).into()
    }

    /// Blob decoded, but expected hash not found
    pub fn block_data_not_found(data_hash: B256) -> Self {
        DecodeError::BlockDataNotFound(data_hash).into()
    }

    /// Missing sidecar error
    pub fn missing_sidecar(tx: B256) -> Self {
        FetchError::MissingSidecar(tx).into()
    }

    /// Blob store error
    pub fn blob_store(err: BlobStoreError) -> Self {
        FetchError::BlobStore(err).into()
    }
}

impl From<BlobStoreError> for BlobberError {
    fn from(err: BlobStoreError) -> Self {
        Self::Fetch(err.into())
    }
}

impl From<reqwest::Error> for BlobberError {
    fn from(err: reqwest::Error) -> Self {
        Self::Fetch(err.into())
    }
}

impl From<Eip2718Error> for BlobberError {
    fn from(err: Eip2718Error) -> Self {
        Self::Decode(err.into())
    }
}

impl From<url::ParseError> for BlobberError {
    fn from(err: url::ParseError) -> Self {
        Self::Fetch(FetchError::UrlParse(err))
    }
}
