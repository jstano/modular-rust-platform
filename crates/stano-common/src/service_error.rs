#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Resource not found")]
    NotFound,

    #[error("Invalid request: {0}")]
    InvalidInput(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,

    #[error("Internal error")]
    Internal(#[from] anyhow::Error),
}
