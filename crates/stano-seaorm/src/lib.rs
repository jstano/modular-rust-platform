//! Postgres pool configuration and domain↔entity [`Mapper`] trait.
#![warn(missing_docs)]

mod db_config;
mod mapper;
mod query_tracing;

pub use db_config::DbConfig;
pub use mapper::Mapper;
pub use query_tracing::{QueryTracingConfig, query_tracing_config_from_env};

pub use sea_orm;
