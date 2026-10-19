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
use core_domain::settings as registry;
use core_mail::internal::{
    ClaimRequest, ClaimResponse, CsatEmailRequest, DigestEmailRequest, EnqueueEmailResponse,
    NotificationEmailRequest, ResultsRequest,
};
use core_mail::outbound::backoff_delay;
use serde_json::{json, Value};
use std::collections::HashMap;

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
    // Enrich each claimed row with the Reply-To resolved from the org/board
    // identity. A resolution failure rolls the claim back (the lease was
    // never committed) — a claim is never lost to a lookup error.
    let mut messages = Vec::with_capacity(rows.len());
    for r in rows {
        let reply_to =
            core_db::emails::resolved_reply_to(&mut tx, r.org_id, &r.from_address).await?;
        messages.push(core_mail::internal::ClaimedOutbound {
            id: r.id,
            org_id: r.org_id,
            rfc_message_id: r.rfc_message_id,
            from_address: r.from_address,
            from_name: r.from_name,
            reply_to,
            to_addresses: r.to_addresses,
            subject: r.subject,
            body_text: r.body_text,
            in_reply_to: r.in_reply_to,
            references: r.references,
            attempts: r.attempts,
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(ClaimResponse { messages }).into_response())
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

// --- Wave 34 lane 8: notification + digest email enqueue --------------------

/// POST /api/internal/email/notification — the worker decided the email leg
/// of one notification delivers (per-channel prefs + quiet hours already
/// applied). core-api renders the branded message and enqueues it on the
/// outbound ledger; the normal send loop carries it.
pub async fn notification_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<NotificationEmailRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, req.org_id, req.user_id)
        .await
        .map_err(ApiError::from)?;
    let recipient = match core_db::users::get_user(&mut *tx, req.org_id, req.user_id).await {
        Ok(user) if user.is_active => user,
        Ok(_) => {
            tx.rollback().await.ok();
            return Ok(Json(EnqueueEmailResponse::skipped("skipped_no_recipient")).into_response());
        }
        Err(core_db::DbError::NotFound) => {
            tx.rollback().await.ok();
            return Ok(Json(EnqueueEmailResponse::skipped("skipped_no_recipient")).into_response());
        }
        Err(e) => return Err(e.into()),
    };
    let brand = crate::email::resolve_brand(&mut tx, req.org_id).await?;
    // A notification is the PRODUCT writing to one of its own staff, so
    // the signer is the system: no personal rung applies, and the envelope
    // resolves to the org's own greeting and signature. The recipient's
    // first name is the fallback salutation when the org has not set one.
    let envelope = crate::email::resolve_envelope(
        &mut tx,
        req.org_id,
        req.ticket_id,
        None,
        &crate::email::EnvelopeOverrides::default(),
    )
    .await?;
    let greeting = envelope
        .greeting
        .clone()
        .unwrap_or_else(|| greeting_for(&recipient.display_name));
    let rendered = core_mail::brand::render_notification(
        &core_mail::brand::Brand {
            brand_name: &brand.brand_name,
            logo_url: brand.logo_url.as_deref(),
            greeting: Some(greeting.as_str()),
            signoff: envelope.signature.as_deref(),
        },
        &req.title,
        req.body.as_deref(),
    );
    let message_id = crate::email::enqueue_rendered_outbound(
        &mut tx,
        &state.mail,
        req.org_id,
        &recipient.email,
        &rendered.subject,
        &rendered.text_body,
        req.ticket_id,
        crate::email::RenderedSources::of(&envelope),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(EnqueueEmailResponse {
        outcome: "queued".into(),
        message_id: Some(message_id),
    })
    .into_response())
}

/// POST /api/internal/email/digest — the worker determined the recipient's
/// local digest hour arrived. core-api enforces the org policy and the
/// once-per-local-day marker under a per-recipient advisory lock, renders
/// the digest, and enqueues it. Repeats for the same day are inert.
pub async fn digest_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DigestEmailRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, req.org_id, req.user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::settings::lock_digest_slot(&mut tx, req.org_id, req.user_id).await?;

    let policy =
        core_db::settings::get_org_value(&mut *tx, req.org_id, registry::EMAIL_DIGEST_POLICY)
            .await?;
    if policy.as_ref().and_then(Value::as_str) == Some("disabled") {
        tx.rollback().await.ok();
        return Ok(Json(EnqueueEmailResponse::skipped("skipped_disabled")).into_response());
    }
    let marker = core_db::settings::get_user_value(
        &mut *tx,
        req.org_id,
        req.user_id,
        registry::EMAIL_DIGEST_LAST_SENT,
    )
    .await?;
    if marker.as_ref().and_then(Value::as_str) == Some(req.day.as_str()) {
        tx.rollback().await.ok();
        return Ok(Json(EnqueueEmailResponse::skipped("skipped_already_sent")).into_response());
    }
    if req.items.is_empty() {
        tx.rollback().await.ok();
        return Ok(Json(EnqueueEmailResponse::skipped("skipped_empty")).into_response());
    }
    let recipient = match core_db::users::get_user(&mut *tx, req.org_id, req.user_id).await {
        Ok(user) if user.is_active => user,
        Ok(_) => {
            tx.rollback().await.ok();
            return Ok(Json(EnqueueEmailResponse::skipped("skipped_no_recipient")).into_response());
        }
        Err(core_db::DbError::NotFound) => {
            tx.rollback().await.ok();
            return Ok(Json(EnqueueEmailResponse::skipped("skipped_no_recipient")).into_response());
        }
        Err(e) => return Err(e.into()),
    };
    let brand = crate::email::resolve_brand(&mut tx, req.org_id).await?;
    // Same shape as the notification fan-out: the product writes this, so
    // the org's own greeting and signature resolve, and the recipient's
    // first name is the salutation fallback.
    let envelope = crate::email::resolve_envelope(
        &mut tx,
        req.org_id,
        None,
        None,
        &crate::email::EnvelopeOverrides::default(),
    )
    .await?;
    let greeting = envelope
        .greeting
        .clone()
        .unwrap_or_else(|| greeting_for(&recipient.display_name));
    let items: Vec<core_mail::brand::DigestItem> = req
        .items
        .iter()
        .map(|item| core_mail::brand::DigestItem {
            title: item.title.clone(),
            detail: item.detail.clone(),
        })
        .collect();
    let rendered = core_mail::brand::render_digest(
        &core_mail::brand::Brand {
            brand_name: &brand.brand_name,
            logo_url: brand.logo_url.as_deref(),
            greeting: Some(greeting.as_str()),
            signoff: envelope.signature.as_deref(),
        },
        &req.day,
        &items,
    );
    let message_id = crate::email::enqueue_rendered_outbound(
        &mut tx,
        &state.mail,
        req.org_id,
        &recipient.email,
        &rendered.subject,
        &rendered.text_body,
        None,
        crate::email::RenderedSources::of(&envelope),
    )
    .await?;
    core_db::settings::upsert_user_setting(
        &mut tx,
        req.org_id,
        req.user_id,
        registry::EMAIL_DIGEST_LAST_SENT,
        &json!(req.day),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(EnqueueEmailResponse {
        outcome: "queued".into(),
        message_id: Some(message_id),
    })
    .into_response())
}

