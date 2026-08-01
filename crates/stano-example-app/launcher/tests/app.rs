use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use stano_example_app::build_context;
use stano_example_rest_api::build_authorization;
use stano_example_security::{demo_jwt_config, issue_token};
use stano_launcher::routes::collect_routes;
use std::convert::Infallible;
use tower::Layer;
use tower::util::{BoxCloneSyncService, ServiceExt};

type App = BoxCloneSyncService<Request<Body>, Response, Infallible>;

fn app() -> App {
    let ctx = build_context();
    let authorization = build_authorization(demo_jwt_config());
    let (router, _) = collect_routes().split_for_parts();
    authorization.layer(router.with_state(ctx))
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

#[tokio::test]
async fn health_is_public_and_requires_no_token() {
    let app = app();
    let (status, _) = request(&app, "GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn protected_route_without_token_is_unauthorized() {
    let app = app();
    let (status, _) = request(&app, "GET", "/widgets/does-not-matter", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_then_get_widget_round_trips_through_di_resolved_service() {
    let app = app();
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
}

#[tokio::test]
async fn get_missing_widget_maps_service_error_not_found_to_404() {
    let app = app();
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
}

#[tokio::test]
async fn admin_route_rejects_non_admin_role() {
    let app = app();
    let token = issue_token("USER", &demo_jwt_config());

    let (status, _) = request(&app, "GET", "/admin/widgets", Some(&token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_route_allows_admin_role_and_extracts_security_context() {
    let app = app();
    let token = issue_token("ADMIN", &demo_jwt_config());

    let (status, body) = request(&app, "GET", "/admin/widgets", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
}

#[tokio::test]
async fn malformed_json_body_maps_to_bad_request() {
    let app = app();
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
}
