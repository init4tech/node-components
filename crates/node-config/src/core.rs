use alloy::genesis::Genesis;
use init4_bin_base::utils::{calc::SlotCalculator, from_env::FromEnv};
use reth::providers::providers::StaticFileProvider;
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_node_api::NodePrimitives;
use signet_blobber::BlobFetcherConfig;
use signet_genesis::GenesisSpec;
use signet_types::constants::{ConfigError, SignetSystemConstants};
use std::{
    borrow::Cow,
    fmt::Display,
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use tracing::warn;
use trevm::revm::primitives::hardfork::SpecId;

/// Defines the default port for serving Signet Node JSON RPC requests over http.
pub const SIGNET_NODE_DEFAULT_HTTP_PORT: u16 = 5959u16;

/// Configuration for a Signet Node instance. Contains system contract and signer
/// information.
#[derive(Debug, Clone, serde::Deserialize, FromEnv)]
#[serde(rename_all = "camelCase")]
pub struct SignetNodeConfig {
    /// Configuration for the block extractor.
    #[from_env(infallible)]
    block_extractor: BlobFetcherConfig,

    /// Path to the static files for reth StaticFileProviders.
    #[from_env(var = "SIGNET_STATIC_PATH", desc = "Path to the static files", infallible)]
    static_path: Cow<'static, str>,
    /// Path to the MDBX database.
    #[from_env(var = "SIGNET_DATABASE_PATH", desc = "Path to the MDBX database", infallbile)]
    database_path: Cow<'static, str>,
    /// URL to which to forward raw transactions.
    #[from_env(
        var = "TX_FORWARD_URL",
        desc = "URL to which to forward raw transactions",
        infallible,
        optional
    )]
    forward_url: Option<Cow<'static, str>>,
    /// RPC port to serve JSON-RPC requests
    #[from_env(var = "RPC_PORT", desc = "RPC port to serve JSON-RPC requests", optional)]
    http_port: Option<u16>,
    /// Websocket port to serve JSON-RPC requests
    #[from_env(var = "WS_RPC_PORT", desc = "Websocket port to serve JSON-RPC requests", optional)]
    ws_port: Option<u16>,
    /// IPC endpoint to serve JSON-RPC requests
    #[from_env(
        var = "IPC_ENDPOINT",
        desc = "IPC endpoint to serve JSON-RPC requests",
        infallible,
        optional
    )]
    ipc_endpoint: Option<Cow<'static, str>>,

    /// Configuration loaded from genesis file, or known genesis.
    genesis: GenesisSpec,

    /// The slot calculator.
    slot_calculator: SlotCalculator,
}

impl Display for SignetNodeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignetNodeConfig").finish_non_exhaustive()
    }
}

