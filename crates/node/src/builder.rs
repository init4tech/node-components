#![allow(clippy::type_complexity)]

use crate::{NodeStatus, SignetNode};
use eyre::OptionExt;
use signet_blobber::CacheHandle;
use signet_block_processor::AliasOracleFactory;
use signet_cold::BlockData;
use signet_hot::db::{HotDbRead, UnsafeDbWrite};
use signet_node_config::SignetNodeConfig;
use signet_node_types::HostNotifier;
use signet_rpc::{ServeConfig, StorageRpcConfig};
use signet_storage::{HistoryRead, HistoryWrite, HotKv, HotKvRead, UnifiedStorage};
use std::sync::Arc;
use tracing::info;
use trevm::revm::database::DBErrorMarker;

/// A type that does not implement [`AliasOracleFactory`].
#[derive(Debug, Clone, Copy)]
pub struct NotAnAof;

/// Sentinel indicating no storage has been provided.
#[derive(Debug, Clone, Copy)]
pub struct NotAStorage;

/// Builder for [`SignetNode`]. This is the main way to create a signet node.
///
/// The builder requires the following components to be set before building:
/// - A [`HostNotifier`], via [`Self::with_notifier`].
/// - An [`Arc<UnifiedStorage<H>>`], via [`Self::with_storage`].
/// - An [`AliasOracleFactory`], via [`Self::with_alias_oracle`].
/// - A [`CacheHandle`], via [`Self::with_blob_cacher`].
/// - A [`ServeConfig`], via [`Self::with_serve_config`].
/// - A [`StorageRpcConfig`], via [`Self::with_rpc_config`].
/// - A `reqwest::Client`, via [`Self::with_client`].
///   - If not set, a default client will be created.
///
/// # Examples
///
/// ```no_run
/// # use signet_node::builder::SignetNodeBuilder;
/// # fn example(
/// #     config: signet_node_config::SignetNodeConfig,
/// #     notifier: impl signet_node_types::HostNotifier,
/// #     storage: std::sync::Arc<signet_storage::UnifiedStorage<signet_hot::db::MemoryHotKv>>,
/// #     alias_oracle: impl signet_block_processor::AliasOracleFactory,
/// #     blob_cacher: signet_blobber::CacheHandle,
/// #     serve_config: signet_rpc::ServeConfig,
/// #     rpc_config: signet_rpc::StorageRpcConfig,
/// # ) {
/// let builder = SignetNodeBuilder::new(config)
///     .with_notifier(notifier)
///     .with_storage(storage)
///     .with_alias_oracle(alias_oracle)
///     .with_blob_cacher(blob_cacher)
///     .with_serve_config(serve_config)
///     .with_rpc_config(rpc_config);
/// # }
/// ```
pub struct SignetNodeBuilder<
    Notifier = (),
    Storage = NotAStorage,
    Aof = NotAnAof,
    Bc = (),
    Sc = (),
    Rc = (),
> {
    config: SignetNodeConfig,
    alias_oracle: Option<Aof>,
    notifier: Option<Notifier>,
    storage: Option<Storage>,
    client: Option<reqwest::Client>,
    blob_cacher: Option<Bc>,
    serve_config: Option<Sc>,
    rpc_config: Option<Rc>,
}

impl<Notifier, Storage, Aof, Bc, Sc, Rc> core::fmt::Debug
    for SignetNodeBuilder<Notifier, Storage, Aof, Bc, Sc, Rc>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignetNodeBuilder").finish_non_exhaustive()
    }
}

impl SignetNodeBuilder {
    /// Create a new SignetNodeBuilder instance.
    pub const fn new(config: SignetNodeConfig) -> Self {
        Self {
            config,
            alias_oracle: None,
            notifier: None,
            storage: None,
            client: None,
            blob_cacher: None,
            serve_config: None,
            rpc_config: None,
        }
    }
}

