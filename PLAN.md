# Dataplane OpenTelemetry Metrics Migration Plan

## Goal

Replace `prometheus-client` with the OpenTelemetry SDK metrics API across the `numaflow-core` and
`serving` crates. The migration achieves dual export:

- **Prometheus scraping** (`/metrics` endpoint) — behavior and metric names preserved exactly
- **OTLP push** — metrics exported via the existing HTTP/proto OTLP transport alongside traces

---

## Background and Decisions

### Why not the OTel Collector bridge?

The OTel project's own recommendation is to use OTLP + Prometheus's native OTLP receiver instead
of any bridge crate. However, that requires running a Prometheus instance with the OTLP receiver
enabled. The current deployment scrapes `/metrics` directly. Option B1 (in-process dual export)
preserves that scrape model without adding infrastructure.

### Why `opentelemetry-prometheus-text-exporter` and not `opentelemetry-prometheus`?

`opentelemetry-prometheus` is officially discontinued (final release 0.29, depends on unmaintained
`protobuf` crate, known security vulnerabilities). `opentelemetry-prometheus-text-exporter`
(crates.io: `sandhose`, v0.2.1, ~397k downloads) is the community replacement: it implements
`MetricReader` for `SdkMeterProvider`, targets `opentelemetry_sdk ^0.31`, has no dependency on the
old `prometheus` crate, and supports exactly the metric types used here (Gauges, Sums,
explicit-bucket Histograms).

### Why is "no exponential histogram support" not a concern?

All histograms in the codebase use explicit bucket boundaries (computed via
`exponential_buckets_range(min, max, count)` or `exponential_buckets(start, factor, count)`). The
OTel SDK `ExponentialHistogram` data type is not used anywhere.

### Deployment model

`numaflow serving` and `numaflow processor` (and other subcommands that use numaflow-core) are
**mutually exclusive process invocations** of the same binary. They never run in the same process.
Each subcommand initializes its own `SdkMeterProvider` at startup. There is no cross-contamination
between the metric sets of the two subcommands.

---

## Architecture

```
numaflow binary (main.rs)
│
├── serving subcommand
│   └── init_metrics() → SdkMeterProvider
│       ├── PrometheusExporter (MetricReader, pull) ──→ GET /metrics
│       └── PeriodicReader(OtlpMetricExporter, push) ──→ OTLP HTTP/proto
│
└── processor / other subcommands (numaflow-core)
    └── init_metrics() → SdkMeterProvider
        ├── PrometheusExporter (MetricReader, pull) ──→ GET /metrics (TLS)
        └── PeriodicReader(OtlpMetricExporter, push) ──→ OTLP HTTP/proto
```

The `PrometheusExporter` is cloned (it is `Clone + Send + Sync`) and passed down to the metrics
HTTP handler. The OTLP endpoint is read from `OTEL_EXPORTER_OTLP_ENDPOINT` or
`OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` at runtime (same pattern as the existing tracer).

---

## Metric Name Preservation

The `opentelemetry-prometheus-text-exporter` applies these transformations:

1. Sanitize: dots/invalid chars → `_`; consecutive `__` → `_`; **case preserved**
2. Unit suffix: appended from instrument unit (disabled by not setting a unit)
3. `_total` suffix: appended to monotonic cumulative Sums only; **skipped if name already ends
   with `_total`**

**Configuration required on `PrometheusExporter`:**
- `.without_scope_info()` — prevents adding an `otel_scope_name` label to every metric (that
  label does not exist in the current output)
- No unit set on any instrument — prevents unit suffixes being appended

**`target_info` metric:** Kept enabled. The exporter emits one new metric:
```
target_info{service_version="x.y.z"} 1
```
This is additive and does not affect any existing metric names or labels.

**Content-Type for `/metrics` responses:** `text/plain; version=0.0.4; charset=utf-8`
(standard Prometheus text exposition format, which is what the exporter produces).
This replaces the current `application/openmetrics-text; version=1.0.0; charset=utf-8` in
`numaflow-core`. The `serving` handler currently sets no Content-Type at all; this migration
fixes that omission.

### Full instrument name → Prometheus name mapping

All 78 metrics preserve their existing Prometheus names. No name changes.