impl SignetNodeConfig {
    /// Create a new Signet Node configuration.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        block_extractor: BlobFetcherConfig,
        static_path: Cow<'static, str>,
        database_path: Cow<'static, str>,
        forward_url: Option<Cow<'static, str>>,
        rpc_port: u16,
        ws_port: u16,
        ipc_endpoint: Option<Cow<'static, str>>,
        genesis: GenesisSpec,
        slot_calculator: SlotCalculator,
    ) -> Self {
        Self {
            block_extractor,
            static_path,
            database_path,
            forward_url,
            http_port: Some(rpc_port),
            ws_port: Some(ws_port),
            ipc_endpoint,
            genesis,
            slot_calculator,
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

    /// Get the static path as a str.
    pub fn static_path_str(&self) -> &str {
        &self.static_path
    }

    /// Get the static path.
    pub fn static_path(&self) -> PathBuf {
        self.static_path.as_ref().to_owned().into()
    }

    /// Get the static file provider for read-only access.
    pub fn static_file_ro<N: NodePrimitives>(&self) -> eyre::Result<StaticFileProvider<N>> {
        StaticFileProvider::read_only(self.static_path(), true).map_err(Into::into)
    }

    /// Get the static file provider for read-write access.
    pub fn static_file_rw<N: NodePrimitives>(&self) -> eyre::Result<StaticFileProvider<N>> {
        StaticFileProvider::read_write(self.static_path()).map_err(Into::into)
    }

    /// Get the database path as a str.
    pub fn database_path_str(&self) -> &str {
        &self.database_path
    }

    /// Get the database path.
    pub fn database_path(&self) -> PathBuf {
        self.database_path.as_ref().to_owned().into()
    }

    /// Get the URL to which to forward raw transactions.
    pub fn forward_url(&self) -> Option<reqwest::Url> {
        self.forward_url
            .as_deref()
            .map(reqwest::Url::parse)?
            .inspect_err(|e| warn!(%e, "failed to parse forward URL"))
            .ok()
    }

    /// Returns the port for serving JSON RPC requests for Signet Node.
    pub const fn http_port(&self) -> u16 {
        if let Some(port) = self.http_port {
            return port;
        }
        SIGNET_NODE_DEFAULT_HTTP_PORT
    }

    /// Set the HTTP port for serving JSON RPC requests for Signet Node.
    pub const fn set_http_port(&mut self, port: u16) {
        self.http_port = Some(port);
    }

    /// Returns the port for serving Websocket RPC requests for Signet Node.
    pub const fn ws_port(&self) -> u16 {
        if let Some(port) = self.ws_port {
            return port;
        }
        SIGNET_NODE_DEFAULT_HTTP_PORT + 1
    }

    /// Set the websocket port for serving JSON RPC requests for Signet.
    pub const fn set_ws_port(&mut self, port: u16) {
        self.ws_port = Some(port);
    }

    /// Returns the IPC endpoint for serving JSON RPC requests for Signet, if any.
    pub fn ipc_endpoint(&self) -> Option<&str> {
        self.ipc_endpoint.as_deref()
    }

    /// Set the IPC endpoint for serving JSON RPC requests for Signet Node.
    pub fn set_ipc_endpoint(&mut self, endpoint: Cow<'static, str>) {
        self.ipc_endpoint = Some(endpoint);
    }

    /// Returns the genesis configuration if any has been loaded
    pub fn genesis(&self) -> &'static Genesis {
        static ONCE: OnceLock<Genesis> = OnceLock::new();
        ONCE.get_or_init(|| self.genesis.load_genesis().expect("Failed to load genesis"))
    }

    /// Create a new chain spec for the Signet Node chain.
    pub fn chain_spec(&self) -> &Arc<ChainSpec> {
        static SPEC: OnceLock<Arc<ChainSpec>> = OnceLock::new();
        SPEC.get_or_init(|| Arc::new(self.genesis().clone().into()))
    }

    /// Get the system constants for the Signet Node chain.
    pub fn constants(&self) -> Result<SignetSystemConstants, ConfigError> {
        SignetSystemConstants::try_from_genesis(self.genesis())
    }

    /// Get the current spec id for the Signet Node chain.
    pub fn spec_id(&self, timestamp: u64) -> SpecId {
        if self.chain_spec().is_prague_active_at_timestamp(timestamp) {
            SpecId::PRAGUE
        } else {
            SpecId::CANCUN
        }
    }
}

#[cfg(test)]
mod defaults {
    use super::*;

    impl Default for SignetNodeConfig {
        fn default() -> Self {
            Self {
                block_extractor: BlobFetcherConfig::new(Cow::Borrowed("")),
                static_path: Cow::Borrowed(""),
                database_path: Cow::Borrowed(""),
                forward_url: None,
                http_port: Some(SIGNET_NODE_DEFAULT_HTTP_PORT),
                ws_port: Some(SIGNET_NODE_DEFAULT_HTTP_PORT + 1),
                ipc_endpoint: None,
                genesis: GenesisSpec::Test,

                slot_calculator: SlotCalculator::new(0, 0, 12),
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
