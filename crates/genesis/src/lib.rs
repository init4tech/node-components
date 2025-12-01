#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

use alloy::genesis::Genesis;
use init4_bin_base::utils::from_env::{
    EnvItemInfo, FromEnv, FromEnvErr, FromEnvVar, parse_env_if_present,
};
use signet_constants::KnownChains;
use std::{borrow::Cow, path::PathBuf, str::FromStr, sync::LazyLock};

/// Signet mainnet genesis file.
pub const MAINNET_GENESIS_JSON: &str = include_str!("./mainnet.genesis.json");

/// Pecorino genesis file.
pub const PECORINO_GENESIS_JSON: &str = include_str!("./pecorino.genesis.json");

/// Pecorino host genesis file.
pub const PECORINO_HOST_GENESIS_JSON: &str = include_str!("./pecorino.host.genesis.json");

/// Local genesis file for testing purposes.
pub const TEST_GENESIS_JSON: &str = include_str!("./local.genesis.json");

/// Mainnet genesis for the Signet mainnet.
pub static MAINNET_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(MAINNET_GENESIS_JSON).expect("Failed to parse mainnet genesis")
});

/// Genesis for the Pecorino testnet.
pub static PECORINO_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(PECORINO_GENESIS_JSON).expect("Failed to parse pecorino genesis")
});

/// Genesis for the Pecorino host testnet.
pub static PECORINO_HOST_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(PECORINO_HOST_GENESIS_JSON).expect("Failed to parse pecorino host genesis")
});

/// Test genesis for local testing.
pub static TEST_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(TEST_GENESIS_JSON).expect("Failed to parse test genesis")
});

/// Environment variable for specifying the genesis JSON file path.
const GENESIS_JSON_PATH: &str = "GENESIS_JSON_PATH";

/// Result type for genesis operations.
pub type Result<T, E = GenesisError> = std::result::Result<T, E>;

/// Errors that can occur when loading the genesis file.
#[derive(Debug, thiserror::Error)]
pub enum GenesisError {
    /// IO error when reading the genesis file.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON parsing error when parsing the genesis file.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Different genesis configurations available.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum GenesisSpec {
    /// Signet mainnet.
    Mainnet,
    /// Pecorino testnet.
    Pecorino,
    /// Local testnet.
    Test,
    /// Custom path to a genesis file.
    Path(PathBuf),
}

impl GenesisSpec {
    /// Load the genesis JSON from the specified source.
    ///
    /// This will alwys return a valid string for [`KnownChains`].
    pub fn load_raw_genesis(&self) -> Result<Cow<'static, str>> {
        match self {
            GenesisSpec::Mainnet => Ok(Cow::Borrowed(MAINNET_GENESIS_JSON)),
            GenesisSpec::Pecorino => Ok(Cow::Borrowed(PECORINO_GENESIS_JSON)),
            GenesisSpec::Test => Ok(Cow::Borrowed(TEST_GENESIS_JSON)),
            GenesisSpec::Path(path) => {
                std::fs::read_to_string(path).map(Cow::Owned).map_err(Into::into)
            }
        }
    }

    /// Load the genesis from the specified source.
    ///
    /// This will always return a valid genesis for [`KnownChains`].
    pub fn load_genesis(&self) -> Result<alloy::genesis::Genesis> {
        match self {
            GenesisSpec::Mainnet => Ok(MAINNET_GENESIS.clone()),
            GenesisSpec::Pecorino => Ok(PECORINO_GENESIS.clone()),
            GenesisSpec::Test => Ok(TEST_GENESIS.clone()),
            GenesisSpec::Path(_) => self
                .load_raw_genesis()
                .and_then(|raw| serde_json::from_str(&raw).map_err(Into::into)),
        }
    }
}

impl FromStr for GenesisSpec {
    type Err = <PathBuf as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(known) = KnownChains::from_str(s) {
            return Ok(known.into());
        }

        Ok(GenesisSpec::Path(s.parse()?))
    }
}

impl FromEnvVar for GenesisSpec {
    type Error = <GenesisSpec as FromStr>::Err;

    fn from_env_var(env_var: &str) -> Result<Self, FromEnvErr<Self::Error>> {
        parse_env_if_present(env_var)
    }
}

impl FromEnv for GenesisSpec {
    type Error = <GenesisSpec as FromStr>::Err;

    fn inventory() -> Vec<&'static init4_bin_base::utils::from_env::EnvItemInfo> {
        vec![
            &EnvItemInfo {
                var: "CHAIN_NAME",
                description: "The name of the chain. If set, the other environment variables are ignored.",
                optional: true,
            },
            &EnvItemInfo {
                var: GENESIS_JSON_PATH,
                description: "A filepath to the genesis JSON file. Required if CHAIN_NAME is not set.",
                optional: true,
            },
        ]
    }

    fn from_env() -> Result<Self, FromEnvErr<Self::Error>> {
        parse_env_if_present::<KnownChains>("CHAIN_NAME")
            .map(Into::into)
            .or_else(|_| parse_env_if_present::<PathBuf>(GENESIS_JSON_PATH).map(Into::into))
    }
}

impl From<KnownChains> for GenesisSpec {
    fn from(known: KnownChains) -> Self {
        match known {
            KnownChains::Pecorino => GenesisSpec::Pecorino,
            KnownChains::Test => GenesisSpec::Test,
        }
    }
}

impl From<PathBuf> for GenesisSpec {
    fn from(path: PathBuf) -> Self {
        GenesisSpec::Path(path)
    }
}
