//! Report schedule CRUD (TASKLIST R4) — the admin-gated HTTP surface for
//! scheduled report delivery.
//!
//! Purpose: create/read/update/delete delivery schedules over stored R3
//! definitions, cursor-paged listing with the materialized next-run
//! preview, riding the instant-first write path (IF1–IF3). The WORKER
//! owns everything that happens after a schedule exists — claiming,
//! rendering, mailing, the delivery ledger — so this module deliberately
//! has no run-now or history surface (the ledger is worker-written data;
//! a reader API lands with the reporting UI work).
//!
//! RBAC: this is org-wide delivery configuration — every operation
//! requires BOTH the Reporting-module action AND owner/admin standing
//! (builtin roles only; a custom role carrying reporting:create still
//! cannot schedule deliveries).
//!
//! Write-time validation is the typed mirror of migration 0038's CHECKs
//! (cadence/day-field shape, tz, window) so clients get a typed 422
//! envelope instead of a raw constraint error, plus recipient existence
//! (staff users of THIS org) and definition existence — the same
//! registry-backed rows the worker will re-load at delivery time.
//!
//! `next_run_at` is computed at write time with the SAME pure math the
//! worker advances slots with (`core_domain::cadence`), so the listing's
//! next-run preview always agrees with what the claim sweep will do.
//!
//! Non-purpose: no report execution, no per-run events (the worker's
//! `report_deliveries` ledger is the run record), no preference writes
//! (recipients govern their own email leg via the notification center).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::get,
    Json, Router,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine as _;
use chrono::{DateTime, NaiveTime, Timelike, Utc};
use core_domain::cadence::{next_run_after, shape_of, ScheduleShape};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Wire cadence of a schedule (snake_case slugs matching the stored
/// column and the worker's parser).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleCadence {
    Daily,
    Weekly,
    Monthly,
}

impl ScheduleCadence {
    fn as_str(self) -> &'static str {
        match self {
            ScheduleCadence::Daily => "daily",
            ScheduleCadence::Weekly => "weekly",
            ScheduleCadence::Monthly => "monthly",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "daily" => Some(ScheduleCadence::Daily),
            "weekly" => Some(ScheduleCadence::Weekly),
            "monthly" => Some(ScheduleCadence::Monthly),
            _ => None,
        }
    }
}

/// One delivery schedule with its next-run preview (the materialized
/// `next_run_at` the worker claims against) and definition coordinates.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReportScheduleView {
    pub id: Uuid,
    pub name: String,
    pub report_definition_id: Uuid,
    /// Denormalized for the listing UI (`definition_name`, `definition_kind`).
    pub definition_name: String,
    pub definition_kind: String,
    /// Recipient staff user ids.
    pub recipients: Vec<Uuid>,
    pub cadence: ScheduleCadence,
    /// Local wall-clock fire time, `HH:MM`.
    #[schema(value_type = String)]
    pub time_of_day: String,
    /// 0 = Monday .. 6 = Sunday (weekly only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_of_week: Option<i16>,
    /// 1..=28 (monthly only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_of_month: Option<i16>,
    pub timezone: String,
    /// Trailing-day window each run covers.
    pub window_days: i32,
    pub enabled: bool,
    /// Next-run preview — the instant the worker will next claim.
    #[schema(value_type = String)]
    pub next_run_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_by: Uuid,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

/// POST /api/report-schedules body.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateReportScheduleRequest {
    /// Org-unique display name (1–200 chars after trimming).
    pub name: String,
    /// The stored R3 definition this schedule delivers.
    pub report_definition_id: Uuid,
    /// Recipient staff users of this org (1–50).
    pub recipients: Vec<Uuid>,
    pub cadence: ScheduleCadence,
    /// `HH:MM` or `HH:MM:SS`, read in `timezone`.
    pub time_of_day: String,
    /// Weekly only: 0 = Monday .. 6 = Sunday.
    #[serde(default)]
    pub day_of_week: Option<i16>,
    /// Monthly only: 1..=28.
    #[serde(default)]
    pub day_of_month: Option<i16>,
    /// IANA zone the wall clock reads in.
    pub timezone: String,
    /// Trailing-day window per run (1..=366); default 30.
    #[serde(default)]
    pub window_days: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// PUT /api/report-schedules/{id} body — full-replace semantics; every
