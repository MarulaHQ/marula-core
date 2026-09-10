//! PM schedules — recurring/scheduled tickets (Wave 11 lane 9,
//! migration 0056).
//!
//! Purpose: the admin CRUD surface for recurring-ticket templates
//! (board + typed cadence, same shape as report schedules) and the
//! run-ledger reader. The WORKER owns the clock: its tick claims due
//! schedules and enqueues generation jobs; core-api's internal surface
//! generates the ticket instance through the normal write path (§4.1,
//! this module never instantiates tickets).
//!
//! Non-purpose: no cron strings (cadence is the typed daily/weekly/
//! monthly enum), no per-ticket billing, no events from the worker (the
//! `pm_runs` ledger is the run record; each instance emits the ordinary
//! `ticket.created`).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::get,
    Json, Router,
};
use core_db::recurring::DuePmSchedule;
use core_domain::cadence::{next_run_after, shape_of, ScheduleShape};
use core_domain::entities::{Actor, AuditEvent, PmSchedule};
use core_domain::events::types;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/pm-schedules"), get(list).post(create))
        .route(
            &format!("{base}/pm-schedules/{{id}}"),
            get(get_one).patch(update).delete(remove),
        )
        .route(&format!("{base}/pm-schedules/{{id}}/runs"), get(list_runs))
}

fn manage_boards() -> Permission {
    Permission::ManageBoards
}

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Tickets,
        action,
    }
}

const NAME_MAX: usize = 120;

fn validate_name(name: &str) -> ApiResult<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    if trimmed.len() > NAME_MAX {
        return Err(ApiError::Validation(format!(
            "name must be at most {NAME_MAX} characters"
        )));
    }
    Ok(trimmed)
}

/// The typed cadence + wall-clock shape (same columns as report
/// schedules; the DB CHECKs pin the same consistency rules).
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct CadenceSpec {
    /// "daily" | "weekly" | "monthly"
    pub cadence: String,
    /// Local wall-clock fire time in `timezone` (HH:MM:SS).
    pub time_of_day: chrono::NaiveTime,
    /// 0 = Monday .. 6 = Sunday; weekly only.
    pub day_of_week: Option<i16>,
    /// 1–28; monthly only.
    pub day_of_month: Option<i16>,
    /// IANA zone the wall clock reads in.
    #[serde(default = "default_tz")]
    pub timezone: String,
}

fn default_tz() -> String {
    "UTC".to_string()
}

impl CadenceSpec {
    /// Validates the shape + materializes the first occurrence strictly
    /// after `after` — cadence math failure is a typed 422, never a
    /// wedged schedule.
    fn validate(&self) -> ApiResult<(ScheduleShape, chrono::DateTime<chrono::Utc>)> {
        let shape = shape_of(
            &self.cadence,
            self.time_of_day,
            self.day_of_week,
            self.day_of_month,
            &self.timezone,
        )
        .map_err(|e| ApiError::Validation(format!("invalid cadence: {e}")))?;
        let next = next_run_after(&shape, chrono::Utc::now())
            .map_err(|e| ApiError::Validation(format!("cadence has no occurrence: {e}")))?;
        Ok((shape, next))
    }
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct ScheduleMutationResponse {
    pub schedule: PmSchedule,
    pub mutation: instant::MutationMeta,
}

/// GET /api/pm-schedules — every schedule, next-run ordered.
#[utoipa::path(operation_id = "pm_schedules_list", get, path = "/api/pm-schedules", tag = "pm-schedules", security(("bearer" = [])),
    responses((status = 200, body = [PmSchedule])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PmSchedule>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let schedules = core_db::recurring::list_schedules(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(schedules))
}

/// GET /api/pm-schedules/{id}
#[utoipa::path(operation_id = "pm_schedules_get", get, path = "/api/pm-schedules/{id}", tag = "pm-schedules", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PmSchedule),
              (status = 404, description = "schedule not found")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PmSchedule>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let schedule = core_db::recurring::get_schedule(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(schedule))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateScheduleRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub name: String,
    #[schema(value_type = uuid::Uuid)]
    pub board_id: Uuid,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// "p1" | "p2" | "p3" | "p4" (defaults p3)
    #[serde(default = "default_priority")]
    pub priority: String,
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub client_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub site_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub assignee_id: Option<Uuid>,
    #[serde(flatten)]
    pub cadence: CadenceSpec,
}

