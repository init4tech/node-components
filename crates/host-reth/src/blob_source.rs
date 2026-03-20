//! Reth transaction pool adapter for [`BlobSource`].

use reth::transaction_pool::TransactionPool;
use signet_blobber::{BlobSource, BlobSpec, Blobs};

/// Newtype wrapper adapting a reth [`TransactionPool`] to [`BlobSource`].
///
/// Necessary because both traits are foreign to this crate (orphan rules).
#[derive(Debug)]
pub struct RethBlobSource<P>(
    /// The inner reth transaction pool.
    pub P,
);

impl<P: TransactionPool> BlobSource for RethBlobSource<P> {
    fn get_blob(
        &self,
        spec: &BlobSpec,
    ) -> Result<Option<Blobs>, Box<dyn core::error::Error + Send + Sync>> {
        self.0.get_blob(spec.tx_hash).map(|opt| opt.map(Blobs::from)).map_err(|e| Box::new(e) as _)
    }
}
