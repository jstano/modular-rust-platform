use super::authorization::{Effect, Rule};
use crate::error::ApiError;
use axum::extract::Request;
use axum::http::{HeaderMap, Method};
use axum::response::{IntoResponse, Response};
use stano_common::ServiceError;
use stano_security::{JwtConfig, SecurityContext, decode_jwt};
use std::sync::Arc;
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};

/// The finalized, type-erased result of [`super::AuthorizationBuilder::build`], applied via
/// `.layer(...)` (or passed into `stano_launcher::run(...)`). Carries no generic over the
/// app's claims extension type — that was resolved when `.build()` was called.
#[derive(Clone)]
pub struct AuthorizationLayer(Arc<CheckFn>);

enum Outcome {
    Allow,
    Unauthorized,
    Forbidden,
}

type CheckFn =
    dyn Fn(&Method, &str, Option<String>, &mut axum::http::Extensions) -> Outcome + Send + Sync;

pub(super) fn build_layer<E>(
    rules: Vec<Rule<E>>,
    any_request: Effect<E>,
    jwt_config: JwtConfig,
) -> AuthorizationLayer
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    let check = move |method: &Method,
                      path: &str,
                      token: Option<String>,
                      extensions: &mut axum::http::Extensions|
          -> Outcome {
        let effect = rules
            .iter()
            .find(|rule| {
                rule.methods
                    .as_ref()
                    .map(|methods| methods.contains(method))
                    .unwrap_or(true)
                    && rule.pattern.matches(path)
            })
            .map(|rule| rule.effect.clone())
            .unwrap_or_else(|| any_request.clone());

        apply_effect(effect, token.as_deref(), &jwt_config, extensions)
    };

    AuthorizationLayer(Arc::new(check))
}

fn apply_effect<E>(
    effect: Effect<E>,
    token: Option<&str>,
    jwt_config: &JwtConfig,
    extensions: &mut axum::http::Extensions,
) -> Outcome
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    match effect {
        Effect::PermitAll => {
            if let Some(token) = token
                && let Ok(claims) = decode_jwt::<E>(token, jwt_config)
            {
                extensions.insert(SecurityContext::new(claims));
            }
            Outcome::Allow
        }
        Effect::Authenticated => match token.and_then(|t| decode_jwt::<E>(t, jwt_config).ok()) {
            Some(claims) => {
                extensions.insert(SecurityContext::new(claims));
                Outcome::Allow
            }
            None => Outcome::Unauthorized,
        },
        Effect::HasRole(predicate) => {
            match token.and_then(|t| decode_jwt::<E>(t, jwt_config).ok()) {
                Some(claims) => {
                    if predicate(&claims.ext) {
                        extensions.insert(SecurityContext::new(claims));
                        Outcome::Allow
                    } else {
                        Outcome::Forbidden
                    }
                }
                None => Outcome::Unauthorized,
            }
        }
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

impl<S> Layer<S> for AuthorizationLayer
where
    S: Service<Request, Response = Response> + Clone + Send + Sync + 'static,
    S::Error: Send + 'static,
    S::Future: Send + 'static,
{
    type Service = BoxCloneSyncService<Request, Response, S::Error>;

    fn layer(&self, inner: S) -> Self::Service {
        let check = Arc::clone(&self.0);

        BoxCloneSyncService::new(tower::service_fn(move |mut req: Request| {
            let check = Arc::clone(&check);
            let mut inner = inner.clone();

            async move {
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                let token = extract_bearer(req.headers()).map(str::to_string);
                let outcome = check(&method, &path, token, req.extensions_mut());

                match outcome {
                    Outcome::Allow => inner.call(req).await,
                    Outcome::Unauthorized => {
                        Ok(ApiError::from(ServiceError::Unauthorized).into_response())
                    }
                    Outcome::Forbidden => {
                        Ok(ApiError::from(ServiceError::Forbidden).into_response())
                    }
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::AuthorizationBuilder;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, StatusCode};
    use axum::routing::get;
    use serde::{Deserialize, Serialize};
    use stano_security::{Claims, JwtConfig, encode_jwt};
    use tower::util::ServiceExt;

    const PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgtgbDmCbWzH1rPZlb
qucYzcKQppWx4YxRh0TfnEd0wd6hRANCAATbjOo4G431D+jMHWgoGXaW/vr20Qxn
QuoeHrU++Hh7LgqOwXbpqEmKfJa5Os5GQfdQ579fyDqZ/MepnZz2ijhz
-----END PRIVATE KEY-----";

    const PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE24zqOBuN9Q/ozB1oKBl2lv769tEM
Z0LqHh61Pvh4ey4KjsF26ahJinyWuTrORkH3UOe/X8g6mfzHqZ2c9oo4cw==
-----END PUBLIC KEY-----";

    #[derive(Clone, Serialize, Deserialize)]
    struct AppClaims {
        role: String,
    }

    fn jwt_config() -> JwtConfig {
        JwtConfig {
            private_key_pem: PRIVATE_KEY_PEM.to_string(),
            public_key_pem: PUBLIC_KEY_PEM.to_string(),
            expiration_seconds: 3600,
        }
    }

    fn token(role: &str, config: &JwtConfig) -> String {
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
            + 3600;
        let claims = Claims {
            sub: "user-1".to_string(),
            session_id: "session-1".to_string(),
            exp,
            ext: AppClaims {
                role: role.to_string(),
            },
        };
        encode_jwt(&claims, config).unwrap()
    }

    fn app(config: &JwtConfig) -> Router {
        let layer = AuthorizationBuilder::<AppClaims>::new()
            .request_matcher("/public/{*rest}")
            .permit_all()
            .request_matcher("/admin/{*rest}")
            .has_role(|c: &AppClaims| c.role == "ADMIN")
            .any_request()
            .authenticated()
            .build(config.clone())
            .expect("valid chain");

        Router::new()
            .route("/public/x", get(|| async { "ok" }))
            .route("/admin/x", get(|| async { "ok" }))
            .route("/other", get(|| async { "ok" }))
            .layer(layer)
    }

    async fn request(app: &Router, path: &str, bearer: Option<&str>) -> StatusCode {
        let mut builder = axum::http::Request::builder().uri(path).method(Method::GET);
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        response.status()
    }

    #[tokio::test]
    async fn permit_all_allows_without_token() {
        let config = jwt_config();
        let app = app(&config);
        assert_eq!(request(&app, "/public/x", None).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn any_request_rejects_missing_token() {
        let config = jwt_config();
        let app = app(&config);
        assert_eq!(
            request(&app, "/other", None).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn any_request_allows_valid_token() {
        let config = jwt_config();
        let app = app(&config);
        let token = token("USER", &config);
        assert_eq!(request(&app, "/other", Some(&token)).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn has_role_rejects_wrong_role() {
        let config = jwt_config();
        let app = app(&config);
        let token = token("USER", &config);
        assert_eq!(
            request(&app, "/admin/x", Some(&token)).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn has_role_allows_correct_role() {
        let config = jwt_config();
        let app = app(&config);
        let token = token("ADMIN", &config);
        assert_eq!(
            request(&app, "/admin/x", Some(&token)).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn has_role_rejects_missing_token() {
        let config = jwt_config();
        let app = app(&config);
        assert_eq!(
            request(&app, "/admin/x", None).await,
            StatusCode::UNAUTHORIZED
        );
    }
}
