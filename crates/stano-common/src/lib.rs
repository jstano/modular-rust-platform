//! Shared error types, typed ID macro, and utilities used across the platform.
#![warn(missing_docs)]

/// [`DomainError`] and the domain→service error conversion.
pub mod error;
/// The [`id_type!`] macro for generating typed UUID wrappers.
pub mod id_types;
/// [`ServiceError`], the service-layer error type.
pub mod service_error;

pub use uuid;

// Re-export for convenience
pub use error::{domain_err_to_service, DomainError};
pub use service_error::ServiceError;
