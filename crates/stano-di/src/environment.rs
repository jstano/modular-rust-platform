use dotenvy::dotenv;
#[cfg(test)]
use std::collections::HashMap;

/// Abstraction over configuration/environment variable lookup, so consumers
/// can swap in a mock implementation in tests instead of reading real env vars.
pub trait Environment: Send + Sync {
    /// Look up the value of an environment/configuration key, if set.
    fn get(&self, key: &str) -> Option<String>;
}

/// [`Environment`] backed by the process's real environment variables.
///
/// Loads a `.env` file (if present) via `dotenvy` on construction, then falls
/// back to `std::env::var` for lookups.
pub struct OsEnvironment;

impl OsEnvironment {
    /// Load `.env` (if present) and construct an environment backed by `std::env::var`.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        dotenv().ok();

        Self {}
    }
}

impl Environment for OsEnvironment {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// In-memory [`Environment`] for tests, built via [`MockEnvironment::with_var`].
#[cfg(test)]
pub struct MockEnvironment {
    vars: HashMap<String, String>,
}

#[cfg(test)]
impl MockEnvironment {
    /// Construct an empty environment with no variables set.
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }

    /// Set a variable, returning `self` for chaining.
    pub fn with_var(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Unset a variable, returning `self` for chaining.
    pub fn remove_var(mut self, key: &str) -> Self {
        self.vars.remove(key);
        self
    }
}

#[cfg(test)]
impl Environment for MockEnvironment {
    fn get(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }
}

#[cfg(test)]
impl Default for MockEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_environment_new_default_empty() {
        let env = MockEnvironment::new();
        assert_eq!(env.get("ANY_KEY"), None);
        let env = MockEnvironment::default();
        assert_eq!(env.get("ANY_KEY"), None);
    }

    #[test]
    fn test_mock_environment_with_var_returns_value() {
        let env = MockEnvironment::new().with_var("FOO", "bar");
        assert_eq!(env.get("FOO"), Some("bar".to_string()));
    }

    #[test]
    fn test_mock_environment_remove_var() {
        let env = MockEnvironment::new()
            .with_var("FOO", "bar")
            .remove_var("FOO");
        assert_eq!(env.get("FOO"), None);
    }

    #[test]
    fn test_os_environment_get_returns_value() {
        unsafe {
            std::env::set_var("STANO_DI_TEST_ENV_VAR_UNIQUE_1", "value1");
        }
        let env = OsEnvironment;
        assert_eq!(
            env.get("STANO_DI_TEST_ENV_VAR_UNIQUE_1"),
            Some("value1".to_string())
        );
        unsafe {
            std::env::remove_var("STANO_DI_TEST_ENV_VAR_UNIQUE_1");
        }
    }

    #[test]
    fn test_os_environment_get_missing_returns_none() {
        let env = OsEnvironment;
        assert_eq!(env.get("STANO_DI_TEST_ENV_VAR_DOES_NOT_EXIST_XYZ"), None);
    }

    #[test]
    fn test_os_environment_new_reads_process_env() {
        unsafe {
            std::env::set_var("STANO_DI_TEST_ENV_VAR_NEW_CTOR", "value2");
        }
        let env = OsEnvironment::new();
        assert_eq!(
            env.get("STANO_DI_TEST_ENV_VAR_NEW_CTOR"),
            Some("value2".to_string())
        );
        unsafe {
            std::env::remove_var("STANO_DI_TEST_ENV_VAR_NEW_CTOR");
        }
    }
}
