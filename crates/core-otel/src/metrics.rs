//! Shared metric instruments — one naming scheme for every lane.
//!
//! Purpose: lazily-built, process-wide instruments on the global meter.
//! In opt-in mode they export via the periodic OTLP reader installed by
//! [`crate::telemetry::init`]; in none mode the global provider is a
//! no-op and every helper below costs a few atomics.
//!
//! Instrument catalog (what dashboards should chart — mirrored by the
//! starter Grafana dashboard in deploy/observability):
//!
//! | Instrument                     | Type        | Attributes          | Chart it as |
//! |--------------------------------|-------------|---------------------|-------------|
//! | `marula.http.requests`       | counter     | route, method, status | request rate by route/status (`rate()`); error ratio = status ≥ 400 share |
//! | `marula.http.request.duration` | histogram (ms) | route, method   | p50/p95/p99 latency per route template |
//! | `marula.ws.sessions`         | up-down     | –                   | current connected WS sessions (gauge) |
//! | `marula.outbox.lag_ms`       | gauge       | –                   | event-pipeline lag: age of oldest unpublished outbox row at last relay pass |
//! | `marula.relay.batch_size`    | histogram   | –                   | relay throughput distribution per tick |
//! | `marula.queue.depth`         | gauge       | state=pending\|running\|dead | backlog health; alert on dead > 0 sustained |
//! | `marula.queue.jobs`          | counter     | kind, outcome=done\|retried\|dead | job settle rates; dead-letter counter = outcome="dead" |
//! | `marula.queue.claim_to_apply` | histogram (ms) | kind            | claim→settle latency per job kind |
//! | `marula.sla.sweep.duration`  | histogram (ms) | –                | sweep cost over time |
//! | `marula.sla.verdicts`        | counter     | trigger=risk\|breach\|escalation | SLA breach counters (trigger="breach") |
//!
//! Cardinality rules: `route` is always the axum TEMPLATE (see
//! [`crate::redact::route_template`]); `kind`/`outcome`/`status` come from
//! small closed enums. Nothing user-derived ever becomes a label.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge, Histogram, UpDownCounter};
use std::sync::OnceLock;

struct Instruments {
    http_requests: Counter<u64>,
    http_duration: Histogram<f64>,
    ws_sessions: UpDownCounter<i64>,
    outbox_lag_ms: Gauge<u64>,
    relay_batch: Histogram<u64>,
    queue_depth: Gauge<u64>,
    jobs: Counter<u64>,
    claim_to_apply_ms: Histogram<f64>,
    sla_sweep: Histogram<f64>,
    sla_verdicts: Counter<u64>,
}

static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();

fn instruments() -> &'static Instruments {
    INSTRUMENTS.get_or_init(|| {
        let meter = global::meter("marula");
        Instruments {
            http_requests: meter
                .u64_counter("marula.http.requests")
                .with_description("HTTP requests by route template, method, status")
                .build(),
            http_duration: meter
                .f64_histogram("marula.http.request.duration")
                .with_unit("ms")
                .with_description("End-to-end HTTP request duration")
                .build(),
            ws_sessions: meter
                .i64_up_down_counter("marula.ws.sessions")
                .with_description("Currently open WebSocket sessions")
                .build(),
            outbox_lag_ms: meter
                .u64_gauge("marula.outbox.lag_ms")
                .with_unit("ms")
                .with_description("Age of oldest unpublished outbox row at last relay pass")
                .build(),
            relay_batch: meter
                .u64_histogram("marula.relay.batch_size")
                .with_description("Outbox rows published per relay pass")
                .build(),
            queue_depth: meter
                .u64_gauge("marula.queue.depth")
                .with_description("Jobs table depth by state (pending/running/dead)")
                .build(),
            jobs: meter
                .u64_counter("marula.queue.jobs")
                .with_description("Job settlements by kind and outcome")
                .build(),
            claim_to_apply_ms: meter
                .f64_histogram("marula.queue.claim_to_apply")
                .with_unit("ms")
                .with_description("Claim-to-settle latency per job")
                .build(),
            sla_sweep: meter
                .f64_histogram("marula.sla.sweep.duration")
                .with_unit("ms")
                .with_description("SLA evaluation sweep duration")
                .build(),
            sla_verdicts: meter
                .u64_counter("marula.sla.verdicts")
                .with_description("SLA ladder verdicts enqueued by trigger class")
                .build(),
        }
    })
}

