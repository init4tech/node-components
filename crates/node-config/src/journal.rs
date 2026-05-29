use core::{num::NonZeroU64, str::FromStr, time::Duration};
use init4_bin_base::utils::from_env::{FromEnv, FromEnvErr, FromEnvVar};
use signet_journal_chain::SAFETY_MARGIN;
use tracing::warn;

/// How a node sources rollup state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStrategy {
    /// Execute host blocks to derive state (the current, default behaviour).
    #[default]
    Blocks,
    /// Apply pre-computed journals from upstream sources without executing blocks.
    Journals,
}

impl FromStr for SyncStrategy {
    type Err = ParseSyncStrategyError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.trim().to_ascii_lowercase().as_str() {
            "blocks" => Ok(Self::Blocks),
            "journals" => Ok(Self::Journals),
            other => Err(ParseSyncStrategyError(other.to_owned())),
        }
    }
}

impl FromEnvVar for SyncStrategy {
    fn from_env_var(env_var: &str) -> Result<Self, FromEnvErr> {
        let raw = String::from_env_var(env_var)?;
        raw.parse().map_err(|error| FromEnvErr::parse_error(env_var, error))
    }
}

/// Error parsing a [`SyncStrategy`] from a string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid journal sync strategy '{0}', expected 'blocks' or 'journals'")]
pub struct ParseSyncStrategyError(String);

