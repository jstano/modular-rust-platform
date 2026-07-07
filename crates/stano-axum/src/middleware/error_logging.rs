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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiError;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{middleware, Router};
    use stano_common::ServiceError;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn test_error_logging_passes_through_response_without_api_error() {
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(middleware::from_fn(error_logging_middleware));

        let response = app
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_error_logging_logs_when_response_has_api_error_extension() {
        let app = Router::new()
            .route(
                "/fail",
                get(|| async { ApiError::from(ServiceError::NotFound) }),
            )
            .layer(middleware::from_fn(error_logging_middleware));

        let response = app
            .oneshot(Request::builder().uri("/fail").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_error_logging_handles_missing_x_request_id_header() {
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(middleware::from_fn(error_logging_middleware));

        let response = app
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_error_logging_handles_present_x_request_id_header() {
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(middleware::from_fn(error_logging_middleware));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ok")
                    .header("x-request-id", "abc-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
