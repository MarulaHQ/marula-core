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
//! | `marula.live.sessions`       | up-down     | –                   | current connected SSE live-stream sessions (gauge) |
//! | `marula.outbox.lag_ms`       | gauge       | –                   | event-pipeline lag: age of oldest unpublished outbox row at last relay pass |
//! | `marula.relay.batch_size`    | histogram   | –                   | relay throughput distribution per tick |
//! | `marula.queue.depth`         | gauge       | state=pending\|running\|dead | backlog health; alert on dead > 0 sustained |
//! | `marula.queue.dead_age_seconds` | gauge (s) | –                | how long the oldest dead letter has been dead |
//! | `marula.queue.jobs`          | counter     | kind, outcome=done\|retried\|dead | job settle rates; dead-letter counter = outcome="dead" |
//! | `marula.queue.claim_to_apply` | histogram (ms) | kind            | claim→settle latency per job kind |
//! | `marula.consumer.pel_depth`  | gauge       | group               | unacked entries held by a stream consumer group |
//! | `marula.consumer.lag`        | gauge       | group               | entries the group has not been delivered yet |
//! | `marula.consumer.oldest_pending_age_seconds` | gauge (s) | group | how long the oldest unacked entry has sat |
//! | `marula.workflow.parked_runs` | gauge      | –                   | workflow runs waiting to be woken |
//! | `marula.workflow.parked_run_age_seconds` | gauge (s) | –       | how long the oldest parked run has waited |
//! | `marula.backup.age_seconds`  | gauge (s)   | –                   | age of the last successful backup |
//! | `marula.org.requests`        | counter     | org_id, status_class=1xx…5xx | per-org request rate and error ratio |
//! | `marula.org.ingest_requests` | counter     | org_id, source=webhook\|alert | per-org machine-ingest rate (the traffic a connector or pack generates) |
//! | `marula.sla.sweep.duration`  | histogram (ms) | –                | sweep cost over time |
//! | `marula.sla.verdicts`        | counter     | trigger=risk\|breach\|escalation | SLA breach counters (trigger="breach") |
//!
//! The table above is the human view; [`crate::contract::METRICS`] is the
//! machine view and the one an external consumer depends on. A metric that is
//! not in the contract is either missing from this file or must be added to
//! both, and the contract test fails when the two disagree across a version.
//!
//! Cardinality rules: `route` is always the axum TEMPLATE (see
//! [`crate::redact::route_template`]); `kind`/`outcome`/`status`/`source` come
//! from small closed enums. Nothing user-derived ever becomes a label. `org_id`
//! is the one per-tenant label, and it is only ever paired with the five-value
//! `status_class` or the two-value `source` set, so per-org series stay
//! bounded by orgs × 5 and orgs × 2.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge, Histogram, UpDownCounter};
use opentelemetry::KeyValue;
use std::sync::OnceLock;

struct Instruments {
    http_requests: Counter<u64>,
    http_duration: Histogram<f64>,
    org_requests: Counter<u64>,
    org_ingest_requests: Counter<u64>,
    live_sessions: UpDownCounter<i64>,
    outbox_lag_ms: Gauge<u64>,
    relay_batch: Histogram<u64>,
    queue_depth: Gauge<u64>,
    dead_age_seconds: Gauge<u64>,
    jobs: Counter<u64>,
    claim_to_apply_ms: Histogram<f64>,
    consumer_pel_depth: Gauge<u64>,
    consumer_lag: Gauge<u64>,
    consumer_oldest_pending_age_seconds: Gauge<u64>,
    parked_runs: Gauge<u64>,
    parked_run_age_seconds: Gauge<u64>,
    backup_age_seconds: Gauge<u64>,
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
            org_requests: meter
                .u64_counter("marula.org.requests")
                .with_description("Authenticated requests attributed to an org, by status class")
                .build(),
            org_ingest_requests: meter
                .u64_counter("marula.org.ingest_requests")
                .with_description("Authenticated machine ingest attributed to an org, by source")
                .build(),
            live_sessions: meter
                .i64_up_down_counter("marula.live.sessions")
                .with_description("Currently open SSE live-stream sessions")
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
            dead_age_seconds: meter
                .u64_gauge("marula.queue.dead_age_seconds")
                .with_unit("s")
                .with_description("Age of the oldest dead-lettered job")
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
            consumer_pel_depth: meter
                .u64_gauge("marula.consumer.pel_depth")
                .with_description("Entries delivered to a consumer group and not yet acked")
                .build(),
            consumer_lag: meter
                .u64_gauge("marula.consumer.lag")
                .with_description("Stream entries a consumer group has not been delivered yet")
                .build(),
            consumer_oldest_pending_age_seconds: meter
                .u64_gauge("marula.consumer.oldest_pending_age_seconds")
                .with_unit("s")
                .with_description("Age of the oldest unacknowledged entry in a consumer group")
                .build(),
            parked_runs: meter
                .u64_gauge("marula.workflow.parked_runs")
                .with_description("Workflow runs waiting to be woken")
                .build(),
            parked_run_age_seconds: meter
                .u64_gauge("marula.workflow.parked_run_age_seconds")
                .with_unit("s")
                .with_description("Idle age of the oldest parked workflow run")
                .build(),
            backup_age_seconds: meter
                .u64_gauge("marula.backup.age_seconds")
                .with_unit("s")
                .with_description("Age of the last successful backup")
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
            KeyValue::new("route", route.to_string()),
            KeyValue::new("method", method.to_string()),
            KeyValue::new("status", i64::from(status)),
        ],
    );
    i.http_duration.record(
        duration_ms,
        &[
            KeyValue::new("route", route.to_string()),
            KeyValue::new("method", method.to_string()),
        ],
    );
}

