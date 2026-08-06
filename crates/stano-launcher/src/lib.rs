//! Application bootstrap and server runner: wires an Axum router, applies a standard
//! middleware stack, handles graceful shutdown, and listens for HTTP traffic.
//!
//! See [`BootstrapConfig`] for startup settings, the [`get`]/[`post`]/[`put`]/[`delete`]/
//! [`patch`] macros and [`routes`] module for auto-registering handlers, and [`run`] for the
//! server entry point.
#![warn(missing_docs)]

/// [`BootstrapConfig`] and related environment-variable helpers.
pub mod config;
/// OTLP-based tracing, metrics, and log export, wired automatically into [`run`].
pub mod observability;
/// [`run`], the server entry point.
pub mod server;
mod shutdown;

/// Re-exported so crates that only depend on `stano-launcher` can still resolve
/// `stano-axum`'s routing/inventory machinery through the `#[get]`/`#[post]`/etc. macros.
pub extern crate stano_axum;

/// Re-exported so a consumer implementing `run()`'s `register_metrics` callback can name
/// `prometheus::Registry`/`Gauge`/`IntCounter`/etc. without adding its own `prometheus`
/// dependency (and risking a version mismatch with the one `run()` actually uses).
pub extern crate prometheus;

/// Auto-registration of `#[get]`/`#[post]`/`#[put]`/`#[delete]`/`#[patch]`-annotated
/// handlers, consumed by [`run`]. Defined in `stano-axum` so adapter crates that only write
/// routes never need to depend on `stano-launcher` itself.
pub use stano_axum::routes;

pub use config::{BootstrapConfig, is_dev_environment, parse_csv_env};
pub use observability::{
    ObservabilityConfig, OtelGuard, OtlpProtocol, observability_config_from_env,
};
pub use server::run;
pub use stano_axum::{delete, get, patch, post, put};
