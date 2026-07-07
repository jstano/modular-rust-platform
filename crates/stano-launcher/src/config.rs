use crate::observability::ObservabilityConfig;
use stano_di::environment::Environment;
use stano_security::JwtConfig;

/// Server startup settings passed to [`crate::run`].
#[derive(Clone, Debug)]
pub struct BootstrapConfig {
    /// TCP port to listen on.
    pub port: u16,
    /// JWT private/public keys and optional expiration duration.
    pub jwt_config: JwtConfig,
    /// Allowed CORS origins, exact match. Empty (together with `cors_origin_suffixes`) means
    /// permissive CORS (allow any origin). Used when `is_dev` is false, or when `is_dev` is
    /// true but `cors_dev_origins` is empty.
    pub cors_origins: Vec<String>,
    /// Allowed CORS origin suffixes, e.g. `.example.com` matches any subdomain. Empty
    /// (together with `cors_origins`) means permissive CORS. Same applicability as
    /// `cors_origins`.
    pub cors_origin_suffixes: Vec<String>,
    /// Exact origins allowed when `is_dev` is true, e.g. localhost dev-server ports.
    /// Ignored when `is_dev` is false. Falls back to `cors_origins`/`cors_origin_suffixes`
    /// matching if empty.
    pub cors_dev_origins: Vec<String>,
    /// When true, CORS uses `cors_dev_origins` instead of `cors_origins`/`cors_origin_suffixes`.
    /// Compute with `is_dev_environment`, or set explicitly.
    pub is_dev: bool,
    /// OTLP tracing/metrics/log export settings. [`crate::run`] initializes observability
    /// from this before doing anything else. Build with
    /// [`crate::observability::observability_config_from_env`], or construct explicitly.
    pub observability: ObservabilityConfig,
}

/// True when running a debug build, or when `RUST_ENV=development` (case-insensitive).
/// Note: debug builds (including `cargo test`) are always considered dev mode.
pub fn is_dev_environment(environment: &dyn Environment) -> bool {
    cfg!(debug_assertions)
        || environment
            .get("RUST_ENV")
            .unwrap_or_default()
            .eq_ignore_ascii_case("development")
}

/// Parses a comma-separated env var into a trimmed, non-empty `Vec<String>`.
/// Returns an empty vec if the var is unset.
pub fn parse_csv_env(environment: &dyn Environment, key: &str) -> Vec<String> {
    environment
        .get(key)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapEnvironment(HashMap<String, String>);

    impl Environment for MapEnvironment {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    fn env_with(key: &str, value: &str) -> MapEnvironment {
        let mut map = HashMap::new();
        map.insert(key.to_string(), value.to_string());
        MapEnvironment(map)
    }

    #[test]
    fn parse_csv_env_splits_and_trims() {
        let env = env_with("ORIGINS", " http://a.com , http://b.com ,http://c.com");
        assert_eq!(
            parse_csv_env(&env, "ORIGINS"),
            vec!["http://a.com", "http://b.com", "http://c.com"]
        );
    }

    #[test]
    fn parse_csv_env_drops_empty_entries() {
        let env = env_with("ORIGINS", "http://a.com,,  ,http://b.com");
        assert_eq!(
            parse_csv_env(&env, "ORIGINS"),
            vec!["http://a.com", "http://b.com"]
        );
    }

    #[test]
    fn parse_csv_env_missing_var_returns_empty() {
        let env = MapEnvironment(HashMap::new());
        assert!(parse_csv_env(&env, "ORIGINS").is_empty());
    }

    #[test]
    fn is_dev_environment_true_when_rust_env_development() {
        let env = env_with("RUST_ENV", "development");
        assert!(is_dev_environment(&env));
    }

    #[test]
    fn is_dev_environment_true_when_rust_env_development_mixed_case() {
        let env = env_with("RUST_ENV", "Development");
        assert!(is_dev_environment(&env));
    }

    #[test]
    fn is_dev_environment_true_in_debug_builds_regardless_of_rust_env() {
        // cfg!(debug_assertions) is true for `cargo test`, so this is always dev
        // regardless of RUST_ENV — matches the convention this mirrors.
        let env = env_with("RUST_ENV", "production");
        assert!(is_dev_environment(&env));
    }
}
