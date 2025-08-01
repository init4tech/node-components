use crate::{
    error::UnrecoverableBlobError, shim::ExtractableChainShim, BlockExtractionError,
    ExtractionResult,
};
use alloy::{
    consensus::{Blob, SidecarCoder, SimpleCoder},
    eips::eip7594::BlobTransactionSidecarVariant,
    primitives::{keccak256, TxHash, B256},
};
use init4_bin_base::utils::calc::SlotCalculator;
use reth::{
    primitives::Receipt, rpc::types::beacon::sidecar::BeaconBlobBundle,
    transaction_pool::TransactionPool,
};
use signet_extract::{ExtractedEvent, Extracts};
use signet_zenith::{Zenith::BlockSubmitted, ZenithBlock};
use smallvec::SmallVec;
use std::{borrow::Cow, ops::Deref, sync::Arc};
use tokio::select;
use tracing::{error, instrument, trace};

/// Blobs which may be a local shared sidecar, or a list of blobs from an
/// external source.
///
/// The contents are arc-wrapped to allow for cheap cloning.
#[derive(Debug, Clone)]
pub enum Blobs {
    /// Local pooled transaction sidecar
    FromPool(Arc<BlobTransactionSidecarVariant>),
    /// Some other blob source.
    Other(Arc<Vec<Blob>>),
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

impl From<Arc<BlobTransactionSidecarVariant>> for Blobs {
    fn from(sidecar: Arc<BlobTransactionSidecarVariant>) -> Self {
        Blobs::FromPool(sidecar)
    }
}

impl From<Vec<Blob>> for Blobs {
    fn from(blobs: Vec<Blob>) -> Self {
        Blobs::Other(Arc::new(blobs))
    }
}

/// Decoder is generic over a Pool and handles fetching and decoding blob
/// transactions. Decoder attempts to fetch from the Pool first and then
/// queries an explorer if it can't find the blob. When Decoder does find a
/// blob, it decodes it and returns the decoded transactions.
#[derive(Debug)]
pub struct BlockExtractor<Pool: TransactionPool> {
    pool: Pool,
    explorer: foundry_blob_explorers::Client,
    client: reqwest::Client,
    cl_url: Option<url::Url>,
    pylon_url: Option<url::Url>,
    slot_calculator: SlotCalculator,
}

impl<Pool> BlockExtractor<Pool>
where
    Pool: TransactionPool,
{
    /// new returns a new `Decoder` generic over a `Pool`
    pub fn new(
        pool: Pool,
        explorer: foundry_blob_explorers::Client,
        cl_client: reqwest::Client,
        cl_url: Option<Cow<'static, str>>,
        pylon_url: Option<Cow<'static, str>>,
        slot_calculator: SlotCalculator,
    ) -> Result<Self, url::ParseError> {
        let cl_url =
            if let Some(url) = cl_url { Some(url::Url::parse(url.as_ref())?) } else { None };

        let pylon_url =
            if let Some(url) = pylon_url { Some(url::Url::parse(url.as_ref())?) } else { None };

        Ok(Self { pool, explorer, client: cl_client, cl_url, pylon_url, slot_calculator })
    }

    /// Get blobs from either the pool or the network and decode them,
    /// searching for the expected hash
    async fn get_and_decode_blobs(
        &self,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        slot: u64,
    ) -> ExtractionResult<Vec<u8>> {
        debug_assert!(extract.tx.is_eip4844(), "Transaction must be of type EIP-4844");
        let hash = extract.tx.tx_hash();
        let bz = self.fetch_blobs(extract, slot).await?;

        SimpleCoder::default()
            .decode_all(bz.as_ref())
            .ok_or_else(BlockExtractionError::blob_decode_error)?
            .into_iter()
            .find(|data| keccak256(data) == extract.block_data_hash())
            .ok_or_else(|| BlockExtractionError::block_data_not_found(*hash))
    }

    /// Fetch blobs from the local txpool, or fall back to remote sources
    async fn fetch_blobs(
        &self,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        slot: u64,
    ) -> ExtractionResult<Blobs> {
        let hash = extract.tx_hash();

        if let Ok(blobs) = self.get_blobs_from_pool(hash) {
            return Ok(blobs);
        }

        // if the pool doesn't have it, reach out to other sources
        // and return the first successful response
        select! {
            Ok(blobs) = self.get_blobs_from_explorer(hash) => {
                 Ok(blobs)
            }
            Ok(blobs) = self.get_blobs_from_cl(extract, slot) => {
                 Ok(blobs)
            }
            Ok(blobs) = self.get_blobs_from_pylon(hash) => {
                Ok(blobs)
            }
            else => {
                error!(%hash, "Blobs not available from any source");
                Err(BlockExtractionError::missing_sidecar(hash))
            }
        }
    }

    /// Return a blob from the local pool or an error
    fn get_blobs_from_pool(&self, tx: TxHash) -> ExtractionResult<Blobs> {
        self.pool
            .get_blob(tx)?
            .map(Into::into)
            .ok_or_else(|| BlockExtractionError::missing_sidecar(tx))
    }

    /// Returns the blob from the explorer
    async fn get_blobs_from_explorer(&self, tx: TxHash) -> ExtractionResult<Blobs> {
        let sidecar = self.explorer.transaction(tx).await?;
        let blobs: Blobs = sidecar.blobs.iter().map(|b| *b.data).collect();
        debug_assert!(!blobs.is_empty(), "Explorer returned no blobs");
        Ok(blobs)
    }

    /// Returns the blob from the pylon blob indexer.
    #[instrument(skip_all, err)]
    async fn get_blobs_from_pylon(&self, tx: TxHash) -> ExtractionResult<Blobs> {
        if let Some(url) = &self.pylon_url {
            let url = url.join(&format!("sidecar/{tx}")).map_err(|err| {
                BlockExtractionError::Unrecoverable(UnrecoverableBlobError::UrlParse(err))
            })?;

            let response = self.client.get(url).header("accept", "application/json").send().await?;
            response
                .json::<Arc<BlobTransactionSidecarVariant>>()
                .await
                .map(Into::into)
                .map_err(Into::into)
        } else {
            Err(BlockExtractionError::Unrecoverable(
                UnrecoverableBlobError::ConsensusClientUrlNotSet,
            ))
        }
    }

    /// Queries the connected consensus client for the blob transaction
    #[instrument(skip_all, err)]
    async fn get_blobs_from_cl(
        &self,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        slot: u64,
    ) -> ExtractionResult<Blobs> {
        if let Some(url) = &self.cl_url {
            let url = url.join(&format!("/eth/v1/beacon/blob_sidecars/{slot}")).map_err(|err| {
                BlockExtractionError::Unrecoverable(UnrecoverableBlobError::UrlParse(err))
            })?;

            let response = self.client.get(url).header("accept", "application/json").send().await?;

            let response: BeaconBlobBundle = response.json().await?;

            extract_blobs_from_bundle(response, extract)
        } else {
            Err(BlockExtractionError::Unrecoverable(
                UnrecoverableBlobError::ConsensusClientUrlNotSet,
            ))
        }
    }

    /// Get the Zenith block from the extracted event.
    /// For 4844 transactions, this fetches the transaction's blobs and decodes them.
    /// For any other type of transactions, it returns a Non4844Transaction error.
    #[tracing::instrument(skip(self, extract), fields(eip4844 = extract.is_eip4844(), tx = %extract.tx_hash(), url = self.explorer.baseurl()))]
    async fn get_signet_block(
        &self,
        extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
        host_block_number: u64,
        host_block_timestamp: u64,
    ) -> ExtractionResult<ZenithBlock> {
        if !extract.is_eip4844() {
            return Err(BlockExtractionError::non_4844_transaction());
        }

        let header = extract.ru_header(host_block_number);

        let slot = self
            .slot_calculator
            .slot_ending_at(host_block_timestamp)
            .expect("host chain has started");

        let block_data = self.get_and_decode_blobs(extract, slot as u64).await?;
        Ok(ZenithBlock::from_header_and_data(header, block_data))
    }

    /// Fetch the [`ZenithBlock`] specified by the outputs.
    ///
    /// ## Returns
    ///
    /// - `Ok(None)` - If the outputs do not contain a [`BlockSubmitted`].
    /// - `Ok(Some(block))` - If the block was successfully fetched and decoded.
    /// - `Err(err)` - If an error occurred while fetching or decoding the
    ///   block.
    pub async fn block_from_outputs(
        &self,
        outputs: &Extracts<'_, ExtractableChainShim<'_>>,
    ) -> ExtractionResult<Option<ZenithBlock>> {
        if !outputs.contains_block() {
            return Ok(None);
        }

        let tx = outputs.submitted.as_ref().expect("checked by contains_block");

        match self
            .get_signet_block(tx, outputs.host_block_number(), outputs.host_block_timestamp())
            .await
            .map(Some)
        {
            Ok(block) => Ok(block),
            Err(err) => {
                if err.is_ignorable() {
                    trace!(%err, "ignorable error in block extraction");
                    Ok(None) // ignore ignorable errors
                } else {
                    Err(err)
                }
            }
        }
    }
}

/// Extracts the blobs from the [`BeaconBlobBundle`], and returns the blobs that match the versioned hashes in the transaction.
/// This also dedups any duplicate blobs if a builder lands the same blob multiple times in a block.
fn extract_blobs_from_bundle(
    bundle: BeaconBlobBundle,
    extract: &ExtractedEvent<'_, Receipt, BlockSubmitted>,
) -> ExtractionResult<Blobs> {
    let mut blobs = vec![];
    // NB: There can be, at most, 9 blobs per block from Pectra forwards. We'll never need more space than this, unless blob capacity is increased again or made dynamic.
    let mut seen_versioned_hashes: SmallVec<[B256; 9]> = SmallVec::new();

    // NB: This is already checked and we know it's an EIP-4844 transaction.
    let tx = extract.tx.as_eip4844().unwrap();

    for item in bundle.data.iter() {
        let versioned_hash =
            alloy::eips::eip4844::kzg_to_versioned_hash(item.kzg_commitment.as_ref());

        if tx.tx().blob_versioned_hashes.contains(&versioned_hash)
            && !seen_versioned_hashes.contains(&versioned_hash)
        {
            blobs.push(*item.blob);
            seen_versioned_hashes.push(versioned_hash);
        }
    }

    Ok(blobs.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{
            BlobTransactionSidecar, SidecarBuilder, SignableTransaction, TxEip2930, TxEnvelope,
        },
        eips::Encodable2718,
        primitives::{bytes, Address, TxKind, U256},
        rlp::encode,
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use foundry_blob_explorers::TransactionDetails;
    use reth::primitives::{Transaction, TransactionSigned};
    use reth_transaction_pool::{
        test_utils::{testing_pool, MockTransaction},
        PoolTransaction, TransactionOrigin,
    };
    use signet_types::constants::SignetSystemConstants;

    const BLOBSCAN_BLOB_RESPONSE: &str = include_str!("../../../tests/artifacts/blob.json");
    /// Blob from Slot 2277733, corresponding to block 277722 on Pecorino host.
    const CL_BLOB_RESPONSE: &str = include_str!("../../../tests/artifacts/cl_blob.json");
    /// EIP4844 blob tx with hash 0x73d1c682fae85c761528a0a7ec22fac613b25ede87b80f0ac052107f3444324f,
    /// corresponding to blob sent to block 277722 on Pecorino host.
    const CL_BLOB_TX: &str = include_str!("../../../tests/artifacts/cl_blob_tx.json");
    /// Blob sidecar from Pylon, corresponding to block 277722 on Pecorino host.
    const PYLON_BLOB_RESPONSE: &str = include_str!("../../../tests/artifacts/pylon_blob.json");

    #[test]
    fn test_process_blob_extraction() {
        let bundle: BeaconBlobBundle = serde_json::from_str(CL_BLOB_RESPONSE).unwrap();
        let tx: TxEnvelope = serde_json::from_str::<TxEnvelope>(CL_BLOB_TX).unwrap();
        let tx: TransactionSigned = tx.into();

        let extract = ExtractedEvent::<'_, Receipt, BlockSubmitted> {
            tx: &tx,
            receipt: &Receipt::default(),
            log_index: 0,
            event: BlockSubmitted {
                sequencer: Address::ZERO,
                rollupChainId: U256::ZERO,
                gasLimit: U256::ZERO,
                rewardAddress: Address::ZERO,
                blockDataHash: B256::ZERO,
            },
        };

        // Extract the blobs from the CL beacon blob bundle.
        let cl_blobs = extract_blobs_from_bundle(bundle, &extract).unwrap();
        assert_eq!(cl_blobs.len(), 1);

        // Now, process the pylon blobs which come in a [`BlobTransactionSidecar`].
        // NB: this should be changes to `BlobTransactionSidecarVariant` in the
        // future. After https://github.com/alloy-rs/alloy/pull/2713
        // The json is definitely a `BlobTransactionSidecar`, so we can
        // deserialize it directly and it doesn't really matter much.
        let sidecar: BlobTransactionSidecar =
            serde_json::from_str::<BlobTransactionSidecar>(PYLON_BLOB_RESPONSE).unwrap();
        let pylon_blobs: Blobs = Arc::<BlobTransactionSidecarVariant>::new(sidecar.into()).into();

        // Make sure that both blob sources have the same blobs after being processed.
        assert_eq!(cl_blobs.len(), pylon_blobs.len());
        assert_eq!(cl_blobs.as_slice(), pylon_blobs.as_slice());

        // Make sure both can be decoded
        let cl_decoded = SimpleCoder::default().decode_all(cl_blobs.as_ref()).unwrap();
        let pylon_decoded = SimpleCoder::default().decode_all(pylon_blobs.as_ref()).unwrap();
        assert_eq!(cl_decoded.len(), pylon_decoded.len());
        assert_eq!(cl_decoded, pylon_decoded);
    }

    #[test]
    fn test_deser_blob() {
        let _: TransactionDetails = serde_json::from_str(BLOBSCAN_BLOB_RESPONSE).unwrap();
    }

    #[tokio::test]
    async fn test_fetch_from_pool() -> eyre::Result<()> {
        let wallet = PrivateKeySigner::random();
        let pool = testing_pool();

        let test = signet_constants::KnownChains::Test;

        let constants: SignetSystemConstants = test.try_into().unwrap();
        let calc = SlotCalculator::new(0, 0, 12);

        let explorer_url = Cow::Borrowed("https://api.holesky.blobscan.com/");
        let client = reqwest::Client::builder().use_rustls_tls().build().unwrap();
        let explorer =
            foundry_blob_explorers::Client::new_with_client(explorer_url.as_ref(), client.clone());

        let extractor =
            BlockExtractor::new(pool.clone(), explorer, client.clone(), None, None, calc)?;

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

        let got = extractor.get_blobs_from_pool(*mock_transaction.hash());
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
