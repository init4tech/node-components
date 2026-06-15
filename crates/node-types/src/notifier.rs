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
/// 1. **`set_head`** — called exactly once at startup before the first
///    [`next_notification`]. Subsequent calls are silently ignored. The
///    backend must resolve the block number to a hash (falling back to
///    genesis if the number is not yet available) and begin delivering
///    notifications from that point.
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
pub trait HostNotifier: Send + Sync {
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
    ///
    /// This must be called exactly once before the first
    /// [`next_notification`]. Subsequent calls are silently ignored.
    ///
    /// [`next_notification`]: HostNotifier::next_notification
    fn set_head(&mut self, block_number: u64);

    /// Configure backfill batch size limits. `None` means use the backend's
    /// default.
    fn set_backfill_thresholds(&mut self, max_blocks: Option<u64>);

    /// Signal that processing is complete up to this host block number.
    /// The backend resolves the block number to a block hash internally.
    fn send_finished_height(&self, block_number: u64) -> Result<(), Self::Error>;

    /// Query the current host-chain tip block number. Used by a syncing node to
    /// decide when its applied state has caught up to the host and it can hand
    /// off to live block execution.
    fn host_tip(&self) -> impl Future<Output = Result<u64, Self::Error>> + Send;

    /// Whether leaving notifications unconsumed backpressures - and can stall - the host.
    ///
    /// A reth ExEx shares the host node's notification pipeline: if it stops consuming, reth's
    /// notification buffer fills and reth's pipeline stalls. A journal-syncing node never
    /// consumes host notifications (it derives state from the upstream journal feed), so for such
    /// a backend it must drain and discard them to keep the host moving. Pull-based followers
    /// that poll a remote host are unaffected and use the default `false`.
    fn backpressures_host(&self) -> bool {
        false
    }
}
