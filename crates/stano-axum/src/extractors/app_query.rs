use crate::error::ApiError;
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;

/// Custom Query extractor that provides detailed error logging
#[derive(Debug)]
pub struct AppQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for AppQuery<T>
where
    T: Send + serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(ApiError::from)?;

        Ok(AppQuery(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct SearchQuery {
        q: String,
        limit: Option<i32>,
    }

    #[tokio::test]
    async fn valid_query_params_extract_successfully() {
        let uri = "/search?q=rust&limit=10";
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();

        let (mut parts, _) = req.into_parts();
        let result = AppQuery::<SearchQuery>::from_request_parts(&mut parts, &()).await;

        assert!(result.is_ok());
        let AppQuery(query) = result.unwrap();
        assert_eq!(query.q, "rust");
        assert_eq!(query.limit, Some(10));
    }

    #[tokio::test]
    async fn optional_query_params_work() {
        let uri = "/search?q=rust";
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();

        let (mut parts, _) = req.into_parts();
        let result = AppQuery::<SearchQuery>::from_request_parts(&mut parts, &()).await;

        assert!(result.is_ok());
        let AppQuery(query) = result.unwrap();
        assert_eq!(query.q, "rust");
        assert_eq!(query.limit, None);
    }

    #[tokio::test]
    async fn missing_required_param_returns_api_error() {
        let uri = "/search?limit=10"; // missing required 'q' param
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();

        let (mut parts, _) = req.into_parts();
        let result = AppQuery::<SearchQuery>::from_request_parts(&mut parts, &()).await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        match error {
            ApiError::QueryExtraction { status, .. } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
            }
            _ => panic!("Expected QueryExtraction error"),
        }
    }
}
