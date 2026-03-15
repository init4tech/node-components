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
///
/// # Implementing
///
/// Implementations must uphold the following contract:
///
/// 1. **`set_head`** — called once at startup before the first
///    [`next_notification`]. The backend must resolve the block number to a
///    hash (falling back to genesis if the number is not yet available) and
///    begin delivering notifications from that point.
/// 2. **`next_notification`** — must yield notifications in host-chain order.
///    Returning `None` signals a clean shutdown.
/// 3. **`set_backfill_thresholds`** — may be called at any time. Passing
///    `None` should restore the backend's default batch size.
/// 4. **`send_finished_height`** — may be called after processing each
///    notification batch. The backend resolves the block number to a hash
///    internally. Sending a height that has already been acknowledged is a
///    no-op.
///
/// [`next_notification`]: HostNotifier::next_notification
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
