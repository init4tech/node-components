use crate::{BlobFetcherError, Blobs, FetchResult};
use alloy::consensus::{SidecarCoder, SimpleCoder, Transaction as _};
use alloy::primitives::{keccak256, Bytes, B256};
use reth::transaction_pool::TransactionPool;
use reth::{network::cache::LruMap, primitives::Receipt};
use signet_extract::ExtractedEvent;
use signet_zenith::Zenith::BlockSubmitted;
use signet_zenith::ZenithBlock;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info, instrument, warn};

const BLOB_CACHE_SIZE: u32 = 144;
const FETCH_RETRIES: usize = 3;
const BETWEEN_RETRIES: Duration = Duration::from_millis(250);

/// Instructions for the cache.
///
/// These instructions are sent to the cache handle to perform operations like
/// retrieving blobs.
#[derive(Debug)]
enum CacheInst {
    Retrieve { slot: usize, tx_hash: B256, version_hashes: Vec<B256>, resp: oneshot::Sender<Blobs> },
}

/// Handle for the cache.
#[derive(Debug, Clone)]
pub struct CacheHandle {
    sender: mpsc::Sender<CacheInst>,
}

impl CacheHandle {
    /// Sends a cache instruction.
    async fn send(&self, inst: CacheInst) {
        let _ = self.sender.send(inst).await;
    }

    /// Fetches blobs from the cache. This triggers a background task to
    /// fetch blobs if they are not found in the cache.
    pub async fn fetch_blobs(
        &self,
        slot: usize,
        tx_hash: B256,
        version_hashes: Vec<B256>,
    ) -> FetchResult<Blobs> {
        let (resp, receiver) = oneshot::channel();

        self.send(CacheInst::Retrieve { slot, tx_hash, version_hashes, resp }).await;

        receiver.await.map_err(|_| BlobFetcherError::missing_sidecar(tx_hash))
    }

    /// Fetch the blobs using [`Self::fetch_blobs`] and decode them to get the
    /// Zenith block data.
    pub async fn fetch_and_decode(
        &self,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
    ) -> FetchResult<Bytes> {
        let tx_hash = extract.tx_hash();
        let versioned_hashes = extract
            .tx
            .as_eip4844()
            .ok_or_else(BlobFetcherError::non_4844_transaction)?
            .blob_versioned_hashes()
            .expect("tx is eip4844");

        let blobs = self.fetch_blobs(slot, tx_hash, versioned_hashes.to_owned()).await?;

        SimpleCoder::default()
            .decode_all(blobs.as_ref())
            .ok_or_else(BlobFetcherError::blob_decode_error)?
            .into_iter()
            .find(|data| keccak256(data) == extract.block_data_hash())
            .map(Into::into)
            .ok_or_else(|| BlobFetcherError::block_data_not_found(tx_hash))
    }

    /// Fetch the blobs, decode them, and construct a Zenith block from the
    /// header and data.
    pub async fn signet_block(
        &self,
        host_block_number: u64,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
    ) -> FetchResult<ZenithBlock> {
        let header = extract.ru_header(host_block_number);
        self.fetch_and_decode(slot, extract)
            .await
            .map(|buf| ZenithBlock::from_header_and_data(header, buf))
    }
}

/// Retrieves blobs and stores them in a cache for later use.
pub struct BlobCacher<Pool> {
    fetcher: crate::BlobFetcher<Pool>,

    cache: Mutex<LruMap<(usize, B256), Blobs>>,
}

impl<Pool: core::fmt::Debug> core::fmt::Debug for BlobCacher<Pool> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobCacher").field("fetcher", &self.fetcher).finish_non_exhaustive()
    }
}

impl<Pool: TransactionPool + 'static> BlobCacher<Pool> {
    /// Creates a new `BlobCacher` with the provided extractor and cache size.
    pub fn new(fetcher: crate::BlobFetcher<Pool>) -> Self {
        Self { fetcher, cache: LruMap::new(BLOB_CACHE_SIZE).into() }
    }

    /// Fetches blobs for a given slot and transaction hash.
    #[instrument(skip(self), target = "signet_blobber::BlobCacher", fields(retries = FETCH_RETRIES))]
    async fn fetch_blobs(
        &self,
        slot: usize,
        tx_hash: B256,
        versioned_hashes: Vec<B256>,
    ) -> FetchResult<Blobs> {
        // Cache hit
        if let Some(blobs) = self.cache.lock().unwrap().get(&(slot, tx_hash)) {
            info!(target: "signet_blobber::BlobCacher", "Cache hit");
            return Ok(blobs.clone());
        }

        // Cache miss, use the fetcher to retrieve blobs
        // Retry fetching blobs up to `FETCH_RETRIES` times
        for attempt in 1..=FETCH_RETRIES {
            let blobs = self.fetcher.fetch_blobs(slot, tx_hash, &versioned_hashes).await;

            match blobs {
                Ok(blobs) => {
                    self.cache.lock().unwrap().insert((slot, tx_hash), blobs.clone());
                    return Ok(blobs);
                }
                Err(BlobFetcherError::Ignorable(e)) => {
                    warn!(target: "signet_blobber::BlobCacher", attempt, %e, "Blob fetch attempt failed.");
                    tokio::time::sleep(BETWEEN_RETRIES).await;
                    continue;
                }
                Err(e) => return Err(e), // unrecoverable error
            }
        }
        error!(target: "signet_blobber::BlobCacher",  "All fetch attempts failed");
        Err(BlobFetcherError::missing_sidecar(tx_hash))
    }

    /// Processes the cache instructions.
    async fn handle_inst(self: Arc<Self>, inst: CacheInst) {
        match inst {
            CacheInst::Retrieve { slot, tx_hash, version_hashes, resp } => {
                if let Ok(blobs) = self.fetch_blobs(slot, tx_hash, version_hashes).await {
                    // if listener has gone away, that's okay, we just won't send the response
                    let _ = resp.send(blobs);
                }
            }
        }
    }

    async fn task_future(self: Arc<Self>, mut inst: mpsc::Receiver<CacheInst>) {
        while let Some(inst) = inst.recv().await {
            let this = Arc::clone(&self);
            tokio::spawn(async move {
                this.handle_inst(inst).await;
            });
        }
    }

    /// Spawns the cache task to handle incoming instructions.
    ///
    /// # Panics
    /// This function will panic if the cache task fails to spawn.
    pub fn spawn(self) -> CacheHandle {
        let (sender, inst) = mpsc::channel(12);
        tokio::spawn(Arc::new(self).task_future(inst));
        CacheHandle { sender }
    }
}
