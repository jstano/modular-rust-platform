//! Business layer for `stano-example-app`. Thin here (no extra rule beyond delegating to
//! the repository port) — its role is structural: the seam where real business rules
//! (validation, role checks) would live, matching a production service layer's shape.

use std::sync::Arc;

use stano_common::ServiceError;
use stano_di_macros::{component, service};
use stano_example_domain::{Widget, WidgetId, WidgetRepository};

/// Injectable via `#[component]`; resolved as `Arc<dyn WidgetService>`.
#[async_trait::async_trait]
#[component]
pub trait WidgetService: Send + Sync {
    async fn create(&self, name: String) -> Result<Widget, ServiceError>;
    async fn get(&self, id: WidgetId) -> Result<Widget, ServiceError>;
    async fn list(&self) -> Result<Vec<Widget>, ServiceError>;
    async fn update(&self, widget: &Widget) -> Result<Widget, ServiceError>;
}

#[service(dyn WidgetService)]
pub struct WidgetServiceImpl {
    repo: Arc<dyn WidgetRepository>,
}

#[async_trait::async_trait]
impl WidgetService for WidgetServiceImpl {
    async fn create(&self, name: String) -> Result<Widget, ServiceError> {
        self.repo.create(name).await
    }

    async fn get(&self, id: WidgetId) -> Result<Widget, ServiceError> {
        self.repo.get(id).await
    }

    async fn list(&self) -> Result<Vec<Widget>, ServiceError> {
        self.repo.list().await
    }

    async fn update(&self, widget: &Widget) -> Result<Widget, ServiceError> {
        self.repo.update(widget).await
    }
}
