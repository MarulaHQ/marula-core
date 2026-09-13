//! Org user management (Wave 11 lane 2): staff directory, invite /
//! create, activate / deactivate, org-role assignment.
//!
//! Permissions: Owner/Admin manage (`special:manage_users`); the
//! directory listing is manager+ (any of `manage_users` / `manage_boards`
//! — dispatchers included, technicians and read-only not).
//!
//! Mutations ride the IF1–IF3 instant-first envelope (idempotency-keyed
//! claim, version-gated seq, read-your-write `mutation` metadata) and emit
//! their domain events + audit rows in the same commit. Invite redemption
//! (`POST /org/users/accept-invite`) is public: the invitee has no session
//! yet, so the one-time token IS the credential; it is stored hashed,
//! single-use, and expiring, resolved through the 0047 SECURITY DEFINER
//! bridge on the raw pool.

use crate::auth::handlers::{issue_tokens, validate_email, validate_password};
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::org_users::OrgUserEntry;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, BuiltinRole, Permission};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

fn manage_users() -> Permission {
    Permission::ManageUsers
}

/// Manager+ for the read-only directory (dispatchers included).
fn require_manager(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(
        &principal.permissions,
        &[Permission::ManageUsers, Permission::ManageBoards],
    )
    .expect()
    .map_err(ApiError::Forbidden)
}

fn parse_role(s: &str) -> ApiResult<BuiltinRole> {
    BuiltinRole::parse_preset(s).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown org role '{s}': expected one of owner, admin, dispatcher, \
             senior_tech, technician, bookkeeper, auditor, read_only"
        ))
    })
}

