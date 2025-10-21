use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry_prometheus::PrometheusExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use prometheus::{Encoder, Registry, TextEncoder};
use tracing_subscriber::{EnvFilter, fmt};

static LOGGING: OnceLock<()> = OnceLock::new();
static TELEMETRY: OnceCell<TelemetryState> = OnceCell::new();
static METRICS: OnceCell<MetricsHandles> = OnceCell::new();

struct TelemetryState {
    _provider: SdkMeterProvider,
    registry: Registry,
}

struct MetricsHandles {
    tool_invocations: Counter<u64>,
    tool_results: Counter<u64>,
    cache_hits: Counter<u64>,
    reward_histogram: Histogram<f64>,
    cycle_latency_histogram: Histogram<f64>,
    stage_latency_histogram: Histogram<f64>,
}

/// Initialize tracing and metrics exporters. Safe to call multiple times.
pub fn init() -> Result<()> {
    configure_logging();
    configure_metrics()?;
    Ok(())
}

fn configure_logging() {
    LOGGING.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let subscriber = fmt::Subscriber::builder()
            .with_env_filter(filter)
            .json()
            .with_current_span(false)
            .with_span_list(false)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

fn configure_metrics() -> Result<&'static TelemetryState> {
    TELEMETRY.get_or_try_init(|| {
        let registry = Registry::new();
        let exporter = build_exporter(&registry)?;

        let provider = SdkMeterProvider::builder()
            .with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                "swe-grep",
            )]))
            .with_reader(exporter)
            .build();

        global::set_meter_provider(provider.clone());

        let meter = global::meter("swe-grep");
        let tool_invocations = meter
            .u64_counter("swegrep_tool_invocations_total")
            .with_description("Number of tool invocations executed by SWE-Grep")
            .init();
        let tool_results = meter
            .u64_counter("swegrep_tool_results_total")
            .with_description("Number of matches produced by tool invocations")
            .init();
        let cache_hits = meter
            .u64_counter("swegrep_cache_hits_total")
            .with_description("Cache hits recorded during search execution")
            .init();
        let reward_histogram = meter
            .f64_histogram("swegrep_reward_score")
            .with_description("Reward signal produced per reasoning cycle")
            .init();
        let cycle_latency_histogram = meter
            .f64_histogram("swegrep_cycle_latency_ms")
            .with_description("End-to-end latency of a reasoning cycle in milliseconds")
            .init();
        let stage_latency_histogram = meter
            .f64_histogram("swegrep_stage_latency_ms")
            .with_description("Latency of individual pipeline stages in milliseconds")
            .init();

        METRICS
            .set(MetricsHandles {
                tool_invocations,
                tool_results,
                cache_hits,
                reward_histogram,
                cycle_latency_histogram,
                stage_latency_histogram,
            })
            .map_err(|_| anyhow!("metrics handles already initialized"))?;

        Ok(TelemetryState {
            _provider: provider,
            registry,
        })
    })
}

fn build_exporter(registry: &Registry) -> Result<PrometheusExporter> {
    opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()
        .context("failed to build Prometheus exporter")
}

fn metrics() -> Option<&'static MetricsHandles> {
    METRICS.get()
}

fn state() -> Option<&'static TelemetryState> {
    TELEMETRY.get()
}

/// Record a tool invocation for the given tool identifier.
pub fn record_tool_invocation(tool: &'static str) {
    if let Some(metrics) = metrics() {
        metrics
            .tool_invocations
            .add(1, &[KeyValue::new("tool", tool)]);
    }
}

/// Record the number of results produced by a tool.
pub fn record_tool_results(tool: &'static str, count: usize) {
    if let Some(metrics) = metrics() {
        metrics
            .tool_results
            .add(count as u64, &[KeyValue::new("tool", tool)]);
    }
}

/// Record cache hits for the given cache identifier.
pub fn record_cache_hits(cache: &'static str, hits: usize) {
    if hits == 0 {
        return;
    }
    if let Some(metrics) = metrics() {
        metrics
            .cache_hits
            .add(hits as u64, &[KeyValue::new("cache", cache)]);
    }
}

/// Record the reward accumulated during a reasoning cycle.
pub fn record_reward(value: f32) {
    if let Some(metrics) = metrics() {
        metrics.reward_histogram.record(value as f64, &[]);
    }
}

/// Record the total latency of a reasoning cycle in milliseconds.
pub fn record_cycle_latency(latency_ms: u64) {
    if let Some(metrics) = metrics() {
        metrics
            .cycle_latency_histogram
            .record(latency_ms as f64, &[]);
    }
}

/// Record the latency for a named stage in milliseconds.
pub fn record_stage_latency(stage: &'static str, latency_ms: u64) {
    if latency_ms == 0 {
        return;
    }
    if let Some(metrics) = metrics() {
        metrics
            .stage_latency_histogram
            .record(latency_ms as f64, &[KeyValue::new("stage", stage)]);
    }
}

/// Render all currently collected metrics in Prometheus text format.
pub fn export_prometheus() -> Result<String> {
    let state = state().ok_or_else(|| anyhow!("telemetry not initialized"))?;
    let encoder = TextEncoder::new();
    let metric_families = state.registry.gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .context("failed to encode metrics")?;
    String::from_utf8(buffer).context("metrics buffer is not valid UTF-8")
}