/// One authenticated request attributed to an org.
///
/// `org_id` is a UUID string the auth layer already resolved; `status` is the
/// response status, collapsed to [`crate::contract::status_class`] here so the
/// caller cannot invent a sixth class. Unauthenticated requests never reach a
/// call site: there is no org to attribute them to, and a synthetic
/// `anonymous` org would make the series look like a tenant.
pub fn org_request(org_id: &str, status: u16) {
    let class = crate::contract::status_class(status);
    instruments().org_requests.add(
        1,
        &[
            KeyValue::new("org_id", org_id.to_string()),
            KeyValue::new("status_class", class),
        ],
    );
}

/// The closed set of machine-ingest surfaces that report per-org volume.
///
/// A token here is a PROPERTY OF THE CORE, not of the caller: the ingest
/// handler picks its own constant, so a pack cannot invent a series by
/// sending a name. That is what keeps `source` bounded at two values while
/// the whole point of the metric is traffic the core does not otherwise see.
pub const INGEST_SOURCES: &[&str] = &["webhook", "alert"];

/// One authenticated machine-ingest request attributed to an org.
///
/// Why this exists next to [`org_request`] rather than as another attribute on
/// it: the two are counted at different boundaries. `org_request` runs in
/// middleware on the authenticated router and answers "what did this tenant's
/// people and API keys do". The ingest endpoints are mounted OUTSIDE that
/// router because they authenticate with an HMAC over the body and a
/// `source_ref` in the path, so no `AuthedUser` extension ever exists for them
/// and their traffic lands in neither series. This is the one seam where a
/// connector's own work becomes visible to a per-org consumer.
///
/// Emit it only once the source has AUTHENTICATED. A request with a bad
/// signature proves nothing about which tenant it belongs to, and counting it
/// would let anyone add to any org's number by guessing a ref.
pub fn org_ingest(org_id: &str, source: &str) {
    debug_assert!(
        INGEST_SOURCES.contains(&source),
        "{source} is not an ingest source token"
    );
    instruments().org_ingest_requests.add(
        1,
        &[
            KeyValue::new("org_id", org_id.to_string()),
            KeyValue::new("source", source.to_string()),
        ],
    );
}

/// One live-stream session opened (an SSE response body began).
pub fn live_session_opened() {
    instruments().live_sessions.add(1, &[]);
}

/// One live-stream session closed (the response body was dropped, whether the
/// client walked away, the hub shut down, or the stream ended on its own).
pub fn live_session_closed() {
    instruments().live_sessions.add(-1, &[]);
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
            .record(value, &[KeyValue::new("state", state)]);
    }
}

/// Age of the OLDEST dead-lettered job, in seconds.
///
/// `None` means "nothing is dead right now" and emits NO series. That is
/// deliberate: `marula.queue.depth{state="dead"} = 0` already says the
/// dead-letter set is empty, and a stale `0` age from a previous tick would
/// read as "the newest dead letter is brand new" to anyone charting
/// `max_over_time`. Absence plus a zero depth is the honest pair.
pub fn dead_letter_age(age_seconds: Option<u64>) {
    if let Some(age) = age_seconds {
        instruments().dead_age_seconds.record(age, &[]);
    }
}

/// One consumer-group health sample, read from Valkey.
///
/// `depth` is the group's PEL size (delivered, not acked). `lag` is how many
/// stream entries the group has not been delivered at all; `None` means the
/// server could not compute it (a stream trimmed past the group's position),
/// which is reported as absence rather than as `0` — `0` claims "caught up"
/// and that would be a guess. `oldest_pending_age_seconds` is the age of the
/// first unacked entry; with an empty PEL it is `0`, because there genuinely
/// is nothing waiting.
pub fn consumer_group_health(group: &str, depth: u64, lag: Option<u64>, oldest_age_seconds: u64) {
    let i = instruments();
    let group_attr = KeyValue::new("group", group.to_string());
    i.consumer_pel_depth
        .record(depth, std::slice::from_ref(&group_attr));
    i.consumer_oldest_pending_age_seconds
        .record(oldest_age_seconds, std::slice::from_ref(&group_attr));
    if let Some(lag) = lag {
        i.consumer_lag.record(lag, &[group_attr]);
    }
}

