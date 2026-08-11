//! OTLP-based observability: tracing, metrics, and log export, wired automatically
//! into [`crate::server::run`].

use axum::{
    extract::MatchedPath,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use opentelemetry::{KeyValue, global, trace::TracerProvider};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    Resource,
    logs::SdkLoggerProvider,
    metrics::SdkMeterProvider,
    trace::{Sampler, SdkTracerProvider},
};
use prometheus::{Encoder, Registry, TextEncoder};
use stano_di::environment::Environment;
use std::time::Instant;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// OTLP wire protocol used to talk to the collector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtlpProtocol {
    /// gRPC transport (typically collector port 4317).
    Grpc,
    /// HTTP/protobuf transport (typically collector port 4318).
    HttpProtobuf,
}

/// Configuration for OTLP-based tracing, metrics, and log export. Constructed via
/// [`observability_config_from_env`] or built directly.
#[derive(Clone, Debug)]
pub struct ObservabilityConfig {
    /// Master switch. When `false`, only a local `fmt` + `EnvFilter` subscriber is
    /// installed and no OTLP export happens — the safe default for local dev without
    /// a collector.
    pub enabled: bool,
    /// Wire protocol to use when talking to the OTLP collector. The endpoint itself is
    /// not configured here — `opentelemetry-otlp` reads `OTEL_EXPORTER_OTLP_ENDPOINT`
    /// (and the per-signal `_TRACES_`/`_LOGS_`/`_METRICS_ENDPOINT` variants) from the
    /// real process environment directly, the same way it already reads
    /// `OTEL_EXPORTER_OTLP_HEADERS`. This matters beyond just less code to keep in
    /// sync with the spec: letting the crate resolve the endpoint itself is also what
    /// gets the OTLP-spec-required `/v1/traces`/`/v1/logs`/`/v1/metrics` path suffix
    /// appended automatically for HTTP/protobuf — passing the endpoint programmatically
    /// (as this used to do via `.with_endpoint(...)`) bypasses that suffixing entirely.
    pub protocol: OtlpProtocol,
    /// `service.name` resource attribute.
    pub service_name: String,
    /// `service.version` resource attribute.
    pub service_version: String,
    /// Additional OTel resource attributes, e.g. `("deployment.environment", "prod")`.
    pub resource_attributes: Vec<(String, String)>,
    /// Trace sampling ratio in `0.0..=1.0`. `1.0` samples every trace.
    pub trace_sample_ratio: f64,
    /// `tracing_subscriber::EnvFilter` directive string, e.g. `"info,my_app=debug"`.
    pub log_filter: String,
    /// Whether to additionally export OTLP metrics and record HTTP server metrics.
    /// Independent of `enabled` so trace/log export can run without metrics.
    pub metrics_enabled: bool,
    /// Whether to expose a local Prometheus scrape endpoint at `GET /metrics`, serving
    /// all metrics recorded via the global OTel meter (including HTTP server metrics
    /// when `record_http_metrics` is mounted). Unlike `metrics_enabled` (which pushes to
    /// an OTLP collector), this is a pull exporter with no collector dependency, so it
    /// works even when `enabled` is `false`.
    pub prometheus_enabled: bool,
    /// Whether to log every HTTP request (method, URI, status, latency, trace_id)
    /// via `stano_axum::http_request_logging_middleware`. Independent of `enabled`.
    pub http_logging_enabled: bool,
    /// Whether to spawn a background process CPU/memory (and disk I/O) observer via the
    /// `opentelemetry-system-metrics` crate, recording `process.cpu.usage`,
    /// `process.cpu.utilization`, `process.memory.usage`, `process.memory.virtual`, and
    /// `process.disk.io` on an interval (`OTEL_METRIC_EXPORT_INTERVAL`, spec default
    /// 30s — read directly by that crate, same as `metrics_enabled`'s OTLP push
    /// interval). Independent flag, not folded into `metrics_enabled`: container/
    /// orchestration-level scraping (cAdvisor, kubelet, Docker stats) usually already
    /// covers this more accurately than in-process self-reporting, so it's opt-in
    /// rather than bundled. Only takes effect when at least one meter reader exists
    /// (`metrics_enabled` and/or `prometheus_enabled`) — otherwise there's nowhere for
    /// the recorded values to go, so the background poller isn't started at all.
    pub process_metrics_enabled: bool,
}