| OTel instrument name | OTel type | Prometheus emitted name |
|---|---|---|
| `sdk_info` | `Gauge<i64>` | `sdk_info` |
| `monovtx_read` | `Counter<u64>` | `monovtx_read_total` |
| `monovtx_read_bytes` | `Counter<u64>` | `monovtx_read_bytes_total` |
| `monovtx_ack` | `Counter<u64>` | `monovtx_ack_total` |
| `monovtx_dropped` | `Counter<u64>` | `monovtx_dropped_total` |
| `monovtx_critical_error` | `Counter<u64>` | `monovtx_critical_error_total` |
| `monovtx_pending_raw` | `Gauge<i64>` | `monovtx_pending_raw` |
| `monovtx_read_batch_size` | `Gauge<i64>` | `monovtx_read_batch_size` |
| `monovtx_processing_time` | `Histogram<f64>` | `monovtx_processing_time_{sum,count,bucket}` |
| `monovtx_read_time` | `Histogram<f64>` | `monovtx_read_time_{sum,count,bucket}` |
| `monovtx_ack_time` | `Histogram<f64>` | `monovtx_ack_time_{sum,count,bucket}` |
| `monovtx_transformer_time` | `Histogram<f64>` | `monovtx_transformer_time_{sum,count,bucket}` |
| `monovtx_transformer_dropped` | `Counter<u64>` | `monovtx_transformer_dropped_total` |
| `monovtx_udf_time` | `Histogram<f64>` | `monovtx_udf_time_{sum,count,bucket}` |
| `monovtx_udf_udf_error` | `Counter<u64>` | `monovtx_udf_udf_error_total` |
| `monovtx_sink_write` | `Counter<u64>` | `monovtx_sink_write_total` |
| `monovtx_sink_time` | `Histogram<f64>` | `monovtx_sink_time_{sum,count,bucket}` |
| `monovtx_sink_write_errors` | `Counter<u64>` | `monovtx_sink_write_errors_total` |
| `monovtx_sink_dropped` | `Counter<u64>` | `monovtx_sink_dropped_total` |
| `monovtx_fallback_sink_write` | `Counter<u64>` | `monovtx_fallback_sink_write_total` |
| `monovtx_fallback_sink_time` | `Histogram<f64>` | `monovtx_fallback_sink_time_{sum,count,bucket}` |
| `monovtx_onsuccess_sink_write` | `Counter<u64>` | `monovtx_onsuccess_sink_write_total` |
| `monovtx_onsuccess_sink_time` | `Histogram<f64>` | `monovtx_onsuccess_sink_time_{sum,count,bucket}` |
| `forwarder_read` | `Counter<u64>` | `forwarder_read_total` |
| `forwarder_data_read` | `Counter<u64>` | `forwarder_data_read_total` |
| `forwarder_read_bytes` | `Counter<u64>` | `forwarder_read_bytes_total` |
| `forwarder_data_read_bytes` | `Counter<u64>` | `forwarder_data_read_bytes_total` |
| `forwarder_read_error` | `Counter<u64>` | `forwarder_read_error_total` |
| `forwarder_write` | `Counter<u64>` | `forwarder_write_total` |
| `forwarder_write_bytes` | `Counter<u64>` | `forwarder_write_bytes_total` |
| `forwarder_write_error` | `Counter<u64>` | `forwarder_write_error_total` |
| `forwarder_drop` | `Counter<u64>` | `forwarder_drop_total` |
| `forwarder_drop_bytes` | `Counter<u64>` | `forwarder_drop_bytes_total` |
| `forwarder_ack` | `Counter<u64>` | `forwarder_ack_total` |
| `forwarder_udf_read` | `Counter<u64>` | `forwarder_udf_read_total` |
| `forwarder_udf_write` | `Counter<u64>` | `forwarder_udf_write_total` |
| `forwarder_ud_drop` | `Counter<u64>` | `forwarder_ud_drop_total` |
| `forwarder_udf_error` | `Counter<u64>` | `forwarder_udf_error_total` |
| `forwarder_critical_error` | `Counter<u64>` | `forwarder_critical_error_total` |
| `forwarder_read_processing_time` | `Histogram<f64>` | `forwarder_read_processing_time_{sum,count,bucket}` |
| `forwarder_write_processing_time` | `Histogram<f64>` | `forwarder_write_processing_time_{sum,count,bucket}` |
| `forwarder_ack_processing_time` | `Histogram<f64>` | `forwarder_ack_processing_time_{sum,count,bucket}` |
| `forwarder_processing_time` | `Histogram<f64>` | `forwarder_processing_time_{sum,count,bucket}` |
| `forwarder_udf_processing_time` | `Histogram<f64>` | `forwarder_udf_processing_time_{sum,count,bucket}` |
| `forwarder_read_batch_size` | `Gauge<i64>` | `forwarder_read_batch_size` |
| `source_forwarder_transformer_read` | `Counter<u64>` | `source_forwarder_transformer_read_total` |
| `source_forwarder_transformer_write` | `Counter<u64>` | `source_forwarder_transformer_write_total` |
| `source_forwarder_transformer_error` | `Counter<u64>` | `source_forwarder_transformer_error_total` |
| `source_forwarder_transformer_drop` | `Counter<u64>` | `source_forwarder_transformer_drop_total` |
| `source_forwarder_transformer_processing_time` | `Histogram<f64>` | `source_forwarder_transformer_processing_time_{sum,count,bucket}` |
| `forwarder_fbsink_write` | `Counter<u64>` | `forwarder_fbsink_write_total` |
| `forwarder_fbsink_write_bytes` | `Counter<u64>` | `forwarder_fbsink_write_bytes_total` |
| `forwarder_fbsink_write_errors` | `Counter<u64>` | `forwarder_fbsink_write_errors_total` |
| `forwarder_fbsink_write_processing_time` | `Histogram<f64>` | `forwarder_fbsink_write_processing_time_{sum,count,bucket}` |
| `forwarder_onsuccess_sink_write` | `Counter<u64>` | `forwarder_onsuccess_sink_write_total` |
| `forwarder_onsuccess_sink_write_bytes` | `Counter<u64>` | `forwarder_onsuccess_sink_write_bytes_total` |
| `forwarder_onsuccess_sink_write_errors` | `Counter<u64>` | `forwarder_onsuccess_sink_write_errors_total` |
| `forwarder_onsuccess_sink_write_processing_time` | `Histogram<f64>` | `forwarder_onsuccess_sink_write_processing_time_{sum,count,bucket}` |
| `isb_jetstream_read_error` | `Counter<u64>` | `isb_jetstream_read_error_total` |
| `isb_jetstream_isFull_error` | `Counter<u64>` | `isb_jetstream_isFull_error_total` |
| `isb_jetstream_isFull` | `Counter<u64>` | `isb_jetstream_isFull_total` |
| `isb_jetstream_write_error` | `Counter<u64>` | `isb_jetstream_write_error_total` |
| `isb_jetstream_write_timeout` | `Counter<u64>` | `isb_jetstream_write_timeout_total` |
| `isb_jetstream_buffer_soft_usage` | `Gauge<f64>` | `isb_jetstream_buffer_soft_usage` |
| `isb_jetstream_buffer_solid_usage` | `Gauge<f64>` | `isb_jetstream_buffer_solid_usage` |
| `isb_jetstream_buffer_pending` | `Gauge<i64>` | `isb_jetstream_buffer_pending` |
| `isb_jetstream_buffer_ack_pending` | `Gauge<i64>` | `isb_jetstream_buffer_ack_pending` |
| `isb_jetstream_write_time_total` | `Histogram<f64>` | `isb_jetstream_write_time_total_{sum,count,bucket}` |
| `isb_jetstream_read_time_total` | `Histogram<f64>` | `isb_jetstream_read_time_total_{sum,count,bucket}` |
| `isb_jetstream_ack_time_total` | `Histogram<f64>` | `isb_jetstream_ack_time_total_{sum,count,bucket}` |
| `vertex_pending_messages_raw` | `Gauge<i64>` | `vertex_pending_messages_raw` |
| `sqs_producer_publish_latency` | `Histogram<f64>` | `sqs_producer_publish_latency_{sum,count,bucket}` |
| `sqs_producer_publish_success` | `Counter<u64>` | `sqs_producer_publish_success_total` |
| `sqs_producer_publish_failure` | `Counter<u64>` | `sqs_producer_publish_failure_total` |
| `http_requests_count` | `Counter<u64>` | `http_requests_count_total` |
| `http_requests_duration` | `Histogram<f64>` | `http_requests_duration_{sum,count,bucket}` |
| `REQUEST_REGISTER` | `Counter<u64>` | `REQUEST_REGISTER_total` |
| `REQUEST_REGISTER_FAIL` | `Counter<u64>` | `REQUEST_REGISTER_FAIL_total` |
| `REQUEST_REGISTER_DUPLICATES` | `Counter<u64>` | `REQUEST_REGISTER_DUPLICATES_total` |
| `REQUEST_REGISTER_DURATION` | `Histogram<f64>` | `REQUEST_REGISTER_DURATION_{sum,count,bucket}` |
| `PAYLOAD_JESTREAM_SAVE_DURATION` | `Histogram<f64>` | `PAYLOAD_JESTREAM_SAVE_DURATION_{sum,count,bucket}` |
| `DATUM_RETRIEVE_DURATION` | `Histogram<f64>` | `DATUM_RETRIEVE_DURATION_{sum,count,bucket}` |
| `PROCESSING_TIME` | `Histogram<f64>` | `PROCESSING_TIME_{sum,count,bucket}` |

