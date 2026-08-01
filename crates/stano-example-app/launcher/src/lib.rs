//! Composition root for the `stano-example-app` demo: wires DI (`#[component]`/`#[service]`
//! auto-registration) and exposes `build_context()` for `main.rs` and the integration
//! tests. See the workspace `AGENTS.md` for how the platform's pieces compose.
//!
//! Not published — exists only to be run (`cargo run -p stano-example-app`) and tested
//! (`cargo test -p stano-example-app`) as a smoke check that the platform crates wire up
//! correctly end-to-end.

use std::sync::Arc;

use stano_di::application_context::ApplicationContext;
use stano_di::environment::OsEnvironment;
use stano_example_infrastructure::WidgetStore;

/// Builds the app's `ApplicationContext`: registers the `WidgetStore` instance the
/// `#[service]`-generated `InMemoryWidgetRepository` factory depends on, then picks up
/// every `#[service]`-registered component (in `stano-example-infrastructure` and
/// `stano-example-services`) via `register_all()`.
pub fn build_context() -> Arc<ApplicationContext> {
    let mut ctx = ApplicationContext::new(Arc::new(OsEnvironment::new()));
    ctx.register_instance(Arc::new(WidgetStore::default()));
    ctx.register_all();
    ctx.validate()
        .expect("all registered components must resolve");
    Arc::new(ctx)
}
