use crate::{BlobberError, BlobberResult, Blobs, FetchResult};
use alloy::consensus::{SidecarCoder, SimpleCoder, Transaction as _};
use alloy::eips::eip7691::MAX_BLOBS_PER_BLOCK_ELECTRA;
use alloy::eips::merge::EPOCH_SLOTS;
use alloy::primitives::{B256, Bytes, keccak256};
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
    pub async fn fetch_and_decode_with_coder<C: SidecarCoder>(
        &self,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        mut coder: C,
    ) -> BlobberResult<Bytes> {
        let tx_hash = extract.tx_hash();
        let versioned_hashes = extract
            .tx
            .as_eip4844()
            .ok_or_else(BlobberError::non_4844_transaction)?
            .blob_versioned_hashes()
            .expect("tx is eip4844");

        let blobs = self.fetch_blobs(slot, tx_hash, versioned_hashes.to_owned()).await?;

        coder
            .decode_all(blobs.as_ref())
            .ok_or_else(BlobberError::blob_decode_error)?
            .into_iter()
            .find(|data| keccak256(data) == extract.block_data_hash())
            .map(Into::into)
            .ok_or_else(|| BlobberError::block_data_not_found(tx_hash))
    }

    /// Fetch the blobs using [`Self::fetch_blobs`] and decode them using
    /// [`SimpleCoder`] to get the Zenith block data.
    pub async fn fech_and_decode(
        &self,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
    ) -> BlobberResult<Bytes> {
        self.fetch_and_decode_with_coder(slot, extract, SimpleCoder::default()).await
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
    pub async fn signet_block_with_coder<C: SidecarCoder>(
        &self,
        host_block_number: u64,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        coder: C,
    ) -> FetchResult<ZenithBlock> {
        let header = extract.ru_header(host_block_number);
        let block_data = match self.fetch_and_decode_with_coder(slot, extract, coder).await {
            Ok(buf) => buf,
            Err(BlobberError::Decode(_)) => {
                trace!("Failed to decode block data");
                Bytes::default()
            }
            Err(BlobberError::Fetch(err)) => return Err(err),
        };
        Ok(ZenithBlock::from_header_and_data(header, block_data))
    }

    /// Fetch the blobs, decode them using [`SimpleCoder`], and construct a
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
    pub async fn signet_block(
        &self,
        host_block_number: u64,
        slot: usize,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
    ) -> FetchResult<ZenithBlock> {
        self.signet_block_with_coder(host_block_number, slot, extract, SimpleCoder::default()).await
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
    ) -> BlobberResult<Blobs> {
        // Cache hit
        if let Some(blobs) = self.cache.lock().unwrap().get(&(slot, tx_hash)) {
            info!(target: "signet_blobber::BlobCacher", "Cache hit");
            return Ok(blobs.clone());
        }

        // Cache miss, use the fetcher to retrieve blobs
        // Retry fetching blobs up to `FETCH_RETRIES` times
        for attempt in 1..=FETCH_RETRIES {
            let Ok(blobs) = self
                .fetcher
                .fetch_blobs(slot, tx_hash, &versioned_hashes)
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
    pub fn spawn(self) -> CacheHandle {
        let (sender, inst) = mpsc::channel(CACHE_REQUEST_CHANNEL_SIZE);
        tokio::spawn(Arc::new(self).task_future(inst));
        CacheHandle { sender }
    }
}

#[cfg(test)]
mod tests {
    use crate::BlobFetcher;

    use super::*;
    use alloy::{
        consensus::{SidecarBuilder, SignableTransaction as _, TxEip2930},
        eips::Encodable2718,
        primitives::{TxKind, U256, bytes},
        rlp::encode,
        signers::{SignerSync, local::PrivateKeySigner},
    };
    use init4_bin_base::utils::calc::SlotCalculator;
    use reth::primitives::Transaction;
    use reth_transaction_pool::{
        PoolTransaction, TransactionOrigin,
        test_utils::{MockTransaction, testing_pool},
    };
    use signet_types::{constants::SignetSystemConstants, primitives::TransactionSigned};

    #[tokio::test]
    async fn test_fetch_from_pool() -> eyre::Result<()> {
        let wallet = PrivateKeySigner::random();
        let pool = testing_pool();

        let test = signet_constants::KnownChains::Test;

        let constants: SignetSystemConstants = test.try_into().unwrap();
        let calc = SlotCalculator::new(0, 0, 12);

        let explorer_url = "https://api.holesky.blobscan.com/";
        let client = reqwest::Client::builder().use_rustls_tls();

        let tx = Transaction::Eip2930(TxEip2930 {
            chain_id: 17001,
            nonce: 2,
            gas_limit: 50000,
            gas_price: 1_500_000_000,
            to: TxKind::Call(constants.host_zenith()),
            value: U256::from(1_f64),
            input: bytes!(""),
            ..Default::default()
        });

        let encoded_transactions =
            encode(vec![sign_tx_with_key_pair(wallet.clone(), tx).encoded_2718()]);

        let result = SidecarBuilder::<SimpleCoder>::from_slice(&encoded_transactions).build();
        assert!(result.is_ok());

        let mut mock_transaction = MockTransaction::eip4844_with_sidecar(result.unwrap().into());
        let transaction =
            sign_tx_with_key_pair(wallet, Transaction::from(mock_transaction.clone()));

        mock_transaction.set_hash(*transaction.hash());

        pool.add_transaction(TransactionOrigin::Local, mock_transaction.clone()).await?;

        // Spawn the cache
        let cache = BlobFetcher::builder()
            .with_pool(pool.clone())
            .with_explorer_url(explorer_url)
            .with_client_builder(client)
            .unwrap()
            .with_slot_calculator(calc)
            .build_cache()?;
        let handle = cache.spawn();

        let got = handle
            .fetch_blobs(
                0, // this is ignored by the pool
                *mock_transaction.hash(),
                mock_transaction.blob_versioned_hashes().unwrap().to_owned(),
            )
            .await;
        assert!(got.is_ok());

        let got_blobs = got.unwrap();
        assert!(got_blobs.len() == 1);

        Ok(())
    }

    fn sign_tx_with_key_pair(wallet: PrivateKeySigner, tx: Transaction) -> TransactionSigned {
        let signature = wallet.sign_hash_sync(&tx.signature_hash()).unwrap();
        TransactionSigned::new_unhashed(tx, signature)
    }
}
