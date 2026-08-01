use crate::error::ApiError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use stano_common::ServiceError;
use stano_security::SecurityContext;

/// Extracts the [`SecurityContext`] inserted into request extensions by
/// [`crate::security::AuthorizationLayer`] once a request has passed an `authenticated()`
/// or `has_role(...)` rule. Rejects with 401 if no context is present (e.g. the route was
/// reached via a `permit_all()` rule with no bearer token supplied).
pub struct AppSecurityContext<E>(pub SecurityContext<E>);

impl<S, E> FromRequestParts<S> for AppSecurityContext<E>
where
    E: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SecurityContext<E>>()
            .cloned()
            .map(AppSecurityContext)
            .ok_or_else(|| ApiError::from(ServiceError::Unauthorized))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use stano_security::Claims;
    use tower::util::ServiceExt;

    #[derive(Clone)]
    struct AppExt {
        role: String,
    }

    async fn handler(AppSecurityContext(sc): AppSecurityContext<AppExt>) -> String {
        sc.ext().role.clone()
    }

    #[tokio::test]
    async fn extracts_context_from_extensions() {
        let router = Router::new()
            .route("/me", get(handler))
            .layer(axum::middleware::from_fn(
                |mut req: Request, next: axum::middleware::Next| async move {
                    req.extensions_mut().insert(SecurityContext::new(Claims {
                        sub: "user-1".to_string(),
                        session_id: "session-1".to_string(),
                        exp: 0,
                        ext: AppExt {
                            role: "ADMIN".to_string(),
                        },
                    }));
                    next.run(req).await
                },
            ));

        let response = router
            .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"ADMIN");
    }

    #[tokio::test]
    async fn rejects_when_missing() {
        let router = Router::new().route("/me", get(handler));
        let response = router
            .oneshot(Request::builder().uri("/me").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
