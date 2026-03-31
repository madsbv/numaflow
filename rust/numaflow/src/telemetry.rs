use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData};
use opentelemetry_sdk::{Resource, runtime, trace as sdktrace};
use std::env;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;
use tracing_opentelemetry::{Layers, OpenTelemetryLayer};

const DEFAULT_ENDPOINT: &str = "http://localhost:4317";
const DEFAULT_PROTOCOL: &str = "grpc";
const DEFAULT_SERVICE_NAME: &str = "numaflow";
const DEFAULT_SAMPLER: &str = "always_on";
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 5;

const ENV_OTEL_EXPORTER_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const ENV_OTEL_EXPORTER_OTLP_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
const ENV_OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
const ENV_OTEL_TRACES_SAMPLER: &str = "OTEL_TRACES_SAMPLER";
const ENV_OTEL_TRACES_SAMPLER_ARG: &str = "OTEL_TRACES_SAMPLER_ARG";
const ENV_OTEL_SHUTDOWN_TIMEOUT_SECS: &str = "OTEL_SHUTDOWN_TIMEOUT_SECS";

#[derive(Debug, Error)]
pub enum TelemetryError {
    #[error("Failed to create OTLP exporter: {0}")]
    ExporterCreation(String),
    #[error("Failed to create tracer provider: {0}")]
    TracerProviderCreation(String),
    #[error("Invalid protocol '{0}', expected 'grpc' or 'http'")]
    InvalidProtocol(String),
    #[error("Invalid sampler '{0}', expected 'always_on', 'always_off', or 'trace_id_ratio'")]
    InvalidSampler(String),
    #[error("Invalid sampler ratio '{0}', expected a value between 0.0 and 1.0")]
    InvalidSamplerRatio(String),
    #[error("Invalid shutdown timeout '{0}', expected a positive integer")]
    InvalidShutdownTimeout(String),
}

#[derive(Debug, Clone, Copy)]
pub enum OtlpProtocol {
    Grpc,
    Http,
}