---

## Histogram Bucket Boundaries

Existing bucket boundaries are preserved exactly by registering an OTel SDK `View` for each
histogram instrument. There are 6 distinct boundary groups across all histograms.

All numaflow-core values are in **microseconds**. All serving values are in **seconds**.

### Group A — 15-minute range (numaflow-core, 10 histograms)

`exponential_buckets_range(100.0, 900_000_000.0, 10)`

Boundaries (µs): `100.0, 592.5, 3510.6, 20800.8, 123246.5, 730244.1, 4326748.7, 25636296.5,
151896895.3, 900000000.0`

Instruments using this group:
- `monovtx_processing_time`, `monovtx_read_time`, `monovtx_ack_time`
- `monovtx_transformer_time`, `monovtx_udf_time`
- `monovtx_sink_time`, `monovtx_fallback_sink_time`, `monovtx_onsuccess_sink_time`
- `forwarder_udf_processing_time`
- `source_forwarder_transformer_processing_time`

### Group B — 10-minute range (numaflow-core, 2 histograms)

`exponential_buckets_range(100.0, 600_000_000.0, 10)`

Boundaries (µs): `100.0, 565.7, 3208.0, 18171.2, 102923.0, 582961.0, 3301927.3, 18702317.3,
105931064.0, 600000000.0`