fn default_priority() -> String {
    "p3".to_string()
}

/// POST /api/pm-schedules — create a recurring-ticket template.
#[utoipa::path(operation_id = "pm_schedules_create", post, path = "/api/pm-schedules", tag = "pm-schedules", security(("bearer" = [])),
    request_body = CreateScheduleRequest,
    responses((status = 200, body = ScheduleMutationResponse),
              (status = 404, description = "board not found"),
              (status = 409, description = "duplicate schedule name or idempotency key reused"),
              (status = 422, description = "invalid cadence shape")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateScheduleRequest>,
) -> ApiResult<Json<ScheduleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = validate_name(&req.name)?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let priority: core_domain::status::Priority = req
        .priority
        .parse()
        .map_err(|e: core_domain::DomainError| ApiError::Validation(e.to_string()))?;
    let next_run = req.cadence.validate()?.1;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "pm_schedule",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let _ = core_db::boards::get_board(&mut *ctx.tx, org_id, req.board_id).await?;
        if let Some(assignee_id) = req.assignee_id {
            core_db::users::get_user(&mut *ctx.tx, org_id, assignee_id).await?;
        }
        let id = Uuid::now_v7();
        core_db::recurring::insert_schedule(
            &mut *ctx.tx,
            &core_db::recurring::NewPmSchedule {
                id,
                org_id,
                name: &name,
                board_id: req.board_id,
                title: req.title.trim(),
                body: req.body.as_deref(),
                priority: priority.as_str(),
                client_id: req.client_id,
                site_id: req.site_id,
                assignee_id: req.assignee_id,
                cadence: &req.cadence.cadence,
                time_of_day: req.cadence.time_of_day,
                day_of_week: req.cadence.day_of_week,
                day_of_month: req.cadence.day_of_month,
                timezone: &req.cadence.timezone,
                next_run_at: next_run,
                created_by: user_id,
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some() => {
                ApiError::Conflict("a schedule with this name already exists".into())
            }
            other => other.into(),
        })?;
        let schedule = core_db::recurring::get_schedule(&mut *ctx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pm_schedule.created".into(),
                entity_type: Some("pm_schedule".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": schedule.name, "cadence": schedule.cadence }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::PM_SCHEDULE_CREATED,
            "pm_schedule",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PmScheduleChanged {
                schedule: schedule.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "pm_schedule",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let stored = ScheduleMutationResponse {
            schedule: schedule.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "pm_schedule",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&schedule).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ScheduleMutationResponse { schedule, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct UpdateScheduleRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub name: Option<String>,
    pub title: Option<String>,
    /// Some → set (null clears); absent → no change.
    #[serde(default)]
    pub body: Option<Option<String>>,
    pub priority: Option<String>,
    #[serde(default)]
    pub client_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub site_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub assignee_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub cadence: Option<CadenceSpec>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// PATCH /api/pm-schedules/{id} — edit template/cadence/enabled. A
/// cadence edit re-materializes `next_run_at` from the new shape.
#[utoipa::path(operation_id = "pm_schedules_update", patch, path = "/api/pm-schedules/{id}", tag = "pm-schedules", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateScheduleRequest,
    responses((status = 200, body = ScheduleMutationResponse),
              (status = 404, description = "schedule not found"),
              (status = 409, description = "version conflict or duplicate name"),
              (status = 422, description = "invalid cadence shape")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> ApiResult<Json<ScheduleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = req.name.as_deref().map(validate_name).transpose()?;
    let cadence = req.cadence.clone();
    let next_run = match &cadence {
        Some(c) => Some(c.validate()?.1),
        None => None,
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "pm_schedule",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::recurring::get_schedule(&mut *ctx.tx, org_id, id).await?;
        let (c_time_of_day, c_day_of_week, c_day_of_month, c_timezone, c_slug, recompute_next) =
            match &cadence {
                Some(c) => (
                    Some(c.time_of_day),
                    Some(c.day_of_week),
                    Some(c.day_of_month),
                    Some(c.timezone.as_str()),
                    Some(c.cadence.as_str()),
                    true,
                ),
                None => (None, None, None, None, None, false),
            };
        // A cadence edit re-materializes the next occurrence from the
        // NEW shape anchored at now (a plain re-enable keeps the row's
        // existing schedule).
        let effective_next = if recompute_next { next_run } else { None };
        let _ = recompute_next;
        core_db::recurring::update_schedule(
            &mut *ctx.tx,
            org_id,
            id,
            &core_db::recurring::PmSchedulePatch {
                name: name.as_deref(),
                title: req.title.as_deref().map(str::trim),
                body: req.body.as_ref().map(|b| b.as_deref()),
                priority: None,
                client_id: req.client_id,
                site_id: req.site_id,
                assignee_id: req.assignee_id,
                cadence: c_slug,
                time_of_day: c_time_of_day,
                day_of_week: c_day_of_week,
                day_of_month: c_day_of_month,
                timezone: c_timezone,
                enabled: req.enabled,
                next_run_at: effective_next,
            },
        )
        .await?;
        let after = core_db::recurring::get_schedule(&mut *ctx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pm_schedule.updated".into(),
                entity_type: Some("pm_schedule".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": after.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::PM_SCHEDULE_UPDATED,
            "pm_schedule",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PmScheduleChanged {
                schedule: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "pm_schedule",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let before_json = serde_json::to_value(&before).unwrap_or(Value::Null);
        let stored = ScheduleMutationResponse {
            schedule: after.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: Some(before_json.clone()),
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "pm_schedule",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(before_json),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ScheduleMutationResponse {
                schedule: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/pm-schedules/{id} — removes the schedule. Existing
/// generated tickets are unaffected (templates are blueprints).
#[utoipa::path(operation_id = "pm_schedules_delete", delete, path = "/api/pm-schedules/{id}", tag = "pm-schedules", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Value),
              (status = 404, description = "schedule not found")))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::recurring::get_schedule(&mut *ctx.tx, org_id, id).await?;
        let removed = core_db::recurring::delete_schedule(&mut *ctx.tx, org_id, id).await?;
        if !removed {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "pm_schedule.deleted".into(),
                entity_type: Some("pm_schedule".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "name": before.name }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::PM_SCHEDULE_DELETED,
            "pm_schedule",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PmScheduleDeleted {
                schedule_id: id,
                name: before.name.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "deleted": true })))
    })
    .await
    .map(Json)
}

/// GET /api/pm-schedules/{id}/runs — the append-only run ledger (the run
/// record; the worker emits nothing).
#[utoipa::path(operation_id = "pm_schedules_runs", get, path = "/api/pm-schedules/{id}/runs", tag = "pm-schedules", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::PmRun]),
              (status = 404, description = "schedule not found")))]
pub async fn list_runs(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::PmRun>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // 404 when the schedule is not in scope; ledger rows for it follow.
    core_db::recurring::get_schedule(&mut *tx, org_id, id).await?;
    let runs = core_db::recurring::list_runs(&mut *tx, org_id, id, 100).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(runs))
}

/// Shared helper for the internal generator: validates a stored schedule
/// row's cadence and stamps the next occurrence after `slot`.
#[allow(dead_code)]
pub(crate) async fn stamp_next_for(pool: &sqlx::PgPool, schedule: &DuePmSchedule) -> ApiResult<()> {
    core_db::recurring::stamp_next_run(pool, schedule, None)
        .await
        .map_err(ApiError::from)?;
    Ok(())
}
