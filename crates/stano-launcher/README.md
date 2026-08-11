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
  - `enabled: bool` — master switch. When `false` (the default), only a local `fmt` + `EnvFilter` console subscriber is installed (JSON-formatted) and no OTLP export happens — safe for local dev without a collector.
  - `protocol: OtlpProtocol` — `Grpc` or `HttpProtobuf`, runtime-selectable (both exporter transports are compiled in). Note there's no `otlp_endpoint` field — the collector endpoint isn't configured here; `opentelemetry-otlp` reads `OTEL_EXPORTER_OTLP_ENDPOINT` (and the per-signal `_TRACES_`/`_LOGS_`/`_METRICS_ENDPOINT` variants) from the real process environment directly, the same way it already reads `OTEL_EXPORTER_OTLP_HEADERS`. This also gets the OTLP-spec-required `/v1/traces`/`/v1/logs`/`/v1/metrics` path suffix appended automatically for HTTP/protobuf, which passing the endpoint programmatically used to bypass.
  - `service_name: String` / `service_version: String` — OTel resource attributes.
  - `resource_attributes: Vec<(String, String)>` — additional OTel resource attributes, e.g. `("deployment.environment", "prod")`.
  - `trace_sample_ratio: f64` — trace sampling ratio in `0.0..=1.0`.
  - `log_filter: String` — `tracing_subscriber::EnvFilter` directive string, e.g. `"info,my_app=debug"`.
  - `metrics_enabled: bool` — independently enables OTLP metrics export (push, requires `enabled: true` and a live collector) and the HTTP server metrics middleware (request count/duration, active requests).
  - `prometheus_enabled: bool` — independently exposes a local Prometheus scrape endpoint at `GET /metrics` (text-exposition format), serving the same metrics recorded via the global OTel meter, including HTTP server metrics when the metrics middleware is mounted. Unlike `metrics_enabled` (an OTLP *push* exporter to a collector), this is a *pull* exporter with no collector dependency, so it works even when `enabled` is `false`. Also implicitly mounts the HTTP server metrics middleware (same as `metrics_enabled`) so there's something to scrape.
  - `http_logging_enabled: bool` — independently enables `stano_axum::http_request_logging_middleware`, which logs every HTTP request (method, URI, status, latency, and the current span's OTel trace_id).
  - `process_metrics_enabled: bool` — independently spawns a background process CPU/memory/disk-I/O observer (`opentelemetry-system-metrics`), recording `process.cpu.usage`, `process.cpu.utilization`, `process.memory.usage`, `process.memory.virtual`, `process.disk.io` on an interval (`OTEL_METRIC_EXPORT_INTERVAL`, read directly by that crate, spec default 30s). Only takes effect when at least one meter reader exists (`metrics_enabled` and/or `prometheus_enabled`) — otherwise the poller isn't started. Deliberately not folded into `metrics_enabled`: container/orchestration-level scraping (cAdvisor, kubelet, Docker stats) usually already covers this more accurately, so it's opt-in rather than bundled.

- **`observability_config_from_env(environment: &dyn Environment) -> ObservabilityConfig`** — reads standard OTel env vars (`OTEL_EXPORTER_OTLP_PROTOCOL`, `OTEL_SERVICE_NAME`, `OTEL_SERVICE_VERSION`, `OTEL_TRACES_SAMPLER_ARG`, `RUST_LOG`) plus `MRP_OTEL_ENABLED`/`MRP_OTEL_METRICS_ENABLED`/`MRP_PROMETHEUS_ENABLED`/`MRP_HTTP_LOGGING_ENABLED`/`MRP_PROCESS_METRICS_ENABLED` (all default `false`). Deliberately does *not* read `OTEL_EXPORTER_OTLP_ENDPOINT` — see the `protocol` field note above.

- **`OtelGuard`** — returned internally by `init_observability` and held by `run()` for the request lifetime; flushed after `axum::serve(...)` resolves so in-flight spans/metrics are exported before shutdown. Also exposes `prometheus_registry()`, present when `prometheus_enabled` is true, which `run()` uses to mount `/metrics`.

`run()` calls `init_observability(&config.observability)` as the first thing it does, so all subsequent `tracing::*!` calls (including from `stano-di`, `stano-axum`, and your own app code) are captured. No call-site changes are needed anywhere — this composes a `tracing_subscriber::Registry` that the plain `tracing` facade already flows through.

### Route Auto-Registration

- **`#[get(...)]` / `#[post(...)]` / `#[put(...)]` / `#[delete(...)]` / `#[patch(...)]`** — per-HTTP-method attribute macros (from `stano-route-macros`, re-exported here) that replace `#[utoipa::path(...)]` on a handler and auto-register it, so it's picked up by `run()` without a manual `.routes(routes!(handler))` call.
  - Infer `operation_id` (from the function name), `request_body` (from a single `AppJson<T>` parameter), the `200` entry of `responses(...)` (from an `AppJson<T>`/`Result<AppJson<T>, E>` return type), and `params(...)` (from `AppPath<T>`/`AppQuery<T>` parameters); forward everything else (`path`, `tag`/`tags`, `security`, extra `responses(...)` entries) verbatim into a generated `#[utoipa::path(...)]` attribute. Write `#[get(...)]` (etc.) **instead of** `#[utoipa::path(...)]`, not in addition to it.
  - Internally, each annotated handler submits a factory into a global `inventory` collection at compile time (the same pattern `stano-di`'s `#[service]` uses for DI registration); `collect_routes()` (see below) folds all of them into one `OpenApiRouter` at startup.
  - These macros don't apply any auth/authz middleware themselves — auth enforcement is applied once, globally, via `run()`'s `authorization` parameter (see Authorization below), not per-route. `security(...)` inside a macro's attributes is documentation only (feeds the generated OpenAPI doc) and has no runtime effect — keep it in sync with your `AuthorizationBuilder` rules by hand.

- **`stano_launcher::routes::collect_routes() -> OpenApiRouter<Arc<ApplicationContext>>`** — builds the router from every `#[get]`/`#[post]`/etc.-annotated handler in the binary. Called automatically by `run()` — you normally don't need to call it directly.

### Authorization

- **`authorization: Option<stano_axum::security::AuthorizationLayer>`** (`run()`'s 4th parameter) — a declarative, Spring-Security-style rule table built via `stano_axum::security::AuthorizationBuilder`, applied globally as the outermost-but-one layer (just inside CORS). Pass `None` to skip authorization enforcement entirely (every route open). See `stano-axum`'s docs for `AuthorizationBuilder`'s API (`.request_matcher(...)`, `.permit_all()`/`.authenticated()`/`.has_role(...)`, `.with_claims_validator(...)`, `.cookie_name(...)`, `.any_request()...build(jwt_config)`).
- **`post_authorization: Option<stano_axum::security::PostAuthorizationHook>`** (`run()`'s 5th parameter) — an optional middleware hook applied immediately after `authorization` in request-flow order (between authorization and the router, closer to handlers). Exists so a consumer can bridge whatever `AuthorizationLayer` inserted into request extensions (a `stano_security::SecurityContext<E>`) into an app-local mechanism (e.g. a `tokio::task_local!` your service layer already reads from), without `stano-launcher`/`stano-axum` needing to know that mechanism exists. Build one via `PostAuthorizationHook::from_fn(your_middleware_fn)`, where `your_middleware_fn` matches `axum::middleware::from_fn`'s simplest shape: `async fn(Request, Next) -> impl IntoResponse`. Pass `None` to skip it — has no effect if `authorization` is also `None`.

### Server Startup

- **`run(ctx: Arc<ApplicationContext>, extra_routes: OpenApiRouter<Arc<ApplicationContext>>, config: BootstrapConfig, authorization: Option<AuthorizationLayer>, post_authorization: Option<PostAuthorizationHook>) -> Result<(), anyhow::Error>`** — start the server.
  - Merges `collect_routes()` (every `#[get]`/`#[post]`/etc.-annotated handler) with `extra_routes` (anything you built by hand — pass `OpenApiRouter::new()` if there's nothing extra).
  - Applies a fixed middleware stack (see below), including `authorization`/`post_authorization` if provided.
  - Binds a `TcpListener` on `0.0.0.0:{port}`.
  - Logs "Listening on port {port}".
  - Runs until Ctrl+C (or SIGTERM on Unix) is received, then performs graceful shutdown.

## Middleware Stack

Applied in this request-processing order (outermost → innermost, closest to handlers):

1. **CORS** — allow/disallow origins based on config. Deliberately outermost so it can answer preflight `OPTIONS` requests directly, without them ever reaching `authorization` — a rule table scoped to specific methods (GET/POST/etc, not OPTIONS) would otherwise reject preflight requests to protected routes with 401.
2. **Security headers** — injects `x-content-type-options: nosniff`, `x-frame-options: DENY`, `strict-transport-security: max-age=31536000; includeSubDomains`.
3. **Authorization** *(when `authorization` is `Some`)* — the declarative `AuthorizationLayer` rule table (see Authorization above).
4. **Post-authorization hook** *(when `post_authorization` is `Some`)* — runs immediately after authorization, closer to handlers (see Authorization above).
5. **Request body limit** — 10 MB max request body.
6. **Propagate request ID** — propagates `x-request-id` upstream.
7. **Set request ID** — injects a unique `x-request-id` if not present.
8. **Compression** — gzip/brotli/deflate (auto-negotiated).
9. **Catch panic** — panics in handlers become 500 responses.
10. **Error logging** — logs `ApiError` with request context (see `stano_axum::error_logging_middleware`).
10a. **HTTP request logging** *(when `observability.http_logging_enabled` is true)* — logs every request with method, URI, status, latency, and trace_id (see `stano_axum::http_request_logging_middleware`). Sits just inside the Tracing layer so it runs within the same span and can read a valid OTel trace_id.
11. **Tracing** — structured request/response logging via `tracing`, exported via OTLP when `observability.enabled` is true (see Observability above).
12. **Timeout** — 300-second per-request timeout.

Additionally, when `observability.metrics_enabled` or `observability.prometheus_enabled` is true, an HTTP metrics middleware is applied via `route_layer` (so it only wraps matched routes, giving it access to `MatchedPath` for the `http.route` attribute) — this records `http.server.request.duration` and `http.server.active_requests` via the global OTel meter.

When `enable_swagger` is true, the Swagger UI router (`/swagger`, `/api-docs/openapi.json`) is merged in alongside the auto-registered and `extra_routes` routers before the middleware stack is applied, so it's subject to the same CORS/timeout/compression/security-header handling as the rest of the API.

When `observability.prometheus_enabled` is true, a `GET /metrics` route is merged in the same way, before `.with_state(...)` — so it's also subject to the same CORS/timeout/compression/security-header/authorization handling as the rest of the API.

## Usage Example

```rust
use stano_axum::security::{AuthorizationBuilder, PostAuthorizationHook};
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
    let jwt_config = JwtConfig {
        private_key_pem: env.get("JWT_PRIVATE_KEY").expect("required"),
        public_key_pem: env.get("JWT_PUBLIC_KEY").expect("required"),
        expiration_seconds: 3600,
    };
    let config = BootstrapConfig {
        port: env.get("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000),
        jwt_config: jwt_config.clone(),
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

    // Declarative, per-route auth — every route not matched by an earlier rule falls to
    // `.any_request()`. Pass `None` in place of `Some(authorization)` below to skip
    // enforcement entirely.
    let authorization = AuthorizationBuilder::<()>::new()
        .request_matcher("/api/public/{*rest}")
        .permit_all()
        .any_request()
        .authenticated()
        .build(jwt_config)?;

    // Start the server (blocks until Ctrl+C or SIGTERM). Every #[get]/#[post]/etc.-annotated
    // handler anywhere in the binary is auto-registered; extra_routes is for anything else.
    run(
        Arc::new(ctx),
        OpenApiRouter::new(),
        config,
        Some(authorization),
        None, // or Some(PostAuthorizationHook::from_fn(your_middleware_fn))
    )
    .await
}

// Your handlers — #[get(...)]/#[post(...)]/etc. replace #[utoipa::path(...)], inferring
// operation_id/request_body/the 200 response/params(...) from the handler's signature.
// `security(...)` documents which routes need a bearer token — it has no runtime effect,
// so keep it in sync with the `AuthorizationBuilder` rules above by hand.

#[post(
    path = "/api/users",
    responses((status = 200, body = UserResponse)),
    security(("bearerAuth" = []))
)]
async fn create_user_handler(/* ... */) -> /* ... */ { /* ... */ }

#[get(path = "/api/profile", responses((status = 200, body = ProfileResponse)))]
async fn get_profile_handler(/* ... */) -> /* ... */ { /* ... */ }
```

## Notes

- **Auth is global, not per-route** — `#[get]`/`#[post]`/etc. don't apply any middleware themselves; there's no `auth = <guard_fn>` argument, and none is planned. Enforcement is entirely `run()`'s `authorization`/`post_authorization` parameters (see Authorization above) — a single declarative rule table covering every route, macro-registered or `extra_routes`.
- **Route merging** — every `#[get]`/`#[post]`/etc.-annotated handler and `extra_routes` are merged into a single router, so define paths carefully to avoid collisions.
- **Graceful shutdown** — the server responds to Ctrl+C on all platforms and SIGTERM on Unix-like systems. Connections are drained gracefully.
- **CORS configuration** — pass `cors_origins`, `cors_origin_suffixes`, and `cors_dev_origins` all empty for permissive CORS (allow any origin); otherwise list exact origins and/or origin suffixes. Populate any of these from an env var with `parse_csv_env`, or set them directly from your app config. Set `is_dev` (via `is_dev_environment` or explicitly) to switch to `cors_dev_origins` in development.
- **Observability is automatic, not opt-in per call** — `run()` always calls `init_observability`; set `observability.enabled = false` (the default via `observability_config_from_env`) to get a JSON-formatted console subscriber and skip OTLP export entirely, e.g. for local dev without a collector. Console output is always JSON, whether or not OTLP export is enabled. If your app already installs its own `tracing_subscriber`, don't call `run()` with `enabled: true` at the same time — only one global subscriber can be installed per process.
- **Swagger UI is auto-discovered, not hand-maintained** — because `#[get]`/`#[post]`/etc. generate a `#[utoipa::path]` attribute internally, the OpenAPI document served at `/api-docs/openapi.json` is generated from the same code that defines the routes. There is no separate spec file to keep in sync by hand.
- **No feature flags** — all APIs available.

See also: [`stano-di`](../stano-di), [`stano-axum`](../stano-axum), [`stano-security`](../stano-security).
