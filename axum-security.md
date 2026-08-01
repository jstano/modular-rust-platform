# Path-based authorization config (Spring-Security-style) for the Stano platform

## Context
Today `stano-launcher`/`#[route]` has zero auth enforcement (confirmed by reading `stano-security`, `stano-axum`, `stano-launcher`). The user wants a central, declarative Spring-Security-style config — a list of path-pattern → requirement rules evaluated globally, e.g. Spring's `authorizeHttpRequests(auth -> auth.requestMatchers("/admin/**").hasRole("ADMIN")...)` — rather than per-handler guards. `stano-security` provides JWT primitives (`JwtConfig`, `Claims<E>`, `decode_jwt`) but has zero Axum coupling; `stano-axum` owns the extractor/`ApiError`/middleware conventions; `stano-launcher` owns the fixed middleware stack in `server::run()`. No path-pattern matcher exists anywhere in the repo today. Two dead stub files (`stano-launcher/src/auth.rs`, `authorization.rs`) currently say "implement per-app" and a stale doc comment on `RouteRegistration` (`stano-launcher/src/routes.rs:16-19`) references a removed, never-implemented `auth = <guard_fn>` mechanism.

Goal: add a global, opt-in, path-pattern-based authorization gate configured once at bootstrap, reusing existing conventions (`ApiError`/`ServiceError::Unauthorized`/`Forbidden` already map to 401/403 with `should_log: false`).

## Design

