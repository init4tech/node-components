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

/// Genesis for the Parmigiana testnet.
pub static PARMIGIANA_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(include_str!("./parmigiana.genesis.json"))
        .expect("Failed to parse parmigiana genesis")
});

/// Genesis for the Parmigiana host testnet.
pub static PARMIGIANA_HOST_GENESIS: LazyLock<Genesis> = LazyLock::new(|| {
    serde_json::from_str(include_str!("./parmigiana.host.genesis.json"))
        .expect("Failed to parse parmigiana host genesis")
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
    pub rollup: Cow<'static, Genesis>,
    /// The host genesis configuration.
    pub host: Cow<'static, Genesis>,
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
    /// Known chain genesis configurations.
    Known(#[serde(deserialize_with = "known_from_str")] KnownChains),
    /// Custom paths to genesis files.
    Custom {
        /// Path to the rollup genesis file.
        rollup: PathBuf,
        /// Path to the host genesis file.
        host: PathBuf,
    },
}

fn known_from_str<'de, D>(deserializer: D) -> std::result::Result<KnownChains, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = <String as serde::Deserialize>::deserialize(deserializer)?;
    KnownChains::from_str(&s).map_err(serde::de::Error::custom)
}

impl GenesisSpec {
    /// Load the raw genesis JSON strings from the specified source.
    ///
    /// Returns both rollup and host genesis JSON strings.
    pub fn load_raw_genesis(&self) -> Result<RawNetworkGenesis> {
        match self {
            GenesisSpec::Known(KnownChains::Mainnet) => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(MAINNET_GENESIS_JSON),
                host: Cow::Borrowed(MAINNET_HOST_GENESIS_JSON),
            }),
            GenesisSpec::Known(KnownChains::Parmigiana) => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(include_str!("./parmigiana.genesis.json")),
                host: Cow::Borrowed(include_str!("./parmigiana.host.genesis.json")),
            }),
            #[allow(deprecated)]
            GenesisSpec::Known(KnownChains::Pecorino) => Ok(RawNetworkGenesis {
                rollup: Cow::Borrowed(PECORINO_GENESIS_JSON),
                host: Cow::Borrowed(PECORINO_HOST_GENESIS_JSON),
            }),
            GenesisSpec::Known(KnownChains::Test) => Ok(RawNetworkGenesis {
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
            GenesisSpec::Known(KnownChains::Mainnet) => Ok(NetworkGenesis {
                rollup: Cow::Borrowed(&*MAINNET_GENESIS),
                host: Cow::Borrowed(&*MAINNET_HOST_GENESIS),
            }),
            GenesisSpec::Known(KnownChains::Parmigiana) => Ok(NetworkGenesis {
                rollup: Cow::Borrowed(&*PARMIGIANA_GENESIS),
                host: Cow::Borrowed(&*PARMIGIANA_HOST_GENESIS),
            }),
            #[allow(deprecated)]
            GenesisSpec::Known(KnownChains::Pecorino) => Ok(NetworkGenesis {
                rollup: Cow::Borrowed(&*PECORINO_GENESIS),
                host: Cow::Borrowed(&*PECORINO_HOST_GENESIS),
            }),
            GenesisSpec::Known(KnownChains::Test) => Ok(NetworkGenesis {
                rollup: Cow::Borrowed(&*TEST_GENESIS),
                host: Cow::Borrowed(&*TEST_HOST_GENESIS),
            }),
            GenesisSpec::Custom { .. } => self.load_raw_genesis().and_then(|genesis| {
                Ok(NetworkGenesis {
                    rollup: Cow::Owned(serde_json::from_str(&genesis.rollup)?),
                    host: Cow::Owned(serde_json::from_str(&genesis.host)?),
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
    fn from_env_var(env_var: &str) -> Result<Self, FromEnvErr> {
        parse_env_if_present(env_var)
    }
}

impl FromEnv for GenesisSpec {
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

    fn from_env() -> Result<Self, FromEnvErr> {
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
        Self::Known(known)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signet_constants::SignetSystemConstants;

    #[test]
    fn load_files() {
        for key in [
            KnownChains::Mainnet,
            KnownChains::Parmigiana,
            #[allow(deprecated)]
            KnownChains::Pecorino,
            KnownChains::Test,
        ] {
            let genesis =
                GenesisSpec::from(key).load_genesis().expect("Failed to load genesis").rollup;
            SignetSystemConstants::try_from_genesis(&genesis).unwrap();
        }
    }
}
