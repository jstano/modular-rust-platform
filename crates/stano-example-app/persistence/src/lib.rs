//! SeaORM-backed `WidgetRepository` adapter for `stano-example-app` — demonstrates the
//! port/adapter seam with a real Postgres-backed implementation via `stano-seaorm`.

mod entity;
mod mapper;

use std::sync::Arc;

use sea_orm::{ActiveModelTrait, DbErr, EntityTrait};
use stano_common::ServiceError;
use stano_di::environment::Environment;
use stano_di_macros::service;
use stano_example_domain::{Widget, WidgetId, WidgetRepository};
use stano_seaorm::{DbConfig, Mapper, query_tracing_config_from_env};

pub use mapper::WidgetMapper;

/// Opens the database connection pool, reading query-tracing config from `environment`.
/// Called once from the launcher's `build_context()`, before DI registration, since
/// `ApplicationContext`'s component factories are synchronous and can't perform this
/// `await` themselves.
///
/// Returns `anyhow::Result` rather than `stano_seaorm::DbConfigError` because that type,
/// despite being `pub`, lives behind a private module and isn't re-exported from
/// `stano-seaorm`'s crate root — it can't be named outside that crate today.
pub async fn connect(
    database_url: &str,
    environment: &dyn Environment,
) -> anyhow::Result<DbConfig> {
    Ok(DbConfig::from_url(database_url, query_tracing_config_from_env(environment)).await?)
}

#[service(dyn WidgetRepository)]
pub struct SeaOrmWidgetRepository {
    db: Arc<DbConfig>,
}

#[async_trait::async_trait]
impl WidgetRepository for SeaOrmWidgetRepository {
    async fn create(&self, name: String) -> Result<Widget, ServiceError> {
        let widget = Widget {
            id: WidgetId::new(),
            name,
        };
        let active = WidgetMapper::to_active_model(&widget);
        let model = active
            .insert(self.db.connection())
            .await
            .map_err(anyhow::Error::from)?;
        Ok(WidgetMapper::to_domain(model))
    }

    async fn get(&self, id: WidgetId) -> Result<Widget, ServiceError> {
        entity::Entity::find_by_id(*id.as_uuid())
            .one(self.db.connection())
            .await
            .map_err(anyhow::Error::from)?
            .map(WidgetMapper::to_domain)
            .ok_or(ServiceError::NotFound)
    }

    async fn list(&self) -> Result<Vec<Widget>, ServiceError> {
        let models = entity::Entity::find()
            .all(self.db.connection())
            .await
            .map_err(anyhow::Error::from)?;
        Ok(models.into_iter().map(WidgetMapper::to_domain).collect())
    }

    async fn update(&self, widget: &Widget) -> Result<Widget, ServiceError> {
        let active = WidgetMapper::to_active_model(widget);
        let model = active
            .update(self.db.connection())
            .await
            .map_err(|err| match err {
                DbErr::RecordNotUpdated => ServiceError::NotFound,
                other => ServiceError::from(anyhow::Error::from(other)),
            })?;
        Ok(WidgetMapper::to_domain(model))
    }
}
