# stano-starter-domain

Convenience re-export bundle for an app's domain layer: transitively re-exports `stano-starter` (common, DI, DI macros) under a domain-focused name.

## Install

```toml
[dependencies]
stano-starter-domain = { path = "../stano-starter-domain" }
```

## What It Re-exports

Transitively re-exports everything from `stano-starter`:
- `stano-common` — error types and the `id_type!` macro for domain entities.
- `stano-di` — the DI container.
- `stano-di-macros` — `#[component]` and `#[service]` macros.

## Why

For apps using the starter crate organization, declare `stano-starter-domain` as your domain layer's dependency:

```toml
[dependencies]
stano-starter-domain = { path = "../stano-starter-domain" }
```

This mirrors the layered architecture and makes dependency intent clear:
- Domain layer → `stano-starter-domain`
- Service layer → `stano-starter-service`
- HTTP layer → `stano-starter-rest`

## Notes

This crate contains **no code of its own** — it is a pure re-export facade. For full documentation of the APIs it provides, see:

- [stano-starter](../stano-starter) (immediate re-export)
- [stano-common](../stano-common) — error types, typed IDs
- [stano-di](../stano-di) — DI container
- [stano-di-macros](../stano-di-macros) — `#[component]`, `#[service]` macros
