#![allow(clippy::type_complexity)]

use crate::{GENESIS_JOURNAL_HASH, SignetNode};
use eyre::OptionExt;
use reth::{
    primitives::EthPrimitives,
    providers::{BlockHashReader, ProviderFactory, StateProviderFactory},
};
use reth_db::transaction::DbTxMut;
use reth_db_common::init;
use reth_exex::ExExContext;
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_block_processor::AliasOracleFactory;
use signet_db::DbProviderExt;
use signet_node_config::SignetNodeConfig;
use signet_node_types::{NodeStatus, NodeTypesDbTrait, SignetNodeTypes};

/// A type that does not implement [`AliasOracleFactory`].
#[derive(Debug, Clone, Copy)]
pub struct NotAnAof;

/// A type that does not implement [`NodeTypesDbTrait`].
#[derive(Debug, Clone, Copy)]
pub struct NotADb;

/// Builder for [`SignetNode`]. This is the main way to create a signet node.
///
/// The builder requires the following components to be set before building:
/// - An [`ExExContext`], via [`Self::with_ctx`].
/// - A [`ProviderFactory`] for the signet node's database.
///     - This can be provided directly via [`Self::with_factory`].
///     - Or created from a database implementing [`NodeTypesDbTrait`] via
///       [`Self::with_db`].
///     - If not set directly, can be created from the config via
///       [`Self::with_config_db`].
/// - An [`AliasOracleFactory`], via [`Self::with_alias_oracle`].
///     - If not set, a default one will be created from the [`ExExContext`]'s
///       provider.
/// - A `reqwest::Client`, via [`Self::with_client`].
///   - If not set, a default client will be created.
pub struct SignetNodeBuilder<Host = (), Db = NotADb, Aof = NotAnAof> {
    config: SignetNodeConfig,
    alias_oracle: Option<Aof>,
    ctx: Option<Host>,
    factory: Option<Db>,
    client: Option<reqwest::Client>,
}

impl<Host, Db, Aof> core::fmt::Debug for SignetNodeBuilder<Host, Db, Aof> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignetNodeBuilder").finish_non_exhaustive()
    }
}

impl SignetNodeBuilder {
    /// Create a new SignetNodeBuilder instance.
    pub const fn new(config: SignetNodeConfig) -> Self {
        Self { config, alias_oracle: None, ctx: None, factory: None, client: None }
    }
}

impl<Host, Db, Aof> SignetNodeBuilder<Host, Db, Aof> {
    /// Set the DB for the signet node.
    pub fn with_db<NewDb: NodeTypesDbTrait>(
        self,
        db: NewDb,
    ) -> eyre::Result<SignetNodeBuilder<Host, ProviderFactory<SignetNodeTypes<NewDb>>, Aof>> {
        let runtime =
            reth::tasks::Runtime::with_existing_handle(tokio::runtime::Handle::current())?;
        let factory = ProviderFactory::new(
            db,
            self.config.chain_spec().clone(),
            self.config.static_file_rw()?,
            self.config.open_rocks_db()?,
            runtime,
        )?;

        Ok(SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: self.ctx,
            factory: Some(factory),
            client: self.client,
        })
    }

    /// Set the DB for the signet node from config, opening the mdbx database.
    pub fn with_config_db(
        self,
    ) -> eyre::Result<
        SignetNodeBuilder<Host, ProviderFactory<SignetNodeTypes<reth_db::DatabaseEnv>>, Aof>,
    > {
        let runtime =
            reth::tasks::Runtime::with_existing_handle(tokio::runtime::Handle::current())?;
        let factory = ProviderFactory::new_with_database_path(
            self.config.database_path(),
            self.config.chain_spec().clone(),
            reth_db::mdbx::DatabaseArguments::default(),
            self.config.static_file_rw()?,
            self.config.open_rocks_db()?,
            runtime,
        )?;
        Ok(SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: self.ctx,
            factory: Some(factory),
            client: self.client,
        })
    }

    /// Set the provider factory for the signet node.
    ///
    /// This is an alternative to [`Self::with_db`] and
    /// [`Self::with_config_db`].
    pub fn with_factory<NewDb>(
        self,
        factory: ProviderFactory<SignetNodeTypes<NewDb>>,
    ) -> SignetNodeBuilder<Host, ProviderFactory<SignetNodeTypes<NewDb>>, Aof>
    where
        NewDb: NodeTypesDbTrait,
    {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: self.ctx,
            factory: Some(factory),
            client: self.client,
        }
    }

    /// Set the [`ExExContext`] for the signet node.
    pub fn with_ctx<NewHost>(
        self,
        ctx: ExExContext<NewHost>,
    ) -> SignetNodeBuilder<ExExContext<NewHost>, Db, Aof>
    where
        NewHost: FullNodeComponents,
        NewHost::Types: NodeTypes<Primitives = EthPrimitives>,
    {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: self.alias_oracle,
            ctx: Some(ctx),
            factory: self.factory,
            client: self.client,
        }
    }

    /// Set the [`AliasOracleFactory`] for the signet node.
    pub fn with_alias_oracle<NewAof: AliasOracleFactory>(
        self,
        alias_oracle: NewAof,
    ) -> SignetNodeBuilder<Host, Db, NewAof> {
        SignetNodeBuilder {
            config: self.config,
            alias_oracle: Some(alias_oracle),
            ctx: self.ctx,
            factory: self.factory,
            client: self.client,
        }
    }

    /// Set the reqwest client for the signet node.
    pub fn with_client(mut self, client: reqwest::Client) -> SignetNodeBuilder<Host, Db, Aof> {
        self.client = Some(client);
        self
    }
}