Instruments: `forwarder_read_processing_time`, `forwarder_ack_processing_time`

### Group C — 20-minute range (numaflow-core, 4 histograms)

`exponential_buckets_range(100.0, 1_200_000_000.0, 10)`

Boundaries (µs): `100.0, 611.9, 3742.3, 22894.3, 140055.7, 856798.2, 5241483.5, 32064898.2,
196157786.3, 1200000000.0`

Instruments: `forwarder_write_processing_time`, `forwarder_processing_time`,
`forwarder_fbsink_write_processing_time`, `forwarder_onsuccess_sink_write_processing_time`

### Group D — 2-minute range (numaflow-core, 4 histograms)

`exponential_buckets_range(100.0, 120_000_000.0, 10)`

Boundaries (µs): `100.0, 473.5, 2243.7, 10626.9, 50334.5, 238409.2, 1129243.0, 5348747.1,
25334752.3, 120000000.0`

Instruments: `isb_jetstream_write_time_total`, `isb_jetstream_read_time_total`,
`isb_jetstream_ack_time_total`, `sqs_producer_publish_latency`

### Group E — 0.001s–1s range (serving, 4 histograms)

`exponential_buckets(0.001, 2.0, 10)` → values in seconds

Boundaries (s): `0.001, 0.002, 0.004, 0.008, 0.016, 0.032, 0.064, 0.128, 0.256, 0.512`

Instruments: `http_requests_duration`, `REQUEST_REGISTER_DURATION`,
`PAYLOAD_JESTREAM_SAVE_DURATION`, `DATUM_RETRIEVE_DURATION`

### Group F — 1s–512s range (serving, 1 histogram)

`exponential_buckets(1.0, 2.0, 10)` → values in seconds

Boundaries (s): `1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0`

Instruments: `PROCESSING_TIME`

---

## Implementation Steps

### Step 1 — `rust/Cargo.toml` (workspace dependencies)

- Add `opentelemetry-prometheus-text-exporter = "0.2"` to `[workspace.dependencies]`
- Add `metrics` feature to `opentelemetry_sdk` (currently only `rt-tokio`)

```toml
opentelemetry_sdk = { version = "0.31", features = ["rt-tokio", "metrics"] }
opentelemetry-prometheus-text-exporter = { version = "0.2", default-features = false }
```

