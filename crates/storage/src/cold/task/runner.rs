//! Cold storage task runner.
//!
//! The [`ColdStorageTask`] processes requests from a channel and dispatches
//! them to the storage backend.

use crate::cold::{
    ColdReadRequest, ColdStorage, ColdStorageHandle, ColdStorageRequest, ColdWriteRequest,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, instrument};

/// Channel size for cold storage requests.
const COLD_STORAGE_CHANNEL_SIZE: usize = 256;

/// Maximum concurrent request handlers.
const MAX_CONCURRENT_HANDLERS: usize = 64;

/// The cold storage task that processes requests.
///
/// This task receives requests over a channel and dispatches them to the
/// storage backend. It supports graceful shutdown via a cancellation token.
pub struct ColdStorageTask<B: ColdStorage> {
    backend: Arc<B>,
    receiver: mpsc::Receiver<ColdStorageRequest>,
    cancel_token: CancellationToken,
    task_tracker: TaskTracker,
}

impl<B: ColdStorage> std::fmt::Debug for ColdStorageTask<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdStorageTask").finish_non_exhaustive()
    }
}

impl<B: ColdStorage> ColdStorageTask<B> {
    /// Create a new cold storage task and return its handle.
    pub fn new(backend: B, cancel_token: CancellationToken) -> (Self, ColdStorageHandle) {
        let (sender, receiver) = mpsc::channel(COLD_STORAGE_CHANNEL_SIZE);
        let task = Self {
            backend: Arc::new(backend),
            receiver,
            cancel_token,
            task_tracker: TaskTracker::new(),
        };
        let handle = ColdStorageHandle::new(sender);
        (task, handle)
    }

    /// Spawn the task and return the handle.
    ///
    /// The task will run until the cancellation token is triggered or the
    /// channel is closed.
    pub fn spawn(backend: B, cancel_token: CancellationToken) -> ColdStorageHandle {
        let (task, handle) = Self::new(backend, cancel_token);
        tokio::spawn(task.run());
        handle
    }

    /// Run the task, processing requests until shutdown.
    #[instrument(skip(self), name = "cold_storage_task")]
    pub async fn run(mut self) {
        debug!("Cold storage task started");

        loop {
            tokio::select! {
                // Check for cancellation
                _ = self.cancel_token.cancelled() => {
                    debug!("Cold storage task received cancellation signal");
                    break;
                }

                // Process incoming requests
                maybe_request = self.receiver.recv() => {
                    match maybe_request {
                        Some(request) => {
                            // Wait if we've hit the concurrent handler limit (backpressure)
                            while self.task_tracker.len() >= MAX_CONCURRENT_HANDLERS {
                                // Wait for at least one task to complete
                                tokio::select! {
                                    _ = self.cancel_token.cancelled() => {
                                        debug!("Cancellation while waiting for task slot");
                                        break;
                                    }
                                    _ = self.task_tracker.wait() => {}
                                }
                            }

                            let backend = Arc::clone(&self.backend);
                            self.task_tracker.spawn(async move {
                                Self::handle_request(backend, request).await;
                            });
                        }
                        None => {
                            debug!("Cold storage channel closed");
                            break;
                        }
                    }
                }
            }
        }

        // Graceful shutdown: wait for in-progress tasks to complete
        debug!("Waiting for in-progress handlers to complete");
        self.task_tracker.close();
        self.task_tracker.wait().await;
        debug!("Cold storage task shut down gracefully");
    }

    async fn handle_request(backend: Arc<B>, request: ColdStorageRequest) {
        match request {
            ColdStorageRequest::Read(read_req) => {
                Self::handle_read(backend, read_req).await;
            }
            ColdStorageRequest::Write(write_req) => {
                Self::handle_write(backend, write_req).await;
            }
        }
    }

    async fn handle_read(backend: Arc<B>, req: ColdReadRequest) {
        match req {
            ColdReadRequest::GetHeader { spec, resp } => {
                let result = backend.get_header(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetHeaders { specs, resp } => {
                let result = backend.get_headers(specs).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetTransaction { spec, resp } => {
                let result = backend.get_transaction(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetTransactionsInBlock { block, resp } => {
                let result = backend.get_transactions_in_block(block).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetTransactionCount { block, resp } => {
                let result = backend.get_transaction_count(block).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetReceipt { spec, resp } => {
                let result = backend.get_receipt(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetReceiptsInBlock { block, resp } => {
                let result = backend.get_receipts_in_block(block).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetSignetEvents { spec, resp } => {
                let result = backend.get_signet_events(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetZenithHeader { spec, resp } => {
                let result = backend.get_zenith_header(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetZenithHeaders { spec, resp } => {
                let result = backend.get_zenith_headers(spec).await;
                let _ = resp.send(result);
            }
            ColdReadRequest::GetLatestBlock { resp } => {
                let result = backend.get_latest_block().await;
                let _ = resp.send(result);
            }
        }
    }

    async fn handle_write(backend: Arc<B>, req: ColdWriteRequest) {
        match req {
            ColdWriteRequest::AppendBlock(boxed) => {
                let result = backend.append_block(boxed.data).await;
                let _ = boxed.resp.send(result);
            }
            ColdWriteRequest::AppendBlocks { data, resp } => {
                let result = backend.append_blocks(data).await;
                let _ = resp.send(result);
            }
            ColdWriteRequest::TruncateAbove { block, resp } => {
                let result = backend.truncate_above(block).await;
                let _ = resp.send(result);
            }
        }
    }
}