/// Error returned by [`JournalConfig::validate`].
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum JournalConfigError {
    /// `sync_strategy` is [`SyncStrategy::Journals`] but no upstream sources were configured.
    #[error(
        "journal sync strategy is 'journals' but no upstream sources were configured \
         (set SIGNET_JOURNAL_SOURCES)"
    )]
    MissingSources,
}

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
#[derive(Debug, Clone, Default, serde::Deserialize, FromEnv)]
#[serde(rename_all = "camelCase", default)]
pub struct JournalConfig {
    /// Sync strategy: execute host blocks (`blocks`, default) or apply journals
    /// from upstream sources (`journals`).
    #[from_env(
        var = "SIGNET_JOURNAL_SYNC_STRATEGY",
        desc = "Journal sync strategy: 'blocks' or 'journals' [default: blocks]",
        optional
    )]
    sync_strategy: Option<SyncStrategy>,

    /// Prioritised upstream journal WebSocket source URLs (comma-separated).
    /// Required when `sync_strategy` is `journals`.
    #[from_env(
        var = "SIGNET_JOURNAL_SOURCES",
        desc = "Comma-separated upstream journal WebSocket URLs (required for journals strategy)",
        optional
    )]
    sources: Option<Vec<String>>,

    /// Per-source stall timeout in milliseconds for the journal client. Falls
    /// back to the client default (60s) when unset.
    #[from_env(
        var = "SIGNET_JOURNAL_CLIENT_SOURCE_STALL_TIMEOUT_MS",
        desc = "Journal client per-source stall timeout in ms [default: 60000]",
        optional
    )]
    client_source_stall_timeout_ms: Option<u64>,

    /// Faulty-source backoff in milliseconds for the journal client. Falls back
    /// to the client default (30s) when unset.
    #[from_env(
        var = "SIGNET_JOURNAL_CLIENT_SOURCE_BACKOFF_MS",
        desc = "Journal client faulty-source backoff in ms [default: 30000]",
        optional
    )]
    client_source_backoff_ms: Option<u64>,

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

    /// The configured sync strategy, defaulting to [`SyncStrategy::Blocks`].
    pub fn sync_strategy(&self) -> SyncStrategy {
        self.sync_strategy.unwrap_or_default()
    }

    /// Upstream journal WebSocket source URLs (as raw strings). Empty when none
    /// are configured. Required when [`Self::sync_strategy`] is
    /// [`SyncStrategy::Journals`].
    pub fn sources(&self) -> &[String] {
        self.sources.as_deref().unwrap_or(&[])
    }

    /// Per-source stall timeout for the journal client, when overridden. `None`
    /// lets the client's own default (60s) stand.
    pub fn client_source_stall_timeout(&self) -> Option<Duration> {
        self.client_source_stall_timeout_ms.map(Duration::from_millis)
    }

    /// Faulty-source backoff for the journal client, when overridden. `None`
    /// lets the client's own default (30s) stand.
    pub fn client_source_backoff(&self) -> Option<Duration> {
        self.client_source_backoff_ms.map(Duration::from_millis)
    }

    /// Emit a warning for any field that is explicitly set to a value the
    /// journal chain will silently normalize. Covers a zero
    /// `max_subscriber_lag` (which the chain rejects, so the default is
    /// substituted) and a `ring_buffer_max_count` below [`SAFETY_MARGIN`]
    /// (which the chain clamps up). Also warns when journal-client-only or
    /// `journals`-strategy-only options are set but the strategy will ignore
    /// them. Intended to be called once at startup.
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
        // The journal-sync inputs (sources and client tuning knobs) are only consulted under
        // the `journals` strategy. If they are set while the node will execute blocks, they are
        // dead config - surface that rather than silently ignoring them.
        if self.sync_strategy() != SyncStrategy::Journals {
            if !self.sources().is_empty() {
                warn!(
                    "SIGNET_JOURNAL_SOURCES is set but the sync strategy is not 'journals'; \
                     the configured sources will be ignored"
                );
            }
            if self.client_source_stall_timeout_ms.is_some()
                || self.client_source_backoff_ms.is_some()
            {
                warn!(
                    "journal client tuning knobs are set but the sync strategy is not \
                     'journals'; they will be ignored"
                );
            }
        }
    }

    /// Validate cross-field invariants. Intended to be called once at startup,
    /// after [`Self::warn_on_misconfiguration`].
    ///
    /// # Errors
    ///
    /// Returns [`JournalConfigError::MissingSources`] when the strategy is
    /// [`SyncStrategy::Journals`] but no upstream sources are configured.
    pub fn validate(&self) -> Result<(), JournalConfigError> {
        if self.sync_strategy() == SyncStrategy::Journals && self.sources().is_empty() {
            return Err(JournalConfigError::MissingSources);
        }
        Ok(())
    }

    /// Construct a journal-sync configuration ([`SyncStrategy::Journals`]) pointing at the given
    /// upstream sources. The client stall timeout is deliberately generous so a working but
    /// idle source (e.g. a test that has served all its journals and is waiting to be torn
    /// down) is never mistaken for a dead one and exhausted mid-test. All other fields take
    /// their defaults. Use [`Self::journal_sync_for_test_fail_fast`] to exercise exhaustion.
    #[cfg(any(test, feature = "test_utils"))]
    pub fn journal_sync_for_test(sources: Vec<String>) -> Self {
        Self {
            sync_strategy: Some(SyncStrategy::Journals),
            sources: Some(sources),
            client_source_stall_timeout_ms: Some(30_000),
            client_source_backoff_ms: Some(100),
            ..Default::default()
        }
    }

    /// Like [`Self::journal_sync_for_test`] but with short client timeouts so a node pointed at
    /// dead sources exhausts them quickly. Only for tests that assert on source exhaustion;
    /// other tests should use [`Self::journal_sync_for_test`] to avoid a spurious exhaustion
    /// racing test teardown.
    #[cfg(any(test, feature = "test_utils"))]
    pub fn journal_sync_for_test_fail_fast(sources: Vec<String>) -> Self {
        Self {
            sync_strategy: Some(SyncStrategy::Journals),
            sources: Some(sources),
            client_source_stall_timeout_ms: Some(200),
            client_source_backoff_ms: Some(50),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_strategy_parses_case_insensitively() {
        assert_eq!("blocks".parse::<SyncStrategy>().unwrap(), SyncStrategy::Blocks);
        assert_eq!("Journals".parse::<SyncStrategy>().unwrap(), SyncStrategy::Journals);
        assert_eq!("  JOURNALS  ".parse::<SyncStrategy>().unwrap(), SyncStrategy::Journals);
        "neither".parse::<SyncStrategy>().unwrap_err();
    }

    #[test]
    fn default_strategy_is_blocks() {
        assert_eq!(JournalConfig::default().sync_strategy(), SyncStrategy::Blocks);
    }

    #[test]
    fn validate_requires_sources_for_journals() {
        let config =
            JournalConfig { sync_strategy: Some(SyncStrategy::Journals), ..Default::default() };
        config.validate().unwrap_err();

        let config = JournalConfig {
            sync_strategy: Some(SyncStrategy::Journals),
            sources: Some(vec!["ws://host:9545".to_owned()]),
            ..Default::default()
        };
        config.validate().unwrap();
    }

    #[test]
    fn validate_allows_blocks_without_sources() {
        JournalConfig::default().validate().unwrap();
    }

    #[test]
    fn client_timeouts_convert_from_millis() {
        let config = JournalConfig {
            client_source_stall_timeout_ms: Some(1500),
            client_source_backoff_ms: Some(250),
            ..Default::default()
        };
        assert_eq!(config.client_source_stall_timeout(), Some(Duration::from_millis(1500)));
        assert_eq!(config.client_source_backoff(), Some(Duration::from_millis(250)));
        assert_eq!(JournalConfig::default().client_source_stall_timeout(), None);
    }
}