### Step 2 — Crate `Cargo.toml` files

**`numaflow/Cargo.toml`** — add:
```toml
opentelemetry-prometheus-text-exporter = { workspace = true }
opentelemetry_sdk = { workspace = true }
```

**`numaflow-core/Cargo.toml`** — add:
```toml
opentelemetry = { workspace = true }
opentelemetry_sdk = { workspace = true }
opentelemetry-prometheus-text-exporter = { workspace = true }
```

**`serving/Cargo.toml`** — add:
```toml
opentelemetry = { workspace = true }
opentelemetry_sdk = { workspace = true }
opentelemetry-prometheus-text-exporter = { workspace = true }
```

### Step 3 — `numaflow/src/telemetry.rs` — add `init_metrics()`

New public function alongside `init_telemetry()`. Returns `Arc<PrometheusExporter>` to the
caller, which passes it into the metrics HTTP server.

```rust
pub fn init_metrics() -> Result<Arc<PrometheusExporter>, Box<dyn Error>> {
    // 1. Prometheus pull reader
    let prometheus_exporter = PrometheusExporter::builder()
        .without_scope_info()
        .build()?;

    // 2. OTLP push reader (HTTP/proto, same transport as traces)
    let otlp_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .build()?;
    let periodic_reader = PeriodicReader::builder(otlp_exporter).build();

    // 3. Resource (reused from tracer setup)
    let resource = build_resource();

    // 4. Provider with both readers and all histogram Views
    let provider = SdkMeterProvider::builder()
        .with_reader(prometheus_exporter.clone())
        .with_reader(periodic_reader)
        .with_resource(resource)
        .with_views(histogram_views())
        .build();

    opentelemetry::global::set_meter_provider(provider.clone());
    let _ = METER_PROVIDER.set(provider);
    Ok(Arc::new(prometheus_exporter))
}

pub fn shutdown_metrics() {
    if let Some(provider) = METER_PROVIDER.get() {
        if let Err(e) = provider.shutdown() {
            warn!("Failed to shutdown meter provider: {e}");
        }
    }
}
```

`histogram_views()` returns a `Vec<Box<dyn ...>>` with one `View` per histogram instrument,
setting `Aggregation::ExplicitBucketHistogram { boundaries: vec![...], record_min_max: false }`.
A helper `fn histogram_view(name: &str, boundaries: Vec<f64>) -> Box<dyn ...>` reduces
boilerplate. The 6 boundary groups above are defined as constants.

`build_resource()` is extracted from `build_tracer_provider` so both tracer and meter share the
same `Resource` (same `service.version`, etc.).

### Step 4 — `numaflow/src/main.rs` — call `init_metrics()`

In each subcommand branch that eventually calls a metrics server:

```rust
let prometheus_exporter = telemetry::init_metrics()?;
// pass prometheus_exporter into the run() / start_metrics_https_server() call
```

`shutdown_metrics()` called alongside `telemetry::shutdown()` on process exit.

### Step 5 — `numaflow-core/src/metrics/mod.rs` — rewrite metric definitions

**Registry removal:** Delete `GlobalRegistry`, `GLOBAL_REGISTRY`, `global_registry()`, and all
`prometheus_client` imports and registry plumbing.

**Meter:** Each metric struct (`GlobalMetrics`, `MonoVtxMetrics`, `PipelineMetrics`, etc.)
creates its instruments from the global meter at initialization time:

```rust
fn meter() -> opentelemetry::metrics::Meter {
    opentelemetry::global::meter("numaflow-core")
}
```

**Type mapping:**

| prometheus-client | OTel SDK instrument | Notes |
|---|---|---|
| `Family<_, Counter<u64>>` | `Counter<u64>` | `.add(n, &kv)` at record time |
| `Family<_, Gauge<i64>>` | `Gauge<i64>` | `.record(v, &kv)` |
| `Family<_, Gauge<f64, AtomicU64>>` | `Gauge<f64>` | `.record(v, &kv)` |
| `Family<_, Histogram>` | `Histogram<f64>` | `.record(v, &kv)` |

OTel instruments are `Clone + Send + Sync` — no external mutex needed. The `OnceLock`
singleton pattern is retained for lazy initialization; the instruments themselves are stored
directly in the struct fields (no `parking_lot::Mutex`).

