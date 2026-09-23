//! worker/dedupe_sweep — the Jev intelligent-dedupe workflow's scheduled
//! tick (Wave 22 lane 2 scope addition).
//!
//! Purpose: claim due `dedupe_workflows` orgs (the claim ADVANCES
//! `next_run_at` in the same statement — at-most-once per schedule, the
//! pm_sweep/report-delivery crash semantics) and dispatch ONE effect call
//! per due org to core-api's internal seam (`/api/internal/dedupe/run`).
//! core-api runs the pass through the SAME runner a manual/workbench
//! trigger uses: one `decision_record_dedupe` ask per pending candidate
//! (suggestion-grade, capped per run), report-only by default, and
//! auto_merge folds at/above the workflow's threshold through the same
//! audited + undoable merge code path a human accept uses.
//!
//! Degrade posture is absolute (ADR 0011 §3): the internal token unset or
//! a rejected call never retries in-loop — the claim already advanced, so
//! the org simply sees its next scheduled pass at the next cadence; the
//! pass outcome report is the operator's source of truth.
//!
//! Non-purpose: no queue writes here (core-api owns every domain table),
//! no notifications (the internal runner delivers them), no asks from
//! this crate (the pass lives behind the seam).

use serde_json::json;
use sqlx::PgPool;
use tokio::sync::watch;

/// The internal seam this sweep dispatches to.
pub const PATH: &str = "/api/internal/dedupe/run";

/// One sweep: claim due orgs, dispatch one effect call per org.
/// Returns the number of dispatched runs.
pub async fn sweep_once(
    pool: &sqlx::PgPool,
    effects: &impl effects::EffectApi,
    batch_limit: i64,
) -> anyhow::Result<usize> {
    let mut conn = pool.acquire().await?;
    let orgs = core_db::entity_dedupe::list_due_dedupe_orgs(&mut conn, batch_limit).await?;
    if orgs.is_empty() {
        return Ok(0);
    }
    let mut dispatched = 0usize;
    for org_id in orgs {
        let outcome = effects.post(PATH, json!({ "org_id": org_id })).await;
        match outcome {
            effects::EffectOutcome::Applied => dispatched += 1,
            effects::EffectOutcome::Rejected => {
                tracing::warn!(org = %org_id, "core-api rejected the dedupe run; slot consumed");
            }
            effects::EffectOutcome::Transient => {
                tracing::error!(org = %org_id, "dedupe run transport failure; slot consumed");
            }
        }
    }
    Ok(dispatched)
}

/// The loop: one sweep per interval, shutdown-clean.
pub async fn run_dedupe_sweep(
    pool: PgPool,
    effects: std::sync::Arc<effects::ReqwestEffectApi>,
    interval_secs: u64,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => return,
            result = sweep_once(&pool, effects.as_ref(), 200) => {
                match result {
                    Ok(0) => {
                        tokio::time::sleep(std::time::Duration::from_secs(
                            interval_secs.max(1),
                        ))
                        .await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "dedupe sweep failed");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

use crate::effects;
