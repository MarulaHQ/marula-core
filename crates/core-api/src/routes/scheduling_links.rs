//! Calendly-style scheduling links (Wave 11 lane 4): meeting-type
//! templates, free-form admin-defined groups, shareable link tokens,
//! and the unauthenticated public booking endpoint.
//!
//! Purpose: client-facing booking built on the 2.12 availability
//! foundation. A completed booking IS a real appointment: the public
//! book endpoint re-validates availability + conflicts at book time
//! (windows − on-call − time-off − bookings, exactly like portal
//! appointment requests), flips the link through the single-winner
//! `WHERE status = 'active'` definer gate, and inserts the appointment
//! — all in ONE transaction, so a slot can never be booked twice by two
//! racing bookings (the appointments EXCLUDE constraint is the
//! concurrency backstop; the link-status predicate is the single-winner
//! gate). The template's auto-email rides the `email_outbound` exactly-
//! once ledger inside the booking transaction (transport `none` is the
//! documented skip).
//!
//! Non-purpose: reminders/recurrence, external calendar sync, the
//! ticket-workspace send UI (lane 6 consumes these endpoints), org-chart
//! hierarchy (groups are free-form availability pools by design).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use core_domain::entities::{
    Appointment, AppointmentKind, AppointmentStatus, CalendarExclusion, MeetingAssigneeScope,
    MeetingType, SchedulingGroup, SchedulingLink, SchedulingLinkBooking, SchedulingLinkStatus,
};
use core_domain::events::{
    types, SchedulingLinkCreated, SchedulingLinkRevoked, SchedulingLinkScheduled,
    SchedulingLinkViewed,
};
use core_domain::policy::{Action, Permission};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Scheduling,
        action,
    }
}

// --- rate limiting -------------------------------------------------------------

const RATE_WINDOW_SECS: u64 = 60;

/// Ceiling on live limiter keys.
///
/// The public surface has no principal, so the limiter is keyed by a
/// caller-supplied token: the key set is attacker-controlled. Without a
/// cap the map gains one entry per distinct token ever presented and nothing
/// ever leaves it — an entry's window expires, but the entry stays. At the
/// cap a request first sweeps windows that already expired (safe: an
/// expired window holds no state a live request depends on) and refuses
/// with the typed 429 when nothing expired, so retained memory is bounded
/// at a constant either way.
const RATE_WINDOW_MAX_KEYS: usize = 10_000;

#[derive(Debug)]
struct RateWindow(Instant, u32);

#[derive(Debug, Default)]
struct RateWindows {
    buckets: HashMap<String, RateWindow>,
    /// When the last capped-resize sweep ran. The sweep is O(keys) and this
    /// surface is unauthenticated, so it runs at most once per window: the
    /// cost of a request that arrives at the cap is a lookup and a length
    /// read, never a full scan.
    last_sweep: Option<Instant>,
    /// When the cap last refused a new token. The refusal is a 429 with no
    /// body difference from a per-key breach, so the warn is the only place an
    /// operator can see that the map itself is the reason. Throttled to one
    /// per window, because this surface is unauthenticated.
    last_cap_warn: Option<Instant>,
}

impl RateWindows {
    /// Records one hit for `key`. `Err(retry_after_secs)` is the refusal:
    /// either this key's own window is full, or the map sits at its cap
    /// with no expired window to reclaim.
    fn hit(&mut self, now: Instant, key: &str, limit_per_min: u32) -> Result<(), i64> {
        if !self.buckets.contains_key(key) && self.buckets.len() >= RATE_WINDOW_MAX_KEYS {
            let sweep_due = self.last_sweep.is_none_or(|swept| {
                now.saturating_duration_since(swept).as_secs() >= RATE_WINDOW_SECS
            });
            if sweep_due {
                self.buckets.retain(|_, window| {
                    now.saturating_duration_since(window.0).as_secs() < RATE_WINDOW_SECS
                });
                self.last_sweep = Some(now);
            }
            if self.buckets.len() >= RATE_WINDOW_MAX_KEYS {
                let warn_due = self.last_cap_warn.is_none_or(|warned| {
                    now.saturating_duration_since(warned).as_secs() >= RATE_WINDOW_SECS
                });
                if warn_due {
                    self.last_cap_warn = Some(now);
                    tracing::warn!(
                        keys = self.buckets.len(),
                        cap = RATE_WINDOW_MAX_KEYS,
                        "scheduling rate map is at capacity and no window expired: refusing a new token with 429"
                    );
                }
                return Err(i64::try_from(RATE_WINDOW_SECS).unwrap_or(i64::MAX));
            }
        }
        let window = self
            .buckets
            .entry(key.to_string())
            .or_insert(RateWindow(now, 0));
        if now.saturating_duration_since(window.0).as_secs() >= RATE_WINDOW_SECS {
            *window = RateWindow(now, 0);
        }
        window.1 += 1;
        if window.1 > limit_per_min {
            let elapsed = now.saturating_duration_since(window.0).as_secs();
            let retry_after = RATE_WINDOW_SECS.saturating_sub(elapsed).max(1);
            return Err(i64::try_from(retry_after).unwrap_or(i64::MAX));
        }
        Ok(())
    }
}

static RATE_WINDOWS: OnceLock<Mutex<RateWindows>> = OnceLock::new();

/// Fixed-window limiter keyed by token hash (the public surface has no
/// principal to key on). `limit == 0` disables limiting (documented
/// escape hatch); a breach carries the seconds until the window resets.
fn check_rate_limit(limit_per_min: u32, key: &str) -> ApiResult<()> {
    if limit_per_min == 0 {
        return Ok(());
    }
    let windows = RATE_WINDOWS.get_or_init(Default::default);
    let mut map = windows
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.hit(Instant::now(), key, limit_per_min)
        .map_err(|retry_after_secs| ApiError::RateLimited { retry_after_secs })
}

// --- tokens ----------------------------------------------------------------------

