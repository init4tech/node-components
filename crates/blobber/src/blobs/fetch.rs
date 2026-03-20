use crate::{AsyncBlobSource, BlobFetcherBuilder, BlobSource, BlobSpec, FetchError, FetchResult};
use alloy::{
    consensus::{Blob, BlobTransactionSidecar},
    eips::eip7594::{BlobTransactionSidecarEip7594, BlobTransactionSidecarVariant},
};
use std::{ops::Deref, sync::Arc};
use tracing::instrument;

/// Blobs which may be a local shared sidecar, or a list of blobs from an
/// external source.
///
/// The contents are arc-wrapped to allow for cheap cloning.
#[derive(Hash, Debug, Clone, PartialEq, Eq)]
pub enum Blobs {
    /// Local pooled transaction sidecar
    FromPool(Arc<BlobTransactionSidecarVariant>),
    /// Some other blob source.
    Other(Arc<Vec<Blob>>),
}

impl From<Vec<Blob>> for Blobs {
    fn from(blobs: Vec<Blob>) -> Self {
        Self::Other(Arc::new(blobs))
    }
}

impl From<Arc<Vec<Blob>>> for Blobs {
    fn from(blobs: Arc<Vec<Blob>>) -> Self {
        Blobs::Other(blobs)
    }
}

impl From<BlobTransactionSidecarVariant> for Blobs {
    fn from(sidecar: BlobTransactionSidecarVariant) -> Self {
        Self::FromPool(Arc::new(sidecar))
    }
}

impl From<Arc<BlobTransactionSidecarVariant>> for Blobs {
    fn from(sidecar: Arc<BlobTransactionSidecarVariant>) -> Self {
        Self::FromPool(sidecar)
    }
}

impl From<BlobTransactionSidecar> for Blobs {
    fn from(sidecar: BlobTransactionSidecar) -> Self {
        Self::FromPool(Arc::new(BlobTransactionSidecarVariant::Eip4844(sidecar)))
    }
}

impl From<BlobTransactionSidecarEip7594> for Blobs {
    fn from(sidecar: BlobTransactionSidecarEip7594) -> Self {
        Self::FromPool(Arc::new(BlobTransactionSidecarVariant::Eip7594(sidecar)))
    }
}

impl AsRef<Vec<Blob>> for Blobs {
    fn as_ref(&self) -> &Vec<Blob> {
        match self {
            Blobs::FromPool(variant) => match variant.deref() {
                BlobTransactionSidecarVariant::Eip4844(sidecar) => &sidecar.blobs,
                BlobTransactionSidecarVariant::Eip7594(sidecar) => &sidecar.blobs,
            },
            Blobs::Other(blobs) => blobs,
        }
    }
}

impl AsRef<[Blob]> for Blobs {
    fn as_ref(&self) -> &[Blob] {
        AsRef::<Vec<Blob>>::as_ref(self)
    }
}

impl FromIterator<Blob> for Blobs {
    fn from_iter<T: IntoIterator<Item = Blob>>(iter: T) -> Self {
        Blobs::Other(Arc::new(iter.into_iter().collect()))
    }
}

impl Blobs {
    /// Returns the blobs as a slice
    pub fn as_slice(&self) -> &[Blob] {
        self.as_ref()
    }

    /// Return the blobs as a Vec
    pub fn as_vec(&self) -> &Vec<Blob> {
        self.as_ref()
    }

    /// Returns true if the sidecar has no blobs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of blobs in the sidecar.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }
}

/// Fetches blobs from multiple sources, trying synchronous sources first,
/// then racing asynchronous sources concurrently.
pub struct BlobFetcher {
    sync_sources: Vec<Box<dyn BlobSource>>,
    async_sources: Vec<Box<dyn AsyncBlobSource>>,
}

impl core::fmt::Debug for BlobFetcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobFetcher")
            .field("sync_sources", &self.sync_sources.len())
            .field("async_sources", &self.async_sources.len())
            .finish()
    }
}

impl BlobFetcher {
    /// Returns a new [`BlobFetcherBuilder`].
    pub fn builder() -> BlobFetcherBuilder {
        BlobFetcherBuilder::default()
    }

    /// Creates a new `BlobFetcher` from pre-built source lists.
    pub(crate) fn new(
        sync_sources: Vec<Box<dyn BlobSource>>,
        async_sources: Vec<Box<dyn AsyncBlobSource>>,
    ) -> Self {
        Self { sync_sources, async_sources }
    }

    /// Fetch blobs by trying sync sources first, then racing async sources.
    #[instrument(skip(self))]
    pub(crate) async fn fetch_blobs(&self, spec: &BlobSpec) -> FetchResult<Blobs> {
        // Try each sync source in order
        for source in &self.sync_sources {
            match source.get_blob(spec) {
                Ok(Some(blobs)) => return Ok(blobs),
                Ok(None) | Err(_) => continue,
            }
        }

        // Race all async sources concurrently using tokio::select on
        // pinned futures (all borrowed from &self, no spawning needed).
        if !self.async_sources.is_empty() {
            let mut futs: Vec<_> = self.async_sources.iter().map(|s| s.get_blob(spec)).collect();

            while !futs.is_empty() {
                let (result, _index, remaining) = futures_util::future::select_all(futs).await;
                if let Ok(Some(blobs)) = result {
                    return Ok(blobs);
                }
                futs = remaining;
            }
        }

        Err(FetchError::MissingSidecar(spec.tx_hash))
    }
}
