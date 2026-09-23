//! The Jev intelligent-dedupe workflow (Wave 22 lane 2 scope addition):
//! admin-configured, classifier-powered dedupe passes over the merge
//! queues — manual and/or scheduled, report-only or auto-merge, riding
//! the EXISTING `decision_record_dedupe` surface (ADR 0010 §3 #8) and its
//! suggestion-grade posture.
//!
//! Doctrine (all inherited, nothing new invented):
//! * **Gated, never silent-on** (ADR 0011 §3): enabling a workflow
//!   requires the org's decision model to be configured; the typed
//!   refusal is `dedupe_decision_not_configured`. Unconfigured orgs see
//!   the pre-AI product — the queue still runs on deterministic rules.
//! * **Merges stay audited + undoable** (ADR 0003): even in auto_merge
//!   mode every fold goes through the same merge code path a human
//!   accept uses (per-merge split substrate; batch-accepted candidate
//!   rows flip to `accepted`). Below threshold ⇒ queued for the
//!   workbench, never merged.
//! * **Resource-efficient** (ADR 0011 §4): one ask per candidate, capped
//!   per run (truncation is visible in the report, never silent);
//!   client reconciliation is NEVER machine-merged (an operator's
//!   mapping decision) so it is report-only by definition.
//! * **Report back**: every non-dry run stamps the workflow row and
//!   notifies the designated users (`dedupe_report` kind).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant;
use crate::org_tx;
use crate::routes::decision_edge::{self, DecisionAsk};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{Extensions, HeaderMap};
use axum::Json;
use core_domain::entities::entity_dedupe::{
    DedupeRunOutcome, DEDUPE_QUEUE_CLIENTS, DEDUPE_QUEUE_CONFIG_ITEMS, DEDUPE_QUEUE_CONTACTS,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// The per-run ask cap (resource-efficiency law: a pass is bounded and
/// its truncation is visible in the report, never silent).
const RUN_ASK_CAP: usize = 50;
const NOTIFY_CAP: usize = 20;

// ---- wire shapes ------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct DedupeWorkflowView {
    pub workflow: core_domain::entities::entity_dedupe::DedupeWorkflow,
    /// The gate's honest state: whether the org's decision model is
    /// configured right now (the workflow can be stored even when the
    /// model later goes down — the RUN then degrades quiet, per ADR 0010).
    pub decision_configured: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DedupeWorkflowRequest {
    pub enabled: bool,
    pub confidence_threshold: f64,
    /// report_only | auto_merge
    pub mode: String,
    /// Scheduled cadence in hours (None = manual runs only).
    pub interval_hours: Option<i32>,
    pub notify_user_ids: Vec<Uuid>,
    pub entity_kinds: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DedupeRunRequest {
    /// Preview posture: fire the asks (suggestion-grade, recorded) but
    /// stamp nothing, merge nothing, notify no one.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub entity_kinds: Option<Vec<String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DedupeRunResponse {
    pub report: core_domain::entities::entity_dedupe::DedupeRunReport,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InternalDedupeRunRequest {
    pub org_id: Uuid,
}

// ---- the pass runner (shared by manual, scheduled, and workbench trigger) ----

/// Runs one dedupe pass over the requested queues. The decision edge is
/// the Jev seam: every ask rides the `decision_record_dedupe` surface,
/// is recorded suggestion-grade in the append-only decision store, and
/// the workbench renders it inline (silent whenever the org's model is
/// unconfigured — degrade-quiet, never a 5xx).
pub async fn run_dedupe_pass(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    entity_kinds: &[String],
    threshold: f64,
    auto_merge: bool,
) -> core_domain::entities::entity_dedupe::DedupeRunReport {
    let started = chrono::Utc::now();
    let mut report = core_domain::entities::entity_dedupe::DedupeRunReport {
        queues_scanned: entity_kinds.to_vec(),
        scanned: 0,
        asked: 0,
        above_threshold: 0,
        auto_merged: 0,
        truncated: false,
        started_at: started,
        finished_at: started,
        outcomes: Vec::new(),
    };

    for kind in entity_kinds {
        match kind.as_str() {
            DEDUPE_QUEUE_CONFIG_ITEMS => {
                let mut tx = match state.pool.begin().await {
                    Ok(tx) => tx,
                    Err(err) => {
                        tracing::debug!(%err, "dedupe pass config-items pool begin failed");
                        continue;
                    }
                };
                if core_db::set_org_and_user(&mut tx, org_id, user_id)
                    .await
                    .is_err()
                {
                    continue;
                }
                let candidates = match core_db::config_identity::list_candidates(
                    &mut tx,
                    org_id,
                    Some("pending"),
                )
                .await
                {
                    Ok(c) => c,
                    Err(err) => {
                        tracing::debug!(%err, "dedupe pass ci queue read failed");
                        continue;
                    }
                };
                for candidate in &candidates {
                    report.scanned += 1;
                    if report.asked >= RUN_ASK_CAP {
                        report.truncated = true;
                        break;
                    }
                    let left = match core_db::assets::get_asset(
                        &mut *tx,
                        org_id,
                        candidate.primary_config_item_id,
                    )
                    .await
                    {
                        Ok(l) => l,
                        Err(err) => {
                            tracing::debug!(%err, "dedupe pass ci read failed");
                            continue;
                        }
                    };
                    let right = match core_db::assets::get_asset(
                        &mut *tx,
                        org_id,
                        candidate.candidate_config_item_id,
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(err) => {
                            tracing::debug!(%err, "dedupe pass ci read failed");
                            continue;
                        }
                    };
                    let label = format!("{} ← {}", left.tag, right.tag);
                    let ask = DecisionAsk {
                        org_id,
                        user_id,
                        surface_key: "decision_record_dedupe",
                        entity_type: "ci_merge_candidate",
                        entity_id: Some(candidate.id),
                        state: serde_json::json!({
                            "left": {
                                "name": left.name,
                                "email": fields_email(&left.fields),
                            },
                            "right": {
                                "name": right.name,
                                "email": fields_email(&right.fields),
                            },
                        }),
                        request_id: format!("decision-dedupe-{}", candidate.id),
                        inventories: Default::default(),
                    };
                    report.asked += 1;
                    let (confidence, answers_json) = match ask_decision(state, ask).await {
                        Some((confidence, answers)) => (confidence, answers),
                        None => {
                            report.outcomes.push(DedupeRunOutcome {
                                queue: DEDUPE_QUEUE_CONFIG_ITEMS.into(),
                                candidate_id: candidate.id,
                                label,
                                confidence: None,
                                disposition: "quiet".into(),
                            });
                            continue;
                        }
                    };
                    let disposition = if confidence >= threshold {
                        report.above_threshold += 1;
                        if auto_merge {
                            if apply_ci_candidate(state, org_id, user_id, candidate.id).await {
                                report.auto_merged += 1;
                                "auto_merged".to_string()
                            } else {
                                "auto_merge_failed".to_string()
                            }
                        } else {
                            "suggested".to_string()
                        }
                    } else {
                        "below_threshold".to_string()
                    };
                    let _ = answers_json;
                    report.outcomes.push(DedupeRunOutcome {
                        queue: DEDUPE_QUEUE_CONFIG_ITEMS.into(),
                        candidate_id: candidate.id,
                        label,
                        confidence: Some(confidence),
                        disposition,
                    });
                }
            }
            DEDUPE_QUEUE_CONTACTS => {
                let mut tx = match state.pool.begin().await {
                    Ok(tx) => tx,
                    Err(err) => {
                        tracing::debug!(%err, "dedupe pass contacts pool begin failed");
                        continue;
                    }
                };
                if core_db::set_org_and_user(&mut tx, org_id, user_id)
                    .await
                    .is_err()
                {
                    continue;
                }
                let candidates = match core_db::entity_dedupe::list_entity_candidates(
                    &mut tx,
                    org_id,
                    Some("contact"),
                    Some("pending"),
                )
                .await
                {
                    Ok(c) => c,
                    Err(err) => {
                        tracing::debug!(%err, "dedupe pass contact queue read failed");
                        continue;
                    }
                };
                for candidate in &candidates {
                    report.scanned += 1;
                    if report.asked >= RUN_ASK_CAP {
                        report.truncated = true;
                        break;
                    }
                    let Some(right_id) = candidate.candidate_entity_id else {
                        continue;
                    };
                    let left = match core_db::clients::get_contact(
                        &mut *tx,
                        org_id,
                        candidate.primary_entity_id,
                    )
                    .await
                    {
                        Ok(l) => l,
                        Err(err) => {
                            tracing::debug!(%err, "dedupe pass contact read failed");
                            continue;
                        }
                    };
                    let right =
                        match core_db::clients::get_contact(&mut *tx, org_id, right_id).await {
                            Ok(r) => r,
                            Err(err) => {
                                tracing::debug!(%err, "dedupe pass contact read failed");
                                continue;
                            }
                        };
                    let label = format!("{} ← {}", left.name, right.name);
                    let ask = DecisionAsk {
                        org_id,
                        user_id,
                        surface_key: "decision_record_dedupe",
                        entity_type: "contact_merge_candidate",
                        entity_id: Some(candidate.id),
                        state: serde_json::json!({
                            "left": {
                                "name": left.name,
                                "email": left.email.clone().unwrap_or_default(),
                            },
                            "right": {
                                "name": right.name,
                                "email": right.email.clone().unwrap_or_default(),
                            },
                        }),
                        request_id: format!("decision-dedupe-{}", candidate.id),
                        inventories: Default::default(),
                    };
                    report.asked += 1;
                    let Some(confidence) = ask_decision(state, ask).await.map(|(c, _)| c) else {
                        report.outcomes.push(DedupeRunOutcome {
                            queue: DEDUPE_QUEUE_CONTACTS.into(),
                            candidate_id: candidate.id,
                            label,
                            confidence: None,
                            disposition: "quiet".into(),
                        });
                        continue;
                    };
                    let disposition = if confidence >= threshold {
                        report.above_threshold += 1;
                        if auto_merge {
                            if apply_contact_candidate(state, org_id, user_id, candidate.id).await {
                                report.auto_merged += 1;
                                "auto_merged".to_string()
                            } else {
                                "auto_merge_failed".to_string()
                            }
                        } else {
                            "suggested".to_string()
                        }
                    } else {
                        "below_threshold".to_string()
                    };
                    report.outcomes.push(DedupeRunOutcome {
                        queue: DEDUPE_QUEUE_CONTACTS.into(),
                        candidate_id: candidate.id,
                        label,
                        confidence: Some(confidence),
                        disposition,
                    });
                }
            }
            // Client reconciliation is an OPERATOR mapping decision by
            // definition (org_map doctrine): a pass scans the queue for
            // the report but never asks and never acts.
            DEDUPE_QUEUE_CLIENTS => {
                let mut tx = match state.pool.begin().await {
                    Ok(tx) => tx,
                    Err(_) => continue,
                };
                if core_db::set_org_and_user(&mut tx, org_id, user_id)
                    .await
                    .is_err()
                {
                    continue;
                }
                let candidates = match core_db::entity_dedupe::list_entity_candidates(
                    &mut tx,
                    org_id,
                    Some("client"),
                    Some("pending"),
                )
                .await
                {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                for candidate in &candidates {
                    report.scanned += 1;
                    report.outcomes.push(DedupeRunOutcome {
                        queue: DEDUPE_QUEUE_CLIENTS.into(),
                        candidate_id: candidate.id,
                        label: format!(
                            "{} ← {}",
                            candidate.vendor_org_id.clone().unwrap_or_default(),
                            candidate.vendor_label.clone().unwrap_or_default(),
                        ),
                        confidence: Some(candidate.confidence),
                        disposition: "operator_decision".into(),
                    });
                }
            }
            _ => {}
        }
    }

    report.finished_at = chrono::Utc::now();
    report
}

/// Fires ONE ask and distills the recorded answer to its strongest
/// numeric signal. Quiet (unconfigured model, master switch off) ⇒ None.
async fn ask_decision(state: &AppState, ask: DecisionAsk<'_>) -> Option<(f64, serde_json::Value)> {
    let org_id = ask.org_id;
    match decision_edge::decide(state, ask).await {
        Ok(Ok(run)) => {
            let answers = decision_edge::answers_to_json(
                &run.config,
                &run.response,
                &std::collections::HashMap::new(),
            );
            let confidence = answer_confidence(&answers);
            Some((confidence?, answers))
        }
        Ok(Err(quiet)) => {
            tracing::debug!(org = %org_id, reason = %quiet.reason(), "dedupe pass quiet");
            None
        }
        Err(err) => {
            tracing::debug!(%err, "dedupe pass ask failed");
            None
        }
    }
}

/// Applies ONE ci candidate merge through the SAME audited, undoable
/// code path a human accept uses (candidate flips to `accepted`).
async fn apply_ci_candidate(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    candidate_id: Uuid,
) -> bool {
    let outcome = org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        let candidate = core_db::config_identity::get_candidate(&mut ctx.tx, org_id, candidate_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if candidate.status != core_domain::entities::CiMergeStatus::Pending {
            return Ok((ctx, ()));
        }
        core_db::config_identity::merge_items(
            &mut ctx.tx,
            org_id,
            candidate.primary_config_item_id,
            candidate.candidate_config_item_id,
            Some(candidate.id),
            Some(candidate.confidence),
            "intelligent dedupe (Jev) auto-merge at/above workflow threshold",
            &candidate.evidence,
            Some(user_id),
        )
        .await?;
        core_db::config_identity::decide_candidate(
            &mut ctx.tx,
            org_id,
            candidate.id,
            core_domain::entities::CiMergeStatus::Accepted.as_str(),
            user_id,
        )
        .await?;
        ctx.emit(
            core_domain::events::types::CONFIG_ITEM_MERGED,
            "config_item",
            Some(candidate.primary_config_item_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ConfigItemMerged {
                primary_config_item_id: candidate.primary_config_item_id,
                merged_config_item_id: candidate.candidate_config_item_id,
                org_id,
                candidate_id: Some(candidate.id),
                confidence: Some(candidate.confidence),
                reason: candidate.reason.clone(),
                auto: true,
                merged_by: Some(user_id),
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await;
    outcome.is_ok()
}

/// Applies ONE contact candidate merge (audited + undoable via split).
async fn apply_contact_candidate(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    candidate_id: Uuid,
) -> bool {
    let outcome = org_tx::with_org_tx(state, org_id, user_id, |mut ctx| async move {
        let candidate =
            core_db::entity_dedupe::get_entity_candidate(&mut ctx.tx, org_id, candidate_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if candidate.status != core_domain::entities::entity_dedupe::EntityMergeStatus::Pending {
            return Ok((ctx, ()));
        }
        let Some(merged_id) = candidate.candidate_entity_id else {
            return Ok((ctx, ()));
        };
        core_db::entity_dedupe::merge_contacts(
            &mut ctx.tx,
            org_id,
            candidate.primary_entity_id,
            merged_id,
            Some(candidate.id),
            Some(candidate.confidence),
            "intelligent dedupe (Jev) auto-merge at/above workflow threshold",
            &candidate.evidence,
            Some(user_id),
        )
        .await?;
        core_db::entity_dedupe::decide_entity_candidate(
            &mut ctx.tx,
            org_id,
            candidate.id,
            core_domain::entities::entity_dedupe::EntityMergeStatus::Accepted.as_str(),
            user_id,
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "contact.merged".into(),
                entity_type: Some("contact".into()),
                entity_id: Some(candidate.primary_entity_id.to_string()),
                detail: serde_json::json!({
                    "merged": merged_id,
                    "origin": "intelligent_dedupe",
                    "candidate": candidate.id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, ()))
    })
    .await;
    outcome.is_ok()
}
/// The recorded answers' strongest numeric signal (probability / level /
/// confidence — the same loose answer shape every surface's store keeps).
fn answer_confidence(answers: &serde_json::Value) -> Option<f64> {
    let obj = answers.as_object()?;
    let mut best: Option<f64> = None;
    for (_, entry) in obj {
        let v = entry
            .get("probability")
            .or_else(|| entry.get("level"))
            .and_then(serde_json::Value::as_f64)
            .or_else(|| entry.get("confidence").and_then(serde_json::Value::as_f64));
        if let Some(v) = v {
            best = Some(best.map_or(v, |b: f64| b.max(v)));
        }
    }
    best
}

fn fields_email(fields: &serde_json::Value) -> String {
    fields
        .get("email")
        .or_else(|| fields.get("Email"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Stamps the report + notifies the designated users (bounded). One org
/// notification per user; the kind rides the typed `dedupe_report` slot
/// (preference-gated upstream in the preference screens).
async fn deliver_report(
    state: &AppState,
    org_id: Uuid,
    report: &core_domain::entities::entity_dedupe::DedupeRunReport,
    notify_user_ids: &[Uuid],
    _auto_merge: bool,
) {
    if notify_user_ids.is_empty() {
        return;
    }
    let title = format!(
        "Intelligent dedupe run: {} scanned, {} above threshold, {} auto-merged",
        report.scanned, report.above_threshold, report.auto_merged
    );
    let body = serde_json::to_string(&report).unwrap_or_default();
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::debug!(%err, "dedupe report notify failed");
            return;
        }
    };
    if core_db::set_org_and_user(&mut tx, org_id, org_id)
        .await
        .is_err()
    {
        return;
    }
    for user_id in notify_user_ids.iter().take(NOTIFY_CAP) {
        let _ = core_db::notifications::insert(
            &mut *tx,
            Uuid::now_v7(),
            org_id,
            *user_id,
            core_domain::entities::NotificationKind::DedupeReport.as_str(),
            None,
            &title,
            Some(&body),
        )
        .await;
    }
    tx.commit().await.ok();
}

// ---- org-facing routes -------------------------------------------------------

/// GET /api/dedupe/workflow — the org's workflow + the honest gate state.
#[utoipa::path(operation_id = "dedupe_get_workflow", get, path = "/api/dedupe/workflow", tag = "dedupe", security(("bearer" = [])),
    responses((status = 200, body = DedupeWorkflowView),
              (status = 404, description = "workflow not configured yet (typed unavailable state)")))]
pub async fn get_workflow(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<DedupeWorkflowView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let workflow = core_db::entity_dedupe::get_dedupe_workflow(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let decision_configured = dedupe_decision_ready(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DedupeWorkflowView {
        workflow,
        decision_configured,
    }))
}

/// The enablement probe (ADR 0011 §3 gate): the org master switch is ON
/// AND the `decision_record_dedupe` surface is enabled AND a decision
/// model is pinned. Cheap read-only preconditions of `decide()` itself —
/// the workflow refuses to turn on without them, typed.
async fn dedupe_decision_ready(tx: &mut sqlx::PgConnection, org_id: Uuid) -> ApiResult<bool> {
    let enabled = core_db::ai_policy::features_enabled(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    if !enabled {
        return Ok(false);
    }
    let cfg = core_db::decision::get_surface_config(tx, org_id, "decision_record_dedupe")
        .await
        .map_err(ApiError::from)?;
    Ok(cfg.is_some_and(|c| c.enabled && c.model_provider.is_some() && c.model_id.is_some()))
}

/// PUT /api/dedupe/workflow — configure the intelligent dedupe workflow.
/// The gate is the ADR 0011 §3 posture: enabling requires the org's
/// decision model to be configured — the typed refusal is
/// `dedupe_decision_not_configured`, never a silent turn-on.
#[utoipa::path(operation_id = "dedupe_put_workflow", put, path = "/api/dedupe/workflow", tag = "dedupe", security(("bearer" = [])),
    request_body = DedupeWorkflowRequest,
    responses((status = 200, body = DedupeWorkflowView),
              (status = 409, description = "dedupe_decision_not_configured — the decision model is not configured"),
              (status = 422, description = "threshold / schedule / entity kinds out of range")))]
pub async fn put_workflow(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<DedupeWorkflowRequest>,
) -> ApiResult<Json<DedupeWorkflowView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let mode = core_domain::entities::entity_dedupe::DedupeRunMode::parse(req.mode.trim())
        .ok_or_else(|| ApiError::Validation("mode must be report_only | auto_merge".into()))?;

    // THE GATE: resolve the decision provider config for the dedupe
    // surface; absent ⇒ typed refusal (enabling is impossible quietly).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let decision_configured = dedupe_decision_ready(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    if req.enabled {
        match core_domain::entities::entity_dedupe::validate_dedupe_workflow(
            req.confidence_threshold,
            &req.entity_kinds,
            req.interval_hours,
            decision_configured,
        ) {
            Ok(()) => {}
            Err(refusal) => {
                return Err(match refusal {
                    core_domain::entities::entity_dedupe::DedupeWorkflowRefusal::DecisionNotConfigured => {
                        ApiError::Conflict(format!(
                            "the decision model is not configured for this org — configure it \
                             under Settings → AI (surface `decision_record_dedupe`) before \
                             enabling the intelligent dedupe workflow ({})",
                            refusal.as_str()
                        ))
                    }
                    _ => ApiError::Validation(format!(
                        "the dedupe workflow payload was rejected ({})",
                        refusal.as_str()
                    )),
                });
            }
        }
    }

    let workflow = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let workflow = core_db::entity_dedupe::upsert_dedupe_workflow(
            &mut ctx.tx,
            org_id,
            req.enabled,
            req.confidence_threshold,
            mode.as_str(),
            req.interval_hours,
            &req.notify_user_ids,
            &req.entity_kinds,
            user_id,
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "dedupe.workflow_configured".into(),
                entity_type: Some("dedupe_workflow".into()),
                entity_id: Some(org_id.to_string()),
                detail: serde_json::json!({
                    "enabled": req.enabled,
                    "mode": mode.as_str(),
                    "threshold": req.confidence_threshold,
                    "interval_hours": req.interval_hours,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, workflow))
    })
    .await?;

    Ok(Json(DedupeWorkflowView {
        workflow,
        decision_configured,
    }))
}

/// POST /api/dedupe/workflow/run — the manual run (and the workbench's
/// "run intelligent dedupe" trigger). Report-first: every suggestion is
/// recorded suggestion-grade and visible in the workbench; in auto_merge
/// mode ONLY the at/above-threshold folds apply (audited, undoable).
#[utoipa::path(operation_id = "dedupe_run_workflow", post, path = "/api/dedupe/workflow/run", tag = "dedupe", security(("bearer" = [])),
    request_body = DedupeRunRequest,
    responses((status = 200, body = DedupeRunResponse),
              (status = 409, description = "the workflow is disabled or its decision model is unconfigured")))]
pub async fn run_workflow(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<DedupeRunRequest>,
) -> ApiResult<Json<DedupeRunResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let workflow = match core_db::entity_dedupe::get_dedupe_workflow(&mut tx, org_id).await {
        Ok(w) => w,
        Err(_) => {
            return Err(ApiError::NotFound);
        }
    };
    tx.commit().await.map_err(ApiError::from)?;

    let kinds = req
        .entity_kinds
        .unwrap_or_else(|| workflow.entity_kinds.clone());
    let auto_merge = !req.dry_run
        && workflow.enabled
        && workflow.mode == core_domain::entities::entity_dedupe::DedupeRunMode::AutoMerge;

    let report = run_dedupe_pass(
        &state,
        org_id,
        user_id,
        &kinds,
        workflow.confidence_threshold,
        auto_merge,
    )
    .await;

    if !req.dry_run {
        let report_for_record = report.clone();
        org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
            core_db::entity_dedupe::record_dedupe_run(&mut ctx.tx, org_id, &report_for_record)
                .await?;
            core_db::audit::insert(
                &mut *ctx.tx,
                &core_domain::entities::AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: core_domain::entities::Actor::Human(user_id),
                    action: "dedupe.manual_run".into(),
                    entity_type: Some("dedupe_workflow".into()),
                    entity_id: Some(org_id.to_string()),
                    detail: serde_json::json!({
                        "scanned": report.scanned,
                        "asked": report.asked,
                        "above_threshold": report.above_threshold,
                        "auto_merged": report.auto_merged,
                        "truncated": report.truncated,
                    }),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await?;
            Ok((ctx, ()))
        })
        .await?;
        deliver_report(
            &state,
            org_id,
            &report,
            &workflow.notify_user_ids,
            auto_merge,
        )
        .await;
    }

    Ok(Json(DedupeRunResponse {
        report,
        dry_run: req.dry_run,
    }))
}

/// POST /api/internal/dedupe/run — the worker sweep's seam: runs the due
/// pass for ONE org, stamps the row (the claim already advanced
/// next_run_at), delivers the report notification. Internal-token gated.
#[utoipa::path(operation_id = "internal_dedupe_run", post, path = "/api/internal/dedupe/run", tag = "internal",
    request_body = InternalDedupeRunRequest,
    responses((status = 200, body = DedupeRunResponse),
              (status = 404, description = "workflow missing or not due / no internal token configured")))]
