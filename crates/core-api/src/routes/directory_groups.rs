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
//! is the same roster read a different way) or the machine read scope
//! `directory_groups:read`. Every write requires `special:manage_roles` or the
//! machine write scope `directory_groups:write`: adding a person to a group or
//! a role to a group is a grant, and a grant is a roles-admin act — the scope
//! is the smallest grant that lets an external identity service perform it,
//! and it reaches nothing else (`/api/roles` still demands the special).
//!
//! Ceiling (Wave 55, `p55-directory-grant-ceiling`): a grant may confer only
//! authority the caller already holds. Mapping a role onto a group is checked
//! against the caller's own resolved permission set, and adding a person to a
//! group is checked against every role that group currently carries, because
//! the membership is what turns those mappings into that person's grants. A
//! role the caller could not grant directly is a typed 403 naming it, and no
//! row is written. Without this the write scope was an escalation path: a
//! machine credential holding only `directory_groups:write` could map the
//! seeded Admin preset onto a group, add any user, and hand that user the
//! admin matrix through the ordinary union at their next sign-in.
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

use crate::auth::middleware::authed;
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
use core_domain::policy::{check, check_any, Permission};
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

/// The grants that may read the group surface.
///
/// `special:manage_users` and `special:manage_roles` are the human grants (a
/// manager reads the roster this way, and the org-users surface reads it too).
/// `directory_groups:read` and `directory_groups:write` are the machine grants
/// (Wave 52 lane R16): the identity pack's SCIM connector and directory-admin
/// plugin run outside core, and no plugin may hold an admin special. The write
/// grant is here as well because a writer that cannot read back what it wrote
/// would be a grant nobody can use.
const GROUP_READERS: [Permission; 4] = [
    Permission::ManageUsers,
    Permission::ManageRoles,
    Permission::ViewDirectoryGroups,
    Permission::ManageDirectoryGroups,
];

/// The grants that may write the group surface.
///
/// `special:manage_roles` is the human grant and stays exactly as it was.
/// `directory_groups:write` is the machine grant: it opens group, membership
/// and role-mapping writes and nothing else — role CRUD stays behind the
/// special, which is what keeps a connector a mirror rather than a roles
/// administrator. Neither grant confers authority on its own: every write that
/// would grant a role also passes the ceiling below.
const GROUP_WRITERS: [Permission; 2] = [Permission::ManageRoles, Permission::ManageDirectoryGroups];

/// Manager+ read (the org-users surface reads this for its pickers too).
fn require_reader(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(&principal.permissions, &GROUP_READERS)
        .expect()
        .map_err(ApiError::Forbidden)
}

/// Roles-admin write, or the directory write scope.
fn require_writer(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(&principal.permissions, &GROUP_WRITERS)
        .expect()
        .map_err(ApiError::Forbidden)
}

// --- the grant ceiling -------------------------------------------------------

/// The resolved permission set a role row carries.
///
/// The row is the authority, not the preset's canonical definition: this is
/// the same JSON `core_db::users::role_from_row` parses when a principal is
/// built, so the set compared here is the set a member will actually hold. A
/// row that does not decode fails closed — treating it as a role that grants
/// nothing is precisely the bug the roles surface fixed in Wave 54 (a stored
/// role that could not be read defaulted to empty), and here that default
/// would also decide an authorization question.
fn role_permissions(role: &core_db::roles::RoleEntry) -> ApiResult<Vec<Permission>> {
    serde_json::from_value::<Vec<Permission>>(role.permissions.clone()).map_err(|error| {
        ApiError::Internal(anyhow::anyhow!(
            "role {} ({}) carries permissions that do not decode: {error}",
            role.id,
            role.name
        ))
    })
}

/// The first grant the caller does not hold, in the order the role lists them.
///
/// One comparison, not a second authority model: `core_domain::policy::check`
/// is the same primitive `service_accounts::require_grantable` uses for the
/// API-key grant ceiling, and `check_any` above uses it for the gates. A set is
/// inside another set exactly when every member passes `check`.
fn first_unheld(caller: &[Permission], conferring: &[Permission]) -> Option<Permission> {
    conferring
        .iter()
        .copied()
        .find(|perm| !check(caller, *perm).is_allowed())
}

/// Refuse a role that carries authority the caller does not hold.
///
/// The refusal names the role and the first grant it carries beyond the
/// caller's reach, so an operator reading a connector's log knows which mapping
/// to drop rather than which request failed. `Forbidden` is the typed 403 this
/// surface already uses (reason code `forbidden` in the error envelope).
fn refuse_unheld_role(
    caller: &[Permission],
    role_name: &str,
    conferring: &[Permission],
) -> ApiResult<()> {
    match first_unheld(caller, conferring) {
        None => Ok(()),
        Some(missing) => Err(ApiError::Forbidden(format!(
            "cannot grant role {role_name:?}: it carries {}, which the caller does not hold",
            missing.key()
        ))),
    }
}

