use crate::error::ApiError;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

/// Middleware that logs API errors from response extensions
///
/// This catches any ApiError stored in response extensions by the IntoResponse impl
/// and logs it with full context including request ID.
pub async fn error_logging_middleware(request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let method = request.method().clone();
    let uri = request.uri().clone();

    let response = next.run(request).await;

    // Check if there's an ApiError in the response extensions
    if let Some(error) = response.extensions().get::<Arc<ApiError>>() {
        tracing::error!(
            method = %method,
            uri = %uri,
            request_id = ?request_id,
            error = ?error,
            "Request failed with API error"
        );
    }

    response
}
