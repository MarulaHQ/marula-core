//! The versioned observability contract: metric, log and health surfaces.
//!
//! Purpose: give an external consumer (the premium observability pack, an
//! operator's dashboard, a federation bundle) something to depend on that is
//! NOT "whatever series happen to exist today". Every metric this process
//! family emits is described here — name, kind, unit, attributes, stability
//! and which process produces it — and the whole table is versioned as one
//! artifact.
//!
//! The version rule, enforced by `contract::tests`:
//!
//! * The published artifact is `contract/metrics.v{CONTRACT_VERSION}.json`,
//!   committed beside this file.
//! * Renaming, removing, changing the kind/unit/attributes of a metric, or
//!   changing its stability, changes the serialized table. A test compares
//!   the table against the committed artifact for the CURRENT version and
//!   fails on any difference.
//! * The only way out of that failure is to bump [`CONTRACT_VERSION`], which
//!   points the test at a file that does not exist yet, so the new version
//!   must be written deliberately. The old file stays in the tree: it is the
//!   contract a consumer built against, and it must keep explaining what that
//!   consumer can rely on.
//! * Adding a descriptor is the one change that does NOT need a bump. A new
//!   metric takes nothing away from a consumer: every entry the artifact
//!   already listed keeps its name, kind, unit and attributes, so the file
//!   stays truthful for anyone who built against it. The current version's
//!   artifact is rewritten in place and the diff should be purely additive —
//!   a rule pack derives its series names from this file, and a bump would
//!   orphan every rule that has not been repointed yet.
//!
//! Stability meanings (the consumer's side of the promise):
//!
//! * [`Stability::Stable`] — the name, kind, unit and attribute set may not
//!   change without a major [`CONTRACT_VERSION`] bump. Panels built on it
//!   keep working across minor versions.
//! * [`Stability::Experimental`] — emitted, but may change or disappear at
//!   any time without notice. Safe to chart for diagnosis; not safe to build
//!   an SLA on.
//!
//! Non-purpose: no alert thresholds (this lane ships hooks, not rules), no
//! dashboard definitions, no sampling policy, no retention.

/// The contract revision. Bump on ANY change to [`METRICS`], [`LOG_FIELDS`]
/// or [`HEALTH_SURFACES`] — the test in this module fails otherwise.
///
/// v2 (Wave 52 lane R12): `marula.org.ingest_requests` added, and the worker's
/// `/livez` and `/readyz` probes plus the `draining` field joined
/// [`HEALTH_SURFACES`]. The process-lifecycle lane introduced them in code
/// without the contract noticing, which is the drift this artifact exists to
/// catch. `contract/metrics.v1.json` stays in the tree: it is what a consumer
/// built against before this revision.
///
/// v3 (Wave 52 lane R4): the eight reconciliation gauges. Six report work that
/// nothing re-attempted before — an index backfill, a stream replay, a lost
/// workflow wake, two report-delivery retry states — and two report each
/// consumer group's distance from the stream's bounds, including the case
/// Redis's own `lag` cannot express. Additive: no v2 name changed.
/// `contract/metrics.v2.json` stays in the tree for the same reason v1 did.
pub const CONTRACT_VERSION: u32 = 3;

/// The OTel instrument kind a metric is recorded with. This is part of the
/// contract: a counter read as a gauge produces nonsense, so a kind change is
/// a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Monotonic sum. Chart with `rate()` / `increase()`.
    Counter,
    /// Sum that can go down. Chart as the value itself.
    UpDownCounter,
    /// Last-value snapshot. Chart as the value itself.
    Gauge,
    /// Distribution. Chart with quantiles.
    Histogram,
}

impl MetricKind {
    /// The token used in the published artifact.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::UpDownCounter => "up_down_counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// How much a consumer may rely on a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    /// Name, kind, unit and attributes are frozen within a major version.
    Stable,
    /// May change or vanish without a version bump.
    Experimental,
}

impl Stability {
    /// The token used in the published artifact.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Experimental => "experimental",
        }
    }
}

/// One metric's published description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDescriptor {
    /// Fully-qualified series name, always `marula.`-prefixed.
    pub name: &'static str,
    /// Instrument kind the producer records with.
    pub kind: MetricKind,
    /// OTel unit string: `1` for a count, `ms`/`s` for time, `{thing}` for a
    /// counted noun.
    pub unit: &'static str,
    /// The complete attribute set. An attribute not listed here is never
    /// attached, and a consumer may key on exactly these.
    pub attributes: &'static [&'static str],
    /// What the consumer may assume about the name surviving.
    pub stability: Stability,
    /// The process and module that emits it, so a missing series has an
    /// owner to chase rather than a guess.
    pub producer: &'static str,
    /// One sentence: what the number means.
    pub summary: &'static str,
}

