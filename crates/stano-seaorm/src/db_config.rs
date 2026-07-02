use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbConfigError {
    #[error("Failed to connect to database: {0}")]
    ConnectionFailed(String),
}

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
    pub async fn from_url(database_url: &str) -> Result<Self, DbConfigError> {
        let mut opt = ConnectOptions::new(database_url);
        opt.max_connections(100)
            .min_connections(5)
            .connect_timeout(Duration::from_secs(30))
            .acquire_timeout(Duration::from_secs(30))
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(3600));

        let connection = Database::connect(opt)
            .await
            .map_err(|e| DbConfigError::ConnectionFailed(e.to_string()))?;

        Ok(Self { connection })
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.connection
    }
}
