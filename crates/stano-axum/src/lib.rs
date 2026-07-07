//! HTTP extractors, unified [`ApiError`] type, and error-logging middleware.
#![warn(missing_docs)]

/// [`ApiError`] and [`ErrorResponse`], the unified HTTP error response types.
pub mod error;
/// [`AppJson`], [`AppPath`], [`AppQuery`] extractors wrapping Axum's `Json`/`Path`/`Query` with structured error conversion.
pub mod extractors;
/// [`error_logging_middleware`], for logging `ApiError`s with request context, and
/// [`http_request_logging_middleware`], for logging every HTTP request.
pub mod middleware;

pub use error::{ApiError, ErrorResponse};
pub use extractors::{AppJson, AppPath, AppQuery};
pub use middleware::{error_logging_middleware, http_request_logging_middleware};
