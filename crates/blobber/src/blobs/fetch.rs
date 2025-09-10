use crate::{BlobFetcherBuilder, FetchError, FetchResult, utils::extract_blobs_from_bundle};
use alloy::{
    consensus::{Blob, BlobTransactionSidecar},
    eips::eip7594::{BlobTransactionSidecarEip7594, BlobTransactionSidecarVariant},
    primitives::{B256, TxHash},
};
use reth::{rpc::types::beacon::sidecar::BeaconBlobBundle, transaction_pool::TransactionPool};
use std::{ops::Deref, sync::Arc};
use tokio::select;
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

/// Decoder is generic over a Pool and handles fetching and decoding blob
/// transactions. Decoder attempts to fetch from the Pool first and then
/// queries an explorer if it can't find the blob. When Decoder does find a
/// blob, it decodes it and returns the decoded transactions.
pub struct BlobFetcher<Pool> {
    pool: Pool,
    explorer: foundry_blob_explorers::Client,
    client: reqwest::Client,
    cl_url: Option<url::Url>,
    pylon_url: Option<url::Url>,
}

impl<Pool: core::fmt::Debug> core::fmt::Debug for BlobFetcher<Pool> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobFetcher")
            .field("pool", &self.pool)
            .field("explorer", &self.explorer.baseurl())
            .field("cl_url", &self.cl_url)
            .field("pylon_url", &self.pylon_url)
            .finish_non_exhaustive()
    }
}

impl BlobFetcher<()> {
    /// Returns a new [`BlobFetcherBuilder`].
    pub fn builder() -> BlobFetcherBuilder<()> {
        BlobFetcherBuilder::default()
    }
}

impl<Pool> BlobFetcher<Pool>
where
    Pool: TransactionPool,
{
    /// new returns a new `Decoder` generic over a `Pool`
    pub const fn new(
        pool: Pool,
        explorer: foundry_blob_explorers::Client,
        cl_client: reqwest::Client,
        cl_url: Option<url::Url>,
        pylon_url: Option<url::Url>,
    ) -> Self {
        Self { pool, explorer, client: cl_client, cl_url, pylon_url }
    }

    /// Fetch blobs from the local txpool, or fall back to remote sources
    #[instrument(skip(self))]
    pub(crate) async fn fetch_blobs(
        &self,
        slot: usize,
        tx_hash: B256,
        versioned_hashes: &[B256],
    ) -> FetchResult<Blobs> {
        if let Ok(blobs) = self.get_blobs_from_pool(tx_hash) {
            return Ok(blobs);
        }

        // if the pool doesn't have it, reach out to other sources
        // and return the first successful response
        select! {
            Ok(blobs) = self.get_blobs_from_explorer(tx_hash) => {
                 Ok(blobs)
            }
            Ok(blobs) = self.get_blobs_from_cl(slot, versioned_hashes) => {
                 Ok(blobs)
            }
            Ok(blobs) = self.get_blobs_from_pylon(tx_hash) => {
                Ok(blobs)
            }
            else => {
                Err(FetchError::MissingSidecar(tx_hash))
            }
        }
    }

    /// Return a blob from the local pool or an error
    fn get_blobs_from_pool(&self, tx: TxHash) -> FetchResult<Blobs> {
        self.pool.get_blob(tx)?.map(Into::into).ok_or_else(|| FetchError::MissingSidecar(tx))
    }

    /// Returns the blob from the explorer
    async fn get_blobs_from_explorer(&self, tx: TxHash) -> FetchResult<Blobs> {
        let sidecar = self.explorer.transaction(tx).await?;
        let blobs: Blobs = sidecar.blobs.iter().map(|b| *b.data).collect();
        debug_assert!(!blobs.is_empty(), "Explorer returned no blobs");
        Ok(blobs)
    }

    /// Returns the blob from the pylon blob indexer.
    #[instrument(skip_all)]
    async fn get_blobs_from_pylon(&self, tx: TxHash) -> FetchResult<Blobs> {
        let Some(url) = &self.pylon_url else {
            return Err(FetchError::ConsensusClientUrlNotSet);
        };
        let url = url.join(&format!("sidecar/{tx}"))?;

        let response = self.client.get(url).header("accept", "application/json").send().await?;
        response
            .json::<Arc<BlobTransactionSidecarVariant>>()
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Queries the connected consensus client for the blob transaction
    #[instrument(skip_all)]
    async fn get_blobs_from_cl(
        &self,
        slot: usize,
        versioned_hashes: &[B256],
    ) -> FetchResult<Blobs> {
        let Some(url) = &self.cl_url else {
            return Err(FetchError::ConsensusClientUrlNotSet);
        };

        let url = url
            .join(&format!("/eth/v1/beacon/blob_sidecars/{slot}"))
            .map_err(FetchError::UrlParse)?;

        let response = self.client.get(url).header("accept", "application/json").send().await?;

        let response: BeaconBlobBundle = response.json().await?;

        extract_blobs_from_bundle(response, versioned_hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{SidecarBuilder, SignableTransaction as _, SimpleCoder, TxEip2930},
        eips::Encodable2718,
        primitives::{TxKind, U256, bytes},
        rlp::encode,
        signers::{SignerSync, local::PrivateKeySigner},
    };
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

        let explorer_url = "https://api.holesky.blobscan.com/";
        let client = reqwest::Client::builder().use_rustls_tls();

        let extractor = BlobFetcher::builder()
            .with_pool(pool.clone())
            .with_explorer_url(explorer_url)
            .with_client_builder(client)
            .unwrap()
            .build()?;

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
