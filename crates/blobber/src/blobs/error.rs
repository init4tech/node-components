use alloy::primitives::B256;

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
    /// A blob source returned an error.
    #[error(transparent)]
    BlobSource(Box<dyn core::error::Error + Send + Sync>),
    /// Url parse error.
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
}