impl<Host, Db, Aof> SignetNodeBuilder<ExExContext<Host>, ProviderFactory<SignetNodeTypes<Db>>, Aof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
{
    /// Prebuild checks for the signet node builder. Shared by all build
    /// commands.
    fn prebuild(&mut self) -> eyre::Result<()> {
        self.client.get_or_insert_default();
        self.ctx.as_ref().ok_or_eyre("Launch context must be set")?;
        let factory = self.factory.as_ref().ok_or_eyre("Provider factory must be set")?;

        // This check appears redundant with the same check made in
        // `init_genesis`, but is not. We init the genesis DB state but then we
        // drop some of it, and reuse those tables for our own nefarious
        // purposes. If we attempt to drop those tables AFTER we have reused
        // them, we will get a key deser error (as the tables will contain keys
        // the old schema does not permit). This check ensures we only attempt
        // to drop the tables once.
        if matches!(
            factory.block_hash(0),
            Ok(None)
                | Err(reth::providers::ProviderError::MissingStaticFileBlock(
                    reth::primitives::StaticFileSegment::Headers,
                    0
                ))
        ) {
            init::init_genesis(factory)?;

            factory.provider_rw()?.update(
                |writer: &mut reth::providers::DatabaseProviderRW<Db, SignetNodeTypes<Db>>| {
                    writer.tx_mut().clear::<reth_db::tables::HashedAccounts>()?;
                    writer.tx_mut().clear::<reth_db::tables::HashedStorages>()?;
                    writer.tx_mut().clear::<reth_db::tables::AccountsTrie>()?;

                    writer.tx_ref().put::<signet_db::JournalHashes>(0, GENESIS_JOURNAL_HASH)?;
                    // we do not need to pre-populate the `ZenithHeaders` or
                    // `SignetEvents` tables, as missing data is legal in those
                    // tables

                    Ok(())
                },
            )?;
        }

        Ok(())
    }
}

impl<Host> SignetNodeBuilder<ExExContext<Host>, NotADb, NotAnAof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits the rollup DB from genesis if needed.
    /// - Creates a default `AliasOracleFactory` from the host DB.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        self,
    ) -> eyre::Result<(
        SignetNode<Host, reth_db::DatabaseEnv, Box<dyn StateProviderFactory>>,
        tokio::sync::watch::Receiver<NodeStatus>,
    )> {
        self.with_config_db()?.build()
    }
}

impl<Host, Aof> SignetNodeBuilder<ExExContext<Host>, NotADb, Aof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Aof: AliasOracleFactory,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits the rollup DB from genesis if needed.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        self,
    ) -> eyre::Result<(
        SignetNode<Host, reth_db::DatabaseEnv, Aof>,
        tokio::sync::watch::Receiver<NodeStatus>,
    )> {
        self.with_config_db()?.build()
    }
}

impl<Host, Db> SignetNodeBuilder<ExExContext<Host>, ProviderFactory<SignetNodeTypes<Db>>, NotAnAof>
where
    Host: FullNodeComponents<Provider: StateProviderFactory>,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits the rollup DB from genesis if needed.
    /// - Creates a default `AliasOracleFactory` from the host DB.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        mut self,
    ) -> eyre::Result<(SignetNode<Host, Db>, tokio::sync::watch::Receiver<NodeStatus>)> {
        self.prebuild()?;
        // This allows the node to look up contract status.
        let ctx = self.ctx.unwrap();
        let provider = ctx.provider().clone();
        let alias_oracle: Box<dyn StateProviderFactory> = Box::new(provider);

        SignetNode::new_unsafe(
            ctx,
            self.config,
            self.factory.unwrap(),
            alias_oracle,
            self.client.unwrap(),
        )
    }
}

impl<Host, Db, Aof> SignetNodeBuilder<ExExContext<Host>, ProviderFactory<SignetNodeTypes<Db>>, Aof>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
    Aof: AliasOracleFactory,
{
    /// Build the node. This performs the following steps:
    ///
    /// - Runs prebuild checks.
    /// - Inits the rollup DB from genesis if needed.
    ///
    /// # Panics
    ///
    /// If called outside a tokio runtime.
    pub fn build(
        mut self,
    ) -> eyre::Result<(SignetNode<Host, Db, Aof>, tokio::sync::watch::Receiver<NodeStatus>)> {
        self.prebuild()?;
        SignetNode::new_unsafe(
            self.ctx.unwrap(),
            self.config,
            self.factory.unwrap(),
            self.alias_oracle.unwrap(),
            self.client.unwrap(),
        )
    }
}
