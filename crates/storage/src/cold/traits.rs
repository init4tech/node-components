//! Core trait definition for cold storage backends.
//!
//! The [`ColdStorage`] trait defines the interface that all cold storage
//! backends must implement. Backends are responsible for data organization,
//! indexing, and keying - the trait is agnostic to these implementation details.

use alloy::primitives::BlockNumber;
use reth::primitives::{Header, Receipt, TransactionSigned};
use signet_db::{DbSignetEvent, DbZenithHeader};
use std::future::Future;

use super::{
    ColdResult, HeaderSpecifier, ReceiptSpecifier, SignetEventsSpecifier, TransactionSpecifier,
    ZenithHeaderSpecifier,
};

/// Data for appending a complete block to cold storage.
#[derive(Debug, Clone)]
pub struct BlockData {
    /// The block header.
    pub header: Header,
    /// The transactions in the block.
    pub transactions: Vec<TransactionSigned>,
    /// The receipts for the transactions.
    pub receipts: Vec<Receipt>,
    /// The signet events in the block.
    pub signet_events: Vec<DbSignetEvent>,
    /// The zenith header for the block, if present.
    pub zenith_header: Option<DbZenithHeader>,
}

impl BlockData {
    /// Create new block data.
    pub const fn new(
        header: Header,
        transactions: Vec<TransactionSigned>,
        receipts: Vec<Receipt>,
        signet_events: Vec<DbSignetEvent>,
        zenith_header: Option<DbZenithHeader>,
    ) -> Self {
        Self { header, transactions, receipts, signet_events, zenith_header }
    }

    /// Get the block number of the block.
    pub const fn block_number(&self) -> BlockNumber {
        self.header.number
    }
}

/// Unified cold storage backend trait.
///
/// Backend is responsible for all data organization, indexing, and keying.
/// The trait is agnostic to how the backend stores or indexes data.
///
/// All methods are async and return futures that are `Send`.
///
/// # Implementation Guide
///
/// Implementers must ensure:
///
/// - **Append-only ordering**: `append_block` must enforce monotonically
///   increasing block numbers. Attempting to append a block with a number <=
///   the current latest should return an error.
///
/// - **Atomic truncation**: `truncate_above` must remove all data for blocks
///   N+1 and higher atomically. Partial truncation is not acceptable.
///
/// - **Index maintenance**: Hash-based lookups (e.g., header by hash,
///   transaction by hash) require the implementation to maintain appropriate
///   indexes. These indexes must be updated during `append_block` and cleaned
///   during `truncate_above`.
///
/// - **Consistent reads**: Read operations should return consistent snapshots.
///   A read started before a write completes should not see partial data from
///   that write.
///
/// - **Tag resolution**: `HeaderSpecifier::Tag` variants (Latest, Finalized,
///   Safe, Earliest) must be resolved by the implementation. For simple
///   backends, Latest/Finalized/Safe may all resolve to the same block.
pub trait ColdStorage: Send + Sync + 'static {
    // --- Headers ---

    /// Get a header by specifier.
    fn get_header(
        &self,
        spec: HeaderSpecifier,
    ) -> impl Future<Output = ColdResult<Option<Header>>> + Send;

    /// Get multiple headers by specifiers.
    fn get_headers(
        &self,
        specs: Vec<HeaderSpecifier>,
    ) -> impl Future<Output = ColdResult<Vec<Option<Header>>>> + Send;

    // --- Transactions ---

    /// Get a transaction by specifier.
    fn get_transaction(
        &self,
        spec: TransactionSpecifier,
    ) -> impl Future<Output = ColdResult<Option<TransactionSigned>>> + Send;

    /// Get all transactions in a block.
    fn get_transactions_in_block(
        &self,
        block: BlockNumber,
    ) -> impl Future<Output = ColdResult<Vec<TransactionSigned>>> + Send;

    /// Get the number of transactions in a block.
    fn get_transaction_count(
        &self,
        block: BlockNumber,
    ) -> impl Future<Output = ColdResult<u64>> + Send;

    // --- Receipts ---

    /// Get a receipt by specifier.
    fn get_receipt(
        &self,
        spec: ReceiptSpecifier,
    ) -> impl Future<Output = ColdResult<Option<Receipt>>> + Send;

    /// Get all receipts in a block.
    fn get_receipts_in_block(
        &self,
        block: BlockNumber,
    ) -> impl Future<Output = ColdResult<Vec<Receipt>>> + Send;

    // --- SignetEvents ---

    /// Get signet events by specifier.
    fn get_signet_events(
        &self,
        spec: SignetEventsSpecifier,
    ) -> impl Future<Output = ColdResult<Vec<DbSignetEvent>>> + Send;

    // --- ZenithHeaders ---

    /// Get a zenith header by specifier.
    fn get_zenith_header(
        &self,
        spec: ZenithHeaderSpecifier,
    ) -> impl Future<Output = ColdResult<Option<DbZenithHeader>>> + Send;

    /// Get multiple zenith headers by specifier.
    fn get_zenith_headers(
        &self,
        spec: ZenithHeaderSpecifier,
    ) -> impl Future<Output = ColdResult<Vec<DbZenithHeader>>> + Send;

    // --- Metadata ---

    /// Get the latest block number in storage.
    fn get_latest_block(&self) -> impl Future<Output = ColdResult<Option<BlockNumber>>> + Send;

    // --- Write operations ---

    /// Append a single block to cold storage.
    fn append_block(&self, data: BlockData) -> impl Future<Output = ColdResult<()>> + Send;

    /// Append multiple blocks to cold storage.
    fn append_blocks(&self, data: Vec<BlockData>) -> impl Future<Output = ColdResult<()>> + Send;

    /// Truncate all data above the given block number (exclusive).
    ///
    /// This removes block N+1 and higher from all tables. Used for reorg handling.
    fn truncate_above(&self, block: BlockNumber) -> impl Future<Output = ColdResult<()>> + Send;
}
