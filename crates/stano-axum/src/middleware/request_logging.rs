use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::trace::TraceContextExt;
use std::time::Instant;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Middleware that logs every HTTP request with method, URI, status, latency,
/// and the current span's OpenTelemetry trace ID.
///
/// `trace_id` will be the all-zero sentinel unless all of the following hold:
/// - OTel export is enabled, so a `tracing-opentelemetry` layer is installed
///   (see `stano_launcher::observability::init_observability`);
/// - the per-request tracing span this middleware reads (created by the
///   `TraceLayer` it must run inside) is actually constructed rather than
///   filtered out by the ambient `EnvFilter`/`RUST_LOG` level; and
/// - this middleware is layered *inside* (i.e. added before) that `TraceLayer`,
///   so its `tracing::Span::current()` read after `next.run(...)` returns still
///   observes the span, rather than running outside its scope entirely.
pub async fn http_request_logging_middleware(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();

    let response = next.run(request).await;

    let latency_ms = start.elapsed().as_millis();
    let status = response.status().as_u16();
    let trace_id = tracing::Span::current()
        .context()
        .span()
        .span_context()
        .trace_id()
        .to_string();

    tracing::info!(
        method = %method,
        uri = %uri,
        status,
        latency_ms,
        trace_id = %trace_id,
        "HTTP request completed"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::{Router, middleware};
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn test_request_logging_passes_through_successful_response() {
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(middleware::from_fn(http_request_logging_middleware));

        let response = app
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_request_logging_passes_through_error_response() {
        let app = Router::new()
            .route("/fail", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .layer(middleware::from_fn(http_request_logging_middleware));

        let response = app
            .oneshot(Request::builder().uri("/fail").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
