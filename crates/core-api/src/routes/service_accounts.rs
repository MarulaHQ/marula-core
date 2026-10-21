//! Service-account management + client-certificate auth surface (Wave 47
//! lane A9, `p47-client-cert-auth`).
//!
//! Purpose: admin CRUD over org service accounts. A service account is a
//! machine identity bound to the SHA-256 fingerprint of an enrolled client
//! certificate; it authenticates through the cert branch of
//! `crate::auth::middleware` and then acts with the exact permission set
//! named by its `scopes` (resolved through the SAME policy matrix a human
//! role flows through — no parallel auth system). Org binding is the row's
//! `org_id`, so a cert enrolled for one org can never authenticate into
//! another. Enrollment, rotation and revocation are admin-only
//! ([`Permission::ManageApiKeys`]) and RLS-scoped.
//!
//! v1 posture (see internal/design/qa-p47-client-cert-auth/LANDING.md): the
//! fingerprint is presented via a header a trusted edge proxy forwards;
//! the binding is fingerprint-PINNED (re-pin = create + revoke). Full
//! CA-chain validation and per-org CA bundles are documented follow-ups.
//!
//! The `PluginCapability::ServiceAccountManage` key (Wave 47 A9) only
//! DECLARES the service-account surface for a runtime and resolves onto the
//! plugins-view gate — it never grants enrollment/revocation, which stays
//! a human [`Permission::ManageApiKeys`] action.
//!
//! Non-purpose: no certificate parsing or fingerprint computation here
//! (the proxy supplies the fingerprint; the middleware compares it), no
//! permission-model changes (grants validate against the EXISTING matrix —
//! this module never mint a new [`core_domain::policy::Permission`]).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{Extensions, StatusCode},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Grant-key vocabulary helpers shared with the auth middleware: the
// middleware resolves a service account's stored scopes into a permission
// set, and this module validates the same keys at write time. ONE parser,
// so what is written is exactly what authenticates.

/// The largest grant set a single service account may hold.
const MAX_GRANTS: usize = 32;
/// Match the API-key surface's name bound.
const MAX_NAME_CHARS: usize = 120;

/// Resolves a single grant key onto the EXISTING permission vocabulary,
/// or `None` when the key is unknown (refused at write time / ignored on
/// read). Matrix grants are `<module>:<action>`; the module-independent
/// operational grants use their wire keys.
///
/// Deliberately exhaustive over today's [`core_domain::policy::Permission`]
/// variants. A permission a future lane adds becomes grantable only once it
/// has a key here (deny-by-default until then) — this lane mints none.
pub fn permission_from_key(key: &str) -> Option<Permission> {
    use core_domain::policy::{Action, Module};
    match key {
        "special:manage_users" => return Some(Permission::ManageUsers),
        "special:manage_roles" => return Some(Permission::ManageRoles),
        "special:manage_boards" => return Some(Permission::ManageBoards),
        "special:approve_time" => return Some(Permission::ApproveTime),
        "special:manage_billing" => return Some(Permission::ManageBilling),
        "special:manage_plugins" => return Some(Permission::ManagePlugins),
        "special:manage_api_keys" => return Some(Permission::ManageApiKeys),
        "special:view_audit_log" => return Some(Permission::ViewAuditLog),
        "special:portal_admin" => return Some(Permission::PortalAdmin),
        // ADR 0004 RB2: financial visibility sits outside the `special:`
        // namespace deliberately.
        "financial:read" => return Some(Permission::ViewFinancials),
        // Wave 47 lane A6: org usage read-only monitoring grant.
        "org_usage:read" => return Some(Permission::ViewOrgUsage),
        // Wave 47 lane A8: custom-domain read + manage, the same
        // module-namespaced shape.
        "org_custom_domain:read" => return Some(Permission::ViewOrgCustomDomain),
        "org_custom_domain:manage" => return Some(Permission::ManageOrgCustomDomain),
        // Wave 47 lane A7: the suspension notice read grant.
        "org_suspension:read" => return Some(Permission::ViewOrgSuspension),
        // Wave 52 lane R12: the observability reader grant. A machine key with
        // this scope reads the org's audit trail (and its export cursor) and
        // still cannot write one, which is what makes it a reader and not the
        // `special:view_audit_log` an admin holds.
        "observability:read" => return Some(Permission::ViewObservability),
        // Wave 45 lane 1 (ADR 0020): RAG capability aliases resolve onto the
        // Kb matrix (`kb:view` / `kb:edit`), matching `PluginCapability::RagRead`
        // and `PluginCapability::RagWrite`.
        "rag:read" | "rag.read" => {
            return Some(Permission::Matrix {
                module: Module::Kb,
                action: Action::View,
            })
        }
        "rag:write" | "rag.write" => {
            return Some(Permission::Matrix {
                module: Module::Kb,
                action: Action::Edit,
            })
        }
        _ => {}
    }
    let (module_key, action_key) = key.split_once(':')?;
    let module = Module::ALL.into_iter().find(|m| m.as_str() == module_key)?;
    let action = Action::ALL.into_iter().find(|a| a.as_str() == action_key)?;
    Some(Permission::Matrix { module, action })
}

