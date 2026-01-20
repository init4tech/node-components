//! Ergonomic handle for interacting with cold storage.
//!
//! The [`ColdStorageHandle`] provides a convenient API for sending requests
//! to the cold storage task without needing to construct request types manually.

use crate::cold::{
    AppendBlockRequest, BlockData, ColdReadRequest, ColdResult, ColdStorageError,
    ColdStorageRequest, ColdWriteRequest, HeaderSpecifier, ReceiptSpecifier, SignetEventsSpecifier,
    TransactionSpecifier, ZenithHeaderSpecifier,
};
use alloy::primitives::{B256, BlockNumber};
use reth::primitives::{Header, Receipt, TransactionSigned};
use signet_db::{DbSignetEvent, DbZenithHeader};
use tokio::sync::{mpsc, oneshot};

/// Handle for interacting with the cold storage task.
///
/// This handle can be cloned and shared across tasks. It provides an ergonomic
/// API for sending requests to the storage task and receiving responses.
#[derive(Clone, Debug)]
pub struct ColdStorageHandle {
    sender: mpsc::Sender<ColdStorageRequest>,
}

impl ColdStorageHandle {
    /// Create a new handle with the given sender.
    pub(crate) const fn new(sender: mpsc::Sender<ColdStorageRequest>) -> Self {
        Self { sender }
    }

    /// Send a request and wait for the response.
    async fn send<T>(
        &self,
        req: ColdStorageRequest,
        rx: oneshot::Receiver<ColdResult<T>>,
    ) -> ColdResult<T> {
        self.sender.send(req).await.map_err(|_| ColdStorageError::Cancelled)?;
        rx.await.map_err(|_| ColdStorageError::Cancelled)?
    }

    // ==========================================================================
    // Headers
    // ==========================================================================

