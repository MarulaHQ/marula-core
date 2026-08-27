//! Request middleware: extracts the Bearer token, verifies the JWT, and
//! loads a fresh Principal (user + roles + unioned permissions) from the DB.
//! Permissions are never cached in the token — revocation applies instantly.

use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};
use core_domain::policy::{BuiltinRole, Permission};

/// Resolves a user's effective permission set: the union of every role's
/// grants, expanded from builtin defaults where applicable.
pub fn resolve_permissions(roles: &[core_domain::entities::Role]) -> Vec<Permission> {
    let mut out: Vec<Permission> = Vec::new();
    for role in roles {
        // A builtin role always carries its full default set (defends
        // against drifted seeds); custom grants layer on top.
        let grants = role
            .builtin
            .map(|b| b.permissions())
            .unwrap_or_else(|| role.permissions.clone());
        for p in grants {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    // Owner/Admin are superusers by definition.
    if roles.iter().any(|r| {
        matches!(
            r.builtin,
            Some(BuiltinRole::Owner) | Some(BuiltinRole::Admin)
        )
    }) {
        for m in core_domain::policy::Module::ALL {
            for a in core_domain::policy::Action::ALL {
                let p = Permission::Matrix {
                    module: m,
                    action: a,
                };
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Axum middleware: `Authorization: Bearer <jwt>` for humans, with a
/// scoped machine path via `x-api-key` (TASKLIST 2.15).
///
/// The API-key branch is deliberately NARROW: only read-only public data
/// prefixes accept keys, scopes are enforced against plugin capability
/// names, and the synthesized principal is read-only by construction
/// (`is_machine` + empty roles). Self-scoped surfaces (notifications,
/// timers) reject keys outright — a key id is not a user id.
pub async fn require_auth(
    State(state): State<crate::state::AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(raw_key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        let path = request.uri().path();
        let allowed = API_KEY_PATHS.iter().any(|p| {
            path == *p
                || path
                    .strip_prefix(p)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        if !allowed {
            return Err(ApiError::Unauthorized(
                "api keys are not accepted on this endpoint".into(),
            ));
        }
        // Constant-time-ish compare happens at the DB index; the hash
        // itself never leaves the process and raw keys are never logged.
        let hash = crate::routes::api_keys::hash_raw_key(&state, raw_key);
        let row = core_db::webhooks::find_active_api_key_by_hash(&state.pool, &hash)
            .await
            .map_err(|_| ApiError::Unauthorized("invalid api key".into()))?
            .ok_or_else(|| ApiError::Unauthorized("invalid api key".into()))?;

        // Throttled last-used stamp: at most one write per minute per key.
        if let Err(error) =
            core_db::webhooks::touch_api_key_last_used(&state.pool, row.id, 60).await
        {
            tracing::warn!(%error, "api key last-used touch failed");
        }

        let permissions = permissions_from_scopes(&row.scopes);
        if permissions.is_empty() {
            return Err(ApiError::Forbidden(
                "api key carries no usable scopes".into(),
            ));
        }
        request.extensions_mut().insert(AuthedUser {
            principal: core_domain::entities::Principal {
                user: machine_user(row.id, row.org_id),
                roles: vec![],
                permissions,
            },
            session_id: None,
            via_api_key: true,
        });
        return Ok(next.run(request).await);
    }

    let token = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?
        .to_string();

    let claims = crate::auth::jwt::verify_access_token(&state.config, &token)
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired token".into()))?;

    // Load user + roles fresh per request. Two cheap indexed queries; the
    // correctness win (instant revocation) outweighs the cost at v1 scale.
    let user = core_db::users::get_user(&state.pool, claims.org, claims.sub)
        .await
        .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
    if !user.is_active {
        return Err(ApiError::Unauthorized("account disabled".into()));
    }
    let roles = core_db::users::roles_for_user(&state.pool, claims.sub, claims.org).await?;
    let permissions = resolve_permissions(&roles);

    request.extensions_mut().insert(AuthedUser {
        principal: core_domain::entities::Principal {
            user,
            roles,
            permissions,
        },
        session_id: Some(claims.sid),
        via_api_key: false,
    });

    Ok(next.run(request).await)
}

/// Read-only prefixes where `x-api-key` auth is accepted. Everything else
/// (mutations, admin surfaces, self-scoped endpoints) is JWT-only.
/// `/api/imports` is the deliberate mutation exception (TASKLIST D2/D3):
/// scoped migration pushes are a documented machine flow, gated in-handler
/// by scope checks on top of this prefix allowance.
const API_KEY_PATHS: &[&str] = &["/api/tickets", "/api/clients", "/api/imports"];

/// Maps capability scopes onto the permission matrix. Unknown scopes are
/// ignored here (they cannot be created — validated at key creation).
fn permissions_from_scopes(scopes: &[String]) -> Vec<core_domain::policy::Permission> {
    use core_domain::policy::{Action, Module, Permission};
    let mut out = Vec::new();
    let mut push = |module: Module, actions: &[Action]| {
        for a in actions {
            let p = Permission::Matrix { module, action: *a };
            if !out.contains(&p) {
                out.push(p);
            }
        }
    };
    for scope in scopes {
        match scope.as_str() {
            "tickets:read" => push(Module::Tickets, &[Action::View]),
            "tickets:write" => push(
                Module::Tickets,
                &[Action::View, Action::Create, Action::Edit],
            ),
            "clients:read" => push(Module::Clients, &[Action::View]),
            "clients:write" => push(
                Module::Clients,
                &[Action::View, Action::Create, Action::Edit],
            ),
            "timers:read" => push(Module::Time, &[Action::View]),
            "timers:write" => push(Module::Time, &[Action::View, Action::Create, Action::Edit]),
            _ => {}
        }
    }
    out
}

/// Synthetic principal user for machine keys: the key id stands in as the
/// actor id (audit rows attribute to it); no email/timezone exist.
fn machine_user(key_id: uuid::Uuid, org_id: uuid::Uuid) -> core_domain::entities::User {
    core_domain::entities::User {
        id: key_id,
        org_id,
        email: String::new(),
        display_name: "api-key".into(),
        timezone: "UTC".into(),
        is_active: true,
        mfa_totp_secret: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// Helper for handlers: extract the AuthedUser extension.
pub fn authed<T>(extensions: &axum::http::Extensions) -> ApiResult<T>
where
    T: Clone + Send + Sync + 'static,
{
    extensions
        .get::<T>()
        .cloned()
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))
}

/// Guard: assert the principal holds a permission or return 403.
pub fn require_permission(
    principal: &core_domain::entities::Principal,
    required: core_domain::policy::Permission,
) -> ApiResult<()> {
    core_domain::policy::check(&principal.permissions, required)
        .expect()
        .map_err(ApiError::Forbidden)
}
