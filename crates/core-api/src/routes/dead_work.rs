// Dead work, and the two decisions an operator can make about it (Wave 52
// lane R3, `p52-ledger-redrive`).
//
// Purpose: list abandoned work from every ledger that can abandon it
// (`ops.dead_work`, migration 0144), requeue one item, cancel one item, and
// resume a webhook destination a repeated failure paused. This is the ONLY
// admin surface for those verbs, and it is API-only: no page ships here.
//
// # Why API-only, and what that costs
//
// The three verbs are consequential (a redrive POSTs a signed body at a third
// party again; a cancel says "we will never send this"), and the surface that
// will use them most is an incident: someone is looking at a receiver returning
// 503 and wants the backlog retried NOW. An operator with curl and an admin
// token can do that today. A page would need the list, the detail, the bulk
// selection, the confirmation step and the audit trail rendered, and it would
// be the second surface to keep in step with the verbs. The wave's close-out
// lane wires staff-facing UI; this lane's job is that the verbs exist, are
// audited, and cannot be called by someone who should not call them.
//
// # Every mutation is audited, and the audit row is the record
//
// `dead_work.redriven`, `dead_work.cancelled` and `webhook_destination.resumed`
// are written in the same transaction as the change they describe. A redrive or
// cancel that changed nothing (a second click, a row another operator already
// moved) writes NO audit row: the trail says what happened, not how many times
// someone asked.
//
// # Permissions
//
// The Admin matrix, the same shape the SLA, quarantine and workflow-ledger
// admin routes use: reads take `View`, the verbs take `Edit`. Owner and admin
// hold both; an auditor holds `View` and therefore sees dead work without being
// able to replay it.
//
// Non-purpose: no UI, no bulk actions, no scheduling of a redrive (it is due
// when the operator asks), no replay of a dead stream event (see the
// `replayable` column: its payload needs the relay's stream writer, which R4
// owns), and no retention.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::Utc;
use core_db::dead_work::{
    CancelOutcome, DeadWorkFilter, DeadWorkRow, DeadWorkSource, DestinationRow, RedriveOutcome,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The Admin matrix, identical to the quarantine/SLA surface's gate.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// Longest reason accepted. The column is `text`; the limit is about what an
/// operator can usefully re-read later, not storage.
const MAX_REASON_CHARS: usize = 500;

// --- wire shapes ---------------------------------------------------------------

/// One dead-work row as the operator surface reads it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeadWorkView {
    /// `job` | `email` | `webhook` | `event` — the ledger it came from.
    pub source: String,
    /// The row id in that ledger; the argument the verbs take.
    pub id: Uuid,
    /// What a human calls it: a job kind, an email subject and recipients, a
    /// webhook URL and event type, a consumer group and event type.
    pub subject: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    /// When it was abandoned, not when it was enqueued.
    pub dead_since: chrono::DateTime<Utc>,
    /// Whether `POST .../redrive` can requeue it. False for a dead stream
    /// event, whose payload needs the relay's stream writer.
    pub replayable: bool,
    /// Set when an operator has already cancelled it and the caller asked for
    /// cancelled rows.
    pub cancelled_at: Option<chrono::DateTime<Utc>>,
}

impl From<DeadWorkRow> for DeadWorkView {
    fn from(r: DeadWorkRow) -> Self {
        Self {
            source: r.source.to_string(),
            id: r.id,
            subject: r.subject,
            attempts: r.attempts,
            last_error: r.last_error,
            dead_since: r.dead_since,
            replayable: r.replayable,
            cancelled_at: r.cancelled_at,
        }
    }
}

/// One page of the list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeadWorkPage {
    /// Newest abandonment first.
    pub items: Vec<DeadWorkView>,
    /// Offset for the next page; absent when this page is the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeadWorkQuery {
    /// Restrict to one ledger: `job`, `email`, `webhook` or `event`.
    pub source: Option<String>,
    /// Include rows an operator already cancelled. Off by default: the list
    /// exists to show what still needs a decision.
    pub include_cancelled: Option<bool>,
    /// Page size, clamped to 1..=200.
    pub limit: Option<i64>,
    /// Rows to skip, from a previous page's `next_offset`.
    pub offset: Option<i64>,
}

/// The cancel verb's body. Required, for the same reason the quarantine
/// release reason is: the row will be read a month later and "why did we give
/// up on this" is the only question it will be asked.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CancelDeadWorkRequest {
    pub reason: String,
}

/// What a redrive did, as the caller sees it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RedriveResponse {
    pub source: String,
    pub id: Uuid,
    /// `requeued` — the row is back in its ledger, due now, with a fresh
    /// attempt budget. `not_dead` — someone already moved it; nothing changed.
    pub outcome: String,
    /// True when this call changed the row, which is also when an audit row was
    /// written. False means the call was a no-op repeat.
    pub changed: bool,
}

