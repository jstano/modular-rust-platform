# stano-launcher

Application bootstrap and server runner: wires an Axum router, applies a standard middleware stack, handles graceful shutdown, and listens for HTTP traffic.

## Install

```toml
[dependencies]
stano-launcher = { path = "../stano-launcher" }
stano-di = { path = "../stano-di" }
stano-axum = { path = "../stano-axum" }
stano-security = { path = "../stano-security" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
axum = "0.8"
```

## API

### Configuration

- **`BootstrapConfig`** — server startup settings.
  - `port: u16` — TCP port to listen on.
  - `jwt_config: JwtConfig` — JWT private/public keys and optional expiration duration.
  - `cors_origins: Vec<String>` — allowed CORS origins, exact match (empty + `cors_origin_suffixes` empty = permissive). Used when `is_dev` is false, or when `is_dev` is true but `cors_dev_origins` is empty.
  - `cors_origin_suffixes: Vec<String>` — allowed CORS origin suffixes, e.g. `.example.com` matches any subdomain (empty + `cors_origins` empty = permissive). Same applicability as `cors_origins`.
  - `cors_dev_origins: Vec<String>` — exact origins allowed when `is_dev` is true (e.g. `http://localhost:5173`). Takes priority over `cors_origins`/`cors_origin_suffixes` while non-empty and `is_dev` is true.
  - `is_dev: bool` — when true, CORS uses `cors_dev_origins` instead of `cors_origins`/`cors_origin_suffixes` (falling back to the latter if `cors_dev_origins` is empty). Compute with `is_dev_environment`, or set explicitly.
  - `observability: ObservabilityConfig` — OTLP tracing/metrics/log export settings (see below). `run()` initializes observability from this before doing anything else.

- **`parse_csv_env(environment: &dyn Environment, key: &str) -> Vec<String>`** — helper to populate `cors_origins`/`cors_origin_suffixes`/`cors_dev_origins` (or any other list-valued config) from a comma-separated environment variable. Trims whitespace and drops empty entries; returns an empty vec if the var is unset.

- **`is_dev_environment(environment: &dyn Environment) -> bool`** — true for debug builds, or when `RUST_ENV=development` (case-insensitive). Note debug builds (including `cargo test`) are always considered dev mode.

### Observability

