use crate::Blobs;
use alloy::primitives::{B256, TxHash};
use std::{future::Future, pin::Pin};

/// Boxed error type for blob source operations.
pub(crate) type BlobSourceError = Box<dyn core::error::Error + Send + Sync>;

/// Boxed future returned by [`AsyncBlobSource::get_blob`].
pub(crate) type BlobFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Blobs>, BlobSourceError>> + Send + 'a>>;

/// All known context for a blob lookup.
///
/// Sources use whichever fields they need and ignore the rest.
#[derive(Debug, Clone)]
pub struct BlobSpec {
    /// The transaction hash to look up.
    pub tx_hash: TxHash,
    /// The beacon slot number.
    pub slot: usize,
    /// The versioned hashes of the blobs.
    pub versioned_hashes: Vec<B256>,
}

/// Synchronous blob source (e.g. in-process transaction pool).
pub trait BlobSource: Send + Sync {
    /// Retrieve blobs for the given spec.
    fn get_blob(&self, spec: &BlobSpec) -> Result<Option<Blobs>, BlobSourceError>;
}

/// Asynchronous blob source (e.g. remote API).
///
/// Uses boxed futures for object safety — `BlobFetcher` stores these
/// as `Box<dyn AsyncBlobSource>`.
pub trait AsyncBlobSource: Send + Sync {
    /// Retrieve blobs for the given spec.
    fn get_blob(&self, spec: &BlobSpec) -> BlobFuture<'_>;
}
