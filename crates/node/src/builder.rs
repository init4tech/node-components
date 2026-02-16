#![allow(clippy::type_complexity)]

use crate::{NodeStatus, SignetNode};
use eyre::OptionExt;
use reth::{primitives::EthPrimitives, providers::StateProviderFactory};
use reth_exex::ExExContext;
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_block_processor::AliasOracleFactory;
use signet_hot::db::UnsafeDbWrite;
use signet_node_config::SignetNodeConfig;
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
/// - An [`ExExContext`], via [`Self::with_ctx`].
/// - An [`Arc<UnifiedStorage<H>>`], via [`Self::with_storage`].
/// - An [`AliasOracleFactory`], via [`Self::with_alias_oracle`].
///     - If not set, a default one will be created from the [`ExExContext`]'s
///       provider.
/// - A `reqwest::Client`, via [`Self::with_client`].
///   - If not set, a default client will be created.
pub struct SignetNodeBuilder<Host = (), Storage = NotAStorage, Aof = NotAnAof> {
    config: SignetNodeConfig,
    alias_oracle: Option<Aof>,
    ctx: Option<Host>,
    storage: Option<Storage>,
    client: Option<reqwest::Client>,
}

impl<Host, Storage, Aof> core::fmt::Debug for SignetNodeBuilder<Host, Storage, Aof> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignetNodeBuilder").finish_non_exhaustive()
    }
}

impl SignetNodeBuilder {
    /// Create a new SignetNodeBuilder instance.
    pub const fn new(config: SignetNodeConfig) -> Self {
        Self { config, alias_oracle: None, ctx: None, storage: None, client: None }
    }
}

impl<Host, Storage, Aof> SignetNodeBuilder<Host, Storage, Aof> {
    /// Set the [`UnifiedStorage`] backend for the signet node.
    pub fn with_storage<H: HotKv>(
        self,
        storage: Arc<UnifiedStorage<H>>,
    ) -> SignetNodeBuilder<Host, Arc<UnifiedStorage<H>>, Aof> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: self.ctx,
            storage: Some(storage),
            client: self.client,
        }
    }

    /// Set the [`ExExContext`] for the signet node.
    pub fn with_ctx<NewHost>(
        self,
        ctx: ExExContext<NewHost>,
    ) -> SignetNodeBuilder<ExExContext<NewHost>, Storage, Aof>
    where
        NewHost: FullNodeComponents,
        NewHost::Types: NodeTypes<Primitives = EthPrimitives>,
    {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: Some(ctx),
            storage: self.storage,
            client: self.client,
        }
    }

    /// Set the [`AliasOracleFactory`] for the signet node.
    pub fn with_alias_oracle<NewAof: AliasOracleFactory>(
        self,
        alias_oracle: NewAof,
    ) -> SignetNodeBuilder<Host, Storage, NewAof> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: Some(alias_oracle),
            ctx: self.ctx,
            storage: self.storage,
            client: self.client,
        }
    }

    /// Set the reqwest client for the signet node.
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = Some(client);
        self
    }
}

impl<Host, H, Aof> SignetNodeBuilder<ExExContext<Host>, Arc<UnifiedStorage<H>>, Aof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv,
{
    /// Prebuild checks for the signet node builder. Shared by all build
    /// commands.
    fn prebuild(&mut self) -> eyre::Result<()> {
        self.client.get_or_insert_default();
        self.ctx.as_ref().ok_or_eyre("Launch context must be set")?;
        let storage = self.storage.as_ref().ok_or_eyre("Storage must be set")?;

        // Check if genesis is loaded
        let reader = storage.reader()?;
        let has_genesis = HistoryRead::has_block(&reader, 0)?;
        drop(reader);

        if !has_genesis {
            let genesis = self.config.genesis();
            let hardforks = signet_genesis::genesis_hardforks(genesis);
            let writer = storage.hot().writer()?;
            writer.load_genesis(genesis, &hardforks)?;
            writer.commit()?;
            info!("loaded genesis into hot storage");
        }

        Ok(())
    }
}

impl<Host, H> SignetNodeBuilder<ExExContext<Host>, Arc<UnifiedStorage<H>>, NotAnAof>
where
    Host: FullNodeComponents<Provider: StateProviderFactory>,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv + Clone + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits storage from genesis if needed.
    /// - Creates a default `AliasOracleFactory` from the host DB.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        mut self,
    ) -> eyre::Result<(SignetNode<Host, H>, tokio::sync::watch::Receiver<NodeStatus>)> {
        self.prebuild()?;
        let ctx = self.ctx.unwrap();
        let provider = ctx.provider().clone();
        let alias_oracle: Box<dyn StateProviderFactory> = Box::new(provider);

        SignetNode::new_unsafe(
            ctx,
            self.config,
            self.storage.unwrap(),
            alias_oracle,
            self.client.unwrap(),
        )
    }
}

impl<Host, H, Aof> SignetNodeBuilder<ExExContext<Host>, Arc<UnifiedStorage<H>>, Aof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv + Clone + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    Aof: AliasOracleFactory,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits storage from genesis if needed.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        mut self,
    ) -> eyre::Result<(SignetNode<Host, H, Aof>, tokio::sync::watch::Receiver<NodeStatus>)> {
        self.prebuild()?;
        SignetNode::new_unsafe(
            self.ctx.unwrap(),
            self.config,
            self.storage.unwrap(),
            self.alias_oracle.unwrap(),
            self.client.unwrap(),
        )
    }
}