/// Records one finished HTTP request. `route` MUST be a route template
/// ([`crate::redact::route_template`]); callers pass `method.as_str()`.
pub fn http_request(route: &str, method: &str, status: u16, duration_ms: f64) {
    let i = instruments();
    i.http_requests.add(
        1,
        &[
            opentelemetry::KeyValue::new("route", route.to_string()),
            opentelemetry::KeyValue::new("method", method.to_string()),
            opentelemetry::KeyValue::new("status", i64::from(status)),
        ],
    );
    i.http_duration.record(
        duration_ms,
        &[
            opentelemetry::KeyValue::new("route", route.to_string()),
            opentelemetry::KeyValue::new("method", method.to_string()),
        ],
    );
}

/// One WS session opened (upgrade accepted).
pub fn ws_session_opened() {
    instruments().ws_sessions.add(1, &[]);
}

/// One WS session closed (socket dropped or send failed).
pub fn ws_session_closed() {
    instruments().ws_sessions.add(-1, &[]);
}

/// Relay pass result: batch size histogram + outbox-lag gauge update.
pub fn relay_pass(batch_size: u64, outbox_lag_ms: Option<u64>) {
    let i = instruments();
    i.relay_batch.record(batch_size, &[]);
    if let Some(lag) = outbox_lag_ms {
        i.outbox_lag_ms.record(lag, &[]);
    }
}

/// Queue-depth snapshot pushed by the worker each tick.
pub fn queue_depth(pending: u64, running: u64, dead: u64) {
    let i = instruments();
    for (state, value) in [("pending", pending), ("running", running), ("dead", dead)] {
        i.queue_depth
            .record(value, &[opentelemetry::KeyValue::new("state", state)]);
    }
}

/// One job settled: outcome counter + claim→apply latency sample.
pub fn job_settled(kind: &str, outcome: JobOutcome, claim_to_apply_ms: f64) {
    let i = instruments();
    let kind_attr = opentelemetry::KeyValue::new("kind", kind.to_string());
    i.jobs.add(
        1,
        &[
            kind_attr.clone(),
            opentelemetry::KeyValue::new("outcome", outcome.as_str()),
        ],
    );
    i.claim_to_apply_ms.record(claim_to_apply_ms, &[kind_attr]);
}

/// Job settlement outcomes (closed set — never free-form).
#[derive(Debug, Clone, Copy)]
pub enum JobOutcome {
    Done,
    Retried,
    Dead,
}

impl JobOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Retried => "retried",
            Self::Dead => "dead",
        }
    }
}

/// SLA sweep duration sample (called once per sweep pass).
pub fn sla_sweep(duration_ms: f64) {
    instruments().sla_sweep.record(duration_ms, &[]);
}

/// One SLA ladder verdict enqueued; trigger class is risk|breach|escalation.
pub fn sla_verdict(trigger: &str) {
    instruments().sla_verdicts.add(
        1,
        &[opentelemetry::KeyValue::new("trigger", trigger.to_string())],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// None-mode fast path: with no OTLP endpoint configured, every helper
    /// runs against the noop global meter — must not panic and must not
    /// install any exporter. (Serial: env vars are process-global.)
    #[test]
    #[serial_test::serial]
    fn all_helpers_are_safe_in_none_mode() {
        unsafe { std::env::remove_var(crate::telemetry::ENDPOINT_VAR) };
        http_request("/api/tickets/{id}", "GET", 200, 12.5);
        http_request("unmatched", "POST", 500, 3.0);
        ws_session_opened();
        ws_session_closed();
        relay_pass(7, Some(150));
        relay_pass(0, None);
        queue_depth(1, 0, 0);
        job_settled("sla.verdict", JobOutcome::Done, 4.2);
        job_settled("automation.effects", JobOutcome::Dead, 900.0);
        sla_sweep(33.3);
        sla_verdict("breach");
    }

    #[test]
    fn outcome_names_are_closed_and_stable() {
        assert_eq!(JobOutcome::Done.as_str(), "done");
        assert_eq!(JobOutcome::Retried.as_str(), "retried");
        assert_eq!(JobOutcome::Dead.as_str(), "dead");
    }
}
