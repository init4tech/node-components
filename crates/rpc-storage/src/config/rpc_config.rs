//! Configuration for the storage-backed RPC server.

use std::time::Duration;

/// Configuration for the storage-backed ETH RPC server.
///
/// Mirrors the subset of reth's `EthConfig` that applies to
/// storage-backed RPC.
///
/// # Example
///
/// ```
/// use signet_rpc_storage::StorageRpcConfig;
///
/// // Use defaults (matches reth defaults).
/// let config = StorageRpcConfig::default();
/// assert_eq!(config.rpc_gas_cap, 30_000_000);
///
/// // Use the builder to customise individual fields.
/// let config = StorageRpcConfig::builder()
///     .rpc_gas_cap(50_000_000)
///     .max_blocks_per_filter(5_000)
///     .build();
/// assert_eq!(config.rpc_gas_cap, 50_000_000);
/// assert_eq!(config.max_blocks_per_filter, 5_000);
/// // Other fields retain their defaults.
/// assert_eq!(config.max_logs_per_response, 20_000);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct StorageRpcConfig {
    /// Maximum gas for `eth_call` and `eth_estimateGas`.
    ///
    /// Default: `30_000_000` (30M gas).
    pub rpc_gas_cap: u64,

    /// Maximum block range per `eth_getLogs` query.
    ///
    /// Default: `10_000`.
    pub max_blocks_per_filter: u64,

    /// Maximum number of logs returned per `eth_getLogs` response.
    /// Set to `0` to disable the limit.
    ///
    /// Default: `20_000`.
    pub max_logs_per_response: usize,

    /// Maximum wall-clock time for a single log query.
    ///
    /// If a log query exceeds this duration, the stream is terminated
    /// early and the handler returns a deadline-exceeded error.
    ///
    /// Default: `10` seconds.
    pub max_log_query_deadline: Duration,

    /// Maximum concurrent tracing/debug requests.
    ///
    /// Controls the size of the semaphore that gates debug
    /// namespace calls.
    ///
    /// Default: `25`.
    pub max_tracing_requests: usize,

    /// Time-to-live for stale filters and subscriptions.
    ///
    /// Default: `5 minutes`.
    pub stale_filter_ttl: Duration,

    /// Number of recent blocks to consider for gas price suggestions.
    ///
    /// Default: `20`.
    pub gas_oracle_block_count: u64,

    /// Percentile of effective tips to use as the gas price suggestion.
    ///
    /// Default: `60.0`.
    pub gas_oracle_percentile: f64,

    /// Default gas price returned when no recent transactions exist.
    ///
    /// Reth defaults to 1 Gwei. Set to `None` to return zero.
    ///
    /// Default: `Some(1_000_000_000)` (1 Gwei).
    pub default_gas_price: Option<u128>,

    /// Minimum effective tip to include in the oracle sample.
    ///
    /// Tips below this threshold are discarded, matching reth's
    /// `ignore_price` behavior.
    ///
    /// Default: `Some(2)` (2 wei).
    pub ignore_price: Option<u128>,

    /// Maximum gas price the oracle will ever suggest.
    ///
    /// Default: `Some(500_000_000_000)` (500 Gwei).
    pub max_price: Option<u128>,

    /// Maximum header history for `eth_feeHistory` without percentiles.
    ///
    /// Default: `1024`.
    pub max_header_history: u64,

    /// Maximum block history for `eth_feeHistory` with percentiles.
    ///
    /// Default: `1024`.
    pub max_block_history: u64,

    /// Default timeout in milliseconds for bundle simulation.
    ///
    /// Used when the bundle request does not specify its own timeout.
    ///
    /// Default: `1000` (1 second).
    pub default_bundle_timeout_ms: u64,
}

impl StorageRpcConfig {
    /// Create a new builder with all fields set to their defaults.
    pub fn builder() -> StorageRpcConfigBuilder {
        StorageRpcConfigBuilder::default()
    }
}

impl Default for StorageRpcConfig {
    fn default() -> Self {
        Self {
            rpc_gas_cap: 30_000_000,
            max_blocks_per_filter: 10_000,
            max_logs_per_response: 20_000,
            max_log_query_deadline: Duration::from_secs(10),
            max_tracing_requests: 25,
            stale_filter_ttl: Duration::from_secs(5 * 60),
            gas_oracle_block_count: 20,
            gas_oracle_percentile: 60.0,
            default_gas_price: Some(1_000_000_000),
            ignore_price: Some(2),
            max_price: Some(500_000_000_000),
            max_header_history: 1024,
            max_block_history: 1024,
            default_bundle_timeout_ms: 1000,
        }
    }
}

/// Builder for [`StorageRpcConfig`].
///
/// All fields default to the same values as [`StorageRpcConfig::default`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StorageRpcConfigBuilder {
    inner: StorageRpcConfig,
}

impl StorageRpcConfigBuilder {
    /// Set the maximum gas for `eth_call` and `eth_estimateGas`.
    pub const fn rpc_gas_cap(mut self, cap: u64) -> Self {
        self.inner.rpc_gas_cap = cap;
        self
    }

    /// Set the maximum block range per `eth_getLogs` query.
    pub const fn max_blocks_per_filter(mut self, max: u64) -> Self {
        self.inner.max_blocks_per_filter = max;
        self
    }

    /// Set the maximum number of logs returned per response.
    pub const fn max_logs_per_response(mut self, max: usize) -> Self {
        self.inner.max_logs_per_response = max;
        self
    }

    /// Set the maximum wall-clock time for a single log query.
    pub const fn max_log_query_deadline(mut self, deadline: Duration) -> Self {
        self.inner.max_log_query_deadline = deadline;
        self
    }

    /// Set the maximum concurrent tracing/debug requests.
    pub const fn max_tracing_requests(mut self, max: usize) -> Self {
        self.inner.max_tracing_requests = max;
        self
    }

    /// Set the time-to-live for stale filters and subscriptions.
    pub const fn stale_filter_ttl(mut self, ttl: Duration) -> Self {
        self.inner.stale_filter_ttl = ttl;
        self
    }

    /// Set the number of recent blocks for gas price suggestions.
    pub const fn gas_oracle_block_count(mut self, count: u64) -> Self {
        self.inner.gas_oracle_block_count = count;
        self
    }

    /// Set the percentile of effective tips for gas price suggestions.
    pub const fn gas_oracle_percentile(mut self, percentile: f64) -> Self {
        self.inner.gas_oracle_percentile = percentile;
        self
    }

    /// Set the maximum header history for `eth_feeHistory`.
    pub const fn max_header_history(mut self, max: u64) -> Self {
        self.inner.max_header_history = max;
        self
    }

    /// Set the maximum block history for `eth_feeHistory`.
    pub const fn max_block_history(mut self, max: u64) -> Self {
        self.inner.max_block_history = max;
        self
    }

    /// Set the default bundle simulation timeout in milliseconds.
    pub const fn default_bundle_timeout_ms(mut self, ms: u64) -> Self {
        self.inner.default_bundle_timeout_ms = ms;
        self
    }

    /// Build the configuration.
    pub const fn build(self) -> StorageRpcConfig {
        self.inner
    }
}
