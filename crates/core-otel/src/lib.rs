//! core-otel — the shared OpenTelemetry baseline (TASKLIST 2.18).
//!
//! Purpose: one place where every Marula process (core-api, realtime,
//! worker, marula-core-cli) initializes tracing + metrics so that spans and
//! metrics flow to an OTLP collector when — and only when —
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is set. It also owns the redaction rules
//! for span attributes (`redact`) and the shared metric instruments
//! (`metrics`) so all lanes emit identically-named series.
//!
//! None mode (the default): with no endpoint configured, the OpenTelemetry
//! SDK is never constructed, no global providers are installed, no
//! exporter/network objects exist, and the process runs exactly as before
//! this crate existed (plain `tracing_subscriber::fmt`). Metric helpers
//! degrade to no-op instruments backed by a noop meter provider — a few
//! atomics per call, zero I/O, zero allocation of exporters. This mirrors
//! the none-mode convention of core-mail and core-store.
//!
//! Opt-in mode: set `OTEL_EXPORTER_OTLP_ENDPOINT` to the collector BASE
//! URL (e.g. `http://otel-collector:4318`); per-signal URLs (`/v1/traces`,
//! `/v1/metrics`, `/v1/logs`) are built here — see `telemetry::init`.
//! Spans become W3C-trace-context-aware and are batch-exported as
//! protobuf; metrics are periodically exported; tracing events flow as
//! OTLP logs.
//!
//! Redaction contract (enforced by construction, tested in `redact`):
//! - Route identity is ALWAYS the axum route TEMPLATE
//!   (`/api/tickets/{id}`), never the raw path — raw paths carry ids,
//!   tokens in query strings (?token=, ?code=), and unbounded cardinality.
//! - Never recorded anywhere: Authorization headers, cookies, JWT/API-key
//!   values, passwords, request/response bodies, email addresses, ticket
//!   titles or any other user content.
//! - Allowed identifiers only: `org_id`, `user_id`, `session_id`,
//!   `job.kind`, queue/outcome enums, route templates, status codes.
//!
//! Non-purpose: no dashboards or alerting policy lives here (see
//! deploy/observability), no log shipping (logs stay on stdout for the
//! platform log driver; Loki receives them via the collector's file/docker
//! receivers if operators want them), no sampling strategy knobs beyond
//! `OTEL_TRACES_SAMPLER`-style env passthrough left to the SDK defaults
//! (parent-based always-on at this stage).
//!
//! Instrument naming: every metric is prefixed `marula.` and documented
//! in `metrics` module docs together with what a dashboard should chart.
//!
//! The published contract for external consumers lives in `contract`: the
//! versioned metric/log/health table an outside pack may depend on, with a
//! test that fails when a name changes without a version bump.

pub mod contract;
pub mod filter;
pub mod metrics;
pub mod redact;
pub mod telemetry;

pub use telemetry::{init, TelemetryGuard, TelemetryMode};

/// The contract revision an external consumer checks its assumptions against.
pub use contract::CONTRACT_VERSION;