/// What a cancel did, as the caller sees it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelResponse {
    pub source: String,
    pub id: Uuid,
    /// `cancelled` — the decision is recorded. `already_cancelled` — an earlier
    /// decision stands and nothing was written.
    pub outcome: String,
    pub changed: bool,
}

/// One webhook destination and the counters that decide its pause.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DestinationView {
    pub id: Uuid,
    pub url: String,
    pub enabled: bool,
    /// Retryable delivery failures since the last delivery it accepted (or
    /// last refused on its merits).
    pub consecutive_failures: i32,
    /// Set while the destination is paused: new events are not matched for it.
    pub paused_at: Option<chrono::DateTime<Utc>>,
    pub paused_reason: Option<String>,
}

impl From<DestinationRow> for DestinationView {
    fn from(r: DestinationRow) -> Self {
        Self {
            id: r.id,
            url: r.url,
            enabled: r.enabled,
            consecutive_failures: r.consecutive_failures,
            paused_at: r.paused_at,
            paused_reason: r.paused_reason,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DestinationQuery {
    /// Only destinations that are currently paused.
    pub paused_only: Option<bool>,
}

// --- validation ----------------------------------------------------------------

fn check_reason(raw: &str) -> ApiResult<String> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(ApiError::Validation(
            "a reason is required to cancel dead work".into(),
        ));
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        return Err(ApiError::Validation(format!(
            "cancel reason is longer than {MAX_REASON_CHARS} characters"
        )));
    }
    Ok(reason.to_string())
}

/// Parses the source path segment into a typed ledger. A typo is a 422, never
/// a default: defaulting `jbos` to `job` would act on a different ledger than
/// the operator named.
fn check_source(raw: &str) -> ApiResult<DeadWorkSource> {
    DeadWorkSource::parse(raw).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown dead-work source {raw:?}; expected one of job, email, webhook, event"
        ))
    })
}

// --- handlers ------------------------------------------------------------------

/// GET /api/admin/dead-work — every ledger's abandoned work, newest first.
#[utoipa::path(operation_id = "dead_work_list", get, path = "/api/admin/dead-work",
    tag = "dead_work", security(("bearer" = [])),
    params(DeadWorkQuery),
    responses((status = 200, body = DeadWorkPage)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DeadWorkQuery>,
) -> ApiResult<Json<DeadWorkPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(100).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let source = match q.source.as_deref() {
        Some(raw) => Some(check_source(raw)?),
        None => None,
    };

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let rows = core_db::dead_work::list(
        &mut tx.tx,
        org_id,
        DeadWorkFilter {
            source,
            include_cancelled: q.include_cancelled.unwrap_or(false),
            limit: limit + 1,
            offset,
        },
    )
    .await?;
    tx.commit().await?;

    // The `+1` is the page probe: one extra row means there is a next page,
    // without a second COUNT over the union.
    let has_more = rows.len() as i64 > limit;
    let items: Vec<DeadWorkView> = rows
        .into_iter()
        .take(limit as usize)
        .map(DeadWorkView::from)
        .collect();
    let seen = offset + items.len() as i64;
    Ok(Json(DeadWorkPage {
        items,
        next_offset: has_more.then_some(seen),
    }))
}

/// POST /api/admin/dead-work/{source}/{id}/redrive — requeue one item.
///
/// The row is requeued in place: a webhook job keeps its dedupe key, so the
/// replay cannot deliver the same `(webhook, event)` twice, and the attempt
/// budget restarts so the retry is real. A repeat call is `not_dead` and writes
/// no second audit row.
#[utoipa::path(operation_id = "dead_work_redrive", post,
    path = "/api/admin/dead-work/{source}/{id}/redrive", tag = "dead_work",
    security(("bearer" = [])),
    params(("source" = String, Path), ("id" = Uuid, Path)),
    responses((status = 200, body = RedriveResponse)))]
