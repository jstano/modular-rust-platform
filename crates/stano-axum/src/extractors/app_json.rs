use crate::error::ApiError;
use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

/// Custom JSON extractor that provides detailed error logging
#[derive(Debug)]
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(ApiError::from)?;

        Ok(AppJson(value))
    }
}

// Allow AppJson to be used as a response (like Json)
impl<T> IntoResponse for AppJson<T>
where
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestPayload {
        name: String,
        count: i32,
    }

    #[tokio::test]
    async fn valid_json_extracts_successfully() {
        let payload = TestPayload {
            name: "test".to_string(),
            count: 42,
        };
        let json_str = serde_json::to_string(&payload).unwrap();

        let req = Request::builder()
            .uri("/test")
            .header("content-type", "application/json")
            .body(Body::from(json_str))
            .unwrap();

        let result = AppJson::<TestPayload>::from_request(req, &()).await;
        assert!(result.is_ok());

        let AppJson(extracted) = result.unwrap();
        assert_eq!(extracted.name, "test");
        assert_eq!(extracted.count, 42);
    }

    #[tokio::test]
    async fn invalid_json_returns_api_error() {
        let req = Request::builder()
            .uri("/test")
            .header("content-type", "application/json")
            .body(Body::from("{invalid json"))
            .unwrap();

        let result = AppJson::<TestPayload>::from_request(req, &()).await;
        assert!(result.is_err());

        let error = result.unwrap_err();
        match error {
            ApiError::JsonExtraction { status, .. } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
            }
            _ => panic!("Expected JsonExtraction error"),
        }
    }
}