/// Resolves a stored scope list into the principal's permission set,
/// de-duplicated and in first-seen order. Unknown keys are dropped (they
/// cannot have been written — validated at create/patch time).
pub fn resolve_scope_permissions(scopes: &[String]) -> Vec<Permission> {
    let mut out: Vec<Permission> = Vec::new();
    for key in scopes {
        if let Some(p) = permission_from_key(key) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

fn validate_grants(scopes: &[String]) -> ApiResult<()> {
    if scopes.is_empty() {
        return Err(ApiError::Validation(
            "at least one permission grant is required".into(),
        ));
    }
    if scopes.len() > MAX_GRANTS {
        return Err(ApiError::Validation(format!(
            "a service account may hold at most {MAX_GRANTS} grants"
        )));
    }
    for s in scopes {
        if permission_from_key(s).is_none() {
            return Err(ApiError::Validation(format!(
                "unknown permission grant {s:?}; grants must be policy keys \
                 (\"tickets:view\", \"special:manage_api_keys\", ...)"
            )));
        }
    }
    Ok(())
}

/// The GRANT CEILING. A caller may pass on only authority it already holds:
/// no credential, human or machine, writes a grant set wider than its own
/// permission set. Without this, a machine account holding
/// `special:manage_api_keys` could PATCH its own scopes to `admin:edit` and
/// the other admin specials and then use them with the certificate it already
/// holds. `validate_grants` proves the keys resolve; this proves the caller
/// owns what it hands out.
fn require_grantable(authed: &crate::auth::AuthedUser, scopes: &[String]) -> ApiResult<()> {
    for key in scopes {
        let Some(perm) = permission_from_key(key) else {
            // `validate_grants` owns the unknown-key message; keep this
            // fail-closed in case it is ever reordered.
            return Err(ApiError::Validation(format!(
                "unknown permission grant {key:?}; grants must be policy keys"
            )));
        };
        if !core_domain::policy::check(&authed.principal.permissions, perm).is_allowed() {
            return Err(ApiError::Forbidden(format!(
                "cannot grant {key:?}: the caller does not hold it"
            )));
        }
    }
    Ok(())
}

/// A machine credential must not rewrite its own grant set. The ceiling above
/// already denies escalation; this closes the case where one machine account
/// edits another's scopes, which has no second party in the loop. Only a
/// client-certificate principal can match its own row id, because a staff
/// principal's id is a `users.id` and a service account's is a
/// `service_accounts.id`.
fn refuse_self_grant_change(authed: &crate::auth::AuthedUser, id: Uuid) -> ApiResult<()> {
    if authed.via_api_key && authed.principal.user.id == id {
        return Err(ApiError::Forbidden(
            "a service account may not change its own grants".into(),
        ));
    }
    Ok(())
}

fn validate_status(status: &str) -> ApiResult<()> {
    match status {
        "active" | "suspended" => Ok(()),
        other => Err(ApiError::Validation(format!(
            "unknown status {other:?}; must be \"active\" or \"suspended\""
        ))),
    }
}

/// The single normalizer for a client-certificate fingerprint, shared by
/// BOTH the enrollment side (`create`) and the auth side
/// (`auth::middleware::cert_presentation`). One owner keeps the two ends of
/// the enroll-vs-auth invariant coupled: the key enrollment stores is
/// exactly the key authentication looks up. Separators and whitespace are
/// stripped so the common `AB:CD:…`/spaced encodings land on the same key,
/// and the result must be exactly 64 lowercase hex digits (a SHA-256
/// digest); anything else is rejected.
pub fn normalize_fingerprint(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    (cleaned.len() == 64).then_some(cleaned)
}

// --- wire shapes ---------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountView {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    /// Lowercase hex SHA-256 of the pinned client certificate.
    pub cert_fingerprint: String,
    pub cert_subject: Option<String>,
    pub scopes: Vec<String>,
    /// Admin lifecycle: "active" | "suspended".
    pub status: String,
    /// Derived lifecycle: "valid" | "expired" | "revoked".
    pub cert_status: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Admin-supplied reason recorded at revocation, if any.
    pub revocation_reason: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    /// The enrolling principal's id (the human admin), if a human enrolled it.
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl ServiceAccountView {
    fn from_row(r: core_db::service_accounts::ServiceAccountRow, now: DateTime<Utc>) -> Self {
        let cert_status = r.cert_status(now).as_str().into();
        Self {
            id: r.id,
            org_id: r.org_id,
            name: r.name,
            cert_fingerprint: r.cert_fingerprint,
            cert_subject: r.cert_subject,
            scopes: r.scopes,
            status: r.status,
            cert_status,
            expires_at: r.expires_at,
            revoked_at: r.revoked_at,
            revocation_reason: r.revocation_reason,
            last_used_at: r.last_used_at,
            created_by: r.created_by,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountList {
    pub service_accounts: Vec<ServiceAccountView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServiceAccountRequest {
    pub name: String,
    /// Lowercase hex SHA-256 of the client certificate to pin.
    pub cert_fingerprint: String,
    /// Optional display-only subject DN.
    #[serde(default)]
    pub cert_subject: Option<String>,
    /// Permission-grant keys (validated against the policy matrix).
    pub scopes: Vec<String>,
    /// Optional expiry; must be in the future when provided.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateServiceAccountRequest {
    /// Replacement grant set. Omitted ⇒ keep current grants.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    /// Replacement expiry (null ⇒ never expires). Omitted ⇒ keep current.
    #[serde(default)]
    pub expires_at: Option<Option<DateTime<Utc>>>,
    /// Replacement status. Omitted ⇒ keep current.
    #[serde(default)]
    pub status: Option<String>,
}

/// Optional query for revoke: a human-supplied `?reason=` recorded on the
/// service account and surfaced in listings.
#[derive(Debug, Default, Deserialize)]
pub struct RevokeQuery {
    #[serde(default)]
    pub reason: Option<String>,
}

// --- handlers ------------------------------------------------------------------

/// The authenticated caller must act only on their own org: the `{org_id}`
/// path segment must equal the principal's org. A cross-org path is a 404
/// (indistinguishable from not-existing under RLS), never a 403 that would
/// confirm another org's existence.
fn org_scope(authed: &crate::auth::AuthedUser, path_org: Uuid) -> ApiResult<Uuid> {
    let org_id = authed.principal.user.org_id;
    if org_id != path_org {
        return Err(ApiError::NotFound);
    }
    Ok(org_id)
}

/// GET /orgs/{org_id}/service-accounts — admin listing with derived cert
/// status.
#[utoipa::path(
    operation_id = "service_accounts_list",
    get,
    path = "/api/orgs/{org_id}/service-accounts",
    tag = "integrations",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path)),
    responses((status = 200, body = ServiceAccountList))
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<ServiceAccountList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let org_id = org_scope(&authed, org_id)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::service_accounts::list_service_accounts(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let now = Utc::now();
    Ok(Json(ServiceAccountList {
        service_accounts: rows
            .into_iter()
            .map(|r| ServiceAccountView::from_row(r, now))
            .collect(),
    }))
}

/// POST /orgs/{org_id}/service-accounts — enroll a service account by
/// pinning a certificate fingerprint with permission grants and an
/// optional expiry.
#[utoipa::path(
    operation_id = "service_accounts_create",
    post,
    path = "/api/orgs/{org_id}/service-accounts",
    tag = "integrations",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path)),
    request_body = CreateServiceAccountRequest,
    responses((status = 201, body = ServiceAccountView))
)]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
    Json(req): Json<CreateServiceAccountRequest>,
) -> ApiResult<(StatusCode, Json<ServiceAccountView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let org_id = org_scope(&authed, org_id)?;
    let (actor, created_by) = acting_identity(&authed);

    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::Validation(format!(
            "name must be 1..={MAX_NAME_CHARS} chars"
        )));
    }
    validate_grants(&req.scopes)?;
    require_grantable(&authed, &req.scopes)?;
    let fingerprint = normalize_fingerprint(&req.cert_fingerprint).ok_or_else(|| {
        ApiError::Validation("cert_fingerprint must be a 64-char hex SHA-256 digest".into())
    })?;
    if req.expires_at.is_some_and(|exp| exp <= Utc::now()) {
        return Err(ApiError::Validation(
            "expires_at must be in the future".into(),
        ));
    }
    if let Some(subject) = req.cert_subject.as_deref() {
        if subject.chars().count() > MAX_NAME_CHARS {
            return Err(ApiError::Validation(format!(
                "cert_subject must be at most {MAX_NAME_CHARS} chars"
            )));
        }
    }

    let id = Uuid::now_v7();
    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let row = core_db::service_accounts::insert_service_account(
        &mut *tx.tx,
        id,
        org_id,
        name,
        &fingerprint,
        req.cert_subject.as_deref(),
        &req.scopes,
        req.expires_at,
        created_by,
    )
    .await
    .map_err(unique_conflict)?;
    audit(
        &mut tx.tx,
        org_id,
        actor,
        id,
        "service_account.created",
        serde_json::json!({ "name": name, "scopes": req.scopes }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(ServiceAccountView::from_row(row, Utc::now())),
    ))
}

