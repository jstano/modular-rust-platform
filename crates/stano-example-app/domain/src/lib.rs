//! Domain layer for the `stano-example-app` demo: the `Widget` entity and the
//! `WidgetRepository` port that `stano-example-persistence` implements.

use stano_common::ServiceError;
use stano_di_macros::component;

stano_common::id_type!(WidgetId, uuid_v7);

#[derive(Debug, Clone)]
pub struct Widget {
    pub id: WidgetId,
    pub name: String,
}

/// Injectable via `#[component]`; resolved as `Arc<dyn WidgetRepository>`. Implemented by
/// `stano-example-persistence`'s SeaORM-backed adapter.
#[async_trait::async_trait]
#[component]
pub trait WidgetRepository: Send + Sync {
    async fn create(&self, name: String) -> Result<Widget, ServiceError>;
    async fn get(&self, id: WidgetId) -> Result<Widget, ServiceError>;
    async fn list(&self) -> Result<Vec<Widget>, ServiceError>;
    async fn update(&self, widget: &Widget) -> Result<Widget, ServiceError>;
}
