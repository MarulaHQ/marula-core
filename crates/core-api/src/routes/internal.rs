//! Internal machine surface for the worker (§4.1 single-writer contract).
//!
//! Purpose: the ONLY write path for worker-computed effects. The SLA
//! evaluation loop posts ladder verdicts here; the automations evaluator
//! posts rule effects here. core-api applies them through its normal,
//! event-emitting, audit-logged code paths — the worker itself never
//! touches domain tables.
//!
//! Auth: `Authorization: Bearer <MERIDIAN_INTERNAL_TOKEN>`, compared in
//! constant time. When the env var is unset the whole surface is disabled
//! (404) — a deployment without the token has no machine seam at all.
//!
//! Non-purpose: no user-facing endpoints, no permission-matrix checks
//! (machine callers carry no principal), no evaluation logic of its own —
//! verdicts arrive precomputed and are re-verified only for idempotency.

use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{extract::State, http::HeaderMap, Json};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Constant-time bearer check against the configured internal token.
fn require_internal_token(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(expected) = state.config.internal_token.as_deref() else {
        // No token configured ⇒ surface disabled entirely.
        return Err(ApiError::NotFound);
    };
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?;
    let ok = expected.as_bytes().ct_eq(provided.as_bytes()).into();
    if ok {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("invalid internal token".into()))
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SlaVerdictRequest {
    pub org_id: Uuid,
    pub ticket_id: Uuid,
    /// 'first_response' | 'resolution'
    pub target: String,
    /// Ladder step that fired (ledger key).
    pub step_id: Uuid,
    /// 'risk' | 'breach' | 'escalate'
    pub trigger: String,
    /// Extra recipients from the ladder step; the assignee is added
    /// server-side.
    pub notify_user_ids: Vec<Uuid>,
    /// Human-facing headline, e.g. "[MER-12] SLA risk on resolution".
    pub title: String,
    pub body: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VerdictResponse {
    /// False when the ledger had already recorded this fire (idempotent
    /// replay after a worker retry) — nothing was written or notified.
    pub applied: bool,
    pub reason: Option<String>,
}

/// POST /api/internal/sla/verdict — fire-once application of one ladder
/// step: ledger insert (PK-guarded), breach flip, notification fan-out with
/// preference gates, events + audit, all in ONE transaction.
#[utoipa::path(operation_id = "internal_sla_verdict", post, path = "/api/internal/sla/verdict", tag = "sla",
    request_body = SlaVerdictRequest,
    responses((status = 200, body = VerdictResponse)))]
