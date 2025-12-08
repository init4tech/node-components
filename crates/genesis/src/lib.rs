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

/// Signet mainnet host genesis file.
pub const MAINNET_HOST_GENESIS_JSON: &str = include_str!("./mainnet.host.genesis.json");

/// Pecorino genesis file.
pub const PECORINO_GENESIS_JSON: &str = include_str!("./pecorino.genesis.json");

/// Pecorino host genesis file.
pub const PECORINO_HOST_GENESIS_JSON: &str = include_str!("./pecorino.host.genesis.json");

/// Local genesis file for testing purposes.
pub const TEST_GENESIS_JSON: &str = include_str!("./local.genesis.json");

/// Local host genesis file for testing purposes.
pub const TEST_HOST_GENESIS_JSON: &str = include_str!("./local.host.genesis.json");

/// Mainnet genesis for the Signet mainnet.
pub static MAINNET_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(MAINNET_GENESIS_JSON).expect("Failed to parse mainnet genesis")
});

/// Signet mainnet host genesis for the Signet mainnet.
pub static MAINNET_HOST_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(MAINNET_HOST_GENESIS_JSON).expect("Failed to parse mainnet host genesis")
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

/// Test host genesis for local testing.
pub static TEST_HOST_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(TEST_HOST_GENESIS_JSON).expect("Failed to parse test host genesis")
});

/// Environment variable for specifying the rollup genesis JSON file path.
const ROLLUP_GENESIS_JSON_PATH: &str = "ROLLUP_GENESIS_JSON_PATH";

/// Environment variable for specifying the host genesis JSON file path.
const HOST_GENESIS_JSON_PATH: &str = "HOST_GENESIS_JSON_PATH";

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

/// Genesis configurations for a network, containing both rollup and host chain genesis.
#[derive(Debug, Clone)]
pub struct NetworkGenesis {
    /// The rollup genesis configuration.
    pub rollup: Genesis,
    /// The host genesis configuration.
    pub host: Genesis,
}

/// Raw genesis JSON strings for a network.
#[derive(Debug, Clone)]
pub struct RawNetworkGenesis {
    /// The rollup genesis JSON.
    pub rollup: Cow<'static, str>,
    /// The host genesis JSON.
    pub host: Cow<'static, str>,
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
    /// Custom paths to genesis files.
    Custom {
        /// Path to the rollup genesis file.
        rollup: PathBuf,
        /// Path to the host genesis file.
        host: PathBuf,
    },
}

impl GenesisSpec {
    /// Load the raw genesis JSON strings from the specified source.
    ///
    /// Returns both rollup and host genesis JSON strings.
    pub fn load_raw_genesis(&self) -> Result<RawNetworkGenesis> {
        match self {
            GenesisSpec::Mainnet => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(MAINNET_GENESIS_JSON),
                host: Cow::Borrowed(MAINNET_HOST_GENESIS_JSON),
            }),
            GenesisSpec::Pecorino => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(PECORINO_GENESIS_JSON),
                host: Cow::Borrowed(PECORINO_HOST_GENESIS_JSON),
            }),
            GenesisSpec::Test => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(TEST_GENESIS_JSON),
                host: Cow::Borrowed(TEST_HOST_GENESIS_JSON),
            }),
            GenesisSpec::Custom { rollup, host } => Ok(RawNetworkGenesis {
                rollup: Cow::Owned(std::fs::read_to_string(rollup)?),
                host: Cow::Owned(std::fs::read_to_string(host)?),
            }),
        }
    }

    /// Load the genesis configurations from the specified source.
    ///
    /// Returns both rollup and host genesis configurations.
    pub fn load_genesis(&self) -> Result<NetworkGenesis> {
        match self {
            GenesisSpec::Mainnet => Ok(NetworkGenesis {
                rollup: MAINNET_GENESIS.clone(),
                host: MAINNET_HOST_GENESIS.clone(),
            }),
            GenesisSpec::Pecorino => Ok(NetworkGenesis {
                rollup: PECORINO_GENESIS.clone(),
                host: PECORINO_HOST_GENESIS.clone(),
            }),
            GenesisSpec::Test => {
                Ok(NetworkGenesis { rollup: TEST_GENESIS.clone(), host: TEST_HOST_GENESIS.clone() })
            }
            GenesisSpec::Custom { .. } => self.load_raw_genesis().and_then(|genesis| {
                Ok(NetworkGenesis {
                    rollup: serde_json::from_str(&genesis.rollup)?,
                    host: serde_json::from_str(&genesis.host)?,
                })
            }),
        }
    }
}

/// Error returned when parsing an unknown chain name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown chain name: {0}")]
pub struct UnknownChainError(String);

impl FromStr for GenesisSpec {
    type Err = UnknownChainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(known) = KnownChains::from_str(s) {
            return Ok(known.into());
        }

        Err(UnknownChainError(s.to_string()))
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
                var: ROLLUP_GENESIS_JSON_PATH,
                description: "A filepath to the rollup genesis JSON file. Required if CHAIN_NAME is not set.",
                optional: true,
            },
            &EnvItemInfo {
                var: HOST_GENESIS_JSON_PATH,
                description: "A filepath to the host genesis JSON file. Required if CHAIN_NAME is not set.",
                optional: true,
            },
        ]
    }

    fn from_env() -> Result<Self, FromEnvErr<Self::Error>> {
        // First try to parse from CHAIN_NAME
        if let Ok(spec) = parse_env_if_present::<KnownChains>("CHAIN_NAME").map(Into::into) {
            return Ok(spec);
        }

        // Otherwise, try to load from custom paths
        let rollup = parse_env_if_present::<PathBuf>(ROLLUP_GENESIS_JSON_PATH)
            .map_err(|_| FromEnvErr::empty(ROLLUP_GENESIS_JSON_PATH))?;
        let host = parse_env_if_present::<PathBuf>(HOST_GENESIS_JSON_PATH)
            .map_err(|_| FromEnvErr::empty(HOST_GENESIS_JSON_PATH))?;

        Ok(GenesisSpec::Custom { rollup, host })
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
