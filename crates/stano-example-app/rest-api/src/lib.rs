//! REST adapter layer for `stano-example-app`: HTTP handlers (`#[get]`/`#[post]`,
//! `AppJson`, `AppSecurityContext`, `ServiceError` -> `ApiError` mapping) and the
//! authorization chain that protects them. Depends on `stano-starter-rest` (routing,
//! extractors, DI facade) but deliberately **not** `stano-launcher` — this crate is the
//! adapter, not the composition root that assembles and runs the server.

use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use stano_common::ServiceError;
use stano_example_domain::{Widget, WidgetId};
use stano_example_security::AppClaims;
use stano_example_services::WidgetService;
use stano_starter_rest::application_context::ApplicationContext;
use stano_starter_rest::security::{AuthorizationBuilder, AuthorizationLayer};
use stano_starter_rest::{ApiError, AppJson, AppPath, AppSecurityContext, JwtConfig, get, post};

// ---------------------------------------------------------------------------------------
// Security wiring (exercises `stano_axum::security::AuthorizationBuilder`)
// ---------------------------------------------------------------------------------------

/// `/health` and the Swagger UI/OpenAPI doc (dev-only tooling, not app data) are public;
/// `/admin/*` requires the `ADMIN` role; everything else just requires a valid JWT.
pub fn build_authorization(jwt_config: JwtConfig) -> AuthorizationLayer {
    AuthorizationBuilder::<AppClaims>::new()
        .request_matcher("/health")
        .permit_all()
        .request_matcher("/swagger")
        .permit_all()
        .request_matcher("/swagger/{*rest}")
        .permit_all()
        .request_matcher("/api-docs/{*rest}")
        .permit_all()
        .request_matcher("/admin/{*rest}")
        .has_role(|c: &AppClaims| c.role == "ADMIN")
        .any_request()
        .authenticated()
        .build(jwt_config)
        .expect("authorization chain is valid")
}

// ---------------------------------------------------------------------------------------
// Routes (exercises `#[get]`/`#[post]`, `AppJson`, `AppSecurityContext`, and `ServiceError` -> `ApiError`)
// ---------------------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct WidgetResponse {
    pub id: String,
    pub name: String,
}

impl From<Widget> for WidgetResponse {
    fn from(widget: Widget) -> Self {
        Self {
            id: widget.id.to_string(),
            name: widget.name,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWidgetRequest {
    pub name: String,
}

#[get(path = "/health", responses((status = 200, body = String)))]
pub async fn health_handler() -> &'static str {
    "ok"
}

#[post(path = "/widgets", tag = "widgets", security(("bearerAuth" = [])))]
pub async fn create_widget_handler(
    axum::extract::State(ctx): axum::extract::State<Arc<ApplicationContext>>,
    AppJson(req): AppJson<CreateWidgetRequest>,
) -> AppJson<WidgetResponse> {
    let service = ctx.get_trait::<dyn WidgetService>();
    AppJson(service.create(req.name).into())
}

#[get(
    path = "/widgets/{id}",
    tag = "widgets",
    responses((status = 404, body = String)),
    security(("bearerAuth" = [])),
)]
pub async fn get_widget_handler(
    axum::extract::State(ctx): axum::extract::State<Arc<ApplicationContext>>,
    AppPath(id): AppPath<String>,
) -> Result<AppJson<WidgetResponse>, ApiError> {
    let widget_id = WidgetId::from_str(&id)
        .map_err(|_| ApiError::from(ServiceError::InvalidInput("invalid widget id".to_string())))?;
    let service = ctx.get_trait::<dyn WidgetService>();
    let widget = service.get(widget_id)?;
    Ok(AppJson(widget.into()))
}

#[get(path = "/admin/widgets", tag = "admin", security(("bearerAuth" = [])))]
pub async fn list_widgets_admin_handler(
    axum::extract::State(ctx): axum::extract::State<Arc<ApplicationContext>>,
    AppSecurityContext(security_context): AppSecurityContext<AppClaims>,
) -> AppJson<Vec<WidgetResponse>> {
    tracing_free_log(&security_context);
    let service = ctx.get_trait::<dyn WidgetService>();
    AppJson(service.list().into_iter().map(Into::into).collect())
}

// Keeps the `AppSecurityContext` extraction demonstrably load-bearing (the caller's role
// was already enforced by `AuthorizationLayer`; this just proves the claims made it through).
fn tracing_free_log(security_context: &stano_starter_rest::SecurityContext<AppClaims>) {
    let _ = security_context.ext().role.as_str();
}