/// First word of the display name, or a neutral `Hi,`.
fn greeting_for(display_name: &str) -> String {
    match display_name.split_whitespace().next() {
        Some(first) if !first.is_empty() => format!("Hi {first},"),
        _ => "Hi,".to_string(),
    }
}

// --- Wave 34 lane 2: CSAT survey mail enqueue -------------------------------

/// POST /api/internal/email/csat — the worker ran the survey policy for a
/// resolved ticket (send-on-resolve, per-client interval, quiet hours).
/// core-api renders the branded tokenized survey mail, flips the survey row
/// to `sent` IN THE SAME TRANSACTION as the ledger insert, and emits
/// `csat.survey_sent` — retries stay inert because `mark_sent` only wins
/// once, and row state and ledger state never diverge.
pub async fn csat_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CsatEmailRequest>,
) -> ApiResult<Response> {
    ensure_internal_token(&state, &headers)?;
    let mail = state.mail.clone();
    let link_base = state.config.web_base_url.trim_end_matches('/').to_string();
    let response = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        // Only the FIRST delivery attempt transitions the row; a redelivered
        // request answers `already_sent` without touching the ledger.
        let won = core_db::csat::mark_sent(
            &mut *ctx.tx,
            req.org_id,
            req.survey_id,
            chrono::Utc::now(),
        )
        .await?;
        if won.is_none() {
            return Ok((
                ctx,
                Json(EnqueueEmailResponse::skipped("skipped_already_sent")),
            ));
        }
        let brand = crate::email::resolve_brand(&mut ctx.tx, req.org_id).await?;
        // The survey is the PRODUCT writing to a client about a resolved
        // ticket, so the signer is the system: the personal rungs are
        // skipped and the board's signature (then the org's) applies. The
        // salutation stays the fixed neutral "Hi," — the survey link is
        // single-use and the respondent is not named on the envelope — so
        // the ledger records it as the product's own line rather than
        // claiming a rung supplied it.
        let envelope = crate::email::resolve_envelope(
            &mut ctx.tx,
            req.org_id,
            Some(req.ticket_id),
            None,
            &crate::email::EnvelopeOverrides::default(),
        )
        .await?;
        let link = format!("{link_base}/portal/csat?token={}", req.token);
        let rendered = core_mail::brand::render_notification(
            &core_mail::brand::Brand {
                brand_name: &brand.brand_name,
                logo_url: brand.logo_url.as_deref(),
                greeting: Some("Hi,"),
                signoff: envelope.signature.as_deref(),
            },
            "How did we do?",
            Some(&format!(
                "Your request has been resolved. Rate your experience here: {link}\n\nThe link works once and stays valid for the period stated in the survey."
            )),
        );
        let message_id = crate::email::enqueue_rendered_outbound(
            &mut ctx.tx,
            &mail,
            req.org_id,
            &req.contact_email,
            &rendered.subject,
            &rendered.text_body,
            Some(req.ticket_id),
            crate::email::RenderedSources {
                greeting: Some(
                    core_domain::email_signature::SignatureSource::Product
                        .as_str()
                        .to_string(),
                ),
                signature: Some(envelope.signature_source.as_str().to_string()),
            },
        )
        .await?;
        let client_id = core_db::csat::get(&mut *ctx.tx, req.org_id, req.survey_id)
            .await?
            .and_then(|s| s.client_id);
        ctx.emit(
            types::CSAT_SURVEY_SENT,
            "csat_survey",
            Some(req.survey_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::CsatSurveyQueued {
                ticket_id: req.ticket_id,
                client_id,
                status: "sent".to_string(),
                skip_reason: None,
                occurred_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((
            ctx,
            Json(EnqueueEmailResponse {
                outcome: "queued".into(),
                message_id: Some(message_id),
            }),
        ))
    })
    .await?;
    Ok(response.into_response())
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
// Auth: `Authorization: Bearer <MARULA_INTERNAL_TOKEN>`, compared in
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
pub(crate) fn require_internal_token(state: &AppState, headers: &HeaderMap) -> ApiResult<()> {
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
    /// Human-facing headline, e.g. "[MAR-12] SLA risk on resolution".
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
    /// The ticket the subject resolved to. `None` on a board-subject match
    /// (Wave 43 lane 3) — there is no ticket to act on.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// The board a ticket-less subject is scoped to.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub board_id: Option<Uuid>,
    /// AutomationAction documents, parsed and validated via core-domain.
    pub actions: serde_json::Value,
}

/// POST /api/internal/effects — applies automation-rule effects to one
/// ticket through the SAME write paths users go through (ticket patch,
/// comment insert, notification fan-out), emitting every corresponding
/// domain event tagged `via_automation` so evaluators never loop.
///
/// The subject may be ticket-less (Wave 43 lane 3): a board-scoped rule
/// whose envelope carried no ticket. Then the board is resolved for scope
/// and only subject-agnostic effects (notifications) run — every
/// ticket-writing effect skips with a warn, never fails the batch, so one
/// bad rule can never wedge the queue.
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
        serde_json::from_value(req.actions.clone()).map_err(|e| {
            // Raw serde text stays server-side (Wave 13 lane 1).
            tracing::debug!(error = %e, "actions document failed to parse");
            ApiError::Validation("actions document is malformed or has the wrong shape".into())
        })?;
    core_domain::automations::validate_actions(&actions)?;

    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        // Ticket subject: the ticket row the effects write through. Board
        // subject: no ticket, and the board must resolve in this org or the
        // whole request is refused — a board that is not ours is not a scope.
        let ticket = match req.ticket_id {
            Some(ticket_id) => Some(
                core_db::tickets::get_ticket(&mut *ctx.tx, req.org_id, ticket_id)
                    .await
                    .map_err(|_| ApiError::NotFound)?,
            ),
            None => {
                if let Some(board_id) = req.board_id {
                    core_db::boards::get_board(&mut *ctx.tx, req.org_id, board_id)
                        .await
                        .map_err(|_| ApiError::NotFound)?;
                }
                None
            }
        };
        // `ticket.board_id` rides every emitted payload so the realtime
        // layer can fan these automation writes out to the board channel.

        // Every payload this request emits carries the loop guard.
        let marker = serde_json::json!({ core_domain::automations::VIA_AUTOMATION_KEY: true });

        for action in &actions {
            match action {
                core_domain::automations::AutomationAction::AssignUser { user_id } => {
                    let Some(ticket) = ticket.as_ref() else {
                        skip_ticket_effect(req.rule_id, action);
                        continue;
                    };
                    let user =
                        match core_db::users::get_user(&mut *ctx.tx, req.org_id, *user_id).await {
                            Ok(u) if u.is_active => u,
                            _ => continue, // unknown/inactive target: skip, never fail the batch
                        };
                    core_db::tickets::update_ticket(
                        &mut *ctx.tx,
                        req.org_id,
                        ticket.id,
                        core_db::tickets::TicketPatch {
                            title: None,
                            body: None,
                            priority: None,
                            assignee_id: Some(Some(*user_id)),
                            client_id: None,
                            site_id: None,
                            contact_id: None,
                            due_at: None,

                            type_id: None,
                            category_id: None,
                        },
                    )
                    .await?;
                    let mut payload = serde_json::to_value(core_domain::events::TicketAssigned {
                        ticket_id: ticket.id,
                        board_id: ticket.board_id,
                        assignee_id: Some(*user_id),
                        assigned_by: None,
                    })
                    .unwrap_or_default();
                    payload["via_automation"] = marker["via_automation"].clone();
                    ctx.emit(
                        types::TICKET_ASSIGNED,
                        "ticket",
                        Some(ticket.id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
                    let _ = user;
                }
                core_domain::automations::AutomationAction::SetPriority { priority } => {
                    let Some(ticket) = ticket.as_ref() else {
                        skip_ticket_effect(req.rule_id, action);
                        continue;
                    };
                    core_db::tickets::update_ticket(
                        &mut *ctx.tx,
                        req.org_id,
                        ticket.id,
                        core_db::tickets::TicketPatch {
                            title: None,
                            body: None,
                            priority: Some(*priority),
                            assignee_id: None,
                            client_id: None,
                            site_id: None,
                            contact_id: None,
                            due_at: None,
                            type_id: None,
                            category_id: None,
                        },
                    )
                    .await?;
                    let mut payload = marker.clone();
                    payload["ticket_id"] = serde_json::json!(ticket.id);
                    ctx.emit(
                        types::TICKET_UPDATED,
                        "ticket",
                        Some(ticket.id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
                }
                core_domain::automations::AutomationAction::AddInternalComment { body } => {
                    let Some(ticket) = ticket.as_ref() else {
                        skip_ticket_effect(req.rule_id, action);
                        continue;
                    };
                    let comment = core_db::tickets::insert_comment(
                        &mut *ctx.tx,
                        core_db::tickets::NewTicketComment {
                            id: Uuid::now_v7(),
                            org_id: req.org_id,
                            ticket_id: ticket.id,
                            author_id: None,
                            kind: core_domain::status::NoteKind::Internal,
                            body: body.trim(),
                            workflow_meta: None,
                        },
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
                    // Subject-agnostic: a notification links to the ticket
                    // when there is one, and stands alone when there is not.
                    let ticket_id = ticket.as_ref().map(|t| t.id);
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
                            ticket_id,
                            title.trim(),
                            None,
                        )
                        .await?;
                        let mut payload =
                            serde_json::to_value(core_domain::events::NotificationCreated {
                                user_id: *user_id,
                                kind: kind.as_str().to_string(),
                                ticket_id,
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
                core_domain::automations::AutomationAction::PluginAction { entry_key, params } => {
                    let Some(ticket) = ticket.as_ref() else {
                        skip_ticket_effect(req.rule_id, action);
                        continue;
                    };
                    // The registry row is the WHOLE authorization: it
                    // exists only while the install is enabled, the grant
                    // stands, and the entry is enabled. A missing entry
                    // skips, never fails the batch.
                    let entry = core_db::plugin_surface::get_entry_by_key(
                        &mut *ctx.tx,
                        req.org_id,
                        "automation_action",
                        entry_key,
                    )
                    .await
                    .ok()
                    .flatten();
                    let Some(entry) = entry else {
                        continue;
                    };
                    // The consented effect vocabulary — currently ONE
                    // effect; unknown effects skip (deny-by-default).
                    if entry.definition.get("effect").and_then(|v| v.as_str())
                        != Some("add_internal_comment")
                    {
                        continue;
                    }
                    // Typed params: validated against the entry's own
                    // declared JSON-Schema subset at execution time.
                    if let Some(schema_value) = entry
                        .definition
                        .get("params_schema")
                        .filter(|v| !v.is_null())
                    {
                        let Ok(schema) = core_domain::entities::plugins::SettingsSchema::new(
                            schema_value.clone(),
                        ) else {
                            continue;
                        };
                        if schema.validate_config(params).is_err() {
                            continue;
                        }
                    }
                    let Some(body) = params.get("body").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if body.trim().is_empty() || body.len() > 4000 {
                        continue;
                    }
                    let comment = core_db::tickets::insert_comment(
                        &mut *ctx.tx,
                        core_db::tickets::NewTicketComment {
                            id: Uuid::now_v7(),
                            org_id: req.org_id,
                            ticket_id: ticket.id,
                            author_id: None,
                            kind: core_domain::status::NoteKind::Internal,
                            body: body.trim(),
                            workflow_meta: None,
                        },
                    )
                    .await?;
                    let mut payload =
                        serde_json::to_value(core_domain::events::TicketCommentAdded {
                            comment: comment.clone(),
                            mentioned_user_ids: vec![],
                        })
                        .unwrap_or_default();
                    payload["via_automation"] = marker["via_automation"].clone();
                    payload["plugin_action"] = serde_json::json!(entry_key);
                    ctx.emit(
                        types::TICKET_COMMENT_ADDED,
                        "ticket_comment",
                        Some(comment.id.to_string().as_str()),
                        &payload,
                    )
                    .await?;
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
                // A ticket-less subject audits against the rule that fired;
                // the ticket variant keeps its per-ticket row.
                entity_type: Some(if req.ticket_id.is_some() {
                    "ticket".into()
                } else {
                    "automation_rule".into()
                }),
                entity_id: match (req.ticket_id, req.rule_id) {
                    (Some(ticket_id), _) => Some(ticket_id.to_string()),
                    (None, Some(rule_id)) => Some(rule_id.to_string()),
                    (None, None) => None,
                },
                detail: serde_json::json!({
                    "rule_id": req.rule_id,
                    "board_id": req.board_id,
                    "actions": actions.len()
                }),
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

/// One warn line for a ticket-writing effect a board-subject rule cannot
/// apply. Kept out of the match arms so each arm stays a `continue`.
fn skip_ticket_effect(rule_id: Option<Uuid>, action: &core_domain::automations::AutomationAction) {
    tracing::warn!(
        rule = ?rule_id,
        effect = action.as_str(),
        "board-subject rule: effect needs a ticket, skipped"
    );
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
            "internal effect surface disabled: MARULA_INTERNAL_TOKEN not configured".into(),
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
    /// The document's client scoping (0089): set ONLY for the `document`
    /// kind; every other kind indexes org-global. Absent in legacy
    /// callers — purely additive.
    #[serde(default)]
    pub client_scope: Option<Uuid>,
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
/// When the org has an embedding-capable provider configured, the batch's
/// vectors are computed BEFORE the write transaction opens (no HTTP while
/// holding a DB tx) and stored alongside; any provider failure leaves the
/// vectors unset and the lexical legs unchanged — indexing never fails
/// because AI is down, and no wrong-width vector is ever stored.
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
    // Vector leg: one provider resolution + one batch call for the whole
    // request; None mode short-circuits to None without any network.
    let provider = crate::ai::resolve_org_completion_provider(&state, req.org_id).await;
    let texts: Vec<String> = req
        .entries
        .iter()
        .map(|entry| crate::ai::search_embed_text(&entry.title, &entry.body))
        .collect();
    let embeddings = crate::ai::embed_batch(&provider, texts).await;
    let applied = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let mut applied = 0_usize;
        for (index, (entry, kind)) in req.entries.iter().zip(kinds).enumerate() {
            core_db::search::upsert_row(
                &mut *ctx.tx,
                ctx.org_id,
                kind,
                entry.entity_id,
                entry.parent_id,
                &entry.title,
                &entry.body,
                entry.client_scope,
            )
            .await?;
            if let Some(vectors) = &embeddings {
                if let Some(vector) = vectors.get(index) {
                    core_db::search::upsert_embedding(
                        &mut *ctx.tx,
                        ctx.org_id,
                        kind,
                        entry.entity_id,
                        &provider_model(&provider),
                        vector,
                    )
                    .await?;
                }
            }
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}

/// Model name of the resolved provider (provenance for stored vectors).
fn provider_model(provider: &crate::ai::OrgCompletionProvider) -> String {
    match provider {
        crate::ai::OrgCompletionProvider::Embeddings(embeddings) => {
            embeddings.model_name().to_string()
        }
        crate::ai::OrgCompletionProvider::None(_) => String::new(),
    }
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
            // The vector side table keeps step with the FTS row it mirrors.
            core_db::search::delete_embedding(&mut *ctx.tx, ctx.org_id, kind, entry.entity_id)
                .await?;
            applied += 1;
        }
        Ok((ctx, applied))
    })
    .await?;
    Ok(Json(IndexResult { applied }))
}

// --- vector backfill (§4.5; Wave 12 vector-search lane) ----------------------

#[derive(Debug, Deserialize)]
pub struct SearchBackfillRequest {
    pub org_id: Uuid,
    /// Provider batches to process this call (each batch is
    /// `EMBEDDING_BATCH` rows); the endpoint is idempotent and re-runnable.
    #[serde(default)]
    pub max_batches: Option<i64>,
}

/// POST /internal/search/backfill — idempotent sweep embedding existing
/// `search_index` rows (migration 0062 backfill). Only rows whose vector is
/// missing, was produced by a different model, or whose content hash moved
/// on are embedded, so re-running is free. None mode reports a typed reason
/// and embeds nothing.
pub async fn search_backfill(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SearchBackfillRequest>,
) -> ApiResult<Json<crate::ai::BackfillReport>> {
    guard(&state, &headers)?;
    let max_batches = req.max_batches.unwrap_or(40).clamp(1, 200);
    let report = crate::ai::backfill_org_embeddings(&state, req.org_id, max_batches).await?;
    Ok(Json(report))
}

// --- async import-batch runner (TASKLIST D2 remainder) -----------------------

/// Job kind the generalized 2.7 queue carries for staged import batches.
/// The worker maps this to [`run_import_batch`] below; core-api stays the
/// ONLY database writer (§4.1).
pub const IMPORT_BATCH_JOB_KIND: &str = "import.batch.run";

/// Payload the enqueue endpoint seals into the job row.
#[derive(Debug, Deserialize)]
pub struct ImportBatchRunRequest {
    pub org_id: Uuid,
    pub batch_id: Uuid,
    #[serde(default)]
    pub job_id: Option<Uuid>,
}

/// POST /api/internal/imports/batch/run — replays an async batch's staged
/// ObjectStore chunks through the SAME plan→commit transaction the
/// synchronous chunk path uses, then closes the batch. Chunk commits are
/// individually atomic and idempotent (identical re-pushes skip), so a
/// retried delivery never double-applies. Missing staged data is a typed
/// 4xx (permanent rejection — dead-letter, operator inspects), while DB
/// failures propagate as 5xx so the worker retries with backoff.
pub async fn run_import_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportBatchRunRequest>,
) -> ApiResult<Response> {
    require_internal_token(&state, &headers)?;
    // RLS-scoped read (pool carries no app.org_id GUC); internal token
    // carries an org but no user principal, so only the org GUC is set.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, req.org_id).await?;
    let batch = core_db::imports::get_batch(&mut *tx, req.org_id, req.batch_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    if batch.dry_run {
        return Err(ApiError::Validation(
            "dry-run probes are never executed".into(),
        ));
    }
    if batch.finished_at.is_some() {
        // Idempotent redelivery: the work already committed.
        return Ok((
            StatusCode::OK,
            Json(json!({ "applied": true, "skipped": "already finished" })),
        )
            .into_response());
    }
    if batch.queued_at.is_none() {
        return Err(ApiError::Conflict(
            "batch was never queued for async execution".into(),
        ));
    }
    let descriptor = format!(
        "worker:{}",
        req.job_id
            .map(|j| j.to_string())
            .unwrap_or_else(|| "unknown".into())
    );

    for index in 0..batch.staged_chunks {
        let key = core_db::imports::staged_chunk_key(req.org_id, req.batch_id, index);
        let obj = state.store.get(&key).await.map_err(|e| match e {
            core_store::StoreError::NotFound(_) => ApiError::Conflict(format!(
                "staged chunk {index} missing from the object store"
            )),
            other => crate::routes::attachments::store_error(other),
        })?;
        let bytes = axum::body::to_bytes(axum::body::Body::from_stream(obj.stream), usize::MAX)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("chunk read: {e}")))?;
        let payload: crate::routes::imports::RowsPayload =
            serde_json::from_slice(&bytes).map_err(|e| {
                // Raw serde text stays server-side (Wave 13 lane 1).
                tracing::debug!(error = %e, "staged chunk failed to parse");
                ApiError::Conflict(format!("staged chunk {index} is not a valid chunk"))
            })?;
        let (raw_rows, byte_len) = payload.decode()?;
        core_domain::import_core::enforce_limits(raw_rows.len(), byte_len)?;
        // created_by drives the org_tx user GUC; the batch opener is the
        // documented driving identity for worker-replayed runs.
        crate::routes::imports::commit_chunk(crate::routes::imports::ChunkRun {
            state: &state,
            org_id: req.org_id,
            user_id: batch.created_by,
            batch_id: req.batch_id,
            descriptor: &descriptor,
            template_id: None,
            via_worker: true,
            raw_rows,
        })
        .await?;
    }

    let closed = org_tx::with_org_tx(&state, req.org_id, batch.created_by, |mut ctx| async move {
        let closed = core_db::imports::mark_batch_finished(&mut *ctx.tx, req.org_id, req.batch_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
        ctx.emit(
            types::IMPORT_BATCH_FINISHED,
            "import_batch",
            Some(closed.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::ImportBatchFinished {
                batch_id: closed.id,
                entity_kind: closed.entity_kind.clone(),
                rows_total: closed.rows_total,
                rows_created: closed.rows_created,
                rows_updated: closed.rows_updated,
                rows_skipped: closed.rows_skipped,
                rows_failed: closed.rows_failed,
            })
            .unwrap_or_else(|_| json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: req.org_id,
                actor: Actor::Import(descriptor.clone()),
                action: "import.batch_finished".into(),
                entity_type: Some("import_batch".into()),
                entity_id: Some(closed.id.to_string()),
                detail: json!({
                    "label": closed.label,
                    "entity_kind": closed.entity_kind,
                    "async": true,
                    "rows_total": closed.rows_total,
                    "rows_created": closed.rows_created,
                    "rows_updated": closed.rows_updated,
                    "rows_skipped": closed.rows_skipped,
                    "rows_failed": closed.rows_failed,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, closed))
    })
    .await?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "applied": true,
            "batch_id": closed.id,
            "rows_total": closed.rows_total,
            "rows_created": closed.rows_created,
            "rows_updated": closed.rows_updated,
            "rows_skipped": closed.rows_skipped,
            "rows_failed": closed.rows_failed,
        })),
    )
        .into_response())
}

// --- PM schedule generation (Wave 11 lane 9, migration 0056) ---------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PmGenerateRequest {
    pub org_id: Uuid,
    pub schedule_id: Uuid,
    /// The discharged slot (the schedule's next_run_at at claim time).
    pub slot_at: chrono::DateTime<chrono::Utc>,
}

/// POST /api/internal/pm/generate — generates the ticket instance for one
/// claimed PM schedule slot. Idempotent: the `pm_runs` ledger is unique
/// per (schedule, slot), so a retried job cannot double-generate; the
/// schedule's next occurrence was already advanced by the worker's claim
/// (§4.1: the worker touches only its own scheduling table; this is the
/// ONLY domain write). The machine actor is the org itself.
#[utoipa::path(operation_id = "internal_pm_generate", post, path = "/api/internal/pm/generate", tag = "automations",
    request_body = PmGenerateRequest,
    responses((status = 200, body = Object)))]
pub async fn pm_generate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PmGenerateRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_internal_token(&state, &headers)?;

    let result = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        // Ledger gate: inert when the slot already discharged (job retry).
        let run_id = Uuid::now_v7();
        let claimed = core_db::recurring::claim_run_slot(
            &mut *ctx.tx,
            run_id,
            req.org_id,
            req.schedule_id,
            req.slot_at,
        )
        .await?;
        if !claimed {
            return Ok((
                ctx,
                json!({ "generated": false, "reason": "slot_already_discharged" }),
            ));
        }
        let schedule =
            core_db::recurring::get_schedule(&mut *ctx.tx, req.org_id, req.schedule_id).await?;
        if !schedule.enabled {
            core_db::recurring::record_run_skipped(
                &mut *ctx.tx,
                run_id,
                req.org_id,
                "schedule_disabled",
            )
            .await?;
            return Ok((
                ctx,
                json!({ "generated": false, "reason": "schedule_disabled" }),
            ));
        }

        // The instance enters the world through the NORMAL creation path
        // (same helpers as POST /api/tickets): board default status,
        // org sequence number, SLA clocks, checklist instantiation, and
        // the ordinary `ticket.created` event — PM instances are first-
        // class tickets, not a parallel artifact kind.
        let status =
            core_db::boards::default_status_for_board(&mut *ctx.tx, schedule.board_id).await?;
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, req.org_id).await?;
        let number = format!("{prefix}-{seq}");
        let ticket = core_db::tickets::insert_ticket(
            &mut ctx.tx,
            core_db::tickets::NewTicket {
                id: Uuid::now_v7(),
                org_id: req.org_id,
                number: &number,
                board_id: schedule.board_id,
                status_id: status.id,
                title: schedule.title.as_str(),
                body: schedule.body.as_deref(),
                priority: schedule.priority,
                client_id: schedule.client_id,
                site_id: schedule.site_id,
                contact_id: None,
                assignee_id: schedule.assignee_id,
                requester_id: None,
                due_at: Some(req.slot_at),
            },
        )
        .await?;
        core_db::recurring::record_run_ticket(&mut *ctx.tx, run_id, req.org_id, ticket.id).await?;
        if let Some(policy) =
            crate::routes::sla_admin::default_policy_cached(&mut ctx.tx, req.org_id).await
        {
            let sla = core_domain::entities::TicketSlaState::init(
                req.org_id,
                ticket.id,
                &policy,
                ticket.priority,
                ticket.created_at,
            );
            core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla).await?;
        }
        core_db::checklists::instantiate_missing(
            &mut ctx.tx,
            req.org_id,
            schedule.board_id,
            ticket.id,
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: req.org_id,
                actor: Actor::System,
                action: "pm_schedule.generated_ticket".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(ticket.id.to_string()),
                detail: serde_json::json!({
                    "schedule_id": req.schedule_id,
                    "schedule_name": schedule.name,
                    "slot_at": req.slot_at.to_rfc3339(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_CREATED,
            "ticket",
            Some(ticket.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCreated {
                ticket_id: ticket.id,
                number: ticket.number.0.clone(),
                board_id: ticket.board_id,
                title: ticket.title.clone(),
                priority: ticket.priority,
                client_id: ticket.client_id,
                assignee_id: ticket.assignee_id,
                created_at: ticket.created_at,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((
            ctx,
            json!({
                "generated": true,
                "ticket_id": ticket.id,
                "ticket_number": ticket.number.0,
                "slot_at": req.slot_at.to_rfc3339(),
            }),
        ))
    })
    .await?;

    Ok(Json(result))
}

// --- Decision fan-out seam (Wave 23 lane 3) ------------------------------

/// The worker decision subsystem composes state from event envelopes and
/// dispatches ONE speculative fan-out per trigger through THIS seam. The
/// handler is a thin wrapper over the internal decision edge: admission,
/// provider dispatch, shadow store + ledger all live there; this route
/// carries no policy of its own.
#[derive(Debug, Deserialize)]
pub struct DecisionFanOutRequest {
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub surface_key: String,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    /// The consumer-composed state (per the surface's `state_template` —
    /// derived temporal fields pre-computed in code, ADR 0011 §4).
    pub state: Value,
    pub request_id: String,
    #[serde(default)]
    pub inventories: HashMap<String, Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct DecisionThreshold {
    pub act: f64,
    pub review: f64,
}

#[derive(Debug, Serialize)]
pub struct DecisionFanOutResponse {
    /// "ok" — typed answers recorded; "quiet" — degrade-never-5xx.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<Value>,
    /// Per-question act/review thresholds from the effective config —
    /// the consumer gates application against these (act above, pending
    /// between, ignore below; ADR 0010 §5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<HashMap<String, DecisionThreshold>>,
}

/// POST /internal/decision/fan-out — the worker's only path into the
/// decision edge. Every quiet variant is reported as
/// `{"status":"quiet","reason":...}` with HTTP 200: a decision model that
/// is off, unconfigured, capped, or down must never read as an error to
/// the consumer (the surface acts exactly as it would without a model).
pub async fn decision_fan_out(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DecisionFanOutRequest>,
) -> ApiResult<Json<DecisionFanOutResponse>> {
    guard(&state, &headers)?;
    let ask = crate::routes::decision_edge::DecisionAsk {
        org_id: req.org_id,
        user_id: req.user_id,
        surface_key: &req.surface_key,
        entity_type: &req.entity_type,
        entity_id: req.entity_id,
        state: req.state,
        request_id: req.request_id,
        inventories: req.inventories.clone(),
    };
    match crate::routes::decision_edge::decide(&state, ask).await? {
        Ok(run) => {
            let answers = crate::routes::decision_edge::answers_to_json(
                &run.config,
                &run.response,
                &req.inventories,
            );
            let mut thresholds = HashMap::new();
            for question in &run.config.questions {
                thresholds.insert(
                    question.id.clone(),
                    DecisionThreshold {
                        act: question.thresholds.act,
                        review: question.thresholds.review,
                    },
                );
            }
            Ok(Json(DecisionFanOutResponse {
                status: "ok".into(),
                reason: None,
                mode: Some(run.mode),
                answers: Some(answers),
                thresholds: Some(thresholds),
            }))
        }
        Err(quiet) => Ok(Json(DecisionFanOutResponse {
            status: "quiet".into(),
            reason: Some(quiet.reason()),
            mode: None,
            answers: None,
            thresholds: None,
        })),
    }
}

#[derive(Debug, Deserialize)]
pub struct DecisionAlertAckRequest {
    pub org_id: Uuid,
    pub alert_id: Uuid,
    /// The worker's bot principal (ledger/audit provenance); None when
    /// the deploy runs no bot user — the ack still lands (decision-model
    /// provenance rides the decision_answers row + request id).
    pub actor_id: Option<Uuid>,
    pub request_id: String,
}

/// POST /internal/alerts/decision-ack — auto-ack above the alert-triage
/// thresholds (ADR 0010 #4: "auto-ack/ignore only above high thresholds;
/// else leaves the alert untouched"). Idempotent: an already-open-only
/// update replaying on the same alert is a benign no-op.
pub async fn alert_decision_ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DecisionAlertAckRequest>,
) -> ApiResult<Json<Value>> {
    guard(&state, &headers)?;
    let acked = org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let row = core_db::alerting::decision_auto_ack_alert(
            &mut *ctx.tx,
            req.org_id,
            req.alert_id,
            req.actor_id,
        )
        .await?;
        Ok((ctx, row.is_some()))
    })
    .await?;
    tracing::info!(
        org = %req.org_id,
        alert = %req.alert_id,
        acked,
        request_id = %req.request_id,
        "decision auto-ack applied"
    );
    Ok(Json(json!({ "acked": acked })))
}

// --- secret rotation reminders (Wave 19 lane 3) -------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SecretRotationRequest {
    pub org_id: Uuid,
    /// `secret_record` or `password`.
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub title: String,
    /// The scheduled instant this reminder fired for (RFC 3339).
    pub rotate_at: String,
}

/// POST /api/internal/secrets/rotation — applies ONE claimed rotation
/// slot: the in-app `secret_rotation` notification (to the subject's
/// creator/last writer when present) + the `secret.rotation_due` domain
/// event on the single event stream (the automation/webhook hook).
/// Idempotent per slot by construction: the sweep's claim already
/// flipped `rotation_reminded_at`, so a retried job re-sends only the
/// deliverables, never re-claims. Payloads carry ids/titles only.
pub async fn rotation_notify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SecretRotationRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    require_internal_token(&state, &headers)?;
    // Machine actor: the org itself (same posture as pm_generate).
    org_tx::with_org_tx(&state, req.org_id, req.org_id, |mut ctx| async move {
        let who: Option<Uuid> = if req.subject_kind == "password" {
            core_db::passwords::get_password(&mut *ctx.tx, req.org_id, req.subject_id)
                .await
                .ok()
                .and_then(|e| e.created_by)
        } else {
            core_db::secret_vault::get_record(&mut ctx.tx, req.org_id, req.subject_id)
                .await
                .ok()
                .and_then(|r| r.created_by)
        };
        let kind = core_domain::entities::NotificationKind::SecretRotation;
        let title = match req.subject_kind.as_str() {
            "password" => "Password entry is due for rotation",
            _ => "Secret record is due for rotation",
        };
        if let Some(to) = who {
            core_db::notifications::insert(
                &mut *ctx.tx,
                Uuid::now_v7(),
                req.org_id,
                to,
                kind.as_str(),
                None,
                &format!("{title}: {t}", t = req.title),
                Some(
                    format!(
                        "Scheduled rotation came due {at} — rotate the credential and move its rotate_at forward.",
                        at = req.rotate_at
                    )
                    .as_str(),
                ),
            )
            .await?;
        }
        core_db::outbox::insert(
            &mut ctx.tx,
            Uuid::now_v7(),
            req.org_id,
            types::SECRET_ROTATION_DUE,
            "secret_record",
            Some(req.subject_id.to_string().as_str()),
            &json!({
                "subject_kind": req.subject_kind,
                "subject_id": req.subject_id,
                "title": req.title,
                "rotate_at": req.rotate_at,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(json!({ "reminded": true })))
}

// ---------------------------------------------------------------------------
// Workflow node execution (Wave 43 lane 15, `p43-workflow-executor`)
// ---------------------------------------------------------------------------
//
// The workflow executor runs in the WORKER and walks the document there, but
// every node that touches the domain crosses THIS seam so the write happens
// through the org-scoped service path as the run's AUTHOR — never as a
// system user with more power (spec item B). `actor_user_id` rides
// `with_org_tx`, so RLS and the permission model see the same principal a
// manual run would.
//
// The node id is the REGISTRY id (`ticket.update`, `notification.send`,
// `plugin.action`, …). A node the executor cannot yet apply is refused with a
// typed validation error (a 4xx, which the executor records as a permanent
// step failure) rather than silently skipped — the coverage boundary is
// explicit, and lane 19 (the first real multi-step consumer) grows it.

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct WorkflowNodeRequest {
    pub org_id: Uuid,
    /// The author the run executes as (spec item B).
    pub actor_user_id: Uuid,
    /// The registry node id, e.g. `ticket.update`.
    pub node_id: String,
    #[serde(default)]
    pub params: Value,
    /// The run context: the triggering event and previous outputs. Load-bearing
    /// since lane 16 — a ticket-writing node with no explicit `ticketId`
    /// resolves the ticket from `context.event.ticket_id` (the envelope that
    /// fired the run).
    #[serde(default)]
    pub context: Value,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct WorkflowNodeResponse {
    pub status: String,
    pub output: Value,
}

/// POST /api/internal/workflow/node — executes ONE workflow node org-scoped
/// as the run's author.
///
/// Deliberately NOT registered in the OpenAPI paths list (no `#[utoipa::path]`
/// and no `openapi.rs` entry): internal machine-to-machine endpoints stay out
/// of the SDK pin, exactly as the module note says. The route is live; the
/// spec is not widened by it.
pub async fn workflow_node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<WorkflowNodeRequest>,
) -> ApiResult<Json<WorkflowNodeResponse>> {
    require_internal_token(&state, &headers)?;
    let org_id = req.org_id;
    let actor = req.actor_user_id;
    let node_id = req.node_id.clone();
    let params = req.params.clone();
    let context = req.context.clone();
    // A clone of the pool so the `ticket.transition` arm can call the shared
    // service (item E), which opens its own transaction.
    let pool = state.pool.clone();
    let output = org_tx::with_org_tx(&state, org_id, actor, move |mut ctx| async move {
        let output =
            execute_workflow_node(&mut ctx, &pool, org_id, &node_id, &params, &context).await?;
        Ok((ctx, output))
    })
    .await?;
    Ok(Json(WorkflowNodeResponse {
        status: "applied".to_string(),
        output,
    }))
}

/// Applies one workflow node's operation inside the org-scoped transaction.
/// A node this build cannot apply is refused, never skipped.
async fn execute_workflow_node(
    ctx: &mut org_tx::OrgTx<'_>,
    pool: &sqlx::PgPool,
    org_id: Uuid,
    node_id: &str,
    params: &Value,
    context: &Value,
) -> ApiResult<Value> {
    let marker = serde_json::json!({ core_domain::automations::VIA_AUTOMATION_KEY: true });
    match node_id {
        "ticket.transition" => {
            // This node CANNOT be a thin wrapper (item E): the legal path
            // carries WIP caps, the checklist gate, board RBAC, the
            // instant-first replay and the audit row. It rides the shared
            // service so the gate a workflow must pass is the gate the
            // ticket surface passes.
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let status_name = params
                .get("status")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApiError::Validation("ticket.transition: status is required".into())
                })?;
            // The registry param is a status NAME; the transition API takes
            // the board's status id, so resolve it against this ticket's board.
            let statuses = core_db::boards::list_statuses(&mut *ctx.tx, ticket.board_id).await?;
            let status_id = statuses
                .iter()
                .find(|s| s.name == status_name)
                .map(|s| s.id)
                .ok_or_else(|| {
                    ApiError::Validation(format!(
                        "ticket.transition: board has no status named {status_name:?}"
                    ))
                })?;
            // The run acts as its AUTHOR: materialise that principal so the
            // shared service re-derives permission and board RBAC exactly as
            // a session request would, never bypassing them.
            let principal = workflow_principal(ctx, org_id).await?;
            crate::services::tickets::transition_status_with_pool(
                pool,
                &principal,
                ticket.id,
                crate::routes::tickets::ChangeStatusRequest {
                    status_id,
                    instant: Default::default(),
                },
            )
            .await?;
            Ok(json!({ "ticketId": ticket.id, "status": status_name }))
        }
        "ticket.assign" => {
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let assignee = params
                .get("userId")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok());
            if let Some(user_id) = assignee {
                // An unknown or inactive target is a typed refusal, not a
                // silent no-op — the ledger shows why the step failed.
                match core_db::users::get_user(&mut *ctx.tx, org_id, user_id).await {
                    Ok(u) if u.is_active => {}
                    _ => {
                        return Err(ApiError::Validation(
                            "ticket.assign: the target user is unknown or inactive".into(),
                        ))
                    }
                }
            }
            core_db::tickets::update_ticket(
                &mut *ctx.tx,
                org_id,
                ticket.id,
                core_db::tickets::TicketPatch {
                    title: None,
                    body: None,
                    priority: None,
                    assignee_id: Some(assignee),
                    client_id: None,
                    site_id: None,
                    contact_id: None,
                    due_at: None,
                    type_id: None,
                    category_id: None,
                },
            )
            .await?;
            let mut payload = serde_json::to_value(core_domain::events::TicketAssigned {
                ticket_id: ticket.id,
                board_id: ticket.board_id,
                assignee_id: assignee,
                assigned_by: None,
            })
            .unwrap_or_default();
            payload["via_automation"] = marker["via_automation"].clone();
            ctx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(ticket.id.to_string().as_str()),
                &payload,
            )
            .await?;
            Ok(json!({ "ticketId": ticket.id, "assigneeId": assignee }))
        }
        "ticket.setPriority" => {
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let priority = params
                .get("priority")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::Validation("ticket.setPriority: priority is required".into()))?;
            let priority = core_db::tickets::parse_priority_or_err(priority)
                .await
                .map_err(|_| ApiError::Validation("ticket.setPriority: unknown priority".into()))?;
            core_db::tickets::update_ticket(
                &mut *ctx.tx,
                org_id,
                ticket.id,
                core_db::tickets::TicketPatch {
                    title: None,
                    body: None,
                    priority: Some(priority),
                    assignee_id: None,
                    client_id: None,
                    site_id: None,
                    contact_id: None,
                    due_at: None,
                    type_id: None,
                    category_id: None,
                },
            )
            .await?;
            let mut payload = marker.clone();
            payload["ticket_id"] = json!(ticket.id);
            ctx.emit(
                types::TICKET_UPDATED,
                "ticket",
                Some(ticket.id.to_string().as_str()),
                &payload,
            )
            .await?;
            Ok(json!({ "ticketId": ticket.id, "priority": priority.as_str() }))
        }
        "ticket.update" => {
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let fields = params.get("fields").and_then(Value::as_object);
            let title = fields.and_then(|f| f.get("title")).and_then(Value::as_str);
            let body = fields.and_then(|f| f.get("body")).and_then(Value::as_str);
            let priority = match fields
                .and_then(|f| f.get("priority"))
                .and_then(Value::as_str)
            {
                Some(p) => Some(
                    core_db::tickets::parse_priority_or_err(p)
                        .await
                        .map_err(|_| {
                            ApiError::Validation("ticket.update: unknown priority".into())
                        })?,
                ),
                None => None,
            };
            if title.is_none() && body.is_none() && priority.is_none() {
                return Err(ApiError::Validation(
                    "ticket.update: fields must carry title, body or priority".into(),
                ));
            }
            core_db::tickets::update_ticket(
                &mut *ctx.tx,
                org_id,
                ticket.id,
                core_db::tickets::TicketPatch {
                    title,
                    body: body.map(Some),
                    priority,
                    assignee_id: None,
                    client_id: None,
                    site_id: None,
                    contact_id: None,
                    due_at: None,
                    type_id: None,
                    category_id: None,
                },
            )
            .await?;
            let mut payload = marker.clone();
            payload["ticket_id"] = json!(ticket.id);
            ctx.emit(
                types::TICKET_UPDATED,
                "ticket",
                Some(ticket.id.to_string().as_str()),
                &payload,
            )
            .await?;
            Ok(json!({ "ticketId": ticket.id }))
        }
        "ticket.comment" | "ticket.internalNote" => {
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let body = params
                .get("body")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::Validation("ticket comment: body is required".into()))?;
            let kind = if node_id == "ticket.internalNote" {
                core_domain::status::NoteKind::Internal
            } else {
                core_domain::status::NoteKind::ClientVisible
            };
            let comment = core_db::tickets::insert_comment(
                &mut *ctx.tx,
                core_db::tickets::NewTicketComment {
                    id: Uuid::now_v7(),
                    org_id,
                    ticket_id: ticket.id,
                    author_id: None,
                    kind,
                    body: body.trim(),
                    workflow_meta: None,
                },
            )
            .await?;
            let mut payload = serde_json::to_value(core_domain::events::TicketCommentAdded {
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
            Ok(json!({ "ticketId": ticket.id, "commentId": comment.id }))
        }
        "notification.send" => {
            let ticket_id = params
                .get("ticketId")
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok());
            let kind = params
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("ticket_update");
            let title = params
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::Validation("notification.send: title is required".into()))?;
            let body = params.get("body").and_then(Value::as_str);
            let user_ids: Vec<Uuid> = params
                .get("userIds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter_map(|s| Uuid::parse_str(s).ok())
                        .collect()
                })
                .unwrap_or_default();
            if user_ids.is_empty() {
                return Err(ApiError::Validation(
                    "notification.send: userIds must name at least one user".into(),
                ));
            }
            let mut sent = 0_i64;
            let mut suppressed = 0_i64;
            for user_id in &user_ids {
                let pref = core_db::notifications::effective_preference(
                    &mut *ctx.tx,
                    org_id,
                    *user_id,
                    kind,
                )
                .await?;
                if !pref.in_app {
                    suppressed += 1;
                    continue;
                }
                let notification = core_db::notifications::insert(
                    &mut *ctx.tx,
                    Uuid::now_v7(),
                    org_id,
                    *user_id,
                    kind,
                    ticket_id,
                    title.trim(),
                    body,
                )
                .await?;
                let mut payload = serde_json::to_value(core_domain::events::NotificationCreated {
                    user_id: *user_id,
                    kind: kind.to_string(),
                    ticket_id,
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
                sent += 1;
            }
            Ok(json!({ "sent": sent, "suppressed": suppressed }))
        }
        "plugin.action" => {
            let ticket = workflow_ticket(ctx, org_id, params, context).await?;
            let entry_key = params
                .get("entryKey")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::Validation("plugin.action: entryKey is required".into()))?;
            // The registry row is the WHOLE authorization: it exists only
            // while the install is enabled, the grant stands and the entry
            // is enabled. A missing entry is a typed skip, never a crash.
            let entry = core_db::plugin_surface::get_entry_by_key(
                &mut *ctx.tx,
                org_id,
                "automation_action",
                entry_key,
            )
            .await
            .ok()
            .flatten();
            let Some(entry) = entry else {
                return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "entry unavailable" }));
            };
            let node_params = params.get("params").cloned().unwrap_or(json!({}));
            if entry.definition.get("effect").and_then(Value::as_str)
                != Some("add_internal_comment")
            {
                return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "effect not consented" }));
            }
            if let Some(schema_value) = entry
                .definition
                .get("params_schema")
                .filter(|v| !v.is_null())
            {
                let Ok(schema) = core_domain::entities::plugins::SettingsSchema::new(
                    schema_value.clone(),
                ) else {
                    return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "schema invalid" }));
                };
                if schema.validate_config(&node_params).is_err() {
                    return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "params refused" }));
                }
            }
            let Some(body) = node_params.get("body").and_then(Value::as_str) else {
                return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "no body" }));
            };
            if body.trim().is_empty() || body.len() > 4000 {
                return Ok(json!({ "entryKey": entry_key, "skipped": true, "reason": "body out of bounds" }));
            }
            let comment = core_db::tickets::insert_comment(
                &mut *ctx.tx,
                core_db::tickets::NewTicketComment {
                    id: Uuid::now_v7(),
                    org_id,
                    ticket_id: ticket.id,
                    author_id: None,
                    kind: core_domain::status::NoteKind::Internal,
                    body: body.trim(),
                    workflow_meta: None,
                },
            )
            .await?;
            let mut payload = serde_json::to_value(core_domain::events::TicketCommentAdded {
                comment: comment.clone(),
                mentioned_user_ids: vec![],
            })
            .unwrap_or_default();
            payload["via_automation"] = marker["via_automation"].clone();
            payload["plugin_action"] = json!(entry_key);
            ctx.emit(
                types::TICKET_COMMENT_ADDED,
                "ticket_comment",
                Some(comment.id.to_string().as_str()),
                &payload,
            )
            .await?;
            Ok(json!({ "entryKey": entry_key, "commentId": comment.id }))
        }
        other => Err(ApiError::Validation(format!(
            "unsupported workflow operation {other:?}: the executor has no org-scoped path for this node yet"
        ))),
    }
}