/// Reads [`ObservabilityConfig`] from environment variables, following the same
/// lookup pattern as [`crate::config::parse_csv_env`]. Uses standard OTel env var
/// names where they exist, plus `MRP_OTEL_ENABLED`/`MRP_OTEL_METRICS_ENABLED`/
/// `MRP_HTTP_LOGGING_ENABLED`/`MRP_PROCESS_METRICS_ENABLED` for the platform-specific
/// enable switches (all default to `false`). Notably does *not* read
/// `OTEL_EXPORTER_OTLP_ENDPOINT` — see [`ObservabilityConfig::protocol`]'s doc comment
/// for why.
pub fn observability_config_from_env(environment: &dyn Environment) -> ObservabilityConfig {
    let protocol = match environment
        .get("OTEL_EXPORTER_OTLP_PROTOCOL")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "http/protobuf" | "http" => OtlpProtocol::HttpProtobuf,
        _ => OtlpProtocol::Grpc,
    };

    ObservabilityConfig {
        enabled: environment
            .get("MRP_OTEL_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        protocol,
        service_name: environment
            .get("OTEL_SERVICE_NAME")
            .unwrap_or_else(|| "stano-app".to_string()),
        service_version: environment
            .get("OTEL_SERVICE_VERSION")
            .unwrap_or_else(|| "0.0.0".to_string()),
        resource_attributes: Vec::new(),
        trace_sample_ratio: environment
            .get("OTEL_TRACES_SAMPLER_ARG")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0),
        log_filter: environment
            .get("RUST_LOG")
            .unwrap_or_else(|| "info".to_string()),
        metrics_enabled: environment
            .get("MRP_OTEL_METRICS_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        prometheus_enabled: environment
            .get("MRP_PROMETHEUS_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        http_logging_enabled: environment
            .get("MRP_HTTP_LOGGING_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        process_metrics_enabled: environment
            .get("MRP_PROCESS_METRICS_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
    }
}

/// Holds the OTel SDK providers installed by [`init_observability`], if any. Must be
/// kept alive for the process lifetime and flushed via [`OtelGuard::shutdown`] (or
/// allowed to drop, which performs a best-effort blocking flush).
pub struct OtelGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
    prometheus_registry: Option<prometheus::Registry>,
}

impl OtelGuard {
    /// The Prometheus registry backing `GET /metrics`, present when
    /// [`ObservabilityConfig::prometheus_enabled`] was true at [`init_observability`]
    /// time. [`crate::server::run`] uses this to mount the scrape endpoint.
    pub fn prometheus_registry(&self) -> Option<&prometheus::Registry> {
        self.prometheus_registry.as_ref()
    }

    /// Forces all configured OTel providers to export whatever spans/metrics/logs are
    /// currently sitting in their batch buffers, without shutting the providers down —
    /// unlike [`OtelGuard::shutdown`], the guard (and the providers behind it) remain
    /// usable afterwards.
    ///
    /// Trace/log/metric export is batched (`BatchSpanProcessor`/`BatchLogProcessor`/
    /// periodic metric reader), so spans normally wait up to `OTEL_BSP_SCHEDULE_DELAY`
    /// (spec default 5s) — or until 512 accumulate — before being sent to the
    /// collector. A process that exits ungracefully (`SIGKILL`, e.g. some IDE "stop"
    /// buttons that skip `SIGTERM`) never runs [`OtelGuard::shutdown`] or this crate's
    /// `Drop` impl, so anything still buffered at that moment is lost — it never
    /// reaches the collector. Call `force_flush` on a timer, before a risky operation,
    /// or from a debug endpoint if you need buffered data flushed without waiting for
    /// the scheduled delay or a full shutdown.
    pub fn force_flush(&self) -> anyhow::Result<()> {
        if let Some(provider) = &self.tracer_provider {
            provider
                .force_flush()
                .map_err(|e| anyhow::anyhow!("failed to flush tracer provider: {e}"))?;
        }
        if let Some(provider) = &self.meter_provider {
            provider
                .force_flush()
                .map_err(|e| anyhow::anyhow!("failed to flush meter provider: {e}"))?;
        }
        if let Some(provider) = &self.logger_provider {
            provider
                .force_flush()
                .map_err(|e| anyhow::anyhow!("failed to flush logger provider: {e}"))?;
        }
        Ok(())
    }

    /// Flushes and shuts down all configured OTel providers. Prefer calling this
    /// explicitly after your server future resolves, rather than relying solely on
    /// `Drop`, so shutdown errors can be observed.
    pub fn shutdown(self) -> anyhow::Result<()> {
        if let Some(provider) = &self.tracer_provider {
            provider
                .shutdown()
                .map_err(|e| anyhow::anyhow!("failed to shut down tracer provider: {e}"))?;
        }
        if let Some(provider) = &self.meter_provider {
            provider
                .shutdown()
                .map_err(|e| anyhow::anyhow!("failed to shut down meter provider: {e}"))?;
        }
        if let Some(provider) = &self.logger_provider {
            provider
                .shutdown()
                .map_err(|e| anyhow::anyhow!("failed to shut down logger provider: {e}"))?;
        }
        Ok(())
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(provider) = &self.tracer_provider
            && let Err(e) = provider.shutdown()
        {
            tracing::warn!(error = %e, "failed to shut down OTel tracer provider on drop");
        }
        if let Some(provider) = &self.meter_provider
            && let Err(e) = provider.shutdown()
        {
            tracing::warn!(error = %e, "failed to shut down OTel meter provider on drop");
        }
        if let Some(provider) = &self.logger_provider
            && let Err(e) = provider.shutdown()
        {
            tracing::warn!(error = %e, "failed to shut down OTel logger provider on drop");
        }
    }
}

fn build_resource(config: &ObservabilityConfig) -> Resource {
    let mut builder = Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attribute(KeyValue::new(
            "service.version",
            config.service_version.clone(),
        ));

    for (key, value) in &config.resource_attributes {
        builder = builder.with_attribute(KeyValue::new(key.clone(), value.clone()));
    }

    builder.build()
}

/// Initializes the global `tracing` subscriber (console `fmt` output, plus OTLP trace
/// and log export when `config.enabled`), and the global OTel meter provider — with an
/// OTLP push reader when `config.enabled && config.metrics_enabled`, and/or a local
/// Prometheus pull reader when `config.prometheus_enabled` (independent of
/// `config.enabled`, since it needs no OTLP collector). Must be called exactly once,
/// before any `tracing::` calls you want captured — [`crate::server::run`] calls this
/// itself as the first thing it does, so most apps never need to call this directly.
/// Also installs the global OTel tracer provider (`opentelemetry::global::set_tracer_provider`,
/// mirroring the meter provider below) whenever `config.enabled` — so
/// `opentelemetry::global::tracer(...)` callers elsewhere in the process (e.g.
/// `stano-seaorm`'s per-query spans) resolve to the real provider rather than a
/// permanent no-op, provided this runs before they first call `global::tracer(...)`.
///
/// ## Troubleshooting: fewer traces showing up in the collector than expected
///
/// Span/log export is batched (`BatchSpanProcessor`/`BatchLogProcessor`), not
/// per-request — spans sit in memory for up to `OTEL_BSP_SCHEDULE_DELAY` (spec
/// default 5s) or until 512 accumulate before being sent. Two consequences:
///
/// - **Ungraceful process kills lose buffered spans.** Only `SIGINT`/`SIGTERM`
///   trigger [`crate::server::run`]'s graceful shutdown, which calls
///   [`OtelGuard::shutdown`] and flushes the buffer. A `SIGKILL` (some IDE "stop"
///   buttons, `docker kill`, OOM) skips that entirely, silently dropping anything
///   not yet exported. Stop the process gracefully, or call
///   [`OtelGuard::force_flush`] first, if you need to guarantee delivery.
/// - **Failed exports are logged, not surfaced as errors here.** `init_observability`
///   only returns `Err` for exporter *construction* failures (bad config). Actual
///   export-time failures (unreachable collector, wrong OTLP path, auth rejection)
///   are reported by the `opentelemetry`/`opentelemetry_sdk` crates themselves via
///   `tracing::error!`/`tracing::warn!` records with target `opentelemetry_sdk` /
///   `opentelemetry-otlp` (the `opentelemetry` crate's `internal-logs` feature,
///   enabled by default) — grep the JSON console output for those targets if traces
///   seem to be disappearing rather than just delayed.
pub fn init_observability(config: &ObservabilityConfig) -> anyhow::Result<OtelGuard> {
    let env_filter =
        EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer().json();

    // Resource and meter-provider construction happen regardless of `config.enabled`:
    // the Prometheus reader is a local pull exporter with no OTLP collector dependency,
    // so it must work even when trace/log export (gated on `enabled`) is off.
    let resource = build_resource(config);

    let mut meter_builder = SdkMeterProvider::builder().with_resource(resource.clone());
    let mut have_meter_reader = false;

    let prometheus_registry = if config.prometheus_enabled {
        let registry = prometheus::Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build Prometheus exporter: {e}"))?;
        meter_builder = meter_builder.with_reader(exporter);
        have_meter_reader = true;
        Some(registry)
    } else {
        None
    };

    if config.enabled && config.metrics_enabled {
        let metric_exporter = match config.protocol {
            OtlpProtocol::Grpc => MetricExporter::builder().with_tonic().build(),
            OtlpProtocol::HttpProtobuf => MetricExporter::builder().with_http().build(),
        }
        .map_err(|e| anyhow::anyhow!("failed to build OTLP metric exporter: {e}"))?;

        meter_builder = meter_builder.with_periodic_exporter(metric_exporter);
        have_meter_reader = true;
    }

    let meter_provider = if have_meter_reader {
        let provider = meter_builder.build();
        global::set_meter_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    if config.process_metrics_enabled && have_meter_reader {
        // Runs forever on its own interval (`OTEL_METRIC_EXPORT_INTERVAL`, read by the
        // crate itself, spec default 30s) — fire-and-forget, not tied to `OtelGuard`'s
        // shutdown/flush; it just gets killed with the process. Only its startup
        // failure (e.g. can't read the current PID/core count) is observable here.
        let meter = global::meter("stano-launcher");
        tokio::spawn(async move {
            if let Err(e) = opentelemetry_system_metrics::init_process_observer(meter).await {
                tracing::warn!(error = %e, "failed to start process CPU/memory metrics observer");
            }
        });
    }

    if !config.enabled {
        // Installing the global `tracing` subscriber can only succeed once per
        // process; a later caller "failing" here just means an earlier one already
        // won that race (e.g. multiple tests in the same binary). That's not fatal —
        // the meter provider / Prometheus registry built above are still valid.
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init();

        return Ok(OtelGuard {
            tracer_provider: None,
            meter_provider,
            logger_provider: None,
            prometheus_registry,
        });
    }

    let span_exporter = match config.protocol {
        OtlpProtocol::Grpc => SpanExporter::builder().with_tonic().build(),
        OtlpProtocol::HttpProtobuf => SpanExporter::builder().with_http().build(),
    }
    .map_err(|e| anyhow::anyhow!("failed to build OTLP span exporter: {e}"))?;

    let sampler = Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
        config.trace_sample_ratio,
    )));

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_sampler(sampler)
        .with_resource(resource.clone())
        .build();
    // Mirrors the `global::set_meter_provider(...)` call below: without this, any
    // crate that resolves a tracer via `opentelemetry::global::tracer(...)` (e.g.
    // `stano-seaorm`'s backdated per-query spans) permanently binds to a no-op
    // tracer, since `global::tracer(...)` never retroactively picks up a provider
    // installed later. Call this (i.e. `stano_launcher::run()`) before any of your
    // own code calls `global::tracer(...)`, same caveat as the meter provider.
    global::set_tracer_provider(tracer_provider.clone());
    let tracer = tracer_provider.tracer(config.service_name.clone());
    let otel_trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let log_exporter = match config.protocol {
        OtlpProtocol::Grpc => LogExporter::builder().with_tonic().build(),
        OtlpProtocol::HttpProtobuf => LogExporter::builder().with_http().build(),
    }
    .map_err(|e| anyhow::anyhow!("failed to build OTLP log exporter: {e}"))?;

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource.clone())
        .build();
    let otel_log_layer = OpenTelemetryTracingBridge::new(&logger_provider);

    // Same non-fatal treatment as above: another caller in this process may already
    // have installed the global subscriber.
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_trace_layer)
        .with(otel_log_layer)
        .try_init();

    Ok(OtelGuard {
        tracer_provider: Some(tracer_provider),
        meter_provider,
        logger_provider: Some(logger_provider),
        prometheus_registry,
    })
}

