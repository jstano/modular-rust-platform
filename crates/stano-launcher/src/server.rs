use axum::{
    http::{HeaderName, HeaderValue, StatusCode}, middleware,
    response::IntoResponse,
    Json,
    Router,
};
use stano_axum::{error_logging_middleware, http_request_logging_middleware, ErrorResponse};
use stano_di::application_context::ApplicationContext;
use std::{sync::Arc, time::Duration};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{AllowOrigin, Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    config::BootstrapConfig,
    observability::{self, record_http_metrics},
    shutdown::shutdown_signal,
};

/// Three route tiers for organizational clarity. `run()` merges all three with the same
/// middleware stack — there is no automatic auth tier differentiation. Your app must apply
/// its own auth/authz middleware to `protected` and `admin` before passing them in here.
pub struct RouteGroups {
    /// Routes accessible to all, no auth applied.
    pub public: Router<Arc<ApplicationContext>>,
    /// Routes requiring auth — you apply the guard.
    pub protected: Router<Arc<ApplicationContext>>,
    /// Routes requiring admin role — you apply the guard.
    pub admin: Router<Arc<ApplicationContext>>,
}

/// Starts the server: merges the three route groups, applies the fixed middleware stack
/// (see the crate README for the full list and ordering), binds a `TcpListener` on
/// `0.0.0.0:{port}`, and runs until Ctrl+C (or SIGTERM on Unix) triggers graceful shutdown.
pub async fn run(
    ctx: Arc<ApplicationContext>,
    routes: RouteGroups,
    config: BootstrapConfig,
) -> anyhow::Result<()> {
    let otel_guard = observability::init_observability(&config.observability)?;
    let port = config.port;
    let metrics_enabled = config.observability.metrics_enabled;
    let http_logging_enabled = config.observability.http_logging_enabled;

    let mut app = Router::new()
        .merge(routes.public)
        .merge(routes.protected)
        .merge(routes.admin)
        .with_state(Arc::clone(&ctx));

    if metrics_enabled {
        app = app.route_layer(middleware::from_fn(record_http_metrics));
    }

    let mut app = app
        .layer(cors_layer(&config))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(TraceLayer::new_for_http());

    if http_logging_enabled {
        app = app.layer(middleware::from_fn(http_request_logging_middleware));
    }

    let app = app
        .layer(middleware::from_fn(error_logging_middleware))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(CompressionLayer::new())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        .layer(middleware::from_fn(add_security_headers));

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Listening on port {port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    otel_guard.shutdown()?;
    Ok(())
}

fn cors_layer(config: &BootstrapConfig) -> CorsLayer {
    if config.is_dev && !config.cors_dev_origins.is_empty() {
        let dev_origins: Vec<HeaderValue> = config
            .cors_dev_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();

        return CorsLayer::new()
            .allow_origin(dev_origins)
            .allow_methods(Any)
            .allow_headers(Any);
    }

    if config.cors_origins.is_empty() && config.cors_origin_suffixes.is_empty() {
        CorsLayer::permissive()
    } else {
        let exact_origins = config.cors_origins.clone();
        let suffixes = config.cors_origin_suffixes.clone();

        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin: &HeaderValue, _| {
                origin
                    .to_str()
                    .map(|s| {
                        exact_origins.iter().any(|o| o == s)
                            || suffixes.iter().any(|suf| s.ends_with(suf.as_str()))
                    })
                    .unwrap_or(false)
            }))
            .allow_methods(Any)
            .allow_headers(Any)
    }
}

async fn add_security_headers(
    req: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        HeaderName::from_static("strict-transport-security"),
        HeaderValue::from_static("max-age=31536000; includeSubDomains"),
    );
    res
}

