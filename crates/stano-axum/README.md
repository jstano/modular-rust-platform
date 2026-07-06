# stano-axum

Axum HTTP layer: custom extractors with detailed error logging, a unified `ApiError` type that maps service errors to HTTP responses, and a standard error response format.

## Install

```toml
[dependencies]
stano-axum = { path = "../stano-axum" }
stano-common = { path = "../stano-common" }
```

## API

### Error Types

- **`ApiError`** — HTTP response error type (implements `IntoResponse`).
  - `JsonExtraction { status, body_text, rejection_type }` — JSON deserialization failed.
  - `PathExtraction { status, body_text, rejection_type }` — path parameter extraction failed.
  - `QueryExtraction { status, body_text, rejection_type }` — query parameter extraction failed.
  - `Service(ServiceError)` — service layer error (mapped to HTTP via `ServiceError` status code).
  - `Internal(String)` — unhandled internal error (returns 500; details hidden from clients).

  **Behavior when converted to HTTP response:**
  - `ServiceError::NotFound` → 404 (no error logged).
  - `ServiceError::InvalidInput` → 400 (logged).
  - `ServiceError::Conflict` → 409 (logged).
  - `ServiceError::Unauthorized` → 401 (no error logged).
  - `ServiceError::Forbidden` → 403 (no error logged).
  - `ServiceError::Internal` or `ApiError::Internal` → 500 (logged, details hidden from response).
  - Extraction errors (`JsonExtraction`, etc.) → client error status with `code: "INVALID_JSON"` / `"INVALID_PATH"` / `"INVALID_QUERY"` (logged).

- **`ErrorResponse`** — standard JSON error shape: `{ status, code, message, details?, request_id? }`.
  - `new(status: u16, code: impl Into<...>, message: impl Into<String>) -> Self`
  - `with_details(mut self, details: impl Into<String>) -> Self`
  - `with_request_id(mut self, request_id: impl Into<String>) -> Self`

### Extractors

- **`AppJson<T>`** — JSON extractor (like `axum::Json`, but with detailed error logging).
  - `impl FromRequest<S> for AppJson<T> where T: DeserializeOwned`
  - `impl IntoResponse for AppJson<T> where T: Serialize` — also usable as a response type.

- **`AppPath<T>`** — path parameter extractor (like `axum::Path`).
  - `impl FromRequestParts<S> for AppPath<T> where T: DeserializeOwned + Send`

- **`AppQuery<T>`** — query parameter extractor (like `axum::Query`).
  - `impl FromRequestParts<S> for AppQuery<T> where T: DeserializeOwned + Send`

### Middleware

- **`error_logging_middleware`** — logs `ApiError` with request context (method, URI, request ID).
  - Reads `x-request-id` from the request and logs any `ApiError` stored in response extensions.

## Usage Example

```rust
use stano_axum::{AppJson, AppPath, ApiError, ErrorResponse};
use stano_common::{ServiceError, id_type};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

id_type!(UserId, uuid_v7);

#[derive(Serialize, Deserialize)]
pub struct UserRequest {
    email: String,
}

#[derive(Serialize)]
pub struct UserResponse {
    id: String,
    email: String,
}

// Simulate a service.
async fn create_user(req: UserRequest) -> Result<UserResponse, ServiceError> {
    if req.email.is_empty() {
        return Err(ServiceError::InvalidInput("email required".into()));
    }
    Ok(UserResponse {
        id: UserId::new().to_string(),
        email: req.email,
    })
}

// HTTP handler using custom extractors.
async fn create_user_handler(
    AppJson(req): AppJson<UserRequest>,
) -> Result<AppJson<UserResponse>, ApiError> {
    let user = create_user(req).await?; // ServiceError -> ApiError via impl From
    Ok(AppJson(user))
}

// Another handler using path extraction.
async fn get_user(
    AppPath(user_id): AppPath<UserId>,
) -> Result<AppJson<UserResponse>, ApiError> {
    // Fetch user...
    Err(ServiceError::NotFound.into()) // -> 404 response
}
```

## Notes

- **Error details privacy** — 4xx responses include `details` field; 5xx responses do not (security: don't leak internals).
- **Request ID integration** — the `error_logging_middleware` reads `x-request-id` from incoming requests. For the ID to be set on the request itself, the middleware stack must include `SetRequestIdLayer` (typically applied in `stano-launcher`). The middleware still works if the ID is missing (just logs `request_id: None`).
- **No feature flags** — all APIs available.
