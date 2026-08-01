//! Business layer for `stano-example-app`. Thin here (no extra rule beyond delegating to
//! the repository port) — its role is structural: the seam where real business rules
//! (validation, role checks) would live, matching a production service layer's shape.

use std::sync::Arc;

use stano_common::ServiceError;
use stano_di_macros::{component, service};
use stano_example_domain::{Widget, WidgetId, WidgetRepository};

/// Injectable via `#[component]`; resolved as `Arc<dyn WidgetService>`.
#[component]
pub trait WidgetService: Send + Sync {
    fn create(&self, name: String) -> Widget;
    fn get(&self, id: WidgetId) -> Result<Widget, ServiceError>;
    fn list(&self) -> Vec<Widget>;
}

#[service(dyn WidgetService)]
pub struct WidgetServiceImpl {
    repo: Arc<dyn WidgetRepository>,
}

impl WidgetService for WidgetServiceImpl {
    fn create(&self, name: String) -> Widget {
        self.repo.create(name)
    }

    fn get(&self, id: WidgetId) -> Result<Widget, ServiceError> {
        self.repo.get(id)
    }

    fn list(&self) -> Vec<Widget> {
        self.repo.list()
    }
}
