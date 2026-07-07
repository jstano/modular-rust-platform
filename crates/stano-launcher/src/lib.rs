//! Application bootstrap and server runner: wires an Axum router, applies a standard
//! middleware stack, handles graceful shutdown, and listens for HTTP traffic.
//!
//! See [`BootstrapConfig`] for startup settings, [`RouteGroups`] for route tiers, and
//! [`run`] for the server entry point.
#![warn(missing_docs)]

/// Deprecated — auth middleware has moved to individual apps. The platform provides the
/// building blocks (`JwtConfig`, `decode_jwt`) but does not impose a specific
/// `SecurityContext` type. See your app's `rest_api/auth.rs` for an example.
pub mod auth;
/// Deprecated — authorization middleware should be implemented per-app, since apps define
/// their own role model and permission logic.
pub mod authorization;
/// [`BootstrapConfig`] and related environment-variable helpers.
pub mod config;
/// OTLP-based tracing, metrics, and log export, wired automatically into [`run`].
pub mod observability;
/// [`RouteGroups`] and [`run`], the server entry point.
pub mod server;
mod shutdown;

pub use config::{is_dev_environment, parse_csv_env, BootstrapConfig};
pub use observability::{
    observability_config_from_env, ObservabilityConfig, OtelGuard, OtlpProtocol,
};
pub use server::{run, RouteGroups};
