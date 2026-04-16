use init4_bin_base::utils::from_env::FromEnv;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use signet_storage::SqlConnector;
use signet_storage::{DatabaseEnv, MdbxConnector, UnifiedStorage, builder::StorageBuilder};
use std::borrow::Cow;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use std::time::Duration;
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
/// When using SQL cold storage, connection pool tuning is configured via
/// [`SqlConnector`]'s own environment variables (e.g.
/// `SIGNET_COLD_SQL_MAX_CONNECTIONS`). See the `cold-sql` feature of
/// `init4-bin-base` for the full list.
///
/// [`SqlConnector`]: signet_storage::SqlConnector
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
#[derive(Debug, Clone, FromEnv)]
pub struct StorageConfig {
    /// Path to the hot MDBX database.
    #[from_env(var = "SIGNET_HOT_PATH", desc = "Path to hot MDBX storage", infallible)]
    hot_path: Cow<'static, str>,

    /// Path to the cold MDBX database.
    #[from_env(var = "SIGNET_COLD_PATH", desc = "Path to cold MDBX storage", infallible)]
    cold_path: Cow<'static, str>,

    /// Pre-configured SQL connector with pool settings. Populated
    /// automatically from environment variables via [`FromEnv`], or from
    /// the pool-tuning fields when deserializing via serde.
    #[cfg(any(feature = "postgres", feature = "sqlite"))]
    cold_sql: Option<SqlConnector>,
}

impl<'de> serde::Deserialize<'de> for StorageConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Flat helper that mirrors the env-var layout so config files use the
        /// same field names operators already know.
        #[derive(serde::Deserialize)]
        struct Helper {
            hot_path: Cow<'static, str>,
            cold_path: Cow<'static, str>,
            #[serde(default)]
            cold_sql_url: Option<String>,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            #[serde(default)]
            cold_sql_max_connections: Option<u32>,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            #[serde(default)]
            cold_sql_min_connections: Option<u32>,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            #[serde(default)]
            cold_sql_acquire_timeout_secs: Option<u64>,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            #[serde(default)]
            cold_sql_idle_timeout_secs: Option<u64>,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            #[serde(default)]
            cold_sql_max_lifetime_secs: Option<u64>,
        }

        let helper = Helper::deserialize(deserializer)?;

        #[cfg(not(any(feature = "postgres", feature = "sqlite")))]
        if helper.cold_sql_url.as_ref().is_some_and(|url| !url.is_empty()) {
            return Err(serde::de::Error::custom(
                "cold_sql_url requires the 'postgres' or 'sqlite' feature",
            ));
        }

        // Defaults must match bin-base's `FromEnv for SqlConnector` so that
        // env-var and config-file paths produce identical pool behavior.
        #[cfg(any(feature = "postgres", feature = "sqlite"))]
        let cold_sql = helper.cold_sql_url.filter(|url| !url.is_empty()).map(|url| {
            SqlConnector::new(url)
                .with_max_connections(helper.cold_sql_max_connections.unwrap_or(100))
                .with_min_connections(helper.cold_sql_min_connections.unwrap_or(5))
                .with_acquire_timeout(Duration::from_secs(
                    helper.cold_sql_acquire_timeout_secs.unwrap_or(5),
                ))
                .with_idle_timeout(Some(Duration::from_secs(
                    helper.cold_sql_idle_timeout_secs.unwrap_or(600),
                )))
                .with_max_lifetime(Some(Duration::from_secs(
                    helper.cold_sql_max_lifetime_secs.unwrap_or(1800),
                )))
        });

        Ok(StorageConfig {
            hot_path: helper.hot_path,
            cold_path: helper.cold_path,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            cold_sql,
        })
    }
}

impl StorageConfig {
    /// Create a new storage configuration with MDBX cold backend.
    pub const fn new(hot_path: Cow<'static, str>, cold_path: Cow<'static, str>) -> Self {
        Self {
            hot_path,
            cold_path,
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            cold_sql: None,
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

    /// Get the cold SQL connection URL. Returns an empty string when SQL
    /// cold storage is not configured.
    #[cfg_attr(
        not(any(feature = "postgres", feature = "sqlite")),
        expect(clippy::missing_const_for_fn, reason = "not const when SQL features are enabled")
    )]
    pub fn cold_sql_url(&self) -> &str {
        #[cfg(any(feature = "postgres", feature = "sqlite"))]
        if let Some(connector) = &self.cold_sql {
            return connector.url();
        }
        ""
    }

    /// Build unified storage from this configuration.
    ///
    /// Creates connectors from the configured paths, spawns the cold storage
    /// background task, and returns a [`UnifiedStorage`] ready for use.
    ///
    /// Exactly one of `cold_path` or `cold_sql` must be configured.
    pub async fn build_storage(
        &self,
        cancel: CancellationToken,
    ) -> eyre::Result<UnifiedStorage<DatabaseEnv>> {
        let hot = MdbxConnector::new(self.hot_path.as_ref());
        let has_mdbx = !self.cold_path.is_empty();

        #[cfg(any(feature = "postgres", feature = "sqlite"))]
        let has_sql = self.cold_sql.is_some();
        #[cfg(not(any(feature = "postgres", feature = "sqlite")))]
        let has_sql = std::env::var("SIGNET_COLD_SQL_URL").is_ok_and(|v| !v.is_empty());

        match (has_mdbx, has_sql) {
            (true, false) => Ok(StorageBuilder::new()
                .hot(hot)
                .cold(MdbxConnector::new(self.cold_path.as_ref()))
                .cancel_token(cancel)
                .build()
                .await?),
            #[cfg(any(feature = "postgres", feature = "sqlite"))]
            (false, true) => {
                let connector =
                    self.cold_sql.clone().expect("cold_sql must be Some when has_sql is true");
                Ok(StorageBuilder::new()
                    .hot(hot)
                    .cold(connector)
                    .cancel_token(cancel)
                    .build()
                    .await?)
            }
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