pub async fn internal_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<InternalDedupeRunRequest>,
) -> ApiResult<Json<DedupeRunResponse>> {
    crate::routes::internal::require_internal_token(&state, &headers)?;
    let org_id = req.org_id;

    // CLAIM the due run (atomically advances next_run_at) BEFORE asking:
    // a retried sweep call cannot double-run a pass. A missing workflow
    // row (or one not due anymore) is a typed no-op.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let claimed = core_db::entity_dedupe::claim_due_dedupe_run(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    let Some(workflow) = claimed else {
        return Err(ApiError::NotFound);
    };

    let actor = workflow.updated_by.unwrap_or(org_id);
    let report = run_dedupe_pass(
        &state,
        org_id,
        actor,
        &workflow.entity_kinds,
        workflow.confidence_threshold,
        workflow.mode == core_domain::entities::entity_dedupe::DedupeRunMode::AutoMerge,
    )
    .await;

    let report_for_record = report.clone();
    org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        core_db::entity_dedupe::record_dedupe_run(&mut ctx.tx, org_id, &report_for_record).await?;
        Ok((ctx, ()))
    })
    .await?;
    deliver_report(
        &state,
        org_id,
        &report,
        &workflow.notify_user_ids,
        workflow.mode == core_domain::entities::entity_dedupe::DedupeRunMode::AutoMerge,
    )
    .await;

    Ok(Json(DedupeRunResponse {
        report,
        dry_run: false,
    }))
}

// ---- router -------------------------------------------------------------------

/// The module-owned router (org-facing + internal seam). Static segments
/// before `{id}`; the internal route mounts on the internal-only mount.
pub fn routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/dedupe/workflow"),
            axum::routing::get(get_workflow).put(put_workflow),
        )
        .route(
            &format!("{base}/dedupe/workflow/run"),
            axum::routing::post(run_workflow),
        )
}
