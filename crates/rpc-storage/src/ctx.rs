//! RPC context wrapping [`UnifiedStorage`].

use crate::{
    EthError, StorageRpcConfig,
    interest::{FilterManager, NewBlockNotification, SubscriptionManager},
    resolve::{BlockTags, ResolveError},
};
use alloy::eips::{BlockId, BlockNumberOrTag};
use signet_cold::ColdStorageReadHandle;
use signet_hot::HotKv;
use signet_hot::db::HotDbRead;
use signet_hot::model::{HotKvRead, RevmRead};
use signet_storage::UnifiedStorage;
use signet_tx_cache::TxCache;
use signet_types::constants::SignetSystemConstants;
use std::sync::Arc;
use tokio::sync::{Semaphore, broadcast};
use trevm::revm::database::DBErrorMarker;
use trevm::revm::database::StateBuilder;

/// Resolved block context for EVM execution.
///
/// Contains the header and a revm-compatible database snapshot at the
/// resolved block height, ready for use with `signet_evm`.
#[derive(Debug)]
pub(crate) struct EvmBlockContext<Db> {
    /// The resolved block header.
    pub header: alloy::consensus::Header,
    /// The revm database at the resolved height.
    pub db: trevm::revm::database::State<Db>,
}

/// RPC context backed by [`UnifiedStorage`].
///
/// Provides access to hot storage (state), cold storage (blocks/txs/receipts),
/// block tag resolution, and optional transaction forwarding.
///
/// # Construction
///
/// ```ignore
/// let ctx = StorageRpcCtx::new(storage, constants, tags, Some(tx_cache), StorageRpcConfig::default());
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
    config: StorageRpcConfig,
    tracing_semaphore: Arc<Semaphore>,
    filter_manager: FilterManager,
    sub_manager: SubscriptionManager,
}

