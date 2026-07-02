use serde::Serialize;
use std::borrow::Cow;

/// Standard JSON error response format
#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    /// HTTP status code
    pub status: u16,

    /// Error code for client-side handling (e.g., "INVALID_JSON", "NOT_FOUND")
    pub code: Cow<'static, str>,

    /// Human-readable error message
    pub message: String,

    /// Optional detailed error information (omitted in production for internal errors)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,

    /// Request ID for correlation with logs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl ErrorResponse {
    pub fn new(
        status: u16,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            details: None,
            request_id: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_basic_error_response() {
        let error = ErrorResponse::new(404, "NOT_FOUND", "Resource not found");

        assert_eq!(error.status, 404);
        assert_eq!(error.code, "NOT_FOUND");
        assert_eq!(error.message, "Resource not found");
        assert_eq!(error.details, None);
        assert_eq!(error.request_id, None);
    }

    #[test]
    fn with_details_adds_details() {
        let error = ErrorResponse::new(400, "INVALID_INPUT", "Invalid request")
            .with_details("Email must be valid");

        assert_eq!(error.status, 400);
        assert_eq!(error.code, "INVALID_INPUT");
        assert_eq!(error.message, "Invalid request");
        assert_eq!(error.details, Some("Email must be valid".to_string()));
        assert_eq!(error.request_id, None);
    }

    #[test]
    fn with_request_id_adds_request_id() {
        let error = ErrorResponse::new(500, "INTERNAL_ERROR", "Something went wrong")
            .with_request_id("req-12345");

        assert_eq!(error.status, 500);
        assert_eq!(error.code, "INTERNAL_ERROR");
        assert_eq!(error.message, "Something went wrong");
        assert_eq!(error.details, None);
        assert_eq!(error.request_id, Some("req-12345".to_string()));
    }

    #[test]
    fn chained_builders_work_correctly() {
        let error = ErrorResponse::new(409, "CONFLICT", "Resource conflict")
            .with_details("User already exists")
            .with_request_id("req-99999");

        assert_eq!(error.status, 409);
        assert_eq!(error.code, "CONFLICT");
        assert_eq!(error.message, "Resource conflict");
        assert_eq!(error.details, Some("User already exists".to_string()));
        assert_eq!(error.request_id, Some("req-99999".to_string()));
    }

    #[test]
    fn serialization_excludes_none_fields() {
        let error = ErrorResponse::new(401, "UNAUTHORIZED", "Authentication required");
        let json = serde_json::to_value(&error).unwrap();

        assert_eq!(json["status"], 401);
        assert_eq!(json["code"], "UNAUTHORIZED");
        assert_eq!(json["message"], "Authentication required");
        assert!(!json.as_object().unwrap().contains_key("details"));
        assert!(!json.as_object().unwrap().contains_key("request_id"));
    }

    #[test]
    fn serialization_includes_present_fields() {
        let error = ErrorResponse::new(400, "BAD_REQUEST", "Invalid input")
            .with_details("Field 'email' is required")
            .with_request_id("req-abc");
        let json = serde_json::to_value(&error).unwrap();

        assert_eq!(json["status"], 400);
        assert_eq!(json["code"], "BAD_REQUEST");
        assert_eq!(json["message"], "Invalid input");
        assert_eq!(json["details"], "Field 'email' is required");
        assert_eq!(json["request_id"], "req-abc");
    }

    #[test]
    fn static_str_codes_work() {
        let error = ErrorResponse::new(404, "NOT_FOUND", "Not found");
        assert_eq!(error.code, "NOT_FOUND");
    }

    #[test]
    fn string_codes_work() {
        let dynamic_code = format!("ERROR_{}", 123);
        let error = ErrorResponse::new(500, dynamic_code.clone(), "Error");
        assert_eq!(error.code, dynamic_code);
    }
}
