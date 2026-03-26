#![allow(clippy::type_complexity)]

use crate::{NodeStatus, SignetNode};
use eyre::OptionExt;
use signet_blobber::CacheHandle;
use signet_block_processor::AliasOracleFactory;
use signet_cold::BlockData;
use signet_hot::{
    db::{HotDbRead, UnsafeDbWrite},
    model::HotKvWrite,
    tables,
};
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
/// # use signet_node::SignetNodeBuilder;
/// # fn example<H: signet_storage::HotKv>(
/// #     config: signet_node_config::SignetNodeConfig,
/// #     notifier: impl signet_node_types::HostNotifier,
/// #     storage: std::sync::Arc<signet_storage::UnifiedStorage<H>>,
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
pub struct SignetNodeBuilder<Notifier = (), Storage = NotAStorage, Aof = NotAnAof> {
    config: SignetNodeConfig,
    alias_oracle: Option<Aof>,
    notifier: Option<Notifier>,
    storage: Option<Storage>,
    client: Option<reqwest::Client>,
    blob_cacher: Option<CacheHandle>,
    serve_config: Option<ServeConfig>,
    rpc_config: Option<StorageRpcConfig>,
}

impl<Notifier, Storage, Aof> core::fmt::Debug for SignetNodeBuilder<Notifier, Storage, Aof> {
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

impl<Notifier, Storage, Aof> SignetNodeBuilder<Notifier, Storage, Aof> {
    /// Set the [`UnifiedStorage`] backend for the signet node.
    pub fn with_storage<H: HotKv>(
        self,
        storage: Arc<UnifiedStorage<H>>,
    ) -> SignetNodeBuilder<Notifier, Arc<UnifiedStorage<H>>, Aof> {
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
    pub fn with_notifier<N: HostNotifier>(self, notifier: N) -> SignetNodeBuilder<N, Storage, Aof> {
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
    ) -> SignetNodeBuilder<Notifier, Storage, NewAof> {
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
    pub fn with_blob_cacher(mut self, blob_cacher: CacheHandle) -> Self {
        self.blob_cacher = Some(blob_cacher);
        self
    }

    /// Set the RPC transport configuration.
    pub fn with_serve_config(mut self, serve_config: ServeConfig) -> Self {
        self.serve_config = Some(serve_config);
        self
    }

    /// Set the RPC behaviour configuration.
    pub const fn with_rpc_config(mut self, rpc_config: StorageRpcConfig) -> Self {
        self.rpc_config = Some(rpc_config);
        self
    }
}

impl<N, H, Aof> SignetNodeBuilder<N, Arc<UnifiedStorage<H>>, Aof>
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

        // Ensure all hot storage tables exist. On a fresh MDBX database,
        // named tables must be created in a write transaction before
        // read-only transactions can open them.
        {
            let writer = storage.hot().writer()?;
            writer.queue_create::<tables::Headers>()?;
            writer.queue_create::<tables::HeaderNumbers>()?;
            writer.queue_create::<tables::Bytecodes>()?;
            writer.queue_create::<tables::PlainAccountState>()?;
            writer.queue_create::<tables::PlainStorageState>()?;
            writer.queue_create::<tables::AccountsHistory>()?;
            writer.queue_create::<tables::AccountChangeSets>()?;
            writer.queue_create::<tables::StorageHistory>()?;
            writer.queue_create::<tables::StorageChangeSets>()?;
            writer.raw_commit()?;
        }

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
        // NB: Notifier, Storage, and Aof are enforced by typestate generics.
        // The remaining fields are set via `Option` and checked at runtime.
        SignetNode::new_unsafe(
            self.notifier.expect("enforced by typestate"),
            self.config,
            self.storage.expect("enforced by typestate"),
            self.alias_oracle.expect("enforced by typestate"),
            self.client.expect("set by prebuild"),
            self.blob_cacher.ok_or_eyre("blob cacher must be set")?,
            self.serve_config.ok_or_eyre("serve config must be set")?,
            self.rpc_config.ok_or_eyre("rpc config must be set")?,
        )
    }
}
