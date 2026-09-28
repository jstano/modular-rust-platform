//! JWT encode/decode (ES256) and a generic [`SecurityContext<E>`] for auth.
#![warn(missing_docs)]

mod jwt;
mod security_context;

pub use jwt::{JwtConfig, JwtError, decode_jwt, encode_jwt};
pub use security_context::{Claims, SecurityContext};
