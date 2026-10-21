//! Directory groups (Wave 52 lane R10): the identity construct an external
//! directory synchronizes into, and the seam that makes a group's authority
//! real.
//!
//! Purpose: a group is storage plus a role mapping, and the mapping is what
//! gives it authority. Membership and role mapping change what people may do,
//! so every write here emits `directory_group.*` and an audit row in the same
//! commit. The permission union itself lives one layer down
//! (`core_db::users::effective_roles_for_user`) and is what every principal the
//! API builds is made of, so a role mapped onto a group is granted through the
//! same path as a directly assigned role, not a parallel one.
//!
//! Permissions: reads are manager+ (any of `special:manage_users` /
//! `special:manage_roles`, the same gate the roles surface uses, because this
//! is the same roster read a different way). Every write requires
//! `special:manage_roles`: adding a person to a group or a role to a group is a
//! grant, and a grant is a roles-admin act.
//!
//! Envelope: group create/update/delete ride IF1–IF3 (idempotency-keyed claim,
//! version-gated seq, read-your-write `mutation` metadata). Membership and role
//! writes are addressed per element (`PUT`/`DELETE …/{userId}`,
//! `…/{roles}/{roleId}`), so they are naturally idempotent set operations and
//! carry no version — the same split the board-membership surface uses.
//!
//! Non-purpose: no sync protocol, no SCIM endpoint, no token, no reconciliation
//! loop, and no tier check. A premium identity product speaks its protocol to
//! this API; core just stores and authorizes what it is told.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx::{self, OrgTx};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::{get, put},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// The entity family for the write trail, the idempotency claim and the
/// contract events. One name, so a write-trail read can find every row this
/// surface ever wrote.
const ENTITY: &str = "directory_group";

const NAME_MAX: usize = 64;
const DESCRIPTION_MAX: usize = 500;
const EXTERNAL_ID_MAX: usize = 256;

/// Admin gate for every write: a group's membership and its roles are grants.
fn manage_roles() -> Permission {
    Permission::ManageRoles
}

/// Manager+ read (the org-users surface reads this for its pickers too).
fn require_reader(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(
        &principal.permissions,
        &[Permission::ManageUsers, Permission::ManageRoles],
    )
    .expect()
    .map_err(ApiError::Forbidden)
}

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/directory-groups"), get(list).post(create))
        .route(
            &format!("{base}/directory-groups/{{id}}"),
            get(detail_handler).patch(update).delete(remove),
        )
        .route(
            &format!("{base}/directory-groups/{{id}}/members/{{userId}}"),
            put(add_member).delete(remove_member),
        )
        .route(
            &format!("{base}/directory-groups/{{id}}/roles/{{roleId}}"),
            put(map_role).delete(unmap_role),
        )
}

// --- wire structs ------------------------------------------------------------

/// One directory group, with the counts a listing shows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DirectoryGroupView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    /// Free-text purpose. `null` when unset.
    #[schema(nullable = true)]
    pub description: Option<String>,
    /// The identity provider's identifier for this group, `null` for a group
    /// created in Marula that no directory knows about. Unique per org.
    #[schema(nullable = true)]
    pub external_id: Option<String>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Active and inactive users in the group.
    pub member_count: i64,
    /// Roles the group carries.
    pub role_count: i64,
}

impl DirectoryGroupView {
    fn from_row(group: core_db::directory_groups::DirectoryGroup) -> Self {
        Self {
            id: group.id,
            org_id: group.org_id,
            name: group.name,
            description: group.description,
            external_id: group.external_id,
            created_at: group.created_at,
            updated_at: group.updated_at,
            member_count: group.member_count,
            role_count: group.role_count,
        }
    }
}

/// One member of a group.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DirectoryGroupMemberView {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    /// A deactivated user keeps the membership row; the principal is refused at
    /// auth instead, so nothing needs unwinding when someone leaves.
    pub is_active: bool,
    #[schema(value_type = String)]
    pub added_at: chrono::DateTime<chrono::Utc>,
    /// Who added them: a staff user id, `null` for a write that came from a
    /// sync.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub added_by: Option<Uuid>,
}

