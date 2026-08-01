//! Auto-registration for routes annotated with `#[get]`/`#[post]`/`#[put]`/`#[delete]`/
//! `#[patch]` (from `stano-route-macros`). Each annotated handler submits a
//! [`RouteRegistration`] via
//! `inventory::submit!` at compile time; [`collect_routes`] folds all of them into a single
//! [`OpenApiRouter`] at startup, so `stano_launcher::run` never needs a hand-maintained
//! list of `.routes(routes!(handler))` calls.

use stano_di::application_context::ApplicationContext;
use std::sync::Arc;
use utoipa_axum::router::OpenApiRouter;

#[doc(hidden)]
pub use inventory;
#[doc(hidden)]
pub use utoipa_axum;

/// A route registration collected via `#[get]`/`#[post]`/etc.'s `inventory::submit!`, applied by
/// [`collect_routes`]. Wraps a factory that builds the handler's `utoipa_axum::routes!`
/// tuple (schemas, OpenAPI paths, and the `MethodRouter`). Per-request authorization is
/// applied globally via `stano_axum::security::AuthorizationLayer` (see
/// `stano_launcher::run`), not per-route here.
pub struct RouteRegistration(
    pub fn() -> utoipa_axum::router::UtoipaMethodRouter<Arc<ApplicationContext>>,
);
inventory::collect!(RouteRegistration);

/// Folds every [`RouteRegistration`] collected via `#[get]`/`#[post]`/etc.'s
/// `inventory::submit!` into a single [`OpenApiRouter`].
pub fn collect_routes() -> OpenApiRouter<Arc<ApplicationContext>> {
    let mut router = OpenApiRouter::new();
    for reg in inventory::iter::<RouteRegistration>() {
        router = router.routes((reg.0)());
    }
    router
}
