//! Configuration for the storage-backed RPC server.

use init4_bin_base::utils::from_env::FromEnv;
use std::time::Duration;

/// Configuration for the storage-backed ETH RPC server.
///
/// Mirrors the subset of reth's `EthConfig` that applies to
/// storage-backed RPC.
///
/// # Example
///
/// ```
/// use signet_rpc::StorageRpcConfig;
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

    /// Maximum block range for `trace_filter` queries.
    ///
    /// Default: `100`.
    pub max_trace_filter_blocks: u64,

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
    /// Reth defaults to 1 Gwei. Set to `None` to disable (returns
    /// zero). When configured via environment variable, set to `0` to
    /// disable.
    ///
    /// Default: `Some(1_000_000_000)` (1 Gwei).
    pub default_gas_price: Option<u128>,

    /// Minimum effective tip to include in the oracle sample.
    ///
    /// Tips below this threshold are discarded, matching reth's
    /// `ignore_price` behavior. Set to `None` to include all tips.
    /// When configured via environment variable, set to `0` to
    /// disable.
    ///
    /// Default: `Some(2)` (2 wei).
    pub ignore_price: Option<u128>,

    /// Maximum gas price the oracle will ever suggest.
    ///
    /// Set to `None` for no cap. When configured via environment
    /// variable, set to `0` to disable.
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
            max_trace_filter_blocks: 100,
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

    /// Set the max block range for trace_filter.
    pub const fn max_trace_filter_blocks(mut self, max: u64) -> Self {
        self.inner.max_trace_filter_blocks = max;
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

    /// Set the default gas price returned when no recent transactions exist.
    pub const fn default_gas_price(mut self, price: Option<u128>) -> Self {
        self.inner.default_gas_price = price;
        self
    }

    /// Set the minimum effective tip to include in the oracle sample.
    pub const fn ignore_price(mut self, price: Option<u128>) -> Self {
        self.inner.ignore_price = price;
        self
    }

    /// Set the maximum gas price the oracle will ever suggest.
    pub const fn max_price(mut self, price: Option<u128>) -> Self {
        self.inner.max_price = price;
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

/// Environment-based configuration for the storage RPC server.
///
/// All fields are optional and default to the same values as
/// [`StorageRpcConfig::default`].
///
/// # Example
///
/// ```no_run
/// use signet_rpc::{StorageRpcConfig, StorageRpcConfigEnv};
/// use init4_bin_base::utils::from_env::FromEnv;
///
/// let config: StorageRpcConfig = StorageRpcConfigEnv::from_env().unwrap().into();
/// ```
#[derive(Debug, Clone, FromEnv)]
pub struct StorageRpcConfigEnv {
    /// Maximum gas for `eth_call` and `eth_estimateGas`.
    #[from_env(
        var = "SIGNET_RPC_GAS_CAP",
        desc = "Max gas for eth_call [default: 30000000]",
        optional
    )]
    rpc_gas_cap: Option<u64>,
    /// Maximum block range per `eth_getLogs` query.
    #[from_env(
        var = "SIGNET_RPC_MAX_BLOCKS_PER_FILTER",
        desc = "Max block range for getLogs [default: 10000]",
        optional
    )]
    max_blocks_per_filter: Option<u64>,
    /// Maximum number of logs returned per response.
    #[from_env(
        var = "SIGNET_RPC_MAX_LOGS",
        desc = "Max logs per response [default: 20000]",
        optional
    )]
    max_logs_per_response: Option<u64>,
    /// Maximum seconds for a single log query.
    #[from_env(
        var = "SIGNET_RPC_LOG_QUERY_DEADLINE_SECS",
        desc = "Max seconds for log query [default: 10]",
        optional
    )]
    max_log_query_deadline_secs: Option<u64>,
    /// Maximum concurrent tracing/debug requests.
    #[from_env(
        var = "SIGNET_RPC_MAX_TRACING_REQUESTS",
        desc = "Concurrent tracing limit [default: 25]",
        optional
    )]
    max_tracing_requests: Option<u64>,
    /// Maximum block range for trace_filter queries.
    #[from_env(
        var = "SIGNET_RPC_MAX_TRACE_FILTER_BLOCKS",
        desc = "Maximum block range for trace_filter queries [default: 100]",
        optional
    )]
    max_trace_filter_blocks: Option<u64>,
    /// Filter TTL in seconds.
    #[from_env(
        var = "SIGNET_RPC_STALE_FILTER_TTL_SECS",
        desc = "Filter TTL in seconds [default: 300]",
        optional
    )]
    stale_filter_ttl_secs: Option<u64>,
    /// Number of recent blocks for gas oracle.
    #[from_env(
        var = "SIGNET_RPC_GAS_ORACLE_BLOCKS",
        desc = "Blocks for gas oracle [default: 20]",
        optional
    )]
    gas_oracle_block_count: Option<u64>,
    /// Tip percentile for gas oracle.
    #[from_env(
        var = "SIGNET_RPC_GAS_ORACLE_PERCENTILE",
        desc = "Tip percentile [default: 60]",
        optional
    )]
    gas_oracle_percentile: Option<u64>,
    /// Default gas price in wei.
    #[from_env(
        var = "SIGNET_RPC_DEFAULT_GAS_PRICE",
        desc = "Default gas price in wei, 0 to disable [default: 1000000000]",
        optional
    )]
    default_gas_price: Option<u128>,
    /// Minimum effective tip in wei.
    #[from_env(
        var = "SIGNET_RPC_IGNORE_PRICE",
        desc = "Min tip in wei, 0 to disable [default: 2]",
        optional
    )]
    ignore_price: Option<u128>,
    /// Maximum gas price in wei.
    #[from_env(
        var = "SIGNET_RPC_MAX_PRICE",
        desc = "Max gas price in wei, 0 to disable [default: 500000000000]",
        optional
    )]
    max_price: Option<u128>,
    /// Maximum header history for `eth_feeHistory`.
    #[from_env(
        var = "SIGNET_RPC_MAX_HEADER_HISTORY",
        desc = "Max feeHistory headers [default: 1024]",
        optional
    )]
    max_header_history: Option<u64>,
    /// Maximum block history for `eth_feeHistory`.
    #[from_env(
        var = "SIGNET_RPC_MAX_BLOCK_HISTORY",
        desc = "Max feeHistory blocks [default: 1024]",
        optional
    )]
    max_block_history: Option<u64>,
    /// Default bundle simulation timeout in milliseconds.
    #[from_env(
        var = "SIGNET_RPC_BUNDLE_TIMEOUT_MS",
        desc = "Bundle sim timeout in ms [default: 1000]",
        optional
    )]
    default_bundle_timeout_ms: Option<u64>,
}

