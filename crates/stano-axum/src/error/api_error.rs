use super::error_response::ErrorResponse;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use stano_common::ServiceError;
use std::sync::Arc;

/// Unified API error type that implements IntoResponse
#[derive(Debug)]
pub enum ApiError {
    /// JSON deserialization failed
    JsonExtraction {
        /// HTTP status Axum's rejection suggested for this failure.
        status: StatusCode,
        /// Axum's rejection body text, included as `details` in the response.
        body_text: String,
        /// The concrete `JsonRejection` variant name, for diagnostics.
        rejection_type: String,
    },

    /// Path parameter extraction failed
    PathExtraction {
        /// HTTP status Axum's rejection suggested for this failure.
        status: StatusCode,
        /// Axum's rejection body text, included as `details` in the response.
        body_text: String,
        /// The concrete `PathRejection` variant name, for diagnostics.
        rejection_type: String,
    },

    /// Query parameter extraction failed
    QueryExtraction {
        /// HTTP status Axum's rejection suggested for this failure.
        status: StatusCode,
        /// Axum's rejection body text, included as `details` in the response.
        body_text: String,
        /// The concrete `QueryRejection` variant name, for diagnostics.
        rejection_type: String,
    },

    /// Service layer error
    Service(ServiceError),

    /// Internal server error (with optional cause)
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message, details, should_log) = match &self {
            ApiError::JsonExtraction {
                status, body_text, ..
            } => (
                *status,
                "INVALID_JSON",
                "Failed to parse JSON request body",
                Some(body_text.clone()),
                true, // Log JSON parsing errors
            ),

            ApiError::PathExtraction {
                status, body_text, ..
            } => (
                *status,
                "INVALID_PATH",
                "Invalid path parameter",
                Some(body_text.clone()),
                true, // Log path parsing errors
            ),

            ApiError::QueryExtraction {
                status, body_text, ..
            } => (
                *status,
                "INVALID_QUERY",
                "Invalid query parameter",
                Some(body_text.clone()),
                true, // Log query parsing errors
            ),

            ApiError::Service(service_err) => {
                match service_err {
                    ServiceError::NotFound => (
                        StatusCode::NOT_FOUND,
                        "NOT_FOUND",
                        "Resource not found",
                        None,
                        false, // Don't log 404s (too noisy)
                    ),
                    ServiceError::InvalidInput(msg) => (
                        StatusCode::BAD_REQUEST,
                        "INVALID_INPUT",
                        "Invalid request",
                        Some(msg.clone()),
                        true,
                    ),
                    ServiceError::Conflict(msg) => (
                        StatusCode::CONFLICT,
                        "CONFLICT",
                        "Resource conflict",
                        Some(msg.clone()),
                        true,
                    ),
                    ServiceError::Unauthorized => (
                        StatusCode::UNAUTHORIZED,
                        "UNAUTHORIZED",
                        "Authentication required",
                        None,
                        false, // Don't log auth failures (too noisy)
                    ),
                    ServiceError::Forbidden => (
                        StatusCode::FORBIDDEN,
                        "FORBIDDEN",
                        "Insufficient permissions",
                        None,
                        false,
                    ),
                    ServiceError::Internal(_err) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "INTERNAL_ERROR",
                        "An internal error occurred",
                        None, // Don't expose internal error details to client
                        true,
                    ),
                }
            }

            ApiError::Internal(_msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "An internal error occurred",
                None, // Don't expose internal details
                true,
            ),
        };

        // Log error with full context (will be picked up by middleware)
        if should_log {
            tracing::error!(
                error_type = ?self,
                status = %status,
                code = %code,
                "API error occurred"
            );
        }

        // Build error response
        let mut error_response = ErrorResponse::new(status.as_u16(), code, message);

        // Only include details for client errors (4xx), not server errors (5xx)
        // This prevents leaking internal error details
        if status.is_client_error()
            && let Some(d) = details
        {
            error_response = error_response.with_details(d);
        }

        // Build HTTP response
        let mut response = (status, Json(error_response)).into_response();

        // Store error in extensions for middleware logging
        response.extensions_mut().insert(Arc::new(self));

        response
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        tracing::warn!(
            rejection_status = %rejection.status(),
            rejection_body = %rejection.body_text(),
            "JSON extraction failed"
        );

        ApiError::JsonExtraction {
            status: rejection.status(),
            body_text: rejection.body_text(),
            rejection_type: std::any::type_name_of_val(&rejection).to_string(),
        }
    }
}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        tracing::warn!(
            rejection_status = %rejection.status(),
            rejection_body = %rejection.body_text(),
            "Path extraction failed"
        );

        ApiError::PathExtraction {
            status: rejection.status(),
            body_text: rejection.body_text(),
            rejection_type: std::any::type_name_of_val(&rejection).to_string(),
        }
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        tracing::warn!(
            rejection_status = %rejection.status(),
            rejection_body = %rejection.body_text(),
            "Query extraction failed"
        );

        ApiError::QueryExtraction {
            status: rejection.status(),
            body_text: rejection.body_text(),
            rejection_type: std::any::type_name_of_val(&rejection).to_string(),
        }
    }
}

impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        ApiError::Service(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_error_not_found_maps_correctly() {
        let api_err = ApiError::from(ServiceError::NotFound);
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn service_error_invalid_input_maps_correctly() {
        let api_err = ApiError::from(ServiceError::InvalidInput("test".to_string()));
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn service_error_conflict_maps_correctly() {
        let api_err = ApiError::from(ServiceError::Conflict("duplicate entry".to_string()));
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn service_error_unauthorized_maps_correctly() {
        let api_err = ApiError::from(ServiceError::Unauthorized);
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn service_error_forbidden_maps_correctly() {
        let api_err = ApiError::from(ServiceError::Forbidden);
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn service_error_internal_maps_correctly() {
        let api_err = ApiError::from(ServiceError::Internal(anyhow::anyhow!("database error")));
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn internal_error_hides_details() {
        let api_err = ApiError::Internal("sensitive info".to_string());
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn json_extraction_error_returns_bad_request() {
        let api_err = ApiError::JsonExtraction {
            status: StatusCode::BAD_REQUEST,
            body_text: "invalid json".to_string(),
            rejection_type: "JsonRejection".to_string(),
        };
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn path_extraction_error_returns_bad_request() {
        let api_err = ApiError::PathExtraction {
            status: StatusCode::BAD_REQUEST,
            body_text: "invalid path".to_string(),
            rejection_type: "PathRejection".to_string(),
        };
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn query_extraction_error_returns_bad_request() {
        let api_err = ApiError::QueryExtraction {
            status: StatusCode::BAD_REQUEST,
            body_text: "invalid query".to_string(),
            rejection_type: "QueryRejection".to_string(),
        };
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn api_error_stored_in_response_extensions() {
        let api_err = ApiError::from(ServiceError::NotFound);
        let response = api_err.into_response();

        // Verify that the error is stored in extensions for middleware
        let has_error = response.extensions().get::<Arc<ApiError>>().is_some();
        assert!(
            has_error,
            "ApiError should be stored in response extensions"
        );
    }

    #[test]
    fn client_errors_include_details() {
        let api_err = ApiError::from(ServiceError::InvalidInput("email is required".to_string()));
        let response = api_err.into_response();

        // 4xx errors should include details
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.status().is_client_error());
    }

    #[test]
    fn server_errors_hide_details() {
        let api_err = ApiError::Internal("database connection failed".to_string());
        let response = api_err.into_response();

        // 5xx errors should not include sensitive details
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.status().is_server_error());
    }
}