/// Resolves the ticket a node acts on. An explicit `params.ticketId` wins;
/// otherwise the ticket is read from the run's triggering envelope
/// (`context.event.ticket_id`, or a top-level `context.ticket_id`), so a
/// ticket-writing node in a rule-triggered workflow acts on the ticket that
/// fired the run without the author hand-copying an id. A run with neither is
/// refused (the node genuinely names no ticket).
async fn workflow_ticket(
    ctx: &mut org_tx::OrgTx<'_>,
    org_id: Uuid,
    params: &Value,
    context: &Value,
) -> ApiResult<core_domain::entities::Ticket> {
    let ticket_id = params
        .get("ticketId")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .or_else(|| trigger_ticket_id(context))
        .ok_or_else(|| {
            ApiError::Validation(
                "node names no ticketId and the triggering event carried no ticket".into(),
            )
        })?;
    core_db::tickets::get_ticket(&mut *ctx.tx, org_id, ticket_id)
        .await
        .map_err(|_| ApiError::NotFound)
}

/// Materialises the run author's principal (item E). The workflow node
/// endpoint runs inside `with_org_tx(actor)`, but board RBAC needs a full
/// `Principal` (user + roles + permissions) — the same shape a session
/// builds. Reusing it means a `ticket.transition` authored by a technician
/// refuses on the board matrix exactly as their own session would.
async fn workflow_principal(
    ctx: &mut org_tx::OrgTx<'_>,
    org_id: Uuid,
) -> ApiResult<core_domain::entities::Principal> {
    let actor = ctx.user_id;
    let user = core_db::users::get_user(&mut *ctx.tx, org_id, actor)
        .await
        .map_err(|_| ApiError::Unauthorized("the run's author is not a valid user".into()))?;
    if !user.is_active {
        return Err(ApiError::Forbidden(
            "the run's author is no longer active".into(),
        ));
    }
    let roles = core_db::users::roles_for_user(&mut *ctx.tx, actor, org_id).await?;
    let permissions = crate::auth::middleware::resolve_permissions(&roles);
    Ok(core_domain::entities::Principal {
        user,
        roles,
        permissions,
    })
}

