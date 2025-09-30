use crate::SignetNodeConfig;
use init4_bin_base::utils::calc::SlotCalculator;
use reth_db::test_utils::tempdir_path;
use signet_blobber::BlobFetcherConfig;
use signet_genesis::GenesisSpec;
use std::borrow::Cow;

/// Make a test config
pub fn test_config() -> SignetNodeConfig {
    let mut tempdir = tempdir_path();
    tempdir.push("signet.ipc");

    // Make a new test config with the IPC endpoint set to the tempdir.
    let mut cfg = TEST_CONFIG;
    cfg.set_ipc_endpoint(Cow::Owned(format!("{}", tempdir.to_string_lossy())));
    cfg
}

/// Test SignetNodeConfig
const TEST_CONFIG: SignetNodeConfig = SignetNodeConfig::new(
    BlobFetcherConfig::new(Cow::Borrowed("")),
    Cow::Borrowed("NOP"),
    Cow::Borrowed("NOP"),
    None,
    31391, // NOP
    31392, // NOP
    Some(Cow::Borrowed("/trethNOP")),
    GenesisSpec::Test,
    SlotCalculator::new(0, 0, 12),
);
