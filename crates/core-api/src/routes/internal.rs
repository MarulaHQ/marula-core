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