/// Map `0` to `None`, preserving all other values.
const fn nonzero(v: u128) -> Option<u128> {
    if v == 0 { None } else { Some(v) }
}

impl From<StorageRpcConfigEnv> for StorageRpcConfig {
    fn from(env: StorageRpcConfigEnv) -> Self {
        let defaults = StorageRpcConfig::default();
        Self {
            rpc_gas_cap: env.rpc_gas_cap.unwrap_or(defaults.rpc_gas_cap),
            max_blocks_per_filter: env
                .max_blocks_per_filter
                .unwrap_or(defaults.max_blocks_per_filter),
            max_logs_per_response: env
                .max_logs_per_response
                .map_or(defaults.max_logs_per_response, |v| v as usize),
            max_log_query_deadline: env
                .max_log_query_deadline_secs
                .map_or(defaults.max_log_query_deadline, Duration::from_secs),
            max_tracing_requests: env
                .max_tracing_requests
                .map_or(defaults.max_tracing_requests, |v| v as usize),
            max_trace_filter_blocks: env
                .max_trace_filter_blocks
                .unwrap_or(defaults.max_trace_filter_blocks),
            stale_filter_ttl: env
                .stale_filter_ttl_secs
                .map_or(defaults.stale_filter_ttl, Duration::from_secs),
            gas_oracle_block_count: env
                .gas_oracle_block_count
                .unwrap_or(defaults.gas_oracle_block_count),
            gas_oracle_percentile: env
                .gas_oracle_percentile
                .map_or(defaults.gas_oracle_percentile, |v| v as f64),
            default_gas_price: env.default_gas_price.map_or(defaults.default_gas_price, nonzero),
            ignore_price: env.ignore_price.map_or(defaults.ignore_price, nonzero),
            max_price: env.max_price.map_or(defaults.max_price, nonzero),
            max_header_history: env.max_header_history.unwrap_or(defaults.max_header_history),
            max_block_history: env.max_block_history.unwrap_or(defaults.max_block_history),
            default_bundle_timeout_ms: env
                .default_bundle_timeout_ms
                .unwrap_or(defaults.default_bundle_timeout_ms),
        }
    }
}
