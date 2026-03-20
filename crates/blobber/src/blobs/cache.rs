use crate::{BlobFetcher, BlobSpec, BlobberError, BlobberResult, Blobs, FetchResult};
use alloy::consensus::{SidecarCoder, SimpleCoder, Transaction as _};
use alloy::eips::eip7691::MAX_BLOBS_PER_BLOCK_ELECTRA;
use alloy::eips::merge::EPOCH_SLOTS;
use alloy::primitives::{B256, Bytes, keccak256};
use core::fmt;
use schnellru::{ByLength, LruMap};
use signet_extract::ExtractedEvent;
use signet_zenith::Zenith::BlockSubmitted;
use signet_zenith::ZenithBlock;
use std::marker::PhantomData;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{Instrument, debug_span, error, info, instrument, trace};

const BLOB_CACHE_SIZE: u32 = (MAX_BLOBS_PER_BLOCK_ELECTRA * EPOCH_SLOTS) as u32;
const CACHE_REQUEST_CHANNEL_SIZE: usize = (MAX_BLOBS_PER_BLOCK_ELECTRA * 2) as usize;
const FETCH_RETRIES: usize = 3;
const BETWEEN_RETRIES: Duration = Duration::from_millis(250);

/// Instructions for the cache.
///
/// These instructions are sent to the cache handle to perform operations like
/// retrieving blobs.
#[derive(Debug)]
enum CacheInst {
    Retrieve {
        slot: usize,
        tx_hash: B256,
        version_hashes: Vec<B256>,
        resp: oneshot::Sender<Blobs>,
        span: tracing::Span,
    },
}

/// Handle for the cache.
#[derive(Debug, Clone)]
pub struct CacheHandle<Coder = SimpleCoder> {
    sender: mpsc::Sender<CacheInst>,

    _coder: PhantomData<Coder>,
}

impl<Coder> CacheHandle<Coder> {
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
    ) -> BlobberResult<Blobs> {
        let (resp, receiver) = oneshot::channel();

        self.send(CacheInst::Retrieve {
            slot,
            tx_hash,
            version_hashes,
            resp,
            span: tracing::Span::current(),
        })
        .await;

        receiver.await.map_err(|_| BlobberError::missing_sidecar(tx_hash))
    }

    /// Fetch the blobs using [`Self::fetch_blobs`] and decode them to get the
    /// Zenith block data using the provided coder.
    pub async fn fetch_and_decode<R>(
        &self,
        slot: usize,
        extract: &ExtractedEvent<'_, R, BlockSubmitted>,
    ) -> BlobberResult<Bytes>
    where
        Coder: SidecarCoder + Default,
    {
        let tx_hash = extract.tx_hash();
        let versioned_hashes = extract
            .tx
            .as_eip4844()
            .ok_or_else(BlobberError::non_4844_transaction)?
            .blob_versioned_hashes()
            .expect("tx is eip4844");

        let blobs = self.fetch_blobs(slot, tx_hash, versioned_hashes.to_owned()).await?;

        Coder::default()
            .decode_all(blobs.as_ref())
            .ok_or_else(BlobberError::blob_decode_error)?
            .into_iter()
            .find(|data| keccak256(data) == extract.block_data_hash())
            .map(Into::into)
            .ok_or_else(|| BlobberError::block_data_not_found(extract.block_data_hash()))
            .inspect(|_| {
                trace!(slot, %tx_hash, "Successfully fetched and decoded blobs");
            })
    }

    /// Fetch the blobs, decode them using the provided coder, and construct a
    /// Zenith block from the header and data.
    ///
    /// # Returns
    ///
    /// - `Ok(ZenithBlock)` if the block was successfully fetched and
    ///   decoded.
    /// - `Ok(ZenithBlock)` with an EMPTY BLOCK if the block_data could not be
    ///   decoded (e.g., due to a malformatted blob).
    /// - `Err(FetchError)` if there was an unrecoverable error fetching the
    ///   blobs.
    pub async fn signet_block<R>(
        &self,
        host_block_number: u64,
        slot: usize,
        extract: &ExtractedEvent<'_, R, BlockSubmitted>,
    ) -> FetchResult<ZenithBlock>
    where
        Coder: SidecarCoder + Default,
    {
        let header = extract.ru_header(host_block_number);
        let block_data = match self.fetch_and_decode(slot, extract).await {
            Ok(buf) => buf,
            Err(BlobberError::Decode(_)) => {
                trace!("Failed to decode block data");
                Bytes::default()
            }
            Err(BlobberError::Fetch(err)) => return Err(err),
        };
        Ok(ZenithBlock::from_header_and_data(header, block_data)).inspect(|block| {
            trace!(
                host_block_number = %block.header().hostBlockNumber,
                tx_count = block.transactions().len(),
                "Constructed ZenithBlock from header and data"
            );
        })
    }
}

/// Retrieves blobs and stores them in a cache for later use.
pub struct BlobCacher {
    fetcher: BlobFetcher,

    cache: Mutex<LruMap<(usize, B256), Blobs>>,
}

impl fmt::Debug for BlobCacher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlobCacher").field("fetcher", &self.fetcher).finish_non_exhaustive()
    }
}

impl BlobCacher {
    /// Creates a new `BlobCacher` with the provided fetcher and cache size.
    pub fn new(fetcher: BlobFetcher) -> Self {
        Self { fetcher, cache: LruMap::new(ByLength::new(BLOB_CACHE_SIZE)).into() }
    }

    /// Fetches blobs for a given slot and transaction hash.
    #[instrument(skip(self), target = "signet_blobber::BlobCacher", fields(retries = FETCH_RETRIES))]
    async fn fetch_blobs(
        &self,
        slot: usize,
        tx_hash: B256,
        versioned_hashes: Vec<B256>,
    ) -> BlobberResult<Blobs> {
        // Cache hit
        if let Some(blobs) = self.cache.lock().unwrap().get(&(slot, tx_hash)) {
            info!(target: "signet_blobber::BlobCacher", "Cache hit");
            return Ok(blobs.clone());
        }

        let spec = BlobSpec { tx_hash, slot, versioned_hashes };

        // Cache miss, use the fetcher to retrieve blobs
        // Retry fetching blobs up to `FETCH_RETRIES` times
        for attempt in 1..=FETCH_RETRIES {
            let Ok(blobs) = self
                .fetcher
                .fetch_blobs(&spec)
                .instrument(debug_span!("fetch_blobs_loop", attempt))
                .await
            else {
                tokio::time::sleep(BETWEEN_RETRIES).await;
                continue;
            };

            self.cache.lock().unwrap().insert((slot, tx_hash), blobs.clone());
            return Ok(blobs);
        }
        error!(target: "signet_blobber::BlobCacher",  "All fetch attempts failed");
        Err(BlobberError::missing_sidecar(tx_hash))
    }

    /// Processes the cache instructions.
    async fn handle_inst(self: Arc<Self>, inst: CacheInst) {
        match inst {
            CacheInst::Retrieve { slot, tx_hash, version_hashes, resp, span } => {
                if let Ok(blobs) =
                    self.fetch_blobs(slot, tx_hash, version_hashes).instrument(span).await
                {
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
    pub fn spawn<C: SidecarCoder + Default>(self) -> CacheHandle<C> {
        let (sender, inst) = mpsc::channel(CACHE_REQUEST_CHANNEL_SIZE);
        tokio::spawn(Arc::new(self).task_future(inst));
        CacheHandle { sender, _coder: PhantomData }
    }
}
