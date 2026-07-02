# Stano Platform

Reusable Rust library crates for building modular Axum web applications. This repo provides the platform; consuming applications implement their own domain/services/infrastructure/http layers on top of these crates.

## Workspace

| Crate | Purpose |
|-------|---------|
| `stano-di` | Lightweight DI container with lazy singleton resolution, cycle detection, and async-safe validation |
| `stano-di-macros` | Proc macros (`#[component]`, `#[service]`) that generate boilerplate and auto-register components |
| `stano-common` | Shared error types, typed ID macro, and utilities used across the platform |
| `stano-security` | JWT encode/decode (ES256) and generic `SecurityContext<E>` for auth |
| `stano-axum` | HTTP extractors, unified `ApiError` type, error-logging middleware |
| `stano-seaorm` | Postgres pool configuration and domain↔entity `Mapper<T>` trait |
| `stano-launcher` | App bootstrap: wires Axum router, applies middleware stack, handles graceful shutdown |

## Dependency Graph

```
stano-common              stano-security            stano-di              stano-seaorm
     (no deps)                (no deps)              (no deps)                (no deps)
                                  ↓                        ↓
                          stano-di-macros           stano-axum
                              (depends on            (depends on
                              stano-di)          stano-common,
                                                 stano-security)
                                                      ↓
                                                stano-launcher
                                                (depends on
                                                stano-di,
                                              stano-security,
                                              stano-axum)
```

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
- `ErrorResponse` — JSON error body: `status`, `code`, `message`, `details`, `request_id`
- `AppJson<T>`, `AppPath<T>`, `AppQuery<T>` — extractors wrapping Axum's `Json`, `Path`, `Query` with structured error conversion
- `error_logging_middleware` — logs `ApiError` with request context (method, uri, request_id)

**stano-seaorm:**
- `DbConfig::from_url()` — Postgres connection pool (max 100, min 5, 30s acquire timeout, 10min idle, 1h max lifetime)
- `Mapper<Domain>` trait — `to_domain(Model) -> Domain` and `to_active_model(Domain) -> ActiveModel` for bidirectional entity mapping
- Re-exports: `sea_orm` crate

**stano-launcher:**
- `BootstrapConfig` — `port`, `jwt_config: JwtConfig`, `cors_origins`
- `RouteGroups` — three `Router<Arc<ApplicationContext>>` tiers: `public`, `protected`, `admin` (currently merged with no automatic differential auth — see "Platform Limitations" below)
- `run()` — composes routes, applies middleware stack, handles graceful shutdown on Ctrl+C/SIGTERM
- Middleware stack (outer → inner): CORS → 300s timeout → TraceLayer → `error_logging_middleware` → CatchPanicLayer → CompressionLayer → SetRequestIdLayer → PropagateRequestIdLayer → 10MB body limit → custom security headers (nosniff, DENY, HSTS)

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
| **Errors** | `DomainError` in domain code only. Convert to `ServiceError` in services. Let `stano-axum` map to HTTP. Use `anyhow::Error` only inside `ServiceError::Internal` and infrastructure layers. |
| **Git** | User handles all commits and pushes — do not use `git commit` or `git push`. |

## Platform Limitations (vs README aspirations)

- **No `Role` enum**: `stano-security` provides generic `Claims<E>` and `SecurityContext<E>`. Apps define their own role/permission types via the `E` extension type — this crate does not mandate a fixed role model.
- **No automatic per-tier auth**: `stano-launcher`'s three route groups (`public`, `protected`, `admin`) are merged without differential middleware. Apps must implement route-level auth guards themselves (e.g., via middleware on protected routes, or guards in handlers).

## Build & Test

```bash
cargo build                    # Compile all crates
cargo test --workspace         # Run all tests
cargo clippy                   # Lint (must be zero warnings)
cargo fmt --check              # Check formatting
cargo make coverage            # Optional: generate coverage report (Mac/Linux)
```

---

*Stack: Axum 0.8, Tokio, SeaORM (Postgres), ES256 JWT. See `README.md` for quick-start and example app structure. Each crate is published independently; check Cargo.toml for current versions and features.*
