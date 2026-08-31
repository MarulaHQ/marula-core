//! Internal send-pipeline endpoints for the worker (machine-to-machine).
//!
//! Purpose: claim due outbound sends and accept per-message results, and
//! run the missing-time gap audit for (org, user, day) triples the worker
//! scheduler deems due — the core-api-callback pattern that keeps core-api
//! the ONLY database writer (§4.1) while the worker performs transport
//! calls and clock scheduling. Authenticated by a shared internal token
//! header; disabled entirely when the token is unconfigured. Deliberately
//! NOT in OpenAPI/SDK.
//!
//! Non-purpose: no rendering, no retry math (core-mail), no direct worker
//! DB access, no nudge scheduling (the worker decides what is due; the
//! audit ledger here makes repeats inert).

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core_domain::events::types;
use core_mail::internal::{ClaimRequest, ClaimResponse, ResultsRequest};
use core_mail::outbound::backoff_delay;
use serde_json::json;

pub const INTERNAL_TOKEN_HEADER: &str = "X-Internal-Token";

/// Constant-time shared-token gate. Unset token disables both endpoints.
fn ensure_internal_token(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(expected) = state.config.email_internal_token.as_deref() else {
        return Err(ApiError::NotFound);
    };
    let presented = headers
        .get(INTERNAL_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let ok = expected.len() == presented.len()
        && subtle_ct_eq(expected.as_bytes(), presented.as_bytes());
    if ok {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("invalid internal token".into()))
    }
}

/// Length-checked constant-time byte equality (subtle without the crate).
fn subtle_ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// POST /api/internal/email/outbound/claim — atomically hands out due
/// messages (queued past backoff, or abandoned leases). Runs as table
/// owner because one batch spans orgs.
pub async fn claim_outbound(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ClaimRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let limit = req.limit.clamp(1, 100);
    let lease_secs = req.lease_secs.clamp(30, 3_600) as f64;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let rows =
        core_db::emails::claim_due_outbound_for_app(&mut tx, limit, lease_secs as i64).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(ClaimResponse {
        messages: rows
            .into_iter()
            .map(|r| core_mail::internal::ClaimedOutbound {
                id: r.id,
                org_id: r.org_id,
                rfc_message_id: r.rfc_message_id,
                from_address: r.from_address,
                from_name: r.from_name,
                to_addresses: r.to_addresses,
                subject: r.subject,
                body_text: r.body_text,
                in_reply_to: r.in_reply_to,
                references: r.references,
                attempts: r.attempts,
            })
            .collect(),
    })
    .into_response())
}

/// POST /api/internal/email/outbound/results — applies worker reports.
/// Guards on `sending` state make stale/duplicate reports inert.
pub async fn report_results(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ResultsRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let mut applied = 0_usize;
    for result in &req.results {
        let outcome = core_db::emails::SendOutcome {
            id: result.id,
            ok: result.ok,
            provider_message_id: result.provider_message_id.clone(),
            error: result.error.clone(),
            retry_delay_secs: backoff_delay(i64::from(result.attempts))
                .num_seconds()
                .max(1),
        };
        // Each report in its own short transaction: one poisoned row must
        // not roll back the whole batch.
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        if core_db::emails::apply_send_outcome_for_app(&mut tx, &outcome).await? {
            applied += 1;
        }
        tx.commit().await.map_err(ApiError::from)?;
    }
    Ok((StatusCode::OK, Json(json!({ "applied": applied }))).into_response())
}

// --- missing-time gap audit run (TASKLIST 2.10) -----------------------------

/// POST /api/internal/time/gap-audit/run — computes and persists the
/// activity-vs-entries comparison for each item and nudges gap days once
/// (ledger claim + notification row + `notification.created` event, all in
/// the item's transaction). Idempotent under worker retries by design.
pub async fn run_gap_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<core_domain::time_audit::GapAuditRunRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let mut results = Vec::with_capacity(req.items.len().min(500));
    for item in req.items.iter().take(500) {
        results.push(run_one_gap_audit(&state, *item).await);
    }
    Ok(Json(
        serde_json::to_value(core_domain::time_audit::GapAuditRunResponse { results })
            .unwrap_or_default(),
    )
    .into_response())
}

