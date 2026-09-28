//! Exercises `SeaOrmWidgetRepository::update`'s `DbErr::RecordNotUpdated` -> `NotFound`
//! mapping directly. `update_widget_handler` (in `stano-example-rest-api`) happens to hit
//! this same path for a missing id since it doesn't fetch first, but that's a detail of
//! that particular handler — this test pins the repository's own behavior regardless of
//! which caller reaches it.
//!
//! Uses `testcontainers` to start a throwaway Postgres for the duration of this test —
//! no `docker compose up` needed first.

use std::sync::Arc;

use migration::MigratorTrait;
use stano_di::environment::OsEnvironment;
use stano_example_domain::{Widget, WidgetId, WidgetRepository};
use stano_example_persistence::{SeaOrmWidgetRepository, connect};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};

async fn start_test_database() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("17-alpine") // match docker-compose.yml's production image
        .start()
        .await
        .expect("postgres container starts");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    (
        container,
        format!("postgres://postgres:postgres@{host}:{port}/postgres"),
    )
}

#[tokio::test]
async fn update_missing_widget_maps_to_not_found() {
    let (_container, database_url) = start_test_database().await;
    let env = OsEnvironment::new();

    let db = connect(&database_url, &env)
        .await
        .expect("connect succeeds against the testcontainers Postgres");
    migration::Migrator::up(db.connection(), None)
        .await
        .expect("migrations apply");

    let repo = SeaOrmWidgetRepository::new(Arc::new(db));
    let missing = Widget {
        id: WidgetId::new(),
        name: "ghost".to_string(),
    };

    let err = repo
        .update(&missing)
        .await
        .expect_err("updating a widget that was never created must fail");
    assert!(matches!(err, stano_common::ServiceError::NotFound));
}
