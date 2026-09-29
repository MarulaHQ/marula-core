//! The sync engine: batches of vendor-normalized [`FeederRecord`]s
//! pushed through a [`FeederSink`], with an honest run report.
//!
//! Semantics (binding per the CI7 conformance gate):
//!
//! * every record goes through the feed-upsert seam — never a direct
//!   asset write, never a blind insert;
//! * the same record re-synced with unchanged content is a REPLAY
//!   (deterministic idempotency key): no twin, no link churn;
//! * the same record with changed content is its own source's update —
//!   the link it created is refreshed, other connectors' links are
//!   untouched (that enforcement is the core's CI2 unique key; the
//!   engine never works around it);
//! * none mode counts as "would have synced" and performs zero I/O.

use uuid::Uuid;

use crate::model::FeederRecord;
use crate::sink::{FeederSink, SinkError};

/// How a run applies: full apply, or a plan-only dry run (report
/// without any sink call).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApplyMode {
    /// Apply every record through the sink.
    #[default]
    Apply,
    /// No sink calls; the report describes what WOULD happen.
    DryRun,
}

/// The sync configuration: one run feeds ONE Marula client (the
/// vendor-org → Marula-client mapping is the operator transport's
/// decision, not the engine's guess).
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// The Marula client the sync's records belong to.
    pub client_id: Uuid,
    /// Apply or dry-run.
    pub apply: ApplyMode,
}

impl SyncConfig {
    pub fn new(client_id: Uuid) -> Self {
        Self {
            client_id,
            apply: ApplyMode::Apply,
        }
    }

    pub fn dry_run(client_id: Uuid) -> Self {
        Self {
            client_id,
            apply: ApplyMode::DryRun,
        }
    }
}

/// What one run did. Every counter is additive evidence for the CI gate
/// (`Synced` includes replays: the sync ran, the core resolved it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncReport {
    /// Records submitted to the run.
    pub total: usize,
    /// Records the sink applied (Apply mode) — includes idempotent
    /// replays.
    pub synced: usize,
    /// Would-be syncs under none mode (Disabled) or DryRun: zero I/O
    /// performed for these, counted honestly.
    pub would_sync: usize,
    /// Of the synced records: how many created a fresh canonical item.
    pub created: usize,
    /// How many deterministically resolved an existing item (identity
    /// keys, cross-source adoption included).
    pub resolved_existing: usize,
    /// How many auto-merged at/above the org threshold.
    pub auto_merged: usize,
    /// Reviewable merge candidates the core queued across the run.
    pub queued_candidates: usize,
    /// Observed values that contradicted curated values (surfaced, never
    /// applied).
    pub contradictions: usize,
    /// Records the sink refused, with reasons.
    pub errors: Vec<SyncError>,
}

/// One failed record, with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncError {
    pub source_system: String,
    pub external_id: String,
    pub error: String,
}

impl SyncReport {
    /// True when every submitted record applied or was honestly counted.
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The sync engine. Stateless: construct per run.
pub struct FeederSync;

impl FeederSync {
    /// Pushes every record through the sink under `config`, producing
    /// the run report. Per-record failure never aborts the batch — a
    /// feeder keeps feeding; the report carries every refusal.
    pub async fn run(
        config: &SyncConfig,
        records: &[FeederRecord],
        sink: &dyn FeederSink,
    ) -> SyncReport {
        let mut report = SyncReport {
            total: records.len(),
            ..SyncReport::default()
        };
        for record in records {
            match config.apply {
                ApplyMode::DryRun => {
                    report.would_sync += 1;
                }
                ApplyMode::Apply => match sink.feed_upsert(record, config.client_id).await {
                    Ok(outcome) => {
                        report.synced += 1;
                        if outcome.created_item {
                            report.created += 1;
                        }
                        if outcome.resolved_existing {
                            report.resolved_existing += 1;
                        }
                        if outcome.auto_merged {
                            report.auto_merged += 1;
                        }
                        report.queued_candidates += outcome.queued_candidates;
                        report.contradictions += outcome.contradictions.len();
                    }
                    Err(SinkError::Disabled) => {
                        report.would_sync += 1;
                    }
                    Err(err) => report.errors.push(SyncError {
                        source_system: record.source_system.clone(),
                        external_id: record.external_id.clone(),
                        error: err.to_string(),
                    }),
                },
            }
        }
        report
    }
}