async fn run_one_gap_audit(
    state: &AppState,
    item: core_domain::time_audit::GapAuditRunItem,
) -> core_domain::time_audit::GapAuditRunResult {
    let mut result = core_domain::time_audit::GapAuditRunResult {
        org_id: item.org_id,
        user_id: item.user_id,
        day: item.day,
        activity: false,
        logged_minutes: 0,
        expected_minutes: 0,
        has_gap: false,
        nudged: false,
        error: None,
    };
    let outcome =
        crate::org_tx::with_org_tx(state, item.org_id, item.user_id, |mut ctx| async move {
            let settings =
                core_db::time_audit::org_time_settings(&mut *ctx.tx, item.org_id).await?;
            let user = core_db::users::get_user(&mut *ctx.tx, item.org_id, item.user_id).await?;
            let tz: chrono_tz::Tz = user.timezone.parse().map_err(|_| {
                ApiError::Validation(format!("unknown timezone: {}", user.timezone))
            })?;
            let (start, end) = core_domain::time_audit::local_day_window(item.day, tz);
            let activity = core_db::time_audit::day_activity(
                &mut *ctx.tx,
                item.org_id,
                item.user_id,
                start,
                end,
            )
            .await?;
            let logged = core_db::timers::minutes_logged_on(
                &mut *ctx.tx,
                item.user_id,
                item.org_id,
                start,
                end,
            )
            .await?;
            let expected = i64::from(settings.time_gap_min_minutes);
            let gap = core_domain::time_audit::evaluate_day(item.day, activity, logged, expected);

            let audit = core_db::time_audit::upsert_gap_audit(
                &mut *ctx.tx,
                &core_db::time_audit::GapAuditFacts {
                    org_id: item.org_id,
                    user_id: item.user_id,
                    day: item.day,
                    activity: gap.activity,
                    logged_minutes: i32::try_from(gap.logged_minutes).unwrap_or(i32::MAX),
                    expected_minutes: i32::try_from(gap.expected_minutes).unwrap_or(i32::MAX),
                    has_gap: gap.has_gap(),
                },
            )
            .await?;

            let mut nudged = false;
            if audit.has_gap
                && audit.nudge_sent_at.is_none()
                && core_db::time_audit::claim_nudge(&mut *ctx.tx, audit.id).await?
            {
                let title = format!("Missing time on {}", item.day);
                let body = format!(
                    "{}m logged of {}m expected — log your time to close the gap",
                    gap.logged_minutes, gap.expected_minutes
                );
                let notification = core_db::notifications::insert(
                    &mut *ctx.tx,
                    uuid::Uuid::now_v7(),
                    item.org_id,
                    item.user_id,
                    "missing_time",
                    None,
                    &title,
                    Some(&body),
                )
                .await?;
                ctx.emit(
                    types::NOTIFICATION_CREATED,
                    "notification",
                    Some(notification.id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::NotificationCreated {
                        user_id: item.user_id,
                        kind: notification.kind.as_str().to_string(),
                        ticket_id: None,
                        title: notification.title.clone(),
                    })
                    .unwrap_or_default(),
                )
                .await?;
                nudged = true;
            }
            Ok((
                ctx,
                (
                    gap.activity,
                    i32::try_from(gap.logged_minutes).unwrap_or(i32::MAX),
                    i32::try_from(gap.expected_minutes).unwrap_or(i32::MAX),
                    gap.has_gap(),
                    nudged,
                ),
            ))
        })
        .await;
    match outcome {
        Ok((activity, logged, expected, has_gap, nudged)) => {
            result.activity = activity;
            result.logged_minutes = logged;
            result.expected_minutes = expected;
            result.has_gap = has_gap;
            result.nudged = nudged;
        }
        Err(err) => result.error = Some(err.to_string()),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare_matches_only_equal_buffers() {
        assert!(subtle_ct_eq(b"abc", b"abc"));
        assert!(!subtle_ct_eq(b"abc", b"abd"));
        assert!(!subtle_ct_eq(b"abc", b"abcd"));
        assert!(!subtle_ct_eq(b"", b"a"));
    }

    #[test]
    fn backoff_from_reported_attempts_is_positive() {
        assert!(backoff_delay(i64::from(1)).num_seconds() >= 1);
        assert!(backoff_delay(i64::from(9)).num_seconds() >= 3_600);
    }
}

// --- SLA/automation effect surface (stage2-sla lane) ---
// Internal machine surface for the worker (§4.1 single-writer contract).
//
// Purpose: the ONLY write path for worker-computed effects. The SLA
// evaluation loop posts ladder verdicts here; the automations evaluator
// posts rule effects here. core-api applies them through its normal,
// event-emitting, audit-logged code paths — the worker itself never
// touches domain tables.
//
// Auth: `Authorization: Bearer <MERIDIAN_INTERNAL_TOKEN>`, compared in
// constant time. When the env var is unset the whole surface is disabled
// (404) — a deployment without the token has no machine seam at all.
//
// Non-purpose: no user-facing endpoints, no permission-matrix checks
// (machine callers carry no principal), no evaluation logic of its own —
// verdicts arrive precomputed and are re-verified only for idempotency.

use crate::org_tx;
use core_db::search::SearchKind;

use core_domain::entities::{Actor, AuditEvent};
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
        // `ticket.board_id` rides every emitted payload so the realtime
        // layer can fan these automation writes out to the board channel.

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
                        board_id: ticket.board_id,
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

// --- webhook delivery ledger (stage2-webhooks lane) ---
//
// Purpose: the write seam for the worker's signed-webhook delivery loop.
// Each delivery attempt is POSTed here and inserted into the per-attempt
// ledger with a unique guard, so at-least-once workers can report the same
// attempt twice without ever double-logging. Auth + disable semantics are
// identical to the SLA/automation surface above (§4.1 single-writer).
// Deliberately NOT in OpenAPI/SDK.
//
// Non-purpose: no signing, no retry math (worker), no event emission —
// deliveries are bookkeeping, never domain mutations.

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct WebhookDeliveryRecord {
    pub webhook_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub attempt: i32,
    pub response_status: Option<i32>,
    pub next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
    pub delivered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct WebhookDeliveriesRequest {
    pub org_id: Uuid,
    pub records: Vec<WebhookDeliveryRecord>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WebhookDeliveriesResponse {
    pub applied: usize,
}

/// POST /api/internal/webhook/deliveries — idempotent attempt-log writes.
/// Cross-org batch runs as table owner; each record commits separately so
/// one bad row cannot roll back the batch.
#[utoipa::path(operation_id = "internal_webhook_deliveries", post, path = "/api/internal/webhook/deliveries", tag = "internal",
    request_body = WebhookDeliveriesRequest,
    responses((status = 200, body = WebhookDeliveriesResponse)))]
pub async fn webhook_deliveries(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WebhookDeliveriesRequest>,
) -> ApiResult<Json<WebhookDeliveriesResponse>> {
    require_internal_token(&state, &headers)?;
    let mut applied = 0_usize;
    for record in &req.records {
        if record.attempt < 1 {
            return Err(ApiError::Validation("attempt must be >= 1".into()));
        }
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        // No set_org on purpose: the worker pool is the owner role and one
        // batch spans orgs (same posture as claim_outbound above).
        core_db::set_org(&mut tx, req.org_id).await?;
        let logged = core_db::webhooks::insert_delivery_attempt(
            &mut tx,
            req.org_id,
            &core_db::webhooks::NewDeliveryAttempt {
                webhook_id: record.webhook_id,
                event_id: record.event_id,
                event_type: record.event_type.clone(),
                attempt: record.attempt,
                response_status: record.response_status,
                next_retry_at: record.next_retry_at,
                delivered_at: record.delivered_at,
                last_error: record.last_error.clone(),
            },
        )
        .await?;
        tx.commit().await.map_err(ApiError::from)?;
        if logged {
            applied += 1;
        }
    }
    Ok(Json(WebhookDeliveriesResponse { applied }))
}

// --- Search index effect surface (stage2-search lane) ---
/// Constant-time equality over raw bytes; length differences still leak
/// only through the early return (lengths are not secret).
fn token_matches(expected: &str, provided: &str) -> bool {
    let a = expected.as_bytes();
    let b = provided.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// Extracts and verifies the internal token; disabled when unset.
fn guard(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
    let Some(expected) = state.config.internal_token.as_deref() else {
        return Err(ApiError::Unavailable(
            "internal effect surface disabled: MERIDIAN_INTERNAL_TOKEN not configured".into(),
        ));
    };
    let provided = headers
        .get("x-internal-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !token_matches(expected, provided) {
        return Err(ApiError::Unauthorized("invalid internal token".into()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UpsertEntry {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteEntry {
    pub entity_kind: String,
    pub entity_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct IndexRequest<E> {
    /// Tenant the rows belong to; bound into the RLS transaction below.
    pub org_id: Uuid,
    pub entries: Vec<E>,
}

#[derive(Debug, serde::Serialize)]
pub struct IndexResult {
    pub applied: usize,
}

/// POST /internal/search/upsert — idempotent row upserts (replays safe).
pub async fn upsert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IndexRequest<UpsertEntry>>,
) -> ApiResult<Json<IndexResult>> {
    guard(&state, &headers)?;
    if req.entries.len() > 500 {
        return Err(ApiError::Validation("too many entries".into()));
    }
    let mut kinds = Vec::with_capacity(req.entries.len());
    for entry in &req.entries {
        kinds.push(
            SearchKind::parse(&entry.entity_kind)
                .ok_or_else(|| ApiError::Validation("unknown entity_kind".into()))?,
        );
    }
    // The caller is a process, not a user; the org binding is what matters
    // for RLS, so org_id doubles as the principal id here.
    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let mut applied = 0_usize;
        for (entry, kind) in req.entries.iter().zip(kinds) {
            core_db::search::upsert_row(
                &mut *ctx.tx,
                ctx.org_id,
                kind,
                entry.entity_id,
                entry.parent_id,
                &entry.title,
                &entry.body,
            )
            .await?;
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}

/// POST /internal/search/delete — removals when source entities vanish.
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IndexRequest<DeleteEntry>>,
) -> ApiResult<Json<IndexResult>> {
    guard(&state, &headers)?;
    if req.entries.len() > 500 {
        return Err(ApiError::Validation("too many entries".into()));
    }
    let mut kinds = Vec::with_capacity(req.entries.len());
    for entry in &req.entries {
        kinds.push(
            SearchKind::parse(&entry.entity_kind)
                .ok_or_else(|| ApiError::Validation("unknown entity_kind".into()))?,
        );
    }
    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let mut applied = 0_usize;
        for (entry, kind) in req.entries.iter().zip(kinds) {
            core_db::search::delete_row(&mut *ctx.tx, ctx.org_id, kind, entry.entity_id).await?;
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}