- **`ObservabilityConfig`** — OTLP tracing/metrics/log export settings.
  - `enabled: bool` — master switch. When `false` (the default), only a local `fmt` + `EnvFilter` console subscriber is installed and no OTLP export happens — safe for local dev without a collector.
  - `otlp_endpoint: String` — collector endpoint, e.g. `http://localhost:4317` (grpc) or `http://localhost:4318` (http/protobuf).
  - `protocol: OtlpProtocol` — `Grpc` or `HttpProtobuf`, runtime-selectable (both exporter transports are compiled in).
  - `service_name: String` / `service_version: String` — OTel resource attributes.
  - `resource_attributes: Vec<(String, String)>` — additional OTel resource attributes, e.g. `("deployment.environment", "prod")`.
  - `trace_sample_ratio: f64` — trace sampling ratio in `0.0..=1.0`.
  - `log_filter: String` — `tracing_subscriber::EnvFilter` directive string, e.g. `"info,my_app=debug"`.
  - `metrics_enabled: bool` — independently enables OTLP metrics export and the HTTP server metrics middleware (request count/duration, active requests).
  - `http_logging_enabled: bool` — independently enables `stano_axum::http_request_logging_middleware`, which logs every HTTP request (method, URI, status, latency, and the current span's OTel trace_id).

- **`observability_config_from_env(environment: &dyn Environment) -> ObservabilityConfig`** — reads standard OTel env vars (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_EXPORTER_OTLP_PROTOCOL`, `OTEL_SERVICE_NAME`, `OTEL_SERVICE_VERSION`, `OTEL_TRACES_SAMPLER_ARG`, `RUST_LOG`) plus `STANO_OTEL_ENABLED`/`STANO_OTEL_METRICS_ENABLED`/`STANO_HTTP_LOGGING_ENABLED` (all default `false`).

- **`OtelGuard`** — returned internally by `init_observability` and held by `run()` for the request lifetime; flushed after `axum::serve(...)` resolves so in-flight spans/metrics are exported before shutdown.

`run()` calls `init_observability(&config.observability)` as the first thing it does, so all subsequent `tracing::*!` calls (including from `stano-di`, `stano-axum`, and your own app code) are captured. No call-site changes are needed anywhere — this composes a `tracing_subscriber::Registry` that the plain `tracing` facade already flows through.

### Router Groups

- **`RouteGroups`** — three route tiers for organizational clarity (no automatic auth tier differentiation).
  - `public: Router<Arc<ApplicationContext>>` — routes accessible to all.
  - `protected: Router<Arc<ApplicationContext>>` — routes requiring auth (you apply the guard).
  - `admin: Router<Arc<ApplicationContext>>` — routes requiring admin role (you apply the guard).

  **Important:** These fields are organizational; `run()` merges all three with the same middleware stack. Auth/authz per tier is **not** automatically applied — your app implements and applies its own auth middleware to `protected` and `admin` routers before passing them to `RouteGroups`.

### Server Startup

- **`run(ctx: Arc<ApplicationContext>, routes: RouteGroups, config: BootstrapConfig) -> Result<(), anyhow::Error>`** — start the server.
  - Merges the three route groups.
  - Applies a fixed middleware stack (see below).
  - Binds a `TcpListener` on `0.0.0.0:{port}`.
  - Logs "Listening on port {port}".
  - Runs until Ctrl+C (or SIGTERM on Unix) is received, then performs graceful shutdown.

## Middleware Stack

Applied in this request-processing order (outermost → innermost, closest to handlers):

1. **Security headers** — injects `x-content-type-options: nosniff`, `x-frame-options: DENY`, `strict-transport-security: max-age=31536000; includeSubDomains`.
2. **Request body limit** — 10 MB max request body.
3. **Propagate request ID** — propagates `x-request-id` upstream.
4. **Set request ID** — injects a unique `x-request-id` if not present.
5. **Compression** — gzip/brotli/deflate (auto-negotiated).
6. **Catch panic** — panics in handlers become 500 responses.
7. **Error logging** — logs `ApiError` with request context (see `stano_axum::error_logging_middleware`).
7a. **HTTP request logging** *(when `observability.http_logging_enabled` is true)* — logs every request with method, URI, status, latency, and trace_id (see `stano_axum::http_request_logging_middleware`). Sits just inside the Tracing layer so it runs within the same span and can read a valid OTel trace_id.
8. **Tracing** — structured request/response logging via `tracing`, exported via OTLP when `observability.enabled` is true (see Observability above).
9. **Timeout** — 300-second per-request timeout.
10. **CORS** — allow/disallow origins based on config.

Additionally, when `observability.metrics_enabled` is true, an HTTP metrics middleware is applied via `route_layer` (so it only wraps matched routes, giving it access to `MatchedPath` for the `http.route` attribute) — this records `http.server.request.duration` and `http.server.active_requests` via the global OTel meter.

## Usage Example

```rust
use axum::routing::{get, post};
use stano_di::{ApplicationContext, OsEnvironment};
use stano_launcher::{
    BootstrapConfig, RouteGroups, is_dev_environment, observability_config_from_env, parse_csv_env,
    run,
};
use stano_security::JwtConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = Arc::new(OsEnvironment::new());

    // Load config from environment.
    let config = BootstrapConfig {
        port: env.get("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000),
        jwt_config: JwtConfig {
            private_key_pem: env.get("JWT_PRIVATE_KEY").expect("required"),
            public_key_pem: env.get("JWT_PUBLIC_KEY").expect("required"),
            expiration_seconds: 3600,
        },
        cors_origins: parse_csv_env(env.as_ref(), "APP_CORS_ORIGINS"),
        cors_origin_suffixes: parse_csv_env(env.as_ref(), "APP_CORS_ORIGIN_SUFFIXES"),
        cors_dev_origins: parse_csv_env(env.as_ref(), "APP_CORS_DEV_ORIGINS"),
        is_dev: is_dev_environment(env.as_ref()),
        observability: observability_config_from_env(env.as_ref()),
    };

    // Build the DI container.
    let mut ctx = ApplicationContext::new(env);
    // Register your services here...
    ctx.validate().map_err(|errs| anyhow::anyhow!("{errs:?}"))?; // Validate the container (detects cycles, etc.)

    // Define routes.
    let routes = RouteGroups {
        public: axum::Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/api/users", post(create_user_handler)),
        protected: axum::Router::new()
            .route("/api/profile", get(get_profile_handler))
            // Apply your own auth middleware here if needed
            .layer(axum::middleware::from_fn(check_auth)),
        admin: axum::Router::new()
            .route("/api/admin/stats", get(admin_stats_handler))
            .layer(axum::middleware::from_fn(check_admin)),
    };

    // Start the server (blocks until Ctrl+C or SIGTERM).
    run(Arc::new(ctx), routes, config).await
}

// Your handlers and middleware...
async fn create_user_handler(/* ... */) -> /* ... */ { /* ... */ }
async fn get_profile_handler(/* ... */) -> /* ... */ { /* ... */ }
async fn admin_stats_handler(/* ... */) -> /* ... */ { /* ... */ }
async fn check_auth(/* ... */) -> /* ... */ { /* ... */ }
async fn check_admin(/* ... */) -> /* ... */ { /* ... */ }
```

## Notes

- **No automatic auth** — despite having `protected` and `admin` route groups, auth/authz middleware is not automatically applied. You implement auth (e.g., via `stano-security::decode_jwt` and a custom middleware) and apply it yourself to the routers before passing them into `RouteGroups`.
- **Route merging** — all three route groups are merged into a single router, so define paths carefully to avoid collisions.
- **Graceful shutdown** — the server responds to Ctrl+C on all platforms and SIGTERM on Unix-like systems. Connections are drained gracefully.
- **CORS configuration** — pass `cors_origins`, `cors_origin_suffixes`, and `cors_dev_origins` all empty for permissive CORS (allow any origin); otherwise list exact origins and/or origin suffixes. Populate any of these from an env var with `parse_csv_env`, or set them directly from your app config. Set `is_dev` (via `is_dev_environment` or explicitly) to switch to `cors_dev_origins` in development.
- **Observability is automatic, not opt-in per call** — `run()` always calls `init_observability`; set `observability.enabled = false` (the default via `observability_config_from_env`) to get a plain console subscriber and skip OTLP export entirely, e.g. for local dev without a collector. If your app already installs its own `tracing_subscriber`, don't call `run()` with `enabled: true` at the same time — only one global subscriber can be installed per process.
- **No feature flags** — all APIs available.

See also: [`stano-di`](../stano-di), [`stano-axum`](../stano-axum), [`stano-security`](../stano-security).