/// The ticket id from the run context: `event.ticket_id` (the triggering
/// envelope's payload, as the executor sends it), then a top-level
/// `ticket_id`. `subject.ticket_id` covers an envelope that nests the ticket.
fn trigger_ticket_id(context: &Value) -> Option<Uuid> {
    ["event.ticket_id", "ticket_id", "event.subject.ticket_id"]
        .iter()
        .find_map(|path| {
            path.split('.')
                .try_fold(context, |cur, key| cur.get(key))
                .and_then(Value::as_str)
                .and_then(|s| Uuid::parse_str(s).ok())
        })
}

#[cfg(test)]
mod workflow_ticket_tests {
    use super::trigger_ticket_id;
    use serde_json::json;

    #[test]
    fn resolves_the_triggering_ticket_from_the_run_context() {
        let id = "00000000-0000-0000-0000-000000000001";
        // The executor sends the triggering envelope's payload as `event`.
        assert_eq!(
            trigger_ticket_id(&json!({ "event": { "ticket_id": id }, "run_id": null })),
            uuid::Uuid::parse_str(id).ok()
        );
        // A top-level ticket_id and a nested subject are also honoured.
        assert_eq!(
            trigger_ticket_id(&json!({ "ticket_id": id })),
            uuid::Uuid::parse_str(id).ok()
        );
        assert_eq!(
            trigger_ticket_id(&json!({ "event": { "subject": { "ticket_id": id } } })),
            uuid::Uuid::parse_str(id).ok()
        );
    }

    #[test]
    fn no_ticket_in_context_is_none_not_an_error() {
        assert!(trigger_ticket_id(&json!({ "event": { "board_id": "x" } })).is_none());
        assert!(trigger_ticket_id(&json!({})).is_none());
        // A malformed id is not silently accepted.
        assert!(trigger_ticket_id(&json!({ "event": { "ticket_id": "not-a-uuid" } })).is_none());
    }
}
