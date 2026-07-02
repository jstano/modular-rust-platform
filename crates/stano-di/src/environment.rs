use dotenvy::dotenv;
#[cfg(test)]
use std::collections::HashMap;

pub trait Environment: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct OsEnvironment;

impl OsEnvironment {
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

#[cfg(test)]
pub struct MockEnvironment {
    vars: HashMap<String, String>,
}

#[cfg(test)]
impl MockEnvironment {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }

    pub fn with_var(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.to_string(), value.to_string());
        self
    }

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
