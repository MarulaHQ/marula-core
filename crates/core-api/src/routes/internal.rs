//! Internal send-pipeline endpoints for the worker (machine-to-machine).
//!
//! Purpose: claim due outbound sends and accept per-message results — the
//! core-api-callback pattern that keeps core-api the ONLY database writer
//! (§4.1) while the worker performs transport calls. Authenticated by a
//! shared internal token header; disabled entirely when the token is
//! unconfigured. Deliberately NOT in OpenAPI/SDK.
//!
//! Non-purpose: no rendering, no retry math (core-mail), no direct worker
//! DB access.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
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
    core_db::emails::assume_table_owner(&mut *tx).await?;
    let rows = core_db::emails::claim_due_outbound(&mut *tx, limit as i64, lease_secs).await?;
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
        core_db::emails::assume_table_owner(&mut *tx).await?;
        if core_db::emails::apply_send_outcome(&mut *tx, &outcome).await? {
            applied += 1;
        }
        tx.commit().await.map_err(ApiError::from)?;
    }
    Ok((StatusCode::OK, Json(json!({ "applied": applied }))).into_response())
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
