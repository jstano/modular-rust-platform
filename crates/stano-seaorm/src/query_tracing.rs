//! Per-query tracing instrumentation, installed automatically by
//! [`crate::DbConfig::from_url`] via `sea_orm`'s `set_metric_callback` hook.

use sea_orm::metric::Info;
use stano_di::environment::Environment;
use std::time::Duration;

const KNOWN_OPERATIONS: &[&str] = &[
    "SELECT", "INSERT", "UPDATE", "DELETE", "BEGIN", "COMMIT", "ROLLBACK",
];

/// Configuration for per-query tracing instrumentation, installed automatically by
/// [`crate::DbConfig::from_url`].
#[derive(Clone, Debug)]
pub struct QueryTracingConfig {
    /// When `false`, no metric callback is installed — zero overhead, matches the
    /// crate's behavior before this instrumentation existed. Defaults to `false`.
    pub enabled: bool,
    /// Whether `db.statement` (the SQL text) is attached to emitted events. Defaults
    /// to `true` — sea-orm's own `statement.sql` is the parameterized query text
    /// (`$1`/`$2` placeholders), not literal parameter values, so this is no more
    /// sensitive than sea-orm/sqlx's existing default query logging.
    pub include_statement: bool,
    /// Queries at or above this duration are logged at `warn` ("slow query") instead
    /// of `debug`. Defaults to 200ms.
    pub slow_query_threshold: Duration,
}

impl Default for QueryTracingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            include_statement: true,
            slow_query_threshold: Duration::from_millis(200),
        }
    }
}

/// Reads [`QueryTracingConfig`] from environment variables, mirroring
/// `stano_launcher::observability::observability_config_from_env`'s pattern: reads
/// `STANO_DB_TRACING_ENABLED`, `STANO_DB_TRACING_INCLUDE_STATEMENT`, and
/// `STANO_DB_SLOW_QUERY_MS`, falling back to [`QueryTracingConfig::default`]'s values
/// for any variable that's unset or unparseable.
pub fn query_tracing_config_from_env(environment: &dyn Environment) -> QueryTracingConfig {
    let defaults = QueryTracingConfig::default();

    QueryTracingConfig {
        enabled: environment
            .get("STANO_DB_TRACING_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(defaults.enabled),
        include_statement: environment
            .get("STANO_DB_TRACING_INCLUDE_STATEMENT")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(defaults.include_statement),
        slow_query_threshold: environment
            .get("STANO_DB_SLOW_QUERY_MS")
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(defaults.slow_query_threshold),
    }
}

/// Parses the first whitespace-delimited token of `sql`, uppercased, restricted to a
/// known allow-list (`SELECT`, `INSERT`, `UPDATE`, `DELETE`, `BEGIN`, `COMMIT`,
/// `ROLLBACK`). Returns `None` for anything else (e.g. CTEs starting with `WITH`).
fn parse_operation(sql: &str) -> Option<&'static str> {
    let first_token = sql.split_whitespace().next()?.to_ascii_uppercase();
    KNOWN_OPERATIONS
        .iter()
        .find(|op| **op == first_token)
        .copied()
}