/// PATCH /orgs/{org_id}/service-accounts/{id} — update grants, expiry, or
/// lifecycle status. The pinned fingerprint and org binding are immutable
/// (a re-pin is a create + revoke).
#[utoipa::path(
    operation_id = "service_accounts_update",
    patch,
    path = "/api/orgs/{org_id}/service-accounts/{id}",
    tag = "integrations",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path), ("id" = Uuid, Path)),
    request_body = UpdateServiceAccountRequest,
    responses((status = 200, body = ServiceAccountView))
)]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((org_id, id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateServiceAccountRequest>,
) -> ApiResult<Json<ServiceAccountView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let org_id = org_scope(&authed, org_id)?;
    let (actor, _created_by) = acting_identity(&authed);

    if let Some(status) = req.status.as_deref() {
        validate_status(status)?;
    }
    if let Some(scopes) = req.scopes.as_ref() {
        validate_grants(scopes)?;
        require_grantable(&authed, scopes)?;
        refuse_self_grant_change(&authed, id)?;
    }
    // Expiry override: the double-Option distinguishes "absent ⇒ keep"
    // (None) from "present as null ⇒ clear" (Some(None)) from
    // "present as timestamp ⇒ set" (Some(t)).
    if let Some(Some(exp)) = req.expires_at {
        if exp <= Utc::now() {
            return Err(ApiError::Validation(
                "expires_at must be in the future".into(),
            ));
        }
    }

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    // Load current within the org scope (RLS): a foreign id is absent ⇒ 404.
    let current = core_db::service_accounts::get_service_account(&mut *tx.tx, org_id, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let scopes = req.scopes.clone().unwrap_or(current.scopes);
    let status = req.status.clone().unwrap_or(current.status);
    let expires_at = req.expires_at.unwrap_or(current.expires_at);

    core_db::service_accounts::update_service_account(
        &mut *tx.tx,
        org_id,
        id,
        &scopes,
        expires_at,
        &status,
    )
    .await
    .map_err(not_visible)?;

    // One owner of the lifecycle: rebuild the row from the mutated fields and
    // derive the view through `ServiceAccountView::from_row`, so the derived
    // cert status is computed by the SAME function every other read path uses
    // (no third hand-maintained copy here). Built before the audit so the
    // detail can borrow/clone from it rather than stealing the fields.
    let updated = core_db::service_accounts::ServiceAccountRow {
        scopes,
        status,
        expires_at,
        ..current
    };
    audit(
        &mut tx.tx,
        org_id,
        actor,
        id,
        "service_account.updated",
        serde_json::json!({
            "scopes": updated.scopes.clone(),
            "status": updated.status.clone(),
            "expires_at": updated.expires_at,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(ServiceAccountView::from_row(updated, Utc::now())))
}

/// DELETE /orgs/{org_id}/service-accounts/{id} — immediate, permanent
/// revocation. Idempotent: a second delete of an already-revoked account
/// stays successful (the row is present in-scope), so the surface is safe
/// to retry.
#[utoipa::path(
    operation_id = "service_accounts_delete",
    delete,
    path = "/api/orgs/{org_id}/service-accounts/{id}",
    tag = "integrations",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path), ("id" = Uuid, Path), ("reason" = Option<String>, Query)),
    responses((status = 204))
)]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((org_id, id)): Path<(Uuid, Uuid)>,
    axum::extract::Query(query): axum::extract::Query<RevokeQuery>,
) -> ApiResult<StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageApiKeys)?;
    let org_id = org_scope(&authed, org_id)?;
    let (actor, _created_by) = acting_identity(&authed);
    let reason = query
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    // Confirm the row exists in-scope first so an unknown id is a clean 404
    // (DB revoke also NotFounds for an out-of-scope id).
    let present = core_db::service_accounts::get_service_account(&mut *tx.tx, org_id, id)
        .await?
        .is_some();
    if !present {
        return Err(ApiError::NotFound);
    }
    // Idempotent revoke: already-revoked rows keep their revoked_at, and the
    // update still affects zero rows on the second pass — treat that as
    // success here since we confirmed the row is in scope above.
    let _ =
        core_db::service_accounts::revoke_service_account(&mut *tx.tx, org_id, id, reason).await;
    audit(
        &mut tx.tx,
        org_id,
        actor,
        id,
        "service_account.revoked",
        serde_json::json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- helpers -------------------------------------------------------------------

async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: Uuid,
    actor: Actor,
    account_id: Uuid,
    action: &str,
    detail: serde_json::Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut **tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor,
            action: action.into(),
            entity_type: Some("service_account".into()),
            entity_id: Some(account_id.to_string()),
            detail,
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    Ok(())
}

