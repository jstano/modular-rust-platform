use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde::{Deserialize, Serialize};
use stano_axum::{AppJson, AppPath, AppQuery};
use stano_di::{application_context::ApplicationContext, environment::Environment};
use stano_launcher::{get, post, routes::collect_routes};
use std::sync::Arc;
use tower::util::ServiceExt;
use utoipa::{IntoParams, ToSchema};

struct TestEnvironment;

impl Environment for TestEnvironment {
    fn get(&self, _key: &str) -> Option<String> {
        None
    }
}

#[derive(Serialize, ToSchema)]
struct Pong {
    message: String,
}

// `AppJson<Pong>`'s return type is enough for `#[get]` to infer the 200/`Pong` response —
// no explicit `responses(...)` needed.
#[get(path = "/ping")]
async fn ping_handler() -> AppJson<Pong> {
    AppJson(Pong {
        message: "pong".to_string(),
    })
}

#[derive(Deserialize, ToSchema)]
struct EchoRequest {
    message: String,
}

// Exercises inference end to end: method from the macro name, `operation_id` from the fn
// name, `request_body` from the `AppJson<EchoRequest>` param, the 200 response from the
// `AppJson<Pong>` return type, and `params(("id" = String, Path))` from `AppPath<String>` +
// the single `{id}` placeholder — plus explicit `tag`, `security`, and an error response.
#[post(
    path = "/echo/{id}",
    tag = "echo",
    responses((status = 400, body = Pong)),
    security(("bearerAuth" = [])),
)]
async fn echo_handler(
    AppPath(id): AppPath<String>,
    AppJson(req): AppJson<EchoRequest>,
) -> AppJson<Pong> {
    AppJson(Pong {
        message: format!("{id}:{}", req.message),
    })
}

#[derive(Deserialize, IntoParams)]
struct SearchParams {
    q: String,
}

// Exercises `AppQuery<T>` inference: `params(SearchParams)`, relying on `SearchParams`
// deriving `utoipa::IntoParams`.
#[get(path = "/search")]
async fn search_handler(AppQuery(params): AppQuery<SearchParams>) -> AppJson<Pong> {
    AppJson(Pong { message: params.q })
}

fn test_context() -> Arc<ApplicationContext> {
    Arc::new(ApplicationContext::new(Arc::new(TestEnvironment)))
}

#[tokio::test]
async fn collect_routes_includes_every_route_annotated_handler_in_the_openapi_doc() {
    let (_, openapi) = collect_routes().split_for_parts();
    assert!(openapi.paths.paths.contains_key("/ping"));
    assert!(openapi.paths.paths.contains_key("/echo/{id}"));
    assert!(openapi.paths.paths.contains_key("/search"));
}

#[tokio::test]
async fn ping_handler_infers_200_response_with_no_explicit_responses() {
    let (router, openapi) = collect_routes().split_for_parts();
    let app = router.with_state(test_context());

    let operation = openapi.paths.paths["/ping"].get.as_ref().unwrap();
    assert_eq!(operation.responses.responses.len(), 1);
    assert!(operation.responses.responses.contains_key("200"));

    let response = app
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn route_with_inferred_and_explicit_grammar_forwards_correctly_and_responds() {
    let (router, openapi) = collect_routes().split_for_parts();
    let app = router.with_state(test_context());

    let operation = openapi.paths.paths["/echo/{id}"].post.as_ref().unwrap();
    assert_eq!(operation.tags.as_deref(), Some(&["echo".to_string()][..]));
    assert!(operation.request_body.is_some());
    assert_eq!(operation.responses.responses.len(), 2);
    assert!(operation.responses.responses.contains_key("200"));
    assert!(operation.responses.responses.contains_key("400"));
    assert!(operation.security.as_ref().is_some_and(|s| !s.is_empty()));
    assert!(!operation.parameters.as_ref().unwrap().is_empty());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo/abc")
                .header("content-type", "application/json")
                .body(Body::from("{\"message\":\"hi\"}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn search_handler_infers_params_from_app_query() {
    let (router, openapi) = collect_routes().split_for_parts();
    let app = router.with_state(test_context());

    let operation = openapi.paths.paths["/search"].get.as_ref().unwrap();
    let params = operation.parameters.as_ref().unwrap();
    assert!(params.iter().any(|p| p.name == "q"));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/search?q=hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
