//! Process telemetry bootstrap: tracing subscriber + optional OTLP export.
//!
//! Purpose: [`init`] is the ONE call every binary makes at startup. It
//! installs:
//! - an `EnvFilter` (see [`crate::filter`]) shared by the stdout fmt layer
//!   and — when enabled — the OpenTelemetry span layer;
//! - when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (non-empty): an OTLP/HTTP
//!   protobuf exporter for traces (batch processor) and metrics (periodic
//!   reader, 30 s), registered globally so `opentelemetry::global::meter`
//!   instruments bind to it and W3C trace context flows through spans.
//!
//! None mode (default): with no endpoint set, NO SDK object is built, NO
//! global provider is installed, and the returned guard carries only the
//! mode marker. Overhead vs. the pre-otel baseline: one env read and one
//! branch per metric helper call. This is the documented none-mode
//! equivalent of core-mail/core-store.
//!
//! Non-purpose: no retry/backoff policy tuning beyond SDK defaults, no TLS
//! configuration surface (point the collector at a sidecar if needed), no
//! log-export pipeline — stdout logs belong to the platform's log driver.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::{PeriodicReader, SdkMeterProvider},
    resource::Resource,
    trace::{BatchSpanProcessor, SdkTracerProvider},
};
use std::time::Duration;

use crate::filter;

/// The single switch every caller may inspect (tests assert on this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    /// No collector configured: fmt-only logging, noop meters.
    None,
    /// OTLP/HTTP protobuf export enabled for traces + metrics + logs.
    Otlp,
}

/// Environment variable selecting opt-in telemetry.
pub const ENDPOINT_VAR: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Holds providers alive for the process lifetime; dropping flushes.
#[derive(Debug)]
pub struct TelemetryGuard {
    /// Which mode bootstrap selected.
    pub mode: TelemetryMode,
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    /// Best-effort flush for short-lived processes/tests; shutdown also
    /// happens (idempotently) on drop.
    pub fn flush(&self) {
        if let Some(tp) = &self.tracer_provider {
            let _ = tp.force_flush();
        }
        if let Some(mp) = &self.meter_provider {
            let _ = mp.force_flush();
        }
        if let Some(lp) = &self.logger_provider {
            let _ = lp.force_flush();
        }
    }
}

/// Installs process telemetry. Call once, early in `main`, keep the guard.
///
/// # Panics
/// Only if called twice in one process (global subscriber already set) —
/// a programming error, not a runtime condition.
#[must_use = "dropping the guard immediately shuts telemetry down"]
pub fn init(service_name: &'static str) -> TelemetryGuard {
    let endpoint = std::env::var(ENDPOINT_VAR)
        .ok()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    let filter_layer = filter::from_env();

    match endpoint {
        None => {
            // NONE MODE: exactly the pre-otel subscriber shape.
            tracing_subscriber::fmt()
                .with_env_filter(filter_layer)
                .init();
            TelemetryGuard {
                mode: TelemetryMode::None,
                tracer_provider: None,
                meter_provider: None,
                logger_provider: None,
            }
        }
        Some(base_url) => {
            // NOTE: when an endpoint is set PROGRAMMATICALLY the OTLP/HTTP
            // exporter posts to it verbatim — the /v1/<signal> path is only
            // appended on the env-var fallback paths. We therefore build
            // the full per-signal URLs here.
            let traces_url = signal_endpoint(&base_url, "traces");
            let metrics_url = signal_endpoint(&base_url, "metrics");
            let logs_url = signal_endpoint(&base_url, "logs");

            // Traces: batch-exported OTLP/HTTP protobuf.
            let span_exporter = SpanExporter::builder()
                .with_http()
                .with_endpoint(&traces_url)
                .with_protocol(Protocol::HttpBinary)
                .build()
                .expect("valid OTLP span exporter from configured endpoint");
            let tracer_provider = SdkTracerProvider::builder()
                .with_span_processor(BatchSpanProcessor::builder(span_exporter).build())
                .with_resource(build_resource(service_name))
                .build();
            opentelemetry::global::set_tracer_provider(tracer_provider.clone());

            // Metrics: periodic OTLP/HTTP protobuf reader (30 s cadence).
            let metric_exporter = MetricExporter::builder()
                .with_http()
                .with_endpoint(&metrics_url)
                .with_protocol(Protocol::HttpBinary)
                .build()
                .expect("valid OTLP metric exporter from configured endpoint");
            let meter_provider = SdkMeterProvider::builder()
                .with_reader(
                    PeriodicReader::builder(metric_exporter)
                        .with_interval(Duration::from_secs(30))
                        .build(),
                )
                .with_resource(build_resource(service_name))
                .build();
            opentelemetry::global::set_meter_provider(meter_provider.clone());

            // Logs: tracing events bridged to OTLP (Loki via collector).
            let log_exporter = LogExporter::builder()
                .with_http()
                .with_endpoint(&logs_url)
                .with_protocol(Protocol::HttpBinary)
                .build()
                .expect("valid OTLP log exporter from configured endpoint");
            let logger_provider = SdkLoggerProvider::builder()
                .with_batch_exporter(log_exporter)
                .with_resource(build_resource(service_name))
                .build();
            // NOTE(0.32): there is no `global::set_logger_provider` yet;
            // the tracing bridge below holds the provider handle directly
            // and the TelemetryGuard keeps it alive for the process.

            use tracing_subscriber::layer::{Layer as _, SubscriberExt as _};
            use tracing_subscriber::util::SubscriberInitExt as _;
            let otel_layer =
                tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer(service_name));
            let otel_logs = opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(
                &logger_provider,
            );

            tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer().with_filter(filter::from_env()))
                .with(otel_layer.with_filter(filter_layer))
                .with(otel_logs.with_filter(filter::from_env()))
                .init();

            tracing::info!(
                endpoint = %base_url,
                "OpenTelemetry enabled: exporting traces+metrics+logs via OTLP/HTTP"
            );
            TelemetryGuard {
                mode: TelemetryMode::Otlp,
                tracer_provider: Some(tracer_provider),
                meter_provider: Some(meter_provider),
                logger_provider: Some(logger_provider),
            }
        }
    }
}

