/// A single request-path pattern compiled via `matchit`, reusing axum/utoipa_axum's own
/// path syntax (`{name}` segment params, `{*name}` catch-all) for consistency with
/// `#[route(path = "...")]`.
pub struct CompiledPattern(matchit::Router<()>);

impl CompiledPattern {
    /// Compile a single path pattern. Fails if `pattern` is not valid `matchit` syntax.
    pub fn new(pattern: &str) -> Result<Self, matchit::InsertError> {
        let mut router = matchit::Router::new();
        router.insert(pattern, ())?;
        Ok(Self(router))
    }

    /// Whether `path` matches this pattern.
    pub fn matches(&self, path: &str) -> bool {
        self.0.at(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_path() {
        let pattern = CompiledPattern::new("/public").unwrap();
        assert!(pattern.matches("/public"));
        assert!(!pattern.matches("/private"));
    }

    #[test]
    fn matches_named_segment() {
        let pattern = CompiledPattern::new("/users/{id}").unwrap();
        assert!(pattern.matches("/users/42"));
        assert!(!pattern.matches("/users"));
        assert!(!pattern.matches("/users/42/extra"));
    }

    #[test]
    fn matches_catch_all() {
        let pattern = CompiledPattern::new("/admin/{*rest}").unwrap();
        assert!(pattern.matches("/admin/x"));
        assert!(pattern.matches("/admin/x/y/z"));
        assert!(!pattern.matches("/admin"));
    }

    #[test]
    fn invalid_pattern_returns_err() {
        assert!(CompiledPattern::new("/bad/{").is_err());
    }
}
