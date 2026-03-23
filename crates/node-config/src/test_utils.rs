use crate::{SignetNodeConfig, StorageConfig};
use init4_bin_base::utils::calc::SlotCalculator;
use signet_blobber::BlobFetcherConfig;
use signet_genesis::GenesisSpec;
use signet_types::constants::KnownChains;
use std::borrow::Cow;

/// Make a test config
pub const fn test_config() -> SignetNodeConfig {
    TEST_CONFIG
}

/// Test SignetNodeConfig
const TEST_CONFIG: SignetNodeConfig = SignetNodeConfig::new(
    BlobFetcherConfig::new(Cow::Borrowed("")),
    StorageConfig::new(Cow::Borrowed("NOP"), Cow::Borrowed("NOP")),
    None,
    GenesisSpec::Known(KnownChains::Test),
    SlotCalculator::new(0, 0, 12),
);