/// The acting identity for an admin mutation. A human bearer is
/// `Actor::Human(<user id>)` and its id can back `created_by`. A machine
/// principal (a certified service account holding ManageApiKeys) is
/// `Actor::Agent("service-account:<id>")`, and its id is NOT a `users` row,
/// so `created_by` is left `None` (the FK to `users` would otherwise fail).
fn acting_identity(authed: &crate::auth::AuthedUser) -> (Actor, Option<Uuid>) {
    let id = authed.principal.user.id;
    if authed.via_api_key {
        (Actor::Agent(format!("service-account:{id}")), None)
    } else {
        (Actor::Human(id), Some(id))
    }
}

/// Maps a fingerprint UNIQUE-violation to a typed 409 (that certificate is
/// already enrolled); every other DB error passes through.
fn unique_conflict(e: core_db::DbError) -> ApiError {
    let unique = matches!(
        &e,
        core_db::DbError::Sql(sqlx::Error::Database(db_err)) if db_err.is_unique_violation()
    );
    if unique {
        ApiError::Conflict("that certificate fingerprint is already enrolled".into())
    } else {
        e.into()
    }
}

/// A `NotFound` from an org-scoped update/delete is the RLS property: the
/// row left the visible scope, was deleted, or never existed. Map it to
/// `NotFound` so the admin surface answers uniformly.
fn not_visible(e: core_db::DbError) -> ApiError {
    match e {
        core_db::DbError::NotFound => ApiError::NotFound,
        other => other.into(),
    }
}

