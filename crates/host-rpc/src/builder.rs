use crate::{RpcHostError, RpcHostNotifier};
use alloy::providers::Provider;
use tracing::warn;

/// Builder for [`RpcHostNotifier`].
///
/// # Example
///
/// ```ignore
/// let notifier = RpcHostNotifierBuilder::new(provider)
///     .with_buffer_capacity(128)
///     .with_backfill_batch_size(64)
///     .with_genesis_timestamp(1_606_824_023)
///     .build()
///     .await?;
/// ```
#[derive(Debug)]
pub struct RpcHostNotifierBuilder<P> {
    provider: P,
    buffer_capacity: usize,
    backfill_batch_size: u64,
    genesis_timestamp: u64,
}

impl<P> RpcHostNotifierBuilder<P>
where
    P: Provider + Clone,
{
    /// Create a new builder with the given provider.
    pub const fn new(provider: P) -> Self {
        Self {
            provider,
            buffer_capacity: crate::DEFAULT_BUFFER_CAPACITY,
            backfill_batch_size: crate::DEFAULT_BACKFILL_BATCH_SIZE,
            genesis_timestamp: 0,
        }
    }

    /// Set the block buffer capacity (default: 64).
    pub const fn with_buffer_capacity(mut self, capacity: usize) -> Self {
        self.buffer_capacity = capacity;
        self
    }

    /// Set the backfill batch size (default: 32).
    pub const fn with_backfill_batch_size(mut self, batch_size: u64) -> Self {
        self.backfill_batch_size = batch_size;
        self
    }

    /// Set the genesis timestamp for epoch calculation.
    pub const fn with_genesis_timestamp(mut self, timestamp: u64) -> Self {
        self.genesis_timestamp = timestamp;
        self
    }

    /// Build the notifier, establishing the `newHeads` WebSocket subscription.
    pub async fn build(self) -> Result<RpcHostNotifier<P>, RpcHostError> {
        if self.genesis_timestamp == 0 {
            warn!("genesis_timestamp not set; epoch calculations will use Unix epoch");
        }
        let sub = self.provider.subscribe_blocks().await?;
        let header_sub = sub.into_stream();

        Ok(RpcHostNotifier::new(
            self.provider,
            header_sub,
            self.buffer_capacity,
            self.backfill_batch_size,
            self.genesis_timestamp,
        ))
    }
}
