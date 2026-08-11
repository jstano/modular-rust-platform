use super::authorization::{ClaimsValidator, Effect, Rule};
use crate::error::ApiError;
use axum::extract::Request;
use axum::http::{HeaderMap, Method};
use axum::response::{IntoResponse, Response};
use opentelemetry::{KeyValue, global, metrics::Counter};
use stano_common::ServiceError;
use stano_security::{JwtConfig, SecurityContext, decode_jwt};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};

/// The finalized, type-erased result of [`super::AuthorizationBuilder::build`], applied via
/// `.layer(...)` (or passed into `stano_launcher::run(...)`). Carries no generic over the
/// app's claims extension type — that was resolved when `.build()` was called.
#[derive(Clone)]
pub struct AuthorizationLayer {
    check: Arc<dyn ErasedCheck>,
    cookie_name: Option<String>,
    /// `http.server.auth.failures` OTel counter, incremented once per `401`/`403`
    /// produced by this layer (never on `Allow`). Resolved lazily, on first actual
    /// rejection, not eagerly in [`build_layer`] — `opentelemetry::global::meter(...)`
    /// permanently binds to whichever `MeterProvider` is installed *at the moment it's
    /// called*, and does not retroactively pick up a real provider installed later via
    /// `set_meter_provider`. Apps commonly call `.build(jwt_config)` (which calls
    /// `build_layer`) during route/middleware wiring, before `stano_launcher::run()`
    /// (which is what installs the real provider) — so resolving eagerly here would
    /// silently bind this counter to a no-op provider forever. `Arc<OnceLock<...>>`
    /// (rather than a bare `OnceLock`) so every `Clone` of this layer — e.g. once per
    /// `tower::Layer::layer` call — shares the same lazily-initialized counter instead
    /// of each independently registering its own.
    failure_counter: Arc<OnceLock<Counter<u64>>>,
}

enum Outcome {
    Allow,
    Unauthorized,
    Forbidden,
}

/// Type-erases the rule table over `E`, exposed as a trait object so [`AuthorizationLayer`]
/// itself carries no generic. A trait method (rather than a `dyn Fn` closure) is used here
/// because closures don't reliably coerce to a higher-ranked `for<'a> Fn(...) -> Pin<Box<dyn
/// Future + 'a>>` bound — an explicit `fn check<'a>(&'a self, ...)` signature does.
trait ErasedCheck: Send + Sync {
    fn check<'a>(
        &'a self,
        method: &'a Method,
        path: &'a str,
        token: Option<String>,
        extensions: &'a mut axum::http::Extensions,
    ) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;
}

struct CheckState<E> {
    rules: Vec<Rule<E>>,
    any_request: Effect<E>,
    jwt_config: JwtConfig,
    claims_validator: Option<ClaimsValidator<E>>,
}

impl<E> ErasedCheck for CheckState<E>
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    fn check<'a>(
        &'a self,
        method: &'a Method,
        path: &'a str,
        token: Option<String>,
        extensions: &'a mut axum::http::Extensions,
    ) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
        let effect = self
            .rules
            .iter()
            .find(|rule| {
                rule.methods
                    .as_ref()
                    .map(|methods| methods.contains(method))
                    .unwrap_or(true)
                    && rule.pattern.matches(path)
            })
            .map(|rule| rule.effect.clone())
            .unwrap_or_else(|| self.any_request.clone());

        Box::pin(apply_effect(
            effect,
            token,
            &self.jwt_config,
            self.claims_validator.as_ref(),
            extensions,
        ))
    }
}

pub(super) fn build_layer<E>(
    rules: Vec<Rule<E>>,
    any_request: Effect<E>,
    jwt_config: JwtConfig,
    claims_validator: Option<ClaimsValidator<E>>,
    cookie_name: Option<String>,
) -> AuthorizationLayer
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    AuthorizationLayer {
        check: Arc::new(CheckState {
            rules,
            any_request,
            jwt_config,
            claims_validator,
        }),
        cookie_name,
        failure_counter: Arc::new(OnceLock::new()),
    }
}