/// Registers the service-account routes under one generation's base so the
/// canonical `/api/v1` and legacy `/api` mounts share identical handlers.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/orgs/{{org_id}}/service-accounts"),
            get(list).post(create),
        )
        .route(
            &format!("{base}/orgs/{{org_id}}/service-accounts/{{id}}"),
            axum::routing::patch(update).delete(remove),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_fingerprint_keeps_enroll_auth_parity() {
        // The contiguous key both sides reduce to.
        let base = "ab12cd34ef56ab78";
        let raw = format!("{base}{base}{base}{base}"); // 64 hex, lowercase
        let canonical = normalize_fingerprint(&raw).expect("64-hex is valid");

        // Every wire encoding a proxy actually emits reduces to the SAME key,
        // so what enrollment stored is what authentication looks up. (A `0x`
        // prefix is deliberately NOT a form: stripping its `x` leaves a stray
        // `0` and would corrupt the digest.)
        let colon = raw
            .chars()
            .collect::<Vec<_>>()
            .chunks(2)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(":");
        let spaced = colon.replace(':', " ");
        let upper = raw.to_uppercase();
        let colon_upper = colon.to_uppercase();
        for form in [raw.clone(), colon, spaced, upper, colon_upper] {
            assert_eq!(
                normalize_fingerprint(&form).as_deref(),
                Some(canonical.as_str()),
                "form must normalize to the enrolled key"
            );
        }
        assert_eq!(canonical, raw);
    }

    #[test]
    fn normalize_fingerprint_rejects_wrong_length_or_non_hex() {
        assert!(normalize_fingerprint("short").is_none());
        assert!(normalize_fingerprint(&"zz".repeat(32)).is_none()); // 64 non-hex
        assert!(normalize_fingerprint(&"ab".repeat(31)).is_none()); // 62 hex
        assert!(normalize_fingerprint("").is_none());
    }

    #[test]
    fn permission_from_key_covers_the_matrix_and_specials() {
        // Matrix grants decompose into module + action.
        assert_eq!(
            permission_from_key("tickets:view"),
            Some(Permission::Matrix {
                module: core_domain::policy::Module::Tickets,
                action: core_domain::policy::Action::View,
            })
        );
        // Operational specials resolve to their variants.
        assert_eq!(
            permission_from_key("special:manage_api_keys"),
            Some(Permission::ManageApiKeys)
        );
        // ADR 0004 RB2: financial visibility sits outside `special:`.
        assert_eq!(
            permission_from_key("financial:read"),
            Some(Permission::ViewFinancials)
        );
        // Unknown keys are refused at write time (typed validation).
        assert!(permission_from_key("not:real").is_none());
        assert!(permission_from_key("tickets:reade").is_none());
    }
}
