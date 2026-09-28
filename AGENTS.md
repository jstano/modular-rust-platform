# Stano Platform

Reusable Rust library crates for building modular Axum web applications. This repo provides the platform; consuming applications implement their own domain/services/persistence/http layers on top of these crates.

## Workspace

| Crate | Purpose |
|-------|---------|
| `stano-di` | Lightweight DI container with lazy singleton resolution, cycle detection, and async-safe validation |
| `stano-di-macros` | Proc macros (`#[component]`, `#[service]`) that generate boilerplate and auto-register components |
| `stano-common` | Shared error types, typed ID macro, and utilities used across the platform |
| `stano-security` | JWT encode/decode (ES256) and generic `SecurityContext<E>` for auth |
| `stano-axum` | HTTP extractors, unified `ApiError` type, error-logging middleware, declarative request-authorization chain, and the route auto-registration machinery (`routes::collect_routes()`, re-exported `#[get]`/`#[post]`/etc.) — so adapter crates that only write routes never need `stano-launcher` |
| `stano-seaorm` | Postgres pool configuration and domain↔entity `Mapper<T>` trait |
| `stano-launcher` | App bootstrap: wires Axum router, applies middleware stack, handles graceful shutdown |
| `stano-route-macros` | Proc macros (`#[get]`/`#[post]`/`#[put]`/`#[delete]`/`#[patch]`), re-exported from `stano-axum`, that infer `operation_id`, `request_body`, the `200` entry of `responses(...)`, and `params(...)` from the handler's signature, forward remaining `#[utoipa::path(...)]`-grammar keys verbatim, and auto-register the handler |
| `stano-starter` | Base scaffolding crate for consuming apps: shared setup atop `stano-common`/`stano-di`/`stano-di-macros` that domain/service/rest starter crates build on |
| `stano-starter-domain` | Example domain layer built on `stano-starter` |
| `stano-starter-service` | Example service layer built on `stano-starter` and `stano-security` |
| `stano-starter-rest` | Example REST adapter facade wiring `stano-axum`, `stano-di`, `stano-security` — deliberately excludes `stano-launcher`, since adapter/route crates shouldn't depend on the composition root |

## Dependency Graph

```
stano-common     stano-security     stano-di     stano-route-macros
   (no deps)         (no deps)      (no deps)         (no deps)
        ↓                 ↓              ↓                  ↓
        └────────────→  stano-axum  ←──────────────────────┘
        ↓              (depends on
   stano-seaorm       stano-common, stano-security,
  (depends on          stano-di, stano-route-macros —
    stano-di)           owns routes::collect_routes()
                         and re-exports #[get]/etc.)
                                    ↓
                              stano-launcher
                              (depends on
                                stano-di,
                              stano-security,
                                stano-axum)
```

`RouteRegistration`/`inventory::collect!`/`collect_routes()` and the `#[get]`/`#[post]`/etc. re-exports live in `stano-axum`, not `stano-launcher` — this is what lets an adapter crate that only defines routes (no server bootstrap) depend on `stano-axum` alone and never need `stano-launcher`. `stano-launcher` re-exports both (`pub use stano_axum::routes;` / `pub use stano_axum::{delete, get, patch, post, put};`) purely for callers that already depend on it directly, e.g. its own `run()` and test suite.

The `stano-starter*` crates are example/template crates for consuming apps, not platform library crates. None of them depend on `stano-launcher` — `stano-starter-rest` models the REST adapter layer (`rest_api`-style crates), which should depend on routing/extractors (`stano-axum`) but never on the composition root (`stano-launcher`) that assembles and runs the server:

```
stano-common, stano-di, stano-di-macros
          ↓
    stano-starter
   ┌──────┴──────────────┐
stano-starter-domain   stano-starter-service
                       (also depends on
                        stano-security)

stano-starter-rest   (sibling crate, not built on stano-starter)
(depends on stano-di, stano-axum,
 stano-security — deliberately no stano-launcher)
```

An app's actual composition-root crate (e.g. a `launcher` binary in a consuming app's own workspace) is what depends on both `stano-launcher` and the adapter crate built on `stano-starter-rest`, not the other way around.

## Key Types by Crate

**stano-di:**
- `Container` — TypeId-keyed factory/singleton registry with `register()`, `register_trait()`, `register_instance()`, cycle-detecting `validate()`
- `ApplicationContext` — wraps `Container` + `Arc<dyn Environment>` for app-level wiring
- `ContainerError` — `NotRegistered`, `DowncastFailed`, `FactoryPanic`, `CyclicDependency`
- `Component`, `Injectable`, `DynComponent` traits for registration contracts
- `register_all()` — consumes `inventory` collected service registrations

**stano-di-macros:**
- `#[component]` — marks traits as injectable (requires `Send + Sync`)
- `#[service(dyn Trait)]` — marks struct impls as factories (fields must be `Arc<T>`)
- Auto-registers via `inventory::submit!`

