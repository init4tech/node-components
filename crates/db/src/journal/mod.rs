//! Utilities for working with Signet journals in a reth database.

mod r#trait;
pub use r#trait::JournalDb;

use futures_util::{Stream, StreamExt};
use reth::providers::{DatabaseProviderRW, ProviderResult};
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use std::sync::Arc;
use tokio::task::JoinHandle;
use trevm::journal::BlockUpdate;

/// A task that ingests journals into a reth database.
#[derive(Debug)]
pub struct JournalIngestor<Db: NodeTypesDbTrait> {
    db: Arc<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>,
}

impl<Db: NodeTypesDbTrait> From<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>
    for JournalIngestor<Db>
{
    fn from(value: DatabaseProviderRW<Db, SignetNodeTypes<Db>>) -> Self {
        Self::new(value.into())
    }
}

impl<Db: NodeTypesDbTrait> From<Arc<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>>
    for JournalIngestor<Db>
{
    fn from(value: Arc<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>) -> Self {
        Self::new(value)
    }
}

impl<Db: NodeTypesDbTrait> JournalIngestor<Db> {
    /// Create a new `JournalIngestor` with the given database provider.
    pub const fn new(db: Arc<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>) -> Self {
        Self { db }
    }

    async fn task_future<S>(self, mut stream: S) -> ProviderResult<()>
    where
        S: Stream<Item = (alloy::consensus::Header, BlockUpdate<'static>)> + Send + Unpin + 'static,
    {
        while let Some(item) = stream.next().await {
            // FUTURE: Sanity check that the header height matches the update
            // height. Sanity check that both heights are 1 greater than the
            // last height in the database.

            let db = self.db.clone();
            let (header, block_update) = item;

            // DB interaction is sync, so we spawn a blocking task for it. We
            // immediately await that task. This prevents blocking the worker
            // thread
            tokio::task::spawn_blocking(move || db.ingest(&header, block_update))
                .await
                .expect("ingestion should not panic")?;
        }
        // Stream has ended, return Ok
        Ok(())
    }

    /// Spawn a task to ingest journals from the provided stream.
    pub fn spawn<S>(self, stream: S) -> JoinHandle<ProviderResult<()>>
    where
        S: Stream<Item = (alloy::consensus::Header, BlockUpdate<'static>)> + Send + Unpin + 'static,
    {
        tokio::spawn(self.task_future(stream))
    }
}

/// Ingest journals from a stream into a reth database.
pub async fn ingest_journals<Db, S>(
    db: Arc<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>,
    stream: S,
) -> ProviderResult<()>
where
    Db: NodeTypesDbTrait,
    S: Stream<Item = (alloy::consensus::Header, BlockUpdate<'static>)> + Send + Unpin + 'static,
{
    let ingestor = JournalIngestor::new(db);
    ingestor.task_future(stream).await
}