/// 32 bytes of OS randomness, URL-safe — unguessable by construction.
fn generate_token() -> String {
    use rand::TryRng;
    let mut bytes = [0_u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// --- free-form groups (availability pools) ----------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Instant-first autosave parameters (IF1/IF2).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GroupMutation {
    pub group: SchedulingGroup,
    pub mutation: instant::MutationMeta,
}

/// GET /api/scheduling-groups — the org's free-form groups (availability
/// pools; deliberately NOT the org chart).
#[utoipa::path(operation_id = "scheduling_groups_list", get, path = "/api/scheduling-groups", tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = Vec<SchedulingGroup>)))]
pub async fn list_groups(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<SchedulingGroup>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let groups = core_db::scheduling_links::list_groups(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(groups))
}

/// POST /api/scheduling-groups — create a pool.
#[utoipa::path(operation_id = "scheduling_groups_create", post, path = "/api/scheduling-groups", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateGroupRequest,
    responses((status = 201, body = SchedulingGroup), (status = 409, description = "name already taken")))]
pub async fn create_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateGroupRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<SchedulingGroup>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("name must not be empty".into()));
    }
    let now = Utc::now();
    let group = SchedulingGroup {
        id: Uuid::now_v7(),
        org_id: authed.principal.user.org_id,
        name,
        description: req.description,
        member_user_ids: Vec::new(),
        created_by: Some(authed.principal.user.id),
        created_at: now,
        updated_at: now,
    };
    let org_id = group.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling_links::create_group(&mut *tx.tx, &group)
            .await
            .map_err(ApiError::from)?;
        Ok((tx, group))
    })
    .await
    .map(|g| (axum::http::StatusCode::CREATED, Json(g)))
}

/// GET /api/scheduling-groups/{id}
#[utoipa::path(operation_id = "scheduling_groups_get", get, path = "/api/scheduling-groups/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = SchedulingGroup), (status = 404)))]
pub async fn get_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SchedulingGroup>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let group = core_db::scheduling_links::get_group(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(group))
}

/// PATCH /api/scheduling-groups/{id}
#[utoipa::path(operation_id = "scheduling_groups_update", patch, path = "/api/scheduling-groups/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = UpdateGroupRequest,
    responses((status = 200, body = GroupMutation), (status = 404), (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn update_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateGroupRequest>,
) -> ApiResult<Json<GroupMutation>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut tx,
            instant::ClaimSpec {
                entity: "scheduling_group",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((tx, MutationOutcome::Replayed(stored)));
        }
        let existing = core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        let name = req.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        core_db::scheduling_links::update_group(
            &mut *tx.tx,
            org_id,
            id,
            name.unwrap_or(&existing.name),
            req.description
                .as_deref()
                .or(existing.description.as_deref()),
        )
        .await
        .map_err(ApiError::from)?;
        let group = core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut tx,
            instant::SeqSpec {
                entity: "scheduling_group",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut tx,
            instant::RecordSpec {
                entity: "scheduling_group",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&existing).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&group).unwrap_or(serde_json::Value::Null),
                updated_at: group.updated_at,
                key: key.as_deref(),
                payload: &serde_json::json!({ "group": group }),
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            tx,
            MutationOutcome::Applied(GroupMutation { group, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/scheduling-groups/{id}/members — add a calendar to the pool.
#[utoipa::path(operation_id = "scheduling_groups_add_member", post, path = "/api/scheduling-groups/{id}/members", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = AddMemberRequest,
    responses((status = 200, body = SchedulingGroup), (status = 404)))]
pub async fn add_group_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddMemberRequest>,
) -> ApiResult<Json<SchedulingGroup>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        core_db::scheduling_links::add_group_member(&mut *tx.tx, org_id, id, req.user_id, actor)
            .await?;
        let group = core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        Ok((tx, group))
    })
    .await
    .map(Json)
}

/// DELETE /api/scheduling-groups/{id}/members/{user_id}
#[utoipa::path(operation_id = "scheduling_groups_remove_member", delete, path = "/api/scheduling-groups/{id}/members/{user_id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path), ("user_id" = uuid::Uuid, Path)),
    responses((status = 200, body = SchedulingGroup), (status = 404)))]
pub async fn remove_group_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<SchedulingGroup>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        core_db::scheduling_links::remove_group_member(&mut *tx.tx, org_id, id, user_id).await?;
        let group = core_db::scheduling_links::get_group(&mut *tx.tx, org_id, id).await?;
        Ok((tx, group))
    })
    .await
    .map(Json)
}

