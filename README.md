# Stano Platform

A set of reusable Rust library crates for building modular web applications. Provides composable, type-safe building blocks for dependency injection, authentication, HTTP routing, and database access.

Apps depend on these crates and implement their own domain/services/infrastructure layers.

## Architecture

```
Your Application (domain, services, infrastructure, http handlers)
    ↓
stano-launcher (router wiring, middleware, auth, graceful shutdown)
    ↓
┌────────────────────────────────────────────────────┐
│ stano-di (IoC)   stano-axum (HTTP)   stano-seaorm  │
│ stano-security (JWT)  stano-common (errors, IDs)   │
│ stano-di-macros                                    │
└────────────────────────────────────────────────────┘
```

**Dependency flow:** App → launcher → platform crates. Domain has zero external deps.

## Quick Start

Create a minimal app:

### 1. Add Dependencies

```toml
[dependencies]
stano-di       = { git = "...", package = "stano-di" }
stano-launcher = { git = "...", package = "stano-launcher" }
stano-security = { git = "...", package = "stano-security" }
tokio          = { version = "1", features = ["macros", "rt-multi-thread"] }
axum           = "0.8"
```

### 2. Set Up Environment

Create `.env`:

```env
JWT_PRIVATE_KEY="-----BEGIN EC PRIVATE KEY-----\n...\n-----END EC PRIVATE KEY-----"
JWT_PUBLIC_KEY="-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"
PORT=3000
```

Generate keys:

```bash
openssl ecparam -name prime256v1 -genkey -noout -out private.pem
openssl ec -in private.pem -pubout -out public.pem
```

### 3. Implement main.rs

```rust
use axum::routing::get;
use stano_di::{application_context::ApplicationContext, environment::OsEnvironment};
use stano_launcher::{BootstrapConfig, RouteGroups, run};
use stano_security::JwtConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env = Arc::new(OsEnvironment::new());

    let config = BootstrapConfig {
        port: env.get("PORT").and_then(|p| p.parse().ok()).unwrap_or(3000),
        jwt_config: JwtConfig {
            private_key_pem: env.get("JWT_PRIVATE_KEY").expect("required"),
            public_key_pem: env.get("JWT_PUBLIC_KEY").expect("required"),
            expiration_seconds: 3600,
        },
        cors_origins: vec![],  // empty = permissive
    };

    let mut ctx = ApplicationContext::new(env);
    // Register services, repos, etc. here
    ctx.validate().map_err(|errs| anyhow::anyhow!("{errs:?}"))?;

    let routes = RouteGroups {
        public: axum::Router::new().route("/health", get(|| async { "ok" })),
        protected: axum::Router::new(),
        admin: axum::Router::new(),
    };

    run(Arc::new(ctx), routes, config).await
}
```

Run:

```bash
cargo run
curl http://localhost:3000/health
```

---

## Crates

### `stano-launcher` — App Bootstrap

Wires Axum router with a fixed middleware stack and manages graceful shutdown.

**Key types:** `BootstrapConfig`, `RouteGroups`, `run()`

Provides:
- Three route tiers (`public`, `protected`, `admin`) for organizational clarity — no automatic auth layer (you implement and apply auth/authz to your routers)
- Full middleware stack: CORS, timeout, tracing, error logging, panic handling, compression, request-id, body limits, security headers
- Graceful shutdown on SIGINT/SIGTERM
- Server bootstrap via `run(ctx, routes, config)` listening on configured port

---

### `stano-di` — Dependency Injection Container
A lightweight IoC container with:
- Lazy singleton resolution via `OnceLock`
- Type-safe dependency injection with macros
- Cycle detection during validation
- Environment variable loading via `dotenvy`

**Key types:** `Container`, `ApplicationContext`, `Environment`

### `stano-di-macros` — DI Macros
Procedural macros that generate boilerplate:
- `#[component]` — marks traits as injectable components
- `#[service(dyn Trait)]` — marks impls as trait object factories

### `stano-common` — Shared Types
Platform primitives used everywhere:
- `id_type!` macro — generates typed UUID wrappers (`uuid_v4` and `uuid_v7` variants)
- `ServiceError` — standard service layer error type
- `DomainError` — business logic error type
- `domain_err_to_service()` — conversion utility