/// Parked workflow runs: how many are waiting, and how long the oldest has
/// been waiting. `oldest_age_seconds` is `None` when nothing is parked, and
/// then no age series is emitted (same reasoning as
/// [`dead_letter_age`]).
pub fn workflow_parked(count: u64, oldest_age_seconds: Option<u64>) {
    let i = instruments();
    i.parked_runs.record(count, &[]);
    if let Some(age) = oldest_age_seconds {
        i.parked_run_age_seconds.record(age, &[]);
    }
}

/// Age of the last successful backup, in seconds.
///
/// Only called when a stamp was actually read: there is no "0 seconds since
/// the last backup" for an install that has never taken one, and a gauge that
/// reports `0` forever is worse than a missing series.
pub fn backup_age(age_seconds: u64) {
    instruments().backup_age_seconds.record(age_seconds, &[]);
}

/// One job settled: outcome counter + claim→apply latency sample.
pub fn job_settled(kind: &str, outcome: JobOutcome, claim_to_apply_ms: f64) {
    let i = instruments();
    let kind_attr = KeyValue::new("kind", kind.to_string());
    i.jobs.add(
        1,
        &[
            kind_attr.clone(),
            KeyValue::new("outcome", outcome.as_str()),
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
    instruments()
        .sla_verdicts
        .add(1, &[KeyValue::new("trigger", trigger.to_string())]);
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
        org_request("00000000-0000-0000-0000-000000000001", 200);
        org_request("00000000-0000-0000-0000-000000000001", 503);
        org_ingest("00000000-0000-0000-0000-000000000001", "webhook");
        org_ingest("00000000-0000-0000-0000-000000000001", "alert");
        live_session_opened();
        live_session_closed();
        relay_pass(7, Some(150));
        relay_pass(0, None);
        queue_depth(1, 0, 0);
        dead_letter_age(Some(90));
        dead_letter_age(None);
        consumer_group_health("csat", 0, Some(0), 0);
        consumer_group_health("workflows", 4, None, 12);
        workflow_parked(0, None);
        workflow_parked(2, Some(300));
        backup_age(3600);
        job_settled("sla.verdict", JobOutcome::Done, 4.2);
        job_settled("automation.effects", JobOutcome::Dead, 900.0);
        sla_sweep(33.3);
        sla_verdict("breach");
    }

    /// Every hook added for the premium observability pack has an entry in the
    /// published contract. A metric that exists here but not there is one an
    /// external consumer cannot discover; one that exists there but not here
    /// is a promise with no producer.
    #[test]
    fn every_emitted_metric_is_in_the_published_contract() {
        const EMITTED: &[&str] = &[
            "marula.http.requests",
            "marula.http.request.duration",
            "marula.org.requests",
            "marula.org.ingest_requests",
            "marula.live.sessions",
            "marula.outbox.lag_ms",
            "marula.relay.batch_size",
            "marula.queue.depth",
            "marula.queue.dead_age_seconds",
            "marula.queue.jobs",
            "marula.queue.claim_to_apply",
            "marula.consumer.pel_depth",
            "marula.consumer.lag",
            "marula.consumer.oldest_pending_age_seconds",
            "marula.workflow.parked_runs",
            "marula.workflow.parked_run_age_seconds",
            "marula.backup.age_seconds",
            "marula.sla.sweep.duration",
            "marula.sla.verdicts",
        ];
        let published: std::collections::BTreeSet<&str> =
            crate::contract::METRICS.iter().map(|m| m.name).collect();
        for name in EMITTED {
            assert!(
                published.contains(name),
                "{name} is emitted but missing from contract::METRICS"
            );
        }
        assert_eq!(
            published.len(),
            EMITTED.len(),
            "contract::METRICS lists a metric this module does not emit: {:?}",
            published
                .iter()
                .filter(|name| !EMITTED.contains(name))
                .collect::<Vec<_>>()
        );
    }

    /// The ingest counter's `source` label is a core constant, not a caller
    /// string, and the published contract must describe the same two-attribute
    /// shape. Renaming a token here without touching the descriptor (or the
    /// other way round) fails this test, which is the pair that a dashboard and
    /// a premium pack's query both depend on.
    #[test]
    fn ingest_sources_are_a_closed_pair_the_contract_describes() {
        assert_eq!(
            INGEST_SOURCES,
            &["webhook", "alert"],
            "the ingest source vocabulary is closed: one token per ingest endpoint"
        );
        let descriptor = crate::contract::METRICS
            .iter()
            .find(|m| m.name == "marula.org.ingest_requests")
            .expect("marula.org.ingest_requests is in the contract");
        assert_eq!(descriptor.attributes, &["org_id", "source"]);
        assert_eq!(descriptor.kind, crate::contract::MetricKind::Counter);
        assert_eq!(descriptor.unit, "{request}");
    }

    #[test]
    fn outcome_names_are_closed_and_stable() {
        assert_eq!(JobOutcome::Done.as_str(), "done");
        assert_eq!(JobOutcome::Retried.as_str(), "retried");
        assert_eq!(JobOutcome::Dead.as_str(), "dead");
    }
}