/// Every metric the core emits. This table IS the contract.
pub const METRICS: &[MetricDescriptor] = &[
    MetricDescriptor {
        name: "marula.http.requests",
        kind: MetricKind::Counter,
        unit: "{request}",
        attributes: &["route", "method", "status"],
        stability: Stability::Stable,
        producer: "core-api otel_http::track",
        summary: "HTTP requests by route template, method and status code",
    },
    MetricDescriptor {
        name: "marula.http.request.duration",
        kind: MetricKind::Histogram,
        unit: "ms",
        attributes: &["route", "method"],
        stability: Stability::Stable,
        producer: "core-api otel_http::track",
        summary: "End-to-end HTTP request duration by route template",
    },
    MetricDescriptor {
        name: "marula.org.requests",
        kind: MetricKind::Counter,
        unit: "{request}",
        attributes: &["org_id", "status_class"],
        stability: Stability::Stable,
        producer: "core-api otel_http::record_org",
        summary: "Authenticated requests attributed to an org, by status class",
    },
    MetricDescriptor {
        name: "marula.org.ingest_requests",
        kind: MetricKind::Counter,
        unit: "{request}",
        attributes: &["org_id", "source"],
        stability: Stability::Stable,
        producer: "core-api routes::{webhook_receiver,alerts}::ingest",
        summary: "Authenticated machine-ingest requests attributed to an org, by source",
    },
    MetricDescriptor {
        name: "marula.live.sessions",
        kind: MetricKind::UpDownCounter,
        unit: "{session}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "core-api routes::events",
        summary: "Currently open SSE live-stream sessions",
    },
    MetricDescriptor {
        name: "marula.outbox.lag_ms",
        kind: MetricKind::Gauge,
        unit: "ms",
        attributes: &[],
        stability: Stability::Stable,
        producer: "core-events relay",
        summary: "Age of the oldest unpublished outbox row at the last relay pass",
    },
    MetricDescriptor {
        name: "marula.relay.batch_size",
        kind: MetricKind::Histogram,
        unit: "{row}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "core-events relay",
        summary: "Outbox rows published per relay pass",
    },
    MetricDescriptor {
        name: "marula.queue.depth",
        kind: MetricKind::Gauge,
        unit: "{job}",
        attributes: &["state"],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Jobs depth by state (pending, running, dead)",
    },
    MetricDescriptor {
        name: "marula.queue.dead_age_seconds",
        kind: MetricKind::Gauge,
        unit: "s",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Age of the OLDEST dead-lettered job; absent while none is dead",
    },
    MetricDescriptor {
        name: "marula.queue.jobs",
        kind: MetricKind::Counter,
        unit: "{job}",
        attributes: &["kind", "outcome"],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Job settlements by kind and outcome (done, retried, dead)",
    },
    MetricDescriptor {
        name: "marula.queue.claim_to_apply",
        kind: MetricKind::Histogram,
        unit: "ms",
        attributes: &["kind"],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Claim-to-settle latency per job kind",
    },
    MetricDescriptor {
        name: "marula.consumer.pel_depth",
        kind: MetricKind::Gauge,
        unit: "{entry}",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker consumer loop (stream_health::emit)",
        summary: "Entries delivered to a consumer group and not yet acknowledged",
    },
    MetricDescriptor {
        name: "marula.consumer.lag",
        kind: MetricKind::Gauge,
        unit: "{entry}",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker consumer loop (stream_health::emit)",
        summary: "Stream entries the group has not been delivered yet; absent when the server cannot tell",
    },
    MetricDescriptor {
        name: "marula.consumer.oldest_pending_age_seconds",
        kind: MetricKind::Gauge,
        unit: "s",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker consumer loop (stream_health::emit)",
        summary: "Age of the oldest unacknowledged entry in a consumer group",
    },
    MetricDescriptor {
        name: "marula.workflow.parked_runs",
        kind: MetricKind::Gauge,
        unit: "{run}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Workflow runs waiting to be woken",
    },
    MetricDescriptor {
        name: "marula.workflow.parked_run_age_seconds",
        kind: MetricKind::Gauge,
        unit: "s",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker queue_runner::tick",
        summary: "Idle age of the oldest parked workflow run; absent while none is parked",
    },
    MetricDescriptor {
        name: "marula.backup.age_seconds",
        kind: MetricKind::Gauge,
        unit: "s",
        attributes: &[],
        stability: Stability::Experimental,
        producer: "worker backup_health::sample (stamp written by deploy/backup/backupd.sh)",
        summary: "Age of the last successful backup; absent until a stamp is readable",
    },
    MetricDescriptor {
        name: "marula.sla.sweep.duration",
        kind: MetricKind::Histogram,
        unit: "ms",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker sla_sweep::sweep_once",
        summary: "SLA evaluation sweep duration",
    },
    MetricDescriptor {
        name: "marula.sla.verdicts",
        kind: MetricKind::Counter,
        unit: "{verdict}",
        attributes: &["trigger"],
        stability: Stability::Stable,
        producer: "worker sla_sweep::sweep_inner",
        summary: "SLA ladder verdicts enqueued by trigger class",
    },
    MetricDescriptor {
        name: "marula.db.page1_p95",
        kind: MetricKind::Gauge,
        unit: "ms",
        attributes: &[],
        stability: Stability::Experimental,
        producer: "worker capacity_watch::sample (measured by core-db capacity::sample_page1)",
        summary: "p95 of the page-1 ticket-list SQL for the install's largest org, against the 150 ms budget docs/perf.md sets",
    },
    MetricDescriptor {
        name: "marula.db.page1_sort_index_scan",
        kind: MetricKind::Gauge,
        unit: "1",
        attributes: &[],
        stability: Stability::Experimental,
        producer: "worker capacity_watch::sample (plan read by core-db capacity::sample_page1)",
        summary: "1 when the page-1 ORDER BY is served by an ordered index scan, 0 when the planner sorts",
    },
    MetricDescriptor {
        name: "marula.reconcile.search_index_backlog",
        kind: MetricKind::Gauge,
        unit: "{row}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker reconcile::backfill_search_index",
        summary: "Source entities with no search index row at the last backfill pass; zero means caught up",
    },
    MetricDescriptor {
        name: "marula.reconcile.dead_events_replayable",
        kind: MetricKind::Gauge,
        unit: "{entry}",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker reconcile::replay_dead_events",
        summary: "Dead stream entries the next replay pass will attempt, by consumer group",
    },
    MetricDescriptor {
        name: "marula.reconcile.dead_events_unrepairable",
        kind: MetricKind::Gauge,
        unit: "{entry}",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker reconcile::replay_dead_events",
        summary: "Dead stream entries a replay cannot fix, by consumer group",
    },
    MetricDescriptor {
        name: "marula.reconcile.workflow_wake_lost",
        kind: MetricKind::Gauge,
        unit: "{run}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker reconcile::reap_parked_runs",
        summary: "Parked workflow runs whose wake deadline passed with no live wake job",
    },
    MetricDescriptor {
        name: "marula.reconcile.report_retry_pending",
        kind: MetricKind::Gauge,
        unit: "{slot}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker delivery::retry_due",
        summary: "Report slots with a failed delivery due another attempt",
    },
    MetricDescriptor {
        name: "marula.reconcile.report_retry_exhausted",
        kind: MetricKind::Gauge,
        unit: "{slot}",
        attributes: &[],
        stability: Stability::Stable,
        producer: "worker delivery::retry_due",
        summary: "Report slots whose delivery failed with no attempts left",
    },
    MetricDescriptor {
        name: "marula.consumer.gap_entries",
        kind: MetricKind::Gauge,
        unit: "{entry}",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker reconcile::sample_consumer_gaps (read by stream_health::read_gaps)",
        summary: "Entries after a consumer group's position that are still in the stream",
    },
    MetricDescriptor {
        name: "marula.consumer.gap_lost",
        kind: MetricKind::Gauge,
        unit: "1",
        attributes: &["group"],
        stability: Stability::Stable,
        producer: "worker reconcile::sample_consumer_gaps (read by stream_health::read_gaps)",
        summary: "1 when a group's position is older than the stream's first entry: entries it never processed were trimmed away",
    },
];

