# stano-security

JWT-based authentication primitives: encode/decode generically-extensible JWT claims and wrap them in a security context for task-local request identity.

## Install

```toml
[dependencies]
stano-security = { path = "../stano-security" }
```

## API

### JWT Configuration & Functions

- **`JwtConfig { private_key_pem, public_key_pem, expiration_seconds }`** — holds EC private/public key PEM strings and an expiration duration in seconds (stored on the config; not automatically enforced at encode time — see Notes). Keys must be EC (prime256v1) for ES256 signing.

- **`encode_jwt(claims: &Claims<E>, config: &JwtConfig) -> Result<String, JwtError>`** — encodes `Claims` into a signed JWT token (ES256 algorithm).

- **`decode_jwt(token: &str, config: &JwtConfig) -> Result<Claims<E>, JwtError>`** — decodes and verifies a JWT token, returning the claims. Validates signature and expiration.

- **`JwtError`** — error type:
  - `EncodingFailed(String)` — failed to encode token.
  - `DecodingFailed(String)` — failed to decode token.
  - `InvalidKey(String)` — PEM key format invalid.
  - `TokenExpired` — token's exp timestamp is in the past.
  - `InvalidToken(String)` — token is malformed or otherwise invalid.

### Claims & Context

- **`Claims<E> { sub: String, session_id: String, exp: usize, ext: E }`** — generic JWT payload.
  - `sub` — typically a user/app typed ID (as a string).
  - `session_id` — arbitrary session tracking ID.
  - `exp` — expiration time (UTC timestamp, seconds since epoch).
  - `ext` — app-defined extension type (flattened in JSON; use for email, role, custom claims, etc.).

- **`SecurityContext<E>`** — wraps `Claims<E>` for use in request handlers.
  - `new(claims: Claims<E>) -> Self`
  - `sub(&self) -> &str` — extract subject.
  - `session_id(&self) -> &str` — extract session ID.
  - `ext(&self) -> &E` — extract app extensions.
  - `claims(&self) -> &Claims<E>` — access full claims struct.

## Usage Example

```rust
use stano_security::{JwtConfig, Claims, SecurityContext, encode_jwt, decode_jwt};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MyExt {
    email: String,
}

// Setup keys (from environment or file).
let config = JwtConfig {
    private_key_pem: "-----BEGIN EC PRIVATE KEY-----\n...".into(),
    public_key_pem: "-----BEGIN PUBLIC KEY-----\n...".into(),
    expiration_seconds: 3600,
};

// Create claims (app fills in `exp` based on desired lifetime).
let claims = Claims {
    sub: "user-123".into(),
    session_id: "session-456".into(),
    exp: (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600) as usize,
    ext: MyExt { email: "user@example.com".into() },
};

// Encode to JWT string.
let token = encode_jwt(&claims, &config)?;

// Decode and verify (in a request handler).
let decoded: Claims<MyExt> = decode_jwt(&token, &config)?;
let ctx = SecurityContext::new(decoded);

println!("User: {}", ctx.sub());
println!("Email: {}", ctx.ext().email);
```

## Notes

- **App-defined extension type** — `E` is your crate's type. Define an enum or struct containing role, permissions, email, or any other custom claims; it gets flattened into the JWT JSON.
- **No authorization** — this crate only handles JWT encode/decode. Authorization (role checks, permission guards) are the app's responsibility.
- **Keys must be EC (prime256v1)** for ES256 signing. Generate with `openssl ecparam -name prime256v1 -genkey -noout -out private.pem` and `openssl ec -in private.pem -pubout -out public.pem`.
- **Token expiration** — the `expiration_seconds` field in `JwtConfig` is stored but not automatically used by encode/decode. Callers must set `Claims.exp` themselves (typically `now + expiration_seconds`).
