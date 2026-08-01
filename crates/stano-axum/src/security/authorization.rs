use super::layer::AuthorizationLayer;
use super::pattern::CompiledPattern;
use axum::http::Method;
use stano_security::JwtConfig;
use std::sync::Arc;

/// The effect applied when a request matches a configured rule.
pub(super) enum Effect<E> {
    /// No authentication required.
    PermitAll,
    /// A valid JWT (any claims) is required.
    Authenticated,
    /// A valid JWT is required, and its decoded claims extension must satisfy the predicate.
    HasRole(Arc<dyn Fn(&E) -> bool + Send + Sync>),
}

impl<E> Clone for Effect<E> {
    fn clone(&self) -> Self {
        match self {
            Effect::PermitAll => Effect::PermitAll,
            Effect::Authenticated => Effect::Authenticated,
            Effect::HasRole(pred) => Effect::HasRole(Arc::clone(pred)),
        }
    }
}

pub(super) struct Rule<E> {
    pub(super) methods: Option<Vec<Method>>,
    pub(super) pattern: CompiledPattern,
    pub(super) effect: Effect<E>,
}

/// Errors returned by [`AuthorizationBuilder::build`].
#[derive(Debug)]
pub enum AuthorizationBuildError {
    /// `.build()` was called without a terminal `.any_request()` rule configured.
    MissingAnyRequest,
    /// A `.request_matcher(...)` pattern was not valid `matchit` syntax.
    InvalidPattern(matchit::InsertError),
}

impl std::fmt::Display for AuthorizationBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorizationBuildError::MissingAnyRequest => {
                write!(
                    f,
                    "authorization chain must end with a terminal any_request() rule"
                )
            }
            AuthorizationBuildError::InvalidPattern(err) => {
                write!(f, "invalid request matcher pattern: {err}")
            }
        }
    }
}

impl std::error::Error for AuthorizationBuildError {}

/// Builds a Spring-Security-style, ordered request-authorization chain, mirroring
/// `HttpSecurity::authorizeHttpRequests(...)`:
///
/// ```ignore
/// let layer = AuthorizationBuilder::<AppClaims>::new()
///     .request_matcher("/public/{*rest}").permit_all()
///     .request_matcher("/admin/{*rest}").has_role(|c: &AppClaims| c.role == "ADMIN")
///     .any_request().authenticated()
///     .build(jwt_config)?;
/// ```
///
/// Rules are evaluated in registration order; the first rule whose method(s) and path
/// pattern match the incoming request wins. `.build()` fails if no terminal
/// `.any_request()` rule was configured.
pub struct AuthorizationBuilder<E> {
    pub(super) rules: Vec<Rule<E>>,
    pub(super) any_request: Option<Effect<E>>,
    poisoned: Option<matchit::InsertError>,
}

impl<E> Default for AuthorizationBuilder<E> {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            any_request: None,
            poisoned: None,
        }
    }
}

impl<E> AuthorizationBuilder<E>
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Start building a new authorization chain.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a rule for requests whose path matches `pattern` (matchit syntax:
    /// `{name}` segment, `{*name}` catch-all). Applies to any HTTP method unless
    /// narrowed via [`RequestMatcherBuilder::methods`].
    pub fn request_matcher(self, pattern: &str) -> RequestMatcherBuilder<E> {
        RequestMatcherBuilder {
            parent: self,
            methods: None,
            pattern: PatternSpec::Pattern(pattern.to_string()),
        }
    }

    /// Register the terminal catch-all rule applied when no other rule matched.
    /// Mirrors Spring's `.anyRequest()`. Mandatory — [`Self::build`] errors without it.
    pub fn any_request(self) -> RequestMatcherBuilder<E> {
        RequestMatcherBuilder {
            parent: self,
            methods: None,
            pattern: PatternSpec::AnyRequest,
        }
    }

    /// Finalize the chain into a type-erased [`AuthorizationLayer`], ready to pass into
    /// `stano_launcher::run(...)`.
    pub fn build(
        self,
        jwt_config: JwtConfig,
    ) -> Result<AuthorizationLayer, AuthorizationBuildError> {
        if let Some(err) = self.poisoned {
            return Err(AuthorizationBuildError::InvalidPattern(err));
        }
        let any_request = self
            .any_request
            .ok_or(AuthorizationBuildError::MissingAnyRequest)?;

        Ok(super::layer::build_layer(
            self.rules,
            any_request,
            jwt_config,
        ))
    }
}

enum PatternSpec {
    Pattern(String),
    AnyRequest,
}

/// Builder for a single rule's effect, returned by [`AuthorizationBuilder::request_matcher`]
/// / [`AuthorizationBuilder::any_request`]. Terminates back into [`AuthorizationBuilder`]
/// via [`Self::permit_all`], [`Self::authenticated`], [`Self::has_role`], or
/// [`Self::has_any_role`].
pub struct RequestMatcherBuilder<E> {
    parent: AuthorizationBuilder<E>,
    methods: Option<Vec<Method>>,
    pattern: PatternSpec,
}

impl<E> RequestMatcherBuilder<E>
where
    E: serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// Restrict this rule to the given HTTP methods (default: any method).
    pub fn methods(mut self, methods: impl IntoIterator<Item = Method>) -> Self {
        self.methods = Some(methods.into_iter().collect());
        self
    }

    /// No authentication required for matching requests.
    pub fn permit_all(self) -> AuthorizationBuilder<E> {
        self.finish(Effect::PermitAll)
    }

    /// A valid JWT is required for matching requests.
    pub fn authenticated(self) -> AuthorizationBuilder<E> {
        self.finish(Effect::Authenticated)
    }

    /// A valid JWT is required, and the decoded claims extension must satisfy `predicate`.
    pub fn has_role(
        self,
        predicate: impl Fn(&E) -> bool + Send + Sync + 'static,
    ) -> AuthorizationBuilder<E> {
        self.finish(Effect::HasRole(Arc::new(predicate)))
    }

    /// A valid JWT is required, and the decoded claims extension must satisfy at least one
    /// of `predicates`.
    pub fn has_any_role<F>(self, predicates: impl IntoIterator<Item = F>) -> AuthorizationBuilder<E>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        let predicates: Vec<F> = predicates.into_iter().collect();
        self.finish(Effect::HasRole(Arc::new(move |ext: &E| {
            predicates.iter().any(|p| p(ext))
        })))
    }

    fn finish(self, effect: Effect<E>) -> AuthorizationBuilder<E> {
        let mut parent = self.parent;
        match self.pattern {
            PatternSpec::AnyRequest => {
                parent.any_request = Some(effect);
            }
            PatternSpec::Pattern(pattern) => {
                // Deferred to `build()` for reporting via `AuthorizationBuildError`, but a
                // pattern is compiled eagerly here so later rules can't shadow a bad one
                // silently; invalid patterns are carried forward as a poison rule that
                // `build()` rejects.
                match CompiledPattern::new(&pattern) {
                    Ok(compiled) => parent.rules.push(Rule {
                        methods: self.methods,
                        pattern: compiled,
                        effect,
                    }),
                    Err(err) => parent.poison(err),
                }
            }
        }
        parent
    }
}

impl<E> AuthorizationBuilder<E> {
    fn poison(&mut self, err: matchit::InsertError) {
        self.poisoned.get_or_insert(err);
    }
}