/// member is applied and the next-run preview recomputes.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateReportScheduleRequest {
    pub name: String,
    pub report_definition_id: Uuid,
    pub recipients: Vec<Uuid>,
    pub cadence: ScheduleCadence,
    pub time_of_day: String,
    #[serde(default)]
    pub day_of_week: Option<i16>,
    #[serde(default)]
    pub day_of_month: Option<i16>,
    pub timezone: String,
    #[serde(default)]
    pub window_days: Option<i32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// GET /api/report-schedules query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ScheduleListQuery {
    /// Opaque keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, clamped to [1, 200].
    pub limit: Option<i64>,
}

/// GET /api/report-schedules envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportSchedulePage {
    /// Deterministically ordered `(name, id)`.
    pub schedules: Vec<ReportScheduleView>,
    /// Pass back as `cursor`; absent when exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Mutation envelope: applied schedule + read-your-write metadata (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReportScheduleMutationResponse {
    #[serde(flatten)]
    pub schedule: ReportScheduleView,
    pub mutation: instant::MutationMeta,
}

/// One delivery-history row (R4): the claimed slot, the outcome status
/// (`delivered|partial|failed|skipped`), the per-recipient outcome
/// detail, and when the ledger row landed.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportDeliveryView {
    pub id: Uuid,
    /// The claimed slot this run discharged.
    #[schema(value_type = String)]
    pub slot_at: DateTime<Utc>,
    /// `delivered` | `partial` | `failed` | `skipped`.
    pub status: String,
    /// Per-recipient outcome counters
    /// (`{"delivered": n, "disabled": n, "quiet_deferred": n, "failed": [..]}`).
    pub detail: Value,
    #[schema(value_type = String)]
    pub recorded_at: DateTime<Utc>,
}

/// GET /api/report-schedules/{id}/deliveries envelope — offset-paged
/// delivery history, newest slot first.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReportDeliveryPage {
    pub deliveries: Vec<ReportDeliveryView>,
    /// Total ledger rows for the schedule (org-scoped).
    pub total: i64,
}

/// GET /api/report-schedules/{id}/deliveries query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DeliveryListQuery {
    /// Page size, clamped to [1, 200].
    pub limit: Option<i64>,
    /// Row offset for paging.
    pub offset: Option<i64>,
}

// --- router ---------------------------------------------------------------------

/// The report-schedules slice of the `/api` surface, built per generation
/// so both mounts register identical routes through one merge line.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/report-schedules"), get(list).post(create))
        .route(
            &format!("{base}/report-schedules/{{id}}"),
            get(get_one).put(update).delete(remove),
        )
        .route(
            &format!("{base}/report-schedules/{{id}}/deliveries"),
            get(deliveries),
        )
}

// --- handlers -------------------------------------------------------------------

/// GET /api/report-schedules — cursor-paged listing (admin), ordered
/// `(name, id)`, each row carrying its next-run preview.
#[utoipa::path(
    operation_id = "report_schedules_list",
    get,
    path = "/api/report-schedules",
    tag = "reports",
    security(("bearer" = [])),
    params(ScheduleListQuery),
    responses(
        (status = 200, body = ReportSchedulePage)
    )
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ScheduleListQuery>,
) -> ApiResult<Json<ReportSchedulePage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let (after_name, after_id) = match q.cursor.as_deref().map(decode_cursor) {
        None => (None, None),
        Some(None) => return Err(ApiError::Validation("malformed cursor".into())),
        Some(Some(pair)) => pair,
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::report_schedules::list_page(
        &mut *tx,
        org_id,
        after_name.as_deref(),
        after_id,
        limit,
    )
    .await?;
    let mut schedules = Vec::with_capacity(rows.len());
    for row in &rows {
        schedules.push(view_of(&mut *tx, row).await?);
    }
    tx.commit().await.map_err(ApiError::from)?;
    let next_cursor = if rows.len() as i64 >= limit {
        rows.last().map(|r| encode_cursor(&r.name, r.id))
    } else {
        None
    };
    Ok(Json(ReportSchedulePage {
        schedules,
        next_cursor,
    }))
}

