//! In-memory `WidgetRepository` adapter for `stano-example-app` — stands in for a real
//! database-backed adapter (e.g. `stano-seaorm`), demonstrating the port/adapter seam.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use stano_common::ServiceError;
use stano_di_macros::service;
use stano_example_domain::{Widget, WidgetId, WidgetRepository};

/// In-memory backing store for [`InMemoryWidgetRepository`]. Registered as a plain instance
/// (via `register_instance`) so `#[service]`'s field-injection can resolve it as a
/// dependency, the same way a real app would inject a DB pool or HTTP client.
///
/// `Container::get::<T>()` requires `T: Clone` (it clones the `Arc<T>` handle, but the
/// bound is stated on `T` itself), so this wraps the shared state in its own inner `Arc`
/// and derives `Clone` rather than exposing a bare `Mutex<HashMap<..>>` as the field type.
#[derive(Clone, Default)]
pub struct WidgetStore(Arc<Mutex<HashMap<WidgetId, Widget>>>);

impl WidgetStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<WidgetId, Widget>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[service(dyn WidgetRepository)]
pub struct InMemoryWidgetRepository {
    store: Arc<WidgetStore>,
}

impl WidgetRepository for InMemoryWidgetRepository {
    fn create(&self, name: String) -> Widget {
        let widget = Widget {
            id: WidgetId::new(),
            name,
        };
        self.store.lock().insert(widget.id, widget.clone());
        widget
    }

    fn get(&self, id: WidgetId) -> Result<Widget, ServiceError> {
        self.store
            .lock()
            .get(&id)
            .cloned()
            .ok_or(ServiceError::NotFound)
    }

    fn list(&self) -> Vec<Widget> {
        self.store.lock().values().cloned().collect()
    }
}
