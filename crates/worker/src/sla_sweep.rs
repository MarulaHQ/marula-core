//! worker/sla_sweep — the periodic SLA evaluation loop.
//!
//! Purpose: on a host-paced cadence, read every open ticket's SLA clocks
//! and decide which escalation-ladder steps are DUE. All decisions use
//! `now()` fetched from Postgres — never the host clock — so a skewed or
//! drifting worker cannot invent progress; it can only observe business
//! time as the database sees it. Due steps become durable `sla.verdict`
//! jobs; core-api applies them fire-once through its internal surface.
//!
//! Non-purpose: no writes outside the queue tables (§4.1), no notification
//! creation, no HTTP at all — this loop only produces durable work items.

use serde_json::json;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use tokio::sync::watch;

/// One sweep over all due candidates; returns verdict jobs enqueued.
///
/// Telemetry (TASKLIST 2.18): a `sla.sweep` span per pass (candidate +
/// enqueued counts), a `marula.sla.sweep.duration` histogram sample, and
/// `marula.sla.verdicts` counters classified by trigger class
/// (risk/breach/escalation).
pub async fn sweep_once(pool: &PgPool, batch_limit: i64) -> anyhow::Result<usize> {
    use tracing::Instrument as _;
    let started = std::time::Instant::now();
    let span = tracing::info_span!("sla.sweep");
    let result = sweep_inner(pool, batch_limit)
        .instrument(span.clone())
        .await;
    if let Ok((candidates, enqueued)) = &result {
        span.record("candidates", *candidates);
        span.record("enqueued", *enqueued);
    } else {
        span.record("failed", true);
    }
    core_otel::metrics::sla_sweep(started.elapsed().as_secs_f64() * 1000.0);
    result.map(|(_candidates, enqueued)| enqueued)
}

