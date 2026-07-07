use crate::query_tracing::{self, QueryTracingConfig};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;
use thiserror::Error;

/// Errors returned by [`DbConfig::from_url`].
#[derive(Debug, Error)]
pub enum DbConfigError {
    /// The database connection could not be established.
    #[error("Failed to connect to database: {0}")]
    ConnectionFailed(String),
}

/// A configured Postgres connection pool.
#[derive(Clone)]
pub struct DbConfig {
    connection: DatabaseConnection,
}

impl DbConfig {
    /// Create a new DbConfig from a database URL string.
    ///
    /// Sets up a connection pool with:
    /// - max_connections: 100
    /// - min_connections: 5
    /// - connect_timeout: 30s
    /// - idle_timeout: 10 min
    /// - max_lifetime: 1h
    ///
    /// When `tracing_config.enabled`, installs a `set_metric_callback` that emits a
    /// `stano_seaorm::query` tracing event per SQL statement (see
    /// [`crate::QueryTracingConfig`]), and disables sea-orm's own `sqlx_logging` to
    /// avoid duplicate log lines for the same query.
    pub async fn from_url(
        database_url: &str,
        tracing_config: QueryTracingConfig,
    ) -> Result<Self, DbConfigError> {
        let mut opt = ConnectOptions::new(database_url);
        opt.max_connections(100)
            .min_connections(5)
            .connect_timeout(Duration::from_secs(30))
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(3600));

        if tracing_config.enabled {
            opt.sqlx_logging(false);
        }

        let mut connection = Database::connect(opt)
            .await
            .map_err(|e| DbConfigError::ConnectionFailed(e.to_string()))?;

        if tracing_config.enabled {
            connection.set_metric_callback(query_tracing::query_tracing_callback(tracing_config));
        }

        Ok(Self { connection })
    }

    /// The underlying SeaORM database connection.
    pub fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_from_url_with_malformed_url_returns_connection_failed() {
        let result =
            DbConfig::from_url("not-a-valid-database-url", QueryTracingConfig::default()).await;
        assert!(matches!(result, Err(DbConfigError::ConnectionFailed(_))));
    }

    #[tokio::test]
    async fn test_from_url_with_tracing_enabled_and_malformed_url_returns_connection_failed() {
        let tracing_config = QueryTracingConfig {
            enabled: true,
            ..QueryTracingConfig::default()
        };
        let result = DbConfig::from_url("not-a-valid-database-url", tracing_config).await;
        assert!(matches!(result, Err(DbConfigError::ConnectionFailed(_))));
    }
}
