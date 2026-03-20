use crate::{BlobSource, BlobSpec, Blobs};
use alloy::{eips::eip7594::BlobTransactionSidecarVariant, primitives::TxHash};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// In-memory blob source for testing.
///
/// Stores blob sidecars in a `HashMap` keyed by transaction hash.
/// Thread-safe via interior `Mutex`.
#[derive(Debug, Clone, Default)]
pub struct MemoryBlobSource {
    blobs: Arc<Mutex<HashMap<TxHash, Arc<BlobTransactionSidecarVariant>>>>,
}

impl MemoryBlobSource {
    /// Create a new empty memory blob source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a blob sidecar for the given transaction hash.
    pub fn insert(&self, tx_hash: TxHash, sidecar: Arc<BlobTransactionSidecarVariant>) {
        self.blobs.lock().unwrap().insert(tx_hash, sidecar);
    }

    /// Remove a blob sidecar.
    pub fn remove(&self, tx_hash: &TxHash) {
        self.blobs.lock().unwrap().remove(tx_hash);
    }
}

impl BlobSource for MemoryBlobSource {
    fn get_blob(
        &self,
        spec: &BlobSpec,
    ) -> Result<Option<Blobs>, Box<dyn core::error::Error + Send + Sync>> {
        Ok(self.blobs.lock().unwrap().get(&spec.tx_hash).cloned().map(Blobs::from))
    }
}