/// DELETE /api/scheduling-groups/{id} — removes the pool (memberships go
/// with it; meeting types pointed at it retire with their FK CASCADE,
/// mirroring Calendly's delete semantics).
#[utoipa::path(operation_id = "scheduling_groups_delete", delete, path = "/api/scheduling-groups/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 204), (status = 404)))]
pub async fn delete_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::scheduling_links::delete_group(&mut tx.tx, org_id, id).await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- meeting types -----------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MeetingTypeBody {
    pub name: String,
    pub description: Option<String>,
    pub duration_minutes: i32,
    #[serde(default)]
    pub buffer_minutes: i32,
    #[serde(default = "default_window")]
    pub booking_window_days: i32,
    /// Free-form org-edited payload (location hint, agenda, questions…).
    #[serde(default)]
    pub parameters: serde_json::Value,
    pub assignee_scope: MeetingAssigneeScope,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_user_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub assignee_group_id: Option<Uuid>,
    #[serde(default)]
    pub auto_email_enabled: bool,
    pub auto_email_subject: Option<String>,
    pub auto_email_body: Option<String>,
    /// PATCH-only toggle (absent on create = active).
    pub is_active: Option<bool>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

fn default_window() -> i32 {
    14
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MeetingTypeMutation {
    pub meeting_type: MeetingType,
    pub mutation: instant::MutationMeta,
}

/// GET /api/meeting-types
#[utoipa::path(operation_id = "meeting_types_list", get, path = "/api/meeting-types", tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = Vec<MeetingType>)))]
pub async fn list_meeting_types(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<MeetingType>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::scheduling_links::list_meeting_types(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

fn build_meeting_type(
    req: &MeetingTypeBody,
    existing: Option<MeetingType>,
    org_id: Uuid,
    actor: Uuid,
    now: DateTime<Utc>,
) -> ApiResult<MeetingType> {
    let mt = match existing {
        Some(mut e) => {
            e.name = req.name.trim().to_string();
            e.description = req.description.clone();
            e.duration_minutes = req.duration_minutes;
            e.buffer_minutes = req.buffer_minutes;
            e.booking_window_days = req.booking_window_days;
            e.parameters = req.parameters.clone();
            e.assignee_scope = req.assignee_scope;
            e.assignee_user_id = req.assignee_user_id;
            e.assignee_group_id = req.assignee_group_id;
            e.auto_email_enabled = req.auto_email_enabled;
            e.auto_email_subject = req.auto_email_subject.clone();
            e.auto_email_body = req.auto_email_body.clone();
            e.is_active = req.is_active.unwrap_or(true);
            e.updated_at = now;
            e
        }
        None => MeetingType {
            id: Uuid::now_v7(),
            org_id,
            name: req.name.trim().to_string(),
            description: req.description.clone(),
            duration_minutes: req.duration_minutes,
            buffer_minutes: req.buffer_minutes,
            booking_window_days: req.booking_window_days,
            parameters: req.parameters.clone(),
            assignee_scope: req.assignee_scope,
            assignee_user_id: req.assignee_user_id,
            assignee_group_id: req.assignee_group_id,
            auto_email_enabled: req.auto_email_enabled,
            auto_email_subject: req.auto_email_subject.clone(),
            auto_email_body: req.auto_email_body.clone(),
            is_active: req.is_active.unwrap_or(true),
            created_by: Some(actor),
            created_at: now,
            updated_at: now,
        },
    };
    mt.validate().map_err(ApiError::from)?;
    Ok(mt)
}

/// The scope target must exist in-org: a bad user/group id is a typed
/// 422 now, never a wedged future booking.
async fn check_scope_target(tx: &mut org_tx::OrgTx<'_>, mt: &MeetingType) -> ApiResult<()> {
    match mt.assignee_scope {
        MeetingAssigneeScope::User => {
            core_db::users::get_user(
                &mut *tx.tx,
                mt.org_id,
                mt.assignee_user_id.unwrap_or_default(),
            )
            .await
            .map_err(|_| ApiError::Validation("assignee_user_id not found in org".into()))?;
        }
        MeetingAssigneeScope::Group => {
            core_db::scheduling_links::get_group(
                &mut *tx.tx,
                mt.org_id,
                mt.assignee_group_id.unwrap_or_default(),
            )
            .await
            .map_err(|_| ApiError::Validation("assignee_group_id not found in org".into()))?;
        }
    }
    Ok(())
}

/// POST /api/meeting-types
#[utoipa::path(operation_id = "meeting_types_create", post, path = "/api/meeting-types", tag = "scheduling", security(("bearer" = [])),
    request_body = MeetingTypeBody,
    responses((status = 201, body = MeetingTypeMutation), (status = 422, description = "validation failure")))]
pub async fn create_meeting_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<MeetingTypeBody>,
) -> ApiResult<(axum::http::StatusCode, Json<MeetingTypeMutation>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let now = Utc::now();
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let mt = build_meeting_type(&req, None, org_id, actor, now)?;
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut tx,
            instant::ClaimSpec {
                entity: "meeting_type",
                entity_id: Some(mt.id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((tx, MutationOutcome::Replayed(stored)));
        }
        check_scope_target(&mut tx, &mt).await?;
        core_db::scheduling_links::create_meeting_type(&mut *tx.tx, &mt)
            .await
            .map_err(ApiError::from)?;
        let write_seq = instant::entity_seq(
            &mut tx,
            instant::SeqSpec {
                entity: "meeting_type",
                entity_id: mt.id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut tx,
            instant::RecordSpec {
                entity: "meeting_type",
                entity_id: mt.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&mt).unwrap_or(serde_json::Value::Null),
                updated_at: mt.updated_at,
                key: key.as_deref(),
                payload: &serde_json::json!({ "meeting_type": mt }),
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            tx,
            MutationOutcome::Applied(MeetingTypeMutation {
                meeting_type: mt,
                mutation,
            }),
        ))
    })
    .await?;
    let created = outcome.into_response()?;
    Ok((axum::http::StatusCode::CREATED, Json(created)))
}

