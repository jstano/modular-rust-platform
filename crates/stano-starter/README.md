# stano-starter

Convenience re-export bundle for domain-layer development: aggregates the foundational platform crates needed to build pure business logic without external dependencies.

## Install

```toml
[dependencies]
stano-starter = { path = "../stano-starter" }
```

## What It Re-exports

- `stano-common` — error types and the `id_type!` macro for domain entities.
- `stano-di` — the DI container (rarely needed in pure domain code, but available).
- `stano-di-macros` — `#[component]` and `#[service]` macros.

## Why

Instead of listing three path dependencies in your domain crate's `Cargo.toml`, declare just one:

```toml
[dependencies]
stano-starter = { path = "../stano-starter" }
```

Then use as:
```rust
use stano_starter::{id_type, DomainError, Container};
```

## Notes

This crate contains **no code of its own** — it is a pure re-export facade. Its public API is exactly the union of the APIs from `stano-common`, `stano-di`, and `stano-di-macros`. For full documentation, see those crates:

- [stano-common](../stano-common) — error types, typed IDs
- [stano-di](../stano-di) — DI container
- [stano-di-macros](../stano-di-macros) — `#[component]`, `#[service]` macros
