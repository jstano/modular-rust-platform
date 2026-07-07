use crate::error::ApiError;
use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;

/// Custom Path extractor that provides detailed error logging
pub struct AppPath<T>(pub T);

impl<T, S> FromRequestParts<S> for AppPath<T>
where
    T: Send + serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(value) = Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(ApiError::from)?;

        Ok(AppPath(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;
    use tower::util::ServiceExt;

    #[test]
    fn app_path_is_tuple_struct() {
        let path = AppPath("test".to_string());
        assert_eq!(path.0, "test");
    }

    async fn handler(AppPath(id): AppPath<u64>) -> String {
        id.to_string()
    }

    fn router() -> Router {
        Router::new().route("/items/{id}", get(handler))
    }

    #[tokio::test]
    async fn valid_path_param_extracts_successfully() {
        let req = Request::builder()
            .uri("/items/42")
            .body(Body::empty())
            .unwrap();

        let response = router().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"42");
    }

    #[tokio::test]
    async fn invalid_path_param_returns_bad_request() {
        let req = Request::builder()
            .uri("/items/not-a-number")
            .body(Body::empty())
            .unwrap();

        let response = router().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("INVALID_PATH"));
    }
}
