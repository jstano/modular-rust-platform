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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceError;

    #[test]
    fn invalid_input_display_message() {
        let err = DomainError::InvalidInput("bad value".to_string());
        assert_eq!(err.to_string(), "Invalid input: bad value");
    }

    #[test]
    fn business_rule_violation_display_message() {
        let err = DomainError::BusinessRuleViolation("cannot do that".to_string());
        assert_eq!(err.to_string(), "Business rule violation: cannot do that");
    }

    #[test]
    fn equality_compares_variant_and_message() {
        assert_eq!(
            DomainError::InvalidInput("x".to_string()),
            DomainError::InvalidInput("x".to_string())
        );
        assert_ne!(
            DomainError::InvalidInput("x".to_string()),
            DomainError::InvalidInput("y".to_string())
        );
        assert_ne!(
            DomainError::InvalidInput("x".to_string()),
            DomainError::BusinessRuleViolation("x".to_string())
        );
    }

    #[test]
    fn maps_invalid_input_to_service_invalid_input() {
        let result = domain_err_to_service(DomainError::InvalidInput("bad".to_string()));
        match result {
            ServiceError::InvalidInput(msg) => assert_eq!(msg, "bad"),
            other => panic!("expected ServiceError::InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn maps_business_rule_violation_to_service_conflict() {
        let result =
            domain_err_to_service(DomainError::BusinessRuleViolation("conflict".to_string()));
        match result {
            ServiceError::Conflict(msg) => assert_eq!(msg, "conflict"),
            other => panic!("expected ServiceError::Conflict, got {other:?}"),
        }
    }
}
