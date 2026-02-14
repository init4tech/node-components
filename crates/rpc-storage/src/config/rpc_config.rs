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

impl Default for StorageRpcConfig {
    fn default() -> Self {
        Self {
            rpc_gas_cap: 30_000_000,
            max_blocks_per_filter: 10_000,
            max_logs_per_response: 20_000,
            max_tracing_requests: 25,
            stale_filter_ttl: Duration::from_secs(5 * 60),
            gas_oracle_block_count: 20,
            gas_oracle_percentile: 60.0,
            max_header_history: 1024,
            max_block_history: 1024,
            default_bundle_timeout_ms: 1000,
        }
    }
}
