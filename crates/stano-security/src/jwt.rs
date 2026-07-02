use crate::Claims;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub private_key_pem: String,
    pub public_key_pem: String,
    pub expiration_seconds: u64,
}

#[derive(Debug, Error)]
pub enum JwtError {
    #[error("Failed to encode JWT: {0}")]
    EncodingFailed(String),

    #[error("Failed to decode JWT: {0}")]
    DecodingFailed(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Token expired")]
    TokenExpired,

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
    // JWT encode/decode tests require valid EC private/public key pairs.
    // Integration tests in apps using stano-security should test encode_jwt / decode_jwt
    // with real keys loaded from environment variables.
}
