use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims<E> {
    /// Subject (typically the app's typed ID as a string)
    pub sub: String,
    /// Session ID (optional, for tracking sessions)
    pub session_id: String,
    pub exp: usize, // Expiration time (as UTC timestamp)
    /// App-defined extensions (e.g., email, role, custom claims)
    #[serde(flatten)]
    pub ext: E,
}

#[derive(Clone, Debug)]
pub struct SecurityContext<E> {
    claims: Claims<E>,
}

impl<E> SecurityContext<E> {
    pub fn new(claims: Claims<E>) -> Self {
        Self { claims }
    }

    pub fn sub(&self) -> &str {
        &self.claims.sub
    }

    pub fn session_id(&self) -> &str {
        &self.claims.session_id
    }

    pub fn ext(&self) -> &E {
        &self.claims.ext
    }

    pub fn claims(&self) -> &Claims<E> {
        &self.claims
    }
}
