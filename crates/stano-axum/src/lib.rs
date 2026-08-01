//! HTTP extractors, unified [`ApiError`] type, and error-logging middleware.
#![warn(missing_docs)]

/// [`ApiError`] and [`ErrorResponse`], the unified HTTP error response types.
pub mod error;
/// [`AppJson`], [`AppPath`], [`AppQuery`], [`AppSecurityContext`] extractors wrapping Axum's
/// `Json`/`Path`/`Query` (and request extensions) with structured error conversion.
pub mod extractors;
/// [`error_logging_middleware`], for logging `ApiError`s with request context, and
/// [`http_request_logging_middleware`], for logging every HTTP request.
pub mod middleware;
/// Auto-registration of `#[get]`/`#[post]`/`#[put]`/`#[delete]`/`#[patch]`-annotated
/// handlers, consumed by `stano_launcher::run`.
pub mod routes;
/// [`security::AuthorizationBuilder`] and [`security::AuthorizationLayer`], a
/// Spring-Security-style declarative request-authorization chain.
pub mod security;

pub use error::{ApiError, ErrorResponse};
pub use extractors::{AppJson, AppPath, AppQuery, AppSecurityContext};
pub use middleware::{error_logging_middleware, http_request_logging_middleware};
pub use stano_route_macros::{delete, get, patch, post, put};