### `stano-security` — Authentication
JWT and security context:
- `Claims<E>` — generalized JWT payload (generic `E` for app-defined extensions: email, role, custom claims)
- `SecurityContext<E>` — wraps claims for request context
- `JwtConfig`, `encode_jwt()`, `decode_jwt()` — JWT utilities (ES256)

### `stano-axum` — HTTP Layer
Axum extractors, error handling, and middleware:
- `AppJson<T>`, `AppPath<T>`, `AppQuery<T>` — custom extractors with structured errors
- `ApiError` — unified HTTP error type that maps `ServiceError` → HTTP status
- `ErrorResponse` — standard JSON error response
- `error_logging_middleware` — logs errors with request context

### `stano-seaorm` — Database Layer
SeaORM helpers:
- `DbConfig` — manages database connection pools
- `Mapper<Domain>` — trait for bidirectional domain ↔ DB conversion

## Starter Crates (Convenience Re-exports)

Four convenience crates bundle platform crates by app layer, reducing `Cargo.toml` boilerplate:

- **`stano-starter`** — re-exports `stano-common`, `stano-di`, `stano-di-macros` (domain/DI foundation).
- **`stano-starter-domain`** — thin re-export of `stano-starter` under a domain-focused name.
- **`stano-starter-service`** — re-exports `stano-starter` + `stano-security` (domain + DI + JWT).
- **`stano-starter-rest`** — re-exports `stano-di` + `stano-axum` + `stano-launcher` + `stano-security` (HTTP layer).

Each contains no code of its own — use them to simplify dependency declarations in your app's layers. See each crate's `README.md` for details.

## Usage Patterns

### Define an ID Type
```rust
use stano_common::id_type;

id_type!(UserId, uuid_v7);  // Sortable
id_type!(TripId, uuid_v4);  // Random
```

### Create a Domain Entity
```rust
use stano_common::{DomainError, id_type};

id_type!(AccountId, uuid_v7);

pub struct Account {
    account_id: AccountId,
    email: String,
    // ... private fields
}

impl Account {
    pub fn new(email: String) -> Result<Self, DomainError> {
        if email.is_empty() {
            return Err(DomainError::InvalidInput("email required".into()));
        }
        Ok(Self {
            account_id: AccountId::new(),
            email,
        })
    }
}
```

### Define a Service
```rust
use stano_di_macros::{component, service};
use stano_common::ServiceError;
use std::sync::Arc;

#[component]
#[async_trait::async_trait]
pub trait UserService: Send + Sync {
    async fn get_user(&self, id: &UserId) -> Result<UserDto, ServiceError>;
}

#[service(dyn UserService)]
pub struct UserServiceImpl {
    user_repo: Arc<dyn UserRepository>,
}

#[async_trait::async_trait]
impl UserService for UserServiceImpl {
    async fn get_user(&self, id: &UserId) -> Result<UserDto, ServiceError> {
        self.user_repo
            .find(id)
            .await
            .map_err(|e| ServiceError::Internal(e))?
            .map(|u| UserDto::from(u))
            .ok_or(ServiceError::NotFound)
    }
}
```

### Create an HTTP Handler
```rust
use stano_axum::{AppJson, AppPath};
use stano_security::SecurityContext;

async fn get_user(
    ctx: SecurityContext,  // Auto-extracted if JWT valid, else 401
    AppPath(user_id): AppPath<UserId>,
    State(s): State<AppState>,
) -> Result<AppJson<UserDto>, ApiError> {
    let service = s.application_context.get::<dyn UserService>();
    Ok(AppJson(service.get_user(&user_id).await?))
}
```

## Error Flow

```
DomainError (InvalidInput, BusinessRuleViolation)
    ↓
(converted by services layer)
    ↓
ServiceError (NotFound, InvalidInput, Conflict, Unauthorized, Forbidden, Internal)
    ↓
(via From impl in rest_api)
    ↓
ApiError → HTTP response (404, 400, 409, 401, 403, 500)
```

## Database Patterns

### SeaORM Entity
```rust
// Generated via sea-orm-cli
#[derive(Clone, Debug, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub email: String,
    pub created_at: joda_rs::ZonedDateTime,
}
```