/// One role a group carries.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DirectoryGroupRoleView {
    #[schema(value_type = uuid::Uuid)]
    pub role_id: Uuid,
    pub name: String,
    /// Shipped preset tag (owner, admin, …), `null` for a custom role.
    #[schema(nullable = true)]
    pub builtin: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DirectoryGroupsListResponse {
    /// Every group in the org, by name.
    pub groups: Vec<DirectoryGroupView>,
}

/// One group with its current members and role mappings.
#[derive(Debug, Serialize, ToSchema)]
pub struct DirectoryGroupDetailResponse {
    pub group: DirectoryGroupView,
    pub members: Vec<DirectoryGroupMemberView>,
    pub roles: Vec<DirectoryGroupRoleView>,
}

/// Instant-first mutation response (IF2): the group fields at the top level
/// plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DirectoryGroupMutationResponse {
    #[serde(flatten)]
    pub group: DirectoryGroupView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateDirectoryGroupRequest {
    /// Display name, 1–64 characters, unique in the org.
    pub name: String,
    /// Free-text purpose, up to 500 characters.
    #[schema(nullable = true)]
    pub description: Option<String>,
    /// The directory's own identifier for this group. Supply it when a sync
    /// owns the row, so the next pass updates rather than duplicates it.
    #[schema(nullable = true)]
    pub external_id: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key. Creates carry
    /// no version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateDirectoryGroupRequest {
    /// New name. Absent keeps the current one.
    #[schema(nullable = true)]
    pub name: Option<String>,
    /// New description. Absent keeps the current one; an explicit `null`
    /// clears it.
    #[schema(nullable = true)]
    pub description: Option<Option<String>>,
    /// New external id. Absent keeps it; an explicit `null` clears it (the row
    /// is then locally owned again).
    #[schema(nullable = true)]
    pub external_id: Option<Option<String>>,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

// --- validation --------------------------------------------------------------

fn validate_name(raw: &str) -> ApiResult<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(ApiError::Validation(format!(
            "group name is required (1..={NAME_MAX} characters)"
        )));
    }
    Ok(name.to_string())
}

fn validate_description(raw: Option<&str>) -> ApiResult<Option<String>> {
    match raw.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(text) if text.chars().count() > DESCRIPTION_MAX => Err(ApiError::Validation(format!(
            "description is limited to {DESCRIPTION_MAX} characters"
        ))),
        Some(text) => Ok(Some(text.to_string())),
    }
}

fn validate_external_id(raw: Option<&str>) -> ApiResult<Option<String>> {
    match raw.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(text) if text.chars().count() > EXTERNAL_ID_MAX => Err(ApiError::Validation(format!(
            "external id is limited to {EXTERNAL_ID_MAX} characters"
        ))),
        Some(text) => Ok(Some(text.to_string())),
    }
}

/// A duplicate name or external id is a typed conflict, not an internal error:
/// both are the caller's to resolve, and the two mean different things.
fn map_conflict(error: core_db::DbError) -> ApiError {
    if let core_db::DbError::Sql(sqlx::Error::Database(db)) = &error {
        match db.constraint() {
            Some("directory_groups_org_id_name_key") => {
                return ApiError::Conflict("a directory group with this name already exists".into())
            }
            Some("idx_directory_groups_external") => {
                return ApiError::Conflict("another group already carries this external id".into())
            }
            _ => {}
        }
    }
    error.into()
}

// --- read helpers ------------------------------------------------------------

