use crate::{AsyncBlobSource, BlobSpec, Blobs};
use std::{future::Future, pin::Pin};

type BlobSourceError = Box<dyn core::error::Error + Send + Sync>;
type BlobFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Blobs>, BlobSourceError>> + Send + 'a>>;

/// Fetches blobs from a blob explorer (e.g. Blobscan) by transaction hash.
///
/// Wraps a [`foundry_blob_explorers::Client`] and queries for blob data
/// using only the transaction hash from the [`BlobSpec`].
#[derive(Debug, Clone)]
pub struct BlobExplorerSource {
    client: foundry_blob_explorers::Client,
}

impl BlobExplorerSource {
    /// Creates a new [`BlobExplorerSource`] from an explorer client.
    pub const fn new(client: foundry_blob_explorers::Client) -> Self {
        Self { client }
    }
}

impl AsyncBlobSource for BlobExplorerSource {
    fn get_blob(&self, spec: &BlobSpec) -> BlobFuture<'_> {
        let tx_hash = spec.tx_hash;
        Box::pin(async move {
            let sidecar = self.client.transaction(tx_hash).await?;
            let blobs: Blobs = sidecar.blobs.iter().map(|b| *b.data).collect();
            debug_assert!(!blobs.is_empty(), "Explorer returned no blobs");
            Ok(Some(blobs))
        })
    }
}
