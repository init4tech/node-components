use alloy::{eips::eip2718::Eip2718Error, primitives::B256};
use reth::transaction_pool::BlobStoreError;

/// Result using [`BlobFetcherError`] as the default error type.
pub type BlobberResult<T, E = BlobberError> = std::result::Result<T, E>;

/// Result using [`FetchError`] as the default error type.
pub type FetchResult<T> = BlobberResult<T, FetchError>;

/// Result using [`DecodeError`] as the default error type.
pub type DecodeResult<T> = BlobberResult<T, DecodeError>;

/// Unrecoverable blob fetching errors. These result in the node shutting
/// down. They occur when the blobstore is down or the sidecar is unretrievable.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// Reqwest error
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    /// Missing sidecar error
    #[error("Cannot retrieve sidecar for {0} from any source")]
    MissingSidecar(B256),
    /// Reth blobstore error.
    #[error(transparent)]
    BlobStore(BlobStoreError),
    /// Url parse error.
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    /// Consensus client URL not set error.
    #[error("Consensus client URL not set")]
    ConsensusClientUrlNotSet,
    /// Pylon client URL not set error.
    #[error("Pylon client URL not set")]
    PylonClientUrlNotSet,
}

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
    pub fn block_data_not_found(tx: B256) -> Self {
        DecodeError::BlockDataNotFound(tx).into()
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

impl From<BlobStoreError> for FetchError {
    fn from(err: BlobStoreError) -> Self {
        match err {
            BlobStoreError::MissingSidecar(tx) => FetchError::MissingSidecar(tx),
            _ => FetchError::BlobStore(err),
        }
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