impl<H: HotKv> StorageRpcCtx<H> {
    /// Create a new storage-backed RPC context.
    ///
    /// The `notif_sender` is used by the subscription manager to receive
    /// new block notifications. Callers send [`NewBlockNotification`]s on
    /// this channel as blocks are appended to storage.
    pub fn new(
        storage: UnifiedStorage<H>,
        constants: SignetSystemConstants,
        tags: BlockTags,
        tx_cache: Option<TxCache>,
        config: StorageRpcConfig,
        notif_sender: broadcast::Sender<NewBlockNotification>,
    ) -> Self {
        let tracing_semaphore = Arc::new(Semaphore::new(config.max_tracing_requests));
        let filter_manager = FilterManager::new(config.stale_filter_ttl, config.stale_filter_ttl);
        let sub_manager = SubscriptionManager::new(notif_sender, config.stale_filter_ttl);
        Self {
            inner: Arc::new(StorageRpcCtxInner {
                storage,
                constants,
                tags,
                tx_cache,
                config,
                tracing_semaphore,
                filter_manager,
                sub_manager,
            }),
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

    /// Access the RPC configuration.
    pub fn config(&self) -> &StorageRpcConfig {
        &self.inner.config
    }

    /// Acquire a permit from the tracing semaphore.
    ///
    /// Limits concurrent tracing/debug requests. Callers should hold
    /// the permit for the duration of their tracing operation.
    pub async fn acquire_tracing_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        Arc::clone(&self.inner.tracing_semaphore)
            .acquire_owned()
            .await
            .expect("tracing semaphore closed")
    }

    /// Access the optional tx cache.
    pub fn tx_cache(&self) -> Option<&TxCache> {
        self.inner.tx_cache.as_ref()
    }

    /// Access the filter manager.
    pub(crate) fn filter_manager(&self) -> &FilterManager {
        &self.inner.filter_manager
    }

    /// Access the subscription manager.
    pub(crate) fn sub_manager(&self) -> &SubscriptionManager {
        &self.inner.sub_manager
    }

    /// Resolve a [`BlockNumberOrTag`] to a block number.
    ///
    /// This is synchronous — no cold storage lookup is needed.
    ///
    /// - `Latest` / `Pending` → latest tag
    /// - `Safe` → safe tag
    /// - `Finalized` → finalized tag
    /// - `Earliest` → `0`
    /// - `Number(n)` → `n`
    pub(crate) fn resolve_block_tag(&self, tag: BlockNumberOrTag) -> u64 {
        match tag {
            BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => self.tags().latest(),
            BlockNumberOrTag::Safe => self.tags().safe(),
            BlockNumberOrTag::Finalized => self.tags().finalized(),
            BlockNumberOrTag::Earliest => 0,
            BlockNumberOrTag::Number(n) => n,
        }
    }

    /// Resolve a [`BlockId`] to a block number.
    ///
    /// For tag/number-based IDs, resolves synchronously via
    /// [`resolve_block_tag`](Self::resolve_block_tag). For hash-based IDs,
    /// looks up the block number from hot storage's `HeaderNumbers` table.
    pub(crate) fn resolve_block_id(&self, id: BlockId) -> Result<u64, ResolveError>
    where
        <H::RoTx as HotKvRead>::Error: std::error::Error + Send + Sync + 'static,
    {
        match id {
            BlockId::Number(tag) => Ok(self.resolve_block_tag(tag)),
            BlockId::Hash(h) => {
                let reader = self.hot_reader()?;
                reader
                    .get_header_number(&h.block_hash)
                    .map_err(|e| ResolveError::Db(Box::new(e)))?
                    .ok_or(ResolveError::HashNotFound(h.block_hash))
            }
        }
    }

    /// Resolve a [`BlockId`] to a header from hot storage.
    ///
    /// For hash-based IDs, fetches the header directly by hash. For
    /// tag/number-based IDs, resolves the tag then fetches the header by
    /// number. Returns `None` if the header is not found.
    pub(crate) fn resolve_header(
        &self,
        id: BlockId,
    ) -> Result<Option<signet_storage_types::SealedHeader>, ResolveError>
    where
        <H::RoTx as HotKvRead>::Error: std::error::Error + Send + Sync + 'static,
    {
        let reader = self.hot_reader()?;
        match id {
            BlockId::Hash(h) => {
                reader.header_by_hash(&h.block_hash).map_err(|e| ResolveError::Db(Box::new(e)))
            }
            BlockId::Number(tag) => {
                let height = self.resolve_block_tag(tag);
                reader.get_header(height).map_err(|e| ResolveError::Db(Box::new(e)))
            }
        }
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
        <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    {
        let revm_read = self.inner.storage.revm_reader_at_height(height)?;
        Ok(StateBuilder::new_with_database(revm_read).build())
    }

    /// Resolve a [`BlockId`] to a header and revm database in one pass.
    ///
    /// Fetches the header from hot storage and creates a revm-compatible
    /// database snapshot at the resolved block height.
    ///
    /// For `Pending` block IDs, remaps to `Latest` and synthesizes a
    /// next-block header (incremented number, timestamp +12s, projected
    /// base fee, gas limit from config). State is loaded at the latest
    /// finalized block in both cases.
    pub(crate) fn resolve_evm_block(
        &self,
        id: BlockId,
    ) -> Result<EvmBlockContext<RevmRead<H::RoTx>>, EthError>
    where
        <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    {
        let pending = id.is_pending();
        let id = if pending { BlockId::latest() } else { id };

        let sealed = self.resolve_header(id)?.ok_or(EthError::BlockNotFound(id))?;
        let db = self.revm_state_at_height(sealed.number)?;

        let parent_hash = sealed.hash();
        let mut header = sealed.into_inner();

        if pending {
            header.parent_hash = parent_hash;
            header.number += 1;
            header.timestamp += 12;
            header.base_fee_per_gas =
                header.next_block_base_fee(alloy::eips::eip1559::BaseFeeParams::ethereum());
            header.gas_limit = self.config().rpc_gas_cap;
        }

        Ok(EvmBlockContext { header, db })
    }
}
