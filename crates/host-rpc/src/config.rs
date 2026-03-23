use crate::{DEFAULT_BACKFILL_BATCH_SIZE, DEFAULT_BUFFER_CAPACITY, RpcHostNotifierBuilder};
use alloy::providers::RootProvider;
use init4_bin_base::utils::{calc::SlotCalculator, from_env::FromEnv, provider::PubSubConfig};

/// Environment-based configuration for the RPC host notifier.
///
/// # Environment Variables
///
/// - `SIGNET_HOST_URL` – WebSocket or IPC URL for the host EL client (required)
/// - `SIGNET_HOST_BUFFER_CAPACITY` – Local chain view size (default: 64)
/// - `SIGNET_HOST_BACKFILL_BATCH_SIZE` – Blocks per backfill batch (default: 32)
///
/// # Example
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use signet_host_rpc::HostRpcConfig;
/// use init4_bin_base::utils::{calc::SlotCalculator, from_env::FromEnv};
///
/// let config = HostRpcConfig::from_env().unwrap();
/// let slot_calculator = SlotCalculator::new(0, 1_606_824_023, 12);
/// let builder = config.into_builder(slot_calculator).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, FromEnv)]
pub struct HostRpcConfig {
    /// WebSocket or IPC connection to the host execution layer client.
    #[from_env(var = "SIGNET_HOST_URL", desc = "Host EL pubsub URL (ws:// or ipc) [required]")]
    provider: PubSubConfig,
    /// Local chain view buffer capacity.
    #[from_env(
        var = "SIGNET_HOST_BUFFER_CAPACITY",
        desc = "Chain view buffer capacity [default: 64]",
        optional
    )]
    buffer_capacity: Option<usize>,
    /// Blocks per backfill RPC batch.
    #[from_env(
        var = "SIGNET_HOST_BACKFILL_BATCH_SIZE",
        desc = "Backfill batch size [default: 32]",
        optional
    )]
    backfill_batch_size: Option<u64>,
}

impl HostRpcConfig {
    /// Connect to the host provider and build an [`RpcHostNotifierBuilder`].
    ///
    /// Uses `slot_calculator` for genesis timestamp rather than
    /// duplicating that setting.
    pub async fn into_builder(
        self,
        slot_calculator: SlotCalculator,
    ) -> Result<RpcHostNotifierBuilder<RootProvider>, alloy::transports::TransportError> {
        let provider = self.provider.connect().await?;
        Ok(RpcHostNotifierBuilder::new(provider)
            .with_buffer_capacity(self.buffer_capacity.unwrap_or(DEFAULT_BUFFER_CAPACITY))
            .with_backfill_batch_size(
                self.backfill_batch_size.unwrap_or(DEFAULT_BACKFILL_BATCH_SIZE),
            )
            .with_genesis_timestamp(slot_calculator.start_timestamp()))
    }
}