/// Axum middleware recording basic HTTP server metrics (`http.server.request.duration`,
/// `http.server.active_requests`) via the global OTel meter. Add this with
/// [`axum::Router::route_layer`] (not `layer`) so [`MatchedPath`] is available for the
/// `http.route` attribute — [`crate::server::run`] does this automatically when
/// [`ObservabilityConfig::metrics_enabled`] is true.
pub async fn record_http_metrics(req: Request, next: Next) -> Response {
    let meter = global::meter("stano-launcher");
    let active_requests = meter
        .i64_up_down_counter("http.server.active_requests")
        .build();
    let duration_histogram = meter.f64_histogram("http.server.request.duration").build();

    let method = req.method().to_string();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let method_attr = KeyValue::new("http.request.method", method.clone());
    active_requests.add(1, std::slice::from_ref(&method_attr));
    let start = Instant::now();

    let response = next.run(req).await;

    active_requests.add(-1, std::slice::from_ref(&method_attr));
    duration_histogram.record(
        start.elapsed().as_secs_f64(),
        &[
            method_attr,
            KeyValue::new("http.route", route),
            KeyValue::new(
                "http.response.status_code",
                response.status().as_u16() as i64,
            ),
        ],
    );

    response
}

async fn serve_prometheus_metrics(registry: Registry) -> Response {
    let metric_families = registry.gather();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!(error = %e, "failed to encode Prometheus metrics");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to encode metrics",
        )
            .into_response();
    }

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            encoder.format_type().to_string(),
        )],
        buffer,
    )
        .into_response()
}

