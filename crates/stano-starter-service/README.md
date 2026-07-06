# stano-starter-service

Convenience re-export bundle for an app's service layer: aggregates `stano-starter` (domain/DI foundation) plus `stano-security` (JWT and security context).

## Install

```toml
[dependencies]
stano-starter-service = { path = "../stano-starter-service" }
```

## What It Re-exports

- Everything from `stano-starter`:
  - `stano-common` — error types, typed IDs, service error types
  - `stano-di` — the DI container
  - `stano-di-macros` — `#[component]`, `#[service]` macros
- `stano-security` — JWT encode/decode, claims, security context

## Why

For apps using the starter crate organization, declare `stano-starter-service` as your service layer's dependency:

```toml
[dependencies]
stano-starter-service = { path = "../stano-starter-service" }
```

This bundles the foundation (from domain layer) plus JWT/auth primitives, so your service code can orchestrate business logic and access request identity via `SecurityContext<E>`.

## Notes

This crate contains **no code of its own** — it is a pure re-export facade. For full documentation of the APIs it provides, see:

- [stano-starter](../stano-starter) — domain/DI foundation
- [stano-common](../stano-common) — error types
- [stano-security](../stano-security) — JWT, claims, security context
