pub mod error;
pub mod id_types;
pub mod service_error;

pub use uuid;

// Re-export for convenience
pub use error::{DomainError, domain_err_to_service};
pub use service_error::ServiceError;
