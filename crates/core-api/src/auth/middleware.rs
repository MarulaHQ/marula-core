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
        matches!(r.builtin, Some(BuiltinRole::Owner) | Some(BuiltinRole::Admin))
    }) {
        for m in core_domain::policy::Module::ALL {
            for a in core_domain::policy::Action::ALL {
                let p = Permission::Matrix { module: m, action: a };
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Axum middleware: `Authorization: Bearer <jwt>`.
pub async fn require_auth(
    State(state): State<crate::state::AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
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
        principal: core_domain::entities::Principal { user, roles, permissions },
    });

    Ok(next.run(request).await)
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
