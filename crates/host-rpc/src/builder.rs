use crate::{RpcHostError, RpcHostNotifier};
use alloy::providers::Provider;
use std::collections::VecDeque;

/// Default block buffer capacity.
const DEFAULT_BUFFER_CAPACITY: usize = 64;
/// Default backfill batch size.
const DEFAULT_BACKFILL_BATCH_SIZE: u64 = 32;

/// Builder for [`RpcHostNotifier`].
///
/// # Example
///
/// ```ignore
/// let notifier = RpcHostNotifierBuilder::new(provider)
///     .with_buffer_capacity(128)
///     .with_backfill_batch_size(64)
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
            buffer_capacity: DEFAULT_BUFFER_CAPACITY,
            backfill_batch_size: DEFAULT_BACKFILL_BATCH_SIZE,
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
        let sub = self.provider.subscribe_blocks().await?;
        let header_sub = sub.into_stream();

        Ok(RpcHostNotifier {
            provider: self.provider,
            header_sub,
            block_buffer: VecDeque::with_capacity(self.buffer_capacity),
            buffer_capacity: self.buffer_capacity,
            cached_safe: None,
            cached_finalized: None,
            last_tag_epoch: None,
            backfill_from: None,
            backfill_batch_size: self.backfill_batch_size,
            genesis_timestamp: self.genesis_timestamp,
        })
    }
}