**Label conversion helper:** The existing label builder functions (`mvtx_forward_metric_labels`,
`pipeline_metric_labels`, etc.) return `Vec<(String, String)>`. A crate-private helper converts
these to `Vec<KeyValue>` at record time:

```rust
fn to_kv(labels: &[(impl AsRef<str>, impl AsRef<str>)]) -> Vec<KeyValue> {
    labels.iter()
        .map(|(k, v)| KeyValue::new(k.as_ref().to_string(), v.as_ref().to_string()))
        .collect()
}
```

Call sites change from:
```rust
monovertex_metrics().read_total.get_or_create(&labels).inc_by(n);
```
to:
```rust
monovertex_metrics().read_total.add(n, &to_kv(&labels));
```

**`MetricsState<C>` struct:** Add `pub prometheus_exporter: Arc<PrometheusExporter>`.

**`metrics_handler()`:** Replace `encode(&mut buffer, &registry.lock())` with
`state.prometheus_exporter.export(&mut buffer)`. Update `Content-Type` to
`text/plain; version=0.0.4; charset=utf-8`.

**`start_metrics_https_server()` signature:** Add
`prometheus_exporter: Arc<PrometheusExporter>` parameter; store in `MetricsState`.

### Step 6 — `numaflow-core/src/metrics/sqs.rs` — rewrite

Same pattern as Step 5. Three instruments (`Histogram<f64>`, `Counter<u64>` × 2), no registry,
no mutex.

### Step 7 — `serving/src/metrics.rs` — rewrite

Same pattern as Step 5. Additionally:
- Fix the missing `Content-Type` header in `metrics_handler()` (current code sets none):
  set `text/plain; version=0.0.4; charset=utf-8`.
- Fix the copy-paste bug where `request_register_duplicate_count` was registered with the
  `request_register_fail_count` instance (the OTel rewrite naturally fixes this since
  instruments are created independently).
- `start_https_metrics_server()` gains `prometheus_exporter: Arc<PrometheusExporter>` parameter.

### Step 8 — Remove `prometheus-client`

Once the build is clean, remove `prometheus-client` from:
- `rust/Cargo.toml` (`[workspace.dependencies]`)
- `numaflow-core/Cargo.toml`
- `serving/Cargo.toml`

Also remove `parking_lot` from any crate that only used it for `Mutex<Registry>` (check whether
it is used elsewhere before removing).

### Step 9 — Build and test

```
cargo build --workspace
cargo test --workspace
```

The unit tests in `numaflow-core/src/metrics/mod.rs` that assert on emitted metric text
(`assert!(output.contains("monovtx_read_total"))`, etc.) will need updating if the whitespace
or comment style of the new exporter differs from `prometheus-client`. The metric names
themselves will not change.

---

## Files Changed

| File | Change |
|---|---|
| `rust/Cargo.toml` | Add `opentelemetry-prometheus-text-exporter`; add `metrics` feature to `opentelemetry_sdk` |
| `rust/numaflow/Cargo.toml` | Add `opentelemetry-prometheus-text-exporter`, `opentelemetry_sdk` deps |
| `rust/numaflow-core/Cargo.toml` | Add `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-prometheus-text-exporter`; remove `prometheus-client` |
| `rust/serving/Cargo.toml` | Add `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-prometheus-text-exporter`; remove `prometheus-client` |
| `rust/numaflow/src/telemetry.rs` | Add `init_metrics()`, `shutdown_metrics()`, `histogram_views()`, refactor `build_resource()` |
| `rust/numaflow/src/main.rs` | Call `init_metrics()`, pass `Arc<PrometheusExporter>` to subcommand runners |
| `rust/numaflow-core/src/metrics/mod.rs` | Full rewrite: remove registry, replace all `prometheus-client` types with OTel SDK instruments, update handler and server signature |
| `rust/numaflow-core/src/metrics/sqs.rs` | Rewrite: remove registry, use OTel SDK instruments |
| `rust/serving/src/metrics.rs` | Rewrite: remove registry, use OTel SDK instruments, fix Content-Type header, fix copy-paste bug |

---

## Out of Scope

- gRPC/tonic OTLP transport (existing TODO in `telemetry.rs`; deferred)
- Setting `service.name` on the OTel resource (existing TODO; deferred)
- Populating `resource_attributes()` from Go parity (existing TODO; deferred)
- Any changes to the watermark, liveness, or readiness handlers
- Any changes to the TLS configuration of the metrics servers
