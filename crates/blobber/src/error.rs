use alloy::{eips::eip2718::Eip2718Error, primitives::B256};
use reth::transaction_pool::BlobStoreError;

/// Extraction Result
pub type ExtractionResult<T, E = BlockExtractionError> = std::result::Result<T, E>;

/// Unrecoverable blob extraction errors. These result in the node shutting
/// down. They occur when the blobstore is down or the sidecar is unretrievable.
#[derive(Debug, thiserror::Error)]
pub enum UnrecoverableBlobError {
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

/// Ignorable blob extraction errors. These result in the block being skipped.
#[derive(Debug, thiserror::Error, Copy, Clone)]
pub enum IgnorableBlobError {
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

/// Blob extraction errors
#[derive(Debug, thiserror::Error)]
pub enum BlockExtractionError {
    /// Unrecoverable blob extraction error
    #[error(transparent)]
    Unrecoverable(#[from] UnrecoverableBlobError),
    /// Ignorable blob extraction error
    #[error(transparent)]
    Ignorable(#[from] IgnorableBlobError),
}

impl BlockExtractionError {
    /// Returns true if the error is ignorable
    pub const fn is_ignorable(&self) -> bool {
        matches!(self, Self::Ignorable(_))
    }

    /// Returns true if the error is unrecoverable
    pub const fn is_unrecoverable(&self) -> bool {
        matches!(self, Self::Unrecoverable(_))
    }

    /// Non-4844 transaction error
    pub fn non_4844_transaction() -> Self {
        IgnorableBlobError::Non4844Transaction.into()
    }

    /// Blob decode error
    pub fn blob_decode_error() -> Self {
        IgnorableBlobError::BlobDecodeError.into()
    }

    /// Blob decode error
    pub fn block_decode_error(err: Eip2718Error) -> Self {
        IgnorableBlobError::BlockDecodeError(err).into()
    }

    /// Blob decoded, but expected hash not found
    pub fn block_data_not_found(tx: B256) -> Self {
        IgnorableBlobError::BlockDataNotFound(tx).into()
    }

    /// Missing sidecar error
    pub fn missing_sidecar(tx: B256) -> Self {
        UnrecoverableBlobError::MissingSidecar(tx).into()
    }

    /// Blob store error
    pub fn blob_store(err: BlobStoreError) -> Self {
        UnrecoverableBlobError::BlobStore(err).into()
    }
}

impl From<BlobStoreError> for UnrecoverableBlobError {
    fn from(err: BlobStoreError) -> Self {
        match err {
            BlobStoreError::MissingSidecar(tx) => UnrecoverableBlobError::MissingSidecar(tx),
            _ => UnrecoverableBlobError::BlobStore(err),
        }
    }
}

impl From<BlobStoreError> for BlockExtractionError {
    fn from(err: BlobStoreError) -> Self {
        Self::Unrecoverable(err.into())
    }
}

impl From<reqwest::Error> for BlockExtractionError {
    fn from(err: reqwest::Error) -> Self {
        Self::Unrecoverable(err.into())
    }
}

impl From<Eip2718Error> for BlockExtractionError {
    fn from(err: Eip2718Error) -> Self {
        Self::Ignorable(err.into())
    }
}
