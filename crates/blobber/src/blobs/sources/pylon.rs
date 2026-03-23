use crate::{
    AsyncBlobSource, BlobSpec, Blobs,
    blobs::source::{BlobFuture, BlobSourceError},
};
use alloy::eips::eip7594::BlobTransactionSidecarVariant;
use std::sync::Arc;
use tracing::instrument;

/// Fetches blobs from a Pylon blob indexer by transaction hash.
///
/// Uses the `GET /sidecar/{tx_hash}` endpoint, returning the full
/// [`BlobTransactionSidecarVariant`] deserialized from the response.
#[derive(Debug, Clone)]
pub struct PylonBlobSource {
    client: reqwest::Client,
    url: url::Url,
}

impl PylonBlobSource {
    /// Creates a new [`PylonBlobSource`].
    pub const fn new(client: reqwest::Client, url: url::Url) -> Self {
        Self { client, url }
    }
}

impl AsyncBlobSource for PylonBlobSource {
    fn get_blob(&self, spec: &BlobSpec) -> BlobFuture<'_> {
        let tx_hash = spec.tx_hash;
        Box::pin(async move {
            let blobs = fetch_from_pylon(&self.client, &self.url, tx_hash).await?;
            Ok((!blobs.is_empty()).then_some(blobs))
        })
    }
}

/// Queries the Pylon blob indexer for a sidecar by transaction hash.
#[instrument(skip_all)]
async fn fetch_from_pylon(
    client: &reqwest::Client,
    base_url: &url::Url,
    tx_hash: alloy::primitives::TxHash,
) -> Result<Blobs, BlobSourceError> {
    let url = base_url.join(&format!("sidecar/{tx_hash}"))?;

    let sidecar: Arc<BlobTransactionSidecarVariant> = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(sidecar.into())
}