async fn read_detail(
    ctx: &mut OrgTx<'_>,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<DirectoryGroupDetailResponse> {
    let group = core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
    let members = core_db::directory_groups::list_members(&mut *ctx.tx, org_id, id).await?;
    let roles = core_db::directory_groups::list_group_roles(&mut *ctx.tx, org_id, id).await?;
    Ok(DirectoryGroupDetailResponse {
        group: DirectoryGroupView::from_row(group),
        members: members
            .into_iter()
            .map(|m| DirectoryGroupMemberView {
                user_id: m.user_id,
                display_name: m.display_name,
                email: m.email,
                is_active: m.is_active,
                added_at: m.added_at,
                added_by: m.added_by,
            })
            .collect(),
        roles: roles
            .into_iter()
            .map(|r| DirectoryGroupRoleView {
                role_id: r.role_id,
                name: r.name,
                builtin: r.builtin,
            })
            .collect(),
    })
}

/// Writes one audit row for a group action.
async fn audit_action(
    ctx: &mut OrgTx<'_>,
    org_id: Uuid,
    actor_id: Uuid,
    action: &str,
    entity_id: Uuid,
    detail: Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(actor_id),
            action: action.into(),
            entity_type: Some(ENTITY.into()),
            entity_id: Some(entity_id.to_string()),
            detail,
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

// --- handlers ----------------------------------------------------------------

/// GET /api/directory-groups — every group in the org (manager+).
#[utoipa::path(operation_id = "directory_groups_list", get, path = "/api/directory-groups", tag = "org-users", security(("bearer" = [])),
    responses((status = 200, body = DirectoryGroupsListResponse),
              (status = 403, description = "requires manage_users or manage_roles")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<DirectoryGroupsListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let groups = core_db::directory_groups::list_groups(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(DirectoryGroupsListResponse {
        groups: groups
            .into_iter()
            .map(DirectoryGroupView::from_row)
            .collect(),
    }))
}

/// POST /api/directory-groups — create a group. The group grants nothing until
/// a role is mapped onto it.
#[utoipa::path(operation_id = "directory_groups_create", post, path = "/api/directory-groups", tag = "org-users", security(("bearer" = [])),
    request_body = CreateDirectoryGroupRequest,
    responses((status = 200, body = DirectoryGroupMutationResponse),
              (status = 409, description = "name or external id already in use, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateDirectoryGroupRequest>,
) -> ApiResult<Json<DirectoryGroupMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let name = validate_name(&req.name)?;
    let description = validate_description(req.description.as_deref())?;
    let external_id = validate_external_id(req.external_id.as_deref())?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
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
        let id = Uuid::now_v7();
        core_db::directory_groups::insert_group(
            &mut ctx.tx,
            &core_db::directory_groups::NewDirectoryGroup {
                id,
                org_id,
                name: &name,
                description: description.as_deref(),
                external_id: external_id.as_deref(),
            },
        )
        .await
        .map_err(map_conflict)?;

        let group = core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        audit_action(
            &mut ctx,
            org_id,
            actor_id,
            "directory_group.created",
            id,
            serde_json::json!({ "name": group.name, "external_id": group.external_id }),
        )
        .await?;
        ctx.emit(
            types::DIRECTORY_GROUP_CREATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DirectoryGroupCreated {
                group_id: id,
                name: group.name.clone(),
                external_id: group.external_id.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;

        let view = DirectoryGroupView::from_row(group);
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
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
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DirectoryGroupMutationResponse {
                group: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/directory-groups/{id} — the group with its members and role
/// mappings (manager+).
#[utoipa::path(operation_id = "directory_groups_detail", get, path = "/api/directory-groups/{id}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupDetailResponse),
              (status = 404, description = "group not visible in scope")))]
pub async fn detail_handler(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DirectoryGroupDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let detail = read_detail(&mut ctx, org_id, id).await?;
        Ok((ctx, detail))
    })
    .await?;
    Ok(Json(outcome))
}

