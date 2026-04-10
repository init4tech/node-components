use crate::{RecoveredBlockShim, error::RethHostError, metrics};
use alloy::consensus::{Block, BlockHeader};
use reth::primitives::{Receipt, RecoveredBlock};
use reth::providers::{
    BlockIdReader, BlockReader, HeaderProvider, ReceiptProvider, TransactionVariant,
};
use signet_extract::{BlockAndReceipts, Extractable};
use signet_types::primitives::TransactionSigned;
use std::time::Instant;
use tracing::{debug, instrument};

/// Default number of blocks fetched per [`DbBackfill`] batch.
const DEFAULT_BATCH_SIZE: u64 = 1000;

/// Reth's recovered block type, aliased for readability.
type RethRecovered = RecoveredBlock<Block<TransactionSigned>>;

/// An owned block and its receipts, read from the reth DB.
#[derive(Debug)]
pub(crate) struct DbBlock {
    block: RethRecovered,
    receipts: Vec<Receipt>,
}

/// A contiguous segment of blocks read from the reth DB.
///
/// Implements [`Extractable`] using the same `RecoveredBlockShim` transmute
/// pattern as [`RethChain`](crate::RethChain).
#[derive(Debug)]
pub struct DbChainSegment(Vec<DbBlock>);

impl Extractable for DbChainSegment {
    type Block = RecoveredBlockShim;
    type Receipt = Receipt;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>> {
        self.0.iter().map(|db_block| {
            // Compile-time check: RecoveredBlockShim must have the same
            // layout as RethRecovered (guaranteed by #[repr(transparent)]
            // on RecoveredBlockShim in shim.rs).
            const {
                assert!(
                    size_of::<RecoveredBlockShim>() == size_of::<RethRecovered>(),
                    "RecoveredBlockShim layout diverged from RethRecovered"
                );
                assert!(
                    align_of::<RecoveredBlockShim>() == align_of::<RethRecovered>(),
                    "RecoveredBlockShim alignment diverged from RethRecovered"
                );
            }
            // SAFETY: `RecoveredBlockShim` is `#[repr(transparent)]` over
            // `RethRecovered`, so these types have identical memory layouts.
            // The lifetime of the reference is tied to `db_block`, which
            // outlives the returned iterator item.
            let block = unsafe {
                std::mem::transmute::<&RethRecovered, &RecoveredBlockShim>(&db_block.block)
            };
            BlockAndReceipts { block, receipts: &db_block.receipts }
        })
    }

    fn first_number(&self) -> u64 {
        self.0.first().map(|b| b.block.number()).expect("DbChainSegment must be non-empty")
    }

    fn tip_number(&self) -> u64 {
        self.0.last().map(|b| b.block.number()).expect("DbChainSegment must be non-empty")
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Reads blocks and receipts from the reth DB provider in batches.
///
/// Starting from a cursor block number, each call to [`DbBackfill::next_batch`]
/// fetches up to `batch_size` blocks (default: 1000) up to and including the
/// current finalized block. Returns `None` when the cursor exceeds finalized.
#[derive(Debug)]
pub(crate) struct DbBackfill<P> {
    provider: P,
    cursor: u64,
    batch_size: u64,
}

impl<P> DbBackfill<P> {
    /// Create a new `DbBackfill` starting at `cursor` with the default batch size.
    pub(crate) const fn new(provider: P, cursor: u64) -> Self {
        Self { provider, cursor, batch_size: DEFAULT_BATCH_SIZE }
    }

    /// Set the batch size (minimum 1).
    pub(crate) fn set_batch_size(&mut self, batch_size: u64) {
        self.batch_size = batch_size.max(1);
    }

    /// Current cursor position (next block to fetch).
    pub(crate) const fn cursor(&self) -> u64 {
        self.cursor
    }
}

impl<P> DbBackfill<P>
where
    P: BlockReader<Block = Block<TransactionSigned>>
        + ReceiptProvider<Receipt = Receipt>
        + HeaderProvider
        + BlockIdReader
        + Clone
        + Send
        + Sync
        + 'static,
{
    /// Fetch the next batch of blocks from the DB.
    ///
    /// Returns `Ok(None)` when the cursor has passed the finalized block
    /// (backfill complete). Otherwise reads up to `batch_size` blocks
    /// starting at `cursor` and advances the cursor.
    #[instrument(
        name = "db_backfill.next_batch",
        skip(self),
        fields(from = self.cursor, to, batch_size = self.batch_size),
        level = "info"
    )]
    pub(crate) async fn next_batch(&mut self) -> Result<Option<DbChainSegment>, RethHostError> {
        let finalized = match self.provider.finalized_block_number()? {
            Some(n) => n,
            None => return Ok(None),
        };

        if self.cursor > finalized {
            return Ok(None);
        }

        let from = self.cursor;
        let to = finalized.min(self.cursor + self.batch_size - 1);

        tracing::Span::current().record("to", to);

        let provider = self.provider.clone();
        let start = Instant::now();

        let segment = tokio::task::spawn_blocking(move || read_block_range(&provider, from, to))
            .await
            .expect("spawn_blocking panicked")?;

        let duration = start.elapsed();
        let block_count = segment.len() as u64;

        metrics::record_backfill_batch(block_count, duration);
        metrics::set_backfill_cursor(to + 1);

        self.cursor = to + 1;

        debug!(block_count, cursor = self.cursor, "backfill batch complete");

        Ok(Some(segment))
    }
}

/// Read blocks and receipts for `from..=to` from the provider.
///
/// Extracted from [`DbBackfill::next_batch`] so it can be called inside
/// `spawn_blocking` with a synchronous provider.
fn read_block_range<P>(provider: &P, from: u64, to: u64) -> Result<DbChainSegment, RethHostError>
where
    P: BlockReader<Block = Block<TransactionSigned>> + ReceiptProvider<Receipt = Receipt>,
{
    let mut blocks = Vec::with_capacity((to - from + 1) as usize);
    let mut prev_hash: Option<alloy::primitives::B256> = None;

    for number in from..=to {
        let block = provider
            .recovered_block(number.into(), TransactionVariant::WithHash)?
            .ok_or(RethHostError::MissingBlock(number))?;

        debug_assert!(
            prev_hash.is_none_or(|h| block.parent_hash() == h),
            "parent hash continuity violated at block {number}"
        );
        prev_hash = Some(block.sealed_block().hash());

        let receipts = provider
            .receipts_by_block(number.into())?
            .ok_or(RethHostError::MissingReceipts(number))?;

        blocks.push(DbBlock { block, receipts });
    }

    Ok(DbChainSegment(blocks))
}