/// Builds a standalone router exposing `GET /metrics` (Prometheus text-exposition
/// format) backed by `registry`. Merge this into the main app router before
/// `.with_state(...)`, mirroring how Swagger UI is mounted — [`crate::server::run`]
/// does this automatically when [`ObservabilityConfig::prometheus_enabled`] is true.
pub(crate) fn prometheus_router<S: Clone + Send + Sync + 'static>(
    registry: Registry,
) -> axum::Router<S> {
    axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || serve_prometheus_metrics(registry.clone())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
    };
    use opentelemetry::metrics::MeterProvider as _;
    use std::collections::HashMap;
    use tower::util::ServiceExt;

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

    // `init_observability` no longer reads `OTEL_EXPORTER_OTLP_ENDPOINT` itself — it's
    // left for `opentelemetry-otlp` to resolve from the real process environment, so
    // tests that need to guarantee an unreachable endpoint (no real network calls
    // during `cargo test`) must set that real env var instead of an `ObservabilityConfig`
    // field. This lock serializes those tests so they don't race each other over the
    // same process-global variable, mirroring `lock_manifest_dir()` in
    // `stano-di-macros`.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn config_from_env_defaults_disabled() {
        let env = MockEnvironment::new();
        let config = observability_config_from_env(&env);
        assert!(!config.enabled);
        assert!(!config.metrics_enabled);
        assert!(!config.http_logging_enabled);
        assert_eq!(config.protocol, OtlpProtocol::Grpc);
        assert_eq!(config.log_filter, "info");
        assert_eq!(config.trace_sample_ratio, 1.0);
    }

    #[test]
    fn config_from_env_reads_http_protocol() {
        let env = MockEnvironment::new().with_var("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf");
        let config = observability_config_from_env(&env);
        assert_eq!(config.protocol, OtlpProtocol::HttpProtobuf);
    }

    #[test]
    fn config_from_env_reads_enabled_flags() {
        let env = MockEnvironment::new()
            .with_var("MRP_OTEL_ENABLED", "true")
            .with_var("MRP_OTEL_METRICS_ENABLED", "TRUE");
        let config = observability_config_from_env(&env);
        assert!(config.enabled);
        assert!(config.metrics_enabled);
    }

    #[test]
    fn config_from_env_defaults_prometheus_disabled() {
        let env = MockEnvironment::new();
        let config = observability_config_from_env(&env);
        assert!(!config.prometheus_enabled);
    }

    #[test]
    fn config_from_env_reads_prometheus_enabled_flag() {
        let env = MockEnvironment::new().with_var("MRP_PROMETHEUS_ENABLED", "true");
        let config = observability_config_from_env(&env);
        assert!(config.prometheus_enabled);
    }

    #[test]
    fn disabled_config_init_returns_noop_guard() {
        let config = ObservabilityConfig {
            enabled: false,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            prometheus_enabled: false,
            http_logging_enabled: false,
            process_metrics_enabled: false,
        };

        let guard = init_observability(&config).expect("init");
        assert!(guard.shutdown().is_ok());
    }

    #[test]
    fn disabled_config_force_flush_is_noop() {
        let config = ObservabilityConfig {
            enabled: false,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            prometheus_enabled: false,
            http_logging_enabled: false,
            process_metrics_enabled: false,
        };

        let guard = init_observability(&config).expect("init");
        // No tracer/meter/logger provider is installed when `enabled` is false, so
        // there's nothing to flush — `force_flush` should still succeed rather than
        // erroring on the absent providers.
        assert!(guard.force_flush().is_ok());
        assert!(guard.shutdown().is_ok());
    }

    #[tokio::test]
    async fn enabled_config_force_flush_with_unreachable_endpoint_does_not_panic() {
        let _lock = lock_env();
        // SAFETY: serialized by `_lock` above; no other test observes this var
        // concurrently.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1");
        }

        let config = ObservabilityConfig {
            enabled: true,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            prometheus_enabled: false,
            http_logging_enabled: false,
            process_metrics_enabled: false,
        };

        // Flushing against an unreachable collector should surface as an `Err`
        // (export failure), not a panic — exercises the same exporter path a real
        // ungraceful-shutdown recovery attempt would hit.
        let guard = init_observability(&config).expect("init");
        let _ = guard.force_flush();

        unsafe {
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        }
    }

    #[tokio::test]
    async fn enabled_config_with_unreachable_endpoint_does_not_panic() {
        let _lock = lock_env();
        // SAFETY: serialized by `_lock` above; no other test observes this var
        // concurrently.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1");
        }

        let config = ObservabilityConfig {
            enabled: true,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: vec![("deployment.environment".to_string(), "test".to_string())],
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: true,
            prometheus_enabled: false,
            http_logging_enabled: true,
            process_metrics_enabled: false,
        };

        // OTLP exporters connect lazily/asynchronously, so building them against an
        // unreachable endpoint should not fail or panic here.
        let _ = init_observability(&config);

        unsafe {
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        }
    }

    #[tokio::test]
    async fn enabled_config_with_http_protobuf_protocol_does_not_panic() {
        let _lock = lock_env();
        // SAFETY: serialized by `_lock` above; no other test observes this var
        // concurrently.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1");
        }

        let config = ObservabilityConfig {
            enabled: true,
            protocol: OtlpProtocol::HttpProtobuf,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: true,
            prometheus_enabled: false,
            http_logging_enabled: true,
            process_metrics_enabled: false,
        };

        // Exercises the `OtlpProtocol::HttpProtobuf` branch of the span/log/metric
        // exporter builders (the Grpc branch is covered above). Also exercises the
        // fix for the bug where `.with_endpoint(...)` used to bypass
        // opentelemetry-otlp's automatic `/v1/traces`/`/v1/logs`/`/v1/metrics`
        // per-signal path suffixing for HTTP/protobuf.
        let _ = init_observability(&config);

        unsafe {
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        }
    }

    #[tokio::test]
    async fn enabled_config_installs_global_tracer_provider() {
        use opentelemetry::trace::{Span as _, Tracer as _};

        let _lock = lock_env();
        // SAFETY: serialized by `_lock` above; no other test observes this var
        // concurrently.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:1");
        }

        let config = ObservabilityConfig {
            enabled: true,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            prometheus_enabled: false,
            http_logging_enabled: false,
            process_metrics_enabled: false,
        };

        // Bound (not discarded) — `OtelGuard::drop` shuts down the tracer provider,
        // and since `global::set_tracer_provider` shares the same underlying provider
        // instance, an immediately-dropped guard would shut down the global one too,
        // making the assertion below spuriously fail.
        let guard = init_observability(&config).expect("init");

        // A no-op tracer (the pre-fix behavior, when `set_tracer_provider` was never
        // called) always produces spans with an invalid `SpanContext`. Getting a
        // valid one here is a smoke test that `global::set_tracer_provider(...)`
        // actually ran and other crates' `global::tracer(...)` calls resolve to the
        // real provider.
        let span = global::tracer("test").start("test-span");
        assert!(span.span_context().is_valid());
        drop(guard);

        unsafe {
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        }
    }

    async fn passthrough_handler() -> &'static str {
        "ok"
    }

    fn metrics_app() -> Router {
        Router::new()
            .route("/hello/{id}", axum::routing::get(passthrough_handler))
            .route_layer(middleware::from_fn(record_http_metrics))
    }

    #[tokio::test]
    async fn record_http_metrics_passes_through_response() {
        let app = metrics_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/hello/42")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn record_http_metrics_passes_through_404_for_unmatched_route() {
        // No `MatchedPath` extension is present for a 404, exercising the
        // `unwrap_or_else(|| "unknown")` fallback for `route`.
        let app = metrics_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn prometheus_enabled_config_populates_registry_without_otlp_enabled() {
        let config = ObservabilityConfig {
            enabled: false,
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            prometheus_enabled: true,
            http_logging_enabled: false,
            process_metrics_enabled: false,
        };

        // The Prometheus reader is a local pull exporter, so it must be populated even
        // though `enabled` (the OTLP trace/log switch) is false.
        let guard = init_observability(&config).expect("init");
        assert!(guard.prometheus_registry().is_some());
        let _ = guard.shutdown();
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_prometheus_text_format() {
        let registry = Registry::new();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
            .expect("exporter");
        let provider = SdkMeterProvider::builder().with_reader(exporter).build();
        let meter = provider.meter("test");
        meter.u64_counter("test_requests").build().add(1, &[]);

        let app: Router = prometheus_router::<()>(registry).with_state(());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("content-type header")
            .to_str()
            .expect("valid header value")
            .to_string();
        assert!(content_type.starts_with("text/plain"));

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body_str = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(body_str.contains("test_requests"));
    }
}
