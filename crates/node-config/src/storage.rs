use init4_bin_base::utils::from_env::FromEnv;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use signet_storage::SqlConnector;
use signet_storage::{DatabaseEnv, MdbxConnector, UnifiedStorage, builder::StorageBuilder};
use std::borrow::Cow;
use tokio_util::sync::CancellationToken;

// Pool-tuning defaults, only compiled when an SQL backend is available.
#[cfg(any(feature = "postgres", feature = "sqlite"))]
mod pool_defaults {
    /// Maximum number of connections in the SQL cold storage pool.
    pub(super) const MAX_CONNECTIONS: u32 = 100;
    /// Minimum number of connections in the SQL cold storage pool.
    pub(super) const MIN_CONNECTIONS: u32 = 5;
    /// Timeout (in seconds) for acquiring a connection from the pool.
    pub(super) const ACQUIRE_TIMEOUT_SECS: u64 = 5;
    /// Idle timeout (in seconds) before closing unused connections.
    pub(super) const IDLE_TIMEOUT_SECS: u64 = 600;
    /// Maximum lifetime (in seconds) of individual connections.
    pub(super) const MAX_LIFETIME_SECS: u64 = 1800;
}

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
/// ## SQL Connection Pool Tuning
///
/// When using SQL cold storage, the following optional variables control
/// the connection pool:
///
/// - `SIGNET_COLD_SQL_MAX_CONNECTIONS` – Maximum pool size (default: 100).
/// - `SIGNET_COLD_SQL_MIN_CONNECTIONS` – Minimum idle connections (default: 5).
/// - `SIGNET_COLD_SQL_ACQUIRE_TIMEOUT_SECS` – Timeout for acquiring a
///   connection (default: 5 s).
/// - `SIGNET_COLD_SQL_IDLE_TIMEOUT_SECS` – Idle timeout before closing a
///   connection (default: 600 s).
/// - `SIGNET_COLD_SQL_MAX_LIFETIME_SECS` – Maximum lifetime of a connection
///   before it is recycled (default: 1800 s).
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

    /// Maximum number of connections in the SQL pool.
    #[from_env(
        var = "SIGNET_COLD_SQL_MAX_CONNECTIONS",
        desc = "Max SQL pool connections",
        optional
    )]
    #[serde(default)]
    #[cfg_attr(not(any(feature = "postgres", feature = "sqlite")), allow(dead_code))]
    cold_sql_max_connections: Option<u32>,

    /// Minimum number of idle connections to maintain.
    #[from_env(
        var = "SIGNET_COLD_SQL_MIN_CONNECTIONS",
        desc = "Min SQL pool connections",
        optional
    )]
    #[serde(default)]
    #[cfg_attr(not(any(feature = "postgres", feature = "sqlite")), allow(dead_code))]
    cold_sql_min_connections: Option<u32>,

    /// Connection acquire timeout in seconds.
    #[from_env(
        var = "SIGNET_COLD_SQL_ACQUIRE_TIMEOUT_SECS",
        desc = "SQL pool acquire timeout (seconds)",
        optional
    )]
    #[serde(default)]
    #[cfg_attr(not(any(feature = "postgres", feature = "sqlite")), allow(dead_code))]
    cold_sql_acquire_timeout_secs: Option<u64>,

    /// Idle connection timeout in seconds.
    #[from_env(
        var = "SIGNET_COLD_SQL_IDLE_TIMEOUT_SECS",
        desc = "SQL pool idle timeout (seconds)",
        optional
    )]
    #[serde(default)]
    #[cfg_attr(not(any(feature = "postgres", feature = "sqlite")), allow(dead_code))]
    cold_sql_idle_timeout_secs: Option<u64>,

    /// Maximum lifetime of a connection in seconds.
    #[from_env(
        var = "SIGNET_COLD_SQL_MAX_LIFETIME_SECS",
        desc = "SQL pool max connection lifetime (seconds)",
        optional
    )]
    #[serde(default)]
    #[cfg_attr(not(any(feature = "postgres", feature = "sqlite")), allow(dead_code))]
    cold_sql_max_lifetime_secs: Option<u64>,
}

impl StorageConfig {
    /// Create a new storage configuration with MDBX cold backend.
    pub const fn new(hot_path: Cow<'static, str>, cold_path: Cow<'static, str>) -> Self {
        Self {
            hot_path,
            cold_path,
            cold_sql_url: Cow::Borrowed(""),
            cold_sql_max_connections: None,
            cold_sql_min_connections: None,
            cold_sql_acquire_timeout_secs: None,
            cold_sql_idle_timeout_secs: None,
            cold_sql_max_lifetime_secs: None,
        }
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

    /// Build a [`SqlConnector`] with pool settings from this configuration.
    #[cfg(any(feature = "postgres", feature = "sqlite"))]
    fn build_sql_connector(&self) -> SqlConnector {
        use pool_defaults as d;
        use std::time::Duration;

        let max_conns = self.cold_sql_max_connections.unwrap_or(d::MAX_CONNECTIONS);
        let min_conns = self.cold_sql_min_connections.unwrap_or(d::MIN_CONNECTIONS);
        let acquire = self.cold_sql_acquire_timeout_secs.unwrap_or(d::ACQUIRE_TIMEOUT_SECS);
        let idle = self.cold_sql_idle_timeout_secs.unwrap_or(d::IDLE_TIMEOUT_SECS);
        let lifetime = self.cold_sql_max_lifetime_secs.unwrap_or(d::MAX_LIFETIME_SECS);

        SqlConnector::new(self.cold_sql_url.as_ref())
            .with_max_connections(max_conns)
            .with_min_connections(min_conns)
            .with_acquire_timeout(Duration::from_secs(acquire))
            .with_idle_timeout(Some(Duration::from_secs(idle)))
            .with_max_lifetime(Some(Duration::from_secs(lifetime)))
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
                .cold(self.build_sql_connector())
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
