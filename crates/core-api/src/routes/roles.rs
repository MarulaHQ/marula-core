//! Role directory + preset lifecycle (Wave 13 lane 6 — RB1, ADR 0004).
//!
//! Purpose: the shipped preset roles (owner, admin, dispatcher,
//! senior_tech, technician, bookkeeper, auditor, read_only + the portal
//! tiers) exist as DATA on the same permission matrix as custom roles.
//! This surface lists the org's roles and the canonical preset roster,
//! forks a preset into a custom copy, and restores a drifted preset row
//! to its canonical grants. Presets are never edited in place — orgs fork.
//!
//! Permissions: the read is manager+ (any of `special:manage_users` /
//! `special:manage_roles` — the team surface and the roles admin share
//! it); fork/restore require `special:manage_roles`. Mutations ride the
//! IF1–IF3 instant-first envelope (idempotency-keyed claim, version-gated
//! seq, read-your-write `mutation` metadata) and emit `role.forked` /
//! `role.restored` + audit rows in the same commit.
//!
//! Non-purpose: no custom-role CRUD editor (the fork path creates the
//! copy; editing a custom role's matrix is the Phase-5 roles editor) and
//! no per-user assignment here (that is the org-users surface).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_db::roles::RoleEntry;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{check_any, BuiltinRole, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn manage_roles() -> Permission {
    Permission::ManageRoles
}

/// Manager+ read (the team surface reads it for its pickers too).
fn require_roles_reader(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    check_any(
        &principal.permissions,
        &[Permission::ManageUsers, Permission::ManageRoles],
    )
    .expect()
    .map_err(ApiError::Forbidden)
}

// --- wire structs ------------------------------------------------------------

/// One org role row (preset or custom). `permissions` is the raw matrix
/// vocabulary: `{"matrix": {"module", "action"}}` entries plus bare
/// special strings (`"financial:read"`, `"special:manage_users"`, …).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RoleView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    /// Shipped preset tag (owner, admin, …), `null` for org-created roles.
    #[schema(nullable = true)]
    pub builtin: Option<String>,
    /// Preset rows are system data: restorable, never editable in place.
    pub is_system: bool,
    /// For a forked copy: the preset tag it was seeded from.
    #[schema(nullable = true)]
    pub forked_from: Option<String>,
    pub permissions: Value,
    /// Active staff holding this role.
    pub holders: i64,
}

impl RoleView {
    fn from_entry(e: RoleEntry, holders: i64) -> Self {
        Self {
            id: e.id,
            org_id: e.org_id,
            name: e.name,
            builtin: e.builtin,
            is_system: e.is_system,
            forked_from: e.forked_from,
            permissions: e.permissions,
            holders,
        }
    }
}

/// The canonical definition of one shipped preset (RB1): the presets stay
/// canonical even though every org carries its own rows — restore resets
/// a row to this set.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PresetView {
    pub builtin: String,
    pub name: String,
    pub description: String,
    pub permissions: Value,
}