async fn apply_effect<E>(
    effect: Effect<E>,
    token: Option<String>,
    jwt_config: &JwtConfig,
    claims_validator: Option<&ClaimsValidator<E>>,
    extensions: &mut axum::http::Extensions,
) -> Outcome
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    match effect {
        Effect::PermitAll => {
            if let Some(token) = token.as_deref()
                && let Ok(claims) = decode_jwt::<E>(token, jwt_config)
            {
                let valid = match claims_validator {
                    Some(validator) => validator(&claims).await.is_ok(),
                    None => true,
                };
                if valid {
                    extensions.insert(SecurityContext::new(claims));
                }
            }
            Outcome::Allow
        }
        Effect::Authenticated => {
            match token
                .as_deref()
                .and_then(|t| decode_jwt::<E>(t, jwt_config).ok())
            {
                Some(claims) => {
                    if let Some(validator) = claims_validator
                        && validator(&claims).await.is_err()
                    {
                        return Outcome::Unauthorized;
                    }
                    extensions.insert(SecurityContext::new(claims));
                    Outcome::Allow
                }
                None => Outcome::Unauthorized,
            }
        }
        Effect::HasRole(predicate) => {
            match token
                .as_deref()
                .and_then(|t| decode_jwt::<E>(t, jwt_config).ok())
            {
                Some(claims) => {
                    if let Some(validator) = claims_validator
                        && validator(&claims).await.is_err()
                    {
                        return Outcome::Unauthorized;
                    }
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

/// Extracts the bearer token from the `Authorization` header, falling back to the named
/// cookie (if configured) when no such header is present.
fn extract_token(headers: &HeaderMap, cookie_name: Option<&str>) -> Option<String> {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        return Some(token.to_string());
    }

    let cookie_name = cookie_name?;
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    cookie_header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == cookie_name).then(|| value.to_string())
    })
}

