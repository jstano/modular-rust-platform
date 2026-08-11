//! Per-query tracing instrumentation, installed automatically by
//! [`crate::DbConfig::from_url`] via `sea_orm`'s `set_metric_callback` hook.

use opentelemetry::metrics::Histogram;
use opentelemetry::trace::{Span, SpanBuilder, SpanKind, Status, Tracer};
use opentelemetry::{KeyValue, global, global::BoxedTracer};
use sea_orm::metric::Info;
use sea_orm::sqlx::PgPool;
use stano_di::environment::Environment;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_opentelemetry::OpenTelemetrySpanExt;

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
/// `MRP_DB_TRACING_ENABLED`, `MRP_DB_TRACING_INCLUDE_STATEMENT`, and
/// `MRP_DB_SLOW_QUERY_MS`, falling back to [`QueryTracingConfig::default`]'s values
/// for any variable that's unset or unparseable.
pub fn query_tracing_config_from_env(environment: &dyn Environment) -> QueryTracingConfig {
    let defaults = QueryTracingConfig::default();

    QueryTracingConfig {
        enabled: environment
            .get("MRP_DB_TRACING_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(defaults.enabled),
        include_statement: environment
            .get("MRP_DB_TRACING_INCLUDE_STATEMENT")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(defaults.include_statement),
        slow_query_threshold: environment
            .get("MRP_DB_SLOW_QUERY_MS")
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
/// event per SQL statement, once it completes, records its duration on a
/// `db.client.operation.duration` OTel histogram (seconds, per the DB semantic
/// conventions naming), and synthesizes a backdated per-query OTel span (see below).
/// Also registers a `db.client.connection.count` observable gauge (attribute
/// `db.client.connection.state` = `"idle"`/`"used"`) against `pool`, reporting live pool
/// utilization on every metrics collection. Emits the tracing event at `error` when
/// `info.failed`, at `warn` when `info.elapsed >= config.slow_query_threshold`, else
/// `debug` — using tracing level to carry pass/fail/slow status rather than a redundant
/// boolean field. The histogram records unconditionally, tagged with the same
/// `db.system`/`db.operation` attributes as the event (never `db.statement` — too
/// high-cardinality for a metric attribute). All four (event, histogram, gauge, span)
/// are installed together — there's no separate opt-out for just one of them; see
/// [`QueryTracingConfig::enabled`].
///
/// Recorded via `opentelemetry::global::meter(...)`/`global::tracer(...)`, the same
/// global meter/tracer providers `stano_launcher::observability::init_observability`
/// installs — so these are exported automatically whenever OTLP export is on, with no
/// extra wiring needed here. **Deliberately deferred to first-query time**, not resolved
/// eagerly when this function itself runs (at `DbConfig::from_url` setup time):
/// `opentelemetry::global` documents that a `Meter`/`Tracer` obtained via
/// `global::meter(...)`/`global::tracer(...)` stays permanently bound to whichever
/// provider was installed *at that moment* — it does not retroactively pick up a real
/// provider installed later via `set_meter_provider`/`set_tracer_provider`.
/// `DbConfig::from_url` commonly runs during app component wiring, before
/// `stano_launcher::run()` (and its internal call to `init_observability`, which is what
/// calls `set_meter_provider`/`set_tracer_provider`) — so resolving these at DB-setup
/// time would silently bind these instruments to the no-op providers forever, and
/// they'd never export. Deferring to first-query time (via the `OnceLock` below) is safe
/// because request handling — and therefore any query — only begins after `run()` has
/// already initialized observability.
///
/// Fields: `db.system = "postgresql"` (static — this crate only enables the
/// `sqlx-postgres` sea-orm feature today), `db.operation` (parsed first SQL token,
/// omitted if not recognized), `db.statement` (gated by `include_statement`),
/// `elapsed_ms`.
///
/// ## Span synthesis
///
/// `set_metric_callback` fires only after the query completes (there's no "query
/// started" hook to open a live span around), but it fires synchronously, in-line,
/// immediately after the awaited query resolves, on the same task — so
/// `tracing::Span::current()` at callback time still correctly reflects whatever span
/// was active around the `.exec()`/`.one()`/etc. call (e.g. the Axum request span from
/// `TraceLayer`). This is exploited to synthesize a *backdated* span rather than a mere
/// event: `SpanBuilder::with_start_time(now - info.elapsed)` plus
/// `Span::end_with_timestamp(now)` produce a span with the query's real start/end times,
/// parented via `tracing::Span::current().context()` — so it appears as a genuine child
/// span (with its own duration bar) nested under the ambient span, the way JDBC
/// auto-instrumentation would produce, without needing a live `.enter()`/drop across the
/// actual await.
pub(crate) fn query_tracing_callback(
    config: QueryTracingConfig,
    pool: PgPool,
) -> impl Fn(&Info<'_>) + Send + Sync + 'static {
    struct Instruments {
        duration_histogram: Histogram<f64>,
        tracer: BoxedTracer,
    }

    let instruments: OnceLock<Instruments> = OnceLock::new();

    move |info: &Info<'_>| {
        let instruments = instruments.get_or_init(|| {
            let meter = global::meter("stano-seaorm");

            // Registration (not just the returned handle) is permanent for the process
            // lifetime — see this function's doc comment for why the `_gauge` binding
            // is deliberately discarded, not held anywhere.
            let pool = pool.clone();
            let _gauge = meter
                .u64_observable_gauge("db.client.connection.count")
                .with_description("Number of connections in the pool, by state")
                .with_callback(move |observer| {
                    let idle = pool.num_idle() as u64;
                    let used = u64::from(pool.size()).saturating_sub(idle);
                    observer.observe(idle, &[KeyValue::new("db.client.connection.state", "idle")]);
                    observer.observe(used, &[KeyValue::new("db.client.connection.state", "used")]);
                })
                .build();

            let duration_histogram = meter
                .f64_histogram("db.client.operation.duration")
                .with_unit("s")
                .with_description("Duration of database operations, in seconds")
                .build();

            Instruments {
                duration_histogram,
                tracer: global::tracer("stano-seaorm"),
            }
        });

        let operation = parse_operation(&info.statement.sql).unwrap_or("UNKNOWN");
        let elapsed_ms = info.elapsed.as_secs_f64() * 1000.0;
        let statement = if config.include_statement {
            info.statement.sql.as_str()
        } else {
            ""
        };

        instruments.duration_histogram.record(
            info.elapsed.as_secs_f64(),
            &[
                KeyValue::new("db.system", "postgresql"),
                KeyValue::new("db.operation", operation),
            ],
        );

        let mut attributes = vec![
            KeyValue::new("db.system", "postgresql"),
            KeyValue::new("db.operation", operation),
        ];
        if config.include_statement {
            attributes.push(KeyValue::new("db.statement", statement.to_string()));
        }

        let end_time = SystemTime::now();
        let start_time = end_time.checked_sub(info.elapsed).unwrap_or(end_time);
        let parent_cx = tracing::Span::current().context();
        let mut span = instruments.tracer.build_with_context(
            SpanBuilder::from_name(operation.to_string())
                .with_kind(SpanKind::Client)
                .with_start_time(start_time)
                .with_attributes(attributes),
            &parent_cx,
        );
        if info.failed {
            span.set_status(Status::error("query failed"));
        }
        span.end_with_timestamp(end_time);

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

    /// A `PgPool` that never actually connects — `connect_lazy` just parses the URL and
    /// defers real connection attempts to first use, which these tests never trigger
    /// (they only exercise the tracing-event path, not an actual metrics collection).
    fn test_pool() -> PgPool {
        PgPool::connect_lazy("postgres://localhost/test").expect("valid lazy pool")
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
            .with_var("MRP_DB_TRACING_ENABLED", "true")
            .with_var("MRP_DB_TRACING_INCLUDE_STATEMENT", "false")
            .with_var("MRP_DB_SLOW_QUERY_MS", "500");
        let config = query_tracing_config_from_env(&env);
        assert!(config.enabled);
        assert!(!config.include_statement);
        assert_eq!(config.slow_query_threshold, Duration::from_millis(500));
    }

    #[traced_test]
    #[tokio::test]
    async fn callback_emits_debug_event_for_fast_successful_query() {
        // Every callback invocation also touches the global OTel tracer (see "Span
        // synthesis" tests below), so this is serialized against them too, even
        // though this test itself only asserts on the emitted event.
        let _lock = lock_tracer();
        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config, test_pool());
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
    #[tokio::test]
    async fn callback_emits_warn_event_for_slow_query() {
        let _lock = lock_tracer();
        let config = QueryTracingConfig {
            slow_query_threshold: Duration::from_millis(50),
            ..QueryTracingConfig::default()
        };
        let callback = query_tracing_callback(config, test_pool());
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
    #[tokio::test]
    async fn callback_emits_error_event_for_failed_query() {
        let _lock = lock_tracer();
        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config, test_pool());
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
    #[tokio::test]
    async fn callback_omits_statement_when_include_statement_is_false() {
        let _lock = lock_tracer();
        let config = QueryTracingConfig {
            include_statement: false,
            ..QueryTracingConfig::default()
        };
        let callback = query_tracing_callback(config, test_pool());
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

    // Span-synthesis tests below (and every other test above that invokes
    // `query_tracing_callback`'s closure) install/touch a real `SdkTracerProvider` via
    // `opentelemetry::global::set_tracer_provider(...)`/`global::tracer(...)`, which —
    // like `stano_launcher::observability`'s tests around
    // `OTEL_EXPORTER_OTLP_ENDPOINT` — mutates process-global state, so all of them are
    // serialized behind this lock to avoid one test's spans leaking into another's
    // exporter.
    use opentelemetry::trace::{SpanId, TraceContextExt, TracerProvider as _};
    use opentelemetry_sdk::trace::{
        InMemorySpanExporter, InMemorySpanExporterBuilder, SdkTracerProvider,
    };
    use tracing_subscriber::layer::SubscriberExt;

    static TRACER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_tracer() -> std::sync::MutexGuard<'static, ()> {
        TRACER_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Installs an in-memory tracer provider as the global provider (via
    /// `SimpleSpanProcessor`, so spans are exported synchronously on `end()`, with no
    /// batching delay to wait out) and returns the exporter to assert against.
    fn install_in_memory_tracer() -> InMemorySpanExporter {
        let exporter = InMemorySpanExporterBuilder::new().build();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider);
        exporter
    }

    #[tokio::test]
    async fn callback_creates_a_real_span_not_just_an_event() {
        let _lock = lock_tracer();
        let exporter = install_in_memory_tracer();

        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config, test_pool());
        let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
        let info = Info {
            elapsed: Duration::from_millis(5),
            statement: &statement,
            failed: false,
        };

        callback(&info);

        let spans = exporter.get_finished_spans().expect("finished spans");
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.name, "SELECT");
        assert_eq!(span.span_kind, SpanKind::Client);
        assert!(
            span.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "db.system" && kv.value.as_str() == "postgresql")
        );
        assert!(
            span.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "db.operation" && kv.value.as_str() == "SELECT")
        );
        assert!(
            span.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "db.statement")
        );
        assert_eq!(span.status, Status::Unset);

        let duration = span
            .end_time
            .duration_since(span.start_time)
            .expect("end_time after start_time");
        assert!(duration >= Duration::from_millis(5));
        assert!(duration < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn callback_span_omits_statement_when_include_statement_is_false() {
        let _lock = lock_tracer();
        let exporter = install_in_memory_tracer();

        let config = QueryTracingConfig {
            include_statement: false,
            ..QueryTracingConfig::default()
        };
        let callback = query_tracing_callback(config, test_pool());
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

        let spans = exporter.get_finished_spans().expect("finished spans");
        assert!(
            !spans[0]
                .attributes
                .iter()
                .any(|kv| kv.key.as_str() == "db.statement")
        );
    }

    #[tokio::test]
    async fn callback_span_has_error_status_for_failed_query() {
        let _lock = lock_tracer();
        let exporter = install_in_memory_tracer();

        let config = QueryTracingConfig::default();
        let callback = query_tracing_callback(config, test_pool());
        let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
        let info = Info {
            elapsed: Duration::from_millis(1),
            statement: &statement,
            failed: true,
        };

        callback(&info);

        let spans = exporter.get_finished_spans().expect("finished spans");
        assert!(matches!(spans[0].status, Status::Error { .. }));
    }

    /// The core regression test for "shouldn't I see spans in a trace for calls to the
    /// database server": the emitted query span must nest as a real child under
    /// whatever ambient `tracing` span was active when the query ran (e.g. the Axum
    /// request span), not just attach as a detached root span.
    #[tokio::test]
    async fn callback_span_is_parented_by_ambient_tracing_span() {
        let _lock = lock_tracer();
        let exporter = install_in_memory_tracer();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);

        let parent_span_id: SpanId = tracing::subscriber::with_default(subscriber, || {
            let parent_span = tracing::info_span!("parent");
            let _enter = parent_span.enter();

            let config = QueryTracingConfig::default();
            let callback = query_tracing_callback(config, test_pool());
            let statement = Statement::from_string(DbBackend::Postgres, "SELECT 1".to_string());
            let info = Info {
                elapsed: Duration::from_millis(1),
                statement: &statement,
                failed: false,
            };

            callback(&info);

            parent_span.context().span().span_context().span_id()
        });

        let spans = exporter.get_finished_spans().expect("finished spans");
        let query_span = spans
            .iter()
            .find(|s| s.name == "SELECT")
            .expect("query span present");
        assert_eq!(query_span.parent_span_id, parent_span_id);
    }
}
