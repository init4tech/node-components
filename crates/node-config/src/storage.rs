use init4_bin_base::utils::from_env::FromEnv;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use signet_storage::SqlConnector;
use signet_storage::{DatabaseEnv, MdbxConnector, UnifiedStorage, builder::StorageBuilder};
use std::borrow::Cow;
use tokio_util::sync::CancellationToken;

/// Configuration for signet unified storage.
///
/// Reads hot and cold storage configuration from environment variables.
///
/// # Environment Variables
///
/// - `SIGNET_HOT_PATH` – Path to the hot MDBX database.
/// - `SIGNET_COLD_PATH` – Path to the cold MDBX database (mutually exclusive
///   with SQL URL).
/// - `SIGNET_COLD_SQL_URL` – SQL connection URL for cold storage (requires
///   `postgres` or `sqlite` feature).
///
/// Exactly one of `SIGNET_COLD_PATH` or `SIGNET_COLD_SQL_URL` must be set.
///
/// # Example
///
/// ```rust,no_run
/// # use signet_node_config::StorageConfig;
/// # use tokio_util::sync::CancellationToken;
/// # async fn example(cfg: &StorageConfig) -> eyre::Result<()> {
/// let cancel = CancellationToken::new();
/// let storage = cfg.build_storage(cancel).await?;
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

    /// SQL connection URL for cold storage (requires `postgres` or `sqlite`
    /// feature).
    #[from_env(
        var = "SIGNET_COLD_SQL_URL",
        desc = "SQL connection URL for cold storage",
        infallible
    )]
    #[serde(default)]
    cold_sql_url: Cow<'static, str>,
}

impl StorageConfig {
    /// Create a new storage configuration with MDBX cold backend.
    pub const fn new(hot_path: Cow<'static, str>, cold_path: Cow<'static, str>) -> Self {
        Self { hot_path, cold_path, cold_sql_url: Cow::Borrowed("") }
    }

    /// Get the hot storage path.
    pub fn hot_path(&self) -> &str {
        &self.hot_path
    }

    /// Get the cold storage path.
    pub fn cold_path(&self) -> &str {
        &self.cold_path
    }

    /// Get the cold SQL connection URL.
    pub fn cold_sql_url(&self) -> &str {
        &self.cold_sql_url
    }

    /// Build unified storage from this configuration.
    ///
    /// Creates connectors from the configured paths, spawns the cold storage
    /// background task, and returns a [`UnifiedStorage`] ready for use.
    ///
    /// Exactly one of `cold_path` or `cold_sql_url` must be non-empty.
    pub async fn build_storage(
        &self,
        cancel: CancellationToken,
    ) -> eyre::Result<UnifiedStorage<DatabaseEnv>> {
        let hot = MdbxConnector::new(self.hot_path.as_ref());
        let has_mdbx = !self.cold_path.is_empty();
        let has_sql = !self.cold_sql_url.is_empty();

        match (has_mdbx, has_sql) {
            (true, false) => Ok(StorageBuilder::new()
                .hot(hot)
                .cold(MdbxConnector::new(self.cold_path.as_ref()))
                .cancel_token(cancel)
                .build()
                .await?),
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            (false, true) => Ok(StorageBuilder::new()
                .hot(hot)
                .cold(SqlConnector::new(self.cold_sql_url.as_ref()))
                .cancel_token(cancel)
                .build()
                .await?),
            #[cfg(not(any(feature = "postgres", feature = "sqlite")))]
            (false, true) => {
                eyre::bail!("SIGNET_COLD_SQL_URL requires the 'postgres' or 'sqlite' feature")
            }
            (true, true) => eyre::bail!(
                "both SIGNET_COLD_PATH and SIGNET_COLD_SQL_URL are set; specify exactly one"
            ),
            (false, false) => eyre::bail!(
                "neither SIGNET_COLD_PATH nor SIGNET_COLD_SQL_URL is set; specify exactly one"
            ),
        }
    }
}