impl<S> Layer<S> for AuthorizationLayer
where
    S: Service<Request, Response = Response> + Clone + Send + Sync + 'static,
    S::Error: Send + 'static,
    S::Future: Send + 'static,
{
    type Service = BoxCloneSyncService<Request, Response, S::Error>;

    fn layer(&self, inner: S) -> Self::Service {
        let check = Arc::clone(&self.check);
        let cookie_name = self.cookie_name.clone();
        let failure_counter = self.failure_counter.clone();

        BoxCloneSyncService::new(tower::service_fn(move |mut req: Request| {
            let check = Arc::clone(&check);
            let cookie_name = cookie_name.clone();
            let failure_counter = failure_counter.clone();
            let mut inner = inner.clone();

            async move {
                let method = req.method().clone();
                let path = req.uri().path().to_string();
                let token = extract_token(req.headers(), cookie_name.as_deref());
                let outcome = check
                    .check(&method, &path, token, req.extensions_mut())
                    .await;

                let failure_status = match outcome {
                    Outcome::Allow => None,
                    Outcome::Unauthorized => Some(401),
                    Outcome::Forbidden => Some(403),
                };

                if let Some(status) = failure_status {
                    // Resolved lazily on first rejection — see the doc comment on
                    // `AuthorizationLayer::failure_counter` for why this can't be built
                    // eagerly in `build_layer`.
                    let counter = failure_counter.get_or_init(|| {
                        global::meter("stano-axum")
                            .u64_counter("http.server.auth.failures")
                            .with_description(
                                "Count of requests rejected by AuthorizationLayer, by status",
                            )
                            .build()
                    });
                    counter.add(
                        1,
                        &[
                            KeyValue::new("http.request.method", method.to_string()),
                            KeyValue::new("http.response.status_code", status),
                        ],
                    );
                }

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

    fn app_with_validator(
        config: &JwtConfig,
        validator: impl Fn(&Claims<AppClaims>) -> bool + Send + Sync + 'static,
    ) -> Router {
        let layer = AuthorizationBuilder::<AppClaims>::new()
            .request_matcher("/public/{*rest}")
            .permit_all()
            .request_matcher("/admin/{*rest}")
            .has_role(|c: &AppClaims| c.role == "ADMIN")
            .any_request()
            .authenticated()
            .with_claims_validator(move |claims: &Claims<AppClaims>| {
                let ok = validator(claims);
                async move {
                    if ok {
                        Ok(())
                    } else {
                        Err("rejected".to_string())
                    }
                }
            })
            .build(config.clone())
            .expect("valid chain");

        Router::new()
            .route("/public/x", get(|| async { "ok" }))
            .route("/admin/x", get(|| async { "ok" }))
            .route("/other", get(|| async { "ok" }))
            .layer(layer)
    }

    #[tokio::test]
    async fn claims_validator_ok_allows_authenticated_request() {
        let config = jwt_config();
        let app = app_with_validator(&config, |_| true);
        let token = token("USER", &config);
        assert_eq!(request(&app, "/other", Some(&token)).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn claims_validator_err_rejects_authenticated_request() {
        let config = jwt_config();
        let app = app_with_validator(&config, |_| false);
        let token = token("USER", &config);
        assert_eq!(
            request(&app, "/other", Some(&token)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn claims_validator_err_rejects_has_role_request_as_unauthorized() {
        let config = jwt_config();
        let app = app_with_validator(&config, |_| false);
        let token = token("ADMIN", &config);
        // A revoked/invalid session is an authentication failure, not an authorization one —
        // even though the role predicate would have passed, this must be 401, not 403.
        assert_eq!(
            request(&app, "/admin/x", Some(&token)).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn claims_validator_err_on_permit_all_still_allows_without_context() {
        let config = jwt_config();
        let app = app_with_validator(&config, |_| false);
        let token = token("USER", &config);
        // permit_all() never requires a token to begin with — a validator rejection on a
        // present-but-untrusted token degrades to "no context", not a hard failure.
        assert_eq!(
            request(&app, "/public/x", Some(&token)).await,
            StatusCode::OK
        );
    }

    fn app_with_cookie(config: &JwtConfig) -> Router {
        let layer = AuthorizationBuilder::<AppClaims>::new()
            .request_matcher("/public/{*rest}")
            .permit_all()
            .any_request()
            .authenticated()
            .cookie_name("jwt_token")
            .build(config.clone())
            .expect("valid chain");

        Router::new()
            .route("/other", get(|| async { "ok" }))
            .layer(layer)
    }

    async fn request_with_cookie(app: &Router, path: &str, cookie: Option<&str>) -> StatusCode {
        let mut builder = axum::http::Request::builder().uri(path).method(Method::GET);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        response.status()
    }

    #[tokio::test]
    async fn cookie_fallback_allows_authenticated_request_without_header() {
        let config = jwt_config();
        let app = app_with_cookie(&config);
        let token = token("USER", &config);
        assert_eq!(
            request_with_cookie(&app, "/other", Some(&format!("jwt_token={token}"))).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn cookie_fallback_ignores_unrelated_cookies() {
        let config = jwt_config();
        let app = app_with_cookie(&config);
        assert_eq!(
            request_with_cookie(&app, "/other", Some("other=value")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn no_cookie_name_configured_is_header_only() {
        let config = jwt_config();
        // Reuses `app`, which never calls `.cookie_name(...)` — confirms backward
        // compatibility for consumers that don't opt in to cookie extraction.
        let app = app(&config);
        let token = token("USER", &config);
        assert_eq!(
            request_with_cookie(&app, "/other", Some(&format!("jwt_token={token}"))).await,
            StatusCode::UNAUTHORIZED
        );
    }
}
