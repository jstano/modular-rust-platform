use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};

/// The type-erased async fn `PostAuthorizationHook` wraps: takes the request and the
/// remaining middleware chain, returns a boxed future resolving to the response.
type ApplyFn =
    dyn Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send>> + Send + Sync;

/// An optional middleware hook [`crate::server::run`](../../stano_launcher/fn.run.html)
/// (in `stano-launcher`) applies immediately after [`AuthorizationLayer`](super::AuthorizationLayer)
/// in request-flow order — between authorization and the router, closer to handlers.
///
/// Exists so a consumer can bridge whatever `AuthorizationLayer` inserted into request
/// extensions (a `stano_security::SecurityContext<E>`) into an app-local mechanism (e.g. a
/// `tokio::task_local!` an existing service layer already reads from), without
/// `stano-launcher`/`stano-axum` needing to know that mechanism exists. Purely additive —
/// pass `None` to `stano_launcher::run` to skip it; every existing consumer is unaffected.
#[derive(Clone)]
pub struct PostAuthorizationHook {
    apply: Arc<ApplyFn>,
}

impl PostAuthorizationHook {
    /// Builds a hook from a plain async fn matching `axum::middleware::from_fn`'s simplest
    /// shape — `async fn(Request, Next) -> impl IntoResponse`, no extractors — the same
    /// signature as any ordinary `axum::middleware::from_fn` middleware function.
    pub fn from_fn<Fut, R>(f: fn(Request, Next) -> Fut) -> Self
    where
        Fut: Future<Output = R> + Send + 'static,
        R: IntoResponse + 'static,
    {
        Self {
            apply: Arc::new(move |req, next| {
                Box::pin(async move { f(req, next).await.into_response() })
            }),
        }
    }
}

impl<S> Layer<S> for PostAuthorizationHook
where
    // `axum::middleware::Next` (built internally by `middleware::from_fn`, which this type
    // delegates to) can only ever wrap a service with `Error = Infallible` — the same
    // implicit constraint any `axum::middleware::from_fn` middleware has. In practice this
    // is never a real restriction: `PostAuthorizationHook` is only ever applied via
    // `axum::Router::layer(...)`, whose internal service type already satisfies it.
    S: Service<Request, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    type Service = BoxCloneSyncService<Request, Response, std::convert::Infallible>;

    fn layer(&self, inner: S) -> Self::Service {
        let apply = Arc::clone(&self.apply);
        // `middleware::from_fn` is what actually knows how to build a `Next` wrapping
        // `inner` — delegate to it for the real invocation, routed through `apply` so the
        // hook function itself stays outside `stano-axum`.
        let from_fn_layer = middleware::from_fn(move |req: Request, next: Next| {
            let apply = Arc::clone(&apply);
            async move { apply(req, next).await }
        });
        BoxCloneSyncService::new(from_fn_layer.layer(inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request as HttpRequest, StatusCode};
    use axum::routing::get;
    use tower::util::ServiceExt;

    async fn tag_response(req: Request, next: Next) -> Response {
        let mut res = next.run(req).await;
        res.headers_mut()
            .insert("x-hook-ran", HeaderValue::from_static("yes"));
        res
    }

    #[tokio::test]
    async fn hook_runs_and_can_modify_the_response() {
        let app = Router::new()
            .route("/hello", get(|| async { "ok" }))
            .layer(PostAuthorizationHook::from_fn(tag_response));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/hello")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-hook-ran").unwrap(), "yes");
    }

    async fn tag_again(req: Request, next: Next) -> Response {
        let mut res = next.run(req).await;
        res.headers_mut()
            .insert("x-hook-2-ran", HeaderValue::from_static("yes"));
        res
    }

    #[tokio::test]
    async fn hook_composes_with_another_layer_stacked_on_top() {
        // Confirms the erased `BoxCloneSyncService` composes fine stacked under another
        // layer, the same way `run()` stacks it alongside `AuthorizationLayer` and the rest
        // of the fixed middleware stack.
        let app = Router::new()
            .route("/hello", get(|| async { "ok" }))
            .layer(PostAuthorizationHook::from_fn(tag_response))
            .layer(PostAuthorizationHook::from_fn(tag_again));

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/hello")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-hook-ran").unwrap(), "yes");
        assert_eq!(response.headers().get("x-hook-2-ran").unwrap(), "yes");
    }
}