/// Structured fields an OTLP log record may carry besides the resource
/// attributes. This is the SAME allowlist the redaction rules describe: a
/// field here is an identifier or a closed enum, never user content.
///
/// Resource attributes (`service.name`, `service.namespace`) come from
/// [`crate::telemetry`], not from a call site, and severity/body are the
/// record's own shape — none of them are listed here because a caller cannot
/// set them wrongly.
pub const LOG_FIELDS: &[&str] = &[
    "org_id",
    "user_id",
    "session_id",
    "correlation_id",
    "entry_id",
    "job.kind",
    "kind",
    "outcome",
    "attempts",
    "batch_size",
    "settled",
    "status",
    "error",
];

/// One health surface an external consumer may poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthSurface {
    /// Path, relative to the process's base URL.
    pub path: &'static str,
    /// The process that serves it.
    pub process: &'static str,
    /// The body shape, in the same vocabulary the payload uses.
    pub shape: &'static str,
    /// Stability of the shape.
    pub stability: Stability,
}

/// Every health surface. Two processes, five paths, all versioned here.
///
/// The worker's three paths answer three different questions and an
/// orchestrator uses each one differently: `/livez` decides whether to restart
/// the process, `/readyz` decides whether to send it traffic, and `/healthz`
/// is the human/dashboard view with one key per loop. A consumer that polls
/// only the last one cannot tell a drain from a crash.
pub const HEALTH_SURFACES: &[HealthSurface] = &[
    HealthSurface {
        path: "/api/v1/health",
        process: "core-api",
        shape: "text/plain: the constant `ok` on 200",
        stability: Stability::Stable,
    },
    HealthSurface {
        path: "/livez",
        process: "worker",
        shape: "empty body: 200 while the process answers, nothing else is checked",
        stability: Stability::Stable,
    },
    HealthSurface {
        path: "/readyz",
        process: "worker",
        shape: "application/json: status (ready|not_ready), draining (bool), \
                down (array of loop labels), roles (label); 503 unless ready",
        stability: Stability::Stable,
    },
    HealthSurface {
        path: "/healthz",
        process: "worker",
        shape: "application/json: status (ok|unhealthy), glitchtip (configured|unset), \
                roles (label), draining (bool), and one key per loop whose value is \
                alive|down|disabled; 503 when a loop this process owns is down \
                and it is not draining",
        stability: Stability::Stable,
    },
];