fn handle_panic(err: Box<dyn std::any::Any + Send>) -> axum::response::Response {
    let msg = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic");
    tracing::error!(panic_message = %msg, "Handler panicked");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(
            500,
            "INTERNAL_ERROR",
            "An internal error occurred",
        )),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{ObservabilityConfig, OtlpProtocol};
    use axum::{
        body::Body,
        http::{Method, Request},
        routing::get,
    };
    use stano_security::JwtConfig;
    use tower::util::ServiceExt;

    fn test_config(
        cors_origins: Vec<String>,
        cors_origin_suffixes: Vec<String>,
    ) -> BootstrapConfig {
        BootstrapConfig {
            port: 0,
            jwt_config: JwtConfig {
                private_key_pem: String::new(),
                public_key_pem: String::new(),
                expiration_seconds: 3600,
            },
            cors_origins,
            cors_origin_suffixes,
            cors_dev_origins: vec![],
            is_dev: false,
            observability: ObservabilityConfig {
                enabled: false,
                otlp_endpoint: String::new(),
                protocol: OtlpProtocol::Grpc,
                service_name: "test-service".to_string(),
                service_version: "0.0.0".to_string(),
                resource_attributes: vec![],
                trace_sample_ratio: 1.0,
                log_filter: "info".to_string(),
                metrics_enabled: false,
                http_logging_enabled: false,
            },
        }
    }

    async fn preflight(config: &BootstrapConfig, origin: &str) -> axum::response::Response {
        let app = Router::new()
            .route("/hello", get(|| async { "ok" }))
            .layer(cors_layer(config));

        app.oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/hello")
                .header("origin", origin)
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
    }

    fn allow_origin_header(response: &axum::response::Response) -> Option<&str> {
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok())
    }

    #[tokio::test]
    async fn permissive_when_config_empty() {
        let config = test_config(vec![], vec![]);
        let response = preflight(&config, "http://anything.example").await;
        assert_eq!(allow_origin_header(&response), Some("*"));
    }

    #[tokio::test]
    async fn allows_exact_origin_match() {
        let config = test_config(vec!["http://localhost:5173".to_string()], vec![]);
        let response = preflight(&config, "http://localhost:5173").await;
        assert_eq!(
            allow_origin_header(&response),
            Some("http://localhost:5173")
        );
    }

    #[tokio::test]
    async fn rejects_unknown_origin_with_exact_list() {
        let config = test_config(vec!["http://localhost:5173".to_string()], vec![]);
        let response = preflight(&config, "http://evil.example").await;
        assert_eq!(allow_origin_header(&response), None);
    }

    #[tokio::test]
    async fn allows_suffix_match() {
        let config = test_config(vec![], vec![".example.com".to_string()]);
        let response = preflight(&config, "https://foo.example.com").await;
        assert_eq!(
            allow_origin_header(&response),
            Some("https://foo.example.com")
        );
    }

    #[tokio::test]
    async fn rejects_non_matching_suffix() {
        let config = test_config(vec![], vec![".example.com".to_string()]);
        let response = preflight(&config, "https://example.com.evil.com").await;
        assert_eq!(allow_origin_header(&response), None);
    }

    #[tokio::test]
    async fn dev_mode_allows_configured_dev_origin() {
        let mut config = test_config(vec!["https://prod.example.com".to_string()], vec![]);
        config.is_dev = true;
        config.cors_dev_origins = vec!["http://localhost:5173".to_string()];

        let response = preflight(&config, "http://localhost:5173").await;
        assert_eq!(
            allow_origin_header(&response),
            Some("http://localhost:5173")
        );
    }

    #[tokio::test]
    async fn dev_mode_rejects_prod_origin_not_in_dev_list() {
        let mut config = test_config(vec!["https://prod.example.com".to_string()], vec![]);
        config.is_dev = true;
        config.cors_dev_origins = vec!["http://localhost:5173".to_string()];

        let response = preflight(&config, "https://prod.example.com").await;
        assert_eq!(allow_origin_header(&response), None);
    }

    #[tokio::test]
    async fn dev_mode_falls_back_to_prod_matching_when_dev_origins_empty() {
        let mut config = test_config(vec!["https://prod.example.com".to_string()], vec![]);
        config.is_dev = true;

        let response = preflight(&config, "https://prod.example.com").await;
        assert_eq!(
            allow_origin_header(&response),
            Some("https://prod.example.com")
        );
    }

    async fn with_security_headers() -> axum::response::Response {
        let app = Router::new()
            .route("/hello", get(|| async { "ok" }))
            .layer(middleware::from_fn(add_security_headers));

        app.oneshot(
            Request::builder()
                .uri("/hello")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response")
    }

    #[tokio::test]
    async fn test_add_security_headers_sets_x_content_type_options() {
        let response = with_security_headers().await;
        assert_eq!(
            response.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
    }

    #[tokio::test]
    async fn test_add_security_headers_sets_x_frame_options() {
        let response = with_security_headers().await;
        assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
    }

    #[tokio::test]
    async fn test_add_security_headers_sets_strict_transport_security() {
        let response = with_security_headers().await;
        assert_eq!(
            response.headers().get("strict-transport-security").unwrap(),
            "max-age=31536000; includeSubDomains"
        );
    }

    #[test]
    fn test_handle_panic_with_string_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom".to_string());
        let response = handle_panic(payload);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_handle_panic_with_str_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        let response = handle_panic(payload);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_handle_panic_with_unknown_payload_type_fallback() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42);
        let response = handle_panic(payload);
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