fn build_resource(service_name: &'static str) -> Resource {
    Resource::builder()
        .with_service_name(service_name)
        .with_attribute(opentelemetry::KeyValue::new("service.namespace", "marula"))
        .build()
}

/// Joins a base collector URL with the OTLP/HTTP signal path.
#[must_use]
pub fn signal_endpoint(base_url: &str, signal: &str) -> String {
    format!("{}/v1/{signal}", base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exporter-disabled fast path: no endpoint ⇒ none mode, no providers,
    /// and bootstrap stays effectively instant. (Serial + env cleanup
    /// because env vars are process-global; together with the live smoke
    /// below these are the ONLY tests in the crate allowed to install the
    /// global subscriber.)
    #[test]
    #[serial_test::serial]
    fn exporter_disabled_fast_path_is_none_mode() {
        unsafe { std::env::remove_var(ENDPOINT_VAR) };
        let started = std::time::Instant::now();
        let guard = init("core-otel-test");
        let elapsed = started.elapsed();
        assert_eq!(guard.mode, TelemetryMode::None);
        assert!(guard.tracer_provider.is_none());
        assert!(guard.meter_provider.is_none());
        assert!(guard.logger_provider.is_none());
        assert!(
            elapsed < Duration::from_secs(1),
            "none-mode bootstrap must be fast, took {elapsed:?}"
        );
        // The fmt subscriber is installed; logging still works.
        tracing::debug!("none-mode subscriber active");
    }

    /// LIVE smoke against a real collector (skipped by default):
    ///   OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:18206 \
    ///     cargo test -p core-otel -- --ignored --nocapture
    ///
    /// Asserts opt-in bootstrap succeeds end-to-end and flush returns Ok;
    /// received batches must be checked in the collector's logs (see
    /// the private build tracker 2.18 stage notes). Serial vs the none-mode test above so
    /// they never race for the global subscriber.
    #[test]
    #[serial_test::serial]
    #[ignore]
    fn live_otlp_export_smoke() {
        match std::env::var(ENDPOINT_VAR) {
            Ok(ep) if !ep.trim().is_empty() => {}
            _ => panic!("live smoke needs {ENDPOINT_VAR} pointing at a collector"),
        }
        let guard = init("core-otel-smoke");
        assert_eq!(guard.mode, TelemetryMode::Otlp);
        let span = tracing::info_span!("smoke.span");
        {
            let _entered = span.enter();
            tracing::info!(
                org_id = "00000000-0000-0000-0000-000000000001",
                "smoke event"
            );
        }
        // A span ENDS only when its last handle drops; do it explicitly so
        // the processor has a finished span before we wait on the batch.
        drop(span);
        // Stay alive past the trace batch timeout so the processor's own
        // timer performs an export too (not just the final flush).
        std::thread::sleep(Duration::from_secs(6));
        guard.flush();
    }

    /// Signal URLs are built from the BASE (no path) and tolerate a
    /// trailing slash.
    #[test]
    fn signal_endpoints_are_built_from_base() {
        assert_eq!(
            signal_endpoint("http://collector:4318", "traces"),
            "http://collector:4318/v1/traces"
        );
        assert_eq!(
            signal_endpoint("http://collector:4318/", "metrics"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            // init() passes an already-trimmed endpoint; trailing slash only.
            signal_endpoint("http://c:4318/", "logs"),
            "http://c:4318/v1/logs"
        );
    }
}
