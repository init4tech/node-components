use crate::HostNotification;
use core::future::Future;
use signet_extract::Extractable;

/// Abstraction over a host chain notification source.
///
/// Drives the signet node's main loop: yielding chain events, controlling
/// backfill, and sending feedback. All block data comes from notifications;
/// the backend handles hash resolution internally.
///
/// # Implementors
///
/// - `signet-host-reth`: wraps reth's `ExExContext`
pub trait HostNotifier {
    /// A chain segment — contiguous blocks with receipts.
    type Chain: Extractable;

    /// The error type for fallible operations.
    type Error: core::error::Error + Send + Sync + 'static;

    /// Yield the next notification. `None` signals host shutdown.
    fn next_notification(
        &mut self,
    ) -> impl Future<Output = Option<Result<HostNotification<Self::Chain>, Self::Error>>> + Send;

    /// Set the head position, requesting backfill from this block number.
    /// The backend resolves the block number to a block hash internally.
    fn set_head(&mut self, block_number: u64);

    /// Configure backfill batch size limits. `None` means use the backend's
    /// default.
    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>);

    /// Signal that processing is complete up to this host block number.
    /// The backend resolves the block number to a block hash internally.
    fn send_finished_height(&self, block_number: u64) -> Result<(), Self::Error>;
}