/// One portal tier in the preset roster (RB1). Portal authorization is
/// deliberately NOT the staff matrix (see `core_domain::policy`) — the
/// tiers render here as data with their capability keys, but they are
/// per-contact settings, never staff `roles` rows and never forkable.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortalTierView {
    pub tier: String,
    pub name: String,
    pub description: String,
    /// Capability keys from the tier×capability matrix (`policy::portal_tier_allows`).
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RolesListResponse {
    /// Every org role: presets (system rows) + org-created custom roles.
    pub roles: Vec<RoleView>,
    /// The canonical shipped presets (the restore source of truth).
    pub presets: Vec<PresetView>,
    /// The portal tiers (client contacts; a separate authorization model).
    pub portal_tiers: Vec<PortalTierView>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ForkRoleRequest {
    /// Name of the custom copy. Must not collide with an existing role
    /// name (typed 409 `role_name_taken`).
    pub name: String,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent
    /// in legacy clients — purely additive. (Creates carry no version.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RestoreRoleRequest {
    /// Instant-first autosave parameters (IF1): idempotency key + version.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the role at the top level plus
/// the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RoleMutationResponse {
    #[serde(flatten)]
    pub role: RoleView,
    pub mutation: instant::MutationMeta,
}

// --- handlers ----------------------------------------------------------------

/// GET /api/roles — the org's roles plus the canonical preset + portal
/// tier rosters (manager+).
#[utoipa::path(operation_id = "roles_list", get, path = "/api/roles", tag = "org-users", security(("bearer" = [])),
    responses((status = 200, body = RolesListResponse),
              (status = 403, description = "requires manage_users or manage_roles")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<RolesListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_roles_reader(&authed.principal)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entries = core_db::roles::list_roles(&mut *tx, org_id).await?;
    let mut roles = Vec::with_capacity(entries.len());
    for entry in entries {
        let holders = core_db::roles::count_holders(&mut *tx, org_id, entry.id).await?;
        roles.push(RoleView::from_entry(entry, holders));
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(RolesListResponse {
        roles,
        presets: preset_roster(),
        portal_tiers: portal_tier_roster(),
    }))
}

/// POST /api/roles/{id}/fork — duplicate a role (typically a preset) into
/// a mutable custom copy on the same matrix. The origin row is untouched.
#[utoipa::path(operation_id = "roles_fork", post, path = "/api/roles/{id}/fork", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ForkRoleRequest,
    responses((status = 200, body = RoleMutationResponse),
              (status = 404, description = "role not visible in scope"),
              (status = 409, description = "role name taken or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn fork(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkRoleRequest>,
) -> ApiResult<Json<RoleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let name = req.name.trim().to_string();
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::Validation(
            "role name is required (1..=64 characters)".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

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
        let origin = core_db::roles::get_role(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        let copy = core_db::roles::insert_forked_role(
            &mut ctx.tx,
            Uuid::now_v7(),
            org_id,
            &name,
            origin.builtin.as_deref(),
            &origin.permissions,
        )
        .await?;

        ctx.emit(
            types::ROLE_FORKED,
            ENTITY,
            Some(copy.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::RoleForked {
                role_id: copy.id,
                name: copy.name.clone(),
                from_builtin: origin.builtin.clone(),
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
                action: "role.forked".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(copy.id.to_string()),
                detail: serde_json::json!({
                    "name": copy.name,
                    "from_role_id": origin.id,
                    "from_builtin": origin.builtin,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: copy.id,
                version: None,
            },
        )
        .await?;
        let view = RoleView::from_entry(copy, 0);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "fork",
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
            MutationOutcome::Applied(RoleMutationResponse {
                role: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/roles/{id}/restore — reset a preset row's permission array to
/// the canonical shipped set (RB1: the presets stay canonical). Custom
/// roles and forks are org-owned data and are never restorable.
#[utoipa::path(operation_id = "roles_restore", post, path = "/api/roles/{id}/restore", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RestoreRoleRequest,
    responses((status = 200, body = RoleMutationResponse),
              (status = 404, description = "role not visible in scope, or not a preset"),
              (status = 409, description = "version conflict or idempotency key reused")))]
pub async fn restore(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RestoreRoleRequest>,
) -> ApiResult<Json<RoleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

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
        let before = core_db::roles::get_role(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)
            .map_err(instant::mutation_scope_err)?;
        if !(before.is_system && before.builtin.is_some()) {
            return Err(ApiError::Validation(
                "only shipped preset roles are restorable; custom roles are org-owned data".into(),
            ));
        }
        let builtin = BuiltinRole::parse_preset(before.builtin.as_deref().unwrap_or_default())
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!(
                    "system role row carries an unknown builtin tag"
                ))
            })?;
        let canonical = core_db::orgs::permissions_for_builtin(builtin);

        let restored =
            core_db::roles::restore_preset_permissions(&mut ctx.tx, org_id, id, &canonical)
                .await
                .map_err(ApiError::from)
                .map_err(instant::mutation_scope_err)?;

        ctx.emit(
            types::ROLE_RESTORED,
            ENTITY,
            Some(restored.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::RoleRestored {
                role_id: restored.id,
                builtin: restored.builtin.clone().unwrap_or_default(),
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
                action: "role.restored".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(restored.id.to_string()),
                detail: serde_json::json!({ "name": restored.name, "builtin": restored.builtin }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: restored.id,
                version: req.instant.version,
            },
        )
        .await?;
        let holders = core_db::roles::count_holders(&mut *ctx.tx, org_id, restored.id).await?;
        let view = RoleView::from_entry(restored, holders);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "restore",
                seq: write_seq,
                before: serde_json::to_value(RoleView::from_entry(before, holders)).ok(),
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
            MutationOutcome::Applied(RoleMutationResponse {
                role: view,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- canonical rosters --------------------------------------------------------

const ENTITY: &str = "role";

fn preset_description(builtin: BuiltinRole) -> &'static str {
    match builtin {
        BuiltinRole::Owner => "Full control including billing and the org itself",
        BuiltinRole::Admin => "Everything except org-level ownership actions",
        BuiltinRole::Dispatcher => "Schedules, assigns, and dispatches work",
        BuiltinRole::SeniorTech => {
            "Escalation tier: full ticket + KB control and time approvals, no financial visibility"
        }
        BuiltinRole::Technician => "Works tickets and the knowledge base",
        BuiltinRole::Bookkeeper => {
            "Money role: invoicing control and financial visibility, no ticket work"
        }
        BuiltinRole::Auditor => "Read everything plus the audit trail; writes nothing",
        BuiltinRole::ReadOnly => "View only, no audit-trail access",
    }
}

fn preset_roster() -> Vec<PresetView> {
    BuiltinRole::PRESETS
        .iter()
        .map(|b| PresetView {
            builtin: b.as_str().to_string(),
            name: display_name(*b).to_string(),
            description: preset_description(*b).to_string(),
            permissions: core_db::orgs::permissions_for_builtin(*b),
        })
        .collect()
}

fn display_name(builtin: BuiltinRole) -> &'static str {
    match builtin {
        BuiltinRole::Owner => "Owner",
        BuiltinRole::Admin => "Admin",
        BuiltinRole::Dispatcher => "Dispatcher",
        BuiltinRole::SeniorTech => "Senior Tech",
        BuiltinRole::Technician => "Technician",
        BuiltinRole::Bookkeeper => "Bookkeeper",
        BuiltinRole::Auditor => "Auditor",
        BuiltinRole::ReadOnly => "Read Only",
    }
}

/// The portal tiers rendered through the same roster (RB1). Capability
/// keys come from the SAME tier×capability matrix `check_portal` enforces
/// — the roster and the rules cannot drift (pinned by a policy test).
fn portal_tier_roster() -> Vec<PortalTierView> {
    use core_domain::entities::PortalRole;
    use core_domain::policy::{portal_tier_allows, PortalAction};
    let tier = |r: PortalRole, name: &str, description: &str| PortalTierView {
        tier: r.as_str().to_string(),
        name: name.to_string(),
        description: description.to_string(),
        capabilities: PortalAction::ALL
            .iter()
            .copied()
            .filter(|a| portal_tier_allows(r, *a))
            .map(PortalAction::as_str)
            .map(str::to_string)
            .collect(),
    };
    vec![
        tier(
            PortalRole::Contact,
            "Contact",
            "Default portal tier: own client's tickets, invoices, KB, and appointments",
        ),
        tier(
            PortalRole::Approver,
            "Approver",
            "Contact capabilities today; a slot for future client-side approval grants",
        ),
        tier(
            PortalRole::ItManager,
            "IT Manager",
            "Adds in-house update notes to the contact tier (never internal notes)",
        ),
    ]
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct UpdateRoleRequest {
    #[schema(value_type = Option<String>)]
    pub name: Option<String>,
    #[schema(value_type = Option<Vec<String>>, max_items = 64)]
    pub permissions: Option<Vec<String>>,
    #[serde(flatten)]
    pub instant: instant::MutationParams,
}

/// PUT /api/roles/{id} — the custom-roles EDITOR (Wave 22): updates a
/// custom row's name and/or permission set. Builtin presets are typed
/// refusals (rb1) — restore is the ONLY preset-matrix mutation.
#[utoipa::path(operation_id = "roles_update", put, path = "/api/roles/{id}", tag = "org-users", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateRoleRequest,
    responses((status = 200, body = RoleMutationResponse),
              (status = 403, description = "builtin_role_immutable"),
              (status = 404),
              (status = 422, description = "validation failure")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateRoleRequest>,
) -> ApiResult<Json<RoleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_roles())?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;
    let view = org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        let origin = core_db::roles::get_role(&mut *ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        if origin.builtin.is_some() {
            return Err(ApiError::Forbidden(
                "BUILTIN_ROLE_IMMUTABLE: fork the preset instead — the shipped presets stay canonical (RB1)".into(),
            ));
        }
        if let Some(name) = req.name.as_deref() {
            let name = name.trim();
            if name.is_empty() || name.len() > 64 {
                return Err(ApiError::Validation(
                    "role name is required (1..=64 characters)".into(),
                ));
            }
        }
        if let Some(perms) = req.permissions.as_ref() {
            if perms.len() > core_domain::policy::Module::ALL.len() * core_domain::policy::Action::ALL.len() {
                return Err(ApiError::Validation(
                    "the permission set exceeds the full matrix".into(),
                ));
            }
        }
        // Resolve every key BEFORE the write. The stored row is read back as
        // `Vec<Permission>`, and a key that does not resolve makes that read
        // fail — which stores a role that reads back EMPTY, silently granting
        // nothing. Refuse the unknown key with a typed 422 naming it, and
        // store the resolved form so what was written is exactly what
        // authorization reads. The parser is the service-account grant parser
        // (one vocabulary for both write paths).
        let resolved: Option<Vec<Permission>> = match req.permissions.as_ref() {
            None => None,
            Some(keys) => {
                let mut out: Vec<Permission> = Vec::with_capacity(keys.len());
                for key in keys {
                    let Some(perm) = crate::routes::service_accounts::permission_from_key(key)
                    else {
                        return Err(ApiError::Validation(format!(
                            "unknown permission key {key:?}; keys are \"<module>:<action>\" \
                             (\"tickets:view\") or a policy special (\"special:manage_roles\", \
                             \"financial:read\", ...)"
                        )));
                    };
                    if !out.contains(&perm) {
                        out.push(perm);
                    }
                }
                Some(out)
            }
        };
        core_db::roles::update_custom_role(
            &mut ctx.tx,
            org_id,
            id,
            req.name.as_deref(),
            resolved.as_deref(),
        )
        .await
        .map_err(ApiError::from)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "role.updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "name": req.name,
                    "permission_count": req.permissions.as_ref().map(|p| p.len()),
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec { entity: ENTITY, entity_id: id, version: None },
        )
        .await?;
        let refreshed = core_db::roles::get_role(&mut *ctx.tx, org_id, id).await.map_err(ApiError::from)?;
        let view = RoleView::from_entry(refreshed, 0);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "update",
                seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: req.instant.idempotency_key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, RoleMutationResponse { role: view, mutation }))
    })
    .await?;
    Ok(Json(view))
}
