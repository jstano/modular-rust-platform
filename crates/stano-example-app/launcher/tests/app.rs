use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use stano_example_app::build_context;
use stano_example_rest_api::build_authorization;
use stano_example_security::{demo_jwt_config, issue_token};
use stano_launcher::routes::collect_routes;
use std::convert::Infallible;
use std::sync::OnceLock;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner};
use tower::Layer;
use tower::util::{BoxCloneSyncService, ServiceExt};

type App = BoxCloneSyncService<Request<Body>, Response, Infallible>;

/// A real SeaORM connection pool ties its background maintenance tasks to whichever
/// Tokio runtime is active when it's created. `#[tokio::test]` gives each test function
/// its own short-lived runtime, so a pool built inside test A's runtime hangs (until the
/// pool's 30s acquire timeout) once test A's runtime is dropped and test B tries to reuse
/// it. Run every test as a plain `#[test]` against one runtime shared for the whole
/// binary instead, so the pool and every query against it live in the same runtime.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime starts"))
}

/// Serializes the `env::set_var` + `build_context()` window across tests. Each test gets
/// its own Postgres container (rather than one shared via a `static`), so its guard is a
/// genuine local variable that drops — and cleans the container up — at the end of that
/// test, with no dependency on the Ryuk reaper sidecar. But `build_context()` reads the
/// database URL from a process-global env var, and tests run in parallel by default, so
/// without this lock two tests' `set_var` calls could race.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Starts a throwaway Postgres container — no `docker compose up` needed first. Kept
/// alive by the caller for as long as `App` is used; dropping it stops the container.
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

async fn app() -> (ContainerAsync<Postgres>, App) {
    let (container, database_url) = start_test_database().await;
    let ctx = {
        let _guard = ENV_LOCK.lock().await;
        // SAFETY: serialized by `ENV_LOCK` — no other test's `set_var` call runs
        // concurrently with this one, and nothing reads the env var again after
        // `build_context()` returns (the URL is already captured into the pool).
        unsafe { std::env::set_var("DATABASE_URL", &database_url) };
        build_context()
            .await
            .expect("build_context succeeds against the testcontainers Postgres")
    };
    let authorization = build_authorization(demo_jwt_config());
    let (router, _) = collect_routes().split_for_parts();
    (container, authorization.layer(router.with_state(ctx)))
}

async fn request(
    app: &App,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(json_body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json_body.to_string())
        }
        None => Body::empty(),
    };

    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json_body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json_body)
}

#[test]
fn health_is_public_and_requires_no_token() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let (status, _) = request(&app, "GET", "/health", None, None).await;
        assert_eq!(status, StatusCode::OK);
    });
}

#[test]
fn protected_route_without_token_is_unauthorized() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let (status, _) = request(&app, "GET", "/widgets/does-not-matter", None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    });
}

#[test]
fn create_then_get_widget_round_trips_through_di_resolved_service() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());

        let (create_status, created) = request(
            &app,
            "POST",
            "/widgets",
            Some(&token),
            Some(json!({ "name": "sprocket" })),
        )
        .await;
        assert_eq!(create_status, StatusCode::OK);
        assert_eq!(created["name"], "sprocket");
        let id = created["id"].as_str().unwrap().to_string();

        let (get_status, fetched) =
            request(&app, "GET", &format!("/widgets/{id}"), Some(&token), None).await;
        assert_eq!(get_status, StatusCode::OK);
        assert_eq!(fetched["id"], id);
        assert_eq!(fetched["name"], "sprocket");
    });
}

#[test]
fn update_then_get_widget_round_trips_through_di_resolved_service() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());

        let (_, created) = request(
            &app,
            "POST",
            "/widgets",
            Some(&token),
            Some(json!({ "name": "sprocket" })),
        )
        .await;
        let id = created["id"].as_str().unwrap().to_string();

        let (update_status, updated) = request(
            &app,
            "PUT",
            &format!("/widgets/{id}"),
            Some(&token),
            Some(json!({ "name": "gizmo" })),
        )
        .await;
        assert_eq!(update_status, StatusCode::OK);
        assert_eq!(updated["id"], id);
        assert_eq!(updated["name"], "gizmo");

        let (get_status, fetched) =
            request(&app, "GET", &format!("/widgets/{id}"), Some(&token), None).await;
        assert_eq!(get_status, StatusCode::OK);
        assert_eq!(fetched["name"], "gizmo");
    });
}

#[test]
fn update_missing_widget_maps_service_error_not_found_to_404() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());
        let missing_id = "01960b3a-0000-7000-8000-000000000001";

        let (status, body) = request(
            &app,
            "PUT",
            &format!("/widgets/{missing_id}"),
            Some(&token),
            Some(json!({ "name": "gizmo" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "NOT_FOUND");
    });
}

#[test]
fn get_missing_widget_maps_service_error_not_found_to_404() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());
        let missing_id = "01960b3a-0000-7000-8000-000000000000";

        let (status, body) = request(
            &app,
            "GET",
            &format!("/widgets/{missing_id}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "NOT_FOUND");
    });
}

#[test]
fn admin_route_rejects_non_admin_role() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());

        let (status, _) = request(&app, "GET", "/admin/widgets", Some(&token), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    });
}

#[test]
fn admin_route_allows_admin_role_and_extracts_security_context() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("ADMIN", &demo_jwt_config());

        let (status, body) = request(&app, "GET", "/admin/widgets", Some(&token), None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_array());
    });
}

#[test]
fn malformed_json_body_maps_to_bad_request() {
    runtime().block_on(async {
        let (_container, app) = app().await;
        let token = issue_token("USER", &demo_jwt_config());

        let request = Request::builder()
            .method("POST")
            .uri("/widgets")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{not valid json"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    });
}
