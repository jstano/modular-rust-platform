/// Errors that originate in the domain layer.
///
/// These are typed business errors that carry semantic meaning.
/// Infrastructure and service layers convert these to their own error types.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum DomainError {
    /// Input data failed a domain validation rule.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// An operation was attempted that violates a business rule.
    #[error("Business rule violation: {0}")]
    BusinessRuleViolation(String),
}

/// Convert a DomainError to a ServiceError for use in service layer call sites.
pub fn domain_err_to_service(err: DomainError) -> crate::ServiceError {
    match err {
        DomainError::InvalidInput(msg) => crate::ServiceError::InvalidInput(msg),
        DomainError::BusinessRuleViolation(msg) => crate::ServiceError::Conflict(msg),
    }
}
