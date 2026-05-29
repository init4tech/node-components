use crate::{JournalConfig, SignetNodeConfig, StorageConfig};
use init4_bin_base::utils::calc::SlotCalculator;
use signet_blobber::BlobFetcherConfig;
use signet_genesis::GenesisSpec;
use signet_types::constants::KnownChains;
use std::borrow::Cow;

/// Make a test config.
pub fn test_config() -> SignetNodeConfig {
    test_config_with_journal(JournalConfig::default())
}

/// Make a test config with a caller-supplied [`JournalConfig`]. Used by journal-sync tests to
/// point the node at an upstream WebSocket source.
pub const fn test_config_with_journal(journal: JournalConfig) -> SignetNodeConfig {
    SignetNodeConfig::new(
        BlobFetcherConfig::new(Cow::Borrowed("")),
        StorageConfig::new(Cow::Borrowed("NOP"), Cow::Borrowed("NOP")),
        None,
        journal,
        GenesisSpec::Known(KnownChains::Test),
        SlotCalculator::new(0, 0, 12),
    )
}
