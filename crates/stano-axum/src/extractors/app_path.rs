use crate::error::ApiError;
use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;

/// Custom Path extractor that provides detailed error logging
pub struct AppPath<T>(pub T);

impl<T, S> FromRequestParts<S> for AppPath<T>
where
    T: Send + serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(value) = Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(ApiError::from)?;

        Ok(AppPath(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_path_is_tuple_struct() {
        let path = AppPath("test".to_string());
        assert_eq!(path.0, "test");
    }
}
