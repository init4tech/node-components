use init4_bin_base::utils::from_env::FromEnv;
use signet_cold::ColdStorageTask;
use signet_cold_mdbx::MdbxColdBackend;
use signet_hot_mdbx::{DatabaseArguments, DatabaseEnv};
use signet_storage::UnifiedStorage;
use std::borrow::Cow;
use tokio_util::sync::CancellationToken;

/// Configuration for signet unified storage.
///
/// Reads hot and cold MDBX paths from environment variables.
///
/// # Environment Variables
///
/// - `SIGNET_HOT_PATH` – Path to the hot MDBX database.
/// - `SIGNET_COLD_PATH` – Path to the cold MDBX database.
///
/// # Example
///
/// ```rust,no_run
/// # use signet_node_config::StorageConfig;
/// # use tokio_util::sync::CancellationToken;
/// # fn example(cfg: &StorageConfig) -> eyre::Result<()> {
/// let cancel = CancellationToken::new();
/// let storage = cfg.build_storage(cancel)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, serde::Deserialize, FromEnv)]
pub struct StorageConfig {
    /// Path to the hot MDBX database.
    #[from_env(var = "SIGNET_HOT_PATH", desc = "Path to hot MDBX storage", infallible)]
    hot_path: Cow<'static, str>,

    /// Path to the cold MDBX database.
    #[from_env(var = "SIGNET_COLD_PATH", desc = "Path to cold MDBX storage", infallible)]
    cold_path: Cow<'static, str>,
}

impl StorageConfig {
    /// Create a new storage configuration.
    pub const fn new(hot_path: Cow<'static, str>, cold_path: Cow<'static, str>) -> Self {
        Self { hot_path, cold_path }
    }

    /// Get the hot storage path.
    pub fn hot_path(&self) -> &str {
        &self.hot_path
    }

    /// Get the cold storage path.
    pub fn cold_path(&self) -> &str {
        &self.cold_path
    }

    /// Build unified storage from this configuration.
    ///
    /// Opens an MDBX read-write environment for both hot and cold storage,
    /// spawns the cold storage background task, and returns a
    /// [`UnifiedStorage`] ready for use.
    pub fn build_storage(
        &self,
        cancel: CancellationToken,
    ) -> eyre::Result<UnifiedStorage<DatabaseEnv>> {
        let hot = DatabaseArguments::new().open_rw(self.hot_path.as_ref().as_ref())?;
        let cold_backend = MdbxColdBackend::open_rw(self.cold_path.as_ref().as_ref())?;
        let cold_handle = ColdStorageTask::spawn(cold_backend, cancel);
        Ok(UnifiedStorage::new(hot, cold_handle))
    }
}