    /// Get a header by specifier.
    pub async fn get_header(&self, spec: HeaderSpecifier) -> ColdResult<Option<Header>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetHeader { spec, resp }.into(), rx).await
    }

    /// Get a header by block number.
    pub async fn get_header_by_number(&self, block: BlockNumber) -> ColdResult<Option<Header>> {
        self.get_header(HeaderSpecifier::Number(block)).await
    }

    /// Get a header by block hash.
    pub async fn get_header_by_hash(&self, hash: B256) -> ColdResult<Option<Header>> {
        self.get_header(HeaderSpecifier::Hash(hash)).await
    }

    /// Get multiple headers by specifiers.
    pub async fn get_headers(
        &self,
        specs: Vec<HeaderSpecifier>,
    ) -> ColdResult<Vec<Option<Header>>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetHeaders { specs, resp }.into(), rx).await
    }

    // ==========================================================================
    // Transactions
    // ==========================================================================

    /// Get a transaction by specifier.
    pub async fn get_transaction(
        &self,
        spec: TransactionSpecifier,
    ) -> ColdResult<Option<TransactionSigned>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetTransaction { spec, resp }.into(), rx).await
    }

    /// Get a transaction by hash.
    pub async fn get_tx_by_hash(&self, hash: B256) -> ColdResult<Option<TransactionSigned>> {
        self.get_transaction(TransactionSpecifier::Hash(hash)).await
    }

    /// Get a transaction by block number and index.
    pub async fn get_tx_by_block_and_index(
        &self,
        block: BlockNumber,
        index: u64,
    ) -> ColdResult<Option<TransactionSigned>> {
        self.get_transaction(TransactionSpecifier::BlockAndIndex { block, index }).await
    }

    /// Get a transaction by block hash and index.
    pub async fn get_tx_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: u64,
    ) -> ColdResult<Option<TransactionSigned>> {
        self.get_transaction(TransactionSpecifier::BlockHashAndIndex { block_hash, index }).await
    }

    /// Get all transactions in a block.
    pub async fn get_transactions_in_block(
        &self,
        block: BlockNumber,
    ) -> ColdResult<Vec<TransactionSigned>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetTransactionsInBlock { block, resp }.into(), rx).await
    }

    /// Get the transaction count for a block.
    pub async fn get_transaction_count(&self, block: BlockNumber) -> ColdResult<u64> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetTransactionCount { block, resp }.into(), rx).await
    }

    // ==========================================================================
    // Receipts
    // ==========================================================================

    /// Get a receipt by specifier.
    pub async fn get_receipt(&self, spec: ReceiptSpecifier) -> ColdResult<Option<Receipt>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetReceipt { spec, resp }.into(), rx).await
    }

    /// Get a receipt by transaction hash.
    pub async fn get_receipt_by_tx_hash(&self, hash: B256) -> ColdResult<Option<Receipt>> {
        self.get_receipt(ReceiptSpecifier::TxHash(hash)).await
    }

    /// Get a receipt by block number and index.
    pub async fn get_receipt_by_block_and_index(
        &self,
        block: BlockNumber,
        index: u64,
    ) -> ColdResult<Option<Receipt>> {
        self.get_receipt(ReceiptSpecifier::BlockAndIndex { block, index }).await
    }

    /// Get all receipts in a block.
    pub async fn get_receipts_in_block(&self, block: BlockNumber) -> ColdResult<Vec<Receipt>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetReceiptsInBlock { block, resp }.into(), rx).await
    }

    // ==========================================================================
    // SignetEvents
    // ==========================================================================

    /// Get signet events by specifier.
    pub async fn get_signet_events(
        &self,
        spec: SignetEventsSpecifier,
    ) -> ColdResult<Vec<DbSignetEvent>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetSignetEvents { spec, resp }.into(), rx).await
    }

    /// Get signet events in a block.
    pub async fn get_signet_events_in_block(
        &self,
        block: BlockNumber,
    ) -> ColdResult<Vec<DbSignetEvent>> {
        self.get_signet_events(SignetEventsSpecifier::Block(block)).await
    }

    /// Get signet events in a range of blocks.
    pub async fn get_signet_events_in_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ColdResult<Vec<DbSignetEvent>> {
        self.get_signet_events(SignetEventsSpecifier::BlockRange { start, end }).await
    }

    // ==========================================================================
    // ZenithHeaders
    // ==========================================================================

    /// Get a zenith header by block number.
    pub async fn get_zenith_header(
        &self,
        block: BlockNumber,
    ) -> ColdResult<Option<DbZenithHeader>> {
        let (resp, rx) = oneshot::channel();
        self.send(
            ColdReadRequest::GetZenithHeader { spec: ZenithHeaderSpecifier::Number(block), resp }
                .into(),
            rx,
        )
        .await
    }

    /// Get zenith headers by specifier.
    pub async fn get_zenith_headers(
        &self,
        spec: ZenithHeaderSpecifier,
    ) -> ColdResult<Vec<DbZenithHeader>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetZenithHeaders { spec, resp }.into(), rx).await
    }

    /// Get zenith headers in a range of blocks.
    pub async fn get_zenith_headers_in_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ColdResult<Vec<DbZenithHeader>> {
        self.get_zenith_headers(ZenithHeaderSpecifier::Range { start, end }).await
    }

    // ==========================================================================
    // Metadata
    // ==========================================================================

    /// Get the latest block number in storage.
    pub async fn get_latest_block(&self) -> ColdResult<Option<BlockNumber>> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdReadRequest::GetLatestBlock { resp }.into(), rx).await
    }

    // ==========================================================================
    // Write Operations
    // ==========================================================================

    /// Append a single block to cold storage.
    pub async fn append_block(&self, data: BlockData) -> ColdResult<()> {
        let (resp, rx) = oneshot::channel();
        self.send(
            ColdWriteRequest::AppendBlock(Box::new(AppendBlockRequest { data, resp })).into(),
            rx,
        )
        .await
    }

    /// Append multiple blocks to cold storage.
    pub async fn append_blocks(&self, data: Vec<BlockData>) -> ColdResult<()> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdWriteRequest::AppendBlocks { data, resp }.into(), rx).await
    }

    /// Truncate all data above the given block number.
    ///
    /// This removes block N+1 and higher from all tables.
    pub async fn truncate_above(&self, block: BlockNumber) -> ColdResult<()> {
        let (resp, rx) = oneshot::channel();
        self.send(ColdWriteRequest::TruncateAbove { block, resp }.into(), rx).await
    }
}