/// GET /api/report-schedules/{id} — one schedule; anything not in this
/// org is a bare 404.
#[utoipa::path(
    operation_id = "report_schedules_get",
    get,
    path = "/api/report-schedules/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ReportScheduleView),
        (status = 404, description = "not in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReportScheduleView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    // RLS-scoped read (pool carries no app.org_id GUC): one tx around
    // the schedule fetch and its definition join.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let row = core_db::report_schedules::get(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    let view = view_of(&mut *tx, &row).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// POST /api/report-schedules — create a schedule through the
/// instant-first write path; the next-run preview computes from now.
#[utoipa::path(
    operation_id = "report_schedules_create",
    post,
    path = "/api/report-schedules",
    tag = "reports",
    security(("bearer" = [])),
    request_body = CreateReportScheduleRequest,
    responses(
        (status = 200, body = ReportScheduleMutationResponse),
        (status = 403, description = "admin-only surface"),
        (status = 409, description = "name conflict or idempotency key reused"),
        (status = 422, description = "validation failed (shape, recipients, definition)")
    )
)]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateReportScheduleRequest>,
) -> ApiResult<Json<ReportScheduleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Create)?;
    let name = validate_name(&req.name)?;
    let parsed = validate_shape(
        &req.cadence,
        &req.time_of_day,
        req.day_of_week,
        req.day_of_month,
        &req.timezone,
    )?;
    let window_days = validate_window_days(req.window_days)?;
    let enabled = req.enabled.unwrap_or(true);
    let recipients = req.recipients.clone();
    let definition_id = req.report_definition_id;
    let next_run_at =
        next_run_after(&parsed, Utc::now()).map_err(|e| ApiError::Validation(e.to_string()))?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        validate_definition(&mut ctx.tx, ctx.org_id, definition_id).await?;
        validate_recipients(&mut ctx.tx, ctx.org_id, &recipients).await?;
        let id = Uuid::now_v7();
        let fields = core_db::report_schedules::ScheduleFields {
            report_definition_id: definition_id,
            name: name.clone(),
            recipients: recipients.clone(),
            cadence: req.cadence.as_str().to_string(),
            time_of_day: parsed.time_of_day,
            day_of_week: parsed.day_of_week,
            day_of_month: parsed.day_of_month,
            timezone: parsed.timezone.clone(),
            window_days,
            enabled,
            next_run_at,
        };
        let row = core_db::report_schedules::insert(&mut ctx.tx, id, ctx.org_id, user_id, &fields)
            .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = view_of(&mut *ctx.tx, &row).await?;
        ctx.emit(
            types::REPORT_SCHEDULE_CREATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "schedule_id": id,
                "definition_id": definition_id,
                "name": view.name,
                "cadence": view.cadence.as_str(),
                "next_run_at": view.next_run_at,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportScheduleMutationResponse {
                schedule: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PUT /api/report-schedules/{id} — full-replace update; the next-run
/// preview recomputes from now (cadence changes cannot strand a slot in
/// the past; the worker's claim sweep catches up enabled schedules
/// anyway, so this only affects the preview's freshness).
#[utoipa::path(
    operation_id = "report_schedules_update",
    put,
    path = "/api/report-schedules/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateReportScheduleRequest,
    responses(
        (status = 200, body = ReportScheduleMutationResponse),
        (status = 403, description = "admin-only surface"),
        (status = 404, description = "rls_sparse: target not visible"),
        (status = 409, description = "name conflict or version conflict"),
        (status = 422, description = "validation failed")
    )
)]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateReportScheduleRequest>,
) -> ApiResult<Json<ReportScheduleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Edit)?;
    let name = validate_name(&req.name)?;
    let parsed = validate_shape(
        &req.cadence,
        &req.time_of_day,
        req.day_of_week,
        req.day_of_month,
        &req.timezone,
    )?;
    let window_days = validate_window_days(req.window_days)?;
    let enabled = req.enabled.unwrap_or(true);
    let recipients = req.recipients.clone();
    let definition_id = req.report_definition_id;
    let next_run_at =
        next_run_after(&parsed, Utc::now()).map_err(|e| ApiError::Validation(e.to_string()))?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let current = core_db::report_schedules::get(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        validate_definition(&mut ctx.tx, ctx.org_id, definition_id).await?;
        validate_recipients(&mut ctx.tx, ctx.org_id, &recipients).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let fields = core_db::report_schedules::ScheduleFields {
            report_definition_id: definition_id,
            name: name.clone(),
            recipients: recipients.clone(),
            cadence: req.cadence.as_str().to_string(),
            time_of_day: parsed.time_of_day,
            day_of_week: parsed.day_of_week,
            day_of_month: parsed.day_of_month,
            timezone: parsed.timezone.clone(),
            window_days,
            enabled,
            next_run_at,
        };
        let row = core_db::report_schedules::update(&mut ctx.tx, ctx.org_id, id, &fields)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let before = view_of(&mut *ctx.tx, &current).await?;
        let view = view_of(&mut *ctx.tx, &row).await?;
        ctx.emit(
            types::REPORT_SCHEDULE_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "schedule_id": id,
                "definition_id": definition_id,
                "name": view.name,
                "cadence": view.cadence.as_str(),
                "next_run_at": view.next_run_at,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportScheduleMutationResponse {
                schedule: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/report-schedules/{id} — removal through the same gated
/// path; ledger rows cascade and the write trail keeps the snapshot.
#[utoipa::path(
    operation_id = "report_schedules_delete",
    delete,
    path = "/api/report-schedules/{id}",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ReportScheduleMutationResponse),
        (status = 403, description = "admin-only surface"),
        (status = 404, description = "rls_sparse: target not visible"),
        (status = 409, description = "version conflict")
    )
)]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ReportScheduleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Delete)?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let current = core_db::report_schedules::get(&mut *ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        core_db::report_schedules::delete(&mut ctx.tx, ctx.org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let before = view_of(&mut *ctx.tx, &current).await?;
        ctx.emit(
            types::REPORT_SCHEDULE_DELETED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "schedule_id": id,
                "name": before.name,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: Value::Null,
                updated_at: before.updated_at,
                key: None,
                payload: &before,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ReportScheduleMutationResponse {
                schedule: before,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/report-schedules/{id}/deliveries — the delivery ledger for
/// one schedule (R4 history surface). Worker-written data: rows are
/// append-only run records, never mutated, newest slot first. A missing
/// schedule is a bare 404.
#[utoipa::path(
    operation_id = "report_schedules_deliveries",
    get,
    path = "/api/report-schedules/{id}/deliveries",
    tag = "reports",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), DeliveryListQuery),
    responses(
        (status = 200, body = ReportDeliveryPage),
        (status = 404, description = "not in scope")
    )
)]
pub async fn deliveries(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<DeliveryListQuery>,
) -> ApiResult<Json<ReportDeliveryPage>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::View)?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    // RLS-scoped read (pool carries no app.org_id GUC): schedule
    // existence check and ledger rows share one org-bound tx.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::report_schedules::get(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    let (rows, total) =
        core_db::report_schedules::list_deliveries_page(&mut tx, org_id, id, limit, offset).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let deliveries = rows
        .into_iter()
        .map(|r| ReportDeliveryView {
            id: r.id,
            slot_at: r.slot_at,
            status: r.status,
            detail: r.detail,
            recorded_at: r.created_at,
        })
        .collect();
    Ok(Json(ReportDeliveryPage { deliveries, total }))
}