pub async fn sla_verdict(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SlaVerdictRequest>,
) -> ApiResult<Json<VerdictResponse>> {
    require_internal_token(&state, &headers)?;
    let trigger = core_domain::sla_engine::StepTrigger::parse(&req.trigger)
        .ok_or_else(|| ApiError::Validation(format!("unknown trigger {}", req.trigger)))?;
    let target = core_domain::sla_engine::SlaTarget::parse(&req.target)
        .ok_or_else(|| ApiError::Validation(format!("unknown target {}", req.target)))?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let kind = trigger.notification_kind();

    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let ticket =
            match core_db::tickets::get_ticket(&mut *ctx.tx, req.org_id, req.ticket_id).await {
                Ok(t) => t,
                Err(_) => {
                    return Ok((
                        ctx,
                        VerdictResponse {
                            applied: false,
                            reason: Some("ticket_not_found".into()),
                        },
                    ));
                }
            };

        // Idempotency floor: the PK decides. A racing duplicate loses here
        // and NOTHING below executes.
        let first_fire = core_db::sla::try_record_fired(
            &mut *ctx.tx,
            req.org_id,
            req.ticket_id,
            req.step_id,
            target.as_str(),
        )
        .await?;
        if !first_fire {
            return Ok((
                ctx,
                VerdictResponse {
                    applied: false,
                    reason: Some("already_fired".into()),
                },
            ));
        }

        if trigger == core_domain::sla_engine::StepTrigger::Breach {
            core_db::sla::mark_breached(&mut *ctx.tx, req.ticket_id).await?;
        }

        // Recipients: explicit ladder recipients ∪ ticket assignee.
        let mut recipients = req.notify_user_ids.clone();
        if let Some(assignee) = ticket.assignee_id {
            if !recipients.contains(&assignee) {
                recipients.push(assignee);
            }
        }
        for user_id in recipients {
            // Preference chain gates whether the inbox row exists; the
            // email leg is decided later by the worker notifier against
            // the same chain (quiet hours auto-bypass for breach/escalate).
            let pref = core_db::notifications::effective_preference(
                &mut *ctx.tx,
                req.org_id,
                user_id,
                kind.as_str(),
            )
            .await?;
            if !pref.in_app {
                continue;
            }
            let notification = core_db::notifications::insert(
                &mut *ctx.tx,
                Uuid::now_v7(),
                req.org_id,
                user_id,
                kind.as_str(),
                Some(req.ticket_id),
                req.title.trim(),
                req.body.as_deref(),
            )
            .await?;
            ctx.emit(
                types::NOTIFICATION_CREATED,
                "notification",
                Some(notification.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::NotificationCreated {
                    user_id,
                    kind: kind.as_str().to_string(),
                    ticket_id: Some(req.ticket_id),
                    title: notification.title.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        ctx.emit(
            types::SLA_ESCALATION_FIRED,
            "ticket",
            Some(req.ticket_id.to_string().as_str()),
            &serde_json::json!({
                "ticket_id": req.ticket_id,
                "target": target.as_str(),
                "trigger": trigger.as_str(),
                "step_id": req.step_id,
                "title": req.title.trim(),
            }),
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: req.org_id,
                actor: Actor::System,
                action: "sla.escalation".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(req.ticket_id.to_string()),
                detail: serde_json::json!({
                    "target": target.as_str(),
                    "trigger": trigger.as_str(),
                    "step_id": req.step_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        Ok((
            ctx,
            VerdictResponse {
                applied: true,
                reason: None,
            },
        ))
    })
    .await?;

    Ok(Json(applied))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct EffectsRequest {
    pub org_id: Uuid,
    pub rule_id: Option<Uuid>,
    pub ticket_id: Uuid,
    /// AutomationAction documents, parsed and validated via core-domain.
    pub actions: serde_json::Value,
}

/// POST /api/internal/effects — applies automation-rule effects to one
/// ticket through the SAME write paths users go through (ticket patch,
/// comment insert, notification fan-out), emitting every corresponding
/// domain event tagged `via_automation` so evaluators never loop.
#[utoipa::path(operation_id = "internal_effects", post, path = "/api/internal/effects", tag = "automations",
    request_body = EffectsRequest,
    responses((status = 200, body = Object)))]
pub async fn effects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EffectsRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_internal_token(&state, &headers)?;
    let actions: Vec<core_domain::automations::AutomationAction> =
        serde_json::from_value(req.actions.clone())
            .map_err(|e| ApiError::Validation(format!("malformed actions document: {e}")))?;
    core_domain::automations::validate_actions(&actions)?;

    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, req.org_id, req.ticket_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
        let _ = ticket;

        // Every payload this request emits carries the loop guard.
        let marker = serde_json::json!({ core_domain::automations::VIA_AUTOMATION_KEY: true });

        for action in &actions {
            match action {
                core_domain::automations::AutomationAction::AssignUser { user_id } => {
                    let user =
                        match core_db::users::get_user(&mut *ctx.tx, req.org_id, *user_id).await {
                            Ok(u) if u.is_active => u,
                            _ => continue, // unknown/inactive target: skip, never fail the batch
                        };
                    core_db::tickets::update_ticket(
                        &mut *ctx.tx,
                        req.org_id,
                        req.ticket_id,
                        core_db::tickets::TicketPatch {
                            title: None,
                            body: None,
                            priority: None,
                            assignee_id: Some(Some(*user_id)),
                            client_id: None,
                            site_id: None,
                            contact_id: None,
                            due_at: None,
                        },
                    )
                    .await?;
                    let mut payload = serde_json::to_value(core_domain::events::TicketAssigned {
                        ticket_id: req.ticket_id,
                        assignee_id: Some(*user_id),
                        assigned_by: None,
                    })
                    .unwrap_or_default();
                    payload["via_automation"] = marker["via_automation"].clone();
                    ctx.emit(
                        types::TICKET_ASSIGNED,
                        "ticket",
                        Some(req.ticket_id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
                    let _ = user;
                }
                core_domain::automations::AutomationAction::SetPriority { priority } => {
                    core_db::tickets::update_ticket(
                        &mut *ctx.tx,
                        req.org_id,
                        req.ticket_id,
                        core_db::tickets::TicketPatch {
                            title: None,
                            body: None,
                            priority: Some(*priority),
                            assignee_id: None,
                            client_id: None,
                            site_id: None,
                            contact_id: None,
                            due_at: None,
                        },
                    )
                    .await?;
                    let mut payload = marker.clone();
                    payload["ticket_id"] = serde_json::json!(req.ticket_id);
                    ctx.emit(
                        types::TICKET_UPDATED,
                        "ticket",
                        Some(req.ticket_id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
                }
                core_domain::automations::AutomationAction::AddInternalComment { body } => {
                    let comment = core_db::tickets::insert_comment(
                        &mut *ctx.tx,
                        Uuid::now_v7(),
                        req.org_id,
                        req.ticket_id,
                        None,
                        core_domain::status::NoteKind::Internal,
                        body.trim(),
                    )
                    .await?;
                    let mut payload =
                        serde_json::to_value(core_domain::events::TicketCommentAdded {
                            comment: comment.clone(),
                            mentioned_user_ids: vec![],
                        })
                        .unwrap_or_default();
                    payload["via_automation"] = marker["via_automation"].clone();
                    ctx.emit(
                        types::TICKET_COMMENT_ADDED,
                        "ticket_comment",
                        Some(comment.id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
                }
                core_domain::automations::AutomationAction::NotifyUsers {
                    user_ids,
                    kind,
                    title,
                } => {
                    for user_id in user_ids {
                        let pref = core_db::notifications::effective_preference(
                            &mut *ctx.tx,
                            req.org_id,
                            *user_id,
                            kind.as_str(),
                        )
                        .await?;
                        if !pref.in_app {
                            continue;
                        }
                        let notification = core_db::notifications::insert(
                            &mut *ctx.tx,
                            Uuid::now_v7(),
                            req.org_id,
                            *user_id,
                            kind.as_str(),
                            Some(req.ticket_id),
                            title.trim(),
                            None,
                        )
                        .await?;
                        let mut payload =
                            serde_json::to_value(core_domain::events::NotificationCreated {
                                user_id: *user_id,
                                kind: kind.as_str().to_string(),
                                ticket_id: Some(req.ticket_id),
                                title: notification.title.clone(),
                            })
                            .unwrap_or_default();
                        payload["via_automation"] = marker["via_automation"].clone();
                        ctx.emit(
                            types::NOTIFICATION_CREATED,
                            "notification",
                            Some(notification.id.to_string().as_str()),
                            &payload,
                        )
                        .await?;
                    }
                }
            }
        }

        ctx.emit(
            types::AUTOMATION_TRIGGERED,
            "automation_rule",
            req.rule_id.map(|r| r.to_string()).as_deref(),
            &serde_json::json!({
                "rule_id": req.rule_id,
                "ticket_id": req.ticket_id,
                "actions_applied": actions.len(),
                "via_automation": true,
            }),
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: req.org_id,
                actor: Actor::System,
                action: "automation.applied".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(req.ticket_id.to_string()),
                detail: serde_json::json!({ "rule_id": req.rule_id, "actions": actions.len() }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        Ok((ctx, serde_json::json!({ "applied": actions.len() })))
    })
    .await?;

    Ok(Json(applied))
}
