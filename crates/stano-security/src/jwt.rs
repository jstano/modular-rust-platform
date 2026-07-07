use crate::Claims;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// ES256 EC key material and expiration used by [`encode_jwt`]/[`decode_jwt`].
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// PEM-encoded EC private key, used to sign tokens.
    pub private_key_pem: String,
    /// PEM-encoded EC public key, used to verify tokens.
    pub public_key_pem: String,
    /// Token lifetime in seconds, applied when computing `exp`.
    pub expiration_seconds: u64,
}

/// Errors returned by [`encode_jwt`]/[`decode_jwt`].
#[derive(Debug, Error)]
pub enum JwtError {
    /// Token signing failed.
    #[error("Failed to encode JWT: {0}")]
    EncodingFailed(String),

    /// Token verification/parsing failed.
    #[error("Failed to decode JWT: {0}")]
    DecodingFailed(String),

    /// The configured PEM key could not be parsed.
    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    /// The token's `exp` claim is in the past.
    #[error("Token expired")]
    TokenExpired,

    /// The token failed validation for a reason other than expiration.
    #[error("Invalid token: {0}")]
    InvalidToken(String),
}

/// Encode a Claims struct into a JWT token.
pub fn encode_jwt<E>(claims: &Claims<E>, config: &JwtConfig) -> Result<String, JwtError>
where
    E: Serialize,
{
    let encoding_key = EncodingKey::from_ec_pem(config.private_key_pem.as_bytes())
        .map_err(|e| JwtError::InvalidKey(e.to_string()))?;

    encode(&Header::new(Algorithm::ES256), claims, &encoding_key)
        .map_err(|e| JwtError::EncodingFailed(e.to_string()))
}

/// Decode and verify a JWT token, returning the Claims.
pub fn decode_jwt<E>(token: &str, config: &JwtConfig) -> Result<Claims<E>, JwtError>
where
    E: for<'de> Deserialize<'de>,
{
    let decoding_key = DecodingKey::from_ec_pem(config.public_key_pem.as_bytes())
        .map_err(|e| JwtError::InvalidKey(e.to_string()))?;

    let validation = Validation::new(Algorithm::ES256);

    decode::<Claims<E>>(token, &decoding_key, &validation)
        .map(|token_data| token_data.claims)
        .map_err(|e| {
            if e.kind() == &jsonwebtoken::errors::ErrorKind::ExpiredSignature {
                JwtError::TokenExpired
            } else {
                JwtError::DecodingFailed(e.to_string())
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-only ES256 (P-256) EC keypair, generated solely for these unit tests.
    const PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgtgbDmCbWzH1rPZlb
qucYzcKQppWx4YxRh0TfnEd0wd6hRANCAATbjOo4G431D+jMHWgoGXaW/vr20Qxn
QuoeHrU++Hh7LgqOwXbpqEmKfJa5Os5GQfdQ579fyDqZ/MepnZz2ijhz
-----END PRIVATE KEY-----";

    const PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE24zqOBuN9Q/ozB1oKBl2lv769tEM
Z0LqHh61Pvh4ey4KjsF26ahJinyWuTrORkH3UOe/X8g6mfzHqZ2c9oo4cw==
-----END PUBLIC KEY-----";

    // A second, unrelated keypair used to exercise signature-mismatch failures.
    const OTHER_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEQYaZ+hmOmyIcf6OlLbdfrdRDIQVP
WvgpcJQZdAq9Q3dsB0xGIC4Ea8ps7xzypEj0W6wXZ/zgKyK9NSmDMtgzPg==
-----END PUBLIC KEY-----";

    fn config() -> JwtConfig {
        JwtConfig {
            private_key_pem: PRIVATE_KEY_PEM.to_string(),
            public_key_pem: PUBLIC_KEY_PEM.to_string(),
            expiration_seconds: 3600,
        }
    }

    fn now() -> usize {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
    }

    fn claims(exp: usize) -> Claims<()> {
        Claims {
            sub: "user-1".to_string(),
            session_id: "session-1".to_string(),
            exp,
            ext: (),
        }
    }

    #[test]
    fn test_encode_decode_round_trip_success() {
        let config = config();
        let original = claims(now() + config.expiration_seconds as usize);
        let token = encode_jwt(&original, &config).expect("encode should succeed");
        let decoded: Claims<()> = decode_jwt(&token, &config).expect("decode should succeed");
        assert_eq!(decoded.sub, original.sub);
        assert_eq!(decoded.session_id, original.session_id);
        assert_eq!(decoded.exp, original.exp);
    }

    #[test]
    fn test_decode_expired_token_returns_token_expired() {
        let config = config();
        let expired = claims(now().saturating_sub(3600));
        let token = encode_jwt(&expired, &config).expect("encode should succeed");
        let result: Result<Claims<()>, JwtError> = decode_jwt(&token, &config);
        assert!(matches!(result, Err(JwtError::TokenExpired)));
    }

    #[test]
    fn test_decode_malformed_token_string_returns_decoding_failed() {
        let config = config();
        let result: Result<Claims<()>, JwtError> = decode_jwt("not-a-valid-jwt", &config);
        assert!(matches!(result, Err(JwtError::DecodingFailed(_))));
    }

    #[test]
    fn test_decode_with_mismatched_public_key_returns_decoding_failed() {
        let signing_config = config();
        let original = claims(now() + 3600);
        let token = encode_jwt(&original, &signing_config).expect("encode should succeed");

        let mut verifying_config = signing_config.clone();
        verifying_config.public_key_pem = OTHER_PUBLIC_KEY_PEM.to_string();

        let result: Result<Claims<()>, JwtError> = decode_jwt(&token, &verifying_config);
        assert!(matches!(result, Err(JwtError::DecodingFailed(_))));
    }

    #[test]
    fn test_encode_with_invalid_pem_returns_invalid_key() {
        let mut config = config();
        config.private_key_pem = "not a pem".to_string();
        let original = claims(now() + 3600);
        let result = encode_jwt(&original, &config);
        assert!(matches!(result, Err(JwtError::InvalidKey(_))));
    }

    #[test]
    fn test_decode_with_invalid_pem_returns_invalid_key() {
        let mut config = config();
        config.public_key_pem = "not a pem".to_string();
        let result: Result<Claims<()>, JwtError> = decode_jwt("irrelevant.token.value", &config);
        assert!(matches!(result, Err(JwtError::InvalidKey(_))));
    }

    #[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
    struct CustomExt {
        email: String,
        role: String,
    }

    #[test]
    fn test_encode_decode_round_trip_with_flatten_ext_struct() {
        let config = config();
        let original = Claims {
            sub: "user-2".to_string(),
            session_id: "session-2".to_string(),
            exp: now() + 3600,
            ext: CustomExt {
                email: "user@example.com".to_string(),
                role: "admin".to_string(),
            },
        };
        let token = encode_jwt(&original, &config).expect("encode should succeed");
        let decoded: Claims<CustomExt> =
            decode_jwt(&token, &config).expect("decode should succeed");
        assert_eq!(decoded.ext, original.ext);
        assert_eq!(decoded.sub, original.sub);
    }
}