// --- helpers ----------------------------------------------------------------------

const ENTITY: &str = "report_schedule";
/// Recipient cap per schedule — a digest list, not a mailing list.
const MAX_RECIPIENTS: usize = 50;

/// Scheduling is org-wide delivery configuration: owner/admin standing
/// required ON TOP of the module action, so a custom role carrying
/// reporting permissions still cannot create delivery surfaces.
fn require_admin(principal: &core_domain::entities::Principal, action: Action) -> ApiResult<()> {
    require_permission(
        principal,
        Permission::Matrix {
            module: Module::Reporting,
            action,
        },
    )?;
    if !principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "report schedules are owner/admin surface".into(),
        ));
    }
    Ok(())
}

/// Reassembles the wire view, denormalizing the definition's name/kind
/// for the listing UI. A definition vanishing between the write and this
/// read cannot happen (FK CASCADE deletes schedules with their
/// definition) — a load failure here is a server-side integrity fault.
async fn view_of(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    row: &core_db::report_schedules::ReportScheduleRow,
) -> ApiResult<ReportScheduleView> {
    let cadence = ScheduleCadence::parse(&row.cadence)
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("stored cadence invalid: {row:?}")))?;
    let definition = core_db::report_defs::get(executor, row.org_id, row.report_definition_id)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("scheduled definition load: {e}")))?;
    Ok(ReportScheduleView {
        id: row.id,
        name: row.name.clone(),
        report_definition_id: row.report_definition_id,
        definition_name: definition.name,
        definition_kind: definition.kind,
        recipients: row.recipients.clone(),
        cadence,
        time_of_day: row.time_of_day.format("%H:%M").to_string(),
        day_of_week: row.day_of_week,
        day_of_month: row.day_of_month,
        timezone: row.timezone.clone(),
        window_days: row.window_days,
        enabled: row.enabled,
        next_run_at: row.next_run_at,
        last_run_at: row.last_run_at,
        last_error: row.last_error.clone(),
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Shape validation — the typed mirror of migration 0038's CHECKs plus
/// the Rust-side gates (tz parse, time parse) the DB cannot express.
#[allow(clippy::too_many_arguments)]
fn validate_shape(
    cadence: &ScheduleCadence,
    time_of_day: &str,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    timezone: &str,
) -> ApiResult<ScheduleShape> {
    let time = parse_time(time_of_day)?;
    if !(0..=23).contains(&time.hour()) {
        return Err(ApiError::Validation("time_of_day out of range".into()));
    }
    let shape = shape_of(cadence.as_str(), time, day_of_week, day_of_month, timezone)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    if let Some(dow) = day_of_week {
        if !(0..=6).contains(&dow) {
            return Err(ApiError::Validation(
                "day_of_week must be 0 (Monday) ..= 6 (Sunday)".into(),
            ));
        }
    }
    if let Some(dom) = day_of_month {
        if !(1..=28).contains(&dom) {
            return Err(ApiError::Validation(
                "day_of_month must be 1..=28 so every month has the day".into(),
            ));
        }
    }
    // The tz must parse NOW, not at first delivery.
    shape
        .timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| ApiError::Validation(format!("unknown timezone: {timezone}")))?;
    Ok(shape)
}

fn parse_time(raw: &str) -> ApiResult<NaiveTime> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(raw.trim(), "%H:%M:%S"))
        .map_err(|_| ApiError::Validation(format!("time_of_day must be HH:MM, got {raw:?}")))
}

