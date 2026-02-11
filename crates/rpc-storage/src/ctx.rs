//! RPC context wrapping [`UnifiedStorage`].

use crate::resolve::BlockTags;
use signet_cold::ColdStorageReadHandle;
use signet_hot::HotKv;
use signet_hot::model::{HotKvRead, RevmRead};
use signet_storage::UnifiedStorage;
use signet_tx_cache::TxCache;
use signet_types::constants::SignetSystemConstants;
use std::sync::Arc;
use trevm::revm::database::DBErrorMarker;
use trevm::revm::database::StateBuilder;

/// RPC context backed by [`UnifiedStorage`].
///
/// Provides access to hot storage (state), cold storage (blocks/txs/receipts),
/// block tag resolution, and optional transaction forwarding.
///
/// # Construction
///
/// ```ignore
/// let ctx = StorageRpcCtx::new(storage, constants, tags, Some(tx_cache), 30_000_000);
/// ```
#[derive(Debug)]
pub struct StorageRpcCtx<H: HotKv> {
    inner: Arc<StorageRpcCtxInner<H>>,
}

impl<H: HotKv> Clone for StorageRpcCtx<H> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}

#[derive(Debug)]
struct StorageRpcCtxInner<H: HotKv> {
    storage: UnifiedStorage<H>,
    constants: SignetSystemConstants,
    tags: BlockTags,
    tx_cache: Option<TxCache>,
    rpc_gas_cap: u64,
}

impl<H: HotKv> StorageRpcCtx<H> {
    /// Create a new storage-backed RPC context.
    pub fn new(
        storage: UnifiedStorage<H>,
        constants: SignetSystemConstants,
        tags: BlockTags,
        tx_cache: Option<TxCache>,
        rpc_gas_cap: u64,
    ) -> Self {
        Self {
            inner: Arc::new(StorageRpcCtxInner { storage, constants, tags, tx_cache, rpc_gas_cap }),
        }
    }

    /// Access the unified storage.
    pub fn storage(&self) -> &UnifiedStorage<H> {
        &self.inner.storage
    }

    /// Get a cold storage read handle.
    pub fn cold(&self) -> ColdStorageReadHandle {
        self.inner.storage.cold_reader()
    }

    /// Get a hot storage read transaction.
    pub fn hot_reader(&self) -> signet_storage::StorageResult<H::RoTx> {
        self.inner.storage.reader()
    }

    /// Access the block tags.
    pub fn tags(&self) -> &BlockTags {
        &self.inner.tags
    }

    /// Access the system constants.
    pub fn constants(&self) -> &SignetSystemConstants {
        &self.inner.constants
    }

    /// Get the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.inner.constants.ru_chain_id()
    }

    /// Get the RPC gas cap.
    pub fn rpc_gas_cap(&self) -> u64 {
        self.inner.rpc_gas_cap
    }

    /// Access the optional tx cache.
    pub fn tx_cache(&self) -> Option<&TxCache> {
        self.inner.tx_cache.as_ref()
    }

    /// Create a revm-compatible database at a specific block height.
    ///
    /// The returned `State<RevmRead<...>>` implements both `Database` and
    /// `DatabaseCommit`, making it suitable for use with `signet_evm`.
    pub fn revm_state_at_height(
        &self,
        height: u64,
    ) -> signet_storage::StorageResult<trevm::revm::database::State<RevmRead<H::RoTx>>>
    where
        H::RoTx: Send + Sync,
        <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    {
        let revm_read = self.inner.storage.revm_reader_at_height(height)?;
        Ok(StateBuilder::new_with_database(revm_read).build())
    }

    /// Create a revm-compatible database at the current tip.
    pub fn revm_state(
        &self,
    ) -> signet_storage::StorageResult<trevm::revm::database::State<RevmRead<H::RoTx>>>
    where
        H::RoTx: Send + Sync,
        <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    {
        let revm_read = self.inner.storage.revm_reader()?;
        Ok(StateBuilder::new_with_database(revm_read).build())
    }
}