/// The status class a status code collapses to for `marula.org.requests`.
///
/// A closed five-value set on purpose: the raw status code is already on
/// `marula.http.requests`, and multiplying that by org is how a metric store
/// falls over. `status_class` keeps the per-org series bounded by orgs × 5.
#[must_use]
pub const fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serialized contract, in the shape the committed artifact uses.
    /// Kept here (not in the publish path) so the runtime crate carries no
    /// serialization dependency for a test-only artifact.
    fn contract_json() -> serde_json::Value {
        let metrics: Vec<serde_json::Value> = METRICS
            .iter()
            .map(|m| {
                serde_json::json!({
                    "name": m.name,
                    "kind": m.kind.as_str(),
                    "unit": m.unit,
                    "attributes": m.attributes,
                    "stability": m.stability.as_str(),
                    "producer": m.producer,
                    "summary": m.summary,
                })
            })
            .collect();
        let health: Vec<serde_json::Value> = HEALTH_SURFACES
            .iter()
            .map(|h| {
                serde_json::json!({
                    "path": h.path,
                    "process": h.process,
                    "shape": h.shape,
                    "stability": h.stability.as_str(),
                })
            })
            .collect();
        serde_json::json!({
            "contract_version": CONTRACT_VERSION,
            "metrics": metrics,
            "log_fields": LOG_FIELDS,
            "health": health,
        })
    }

    fn gold_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contract")
            .join(format!("metrics.v{CONTRACT_VERSION}.json"))
    }

    /// The gate: the descriptor table must equal the committed artifact for
    /// the current version. Renaming a metric, changing a unit, adding or
    /// dropping an attribute, or flipping its stability all fail here unless
    /// the version is bumped (which sends this test to a file that does not
    /// exist yet, so the new contract has to be written on purpose).
    #[test]
    fn committed_contract_matches_the_descriptor_table() {
        let rendered = serde_json::to_string_pretty(&contract_json()).expect("serialize contract");
        let path = gold_path();
        if std::env::var("MARULA_CONTRACT_WRITE").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().expect("contract dir"))
                .expect("create contract dir");
            std::fs::write(&path, format!("{rendered}\n")).expect("write contract artifact");
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "no published contract at {} ({e}).\n\
                 A version bump needs a deliberate artifact: review the table, then \
                 run `MARULA_CONTRACT_WRITE=1 cargo test -p core-otel contract::` \
                 and commit the new file alongside the version constant.",
                path.display()
            )
        });
        if committed.trim_end() == rendered.trim_end() {
            return;
        }
        // Report the first differing line, not two multi-kilobyte blobs: the
        // point of this failure is to name the metric that moved.
        let published: Vec<&str> = committed.lines().collect();
        let current: Vec<&str> = rendered.lines().collect();
        let first_diff = published
            .iter()
            .zip(current.iter())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| published.len().min(current.len()));
        panic!(
            "the metric contract changed without a version bump.\n\
             first difference at line {}:\n  published: {}\n  current:   {}\n\
             Compare against {}. If the change is intended, bump CONTRACT_VERSION \
             and publish contract/metrics.v{{new}}.json — a rename in place breaks \
             every consumer that built on this file.",
            first_diff + 1,
            published.get(first_diff).unwrap_or(&"<end of file>"),
            current.get(first_diff).unwrap_or(&"<end of file>"),
            path.display()
        );
    }

    #[test]
    fn names_are_unique_and_prefixed() {
        let mut seen = std::collections::BTreeSet::new();
        for m in METRICS {
            assert!(
                m.name.starts_with("marula."),
                "{} must be marula.-prefixed",
                m.name
            );
            assert!(
                seen.insert(m.name),
                "{} appears twice in the contract",
                m.name
            );
        }
    }

    /// Units are drawn from a small vocabulary so a consumer can format a
    /// value without guessing, and time units end in the OTel suffix.
    #[test]
    fn units_come_from_the_agreed_vocabulary() {
        for m in METRICS {
            assert!(
                matches!(
                    m.unit,
                    "s" | "ms"
                        | "1"
                        | "{request}"
                        | "{session}"
                        | "{job}"
                        | "{row}"
                        | "{entry}"
                        | "{run}"
                        | "{verdict}"
                        | "{slot}"
                ),
                "{} declares unit {:?}, which is outside the agreed vocabulary",
                m.name,
                m.unit
            );
        }
    }

    /// Attribute names are lowercase snake_case and carry no value that could
    /// be user content. `org_id` is allowed by the redaction contract; a
    /// credential-shaped name is not.
    #[test]
    fn attribute_names_are_low_cardinality_tokens() {
        for m in METRICS {
            for attr in m.attributes {
                assert!(
                    attr.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "{} attribute {attr:?} is not a lowercase token",
                    m.name
                );
                assert!(
                    crate::redact::redact_secret_like(attr, None).is_none(),
                    "{} attribute {attr:?} is credential-shaped",
                    m.name
                );
            }
        }
    }

    /// The log contract cannot smuggle a secret-shaped field past the
    /// redaction rules: the same helper that guards span attributes guards
    /// this list.
    #[test]
    fn log_fields_survive_the_redaction_rules() {
        for field in LOG_FIELDS {
            assert!(
                crate::redact::redact_secret_like(field, None).is_none(),
                "log field {field:?} is credential-shaped and must not be published"
            );
            assert!(
                !field.is_empty() && field.len() <= 32,
                "log field {field:?} is not a bounded token"
            );
        }
    }

    /// Every worker path the contract publishes must still be ROUTED by the
    /// worker binary. The artifact comparison above cannot catch a probe path
    /// that was renamed in code, because the table and the artifact would move
    /// together on the next write; this reads the router instead. It caught
    /// exactly that in v2: `/livez` and `/readyz` existed in the worker for a
    /// whole lane before the contract knew their names.
    #[test]
    fn published_worker_health_paths_are_routed_by_the_worker() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../worker/src/lib.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let worker: Vec<&HealthSurface> = HEALTH_SURFACES
            .iter()
            .filter(|h| h.process == "worker")
            .collect();
        assert!(
            !worker.is_empty(),
            "the contract must describe at least one worker surface"
        );
        for surface in worker {
            let routed = format!(".route(\"{}\"", surface.path);
            assert!(
                src.contains(&routed),
                "the contract publishes the worker path {} but {} has no {} route — \
                 update HEALTH_SURFACES (and bump CONTRACT_VERSION) when a probe moves",
                surface.path,
                path.display(),
                routed
            );
        }
    }

    #[test]
    fn status_class_covers_every_http_status() {
        for status in 100u16..=599 {
            assert_eq!(
                status_class(status),
                match status / 100 {
                    1 => "1xx",
                    2 => "2xx",
                    3 => "3xx",
                    4 => "4xx",
                    _ => "5xx",
                }
            );
        }
    }
}