fn validate_window_days(raw: Option<i32>) -> ApiResult<i32> {
    let days = raw.unwrap_or(30);
    if !(1..=366).contains(&days) {
        return Err(ApiError::Validation("window_days must be 1..=366".into()));
    }
    Ok(days)
}

fn validate_name(raw: &str) -> ApiResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if trimmed.len() > 200 {
        return Err(ApiError::Validation(
            "name must be at most 200 characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Recipients must be resolvable staff users of THIS org (the
/// preference chain the worker evaluates is keyed on them).
async fn validate_recipients(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    recipients: &[Uuid],
) -> ApiResult<()> {
    if recipients.is_empty() {
        return Err(ApiError::Validation(
            "at least one recipient is required".into(),
        ));
    }
    if recipients.len() > MAX_RECIPIENTS {
        return Err(ApiError::Validation(format!(
            "at most {MAX_RECIPIENTS} recipients per schedule"
        )));
    }
    for user_id in recipients {
        if core_db::users::get_user(&mut *conn, org_id, *user_id)
            .await
            .is_err()
        {
            return Err(ApiError::Validation(format!(
                "recipient {user_id} is not a user of this org"
            )));
        }
    }
    Ok(())
}

/// The referenced definition must exist in the org — the worker re-loads
/// and revalidates the spec at delivery time; a missing ref would be a
/// guaranteed run failure.
async fn validate_definition(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    definition_id: Uuid,
) -> ApiResult<()> {
    if core_db::report_defs::get(&mut *conn, org_id, definition_id)
        .await
        .is_err()
    {
        return Err(ApiError::Validation(
            "report_definition_id does not name a definition of this org".into(),
        ));
    }
    Ok(())
}

fn decode_cursor(raw: &str) -> Option<(Option<String>, Option<Uuid>)> {
    let decoded = B64.decode(raw.as_bytes()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (name, id) = text.split_once('|')?;
    Some((Some(name.to_string()), Some(id.parse().ok()?)))
}

fn encode_cursor(name: &str, id: Uuid) -> String {
    B64.encode(format!("{name}|{id}").as_bytes())
}
