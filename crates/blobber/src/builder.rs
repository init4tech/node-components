use crate::{BlockExtractorConfig, block_data::BlockExtractor};
use init4_bin_base::utils::calc::SlotCalculator;
use reth::transaction_pool::TransactionPool;
use url::Url;

/// Errors that can occur while building the [`BlockExtractor`] with a
/// [`BlockExtractorBuilder`].
#[derive(Debug, thiserror::Error)]
pub enum BuilderError {
    /// The transaction pool was not provided.
    #[error("transaction pool is required")]
    MissingPool,
    /// The explorer URL was not provided or could not be parsed.
    #[error("explorer URL is required and must be valid")]
    MissingExplorerUrl,
    /// The URL provided was invalid.
    #[error("invalid URL provided")]
    Url(#[from] url::ParseError),
    /// The client was not provided.
    #[error("client is required")]
    MissingClient,
    /// The client failed to build.
    #[error("failed to build client: {0}")]
    Client(#[from] reqwest::Error),
    /// The slot calculator was not provided.
    #[error("slot calculator is required")]
    MissingSlotCalculator,
}

/// Builder for the [`BlockExtractor`].
#[derive(Debug, Default, Clone)]
pub struct BlockExtractorBuilder<Pool> {
    pool: Option<Pool>,
    explorer_url: Option<String>,
    client: Option<reqwest::Client>,
    cl_url: Option<String>,
    pylon_url: Option<String>,
    slot_calculator: Option<SlotCalculator>,
}

impl<Pool> BlockExtractorBuilder<Pool> {
    /// Set the transaction pool to use for the extractor.
    pub fn with_pool<P2>(self, pool: P2) -> BlockExtractorBuilder<P2> {
        BlockExtractorBuilder {
            pool: Some(pool),
            explorer_url: self.explorer_url,
            client: self.client,
            cl_url: self.cl_url,
            pylon_url: self.pylon_url,
            slot_calculator: self.slot_calculator,
        }
    }

    /// Set the transaction pool to use a mock test pool.
    #[cfg(feature = "test-utils")]
    pub fn with_test_pool(
        self,
    ) -> BlockExtractorBuilder<reth_transaction_pool::test_utils::TestPool> {
        self.with_pool(reth_transaction_pool::test_utils::testing_pool())
    }

    /// Set the configuration for the CL url, pylon url, from the provided
    /// [`BlockExtractorConfig`].
    pub fn with_config(self, config: &BlockExtractorConfig) -> Result<Self, BuilderError> {
        let this = self.with_explorer_url(config.blob_explorer_url());
        let this =
            if let Some(cl_url) = config.cl_url() { this.with_cl_url(cl_url)? } else { this };

        if let Some(pylon_url) = config.pylon_url() {
            this.with_pylon_url(pylon_url)
        } else {
            Ok(this)
        }
    }

    /// Set the blob explorer URL to use for the extractor. This will be used
    /// to construct a [`foundry_blob_explorers::Client`].
    pub fn with_explorer_url(mut self, explorer_url: &str) -> Self {
        self.explorer_url = Some(explorer_url.to_string());
        self
    }

    /// Set the [`reqwest::Client`] to use for the extractor. This client will
    /// be used to make requests to the blob explorer, and the CL and Pylon URLs
    /// if provided.
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Set the [`reqwest::Client`] via a [reqwest::ClientBuilder]. This
    /// function will immediately build the client and return an error if it
    /// fails.
    ///
    /// This client will be used to make requests to the blob explorer, and the
    /// CL and Pylon URLs if provided.
    pub fn with_client_builder(self, client: reqwest::ClientBuilder) -> Result<Self, BuilderError> {
        client.build().map(|client| self.with_client(client)).map_err(Into::into)
    }

    /// Set the CL URL to use for the extractor.
    pub fn with_cl_url(mut self, cl_url: &str) -> Result<Self, BuilderError> {
        self.cl_url = Some(cl_url.to_string());
        Ok(self)
    }

    /// Set the Pylon URL to use for the extractor.
    pub fn with_pylon_url(mut self, pylon_url: &str) -> Result<Self, BuilderError> {
        self.pylon_url = Some(pylon_url.to_string());
        Ok(self)
    }

    /// Set the slot calculator to use for the extractor.
    pub const fn with_slot_calculator(
        mut self,
        slot_calculator: SlotCalculator,
    ) -> BlockExtractorBuilder<Pool> {
        self.slot_calculator = Some(slot_calculator);
        self
    }

    /// Set the slot calculator to use for the extractor, using the Pecornino
    /// host configuration.
    pub const fn with_pecornino_slots(mut self) -> BlockExtractorBuilder<Pool> {
        self.slot_calculator = Some(SlotCalculator::pecorino_host());
        self
    }
}

impl<Pool: TransactionPool> BlockExtractorBuilder<Pool> {
    /// Build the [`BlockExtractor`] with the provided parameters.
    pub fn build(self) -> Result<BlockExtractor<Pool>, BuilderError> {
        let pool = self.pool.ok_or(BuilderError::MissingPool)?;

        let explorer_url = self.explorer_url.ok_or(BuilderError::MissingExplorerUrl)?;

        let cl_url = self.cl_url.map(parse_url).transpose()?;

        let pylon_url = self.pylon_url.map(parse_url).transpose()?;

        let client = self.client.ok_or(BuilderError::MissingClient)?;

        let explorer =
            foundry_blob_explorers::Client::new_with_client(explorer_url, client.clone());

        let slot_calculator = self.slot_calculator.ok_or(BuilderError::MissingSlotCalculator)?;

        Ok(BlockExtractor::new(pool, explorer, client, cl_url, pylon_url, slot_calculator))
    }
}

fn parse_url(url: String) -> Result<Url, BuilderError> {
    Url::parse(url.as_ref()).map_err(BuilderError::Url)
}
