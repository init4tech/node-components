use crate::{DataCompat, journal::JournalDb};
use futures_util::{Stream, StreamExt};
use reth::{
    providers::{
        CanonChainTracker, DatabaseProviderFactory, DatabaseProviderRW, ProviderResult,
        providers::BlockchainProvider,
    },
    rpc::types::engine::ForkchoiceState,
};
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use signet_types::primitives::SealedHeader;
use tokio::task::JoinHandle;
use trevm::journal::BlockUpdate;

/// A task that processes journal updates for a specific database, and calls
/// the appropriate methods on a [`BlockchainProvider`] to update the in-memory
/// chain view.
#[derive(Debug, Clone)]
pub struct JournalProviderTask<Db: NodeTypesDbTrait> {
    provider: BlockchainProvider<SignetNodeTypes<Db>>,
}

impl<Db: NodeTypesDbTrait> JournalProviderTask<Db> {
    /// Instantiate a new task.
    pub const fn new(provider: BlockchainProvider<SignetNodeTypes<Db>>) -> Self {
        Self { provider }
    }

    /// Get a reference to the provider.
    pub const fn provider(&self) -> &BlockchainProvider<SignetNodeTypes<Db>> {
        &self.provider
    }

    /// Deconstruct the task into its provider.
    pub fn into_inner(self) -> BlockchainProvider<SignetNodeTypes<Db>> {
        self.provider
    }

    /// Create a future for the task, suitable for [`tokio::spawn`] or another
    /// task-spawning system.
    pub async fn task_future<S>(self, mut journals: S) -> ProviderResult<()>
    where
        S: Stream<Item = (SealedHeader, BlockUpdate<'static>)> + Send + Unpin + 'static,
    {
        loop {
            let Some((header, block_update)) = journals.next().await else { break };

            let block_hash = header.hash();

            let rw = self.provider.database_provider_rw().map(DatabaseProviderRW);

            let r_header = header.clone_convert();

            // DB interaction is sync, so we spawn a blocking task for it. We
            // immediately await that task. This prevents blocking the worker
            // thread
            tokio::task::spawn_blocking(move || rw?.ingest(header, block_update))
                .await
                .expect("ingestion should not panic")?;

            self.provider.set_canonical_head(r_header.clone());
            self.provider.set_safe(r_header.clone());
            self.provider.set_finalized(r_header);
            self.provider.on_forkchoice_update_received(&ForkchoiceState {
                head_block_hash: block_hash,
                safe_block_hash: block_hash,
                finalized_block_hash: block_hash,
            });
        }

        Ok(())
    }

    /// Spawn the journal provider task.
    pub fn spawn<S>(self, journals: S) -> JoinHandle<ProviderResult<()>>
    where
        S: Stream<Item = (SealedHeader, BlockUpdate<'static>)> + Send + Unpin + 'static,
    {
        tokio::spawn(self.task_future(journals))
    }
}