/// PATCH /api/directory-groups/{id} — rename, re-describe, or re-point the
/// external id. Absent fields keep their values.
#[utoipa::path(operation_id = "directory_groups_update", patch, path = "/api/directory-groups/{id}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateDirectoryGroupRequest,
    responses((status = 200, body = DirectoryGroupMutationResponse),
              (status = 404, description = "group not visible in scope"),
              (status = 409, description = "name or external id already in use, version conflict, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDirectoryGroupRequest>,
) -> ApiResult<Json<DirectoryGroupMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let name = req.name.as_deref().map(validate_name).transpose()?;
    let description = match &req.description {
        Some(inner) => Some(validate_description(inner.as_deref())?),
        None => None,
    };
    let external_id = match &req.external_id {
        Some(inner) => Some(validate_external_id(inner.as_deref())?),
        None => None,
    };

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
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
        let before = core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let name = name.unwrap_or_else(|| before.name.clone());
        let description = description.unwrap_or_else(|| before.description.clone());
        let external_id = external_id.unwrap_or_else(|| before.external_id.clone());

        core_db::directory_groups::update_group(
            &mut ctx.tx,
            org_id,
            id,
            &name,
            description.as_deref(),
            external_id.as_deref(),
        )
        .await
        .map_err(map_conflict)?;

        let group = core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        audit_action(
            &mut ctx,
            org_id,
            actor_id,
            "directory_group.updated",
            id,
            serde_json::json!({
                "name": group.name,
                "previous_name": before.name,
                "external_id": group.external_id,
            }),
        )
        .await?;
        ctx.emit(
            types::DIRECTORY_GROUP_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DirectoryGroupUpdated {
                group_id: id,
                name: group.name.clone(),
                previous_name: before.name.clone(),
                external_id: group.external_id.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;

        let view = DirectoryGroupView::from_row(group);
        let before_view = DirectoryGroupView::from_row(before);
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before_view).unwrap_or(Value::Null)),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DirectoryGroupMutationResponse {
                group: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/directory-groups/{id} — remove the group. Its memberships and
/// role mappings go with it, so every member loses exactly what this group
/// granted.
#[utoipa::path(operation_id = "directory_groups_delete", delete, path = "/api/directory-groups/{id}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupMutationResponse),
              (status = 404, description = "group not visible in scope")))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DirectoryGroupMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let before = core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        core_db::directory_groups::delete_group(&mut ctx.tx, org_id, id).await?;

        audit_action(
            &mut ctx,
            org_id,
            actor_id,
            "directory_group.deleted",
            id,
            serde_json::json!({
                "name": before.name,
                "external_id": before.external_id,
                "member_count": before.member_count,
                "role_count": before.role_count,
            }),
        )
        .await?;
        ctx.emit(
            types::DIRECTORY_GROUP_DELETED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DirectoryGroupDeleted {
                group_id: id,
                name: before.name.clone(),
                external_id: before.external_id.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;

        let view = DirectoryGroupView::from_row(before);
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::to_value(&view).unwrap_or(Value::Null)),
                after: Value::Null,
                updated_at: chrono::Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DirectoryGroupMutationResponse {
                group: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PUT /api/directory-groups/{id}/members/{userId} — put a user in the group.
/// Idempotent; a repeated call reports the same membership and emits nothing.
#[utoipa::path(operation_id = "directory_groups_add_member", put, path = "/api/directory-groups/{id}/members/{userId}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("userId" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupDetailResponse),
              (status = 404, description = "group or user not visible in scope")))]
pub async fn add_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DirectoryGroupDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        // Both sides must exist in THIS org; each lookup is org-pinned, so a
        // foreign id is a clean 404 rather than a silent no-op insert.
        core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let member = core_db::users::get_user(&mut *ctx.tx, org_id, user_id)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let already =
            core_db::directory_groups::is_member(&mut *ctx.tx, org_id, id, user_id).await?;
        if !already {
            core_db::directory_groups::add_member(&mut ctx.tx, org_id, id, user_id, Some(actor_id))
                .await?;
            audit_action(
                &mut ctx,
                org_id,
                actor_id,
                "directory_group.member_added",
                id,
                serde_json::json!({ "user_id": user_id, "email": member.email }),
            )
            .await?;
            ctx.emit(
                types::DIRECTORY_GROUP_MEMBER_ADDED,
                ENTITY,
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::DirectoryGroupMemberAdded {
                    group_id: id,
                    user_id,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        let detail = read_detail(&mut ctx, org_id, id).await?;
        Ok((ctx, detail))
    })
    .await?;

    Ok(Json(outcome))
}