**Crate placement** (one new dependency edge: `stano-axum` → `stano-security`, confirmed absent today):
- `stano-security`: add a small `AuthorizationCheck` trait only (`fn has_role(&self, role: &str) -> bool`). Keeps this crate framework/Axum-agnostic — it's about the shape of claims, not request handling.
- `stano-axum`: owns everything Axum-facing — the path-pattern rule engine, JWT-decoding guard, and middleware. Add `stano-security` and `matchit` as direct dependencies (`matchit` is already transitive via `axum`, so this adds no new crate to the tree, just a direct edge/version pin). Using `matchit` (Axum's own router matcher) keeps one pattern syntax (`{id}`, `{*rest}`) across route declarations and security rules, instead of inventing a second Ant-glob (`**`) language. Blanket `.layer()` + independent `matchit::Router` lookup against the raw request path is used instead of `route_layer`/`MatchedPath`, because `route_layer` only fires for routes Axum's router actually matched — unmatched/404 paths would silently skip the guard, which breaks "deny by default."
- `stano-launcher`: wiring only — new optional `BootstrapConfig` field, and a conditional layer in `server::run()`.

**Builder API** (`stano-axum::authz::SecurityRules<E>`), first-match-wins by registration order:
```rust
let rules = SecurityRules::<MyClaimsExt>::new()
    .permit_all(["/health", "/swagger/{*rest}", "/api-docs/{*rest}"])
    .require_role(["/admin/{*rest}"], "ADMIN")
    .require_auth(["/api/{*rest}"])
    .default_deny();

let config = BootstrapConfig {
    // ...existing fields...
    security_rules: Some(Arc::new(rules)),
};
stano_launcher::run(ctx, extra_routes, config).await?;
```
`require_role` desugars to `require_custom(patterns, move |ext: &E| ext.has_role(&role))`, bounded on `E: AuthorizationCheck` only where needed — `permit_all`/`require_auth` stay usable even for apps with no role concept (e.g. `E = ()`).

**Type-erasure boundary**: `SecurityRules<E>` is generic over the app's claims-extension type, but `BootstrapConfig`/`run()` must stay non-generic (changing their signature would break every existing app). So `SecurityRules<E>` implements a non-generic `SecurityGuard` trait object (`fn check(&self, path, method, token, jwt_config) -> AuthzOutcome { Permitted | Unauthenticated | Forbidden }`), and `BootstrapConfig` gains `security_rules: Option<Arc<dyn SecurityGuard>>` — `None` by default, fully backward compatible, matches the existing `if http_logging_enabled { ... }` opt-in pattern already used for logging middleware.

**401 vs 403**: missing/malformed header or any `JwtError` (invalid signature, expired, malformed) → `Unauthenticated` → `ApiError::from(ServiceError::Unauthorized)` (401). Token decodes fine but `Requirement` check fails (e.g. wrong role) → `Forbidden` → `ApiError::from(ServiceError::Forbidden)` (403). Reuses the existing mapping — no new error variants, no bespoke JSON shape.

**Middleware placement in `server.rs`**: insert directly after `.layer(middleware::from_fn(error_logging_middleware))` (line 77) and before `.layer(CatchPanicLayer::custom(handle_panic))` (line 78), only when `config.security_rules.is_some()`. This keeps it inside `TraceLayer`/`error_logging_middleware` (so denied requests are still traced and logged like any other `ApiError`) but ahead of Compression/RequestId/panic-catch (generic infra that should still apply to 401/403 responses).

**Cleanup**: delete `stano-launcher/src/auth.rs` and `authorization.rs` (dead stubs, remove their `mod` decls from `lib.rs`); rewrite the stale doc comment on `RouteRegistration` in `routes.rs:16-19` to state plainly that `#[route(...)]` carries no per-route auth attribute — authorization is global via `BootstrapConfig::security_rules`, not per-handler. Do **not** extend `#[route(...)]`/`stano-route-macros` with a per-handler `auth = ...` attribute — that's the per-handler pattern the user wants to move away from.

## Files to add/change
| File | Change |
|---|---|
| `crates/stano-security/src/authorization_check.rs` | New: `AuthorizationCheck` trait |
| `crates/stano-security/src/lib.rs` | Re-export it |
| `crates/stano-axum/Cargo.toml` | Add `stano-security`, `matchit` deps |
| `crates/stano-axum/src/authz/rules.rs` | New: `SecurityRules<E>`, `Requirement<E>`, builder methods, `matchit`-based matching |
| `crates/stano-axum/src/authz/guard.rs` | New: `SecurityGuard` trait, `AuthzOutcome`, `impl SecurityGuard for SecurityRules<E>` (JWT decode happens here) |
| `crates/stano-axum/src/authz/middleware.rs` | New: `authorization_middleware` async fn (reads `Authorization` header, calls guard, maps outcome to `ApiError`) |
| `crates/stano-axum/src/lib.rs` | `pub mod authz;` + re-exports |
| `crates/stano-launcher/src/config.rs` | Add `security_rules: Option<Arc<dyn SecurityGuard>>` to `BootstrapConfig`; update `test_config` fixture in `server.rs` tests |
| `crates/stano-launcher/src/server.rs` | Conditional `.layer(...)` insertion per placement above |
| `crates/stano-launcher/src/lib.rs` | Remove `mod auth; mod authorization;` |
| `crates/stano-launcher/src/routes.rs` | Fix stale doc comment |
| `crates/stano-launcher/src/auth.rs`, `authorization.rs` | Delete |

Note: `BootstrapConfig` currently derives `Clone, Debug`; `Arc<dyn SecurityGuard>` needs `SecurityGuard: Debug` (or a manual `Debug` impl skipping this field) to keep that derive working — resolve during implementation.

## Test strategy
Follow existing colocated `#[cfg(test)] mod tests` convention (`server.rs`'s `preflight`/`with_security_headers` build-a-router-and-`oneshot` pattern):
- `rules.rs`: pattern matching (`{*rest}` wildcard, `{id}` param, no-match → default, first-match-wins, root `/`), `permit_all` bypasses decode entirely, `require_role` allow/deny via `AuthorizationCheck`, `require_auth` with valid token, `require_custom`.
- `guard.rs`: missing header, malformed header, expired token, wrong-key-signed token → all `Unauthenticated`; valid token + failed role check → `Forbidden`.
- `middleware.rs`: integration via `oneshot` — 401/403/pass-through cases, assert `ErrorResponse` JSON shape and `Arc<ApiError>` present in `response.extensions()` (mirrors existing `api_error_stored_in_response_extensions` test).
- `stano-launcher`: new `tests/authorization.rs` (or extend `route_macro.rs`) — end-to-end with `security_rules: Some(...)` against a `#[route]`-registered handler (401 no token, 403 wrong role, 200 correct role); regression-check that `security_rules: None` leaves existing `route_macro.rs` behavior unchanged.

## Sequencing
1. `stano-security`: `AuthorizationCheck` trait + test.
2. `stano-axum`: `authz::rules::SecurityRules<E>` + matching unit tests (no Axum `Request` yet).
3. `stano-axum`: `authz::guard::SecurityGuard`/`AuthzOutcome` + JWT-decode unit tests.
4. `stano-axum`: `authz::middleware::authorization_middleware` + integration tests.
5. `stano-axum/src/lib.rs`: wire `pub mod authz`, satisfy `#![warn(missing_docs)]`.
6. `stano-launcher`: `BootstrapConfig` field, test fixture update, conditional layer in `run()`.
7. `stano-launcher`: delete dead stub files, fix stale doc comment.
8. `stano-launcher`: end-to-end + opt-out regression tests.
9. `cargo test --workspace`, `cargo clippy`, `cargo fmt --check`; update `stano-launcher` README's documented middleware stack list.

## Verification
- `cargo test --workspace` — all new unit/integration tests plus full existing suite green.
- `cargo clippy` — zero warnings (repo requirement).
- Manually exercise: run a minimal app with `security_rules` configured, hit an admin-only route with no token (expect 401), wrong role (expect 403), correct role (expect 200); hit a `permit_all` route with no token (expect 200); confirm Swagger UI (if `permit_all`-listed) still loads.
