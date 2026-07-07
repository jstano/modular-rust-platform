//! OTLP-based observability: tracing, metrics, and log export, wired automatically
//! into [`crate::server::run`].

use axum::{extract::MatchedPath, extract::Request, middleware::Next, response::Response};
use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::SdkMeterProvider,
    trace::{Sampler, SdkTracerProvider},
    Resource,
};
use stano_di::environment::Environment;
use std::time::Instant;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

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
    /// OTLP collector endpoint, e.g. `http://localhost:4317` (grpc) or
    /// `http://localhost:4318` (http/protobuf).
    pub otlp_endpoint: String,
    /// Wire protocol to use when talking to `otlp_endpoint`.
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
    /// Whether to log every HTTP request (method, URI, status, latency, trace_id)
    /// via `stano_axum::http_request_logging_middleware`. Independent of `enabled`.
    pub http_logging_enabled: bool,
}

/// Reads [`ObservabilityConfig`] from environment variables, following the same
/// lookup pattern as [`crate::config::parse_csv_env`]. Uses standard OTel env var
/// names where they exist, plus `STANO_OTEL_ENABLED`/`STANO_OTEL_METRICS_ENABLED`/
/// `STANO_HTTP_LOGGING_ENABLED` for the platform-specific enable switches (all
/// default to `false`).
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

    let default_endpoint = match protocol {
        OtlpProtocol::Grpc => "http://localhost:4317",
        OtlpProtocol::HttpProtobuf => "http://localhost:4318",
    };

    ObservabilityConfig {
        enabled: environment
            .get("STANO_OTEL_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        otlp_endpoint: environment
            .get("OTEL_EXPORTER_OTLP_ENDPOINT")
            .unwrap_or_else(|| default_endpoint.to_string()),
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
            .get("STANO_OTEL_METRICS_ENABLED")
            .unwrap_or_default()
            .eq_ignore_ascii_case("true"),
        http_logging_enabled: environment
            .get("STANO_HTTP_LOGGING_ENABLED")
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
}

impl OtelGuard {
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
/// and log export when `config.enabled`), and the global OTLP meter provider when
/// `config.enabled && config.metrics_enabled`. Must be called exactly once, before any
/// `tracing::` calls you want captured — [`crate::server::run`] calls this itself as
/// the first thing it does, so most apps never need to call this directly.
pub fn init_observability(config: &ObservabilityConfig) -> anyhow::Result<OtelGuard> {
    let env_filter =
        EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer();

    if !config.enabled {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init()
            .map_err(|e| anyhow::anyhow!("failed to install tracing subscriber: {e}"))?;

        return Ok(OtelGuard {
            tracer_provider: None,
            meter_provider: None,
            logger_provider: None,
        });
    }

    let resource = build_resource(config);

    let span_exporter = match config.protocol {
        OtlpProtocol::Grpc => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&config.otlp_endpoint)
            .build(),
        OtlpProtocol::HttpProtobuf => SpanExporter::builder()
            .with_http()
            .with_endpoint(&config.otlp_endpoint)
            .build(),
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
    let tracer = tracer_provider.tracer(config.service_name.clone());
    let otel_trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let log_exporter = match config.protocol {
        OtlpProtocol::Grpc => LogExporter::builder()
            .with_tonic()
            .with_endpoint(&config.otlp_endpoint)
            .build(),
        OtlpProtocol::HttpProtobuf => LogExporter::builder()
            .with_http()
            .with_endpoint(&config.otlp_endpoint)
            .build(),
    }
    .map_err(|e| anyhow::anyhow!("failed to build OTLP log exporter: {e}"))?;

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .with_resource(resource.clone())
        .build();
    let otel_log_layer = OpenTelemetryTracingBridge::new(&logger_provider);

    let meter_provider = if config.metrics_enabled {
        let metric_exporter = match config.protocol {
            OtlpProtocol::Grpc => MetricExporter::builder()
                .with_tonic()
                .with_endpoint(&config.otlp_endpoint)
                .build(),
            OtlpProtocol::HttpProtobuf => MetricExporter::builder()
                .with_http()
                .with_endpoint(&config.otlp_endpoint)
                .build(),
        }
        .map_err(|e| anyhow::anyhow!("failed to build OTLP metric exporter: {e}"))?;

        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(metric_exporter)
            .with_resource(resource)
            .build();
        global::set_meter_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_trace_layer)
        .with(otel_log_layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to install tracing subscriber: {e}"))?;

    Ok(OtelGuard {
        tracer_provider: Some(tracer_provider),
        meter_provider,
        logger_provider: Some(logger_provider),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn config_from_env_defaults_disabled() {
        let env = MockEnvironment::new();
        let config = observability_config_from_env(&env);
        assert!(!config.enabled);
        assert!(!config.metrics_enabled);
        assert!(!config.http_logging_enabled);
        assert_eq!(config.protocol, OtlpProtocol::Grpc);
        assert_eq!(config.otlp_endpoint, "http://localhost:4317");
        assert_eq!(config.log_filter, "info");
        assert_eq!(config.trace_sample_ratio, 1.0);
    }

    #[test]
    fn config_from_env_reads_http_protocol() {
        let env = MockEnvironment::new().with_var("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf");
        let config = observability_config_from_env(&env);
        assert_eq!(config.protocol, OtlpProtocol::HttpProtobuf);
        assert_eq!(config.otlp_endpoint, "http://localhost:4318");
    }

    #[test]
    fn config_from_env_reads_enabled_flags() {
        let env = MockEnvironment::new()
            .with_var("STANO_OTEL_ENABLED", "true")
            .with_var("STANO_OTEL_METRICS_ENABLED", "TRUE");
        let config = observability_config_from_env(&env);
        assert!(config.enabled);
        assert!(config.metrics_enabled);
    }

    #[test]
    fn disabled_config_init_returns_noop_guard() {
        let config = ObservabilityConfig {
            enabled: false,
            otlp_endpoint: "http://127.0.0.1:1".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: Vec::new(),
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            http_logging_enabled: false,
        };

        // May fail to install if another test already installed a global subscriber in
        // this process — either outcome (Ok or the "already set" error) is acceptable;
        // what matters is that it never panics.
        let result = init_observability(&config);
        if let Ok(guard) = result {
            assert!(guard.shutdown().is_ok());
        }
    }

    #[tokio::test]
    async fn enabled_config_with_unreachable_endpoint_does_not_panic() {
        let config = ObservabilityConfig {
            enabled: true,
            otlp_endpoint: "http://127.0.0.1:1".to_string(),
            protocol: OtlpProtocol::Grpc,
            service_name: "test-service".to_string(),
            service_version: "0.0.0".to_string(),
            resource_attributes: vec![("deployment.environment".to_string(), "test".to_string())],
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: true,
            http_logging_enabled: true,
        };

        // OTLP exporters connect lazily/asynchronously, so building them against an
        // unreachable endpoint should not fail or panic here.
        let _ = init_observability(&config);
    }
}
