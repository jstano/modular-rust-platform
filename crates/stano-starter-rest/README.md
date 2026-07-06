# stano-starter-rest

Convenience re-export bundle for an app's HTTP layer: aggregates DI, Axum HTTP utilities, server launcher, and security context.

## Install

```toml
[dependencies]
stano-starter-rest = { path = "../stano-starter-rest" }
```

## What It Re-exports

- `stano-di` — the DI container and application context
- `stano-axum` — HTTP extractors (`AppJson`, `AppPath`, `AppQuery`), error types, middleware
- `stano-launcher` — server bootstrap (`run`, `RouteGroups`, `BootstrapConfig`)
- `stano-security` — JWT, claims, security context (for use in handlers)

## Why

For apps using the starter crate organization, declare `stano-starter-rest` as your HTTP/REST layer's dependency:

```toml
[dependencies]
stano-starter-rest = { path = "../stano-starter-rest" }
```

This bundles the HTTP layer (extractors, error handling, server wiring) along with security primitives, so you can build request handlers that integrate with the platform's error flow and auth system.

## Notes

This crate contains **no code of its own** — it is a pure re-export facade. For full documentation of the APIs it provides, see:

- [stano-di](../stano-di) — DI container, `ApplicationContext`
- [stano-axum](../stano-axum) — HTTP extractors, `ApiError`, middleware
- [stano-launcher](../stano-launcher) — server bootstrap, `RouteGroups`, middleware stack
- [stano-security](../stano-security) — JWT, `SecurityContext`