impl<Notifier, Storage, Aof, Bc, Sc, Rc> SignetNodeBuilder<Notifier, Storage, Aof, Bc, Sc, Rc> {
    /// Set the [`UnifiedStorage`] backend for the signet node.
    pub fn with_storage<H: HotKv>(
        self,
        storage: Arc<UnifiedStorage<H>>,
    ) -> SignetNodeBuilder<Notifier, Arc<UnifiedStorage<H>>, Aof, Bc, Sc, Rc> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            notifier: self.notifier,
            storage: Some(storage),
            client: self.client,
            blob_cacher: self.blob_cacher,
            serve_config: self.serve_config,
            rpc_config: self.rpc_config,
        }
    }

    /// Set the [`HostNotifier`] for the signet node.
    pub fn with_notifier<N: HostNotifier>(
        self,
        notifier: N,
    ) -> SignetNodeBuilder<N, Storage, Aof, Bc, Sc, Rc> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            notifier: Some(notifier),
            storage: self.storage,
            client: self.client,
            blob_cacher: self.blob_cacher,
            serve_config: self.serve_config,
            rpc_config: self.rpc_config,
        }
    }

    /// Set the [`AliasOracleFactory`] for the signet node.
    pub fn with_alias_oracle<NewAof: AliasOracleFactory>(
        self,
        alias_oracle: NewAof,
    ) -> SignetNodeBuilder<Notifier, Storage, NewAof, Bc, Sc, Rc> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: Some(alias_oracle),
            notifier: self.notifier,
            storage: self.storage,
            client: self.client,
            blob_cacher: self.blob_cacher,
            serve_config: self.serve_config,
            rpc_config: self.rpc_config,
        }
    }

    /// Set the reqwest client for the signet node.
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Set the pre-built blob cacher handle.
    pub fn with_blob_cacher(
        self,
        blob_cacher: CacheHandle,
    ) -> SignetNodeBuilder<Notifier, Storage, Aof, CacheHandle, Sc, Rc> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            notifier: self.notifier,
            storage: self.storage,
            client: self.client,
            blob_cacher: Some(blob_cacher),
            serve_config: self.serve_config,
            rpc_config: self.rpc_config,
        }
    }

    /// Set the RPC transport configuration.
    pub fn with_serve_config(
        self,
        serve_config: ServeConfig,
    ) -> SignetNodeBuilder<Notifier, Storage, Aof, Bc, ServeConfig, Rc> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            notifier: self.notifier,
            storage: self.storage,
            client: self.client,
            blob_cacher: self.blob_cacher,
            serve_config: Some(serve_config),
            rpc_config: self.rpc_config,
        }
    }

    /// Set the RPC behaviour configuration.
    pub fn with_rpc_config(
        self,
        rpc_config: StorageRpcConfig,
    ) -> SignetNodeBuilder<Notifier, Storage, Aof, Bc, Sc, StorageRpcConfig> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            notifier: self.notifier,
            storage: self.storage,
            client: self.client,
            blob_cacher: self.blob_cacher,
            serve_config: self.serve_config,
            rpc_config: Some(rpc_config),
        }
    }
}

impl<N, H, Aof>
    SignetNodeBuilder<N, Arc<UnifiedStorage<H>>, Aof, CacheHandle, ServeConfig, StorageRpcConfig>
where
    N: HostNotifier,
    H: HotKv + Clone + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    Aof: AliasOracleFactory,
{
    /// Prebuild checks for the signet node builder. Shared by all build
    /// commands.
    async fn prebuild(&mut self) -> eyre::Result<()> {
        self.client.get_or_insert_default();
        self.notifier.as_ref().ok_or_eyre("Notifier must be set")?;
        let storage = self.storage.as_ref().ok_or_eyre("Storage must be set")?;

        // Load genesis into hot storage if absent.
        let reader = storage.reader()?;
        let has_hot_genesis = reader.has_block(0)?;
        drop(reader);

        if !has_hot_genesis {
            let genesis = self.config.genesis();
            let hardforks = signet_genesis::genesis_hardforks(genesis);
            let writer = storage.hot().writer()?;
            writer.load_genesis(genesis, &hardforks)?;
            writer.commit()?;
            info!("loaded genesis into hot storage");
        }

        // Load genesis into cold storage if absent. Hot genesis may have
        // been loaded externally (e.g. test harness with custom allocs),
        // so we check cold independently.
        let has_cold_genesis = storage.cold_reader().get_latest_block().await?.is_some();
        if !has_cold_genesis {
            let reader = storage.reader()?;
            let genesis_header =
                reader.get_header(0)?.ok_or_eyre("genesis header missing from hot storage")?;
            drop(reader);
            let genesis_block = BlockData::new(genesis_header, vec![], vec![], vec![], None);
            storage.cold().append_block(genesis_block).await?;
            info!("loaded genesis into cold storage");
        }

        Ok(())
    }

    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits storage from genesis if needed.
    pub async fn build(
        mut self,
    ) -> eyre::Result<(SignetNode<N, H, Aof>, tokio::sync::watch::Receiver<NodeStatus>)> {
        self.prebuild().await?;
        SignetNode::new_unsafe(
            self.notifier.unwrap(),
            self.config,
            self.storage.unwrap(),
            self.alias_oracle.unwrap(),
            self.client.unwrap(),
            self.blob_cacher.unwrap(),
            self.serve_config.unwrap(),
            self.rpc_config.unwrap(),
        )
    }
}
