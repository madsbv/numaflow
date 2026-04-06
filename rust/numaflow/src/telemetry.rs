use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::{Resource, trace as sdktrace};
use std::env;
use std::error::Error;
use std::sync::OnceLock;
use tracing::warn;
use tracing_opentelemetry::OpenTelemetryLayer;

static TRACER_PROVIDER: OnceLock<sdktrace::SdkTracerProvider> = OnceLock::new();

pub fn resource_attributes() -> Vec<KeyValue> {
    // TODO: Duplicate resources from otel.go:67?
    vec![]
}

fn build_exporter() -> Result<opentelemetry_otlp::SpanExporter, Box<dyn Error>> {
    Ok(opentelemetry_otlp::SpanExporter::builder()
        // Uses protobuf by default (because `http-proto` feature flag is set and `http-json` is not).
        // TODO: Figure out how to enable grpc; tonic requires a tokio runtime, how to handle?
        .with_http()
        .build()?)
}

fn build_tracer_provider(
    exporter: opentelemetry_otlp::SpanExporter,
) -> sdktrace::SdkTracerProvider {
    let resource = Resource::builder()
        // TODO: Figure out how to set default service name correctly (while allowing override?). Can maybe use Resource::from_empty and Resource::merge.
        // .with_service_name(config.service_name.clone())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .with_attributes(resource_attributes())
        .build();

    sdktrace::SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build()
}

pub fn init_telemetry<S>() -> Result<OpenTelemetryLayer<S, sdktrace::Tracer>, Box<dyn Error>>
where
    S: tracing::Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    let exporter = build_exporter()?;
    let tracer_provider = build_tracer_provider(exporter);

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());
    let _ = TRACER_PROVIDER.set(tracer_provider.clone());

    let tracer = tracer_provider.tracer("opentelemetry");
    let layer = OpenTelemetryLayer::new(tracer);
    Ok(layer)
}

pub fn shutdown() {
    if let Some(provider) = TRACER_PROVIDER.get()
        && let Err(e) = provider.shutdown()
    {
        warn!("Failed to shutdown tracer provider: {}", e);
    }
}
