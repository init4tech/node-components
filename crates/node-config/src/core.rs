use crate::StorageConfig;
use alloy::genesis::Genesis;
use init4_bin_base::utils::{calc::SlotCalculator, from_env::FromEnv};
use signet_blobber::BlobFetcherConfig;
use signet_genesis::GenesisSpec;
use signet_types::constants::{ConfigError, SignetSystemConstants};
use std::{borrow::Cow, fmt::Display, sync::OnceLock};
use tracing::warn;

/// Configuration for a Signet Node instance. Contains system contract and signer
/// information.
#[derive(Debug, Clone, serde::Deserialize, FromEnv)]
#[serde(rename_all = "camelCase")]
pub struct SignetNodeConfig {
    /// Configuration for the block extractor.
    #[from_env(infallible)]
    block_extractor: BlobFetcherConfig,

    /// Unified storage configuration (hot + cold MDBX paths).
    #[from_env(infallible)]
    storage: StorageConfig,
    /// URL to which to forward raw transactions.
    #[from_env(
        var = "TX_FORWARD_URL",
        desc = "URL to which to forward raw transactions",
        infallible,
        optional
    )]
    forward_url: Option<Cow<'static, str>>,

    /// Configuration loaded from genesis file, or known genesis.
    genesis: GenesisSpec,

    /// The slot calculator.
    slot_calculator: SlotCalculator,

    /// Maximum number of blocks to process per backfill batch.
    /// Lower values reduce memory usage during sync. Default is 10,000
    /// (reth's default of 500K can cause OOM on mainnet).
    #[from_env(
        var = "BACKFILL_MAX_BLOCKS",
        desc = "Maximum blocks per backfill batch (lower = less memory)",
        optional
    )]
    backfill_max_blocks: Option<u64>,
}

impl Display for SignetNodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignetNodeConfig").finish_non_exhaustive()
    }
}

impl SignetNodeConfig {
    /// Create a new Signet Node configuration.
    pub const fn new(
        block_extractor: BlobFetcherConfig,
        storage: StorageConfig,
        forward_url: Option<Cow<'static, str>>,
        genesis: GenesisSpec,
        slot_calculator: SlotCalculator,
    ) -> Self {
        Self {
            block_extractor,
            storage,
            forward_url,
            genesis,
            slot_calculator,
            backfill_max_blocks: None,
        }
    }

    /// Get the blob explorer URL.
    pub fn blob_explorer_url(&self) -> &str {
        self.block_extractor.blob_explorer_url()
    }

    /// Get the block extractor configuration.
    pub const fn block_extractor(&self) -> &BlobFetcherConfig {
        &self.block_extractor
    }

    /// Get the consensus layer URL.
    pub fn cl_url(&self) -> Option<&str> {
        self.block_extractor.cl_url()
    }

    /// Get the pylon URL
    pub fn pylon_url(&self) -> Option<&str> {
        self.block_extractor.pylon_url()
    }

    /// Get a [`SlotCalculator`]
    pub const fn slot_calculator(&self) -> SlotCalculator {
        self.slot_calculator
    }

    /// Get the storage configuration.
    pub const fn storage(&self) -> &StorageConfig {
        &self.storage
    }

    /// Get the URL to which to forward raw transactions.
    pub fn forward_url(&self) -> Option<reqwest::Url> {
        self.forward_url
            .as_deref()
            .map(reqwest::Url::parse)?
            .inspect_err(|e| warn!(%e, "failed to parse forward URL"))
            .ok()
    }

    /// Returns the rollup genesis configuration if any has been loaded.
    pub fn genesis(&self) -> &'static Genesis {
        static ONCE: OnceLock<Cow<'static, Genesis>> = OnceLock::new();
        ONCE.get_or_init(|| self.genesis.load_genesis().expect("Failed to load genesis").rollup)
    }

    /// Get the system constants for the Signet Node chain.
    pub fn constants(&self) -> Result<SignetSystemConstants, ConfigError> {
        SignetSystemConstants::try_from_genesis(self.genesis())
    }

    /// Get the maximum number of blocks to process per backfill batch.
    /// Returns `Some(10_000)` by default if not configured, to avoid OOM
    /// during mainnet sync (reth's default of 500K is too aggressive).
    pub fn backfill_max_blocks(&self) -> Option<u64> {
        // Default to 10,000 if not explicitly configured
        Some(self.backfill_max_blocks.unwrap_or(10_000))
    }
}

#[cfg(test)]
mod defaults {
    use super::*;
    use signet_types::constants::KnownChains;

    impl Default for SignetNodeConfig {
        fn default() -> Self {
            Self {
                block_extractor: BlobFetcherConfig::new(Cow::Borrowed("")),
                storage: StorageConfig::new(Cow::Borrowed(""), Cow::Borrowed("")),
                forward_url: None,
                genesis: GenesisSpec::Known(KnownChains::Test),
                slot_calculator: SlotCalculator::new(0, 0, 12),
                backfill_max_blocks: None,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn loads_genesis() {
        let config = SignetNodeConfig::default();
        let genesis = config.genesis();
        assert_eq!(genesis.gas_limit, 0x1c9c380);
        assert_eq!(genesis.number, None);
        assert_eq!(genesis.base_fee_per_gas, Some(0x3b9aca00));
        assert!(genesis.config.extra_fields.get("signetConstants").is_some());
    }
}