pub async fn redrive(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((source, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<RedriveResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let source = check_source(&source)?;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let outcome = core_db::dead_work::redrive(&mut tx.tx, org_id, source, id).await?;
    let (label, changed) = match outcome {
        RedriveOutcome::Requeued => ("requeued", true),
        RedriveOutcome::NotDead => ("not_dead", false),
        RedriveOutcome::NotFound => return Err(ApiError::NotFound),
        RedriveOutcome::Cancelled => {
            // Cancel is terminal. Reviving it would make the decision a
            // suggestion and put two contradictory statements about one row in
            // the audit trail.
            return Err(ApiError::Conflict(
                "this work was cancelled by an operator, and a cancel is terminal; \
                 requeueing it would contradict the decision on record"
                    .into(),
            ));
        }
        RedriveOutcome::NotReplayable => {
            // A typed refusal, not a silent success: the caller asked for a
            // replay this surface cannot perform, and the reason names the path
            // that can.
            return Err(ApiError::Conflict(
                "a dead stream event cannot be redriven from the admin surface: its payload is \
                 the raw stream body and re-publishing needs the relay's stream writer; the \
                 consumer-gap reconciliation path owns that replay"
                    .to_string(),
            ));
        }
    };
    if changed {
        core_db::audit::insert(
            &mut *tx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "dead_work.redriven".into(),
                entity_type: Some("dead_work".into()),
                entity_id: Some(format!("{source}:{id}")),
                detail: serde_json::json!({ "source": source.as_str(), "id": id }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(RedriveResponse {
        source: source.to_string(),
        id,
        outcome: label.to_string(),
        changed,
    }))
}

/// POST /api/admin/dead-work/{source}/{id}/cancel — record that this work is
/// not wanted.
///
/// For a job (including a webhook delivery job) the row also moves to `failed`,
/// so the dead depth an alert reads actually falls. For email and dead events
/// the ledger state stays and the decision is this table's row: see
/// `core_db::dead_work::cancel` for why that asymmetry is deliberate.
#[utoipa::path(operation_id = "dead_work_cancel", post,
    path = "/api/admin/dead-work/{source}/{id}/cancel", tag = "dead_work",
    security(("bearer" = [])),
    params(("source" = String, Path), ("id" = Uuid, Path)),
    request_body = CancelDeadWorkRequest,
    responses((status = 200, body = CancelResponse)))]
pub async fn cancel(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((source, id)): Path<(String, Uuid)>,
    Json(req): Json<CancelDeadWorkRequest>,
) -> ApiResult<Json<CancelResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let source = check_source(&source)?;
    let reason = check_reason(&req.reason)?;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let outcome =
        core_db::dead_work::cancel(&mut tx.tx, org_id, source, id, &reason, Some(user_id)).await?;
    let (label, changed) = match outcome {
        CancelOutcome::Cancelled => ("cancelled", true),
        CancelOutcome::AlreadyCancelled => ("already_cancelled", false),
        CancelOutcome::NotFound => return Err(ApiError::NotFound),
    };
    if changed {
        core_db::audit::insert(
            &mut *tx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "dead_work.cancelled".into(),
                entity_type: Some("dead_work".into()),
                entity_id: Some(format!("{source}:{id}")),
                detail: serde_json::json!({
                    "source": source.as_str(),
                    "id": id,
                    "reason": reason,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(CancelResponse {
        source: source.to_string(),
        id,
        outcome: label.to_string(),
        changed,
    }))
}

/// GET /api/admin/dead-work/destinations — webhook destinations and their
/// failure counters.
///
/// Paused first, then the ones closest to pausing: the operator question is
/// "what is suppressed, and what is about to be".
#[utoipa::path(operation_id = "dead_work_destinations", get,
    path = "/api/admin/dead-work/destinations", tag = "dead_work",
    security(("bearer" = [])),
    params(DestinationQuery),
    responses((status = 200, body = Vec<DestinationView>)))]
pub async fn destinations(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DestinationQuery>,
) -> ApiResult<Json<Vec<DestinationView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let rows =
        core_db::dead_work::list_destinations(&mut tx.tx, org_id, q.paused_only.unwrap_or(false))
            .await?;
    tx.commit().await?;
    Ok(Json(rows.into_iter().map(DestinationView::from).collect()))
}

/// POST /api/admin/dead-work/destinations/{id}/resume — clear a pause.
///
/// Also resets the counter, so the destination gets a full budget of failures
/// before it pauses again rather than pausing on the next one. The operator
/// fixed something; the evidence that it was broken is history, not a debt.
#[utoipa::path(operation_id = "dead_work_resume_destination", post,
    path = "/api/admin/dead-work/destinations/{id}/resume", tag = "dead_work",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn resume_destination(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let found = core_db::webhooks::resume_destination(&mut *tx.tx, org_id, id).await?;
    if !found {
        return Err(ApiError::NotFound);
    }
    core_db::audit::insert(
        &mut *tx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "webhook_destination.resumed".into(),
            entity_type: Some("webhook".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({ "webhook_id": id }),
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ledger_the_view_emits_is_accepted_by_the_path_parser() {
        for source in DeadWorkSource::ALL {
            assert_eq!(check_source(source.as_str()).expect("known source"), source);
        }
    }

    #[test]
    fn a_mistyped_source_is_a_validation_error_not_a_default() {
        for bad in ["jobs", "emails", "webhooks", "events", ""] {
            assert!(matches!(check_source(bad), Err(ApiError::Validation(_))));
        }
    }

    #[test]
    fn a_cancel_needs_a_reason_an_operator_can_re_read() {
        assert!(check_reason("   ").is_err());
        assert!(check_reason(&"x".repeat(MAX_REASON_CHARS + 1)).is_err());
        assert_eq!(
            check_reason("  receiver retired  ").unwrap(),
            "receiver retired"
        );
    }
}