/// Builds the `set_metric_callback` closure that emits a `stano_seaorm::query` tracing
/// event per SQL statement, once it completes. Emits at `error` when `info.failed`, at
/// `warn` when `info.elapsed >= config.slow_query_threshold`, else `debug` — using
/// tracing level to carry pass/fail/slow status rather than a redundant boolean field.
///
/// Fields: `db.system = "postgresql"` (static — this crate only enables the
/// `sqlx-postgres` sea-orm feature today), `db.operation` (parsed first SQL token,
/// omitted if not recognized), `db.statement` (gated by `include_statement`),
/// `elapsed_ms`.
///
/// Limitation (documented, not fixed here): this is a `tracing::event!`, not a real
/// span — `set_metric_callback` fires only after the query completes, so there is no
/// live await to wrap. If the event happens to fire inside an ambient parent span, it
/// still attaches as a span event via `tracing-opentelemetry`; there is no standalone
/// per-query span the way JDBC auto-instrumentation produces.
pub(crate) fn query_tracing_callback(
    config: QueryTracingConfig,
) -> impl Fn(&Info<'_>) + Send + Sync + 'static {
    move |info: &Info<'_>| {
        let operation = parse_operation(&info.statement.sql).unwrap_or("UNKNOWN");
        let elapsed_ms = info.elapsed.as_secs_f64() * 1000.0;
        let statement = if config.include_statement {
            info.statement.sql.as_str()
        } else {
            ""
        };

        if info.failed {
            tracing::error!(
                target: "stano_seaorm::query",
                elapsed_ms,
                db.system = "postgresql",
                db.operation = operation,
                db.statement = statement,
                "query failed"
            );
        } else if info.elapsed >= config.slow_query_threshold {
            tracing::warn!(
                target: "stano_seaorm::query",
                elapsed_ms,
                db.system = "postgresql",
                db.operation = operation,
                db.statement = statement,
                "slow query"
            );
        } else {
            tracing::debug!(
                target: "stano_seaorm::query",
                elapsed_ms,
                db.system = "postgresql",
                db.operation = operation,
                db.statement = statement,
                "query"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, Statement};
    use std::collections::HashMap;
    use tracing_test::traced_test;

    struct MockEnvironment(HashMap<String, String>);

    impl MockEnvironment {
        fn new() -> Self {
            Self(HashMap::new())
        }

        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.0.insert(key.to_string(), value.to_string());
            self
        }
    }

    impl Environment for MockEnvironment {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn parse_operation_recognizes_select() {
        assert_eq!(parse_operation("SELECT * FROM users"), Some("SELECT"));
    }

    #[test]
    fn parse_operation_is_case_insensitive() {
        assert_eq!(
            parse_operation("insert into users (id) values ($1)"),
            Some("INSERT")
        );
    }

    #[test]
    fn parse_operation_returns_none_for_cte() {
        assert_eq!(
            parse_operation("WITH cte AS (SELECT 1) SELECT * FROM cte"),
            None
        );
    }

    #[test]
    fn parse_operation_returns_none_for_empty_string() {
        assert_eq!(parse_operation(""), None);
    }

    #[test]
    fn config_from_env_defaults_match_default_impl() {
        let env = MockEnvironment::new();
        let config = query_tracing_config_from_env(&env);
        let defaults = QueryTracingConfig::default();
        assert_eq!(config.enabled, defaults.enabled);
        assert_eq!(config.include_statement, defaults.include_statement);
        assert_eq!(config.slow_query_threshold, defaults.slow_query_threshold);
    }

    #[test]
    fn config_from_env_reads_overrides() {
        let env = MockEnvironment::new()
            .with_var("STANO_DB_TRACING_ENABLED", "true")
            .with_var("STANO_DB_TRACING_INCLUDE_STATEMENT", "false")
            .with_var("STANO_DB_SLOW_QUERY_MS", "500");
        let config = query_tracing_config_from_env(&env);
        assert!(config.enabled);
        assert!(!config.include_statement);
        assert_eq!(config.slow_query_threshold, Duration::from_millis(500));
    }

    #[traced_test]
    #[test]
    fn callback_emits_debug_event_for_fast_successful_query() {
        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config);
        let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
        let info = Info {
            elapsed: Duration::from_millis(1),
            statement: &statement,
            failed: false,
        };

        callback(&info);

        assert!(logs_contain("query"));
        assert!(logs_contain("db.operation"));
    }

    #[traced_test]
    #[test]
    fn callback_emits_warn_event_for_slow_query() {
        let config = QueryTracingConfig {
            slow_query_threshold: Duration::from_millis(50),
            ..QueryTracingConfig::default()
        };
        let callback = query_tracing_callback(config);
        let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
        let info = Info {
            elapsed: Duration::from_millis(100),
            statement: &statement,
            failed: false,
        };

        callback(&info);

        assert!(logs_contain("slow query"));
    }

    #[traced_test]
    #[test]
    fn callback_emits_error_event_for_failed_query() {
        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config);
        let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
        let info = Info {
            elapsed: Duration::from_millis(1),
            statement: &statement,
            failed: true,
        };

        callback(&info);

        assert!(logs_contain("query failed"));
    }

    #[traced_test]
    #[test]
    fn callback_omits_statement_when_include_statement_is_false() {
        let config = QueryTracingConfig {
            include_statement: false,
            ..QueryTracingConfig::default()
        };
        let callback = query_tracing_callback(config);
        let statement = Statement::from_string(
            DbBackend::Postgres,
            "SELECT secret FROM accounts".to_string(),
        );
        let info = Info {
            elapsed: Duration::from_millis(1),
            statement: &statement,
            failed: false,
        };

        callback(&info);

        assert!(!logs_contain("secret"));
    }
}
