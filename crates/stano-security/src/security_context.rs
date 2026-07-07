use serde::{Deserialize, Serialize};

/// JWT payload, generic over an app-defined extension type `E` for custom claims.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims<E> {
    /// Subject (typically the app's typed ID as a string)
    pub sub: String,
    /// Session ID (optional, for tracking sessions)
    pub session_id: String,
    /// Expiration time (as UTC timestamp)
    pub exp: usize,
    /// App-defined extensions (e.g., email, role, custom claims)
    #[serde(flatten)]
    pub ext: E,
}

/// Wraps validated JWT [`Claims`] for use in request handlers/extractors.
#[derive(Clone, Debug)]
pub struct SecurityContext<E> {
    claims: Claims<E>,
}

impl<E> SecurityContext<E> {
    /// Wrap already-validated claims (typically produced by `decode_jwt`).
    pub fn new(claims: Claims<E>) -> Self {
        Self { claims }
    }

    /// The subject of the token (typically the app's typed user ID as a string).
    pub fn sub(&self) -> &str {
        &self.claims.sub
    }

    /// The session ID the token was issued for.
    pub fn session_id(&self) -> &str {
        &self.claims.session_id
    }

    /// The app-defined extension claims (e.g. email, role).
    pub fn ext(&self) -> &E {
        &self.claims.ext
    }

    /// The full underlying claims.
    pub fn claims(&self) -> &Claims<E> {
        &self.claims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_context_new_and_accessors_with_unit_ext() {
        let claims = Claims {
            sub: "user-1".to_string(),
            session_id: "session-1".to_string(),
            exp: 1000,
            ext: (),
        };
        let context = SecurityContext::new(claims.clone());
        assert_eq!(context.sub(), "user-1");
        assert_eq!(context.session_id(), "session-1");
        assert_eq!(*context.ext(), ());
        assert_eq!(context.claims().exp, 1000);
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct CustomExt {
        role: String,
    }

    #[test]
    fn test_security_context_new_and_accessors_with_custom_ext_struct() {
        let claims = Claims {
            sub: "user-2".to_string(),
            session_id: "session-2".to_string(),
            exp: 2000,
            ext: CustomExt {
                role: "admin".to_string(),
            },
        };
        let context = SecurityContext::new(claims.clone());
        assert_eq!(context.sub(), "user-2");
        assert_eq!(context.session_id(), "session-2");
        assert_eq!(context.ext().role, "admin");
        assert_eq!(context.claims().exp, 2000);
    }
}
