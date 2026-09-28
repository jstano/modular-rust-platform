//! Composition root for the `stano-example-app` demo: wires DI (`#[component]`/`#[service]`
//! auto-registration) and exposes `build_context()` for `main.rs` and the integration
//! tests. See the workspace `AGENTS.md` for how the platform's pieces compose.
//!
//! Not published — exists only to be run (`cargo run -p stano-example-app`) and tested
//! (`cargo test -p stano-example-app`) as a smoke check that the platform crates wire up
//! correctly end-to-end.

use std::sync::Arc;

use migration::MigratorTrait;
use stano_di::application_context::ApplicationContext;
use stano_di::environment::{Environment, OsEnvironment};

/// Builds the app's `ApplicationContext`: connects to Postgres, runs pending migrations,
/// registers the `DbConfig` instance the `#[service]`-generated `SeaOrmWidgetRepository`
/// factory depends on, then picks up every `#[service]`-registered component (in
/// `stano-example-persistence` and `stano-example-services`) via `register_all()`.
pub async fn build_context() -> anyhow::Result<Arc<ApplicationContext>> {
    let env = OsEnvironment::new();
    let database_url = env
        .get("DATABASE_URL")
        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL not set"))?;

    let db = stano_example_persistence::connect(&database_url, &env).await?;
    migration::Migrator::up(db.connection(), None).await?;

    let mut ctx = ApplicationContext::new(Arc::new(env));
    ctx.register_instance(Arc::new(db));
    ctx.register_all();
    ctx.validate()
        .expect("all registered components must resolve");
    Ok(Arc::new(ctx))
}
