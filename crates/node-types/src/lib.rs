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

mod utils;
pub use utils::{NodeTypesDbTrait, Pnt};

use reth::{
    primitives::EthPrimitives,
    providers::{
        CanonStateNotification, CanonStateNotificationSender, CanonStateNotifications,
        CanonStateSubscriptions, EthStorage, NodePrimitivesProvider,
    },
};
use reth_chainspec::ChainSpec;
use reth_node_api::{NodePrimitives, NodeTypes, NodeTypesWithDB};
use reth_node_ethereum::EthEngineTypes;
use std::marker::PhantomData;
use tokio::sync::broadcast::error::SendError;

/// Items that can be sent via the status channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Node is booting.
    Booting,
    /// Node's current height.
    AtHeight(u64),
}

/// Signet node types for [`NodeTypes`] and [`NodeTypesWithDB`].
#[derive(Copy, Debug)]
pub struct SignetNodeTypes<Db> {
    _db: PhantomData<fn() -> Db>,
}

impl<Db> Clone for SignetNodeTypes<Db> {
    fn clone(&self) -> Self {
        Self { _db: PhantomData }
    }
}

impl<Db> PartialEq for SignetNodeTypes<Db> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<Db> Eq for SignetNodeTypes<Db> {}

impl<Db> Default for SignetNodeTypes<Db> {
    fn default() -> Self {
        Self { _db: PhantomData }
    }
}

impl<Db> NodePrimitives for SignetNodeTypes<Db>
where
    Db: NodeTypesDbTrait,
{
    type Block = <reth::primitives::EthPrimitives as NodePrimitives>::Block;
    type BlockHeader = <reth::primitives::EthPrimitives as NodePrimitives>::BlockHeader;
    /// Block body primitive.
    type BlockBody = <reth::primitives::EthPrimitives as NodePrimitives>::BlockBody;
    /// Signed version of the transaction type.
    type SignedTx = <reth::primitives::EthPrimitives as NodePrimitives>::SignedTx;
    /// A receipt.
    type Receipt = <reth::primitives::EthPrimitives as NodePrimitives>::Receipt;
}

impl<Db> NodeTypes for SignetNodeTypes<Db>
where
    Db: NodeTypesDbTrait,
{
    type Primitives = EthPrimitives;

    type ChainSpec = ChainSpec;

    type Storage = EthStorage;

    type Payload = EthEngineTypes;
}

impl<Db> NodeTypesWithDB for SignetNodeTypes<Db>
where
    Db: NodeTypesDbTrait,
{
    type DB = Db;
}

/// Shim to impl [`CanonStateSubscriptions`]
#[derive(Debug, Clone)]
pub struct SharedCanonState<Db> {
    sender: CanonStateNotificationSender,
    _pd: PhantomData<fn() -> Db>,
}

impl<Db> SharedCanonState<Db>
where
    Db: NodeTypesDbTrait,
{
    /// Get the number of receivers, via [`CanonStateNotificationSender::receiver_count`].
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Send a notification to all subscribers.
    pub fn send(
        &self,
        notification: CanonStateNotification,
    ) -> Result<usize, SendError<CanonStateNotification>> {
        self.sender.send(notification)
    }
}

impl<Db> Default for SharedCanonState<Db>
where
    Db: NodeTypesDbTrait,
{
    fn default() -> Self {
        // magic constant matches reth behavior in blockchain_tree.
        // Max reorg depth is default 64, blockchain tree doubles it to 128.
        Self::new(128)
    }
}

impl<Db> NodePrimitivesProvider for SharedCanonState<Db>
where
    Db: NodeTypesDbTrait,
{
    type Primitives = EthPrimitives;
}

impl<Db> SharedCanonState<Db>
where
    Db: NodeTypesDbTrait,
{
    /// Create a new shared canon state.
    pub fn new(capacity: usize) -> Self {
        Self { sender: tokio::sync::broadcast::channel(capacity).0, _pd: PhantomData }
    }
}

impl<Db> CanonStateSubscriptions for SharedCanonState<Db>
where
    Db: NodeTypesDbTrait,
{
    fn subscribe_to_canonical_state(&self) -> CanonStateNotifications<Self::Primitives> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[allow(dead_code)]
    fn compile_check() {
        fn inner<P: Pnt>() {}

        inner::<SignetNodeTypes<std::sync::Arc<reth_db::mdbx::DatabaseEnv>>>();
    }
}
