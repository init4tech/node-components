use core::num::NonZeroU64;
use init4_bin_base::utils::from_env::FromEnv;
use signet_journal_chain::SAFETY_MARGIN;
use tracing::warn;

/// Default maximum total byte size of the journal ring buffer (64 MiB).
pub const DEFAULT_RING_BUFFER_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Default maximum number of journals held in the ring buffer. Must be at
/// least [`SAFETY_MARGIN`]; smaller values are clamped up by the chain.
pub const DEFAULT_RING_BUFFER_MAX_COUNT: u64 = 200;

const _: () = assert!(
    DEFAULT_RING_BUFFER_MAX_COUNT >= SAFETY_MARGIN,
    "DEFAULT_RING_BUFFER_MAX_COUNT must be at least signet_journal_chain::SAFETY_MARGIN"
);

/// Default broadcast-subscriber lag tolerance (in journals) before the
/// chain disconnects a slow subscriber.
pub const DEFAULT_MAX_SUBSCRIBER_LAG: u64 = 100;

/// Configuration settings for the embedded journal chain.
///
/// All fields are optional. When unset, [`JournalConfig`] returns the
/// constants above via its accessors. Configurable via environment variables
/// (`SIGNET_JOURNAL_*`) or via serde for file-based config.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize, FromEnv)]
#[serde(rename_all = "camelCase", default)]
pub struct JournalConfig {
    /// Maximum total byte size of the journal ring buffer.
    #[from_env(
        var = "SIGNET_JOURNAL_RING_BUFFER_MAX_BYTES",
        desc = "Journal ring buffer byte limit [default: 67108864 (64 MiB)]",
        optional
    )]
    ring_buffer_max_bytes: Option<u64>,

    /// Maximum number of journals in the ring buffer. Values below the
    /// chain's `SAFETY_MARGIN` are clamped up.
    #[from_env(
        var = "SIGNET_JOURNAL_RING_BUFFER_MAX_COUNT",
        desc = "Journal ring buffer count limit [default: 200]",
        optional
    )]
    ring_buffer_max_count: Option<u64>,

    /// Maximum number of journals a `/journal` WebSocket subscriber may lag
    /// behind the broadcast tip before the chain closes the connection with
    /// a `Lagged` (4003) close frame. Zero is normalized to the default
    /// because the chain requires a non-zero value.
    #[from_env(
        var = "SIGNET_JOURNAL_MAX_SUBSCRIBER_LAG",
        desc = "Journal subscriber lag tolerance [default: 100, 0 means use default]",
        optional
    )]
    max_subscriber_lag: Option<u64>,
}

impl JournalConfig {
    /// Maximum total byte size of the ring buffer, falling back to
    /// [`DEFAULT_RING_BUFFER_MAX_BYTES`].
    pub const fn ring_buffer_max_bytes(&self) -> u64 {
        match self.ring_buffer_max_bytes {
            Some(bytes) => bytes,
            None => DEFAULT_RING_BUFFER_MAX_BYTES,
        }
    }

    /// Maximum ring buffer entry count, falling back to
    /// [`DEFAULT_RING_BUFFER_MAX_COUNT`].
    pub const fn ring_buffer_max_count(&self) -> u64 {
        match self.ring_buffer_max_count {
            Some(count) => count,
            None => DEFAULT_RING_BUFFER_MAX_COUNT,
        }
    }

    /// Subscriber lag tolerance, falling back to
    /// [`DEFAULT_MAX_SUBSCRIBER_LAG`]. Zero is normalized to the default
    /// because the chain requires a non-zero value.
    pub const fn max_subscriber_lag(&self) -> NonZeroU64 {
        let value = match self.max_subscriber_lag {
            Some(0) | None => DEFAULT_MAX_SUBSCRIBER_LAG,
            Some(lag) => lag,
        };
        NonZeroU64::new(value).expect("DEFAULT_MAX_SUBSCRIBER_LAG is non-zero")
    }

    /// Emit a warning for any field that is explicitly set to a value the
    /// journal chain will silently normalize. Covers a zero
    /// `max_subscriber_lag` (which the chain rejects, so the default is
    /// substituted) and a `ring_buffer_max_count` below [`SAFETY_MARGIN`]
    /// (which the chain clamps up). Intended to be called once at startup.
    pub fn warn_on_misconfiguration(&self) {
        if self.max_subscriber_lag == Some(0) {
            warn!(
                default = DEFAULT_MAX_SUBSCRIBER_LAG,
                "SIGNET_JOURNAL_MAX_SUBSCRIBER_LAG=0 is not a valid lag tolerance; \
                 falling back to the default"
            );
        }
        if let Some(configured) = self.ring_buffer_max_count
            && configured < SAFETY_MARGIN
        {
            warn!(
                configured,
                safety_margin = SAFETY_MARGIN,
                "SIGNET_JOURNAL_RING_BUFFER_MAX_COUNT is below the journal chain's safety \
                 margin and will be clamped up"
            );
        }
    }
}
