# stano-common

Shared types and utilities for the Stano platform: typed error types for domain and service layers, and a macro for generating type-safe UUID wrappers.

## Install

```toml
[dependencies]
stano-common = { path = "../stano-common" }
```

## API

### Error Types

- **`DomainError`** — business logic errors (pure domain layer, no external context).
  - `InvalidInput(String)` — validation failure.
  - `BusinessRuleViolation(String)` — operation violates a business rule.
  
- **`ServiceError`** — orchestration layer errors (services catch `DomainError`, convert to `ServiceError` via scope).
  - `NotFound` — resource missing.
  - `InvalidInput(String)` — bad request data.
  - `Conflict(String)` — operation conflicts with system state.
  - `Unauthorized` — authentication required.
  - `Forbidden` — operation not allowed for this user/role.
  - `Internal(anyhow::Error)` — unhandled infrastructure failure.

- **`domain_err_to_service(err: DomainError) -> ServiceError`** — converts `DomainError::InvalidInput` → `ServiceError::InvalidInput`, `BusinessRuleViolation` → `ServiceError::Conflict`.

### ID Type Macro

- **`id_type!(Name, uuid_v4 | uuid_v7)`** — generates a newtype struct `pub struct Name(Uuid)` wrapping a UUID.
  - `v4` variant (random): derives `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize`.
  - `v7` variant (sortable/time-ordered): derives the above plus `PartialOrd, Ord`, enabling collections like `BTreeSet<Name>`.
  - Both generate inherent methods: `new()` (generates a new UUID), `from(uuid: Uuid)`, `as_uuid(&self) -> &Uuid`.
  - Both impl `FromStr` (parse via `Uuid::parse_str`) and `Display`.

### Re-exports

- `uuid` crate — for raw UUID operations if needed.

## Usage Example

```rust
use stano_common::{id_type, DomainError, ServiceError, domain_err_to_service};

// Define typed IDs for your domain.
id_type!(UserId, uuid_v7);  // Sortable for database queries.
id_type!(SessionId, uuid_v4); // Random for tokens.

// Domain logic with typed errors.
pub fn create_user(email: String) -> Result<UserId, DomainError> {
    if email.is_empty() {
        return Err(DomainError::InvalidInput("email required".into()));
    }
    Ok(UserId::new())
}

// Service layer converts errors.
pub async fn create_user_service(email: String) -> Result<UserId, ServiceError> {
    create_user(email).map_err(domain_err_to_service)
}
```

## Notes

- **No feature flags** — all APIs are always available.
- UUID v7 (sortable) is recommended for primary entity IDs (faster indexing); v4 (random) for tokens/nonces.
- The `id_type!` macro is re-exported by convenience crates like `stano-starter` if your app depends on them.
