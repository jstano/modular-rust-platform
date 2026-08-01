//! Security demo helpers for `stano-example-app`: the app-defined JWT extension type and a
//! hardcoded demo keypair/token issuer. Never use a hardcoded key for anything beyond
//! local demo/testing purposes.

use serde::{Deserialize, Serialize};
use stano_security::{Claims, JwtConfig};

/// App-defined JWT extension type (`E` in `Claims<E>`/`SecurityContext<E>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppClaims {
    pub role: String,
}

// Demo-only ES256 (P-256) EC keypair, generated solely for this example app.
const DEMO_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgtgbDmCbWzH1rPZlb
qucYzcKQppWx4YxRh0TfnEd0wd6hRANCAATbjOo4G431D+jMHWgoGXaW/vr20Qxn
QuoeHrU++Hh7LgqOwXbpqEmKfJa5Os5GQfdQ579fyDqZ/MepnZz2ijhz
-----END PRIVATE KEY-----";

const DEMO_PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE24zqOBuN9Q/ozB1oKBl2lv769tEM
Z0LqHh61Pvh4ey4KjsF26ahJinyWuTrORkH3UOe/X8g6mfzHqZ2c9oo4cw==
-----END PUBLIC KEY-----";

pub fn demo_jwt_config() -> JwtConfig {
    JwtConfig {
        private_key_pem: DEMO_PRIVATE_KEY_PEM.to_string(),
        public_key_pem: DEMO_PUBLIC_KEY_PEM.to_string(),
        expiration_seconds: 3600,
    }
}

/// Test/demo helper: mints a signed token for the given role.
pub fn issue_token(role: &str, jwt_config: &JwtConfig) -> String {
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + jwt_config.expiration_seconds as usize;

    let claims = Claims {
        sub: "demo-user".to_string(),
        session_id: "demo-session".to_string(),
        exp,
        ext: AppClaims {
            role: role.to_string(),
        },
    };
    stano_security::encode_jwt(&claims, jwt_config).expect("token encodes")
}
