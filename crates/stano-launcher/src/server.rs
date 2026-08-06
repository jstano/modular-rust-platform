use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, StatusCode},
    middleware,
    response::IntoResponse,
};
use stano_axum::{ErrorResponse, error_logging_middleware, http_request_logging_middleware};
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
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    config::BootstrapConfig,
    observability::{self, record_http_metrics},
    routes::collect_routes,
    shutdown::shutdown_signal,
};

/// Starts the server: merges all `#[get]`/`#[post]`/etc.-registered handlers (see
/// [`crate::routes`]) with any hand-built `extra_routes`, applies the fixed middleware stack
/// (see the crate README for the full list and ordering), binds a `TcpListener` on
/// `0.0.0.0:{port}`, and runs until Ctrl+C (or SIGTERM on Unix) triggers graceful shutdown.
///
/// `extra_routes` is for routes that don't go through `#[get]`/`#[post]`/etc. (e.g. a raw
/// health check with no `#[utoipa::path]`) — pass `OpenApiRouter::new()` if there are none.
///
/// `authorization` is an optional [`stano_axum::security::AuthorizationLayer`], built by the
/// app via `stano_axum::security::AuthorizationBuilder` — pass `None` to skip authorization
/// enforcement entirely.
///
/// `post_authorization` is an optional [`stano_axum::security::PostAuthorizationHook`],
/// applied immediately after `authorization` in request-flow order (between authorization
/// and the router) — e.g. to bridge `AuthorizationLayer`'s `SecurityContext` into an
/// app-local mechanism. Pass `None` to skip it; has no effect if `authorization` is also
/// `None`.
///
/// `register_metrics` is an optional callback invoked once, immediately after the
/// Prometheus registry is built — only if [`ObservabilityConfig::prometheus_enabled`] is
/// true — so a consumer can register its own custom collectors (`prometheus::Gauge`,
/// `IntCounter`, etc.) into the same registry `run()` mounts at `GET /metrics`, rather than
/// standing up a second, separate metrics endpoint. Hold onto whatever handle
/// `registry.register(...)` returns (e.g. in a `std::sync::OnceLock`) to update the metric
/// later from elsewhere in your app. A no-op (never called) if `prometheus_enabled` is
/// false. Pass `None` if you have no custom metrics to register.
pub async fn run(
    ctx: Arc<ApplicationContext>,
    extra_routes: OpenApiRouter<Arc<ApplicationContext>>,
    config: BootstrapConfig,
    authorization: Option<stano_axum::security::AuthorizationLayer>,
    post_authorization: Option<stano_axum::security::PostAuthorizationHook>,
    register_metrics: Option<fn(&prometheus::Registry)>,
) -> anyhow::Result<()> {
    let otel_guard = observability::init_observability(&config.observability)?;
    let port = config.port;
    let metrics_enabled = config.observability.metrics_enabled;
    let prometheus_enabled = config.observability.prometheus_enabled;
    let http_logging_enabled = config.observability.http_logging_enabled;

    if let Some(registry) = otel_guard.prometheus_registry()
        && let Some(register_metrics) = register_metrics
    {
        register_metrics(registry);
    }

    let (router, mut openapi) = OpenApiRouter::<Arc<ApplicationContext>>::new()
        .merge(collect_routes())
        .merge(extra_routes)
        .split_for_parts();
    register_bearer_auth_scheme(&mut openapi);

    let mut app = router;
    if config.enable_swagger {
        let swagger_router: Router<Arc<ApplicationContext>> = SwaggerUi::new("/swagger")
            .url("/api-docs/openapi.json", openapi)
            .into();
        app = app.merge(swagger_router);
    }
    if let Some(registry) = otel_guard.prometheus_registry() {
        app = app.merge(observability::prometheus_router::<Arc<ApplicationContext>>(
            registry.clone(),
        ));
    }
    let mut app = app.with_state(Arc::clone(&ctx));

    if metrics_enabled || prometheus_enabled {
        app = app.route_layer(middleware::from_fn(record_http_metrics));
    }

    let mut app = app
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(TraceLayer::new_for_http());

    if http_logging_enabled {
        app = app.layer(middleware::from_fn(http_request_logging_middleware));
    }

    let mut app = app
        .layer(middleware::from_fn(error_logging_middleware))
        .layer(CatchPanicLayer::custom(handle_panic))
        .layer(CompressionLayer::new())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

    // `post_authorization` is layered *before* `authorization` here so it ends up more
    // inner (closer to the router, hit after authorization has run) — see its own doc
    // comment for why (bridging `SecurityContext` into an app-local mechanism needs the
    // request to have already passed the authorization check).
    if let Some(post_authorization) = post_authorization {
        app = app.layer(post_authorization);
    }

    if let Some(authorization) = authorization {
        app = app.layer(authorization);
    }

    // CORS must be outermost (hit first) so it can answer preflight `OPTIONS` requests
    // directly — `AuthorizationLayer`'s rule table is typically scoped to specific methods
    // (GET/POST/etc, not OPTIONS), so an `OPTIONS` preflight to a protected route falls
    // through to the mandatory `.any_request().authenticated()` rule and gets rejected with
    // 401 *before* `CorsLayer` ever gets a chance to answer it, if CORS sits anywhere inside
    // `authorization` in request-flow order. See `cors_preflight_...` tests below.
    let app = app
        .layer(middleware::from_fn(add_security_headers))
        .layer(cors_layer(&config));

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("Listening on port {port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    otel_guard.shutdown()?;
    Ok(())
}

/// Registers a `bearerAuth` HTTP-bearer `SecurityScheme` in the generated OpenAPI document's
/// `components.securitySchemes`, matching the scheme name every `#[get]`/`#[post]`/etc.
/// handler's `security(("bearerAuth" = []))` annotation already references. Without this,
/// those annotations point at an undefined scheme — Swagger UI's Authorize button doesn't
/// render correctly, and the document is invalid against strict OpenAPI validators. Purely
/// additive to the generated doc; has no effect on request handling.
fn register_bearer_auth_scheme(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

    let components = openapi
        .components
        .get_or_insert_with(utoipa::openapi::schema::Components::new);
    components.add_security_scheme(
        "bearerAuth",
        SecurityScheme::Http(
            HttpBuilder::new()
                .scheme(HttpAuthScheme::Bearer)
                .bearer_format("JWT")
                .build(),
        ),
    );
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
    use stano_di::environment::Environment;
    use stano_security::JwtConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
            enable_swagger: false,
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
                prometheus_enabled: false,
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

    struct TestEnvironment;

    impl Environment for TestEnvironment {
        fn get(&self, _key: &str) -> Option<String> {
            None
        }
    }

    /// Reserves an OS-assigned free port by binding then immediately dropping a
    /// listener, so `run()` (which binds its own listener) can be pointed at a port
    /// known not to be in use.
    async fn free_port() -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral port");
        listener.local_addr().expect("local addr").port()
    }

    #[tokio::test]
    async fn run_binds_listener_and_serves_requests_through_the_middleware_stack() {
        let port = free_port().await;
        let config = test_config(vec![], vec![]);
        let config = BootstrapConfig { port, ..config };
        let ctx = Arc::new(ApplicationContext::new(Arc::new(TestEnvironment)));

        let server = tokio::spawn(run(ctx, OpenApiRouter::new(), config, None, None, None));

        let mut stream = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };

        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");

        // No route is registered, so the router 404s — but that response still had to
        // flow through the full middleware stack `run()` assembles (CORS, timeout,
        // trace, catch-panic, compression, request-id, body-limit, security headers).
        assert!(response.starts_with("HTTP/1.1 404"), "got: {response}");
        assert!(
            response
                .to_ascii_lowercase()
                .contains("x-content-type-options: nosniff")
        );
        assert!(
            response
                .to_ascii_lowercase()
                .contains("x-frame-options: deny")
        );

        // `run()` only returns after graceful shutdown (Ctrl+C/SIGTERM), which this
        // test can't trigger without signaling the whole test process, so the
        // `otel_guard.shutdown()` tail of `run()` is intentionally left uncovered here.
        server.abort();
    }

    #[tokio::test]
    async fn run_mounts_prometheus_metrics_endpoint_when_enabled() {
        let port = free_port().await;
        let mut config = test_config(vec![], vec![]);
        config.port = port;
        config.observability.prometheus_enabled = true;
        let ctx = Arc::new(ApplicationContext::new(Arc::new(TestEnvironment)));

        let server = tokio::spawn(run(ctx, OpenApiRouter::new(), config, None, None, None));

        let mut stream = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };

        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");

        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(
            response
                .to_ascii_lowercase()
                .contains("content-type: text/plain")
        );

        server.abort();
    }

    #[tokio::test]
    async fn run_invokes_register_metrics_callback_and_scrape_includes_it() {
        static CUSTOM_GAUGE: std::sync::OnceLock<prometheus::IntGauge> = std::sync::OnceLock::new();

        fn register(registry: &prometheus::Registry) {
            let gauge = prometheus::IntGauge::new("wb_build_info", "build marker").expect("gauge");
            gauge.set(1);
            registry
                .register(Box::new(gauge.clone()))
                .expect("register");
            let _ = CUSTOM_GAUGE.set(gauge);
        }

        let port = free_port().await;
        let mut config = test_config(vec![], vec![]);
        config.port = port;
        config.observability.prometheus_enabled = true;
        let ctx = Arc::new(ApplicationContext::new(Arc::new(TestEnvironment)));

        let server = tokio::spawn(run(
            ctx,
            OpenApiRouter::new(),
            config,
            None,
            None,
            Some(register),
        ));

        let mut stream = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };

        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");

        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(
            response.contains("wb_build_info 1"),
            "custom collector should be scraped alongside built-in metrics, got: {response}"
        );

        server.abort();
    }

    /// Regression test for the ordering bug found during wanderbooks' Phase 6/7 migration
    /// (see `docs/decisions/2026-08-03-modular-rust-platform-migration.md` in that repo):
    /// `AuthorizationLayer` has no special-casing for `OPTIONS` preflight requests, and its
    /// rule table is typically scoped to specific methods (GET/POST/etc, not OPTIONS) — so
    /// a preflight to a method-scoped-protected route falls through to the mandatory
    /// `.any_request().authenticated()` rule and gets rejected with 401 *before* `CorsLayer`
    /// ever answers it, unless CORS sits outside `authorization` in request-flow order (i.e.
    /// is the outermost layer, applied last in code). This exercises the real fix in `run()`
    /// end-to-end, not just `cors_layer` in isolation.
    #[tokio::test]
    async fn options_preflight_to_authorization_protected_route_gets_cors_headers_not_401() {
        let port = free_port().await;
        let mut config = test_config(vec!["http://localhost:5173".to_string()], vec![]);
        config.port = port;
        let ctx = Arc::new(ApplicationContext::new(Arc::new(TestEnvironment)));

        // A rule table where every route (including the one under test) requires
        // authentication — mirrors a real app's `AuthorizationBuilder` chain.
        let authorization = stano_axum::security::AuthorizationBuilder::<()>::new()
            .any_request()
            .authenticated()
            .build(config.jwt_config.clone())
            .expect("valid chain");

        let server = tokio::spawn(run(
            ctx,
            OpenApiRouter::new(),
            config,
            Some(authorization),
            None,
            None,
        ));

        let mut stream = loop {
            match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };

        stream
            .write_all(
                b"OPTIONS / HTTP/1.1\r\n\
                  Host: localhost\r\n\
                  Origin: http://localhost:5173\r\n\
                  Access-Control-Request-Method: GET\r\n\
                  Connection: close\r\n\r\n",
            )
            .await
            .expect("write request");

        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");

        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(
            response
                .to_ascii_lowercase()
                .contains("access-control-allow-origin: http://localhost:5173"),
            "got: {response}"
        );

        server.abort();
    }
}