**stano-common:**
- `DomainError` — `InvalidInput`, `BusinessRuleViolation` (pure business logic errors)
- `ServiceError` — `NotFound`, `InvalidInput`, `Conflict`, `Unauthorized`, `Forbidden`, `Internal(#[from] anyhow::Error)` (service layer errors)
- `domain_err_to_service()` — conversion utility
- `id_type!(Name, uuid_v4|uuid_v7)` — macro generating typed UUID wrappers with `new()`, `from()`, `as_uuid()`, `FromStr`, `Display`
- Re-exports: `uuid` crate

**stano-security:**
- `JwtConfig` — EC key paths and expiration seconds
- `Claims<E>` — generic JWT payload with `sub`, `session_id`, `exp`, and app-defined `ext: E`
- `SecurityContext<E>` — wraps claims with accessors: `sub()`, `session_id()`, `ext()`, `claims()`
- `encode_jwt()` / `decode_jwt()` — ES256 encode/decode using EC PEM keys
- `JwtError` — encoding/decoding/key/expiration errors

**stano-axum:**
- `ApiError` — HTTP response type (implements `IntoResponse`), maps `ServiceError` variants to status codes
- `ErrorResponse` — JSON error body: `status`, `code`, `message`, `details`, `request_id`; derives `utoipa::ToSchema` for use in `#[utoipa::path(responses(...))]`
- `AppJson<T>`, `AppPath<T>`, `AppQuery<T>` — extractors wrapping Axum's `Json`, `Path`, `Query` with structured error conversion
- `AppSecurityContext<E>` — extractor pulling the `SecurityContext<E>` inserted into request extensions by `AuthorizationLayer`; rejects with 401 if absent
- `error_logging_middleware` — logs `ApiError` with request context (method, uri, request_id)
- `security::AuthorizationBuilder<E>` — Spring-Security-style declarative request-authorization chain, mirroring `HttpSecurity::authorizeHttpRequests(...)`: `.request_matcher(pattern)` (matchit syntax: `{id}`, `{*rest}`) or `.any_request()` (mandatory terminal rule), each followed by `.permit_all()` / `.authenticated()` / `.has_role(pred)` / `.has_any_role(preds)`; optionally narrowed to specific methods via `.methods([...])`. Rules are evaluated in registration order, first match wins. `.build(jwt_config)` returns `Result<AuthorizationLayer, AuthorizationBuildError>`, erroring if no `.any_request()` was configured.
- `security::AuthorizationLayer` — the type-erased, `Clone`-able result of `.build(...)`, passed into `stano_launcher::run(...)`'s `authorization` parameter. Auth failures produce `ApiError`-shaped 401/403 responses, so `error_logging_middleware` picks them up automatically. Also records an `http.server.auth.failures` OTel counter (attributes `http.request.method`, `http.response.status_code`) on every 401/403 it produces — always on, no separate switch. The counter is resolved lazily, on the first actual rejection, not eagerly when `.build(...)` runs — `opentelemetry::global::meter(...)` permanently binds to whichever `MeterProvider` is installed at call time and apps commonly call `.build(...)` before `stano_launcher::run()` (which is what installs the real one), so eager resolution would silently produce a no-op counter forever.
- `#[get(...)]` / `#[post(...)]` / `#[put(...)]` / `#[delete(...)]` / `#[patch(...)]` (re-exported from `stano-route-macros`) — per-HTTP-method attribute macros that infer as much as possible from the handler's signature and forward the rest of `#[utoipa::path(...)]`'s argument grammar (`path`, `tag`/`tags`, `security`, extra `responses(...)` entries) verbatim, then auto-register the handler via `inventory::submit!`, mirroring the `#[service]`/`inventory` pattern in `stano-di-macros`. Write `#[get(...)]` (etc.) in place of `#[utoipa::path(...)]`, not stacked with it. Inference rules: the HTTP method comes from which macro is used; `operation_id` is the function name; `request_body` is inferred from a single `AppJson<T>` parameter; the `200` entry of `responses(...)` is inferred from an `AppJson<T>` or `Result<AppJson<T>, E>` return type (the `Err` type's status is never inferred — add error entries via `responses(...)` explicitly); `params(...)` is inferred from `AppPath<T>`/`AppQuery<T>` parameters — `AppQuery<T>` and multi-segment `AppPath<T>` require `T: utoipa::IntoParams`, while a single-placeholder `AppPath<primitive-or-`Id`-suffixed-type>` gets the familiar `("name" = T, Path)` form generated automatically. `path`, `tag`/`tags`, and `security` remain explicit-only. No per-handler auth annotation yet — see "Platform Limitations" below; global path-pattern authorization is available via `security::AuthorizationBuilder`, above.
- `routes::collect_routes()` — folds every `#[get]`/`#[post]`/etc. registration into an `OpenApiRouter<Arc<ApplicationContext>>` (from `utoipa_axum`); called automatically by `stano_launcher::run()`. Because these macros generate a `#[utoipa::path(...)]` attribute internally, the OpenAPI spec is generated from the same code that defines the routes — no hand-maintained spec file to drift out of sync. Defined here (not in `stano-launcher`) so a crate that only writes routes never needs to depend on `stano-launcher`.

**stano-seaorm:**
- `DbConfig::from_url()` — Postgres connection pool (max 100, min 5, 30s acquire timeout, 10min idle, 1h max lifetime)
- `Mapper<Domain>` trait — `to_domain(Model) -> Domain` and `to_active_model(Domain) -> ActiveModel` for bidirectional entity mapping
- Query tracing reads env-based config via `stano_di::environment::Environment`, mirroring `stano-launcher::observability`'s pattern (this is the crate's one platform dependency)
- Re-exports: `sea_orm` crate

