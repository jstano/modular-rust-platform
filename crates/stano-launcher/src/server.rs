use axum::{
    Json, Router,
    http::{HeaderName, HeaderValue, StatusCode},
    middleware,
    response::IntoResponse,
};
use stano_axum::{ErrorResponse, error_logging_middleware};
use stano_di::application_context::ApplicationContext;
use std::{sync::Arc, time::Duration};
use tower_http::{
    catch_panic::CatchPanicLayer,
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{config::BootstrapConfig, shutdown::shutdown_signal};

pub struct RouteGroups {
    pub public: Router<Arc<ApplicationContext>>,
    pub protected: Router<Arc<ApplicationContext>>,
    pub admin: Router<Arc<ApplicationContext>>,
}

pub async fn run(
    ctx: Arc<ApplicationContext>,
    routes: RouteGroups,
    config: BootstrapConfig,
) -> anyhow::Result<()> {
    let port = config.port;

    let app = Router::new()
        .merge(routes.public)
        .merge(routes.protected)
        .merge(routes.admin)
        .with_state(Arc::clone(&ctx))
        .layer(cors_layer(&config))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(TraceLayer::new_for_http())
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
    Ok(())
}

fn cors_layer(config: &BootstrapConfig) -> CorsLayer {
    if config.cors_origins.is_empty() {
        CorsLayer::permissive()
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
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