/// Returns `(candidates_seen, verdict_jobs_enqueued)`.
async fn sweep_inner(pool: &PgPool, batch_limit: i64) -> anyhow::Result<(usize, usize)> {
    // THE clock for this entire sweep: Postgres, read exactly once per pass.
    let db_now: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT now()").fetch_one(pool).await?;

    let candidates = {
        let mut tx = pool.begin().await?;
        let rows = core_db::sla::list_evaluation_candidates(&mut *tx, batch_limit).await?;
        tx.commit().await?;
        rows
    };
    if candidates.is_empty() {
        return Ok((0, 0));
    }

    let policy_ids: Vec<uuid::Uuid> = candidates
        .iter()
        .map(|c| c.policy_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let ticket_ids: Vec<uuid::Uuid> = candidates.iter().map(|c| c.ticket_id).collect();
    let (steps, fired) = {
        let mut tx = pool.begin().await?;
        let steps = core_db::sla::list_steps_for_policies(&mut *tx, &policy_ids).await?;
        let fired = core_db::sla::list_fired_for_tickets(&mut *tx, &ticket_ids).await?;
        tx.commit().await?;
        (steps, fired)
    };

    // Engine-shaped ladder per policy + the notify list per step id.
    let mut ladders: HashMap<uuid::Uuid, Vec<core_domain::sla_engine::LadderStep>> = HashMap::new();
    let mut notify_by_step: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
    for s in steps {
        let Some(trigger) = core_domain::sla_engine::StepTrigger::parse(&s.trigger_kind) else {
            tracing::warn!(step_id = %s.id, "unknown trigger_kind stored; skipping step");
            continue;
        };
        ladders
            .entry(s.policy_id)
            .or_default()
            .push(core_domain::sla_engine::LadderStep {
                id: s.id,
                step_index: s.step_index,
                trigger,
                at_pct: s.at_pct,
            });
        notify_by_step.insert(s.id, s.notify_user_ids.clone());
    }
    // Fire-once ledger is PER TICKET — grouping keeps one ticket's history
    // from suppressing another's identical step.
    let mut fired_by_ticket: HashMap<
        uuid::Uuid,
        HashSet<(uuid::Uuid, core_domain::sla_engine::SlaTarget)>,
    > = HashMap::new();
    for (ticket_id, step_id, target) in fired {
        if let Some(tg) = core_domain::sla_engine::SlaTarget::parse(&target) {
            fired_by_ticket
                .entry(ticket_id)
                .or_default()
                .insert((step_id, tg));
        }
    }

    let candidates_seen = candidates.len();
    let mut enqueued = 0usize;
    for candidate in candidates {
        let Some(ladder) = ladders.get(&candidate.policy_id) else {
            continue;
        };
        if ladder.is_empty() {
            continue;
        }
        let Ok(priority) = candidate.priority.parse::<core_domain::Priority>() else {
            continue;
        };
        let Ok(targets) = serde_json::from_value::<
            HashMap<String, core_domain::entities::SlaTargets>,
        >(candidate.targets.clone()) else {
            continue;
        };
        if !targets.contains_key(priority.as_str()) {
            continue;
        }

        let (fr_pos, res_pos) = core_domain::sla_engine::ticket_positions(
            &candidate.state,
            &candidate.calendar,
            candidate.created_at,
            db_now,
        );
        let empty = HashSet::new();
        let fired_set = fired_by_ticket.get(&candidate.ticket_id).unwrap_or(&empty);
        for verdict in core_domain::sla_engine::evaluate_ladder(ladder, fired_set, fr_pos, res_pos)
        {
            let budget_label = match verdict.target {
                core_domain::sla_engine::SlaTarget::FirstResponse => "first response",
                core_domain::sla_engine::SlaTarget::Resolution => "resolution",
            };
            let kind_label = match verdict.trigger.notification_kind() {
                core_domain::entities::NotificationKind::SlaRisk => "SLA risk",
                core_domain::entities::NotificationKind::SlaBreach => "SLA breach",
                _ => "SLA escalation",
            };
            let payload = json!({
                "org_id": candidate.org_id,
                "ticket_id": candidate.ticket_id,
                "target": verdict.target.as_str(),
                "step_id": verdict.step_id,
                "trigger": verdict.trigger.as_str(),
                "notify_user_ids": notify_by_step.get(&verdict.step_id).cloned().unwrap_or_default(),
                "title": format!("[{}] {} at {}% of {} budget",
                    candidate.number, kind_label, verdict.at_pct, budget_label),
                "body": Option::<String>::None,
            });
            let job = core_db::queue::NewJob {
                org_id: candidate.org_id,
                kind: "sla.verdict".to_string(),
                payload,
                max_attempts: crate::queue_runner::queue_max_attempts(),
            };
            match core_db::queue::enqueue(pool, &job).await {
                Ok(_) => {
                    enqueued += 1;
                    // SLA breach counter (TASKLIST 2.18): classify by the
                    // ladder trigger's notification kind — "breach" is the
                    // SLA-breach counter, "risk" risk warnings, everything
                    // else is escalation.
                    let trigger_class = match verdict.trigger.notification_kind() {
                        core_domain::entities::NotificationKind::SlaBreach => "breach",
                        core_domain::entities::NotificationKind::SlaRisk => "risk",
                        _ => "escalation",
                    };
                    core_otel::metrics::sla_verdict(trigger_class);
                }
                Err(error) => {
                    tracing::error!(%error, ticket_id = %candidate.ticket_id, "failed to enqueue verdict")
                }
            }
        }
    }
    Ok((candidates_seen, enqueued))
}

/// Host-paced loop; every DECISION still reads the DB clock.
pub async fn run_sla_sweep(
    pool: PgPool,
    interval_secs: u64,
    batch_limit: i64,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = shutdown.changed() => return,
            _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs.max(1))) => {
                match sweep_once(&pool, batch_limit).await {
                    Ok(n) if n > 0 => {
                        tracing::info!(enqueued = n, "sla sweep enqueued verdicts");
                    }
                    Ok(_) => {}
                    Err(error) => tracing::error!(%error, "sla sweep failed"),
                }
            }
        }
    }
}
