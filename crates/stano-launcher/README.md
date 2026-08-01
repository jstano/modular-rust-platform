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
utoipa = "5"
utoipa-axum = "0.2"
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
  - `enable_swagger: bool` — when true, mounts Swagger UI at `/swagger` and the generated OpenAPI document at `/api-docs/openapi.json`. Typically wired to `is_dev_environment()` so it's off in production.

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

### Route Auto-Registration

- **`#[get(...)]` / `#[post(...)]` / `#[put(...)]` / `#[delete(...)]` / `#[patch(...)]`** — per-HTTP-method attribute macros (from `stano-route-macros`, re-exported here) that replace `#[utoipa::path(...)]` on a handler and auto-register it, so it's picked up by `run()` without a manual `.routes(routes!(handler))` call.
  - Infer `operation_id` (from the function name), `request_body` (from a single `AppJson<T>` parameter), the `200` entry of `responses(...)` (from an `AppJson<T>`/`Result<AppJson<T>, E>` return type), and `params(...)` (from `AppPath<T>`/`AppQuery<T>` parameters); forward everything else (`path`, `tag`/`tags`, `security`, extra `responses(...)` entries) verbatim into a generated `#[utoipa::path(...)]` attribute. Write `#[get(...)]` (etc.) **instead of** `#[utoipa::path(...)]`, not in addition to it.
  - Internally, each annotated handler submits a factory into a global `inventory` collection at compile time (the same pattern `stano-di`'s `#[service]` uses for DI registration); `collect_routes()` (see below) folds all of them into one `OpenApiRouter` at startup.
  - These macros don't apply any auth/authz middleware themselves — there's no `auth = <guard_fn>` argument today. Per-route auth is planned as its own dedicated macro; until then, apply `.layer(axum::middleware::from_fn(guard))` by hand to routes you compose yourself and pass in via `run()`'s `extra_routes`.

- **`stano_launcher::routes::collect_routes() -> OpenApiRouter<Arc<ApplicationContext>>`** — builds the router from every `#[get]`/`#[post]`/etc.-annotated handler in the binary. Called automatically by `run()` — you normally don't need to call it directly.

### Server Startup

- **`run(ctx: Arc<ApplicationContext>, extra_routes: OpenApiRouter<Arc<ApplicationContext>>, config: BootstrapConfig) -> Result<(), anyhow::Error>`** — start the server.
  - Merges `collect_routes()` (every `#[get]`/`#[post]`/etc.-annotated handler) with `extra_routes` (anything you built by hand — pass `OpenApiRouter::new()` if there's nothing extra).
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

When `enable_swagger` is true, the Swagger UI router (`/swagger`, `/api-docs/openapi.json`) is merged in alongside the auto-registered and `extra_routes` routers before the middleware stack is applied, so it's subject to the same CORS/timeout/compression/security-header handling as the rest of the API.

## Usage Example

```rust
use stano_di::{ApplicationContext, OsEnvironment};
use stano_launcher::{
    get, post, BootstrapConfig, is_dev_environment, observability_config_from_env, parse_csv_env, run,
};
use stano_security::JwtConfig;
use std::sync::Arc;
use utoipa_axum::router::OpenApiRouter;

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
        enable_swagger: is_dev_environment(env.as_ref()),
        observability: observability_config_from_env(env.as_ref()),
    };

    // Build the DI container.
    let mut ctx = ApplicationContext::new(env);
    // Register your services here...
    ctx.validate().map_err(|errs| anyhow::anyhow!("{errs:?}"))?; // Validate the container (detects cycles, etc.)

    // Start the server (blocks until Ctrl+C or SIGTERM). Every #[get]/#[post]/etc.-annotated
    // handler anywhere in the binary is auto-registered; extra_routes is for anything else.
    run(Arc::new(ctx), OpenApiRouter::new(), config).await
}

// Your handlers — #[get(...)]/#[post(...)]/etc. replace #[utoipa::path(...)], inferring
// operation_id/request_body/the 200 response/params(...) from the handler's signature.

#[post(path = "/api/users", responses((status = 200, body = UserResponse)))]
async fn create_user_handler(/* ... */) -> /* ... */ { /* ... */ }

#[get(path = "/api/profile", responses((status = 200, body = ProfileResponse)))]
async fn get_profile_handler(/* ... */) -> /* ... */ { /* ... */ }
```

## Notes

- **No built-in auth mechanism yet** — `#[get]`/`#[post]`/etc. don't apply any middleware; there's no `auth = <guard_fn>` argument. Per-route auth is planned as its own dedicated macro. Until then, build any auth-guarded routes by hand (`OpenApiRouter::new().routes(routes!(handler)).layer(axum::middleware::from_fn(guard))`) and pass them in via `run()`'s `extra_routes`.
- **Route merging** — every `#[get]`/`#[post]`/etc.-annotated handler and `extra_routes` are merged into a single router, so define paths carefully to avoid collisions.
- **Graceful shutdown** — the server responds to Ctrl+C on all platforms and SIGTERM on Unix-like systems. Connections are drained gracefully.
- **CORS configuration** — pass `cors_origins`, `cors_origin_suffixes`, and `cors_dev_origins` all empty for permissive CORS (allow any origin); otherwise list exact origins and/or origin suffixes. Populate any of these from an env var with `parse_csv_env`, or set them directly from your app config. Set `is_dev` (via `is_dev_environment` or explicitly) to switch to `cors_dev_origins` in development.
- **Observability is automatic, not opt-in per call** — `run()` always calls `init_observability`; set `observability.enabled = false` (the default via `observability_config_from_env`) to get a plain console subscriber and skip OTLP export entirely, e.g. for local dev without a collector. If your app already installs its own `tracing_subscriber`, don't call `run()` with `enabled: true` at the same time — only one global subscriber can be installed per process.
- **Swagger UI is auto-discovered, not hand-maintained** — because `#[get]`/`#[post]`/etc. generate a `#[utoipa::path]` attribute internally, the OpenAPI document served at `/api-docs/openapi.json` is generated from the same code that defines the routes. There is no separate spec file to keep in sync by hand.
- **No feature flags** — all APIs available.

See also: [`stano-di`](../stano-di), [`stano-axum`](../stano-axum), [`stano-security`](../stano-security).