impl FromStr for OtlpProtocol {
    type Err = TelemetryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "grpc" => Ok(OtlpProtocol::Grpc),
            "http" | "http/protobuf" => Ok(OtlpProtocol::Http),
            _ => Err(TelemetryError::InvalidProtocol(s.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SamplerConfig {
    AlwaysOn,
    AlwaysOff,
    TraceIdRatio(f64),
}

impl FromStr for SamplerConfig {
    type Err = TelemetryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "always_on" => Ok(SamplerConfig::AlwaysOn),
            "always_off" => Ok(SamplerConfig::AlwaysOff),
            "trace_id_ratio" => {
                let ratio =
                    env::var(ENV_OTEL_TRACES_SAMPLER_ARG).unwrap_or_else(|_| "1.0".to_string());
                let ratio: f64 = ratio
                    .parse()
                    .map_err(|_| TelemetryError::InvalidSamplerRatio(ratio.clone()))?;
                if !(0.0..=1.0).contains(&ratio) {
                    return Err(TelemetryError::InvalidSamplerRatio(ratio.to_string()));
                }
                Ok(SamplerConfig::TraceIdRatio(ratio))
            }
            _ => Err(TelemetryError::InvalidSampler(s.to_string())),
        }
    }
}

impl SamplerConfig {
    fn to_sdk_sampler(&self) -> sdktrace::Sampler {
        match self {
            SamplerConfig::AlwaysOn => sdktrace::Sampler::AlwaysOn,
            SamplerConfig::AlwaysOff => sdktrace::Sampler::AlwaysOff,
            SamplerConfig::TraceIdRatio(ratio) => sdktrace::Sampler::TraceIdRatioBased(*ratio),
        }
    }
}

pub struct OtelConfig {
    pub endpoint: String,
    pub protocol: OtlpProtocol,
    pub service_name: String,
    pub sampler: SamplerConfig,
    pub shutdown_timeout_secs: u64,
}

impl OtelConfig {
    fn from_env() -> Self {
        let endpoint = env::var(ENV_OTEL_EXPORTER_OTLP_ENDPOINT)
            .unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
        let protocol = env::var(ENV_OTEL_EXPORTER_OTLP_PROTOCOL)
            .unwrap_or_else(|_| DEFAULT_PROTOCOL.to_string())
            .parse()
            .unwrap_or_else(|e| {
                warn!(
                    "Invalid OTEL_EXPORTER_OTLP_PROTOCOL '{}': {}, using default '{}'",
                    env::var(ENV_OTEL_EXPORTER_OTLP_PROTOCOL).unwrap_or_default(),
                    e,
                    DEFAULT_PROTOCOL
                );
                OtlpProtocol::Grpc
            });
        let service_name =
            env::var(ENV_OTEL_SERVICE_NAME).unwrap_or_else(|_| DEFAULT_SERVICE_NAME.to_string());
        let sampler = env::var(ENV_OTEL_TRACES_SAMPLER)
            .unwrap_or_else(|_| DEFAULT_SAMPLER.to_string())
            .parse()
            .unwrap_or_else(|e| {
                warn!(
                    "Invalid OTEL_TRACES_SAMPLER '{}': {}, using default '{}'",
                    env::var(ENV_OTEL_TRACES_SAMPLER).unwrap_or_default(),
                    e,
                    DEFAULT_SAMPLER
                );
                SamplerConfig::AlwaysOn
            });
        let shutdown_timeout_secs = env::var(ENV_OTEL_SHUTDOWN_TIMEOUT_SECS)
            .unwrap_or_else(|_| DEFAULT_SHUTDOWN_TIMEOUT_SECS.to_string())
            .parse()
            .unwrap_or_else(|e| {
                warn!(
                    "Invalid OTEL_SHUTDOWN_TIMEOUT_SECS '{}': {}, using default '{}'",
                    env::var(ENV_OTEL_SHUTDOWN_TIMEOUT_SECS).unwrap_or_default(),
                    e,
                    DEFAULT_SHUTDOWN_TIMEOUT_SECS
                );
                DEFAULT_SHUTDOWN_TIMEOUT_SECS
            });

        Self {
            endpoint,
            protocol,
            service_name,
            sampler,
            shutdown_timeout_secs,
        }
    }
}

pub fn resource_attributes() -> Vec<KeyValue> {
    vec![]
}

fn build_exporter(config: &OtelConfig) -> Result<SpanExporter, TelemetryError> {
    match config.protocol {
        OtlpProtocol::Grpc => opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(&config.endpoint)
            .with_timeout(Duration::from_secs(3))
            .try_init()
            .map_err(|e| TelemetryError::ExporterCreation(e.to_string())),
        OtlpProtocol::Http => opentelemetry_otlp::new_exporter()
            .http()
            .with_endpoint(&config.endpoint)
            .with_timeout(Duration::from_secs(3))
            .try_init()
            .map_err(|e| TelemetryError::ExporterCreation(e.to_string())),
    }
}

fn build_tracer_provider(
    exporter: SpanExporter,
    config: &OtelConfig,
) -> Result<sdktrace::TracerProvider, TelemetryError> {
    let resource = Resource::builder()
        .with_service_name(&config.service_name)
        .with_service_version(env!("CARGO_PKG_VERSION"))
        .with_attributes(resource_attributes())
        .build();

    sdktrace::TracerProvider::builder()
        .with_resource(resource)
        .with_sampler(config.sampler.to_sdk_sampler())
        .with_batch_exporter(exporter, runtime::Tokio)
        .build()
        .map_err(|e| TelemetryError::TracerProviderCreation(e.to_string()))
}

pub fn init_telemetry() -> Result<OpenTelemetryLayer<sdktrace::Tracer, Layers>, TelemetryError> {
    let config = OtelConfig::from_env();
    let exporter = build_exporter(&config)?;
    let tracer_provider = build_tracer_provider(exporter, &config)?;
    let tracer = tracer_provider.versioned_tracer("opentelemetry", Some(env!("CARGO_PKG_VERSION")));
    let layer = OpenTelemetryLayer::new(tracer);
    Ok(layer)
}

pub fn shutdown() {
    opentelemetry::global::shutdown_tracer_provider();
}
