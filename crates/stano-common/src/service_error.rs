/// Errors that originate in the service layer.
///
/// Distinct from [`crate::DomainError`]: these map directly to HTTP status
/// codes by `stano-axum`'s `ApiError`, so they carry request-handling
/// semantics (not-found, conflict, auth) rather than pure business rules.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The requested resource does not exist.
    #[error("Resource not found")]
    NotFound,

    /// The request was malformed or failed validation.
    #[error("Invalid request: {0}")]
    InvalidInput(String),

    /// The request conflicts with existing state.
    #[error("Conflict: {0}")]
    Conflict(String),

    /// The caller is not authenticated.
    #[error("Unauthorized")]
    Unauthorized,

    /// The caller is authenticated but lacks permission.
    #[error("Forbidden")]
    Forbidden,

    /// An unexpected internal error occurred.
    #[error("Internal error")]
    Internal(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn not_found_display_message() {
        assert_eq!(ServiceError::NotFound.to_string(), "Resource not found");
    }

    #[test]
    fn invalid_input_display_message_includes_detail() {
        let err = ServiceError::InvalidInput("bad".to_string());
        assert_eq!(err.to_string(), "Invalid request: bad");
    }

    #[test]
    fn conflict_display_message_includes_detail() {
        let err = ServiceError::Conflict("already exists".to_string());
        assert_eq!(err.to_string(), "Conflict: already exists");
    }

    #[test]
    fn unauthorized_display_message() {
        assert_eq!(ServiceError::Unauthorized.to_string(), "Unauthorized");
    }

    #[test]
    fn forbidden_display_message() {
        assert_eq!(ServiceError::Forbidden.to_string(), "Forbidden");
    }

    #[test]
    fn internal_from_anyhow_display_is_generic() {
        let err: ServiceError = anyhow!("db down").into();
        assert_eq!(err.to_string(), "Internal error");
    }

    #[test]
    fn internal_debug_includes_source_chain() {
        let err: ServiceError = anyhow!("db down").into();
        assert!(format!("{err:?}").contains("db down"));
    }
}
