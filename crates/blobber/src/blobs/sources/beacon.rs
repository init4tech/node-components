use crate::{AsyncBlobSource, BlobSpec, Blobs};
use alloy::rpc::types::beacon::sidecar::GetBlobsResponse;
use std::{future::Future, pin::Pin, sync::Arc};
use tracing::instrument;

type BlobSourceError = Box<dyn core::error::Error + Send + Sync>;
type BlobFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Blobs>, BlobSourceError>> + Send + 'a>>;

/// Fetches blobs from a beacon-chain consensus-layer node.
///
/// Uses the `GET /eth/v1/beacon/blobs/{slot}` endpoint, filtering by the
/// versioned hashes from the [`BlobSpec`]. This is a lenient fetch: whatever
/// blobs the CL provides are returned without exact-count enforcement.
#[derive(Debug, Clone)]
pub struct BeaconBlobSource {
    client: reqwest::Client,
    url: url::Url,
}

impl BeaconBlobSource {
    /// Creates a new [`BeaconBlobSource`].
    pub const fn new(client: reqwest::Client, url: url::Url) -> Self {
        Self { client, url }
    }
}

impl AsyncBlobSource for BeaconBlobSource {
    fn get_blob(&self, spec: &BlobSpec) -> BlobFuture<'_> {
        let slot = spec.slot;
        let versioned_hashes = spec.versioned_hashes.clone();
        Box::pin(async move {
            let blobs = fetch_from_cl(&self.client, &self.url, slot, &versioned_hashes).await?;
            Ok((!blobs.is_empty()).then_some(blobs))
        })
    }
}

/// Queries the consensus client for blobs at a given slot, filtering by
/// versioned hashes (best-effort).
///
/// Returns whatever blobs the consensus client provides, even if fewer than
/// requested. We assume the CL will never return unrelated blobs.
#[instrument(skip_all)]
async fn fetch_from_cl(
    client: &reqwest::Client,
    base_url: &url::Url,
    slot: usize,
    versioned_hashes: &[alloy::primitives::B256],
) -> Result<Blobs, BlobSourceError> {
    let mut url = base_url.join(&format!("/eth/v1/beacon/blobs/{slot}"))?;

    let hashes = versioned_hashes.iter().map(|h| h.to_string()).collect::<Vec<_>>().join(",");
    url.query_pairs_mut().append_pair("versioned_hashes", &hashes);

    let response = client.get(url).header("accept", "application/json").send().await?;
    let response: GetBlobsResponse = response.json().await?;

    Ok(Arc::new(response.data).into())
}