/// DELETE /api/directory-groups/{id}/members/{userId} — take a user out of the
/// group. The roles that group carried for them are gone; every other grant
/// survives.
#[utoipa::path(operation_id = "directory_groups_remove_member", delete, path = "/api/directory-groups/{id}/members/{userId}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("userId" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupDetailResponse),
              (status = 404, description = "group or membership not visible in scope")))]
pub async fn remove_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DirectoryGroupDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let present =
            core_db::directory_groups::is_member(&mut *ctx.tx, org_id, id, user_id).await?;
        if !present {
            return Err(ApiError::NotFound);
        }
        core_db::directory_groups::remove_member(&mut ctx.tx, org_id, id, user_id).await?;
        audit_action(
            &mut ctx,
            org_id,
            actor_id,
            "directory_group.member_removed",
            id,
            serde_json::json!({ "user_id": user_id }),
        )
        .await?;
        ctx.emit(
            types::DIRECTORY_GROUP_MEMBER_REMOVED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DirectoryGroupMemberRemoved {
                group_id: id,
                user_id,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let detail = read_detail(&mut ctx, org_id, id).await?;
        Ok((ctx, detail))
    })
    .await?;

    Ok(Json(outcome))
}

/// PUT /api/directory-groups/{id}/roles/{roleId} — map a role onto the group.
/// Every current and future member holds it from now on (additive union with
/// their directly assigned roles). Idempotent.
#[utoipa::path(operation_id = "directory_groups_map_role", put, path = "/api/directory-groups/{id}/roles/{roleId}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("roleId" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupDetailResponse),
              (status = 404, description = "group or role not visible in scope")))]
pub async fn map_role(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, role_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DirectoryGroupDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let role = core_db::roles::get_role(&mut *ctx.tx, org_id, role_id)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let already =
            core_db::directory_groups::has_role(&mut *ctx.tx, org_id, id, role_id).await?;
        if !already {
            core_db::directory_groups::add_role(&mut ctx.tx, org_id, id, role_id, Some(actor_id))
                .await?;
            audit_action(
                &mut ctx,
                org_id,
                actor_id,
                "directory_group.role_mapped",
                id,
                serde_json::json!({ "role_id": role_id, "role": role.name }),
            )
            .await?;
            ctx.emit(
                types::DIRECTORY_GROUP_ROLE_MAPPED,
                ENTITY,
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::DirectoryGroupRoleMapped {
                    group_id: id,
                    role_id,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        let detail = read_detail(&mut ctx, org_id, id).await?;
        Ok((ctx, detail))
    })
    .await?;

    Ok(Json(outcome))
}

/// DELETE /api/directory-groups/{id}/roles/{roleId} — drop a role from the
/// group. Members lose exactly that role where the group was its only source.
#[utoipa::path(operation_id = "directory_groups_unmap_role", delete, path = "/api/directory-groups/{id}/roles/{roleId}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("roleId" = Uuid, Path)),
    responses((status = 200, body = DirectoryGroupDetailResponse),
              (status = 404, description = "group or mapping not visible in scope")))]
pub async fn unmap_role(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, role_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DirectoryGroupDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let present =
            core_db::directory_groups::has_role(&mut *ctx.tx, org_id, id, role_id).await?;
        if !present {
            return Err(ApiError::NotFound);
        }
        core_db::directory_groups::remove_role(&mut ctx.tx, org_id, id, role_id).await?;
        audit_action(
            &mut ctx,
            org_id,
            actor_id,
            "directory_group.role_unmapped",
            id,
            serde_json::json!({ "role_id": role_id }),
        )
        .await?;
        ctx.emit(
            types::DIRECTORY_GROUP_ROLE_UNMAPPED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DirectoryGroupRoleUnmapped {
                group_id: id,
                role_id,
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        let detail = read_detail(&mut ctx, org_id, id).await?;
        Ok((ctx, detail))
    })
    .await?;

    Ok(Json(outcome))
}
