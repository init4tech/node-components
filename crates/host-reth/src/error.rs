use reth_exex::ExExEvent;

/// Errors from the [`RethHostNotifier`](crate::RethHostNotifier).
#[derive(Debug, thiserror::Error)]
pub enum RethHostError {
    /// A notification stream error forwarded from reth.
    #[error("notification stream error: {0}")]
    Notification(#[source] Box<dyn core::error::Error + Send + Sync>),
    /// The provider failed to look up a header or block tag.
    #[error("provider error: {0}")]
    Provider(#[from] reth::providers::ProviderError),
    /// Failed to send an ExEx event back to the host.
    #[error("failed to send ExEx event")]
    EventSend(#[from] tokio::sync::mpsc::error::SendError<ExExEvent>),
    /// A required header was missing from the provider.
    #[error("missing header for block {0}")]
    MissingHeader(u64),
    /// A required block was missing from the provider during DB backfill.
    #[error("missing block at number {0}")]
    MissingBlock(u64),
    /// Receipts were missing for a block during DB backfill.
    #[error("missing receipts for block {0}")]
    MissingReceipts(u64),
}

impl RethHostError {
    /// Wrap a notification stream error.
    pub fn notification(e: impl Into<Box<dyn core::error::Error + Send + Sync>>) -> Self {
        Self::Notification(e.into())
    }
}
