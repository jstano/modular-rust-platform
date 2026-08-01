//! Domain layer for the `stano-example-app` demo: the `Widget` entity and the
//! `WidgetRepository` port that `stano-example-infrastructure` implements.

use stano_common::ServiceError;
use stano_di_macros::component;

stano_common::id_type!(WidgetId, uuid_v7);

#[derive(Debug, Clone)]
pub struct Widget {
    pub id: WidgetId,
    pub name: String,
}

/// Injectable via `#[component]`; resolved as `Arc<dyn WidgetRepository>`. Implemented by
/// `stano-example-infrastructure`'s in-memory adapter.
#[component]
pub trait WidgetRepository: Send + Sync {
    fn create(&self, name: String) -> Widget;
    fn get(&self, id: WidgetId) -> Result<Widget, ServiceError>;
    fn list(&self) -> Vec<Widget>;
}