/// Every role currently mapped onto a group, resolved.
///
/// Two org-pinned reads rather than one per mapping: this runs on the member
/// add path, and a group carrying several roles must not turn one membership
/// write into a query per role. The role directory is small (eight presets plus
/// whatever the org forked), and `list_roles` is the same read the roles
/// surface performs.
async fn roles_conferred_by_group(
    ctx: &mut OrgTx<'_>,
    org_id: Uuid,
    group_id: Uuid,
) -> ApiResult<Vec<(String, Vec<Permission>)>> {
    let mapped =
        core_db::directory_groups::list_group_roles(&mut *ctx.tx, org_id, group_id).await?;
    if mapped.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: std::collections::HashSet<Uuid> = mapped.iter().map(|r| r.role_id).collect();
    let mut out = Vec::with_capacity(mapped.len());
    for role in core_db::roles::list_roles(&mut *ctx.tx, org_id).await? {
        if wanted.contains(&role.id) {
            out.push((role.name.clone(), role_permissions(&role)?));
        }
    }
    Ok(out)
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
              (status = 403, description = "requires manage_users, manage_roles, directory_groups:read or directory_groups:write")))]
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
    require_writer(&authed.principal)?;
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
    require_writer(&authed.principal)?;
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
    require_writer(&authed.principal)?;
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
///
/// The ceiling applies to the insert and not to the no-op: adding a person is
/// what turns the roles this group already carries into that person's grants,
/// so every mapped role must be inside the caller's own set. A repeat of a
/// membership that already exists confers nothing new, which is why it stays a
/// 200 — refusing it would break the documented idempotency a sync depends on
/// while closing no hole.
///
/// The 403 the ceiling raises is not added to the `utoipa` response list here:
/// the OpenAPI pins are a reviewed artifact, and this lane changes behaviour
/// rather than the documented contract. The next sanctioned re-pin should add
/// it to this operation and to `map_role`.
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
    require_writer(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let caller_permissions = authed.principal.permissions.clone();

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
            // The ceiling: this membership would confer every role the group
            // carries, so the caller must already hold every one of them.
            for (role_name, conferring) in roles_conferred_by_group(&mut ctx, org_id, id).await? {
                refuse_unheld_role(&caller_permissions, &role_name, &conferring)?;
            }
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
    require_writer(&authed.principal)?;
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
///
/// The ceiling applies to the mapping write itself: the role's resolved
/// permission set must be inside the caller's own, or the mapping is a typed
/// 403 and no row is written. A no-op repeat of an existing mapping confers
/// nothing new, so it stays a 200 for the same reason a repeated member add
/// does.
///
/// The 403 stays out of the `utoipa` response list for the reason given on
/// `add_member`: the pins are a reviewed artifact and this lane does not
/// re-pin them.
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
    require_writer(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let caller_permissions = authed.principal.permissions.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        core_db::directory_groups::get_group(&mut *ctx.tx, org_id, id).await?;
        let role = core_db::roles::get_role(&mut *ctx.tx, org_id, role_id)
            .await
            .map_err(|_| ApiError::NotFound)?;

        let already =
            core_db::directory_groups::has_role(&mut *ctx.tx, org_id, id, role_id).await?;
        if !already {
            let conferring = role_permissions(&role)?;
            refuse_unheld_role(&caller_permissions, &role.name, &conferring)?;
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
    require_writer(&authed.principal)?;
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

#[cfg(test)]
mod tests {
    use super::{first_unheld, refuse_unheld_role, GROUP_READERS, GROUP_WRITERS};
    use crate::error::ApiError;
    use crate::routes::service_accounts::permission_from_key;
    use core_domain::policy::{check_any, Action, BuiltinRole, Module, Permission, PolicyDecision};

    /// The permissions a service account's grant keys resolve to. The API-key
    /// path in `auth::middleware` keeps its own parser, so the two new keys are
    /// spelled identically in both: a scope means one thing whichever machine
    /// credential presents it, and
    /// `the_directory_scopes_resolve_onto_their_own_pair_and_nothing_wider`
    /// pins the other half.
    fn scopes(keys: &[&str]) -> Vec<Permission> {
        keys.iter()
            .map(|k| permission_from_key(k).unwrap_or_else(|| panic!("unknown scope {k}")))
            .collect()
    }

    fn allowed(granted: &[Permission], required: &[Permission]) -> bool {
        check_any(granted, required).is_allowed()
    }

    /// Deny-by-default, in the direction that matters: a principal holding
    /// nothing, or holding a read that belongs to some other module, is
    /// refused on both gates. A gate that admitted the pack and also admitted
    /// everything else would pass the positive tests in this module and fail
    /// this one.
    #[test]
    fn the_group_surface_refuses_a_principal_without_one_of_its_grants() {
        assert!(matches!(
            check_any(&[], &GROUP_READERS),
            PolicyDecision::Denied { .. }
        ));
        assert!(matches!(
            check_any(&[], &GROUP_WRITERS),
            PolicyDecision::Denied { .. }
        ));
        // A read of another module is not a directory read.
        let clients_read = [Permission::Matrix {
            module: Module::Clients,
            action: core_domain::policy::Action::View,
        }];
        assert!(!allowed(&clients_read, &GROUP_READERS));
        assert!(!allowed(&clients_read, &GROUP_WRITERS));
        // Nor is a scope from another surface that happens to be a machine
        // grant. A key carrying observability, usage and a write elsewhere is
        // still refused here.
        let unrelated = scopes(&[
            "observability:read",
            "org_usage:read",
            "tickets:edit",
            "clients:edit",
        ]);
        assert!(!allowed(&unrelated, &GROUP_READERS));
        assert!(!allowed(&unrelated, &GROUP_WRITERS));
    }

    /// The same refusal stated in a pack's own vocabulary: take every
    /// capability the closed vocabulary offers except the two directory ones,
    /// resolve all of them, and neither gate opens. A single capability that
    /// drifted onto an admin special would break this, which is why the union
    /// is used rather than a hand-picked few.
    #[test]
    fn a_plugin_granted_every_other_capability_still_cannot_touch_the_surface() {
        use core_domain::entities::PluginCapability;

        let granted: Vec<Permission> = PluginCapability::ALL
            .iter()
            .filter(|c| {
                !matches!(
                    c,
                    PluginCapability::DirectoryGroupsRead | PluginCapability::DirectoryGroupsWrite
                )
            })
            .map(|c| c.permission())
            .collect();
        assert!(
            granted.len() >= 30,
            "only {} capabilities resolved — the filter is wrong and this test \
             proves nothing",
            granted.len()
        );
        assert!(
            !allowed(&granted, &GROUP_READERS),
            "a plugin without directory_groups.read must not list groups"
        );
        assert!(
            !allowed(&granted, &GROUP_WRITERS),
            "a plugin without directory_groups.write must not change a group"
        );
    }

    /// The positive case, stated as the connector's job: three operations, two
    /// reads and one write, all reachable with the two scopes and nothing else.
    #[test]
    fn the_identity_pack_scopes_cover_the_connectors_three_operations() {
        let connector = scopes(&["directory_groups:read", "directory_groups:write"]);

        // list groups -> GET /api/directory-groups
        assert!(allowed(&connector, &GROUP_READERS));
        // list members -> GET /api/directory-groups/{id} (members ride in the
        // detail payload, so this is the same read gate)
        assert!(allowed(&connector, &GROUP_READERS));
        // map a role -> PUT /api/directory-groups/{id}/roles/{roleId}
        assert!(allowed(&connector, &GROUP_WRITERS));
    }

    /// Read alone is enough for a groups-only mirror and not for a writer: the
    /// read scope opens no write, so the second scope is a real decision an org
    /// makes rather than a formality.
    #[test]
    fn the_read_scope_opens_no_write() {
        let reader = scopes(&["directory_groups:read"]);
        assert!(allowed(&reader, &GROUP_READERS));
        assert!(!allowed(&reader, &GROUP_WRITERS));
    }

    /// The human admin is unchanged, and the special still loses nothing. Both
    /// halves are asserted because a change that kept the admin working by
    /// admitting everyone would pass the first.
    #[test]
    fn an_admin_special_still_opens_the_surface_and_only_the_special_writes() {
        // special:manage_users has always read the roster and never written it.
        let manager = [Permission::ManageUsers];
        assert!(allowed(&manager, &GROUP_READERS));
        assert!(!allowed(&manager, &GROUP_WRITERS));
        // special:manage_roles does both, exactly as before this lane.
        let roles_admin = [Permission::ManageRoles];
        assert!(allowed(&roles_admin, &GROUP_READERS));
        assert!(allowed(&roles_admin, &GROUP_WRITERS));
    }

    /// The write grant reaches this module and nowhere else. This is the
    /// tenancy half of the argument at the gate level: the group routes pin
    /// every query to the caller's org (`set_org_and_user` plus the org-pinned
    /// `core_db::directory_groups` reads, which
    /// `crates/core-db/tests/directory_groups.rs` pins across an org boundary),
    /// so a new grant is safe only while it opens no route with a weaker scope.
    /// A grant that leaked into the users or roles surface would be a way
    /// around tenancy, and this test is what notices.
    #[test]
    fn no_other_route_accepts_the_directory_grants() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
        let mut scanned = 0usize;
        for entry in std::fs::read_dir(&dir).expect("read the routes directory") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("directory_groups.rs") {
                continue;
            }
            // `service_accounts.rs` owns the grant-KEY parser, not a request
            // gate: it is where a service account's scope strings become
            // permissions, so it has to name every grant in the vocabulary.
            // Skipping it here does not weaken the check — a gate that accepted
            // the grant would appear in a handler module, and these are all of
            // them.
            if path.file_name().and_then(|n| n.to_str()) == Some("service_accounts.rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read a route file");
            scanned += 1;
            for name in ["ViewDirectoryGroups", "ManageDirectoryGroups"] {
                assert!(
                    !src.contains(name),
                    "{} accepts {name}: the directory grant must reach the group \
                     routes and no other surface, or it becomes a way around their \
                     org scope",
                    path.display()
                );
            }
        }
        assert!(
            scanned > 20,
            "only {scanned} route files were scanned — the directory moved and this \
             test is now checking nothing"
        );
    }

    // --- the grant ceiling ---------------------------------------------------

    /// The finding, stated as a policy fact: the directory write scope is not
    /// authority to confer any shipped preset. Every preset carries at least a
    /// view matrix, and the directory grants are the only thing a connector
    /// holds, so none of the eight is inside its set. A preset that gained a
    /// directory grant would fail this assertion, and the positive case below
    /// is what would then need the decision.
    ///
    /// `policy::directory_group_grants_are_machine_grants_that_no_preset_carries`
    /// already pins the input to this test; this one pins the consequence
    /// through the ceiling function.
    #[test]
    fn a_directory_writer_holds_no_preset_inside_its_own_set() {
        let writer = scopes(&["directory_groups:write"]);
        for preset in BuiltinRole::PRESETS {
            assert!(
                first_unheld(&writer, &preset.permissions()).is_some(),
                "{} must not be grantable from directory_groups:write alone",
                preset.as_str()
            );
        }
    }

    /// The refusal is typed and names the role. The case the audit record
    /// reproduced is the Admin preset, so it is asserted by name and by the
    /// first grant it carries beyond the caller.
    #[test]
    fn refusing_a_wider_role_names_the_role_and_the_grant() {
        let writer = scopes(&["directory_groups:write"]);
        let error = refuse_unheld_role(&writer, "Admin", &BuiltinRole::Admin.permissions())
            .expect_err("Admin is wider than a directory write");
        let ApiError::Forbidden(message) = error else {
            panic!("the ceiling refuses with the typed 403, not another variant");
        };
        assert!(message.contains("Admin"), "the role is named: {message}");
        assert!(
            message.contains("does not hold"),
            "the refusal states the direction of the comparison: {message}"
        );
        assert!(
            message.contains("tickets:view"),
            "the first grant beyond the caller is named: {message}"
        );
    }

    /// One held grant is not a held role: the comparison is containment of the
    /// whole set, never "any of". A caller holding the first cell of the Admin
    /// matrix still cannot map it, and the refusal moves to the next cell.
    #[test]
    fn holding_one_grant_of_a_role_does_not_admit_it() {
        let caller = [Permission::Matrix {
            module: Module::Tickets,
            action: Action::View,
        }];
        assert_eq!(
            first_unheld(&caller, &BuiltinRole::Admin.permissions()),
            Some(Permission::Matrix {
                module: Module::Tickets,
                action: Action::Create,
            })
        );
    }

    /// The ceiling admits what the caller already holds, in both directions:
    /// the role it holds itself, a narrower preset, and (the case the ruling
    /// names) an owner mapping Admin, because the owner's resolved set is the
    /// Admin set. The second half matters — a ceiling that refused everyone
    /// would pass the refusals above.
    #[test]
    fn the_ceiling_admits_roles_inside_the_callers_own_set() {
        let admin = BuiltinRole::Admin.permissions();
        assert!(refuse_unheld_role(&admin, "Admin", &admin).is_ok());
        assert!(
            refuse_unheld_role(&admin, "Technician", &BuiltinRole::Technician.permissions())
                .is_ok()
        );
        let owner = BuiltinRole::Owner.permissions();
        assert!(refuse_unheld_role(&owner, "Admin", &BuiltinRole::Admin.permissions()).is_ok());
        // A role that grants nothing is inside every set, including a writer's.
        let writer = scopes(&["directory_groups:write"]);
        assert!(refuse_unheld_role(&writer, "Empty", &[]).is_ok());
    }
}