/// PATCH /api/meeting-types/{id}
#[utoipa::path(operation_id = "meeting_types_update", patch, path = "/api/meeting-types/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = MeetingTypeBody,
    responses((status = 200, body = MeetingTypeMutation), (status = 404), (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn update_meeting_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<MeetingTypeBody>,
) -> ApiResult<Json<MeetingTypeMutation>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut tx,
            instant::ClaimSpec {
                entity: "meeting_type",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((tx, MutationOutcome::Replayed(stored)));
        }
        let existing = core_db::scheduling_links::get_meeting_type(&mut *tx.tx, org_id, id).await?;
        let mt = build_meeting_type(&req, Some(existing.clone()), org_id, actor, Utc::now())?;
        check_scope_target(&mut tx, &mt).await?;
        core_db::scheduling_links::update_meeting_type(&mut *tx.tx, &mt)
            .await
            .map_err(ApiError::from)?;
        let write_seq = instant::entity_seq(
            &mut tx,
            instant::SeqSpec {
                entity: "meeting_type",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut tx,
            instant::RecordSpec {
                entity: "meeting_type",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&existing).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&mt).unwrap_or(serde_json::Value::Null),
                updated_at: mt.updated_at,
                key: key.as_deref(),
                payload: &serde_json::json!({ "meeting_type": mt }),
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            tx,
            MutationOutcome::Applied(MeetingTypeMutation {
                meeting_type: mt,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeetingTypeDeleted {
    pub deleted: bool,
}

/// DELETE /api/meeting-types/{id} — HARD delete with audit (Wave 17 lane
/// 13): unlike the Pause toggle (PATCH `is_active`), this removes the
/// template outright. A scheduling link still referencing the row refuses
/// with a typed conflict (the FK RESTRICT surfaces) — pause the links
/// FIRST or re-target them; history rows (bookings) are untouched by the
/// template's deletion.
#[utoipa::path(operation_id = "meeting_types_delete", delete, path = "/api/meeting-types/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = MeetingTypeDeleted),
              (status = 404, description = "meeting type not visible in scope"),
              (status = 409, description = "a scheduling link still references this meeting type")))]
pub async fn delete_meeting_type(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MeetingTypeDeleted>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        // Load first so 404 is visible-from-scope and the audit + event
        // carry the exact record that went away.
        let existing =
            core_db::scheduling_links::get_meeting_type(&mut *tx.tx, org_id, id).await?;
        let deleted =
            core_db::scheduling_links::delete_meeting_type(&mut *tx.tx, org_id, id)
                .await
                .map_err(|e| match &e {
                    // FK RESTRICT: scheduling_links.meeting_type_id — the
                    // proof of "in use", surfaced honestly instead of a 500.
                    core_db::DbError::Sql(sqlx::Error::Database(db))
                        if db.constraint()
                            == Some("scheduling_links_meeting_type_id_fkey") =>
                    {
                        ApiError::Conflict(
                            "this meeting type still has scheduling links pointing at it — pause or re-target those links first"
                                .into(),
                        )
                    }
                    _ => ApiError::from(e),
                })?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "meeting_type.deleted".into(),
                entity_type: Some("meeting_type".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": existing.name,
                    "is_active": existing.is_active,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        tx.emit(
            core_domain::events::types::MEETING_TYPE_DELETED,
            "meeting_type",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::MeetingTypeDeleted {
                meeting_type_id: id,
                name: existing.name.clone(),
                deleted_by: actor,
                deleted_at: chrono::Utc::now(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((tx, MeetingTypeDeleted { deleted: true }))
    })
    .await?;
    Ok(Json(outcome))
}

// --- links --------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateSchedulingLinkRequest {
    #[schema(value_type = uuid::Uuid)]
    pub meeting_type_id: Uuid,
    pub label: Option<String>,
    /// Bookings made through this link land back on this ticket.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// Absolute expiry (lifecycle signal); absent = stays valid until
    /// used or revoked.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// The create response. `token` is the RAW shareable token — shown ONCE,
/// never stored server-side (only its SHA-256 hash).
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreatedLink {
    pub link: SchedulingLink,
    pub token: String,
    /// Relative booking path clients render against the public origin.
    pub booking_path: String,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct LinkQuery {
    #[serde(default)]
    pub ticket_id: Option<Uuid>,
}

/// GET /api/scheduling-links — newest-first (optionally per ticket) so
/// the tech sees the created / viewed / scheduled / expired lifecycle.
#[utoipa::path(operation_id = "scheduling_links_list", get, path = "/api/scheduling-links", tag = "scheduling", security(("bearer" = [])),
    params(LinkQuery),
    responses((status = 200, body = Vec<SchedulingLink>)))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<LinkQuery>,
) -> ApiResult<Json<Vec<SchedulingLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let mut rows = core_db::scheduling_links::list_links(&mut *tx, org_id).await?;
    if let Some(t) = q.ticket_id {
        rows.retain(|l| l.ticket_id == Some(t));
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/scheduling-links — mint a shareable link (token returned
/// once). A replay (same key + same body) replays the SAME token.
#[utoipa::path(operation_id = "scheduling_links_create", post, path = "/api/scheduling-links", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateSchedulingLinkRequest,
    responses((status = 201, body = CreatedLink), (status = 404, description = "meeting type not found")))]
pub async fn create_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateSchedulingLinkRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<CreatedLink>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let token = generate_token();
    let token_hash = hash_token(&token);
    let now = Utc::now();
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let link = SchedulingLink {
        id: Uuid::now_v7(),
        org_id,
        meeting_type_id: req.meeting_type_id,
        label: req.label.clone(),
        ticket_id: req.ticket_id,
        status: SchedulingLinkStatus::Active,
        expires_at: req.expires_at,
        first_viewed_at: None,
        view_count: 0,
        scheduled_at: None,
        scheduled_appointment_id: None,
        created_by: Some(actor),
        created_at: now,
        updated_at: now,
    };
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut tx,
            instant::ClaimSpec {
                entity: "scheduling_link",
                entity_id: Some(link.id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((tx, MutationOutcome::Replayed(stored)));
        }
        // The template must exist AND be active — a retired template
        // cannot mint new booking surfaces.
        let meeting_type =
            core_db::scheduling_links::get_meeting_type(&mut *tx.tx, org_id, link.meeting_type_id)
                .await
                .map_err(ApiError::from)?;
        if !meeting_type.is_active {
            return Err(ApiError::Validation(
                "meeting type is retired; links cannot be minted".into(),
            ));
        }
        core_db::scheduling_links::create_link(&mut *tx.tx, &link, &token_hash)
            .await
            .map_err(ApiError::from)?;
        let created = core_db::scheduling_links::get_link(&mut *tx.tx, org_id, link.id).await?;
        tx.emit(
            types::SCHEDULING_LINK_CREATED,
            "scheduling_link",
            Some(link.id.to_string().as_str()),
            &serde_json::to_value(SchedulingLinkCreated {
                link: created.clone(),
                created_by: actor,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut tx,
            instant::SeqSpec {
                entity: "scheduling_link",
                entity_id: link.id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut tx,
            instant::RecordSpec {
                entity: "scheduling_link",
                entity_id: link.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&created).unwrap_or(serde_json::Value::Null),
                updated_at: created.updated_at,
                key: key.as_deref(),
                payload: &serde_json::json!({
                    "link": created, "token": token, "booking_path": booking_path(&token)
                }),
                provenance: Default::default(),
            },
        )
        .await?;
        let path = booking_path(&token);
        Ok((
            tx,
            MutationOutcome::Applied(CreatedLink {
                link: created,
                token,
                booking_path: path,
                mutation,
            }),
        ))
    })
    .await?;
    let created @ CreatedLink { .. } = outcome.into_response()?;
    Ok((axum::http::StatusCode::CREATED, Json(created)))
}

fn booking_path(token: &str) -> String {
    format!("/s/{token}")
}

/// POST /api/scheduling-links/{id}/revoke — the link can never book
/// again; an already-used link keeps its `used` state so the booking
/// trail never changes meaning.
#[utoipa::path(operation_id = "scheduling_links_revoke", post, path = "/api/scheduling-links/{id}/revoke", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = SchedulingLink), (status = 404)))]
pub async fn revoke_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SchedulingLink>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let link = core_db::scheduling_links::revoke_link(&mut tx.tx, org_id, id).await?;
        if link.status == SchedulingLinkStatus::Revoked {
            tx.emit(
                types::SCHEDULING_LINK_REVOKED,
                "scheduling_link",
                Some(link.id.to_string().as_str()),
                &serde_json::to_value(SchedulingLinkRevoked {
                    link_id: id,
                    revoked_by: actor,
                    revoked_at: Utc::now(),
                })
                .unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        Ok((tx, link))
    })
    .await
    .map(Json)
}

/// GET /api/scheduling-links/{id}/bookings — the link → booking trail.
#[utoipa::path(operation_id = "scheduling_links_bookings", get, path = "/api/scheduling-links/{id}/bookings", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = Vec<SchedulingLinkBooking>), (status = 404)))]
pub async fn list_link_bookings(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<SchedulingLinkBooking>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::scheduling_links::get_link(&mut *tx, org_id, id).await?;
    let rows = core_db::scheduling_links::list_link_bookings(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

// --- PUBLIC booking surface (token-scoped, unauthenticated) -------------------------

/// Availability is computed in bounded server-side chunks over the
/// bookable horizon (Wave 12 lane 6 lifted the old 31-day calendar cap —
/// the horizon now follows the template's `booking_window_days`, up to
/// its 1..=365 CHECK).
const HORIZON_CHUNK_DAYS: i64 = 31;
/// Payload bound for the public preview: after merging, at most this many
/// free intervals are returned (the earliest ones). A 365-day window over
/// a busy calendar would otherwise render an unusable payload.
const MAX_PREVIEW_SLOTS: usize = 400;

/// What the public page needs to render the booking UI: the template's
/// bookable shape plus a slot preview over the booking window. The
/// horizon follows `booking_window_days`; intervals are computed in
/// server-side chunks and capped at [`MAX_PREVIEW_SLOTS`].
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicLinkView {
    #[schema(value_type = uuid::Uuid)]
    pub link_id: Uuid,
    pub label: Option<String>,
    pub meeting_type_name: String,
    pub meeting_type_description: Option<String>,
    pub duration_minutes: i32,
    pub buffer_minutes: i32,
    pub booking_window_days: i32,
    pub parameters: serde_json::Value,
    /// Absolute expiry, when the tech set one.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub expires_at: Option<DateTime<Utc>>,
    pub is_expired: bool,
    /// Free intervals (unioned across the assignee scope) within the
    /// bookable horizon; clients slice them into bookable choices.
    pub slots: Vec<SlotInterval>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SlotInterval {
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
}

/// GET /api/public/scheduling-links/{token} — unauthenticated, token-
/// scoped, rate-limited. First view stamps the link's lifecycle
/// (`first_viewed_at` + count) and emits `scheduling_link.viewed`.
#[utoipa::path(operation_id = "public_scheduling_link_view", get, path = "/api/public/scheduling-links/{token}", tag = "scheduling",
    params(("token" = String, Path)),
    responses((status = 200, body = PublicLinkView), (status = 404, description = "unknown token"), (status = 409, description = "revoked or already used"), (status = 429)))]
pub async fn public_view(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Json<PublicLinkView>> {
    check_rate_limit(state.config.scheduling_links_rate_limit_per_min, &token)?;
    let token_hash = hash_token(&token);
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    let resolved = core_db::scheduling_links::resolve_link_by_token(&mut conn, &token_hash).await?;
    if resolved.link_status == "used" {
        return Err(ApiError::Conflict(
            "this scheduling link has already been used".into(),
        ));
    }
    if resolved.link_status == "revoked" {
        return Err(ApiError::Conflict(
            "this scheduling link has been revoked".into(),
        ));
    }
    if !resolved.meeting_type.is_active {
        return Err(ApiError::Conflict(
            "this meeting type is no longer bookable".into(),
        ));
    }
    let now = Utc::now();
    let is_expired = resolved.expires_at.map(|e| e <= now).unwrap_or(false);
    if is_expired {
        return Err(ApiError::Conflict(
            "this scheduling link has expired".into(),
        ));
    }
    // Lifecycle stamp + viewed event (org-bound after resolution).
    core_db::scheduling_links::mark_link_viewed(&mut conn, &token_hash)
        .await
        .map_err(ApiError::from)?;
    drop(conn);
    org_tx::with_org_tx(&state, resolved.org_id, Uuid::nil(), |mut tx| async move {
        tx.emit(
            types::SCHEDULING_LINK_VIEWED,
            "scheduling_link",
            Some(resolved.link_id.to_string().as_str()),
            &serde_json::to_value(SchedulingLinkViewed {
                link_id: resolved.link_id,
                viewed_at: now,
                view_count: resolved.view_count + 1,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;

    // The horizon follows the template's booking window — the 31-day
    // calendar cap was lifted (Wave 12 lane 6); the window itself is
    // bounded 1..=365 by the meeting-type CHECK.
    let horizon = now + Duration::days(resolved.meeting_type.booking_window_days.into());
    let slots = free_slots(
        &state,
        resolved.org_id,
        &resolved.meeting_type,
        now,
        horizon,
    )
    .await?;
    Ok(Json(PublicLinkView {
        link_id: resolved.link_id,
        label: resolved.label.clone(),
        meeting_type_name: resolved.meeting_type.name.clone(),
        meeting_type_description: resolved.meeting_type.description.clone(),
        duration_minutes: resolved.meeting_type.duration_minutes,
        buffer_minutes: resolved.meeting_type.buffer_minutes,
        booking_window_days: resolved.meeting_type.booking_window_days,
        parameters: resolved.meeting_type.parameters.clone(),
        expires_at: resolved.expires_at,
        is_expired,
        slots,
    }))
}

/// Unioned free intervals across the assignee scope over [from, to]:
/// one user's calendar, or every group member's calendar combined.
/// Availability is computed in [`HORIZON_CHUNK_DAYS`] chunks server-side
/// so a long horizon keeps per-query cost bounded, and the merged result
/// is capped at [`MAX_PREVIEW_SLOTS`] earliest intervals.
async fn free_slots(
    state: &AppState,
    org_id: Uuid,
    meeting_type: &MeetingType,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> ApiResult<Vec<SlotInterval>> {
    if to <= from {
        return Ok(Vec::new());
    }
    let member_ids = assignee_ids(state, org_id, meeting_type).await?;
    if member_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut unioned: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    let mut cursor = from;
    while cursor < to {
        let chunk_end = (cursor + Duration::days(HORIZON_CHUNK_DAYS)).min(to);
        for user_id in &member_ids {
            unioned.extend(member_free(state, org_id, *user_id, cursor, chunk_end).await?);
        }
        cursor = chunk_end;
    }
    unioned.sort();
    let merged = merge_sorted(unioned);
    Ok(merged
        .into_iter()
        .take(MAX_PREVIEW_SLOTS)
        .map(|(s, e)| SlotInterval {
            starts_at: s,
            ends_at: e,
        })
        .collect())
}

/// The calendars a template books against.
async fn assignee_ids(
    state: &AppState,
    org_id: Uuid,
    meeting_type: &MeetingType,
) -> ApiResult<Vec<Uuid>> {
    match meeting_type.assignee_scope {
        MeetingAssigneeScope::User => Ok(meeting_type
            .assignee_user_id
            .map(|u| vec![u])
            .unwrap_or_default()),
        MeetingAssigneeScope::Group => {
            let group_id = meeting_type
                .assignee_group_id
                .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("group scope without group")))?;
            let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
            core_db::set_org(&mut tx, org_id)
                .await
                .map_err(ApiError::from)?;
            let group = core_db::scheduling_links::get_group(&mut *tx, org_id, group_id).await?;
            tx.commit().await.map_err(ApiError::from)?;
            Ok(group.member_user_ids)
        }
    }
}

/// One member's free intervals: availability windows − exclusions −
/// busy (live appointments + on-call). The 2.12 primitive set, verbatim.
async fn member_free(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> ApiResult<Vec<(DateTime<Utc>, DateTime<Utc>)>> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let ids = [user_id];
    let windows = core_db::scheduling::list_availability_windows(&mut *tx, org_id, user_id).await?;
    if windows.is_empty() {
        tx.commit().await.map_err(ApiError::from)?;
        return Ok(Vec::new());
    }
    let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> =
        core_db::scheduling::list_appointments_between(&mut *tx, org_id, &ids, from, to)
            .await?
            .into_iter()
            .filter(|a| a.status != AppointmentStatus::Cancelled)
            .map(|a| (a.starts_at, a.ends_at))
            .collect();
    busy.extend(
        core_db::scheduling::list_on_call_blocks_between(&mut *tx, org_id, &ids, from, to)
            .await?
            .into_iter()
            .map(|b| (b.starts_at, b.ends_at)),
    );
    // External busy time (two-way calendar sync, Wave 14 lane 2): the
    // same book-time double-check as portal bookings — synced busy time
    // is just more busy time to the availability math.
    busy.extend(
        core_db::calendar_sync::list_busy_spans_between(&mut *tx, org_id, user_id, from, to)
            .await?,
    );
    let exclusions: Vec<CalendarExclusion> =
        core_db::scheduling::list_calendar_exclusions_for_users_between(
            &mut *tx, org_id, &ids, from, to,
        )
        .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(core_domain::scheduling::free_intervals(
        &windows,
        &exclusions,
        &busy,
        from,
        to,
    ))
}

/// Sort + merge a list of possibly-overlapping intervals into sorted,
/// internally disjoint form (the union across member calendars).
fn merge_sorted(
    mut spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    spans.sort();
    let mut out: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (s, e) in spans {
        if let Some(last) = out.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        out.push((s, e));
    }
    out
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BookRequest {
    /// Slot start (the template's duration extends it). Must sit fully
    /// inside ONE free interval of the scope at book time.
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    pub booker_name: Option<String>,
    pub booker_email: Option<String>,
    pub booker_notes: Option<String>,
    /// Instant-first autosave parameters (IF1): debounce/retry safe.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BookResponse {
    pub appointment: Appointment,
    pub mutation: instant::MutationMeta,
}

/// POST /api/public/scheduling-links/{token}/book — unauthenticated but
/// token-scoped and rate-limited. Race-safe exactly like portal
/// appointment-request approval: the availability + conflict gate and
/// the single-winner link flip run INSIDE the booking transaction, and
/// the appointments EXCLUDE constraint is the concurrency backstop.
#[utoipa::path(operation_id = "public_scheduling_link_book", post, path = "/api/public/scheduling-links/{token}/book", tag = "scheduling",
    params(("token" = String, Path)), request_body = BookRequest,
    responses((status = 201, body = BookResponse),
              (status = 404, description = "unknown token"),
              (status = 409, description = "revoked/used/expired link, slot no longer free, double-booking backstop, or idempotency key reused"),
              (status = 422, description = "slot outside the link's availability or booking window"),
              (status = 429)))]
pub async fn public_book(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(req): Json<BookRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<BookResponse>)> {
    check_rate_limit(state.config.scheduling_links_rate_limit_per_min, &token)?;
    let token_hash = hash_token(&token);
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    let resolved = core_db::scheduling_links::resolve_link_by_token(&mut conn, &token_hash).await?;
    drop(conn);
    let now = Utc::now();
    if resolved.link_status != "active" {
        return Err(ApiError::Conflict(format!(
            "this scheduling link is no longer active (status: {})",
            resolved.link_status
        )));
    }
    if !resolved.meeting_type.is_active {
        return Err(ApiError::Conflict(
            "this meeting type is no longer bookable".into(),
        ));
    }
    if resolved.expires_at.map(|e| e <= now).unwrap_or(false) {
        return Err(ApiError::Conflict(
            "this scheduling link has expired".into(),
        ));
    }
    let ends_at = req.starts_at + Duration::minutes(resolved.meeting_type.duration_minutes.into());
    if ends_at <= req.starts_at {
        return Err(ApiError::Validation("duration must be positive".into()));
    }
    if req.starts_at < now {
        return Err(ApiError::Validation("the slot is in the past".into()));
    }
    if req.starts_at > now + Duration::days(resolved.meeting_type.booking_window_days.into()) {
        return Err(ApiError::Validation(
            "the slot is beyond this link's booking window".into(),
        ));
    }
    // Buffered occupancy: the template's buffer reserves breathing room
    // around the meeting on the chosen calendar.
    let buffer = Duration::minutes(resolved.meeting_type.buffer_minutes.into());
    let busy_span_start = req.starts_at - buffer;
    let busy_span_end = ends_at + buffer;

    let org_id = resolved.org_id;
    let meeting_type = resolved.meeting_type.clone();
    let meeting_type_for_title = meeting_type.clone();
    let booker_name = req.booker_name.clone();
    let booker_email = req.booker_email.clone();
    let booker_notes = req.booker_notes.clone();
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let mail = state.mail.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, Uuid::nil(), |mut tx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut tx,
            instant::ClaimSpec {
                entity: "scheduling_link_booking",
                entity_id: Some(resolved.link_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((tx, MutationOutcome::Replayed(stored)));
        }
        let member_ids = {
            match meeting_type.assignee_scope {
                MeetingAssigneeScope::User => meeting_type
                    .assignee_user_id
                    .map(|u| vec![u])
                    .unwrap_or_default(),
                MeetingAssigneeScope::Group => {
                    let group_id = meeting_type.assignee_group_id.ok_or_else(|| {
                        ApiError::Internal(anyhow::anyhow!("group scope without group"))
                    })?;
                    core_db::scheduling_links::get_group(&mut *tx.tx, org_id, group_id)
                        .await
                        .map_err(ApiError::from)?
                        .member_user_ids
                }
            }
        };
        if member_ids.is_empty() {
            return Err(ApiError::Conflict(
                "no staff calendars are assigned to this meeting type".into(),
            ));
        }
        // THE book-time gate (2.12, verbatim semantics): the slot must
        // sit fully inside ONE member's free interval, and no OTHER
        // booking may overlap the buffered span on that member's
        // calendar. Availability and occupancy are re-read NOW — the
        // slot preview the client saw may be stale.
        let mut chosen: Option<Uuid> = None;
        for user_id in member_ids {
            let ids = [user_id];
            let windows =
                core_db::scheduling::list_availability_windows(&mut *tx.tx, org_id, user_id)
                    .await?;
            if windows.is_empty() {
                continue;
            }
            let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> =
                core_db::scheduling::list_appointments_between(
                    &mut *tx.tx,
                    org_id,
                    &ids,
                    busy_span_start,
                    busy_span_end,
                )
                .await?
                .into_iter()
                .filter(|a| a.status != AppointmentStatus::Cancelled)
                .map(|a| (a.starts_at, a.ends_at))
                .collect();
            busy.extend(
                core_db::scheduling::list_on_call_blocks_between(
                    &mut *tx.tx,
                    org_id,
                    &ids,
                    busy_span_start,
                    busy_span_end,
                )
                .await?
                .into_iter()
                .map(|b| (b.starts_at, b.ends_at)),
            );
            // External busy time counts here too — the same gate.
            busy.extend(
                core_db::calendar_sync::list_busy_spans_between(
                    &mut *tx.tx,
                    org_id,
                    user_id,
                    busy_span_start,
                    busy_span_end,
                )
                .await?,
            );
            if core_domain::scheduling::conflict_indices(
                busy_span_start,
                busy_span_end,
                busy.iter().copied(),
            )
            .is_empty()
            {
                let exclusions: Vec<CalendarExclusion> =
                    core_db::scheduling::list_calendar_exclusions_for_users_between(
                        &mut *tx.tx,
                        org_id,
                        &ids,
                        busy_span_start,
                        busy_span_end,
                    )
                    .await?;
                let free = core_domain::scheduling::free_intervals(
                    &windows,
                    &exclusions,
                    &busy,
                    busy_span_start,
                    busy_span_end,
                );
                if free
                    .iter()
                    .any(|(s, e)| *s <= req.starts_at && ends_at <= *e)
                {
                    chosen = Some(user_id);
                    break;
                }
            }
        }
        let user_id = chosen.ok_or_else(|| {
            ApiError::Conflict("that slot is no longer free; please pick another".into())
        })?;

        // Single-winner gate: only an ACTIVE link flips; two racing
        // bookings cannot both claim it.
        let won =
            core_db::scheduling_links::mark_link_scheduled(&mut *tx.tx, &token_hash, Uuid::nil())
                .await
                .map_err(ApiError::from)?;
        if !won {
            return Err(ApiError::Conflict(
                "this scheduling link has already been used or revoked".into(),
            ));
        }

        let appointment = Appointment {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            ticket_id: None,
            client_id: None,
            site_id: None,
            title: format!(
                "Booked: {}",
                resolved
                    .label
                    .clone()
                    .unwrap_or_else(|| meeting_type_for_title.name.clone())
            ),
            starts_at: req.starts_at,
            ends_at,
            kind: AppointmentKind::Remote,
            status: AppointmentStatus::Scheduled,
            travel_minutes: 0,
            notes: booker_notes.clone(),
            scheduling_link_id: Some(resolved.link_id),
            travel_from: None,
            travel_distance_meters: None,
            travel_provider: None,
            travel_quality: None,
            created_at: now,
            updated_at: now,
        };
        // The EXCLUDE constraint re-validates the real span atomically.
        if let Err(e) = core_db::scheduling::create_appointment(&mut *tx.tx, &appointment).await {
            if matches!(
                &e,
                core_db::DbError::Sql(sqlx::Error::Database(db))
                    if db.constraint().map(|c| c.contains("appointments")).unwrap_or(false)
            ) {
                return Err(ApiError::Conflict(
                    "that slot was just booked by someone else; please pick another".into(),
                ));
            }
            return Err(ApiError::from(e));
        }
        // Point the link at the real appointment (idempotent flip within
        // this transaction — the gate above already won it).
        sqlx::query(
            "UPDATE scheduling_links SET scheduled_appointment_id = $3, scheduled_at = now()
             WHERE id = $1 AND org_id = $2 AND status = 'used'",
        )
        .bind(resolved.link_id)
        .bind(org_id)
        .bind(appointment.id)
        .execute(&mut *tx.tx)
        .await
        .map_err(ApiError::from)?;
        core_db::scheduling_links::create_link_booking(
            &mut *tx.tx,
            &SchedulingLinkBooking {
                link_id: resolved.link_id,
                appointment_id: appointment.id,
                booker_name: booker_name.clone(),
                booker_email: booker_email.clone(),
                booker_notes: booker_notes.clone(),
                created_at: now,
            },
        )
        .await
        .map_err(ApiError::from)?;
        // The booking enters the world through the normal creation event;
        // the link lifecycle event rides alongside it.
        tx.emit(
            types::APPOINTMENT_CREATED,
            "appointment",
            Some(appointment.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::AppointmentCreated {
                appointment: appointment.clone(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        tx.emit(
            types::SCHEDULING_LINK_SCHEDULED,
            "scheduling_link",
            Some(resolved.link_id.to_string().as_str()),
            &serde_json::to_value(SchedulingLinkScheduled {
                link_id: resolved.link_id,
                appointment_id: appointment.id,
                booker_name: booker_name.clone(),
                booker_email: booker_email.clone(),
                scheduled_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut tx,
            instant::SeqSpec {
                entity: "scheduling_link",
                entity_id: resolved.link_id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut tx,
            instant::RecordSpec {
                entity: "scheduling_link",
                entity_id: resolved.link_id,
                op: "book",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&appointment).unwrap_or(serde_json::Value::Null),
                updated_at: appointment.updated_at,
                key: key.as_deref(),
                payload: &serde_json::json!({ "appointment": appointment }),
                provenance: Default::default(),
            },
        )
        .await?;
        // The template's auto-email rides the `email_outbound` exactly-once
        // ledger INSIDE the booking transaction (Wave 12 lane 6 — the old
        // post-commit fire-and-forget only logged a transport failure). The
        // worker claims, transports, and retries with backoff; an idempotent
        // replay returned above before any enqueue, so a booking can never
        // double-send. `ticket_id` (when the link was minted from a ticket)
        // puts the confirmation on that ticket's email timeline. Transport
        // `none` remains the documented skip. Only the BODY expands the
        // `{date}`/`{name}` placeholders, matching the lane-4 template.
        if meeting_type.auto_email_enabled {
            if let (Some(subject), Some(body), Some(to)) = (
                meeting_type.auto_email_subject.as_deref(),
                meeting_type.auto_email_body.as_deref(),
                booker_email.as_deref().filter(|e| e.contains('@')),
            ) {
                if mail.transport.name() != "none" {
                    let body = body
                        .replace("{date}", &appointment.starts_at.to_rfc3339())
                        .replace("{name}", booker_name.as_deref().unwrap_or("there"));
                    crate::email::enqueue_outbound(
                        &mut tx.tx,
                        crate::email::OutboundEnqueue {
                            mail: &mail,
                            org_id,
                            to_addresses: vec![to.to_string()],
                            kind: &core_mail::outbound::OutboundKind::Templated {
                                subject: subject.to_string(),
                                body,
                            },
                            in_reply_to: None,
                            references: vec![],
                            ticket_id: resolved.ticket_id,
                            // A public booking is written by the BOOKER, not
                            // by a member: nobody on the org side signed it,
                            // so the personal rungs are skipped and the
                            // ticket's board (then the org) closes the mail.
                            signer: None,
                            overrides: crate::email::EnvelopeOverrides::default(),
                        },
                    )
                    .await?;
                }
            }
        }
        Ok((
            tx,
            MutationOutcome::Applied(BookResponse {
                appointment,
                mutation,
            }),
        ))
    })
    .await?;
    let booked = outcome.into_response()?;
    Ok((axum::http::StatusCode::CREATED, Json(booked)))
}

/// Public-route mount: merged OUTSIDE the JWT middleware (alerts
/// pattern). The handler resolves org scope itself.
pub fn public_routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    use axum::routing::{get, post};
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/public/scheduling-links/{{token}}"),
            get(public_view),
        )
        .route(
            &format!("{base}/public/scheduling-links/{{token}}/book"),
            post(public_book),
        )
}

#[cfg(test)]
mod rate_window_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_full_map_refuses_a_new_token_instead_of_growing() {
        let mut windows = RateWindows::default();
        let now = Instant::now();
        for i in 0..RATE_WINDOW_MAX_KEYS {
            windows
                .hit(now, &format!("token-{i}"), 60)
                .expect("a fresh key admits its first hit");
        }
        assert_eq!(windows.buckets.len(), RATE_WINDOW_MAX_KEYS);
        let refused = windows.hit(now, "token-over-the-cap", 60);
        assert!(refused.is_err(), "the cap refuses rather than growing");
        assert_eq!(
            windows.buckets.len(),
            RATE_WINDOW_MAX_KEYS,
            "a refused token is never inserted"
        );
    }

    #[test]
    fn expired_windows_are_reclaimed_before_the_cap_refuses() {
        let mut windows = RateWindows::default();
        let stale = Instant::now() - Duration::from_secs(RATE_WINDOW_SECS * 2);
        for i in 0..RATE_WINDOW_MAX_KEYS {
            windows
                .hit(stale, &format!("token-{i}"), 60)
                .expect("a fresh key admits its first hit");
        }
        // Every retained window has expired, so the sweep frees the space a
        // new token needs instead of refusing it.
        assert!(windows.hit(Instant::now(), "token-fresh", 60).is_ok());
        assert_eq!(
            windows.buckets.len(),
            1,
            "the sweep drops every expired entry"
        );
    }

    #[test]
    fn one_token_over_its_limit_still_carries_a_retry_hint() {
        let mut windows = RateWindows::default();
        let now = Instant::now();
        assert!(windows.hit(now, "token-a", 2).is_ok());
        assert!(windows.hit(now, "token-a", 2).is_ok());
        let retry_after = windows
            .hit(now, "token-a", 2)
            .expect_err("the third hit is over a two-per-minute limit");
        assert!(retry_after >= 1, "Retry-After is at least one second");
    }
}