### Mapper
```rust
use stano_seaorm::Mapper;

impl Mapper<Account> for AccountMapper {
    type Model = user::Model;
    type ActiveModel = user::ActiveModel;

    fn to_domain(model: user::Model) -> Account {
        Account {
            account_id: AccountId::from(model.id),
            email: model.email,
        }
    }

    fn to_active_model(domain: &Account) -> user::ActiveModel {
        user::ActiveModel {
            id: Set(*domain.account_id.as_uuid()),
            email: Set(domain.email.clone()),
        }
    }
}
```

## Quality Checks

```bash
cargo build
cargo test --workspace
cargo clippy -- -D warnings
cargo fmt --check
```

All crates are:
- ✅ Zero warnings under Clippy
- ✅ Fully formatted with `rustfmt`
- ✅ Test-covered
- ✅ Properly documented

## Middleware Stack

`stano-launcher` applies this stack in request-processing order (outermost/first-to-see-request → innermost/closest-to-handlers):

1. **Security Headers** — `x-content-type-options: nosniff`, `x-frame-options: DENY`, HSTS
2. **RequestBodyLimit** — 10 MB max
3. **PropagateRequestId** — propagates upstream
4. **SetRequestId** — injects `x-request-id`
5. **Compression** — gzip/brotli/deflate auto-negotiated
6. **CatchPanic** — panics become 500 responses
7. **error_logging_middleware** — logs `ApiError` with request context
8. **TraceLayer** — structured logging
9. **Timeout** — 300s per request
10. **CORS** — configurable origins or permissive

---

## Dependencies

| Crate | Purpose | Key Dependencies |
|-------|---------|------------------|
| `stano-launcher` | App bootstrap | `stano-di`, `stano-axum`, `stano-security`, `axum`, `tokio`, `tower-http` |
| `stano-di` | DI container | `dotenvy`, `thiserror` |
| `stano-di-macros` | Macros | `proc-macro`, `quote`, `syn` |
| `stano-common` | Shared types | `uuid`, `serde`, `thiserror`, `anyhow` |
| `stano-security` | JWT/Auth | `jsonwebtoken`, `tokio`, `serde` |
| `stano-axum` | HTTP | `axum`, `serde`, `tokio` |
| `stano-seaorm` | Database | `sea-orm`, `tokio` |

## Project Structure for Apps

Recommended structure for apps consuming these crates:

```
my-app/
  src/
    main.rs                 # App entry point, calls stano_launcher::run()
    domain/                 # Pure business logic (no external deps)
      mod.rs
      user.rs             # Entity definitions
    infrastructure/         # Database adapters
      mod.rs
      user_repo.rs        # SeaORM repositories + Mapper impls
    services/              # Business orchestration
      mod.rs
      user_service.rs     # Service traits & impls, DTOs
    http/                  # HTTP handlers & routes
      mod.rs
      user_routes.rs      # Route definitions
  Cargo.toml               # Depends on stano-* crates
  .env                     # JWT keys, database URL, etc.
```

**Flow:** HTTP handler → calls service → calls repository → maps domain entities ↔ database models.

---

## Layering Rules

| Layer | Purpose | Error Type | External Deps Allowed |
|---|---|---|---|
| Domain | Pure business logic | `DomainError` | None |
| Infrastructure | DB adapters, external services | `anyhow::Error` | SeaORM, HTTP clients |
| Services | Orchestration, guards | `ServiceError` | Domain, Infra, macros |
| HTTP | Request/response mapping | `ApiError` | Services, stano-axum, stano-launcher |
| App (main.rs) | Bootstrap & wiring | `anyhow::Error` | All of the above |

Domain has zero external dependencies — it's pure Rust. Everything else builds on it.

---

## Error Chain

```
DomainError (InvalidInput, BusinessRuleViolation)
    ↓ (domain_err_to_service())
ServiceError (NotFound, InvalidInput, Conflict, Unauthorized, Forbidden, Internal)
    ↓ (impl IntoResponse)
ApiError
    ↓
HTTP status (400, 401, 403, 404, 409, 500)
```

---

## License

These crates are part of the Stano platform.
