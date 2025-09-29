use alloy::primitives::B256;
use reth::transaction_pool::BlobStoreError;

/// Result using [`FetchError`] as the default error type.
pub type FetchResult<T> = Result<T, FetchError>;

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

impl From<BlobStoreError> for FetchError {
    fn from(err: BlobStoreError) -> Self {
        match err {
            BlobStoreError::MissingSidecar(tx) => FetchError::MissingSidecar(tx),
            _ => FetchError::BlobStore(err),
        }
    }
}
