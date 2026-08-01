use stano_example_app::build_context;
use stano_example_rest_api::build_authorization;
use stano_example_security::demo_jwt_config;
use stano_launcher::{BootstrapConfig, ObservabilityConfig, OtlpProtocol, run};
use utoipa_axum::router::OpenApiRouter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ctx = build_context();
    let jwt_config = demo_jwt_config();
    let authorization = build_authorization(jwt_config.clone());

    let config = BootstrapConfig {
        port: 8080,
        jwt_config,
        cors_origins: vec![],
        cors_origin_suffixes: vec![],
        cors_dev_origins: vec![],
        is_dev: true,
        // No OTLP collector required to run this example: observability export stays off.
        observability: ObservabilityConfig {
            enabled: false,
            otlp_endpoint: String::new(),
            protocol: OtlpProtocol::Grpc,
            service_name: "stano-example-app".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            resource_attributes: vec![],
            trace_sample_ratio: 1.0,
            log_filter: "info".to_string(),
            metrics_enabled: false,
            http_logging_enabled: true,
        },
        enable_swagger: true,
    };

    run(ctx, OpenApiRouter::new(), config, Some(authorization)).await
}
