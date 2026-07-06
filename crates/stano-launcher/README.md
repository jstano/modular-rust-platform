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
  - `cors_origins: Vec<String>` — allowed CORS origins (empty list = permissive).

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
8. **Tracing** — structured request/response logging via `tracing`.
9. **Timeout** — 300-second per-request timeout.
10. **CORS** — allow/disallow origins based on config.

## Usage Example

```rust
use axum::routing::{get, post};
use stano_di::{ApplicationContext, OsEnvironment};
use stano_launcher::{BootstrapConfig, RouteGroups, run};
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
        cors_origins: vec![], // empty = permissive
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
- **CORS configuration** — pass empty `cors_origins` for permissive CORS (allow any origin); otherwise list specific origins as strings.
- **No feature flags** — all APIs available.

See also: [`stano-di`](../stano-di), [`stano-axum`](../stano-axum), [`stano-security`](../stano-security).