/// Directory view of one staff user (route-level DTO; `org_role` is the
/// user's highest-precedence builtin role, `null` for custom-only).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OrgUser {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub email: String,
    pub display_name: String,
    #[schema(nullable = true)]
    pub org_role: Option<String>,
    pub is_active: bool,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<OrgUserEntry> for OrgUser {
    fn from(e: OrgUserEntry) -> Self {
        Self {
            id: e.id,
            org_id: e.org_id,
            email: e.email,
            display_name: e.display_name,
            org_role: e.org_role,
            is_active: e.is_active,
            created_at: e.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct ListOrgUsersQuery {
    /// Page size (default 50, max 200).
    pub limit: Option<i64>,
    /// Offset paging.
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OrgUserListResponse {
    pub users: Vec<OrgUser>,
    /// Total directory size (for offset paging).
    pub total: i64,
}

/// GET /api/org/users — the staff directory (manager+).
#[utoipa::path(operation_id = "org_users_list", get, path = "/api/org/users", tag = "org-users", security(("bearer" = [])),
    params(ListOrgUsersQuery),
    responses((status = 200, body = OrgUserListResponse),
              (status = 403, description = "requires manager role")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListOrgUsersQuery>,
) -> ApiResult<Json<OrgUserListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_manager(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let users = core_db::org_users::list_directory(&mut *tx, org_id, limit, offset)
        .await?
        .into_iter()
        .map(OrgUser::from)
        .collect();
    let total = core_db::org_users::count_directory(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(OrgUserListResponse { users, total }))
}

/// The one-time setup token an admin delivers to the invitee (shown once;
/// only its sha-256 is stored).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InviteInfo {
    pub setup_token: String,
    #[schema(value_type = String)]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InviteUserRequest {
    pub email: String,
    pub display_name: String,
    /// Builtin org role preset: owner, admin, dispatcher, senior_tech, technician, bookkeeper, auditor, read_only.
    pub role: String,
    #[serde(default)]
    pub timezone: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the invited (still inactive)
/// user at the top level plus the one-time setup token and the additive
/// `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OrgUserMutationResponse {
    #[serde(flatten)]
    pub user: OrgUser,
    /// Present on invite only: the setup token for the invitee.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub invite: Option<InviteInfo>,
    pub mutation: instant::MutationMeta,
}

const INVITE_TTL_SECS: i64 = 7 * 24 * 3600;

pub(crate) fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn new_setup_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// POST /api/org/users — invite (create) a staff user. The row lands
/// INACTIVE with an unusable password; the response carries a one-time
/// setup token the invitee redeems at `POST /org/users/accept-invite`.
#[utoipa::path(operation_id = "org_users_invite", post, path = "/api/org/users", tag = "org-users", security(("bearer" = [])),
    request_body = InviteUserRequest,
    responses((status = 200, body = OrgUserMutationResponse),
              (status = 409, description = "email already in org, or idempotency key reused")))]
pub async fn invite(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<InviteUserRequest>,
) -> ApiResult<Json<OrgUserMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_users())?;
    validate_email(&req.email)?;
    let display_name = req.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(ApiError::Validation("display_name is required".into()));
    }
    let role = parse_role(&req.role)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "org_user",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if core_db::org_users::find_any_by_email(&mut *ctx.tx, org_id, &req.email)
            .await?
            .is_some()
        {
            return Err(ApiError::Conflict(format!(
                "a user with email {} already exists in this org",
                req.email
            )));
        }
        let entry = core_db::org_users::create_invited_user(
            &mut *ctx.tx,
            Uuid::now_v7(),
            org_id,
            req.email.trim(),
            &display_name,
        )
        .await?;
        let role_id =
            core_db::org_users::find_builtin_role_id(&mut *ctx.tx, org_id, role.as_str()).await?;
        core_db::users::assign_role(&mut *ctx.tx, entry.id, role_id).await?;

        let setup_token = new_setup_token();
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(INVITE_TTL_SECS);
        core_db::org_users::insert_invite(
            &mut *ctx.tx,
            Uuid::now_v7(),
            org_id,
            entry.id,
            &hash_token(&setup_token),
            user_id,
            expires_at,
        )
        .await?;

        ctx.emit(
            types::USER_INVITED,
            "user",
            Some(entry.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::UserInvited {
                user_id: entry.id,
                email: entry.email.clone(),
                display_name: entry.display_name.clone(),
                role: role.as_str().to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "org_user.invited".into(),
                entity_type: Some("user".into()),
                entity_id: Some(entry.id.to_string()),
                detail: serde_json::json!({ "email": entry.email, "role": role.as_str() }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "org_user",
                entity_id: entry.id,
                version: None,
            },
        )
        .await?;
        let view: OrgUser = entry.clone().into();
        // The stored replay payload must include the setup token: the
        // client must receive the SAME invite response on every retry.
        let stored = OrgUserMutationResponse {
            user: view.clone(),
            invite: Some(InviteInfo {
                setup_token,
                expires_at,
            }),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: entry.created_at,
                before: None,
                after: Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "org_user",
                entity_id: entry.id,
                op: "invite",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: entry.created_at,
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        let response = OrgUserMutationResponse {
            user: view,
            invite: stored.invite,
            mutation,
        };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetUserStatusRequest {
    /// `false` deactivates: sessions revoked, login blocked. Never deletes.
    pub active: bool,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetUserRoleRequest {
    /// Builtin org role preset: owner, admin, dispatcher, senior_tech, technician, bookkeeper, auditor, read_only.
    pub role: String,
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

async fn load_entry(
    ctx: &mut org_tx::OrgTx<'_>,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<OrgUserEntry> {
    core_db::org_users::get_entry(&mut *ctx.tx, org_id, id)
        .await
        .map_err(ApiError::from)
        .map_err(instant::mutation_scope_err)
}

/// PATCH /api/org/users/{id}/status — activate or deactivate. Deactivation
/// revokes every live session and blocks login in the same commit.
#[utoipa::path(operation_id = "org_users_set_status", patch, path = "/api/org/users/{id}/status", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SetUserStatusRequest,
    responses((status = 200, body = OrgUserMutationResponse),
              (status = 409, description = "last active owner, version conflict, or idempotency key reused")))]
pub async fn set_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SetUserStatusRequest>,
) -> ApiResult<Json<OrgUserMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_users())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "org_user",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = load_entry(&mut ctx, org_id, id).await?;
        if before.id == actor_id && !req.active {
            return Err(ApiError::Validation(
                "you cannot deactivate your own account".into(),
            ));
        }
        // Last-active-owner guard: deactivating (or already-deactivated
        // owner shenanigans) must never orphan the org without an owner.
        if !req.active && before.org_role.as_deref() == Some(BuiltinRole::Owner.as_str()) {
            let owners = core_db::org_users::count_active_with_builtin_role(
                &mut *ctx.tx,
                org_id,
                BuiltinRole::Owner.as_str(),
            )
            .await?;
            if owners <= 1 {
                return Err(ApiError::Conflict(
                    "cannot deactivate the last active owner".into(),
                ));
            }
        }

        let after = core_db::org_users::set_active(&mut *ctx.tx, org_id, id, req.active)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        if !req.active {
            core_db::org_users::revoke_all_sessions(&mut *ctx.tx, id).await?;
            ctx.emit(
                types::USER_DEACTIVATED,
                "user",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::UserDeactivated { user_id: id })
                    .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        } else {
            ctx.emit(
                types::USER_ACTIVATED,
                "user",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::UserActivated { user_id: id })
                    .unwrap_or_else(|_| serde_json::json!({})),
            )
            .await?;
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: if req.active {
                    "org_user.activated".into()
                } else {
                    "org_user.deactivated".into()
                },
                entity_type: Some("user".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "email": after.email }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "org_user",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view: OrgUser = after.clone().into();
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "org_user",
                entity_id: id,
                op: "set_status",
                seq: write_seq,
                before: serde_json::to_value(OrgUser::from(before)).ok(),
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
            MutationOutcome::Applied(OrgUserMutationResponse {
                user: view,
                invite: None,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/org/users/{id}/role — assign a builtin org role. Each change
/// is audited; the last active owner cannot be demoted away from owner.
#[utoipa::path(operation_id = "org_users_set_role", patch, path = "/api/org/users/{id}/role", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SetUserRoleRequest,
    responses((status = 200, body = OrgUserMutationResponse),
              (status = 409, description = "last active owner, version conflict, or idempotency key reused")))]
pub async fn set_role(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SetUserRoleRequest>,
) -> ApiResult<Json<OrgUserMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_users())?;
    let role = parse_role(&req.role)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "org_user",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = load_entry(&mut ctx, org_id, id).await?;
        let role_id =
            core_db::org_users::find_builtin_role_id(&mut *ctx.tx, org_id, role.as_str()).await?;
        if before.org_role.as_deref() == Some(BuiltinRole::Owner.as_str())
            && role != BuiltinRole::Owner
        {
            let owners = core_db::org_users::count_active_with_builtin_role(
                &mut *ctx.tx,
                org_id,
                BuiltinRole::Owner.as_str(),
            )
            .await?;
            if owners <= 1 {
                return Err(ApiError::Conflict(
                    "cannot demote the last active owner".into(),
                ));
            }
        }

        let previous = core_db::org_users::set_builtin_role(&mut ctx.tx, org_id, id, role_id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        let after = core_db::org_users::get_entry(&mut *ctx.tx, org_id, id).await?;

        ctx.emit(
            types::USER_ROLE_CHANGED,
            "user",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::UserRoleChanged {
                user_id: id,
                from_role: previous.clone(),
                to_role: role.as_str().to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "org_user.role_changed".into(),
                entity_type: Some("user".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "email": after.email,
                    "from_role": previous,
                    "to_role": role.as_str(),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "org_user",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let view: OrgUser = after.clone().into();
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "org_user",
                entity_id: id,
                op: "set_role",
                seq: write_seq,
                before: serde_json::to_value(OrgUser::from(before)).ok(),
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
            MutationOutcome::Applied(OrgUserMutationResponse {
                user: view,
                invite: None,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcceptInviteRequest {
    /// The one-time setup token from the invite response.
    pub token: String,
    /// First password (argon2-hashed server-side).
    pub password: String,
    #[serde(default)]
    pub timezone: Option<String>,
}

/// POST /api/org/users/accept-invite — public invite redemption: set the
/// first password and activate the account; returns a normal token pair.
/// The token is single-use and expiring; an unknown, used, or expired
/// token is an indistinguishable 404.
#[utoipa::path(operation_id = "org_users_accept_invite", post, path = "/api/org/users/accept-invite", tag = "org-users",
    request_body = AcceptInviteRequest,
    responses((status = 200, body = TokenPair),
              (status = 404, description = "invite unknown, already used, or expired"),
              (status = 422, description = "validation error")))]
pub async fn accept_invite(
    State(state): State<AppState>,
    Json(req): Json<AcceptInviteRequest>,
) -> ApiResult<Json<TokenPair>> {
    let token = req.token.trim();
    if token.is_empty() {
        return Err(ApiError::Validation("token is required".into()));
    }
    validate_password(&req.password)?;
    let token_hash = hash_token(token);

    // Raw pool: no session exists yet, so the 0047 SECURITY DEFINER
    // bridge resolves the invite's org + user. Returns only ids.
    let (org_id, user_id) = core_db::org_users::resolve_invite_by_token(&state.pool, &token_hash)
        .await
        .map_err(ApiError::from)?;

    let password_hash = crate::auth::password::hash(&req.password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("password hashing failed: {e}")))?;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Re-verify single-use within the scoped tx (race-proof: the
        // UPDATE is the claim).
        let claimed_user = core_db::org_users::take_invite(&mut *ctx.tx, org_id, &token_hash)
            .await
            .map_err(ApiError::from)?;
        if claimed_user != user_id {
            return Err(ApiError::NotFound);
        }
        core_db::org_users::complete_invite(&mut *ctx.tx, org_id, user_id, &password_hash)
            .await
            .map_err(ApiError::from)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "org_user.accept_invite".into(),
                entity_type: Some("user".into()),
                entity_id: Some(user_id.to_string()),
                detail: serde_json::json!({}),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    let tokens = issue_tokens(&state, user_id, org_id, None).await?;
    Ok(Json(tokens))
}