**stano-launcher:**
- `BootstrapConfig` — `port`, `jwt_config: JwtConfig`, `cors_origins`, `enable_swagger` (mounts `/swagger` + `/api-docs/openapi.json` when true)
- `run(ctx, extra_routes, config, authorization)` — merges `stano_axum::routes::collect_routes()` (re-exported as `routes::collect_routes()`) with `extra_routes` (anything not using `#[get]`/`#[post]`/etc.), applies middleware stack, handles graceful shutdown on Ctrl+C/SIGTERM. `authorization: Option<stano_axum::security::AuthorizationLayer>` — pass `None` to skip authorization enforcement entirely.
- Middleware stack (outer → inner): CORS → 300s timeout → TraceLayer → `error_logging_middleware` → CatchPanicLayer → CompressionLayer → SetRequestIdLayer → PropagateRequestIdLayer → 10MB body limit → `AuthorizationLayer` (if configured) → custom security headers (nosniff, DENY, HSTS)
- Re-exports `stano_axum::routes` and `stano_axum::{delete, get, patch, post, put}` for convenience — the actual definitions live in `stano-axum` (see above).

**stano-route-macros:**
- `#[get]` / `#[post]` / `#[put]` / `#[delete]` / `#[patch]` — see `stano-axum`, above; the proc macros that generate the `inventory::submit!` registration. Their generated code resolves a path to whichever of `stano-axum` (direct dependency), `stano-launcher`, or `stano-starter-rest` (nested re-exports) the caller depends on.

## Error Chain

```
DomainError (app domain layer)
    ↓ domain_err_to_service()
ServiceError (app service layer)
    ↓ impl From/IntoResponse in stano-axum
ApiError (HTTP response, 400/401/403/404/409/500)
```

## Cross-Cutting Rules

| Concern | Rule |
|---------|------|
| **IDs** | Use `id_type!(Name, uuid_v4\|uuid_v7)` macro — never raw `Uuid` types. v7 (sortable) for primary entities, v4 (random) for nonces/transient IDs. |
| **Errors** | `DomainError` in domain code only. Convert to `ServiceError` in services. Let `stano-axum` map to HTTP. Use `anyhow::Error` only inside `ServiceError::Internal` and persistence layers. |
| **Git** | User handles all commits and pushes — do not use `git commit` or `git push`. |

## Platform Limitations (vs README aspirations)

- **No `Role` enum**: `stano-security` provides generic `Claims<E>` and `SecurityContext<E>`. Apps define their own role/permission types via the `E` extension type — this crate does not mandate a fixed role model. `AuthorizationBuilder<E>::has_role(...)` takes an app-supplied `Fn(&E) -> bool` predicate rather than a platform-defined role type.
- **The filter-chain model is built; the per-method annotation model is not.** `stano-axum::security::AuthorizationBuilder` covers Spring's `authorizeHttpRequests(...)` (global, ordered path-pattern rules). A dedicated per-handler auth annotation macro (in the spirit of Spring Security's `@PreAuthorize`, e.g. `#[get(auth = <guard_fn>)]`) is still planned but not designed or built — that argument existed briefly and was removed prior to this repo's recorded history. Until then, apps needing a guard on one specific handler that the path-pattern chain can't express build that route by hand (`OpenApiRouter::new().routes(routes!(handler)).layer(axum::middleware::from_fn(guard))`) and pass it into `run()` via `extra_routes`.
- **OpenAPI security is per-handler**: `#[utoipa::path(security(...))]`/`#[get(security(...))]` (etc.) requirements are declared on each handler individually — nothing infers a `bearerAuth` requirement automatically.

## Build & Test

```bash
cargo build                    # Compile all crates
cargo test --workspace         # Run all tests
cargo clippy                   # Lint (must be zero warnings)
cargo fmt --check              # Check formatting
cargo make coverage            # Optional: generate coverage report (Mac/Linux)
```

`stano-example-app`'s persistence/launcher integration tests each start their own
throwaway Postgres via `testcontainers` (no `docker compose up` needed first, cleaned up
automatically). This needs Docker running; on Colima/Podman/Rancher Desktop rather than
Docker Desktop, set `DOCKER_HOST` to your active context's socket (`docker context ls`)
first — `testcontainers` talks to the daemon socket directly, unlike `docker compose`,
which respects the `docker` CLI's active context automatically.

---

*Stack: Axum 0.8, Tokio, SeaORM (Postgres), ES256 JWT. See `README.md` for quick-start and example app structure. Each crate is published independently; check Cargo.toml for current versions and features.*
