//! Auth HTTP handlers: register (org bootstrap), login, refresh, logout, me.
//! All audit-relevant events are recorded (§6: auth events).

use crate::auth::{jwt, password, AccessClaims, AuthedUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::entities::{Actor, AuditEvent};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    /// Organization name — becomes the tenant for this deployment.
    pub org_name: String,
    /// URL-safe org slug (unique).
    pub org_slug: String,
    /// Ticket number prefix, e.g. "MER" → MER-2841. Defaults to "MER".
    pub ticket_prefix: Option<String>,
    pub email: String,
    pub password: String,
    pub display_name: String,
    #[schema(default = "UTC")]
    pub timezone: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
    /// Session id returned alongside the refresh token.
    pub session_id: Uuid,
}

fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.len() < 10 {
        return Err(ApiError::Validation("password must be at least 10 characters".into()));
    }
    Ok(())
}

fn validate_email(email: &str) -> ApiResult<()> {
    let ok = email.contains('@') && email.contains('.') && !email.contains(' ') && email.len() >= 5;
    if ok { Ok(()) } else { Err(ApiError::Validation("invalid email".into())) }
}

/// POST /api/auth/register — creates the org, its default roles/board/SLA
/// policy, and the first user as Owner. One transaction: a half-created org
/// can never exist.
#[utoipa::path(post, path = "/api/auth/register", tag = "auth",
    request_body = RegisterRequest, responses(
        (status = 200, body = TokenPair),
        (status = 422, description = "validation error"),
        (status = 409, description = "slug or email already exists")
))]
pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<TokenPair>> {
    validate_email(&req.email)?;
    validate_password(&req.password)?;
    let slug_ok = !req.org_slug.is_empty()
        && req.org_slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !slug_ok {
        return Err(ApiError::Validation("org_slug must be lowercase alphanumeric or '-'".into()));
    }

    let org_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let password_hash = password::hash(&req.password).map_err(|e| ApiError::Internal(e.into()))?;

    // The registration transaction runs as the app role with RLS. We bind
    // the tx to the NEW org id first, so the org row insert satisfies its
    // own isolation policy (id = app.org_id) and everything downstream
    // inherits the scope.
    let mut tx = state.pool.begin().await.map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;

    core_db::orgs::insert_org_tx(&mut tx, org_id, &req.org_name, &req.org_slug,
        req.ticket_prefix.as_deref().unwrap_or("MER")).await?;
    core_db::orgs::seed_default_roles_tx(&mut tx, org_id).await?;
    core_db::orgs::seed_default_board_tx(&mut tx, org_id).await?;
    core_db::orgs::seed_default_sla_policy_tx(&mut tx, org_id).await?;

    core_db::users::create_user_tx(&mut tx, user_id, org_id, &req.email,
        &req.display_name, &password_hash, req.timezone.as_deref().unwrap_or("UTC")).await?;

    let roles = core_db::users::roles_for_user_in_tx(&mut tx, user_id, org_id).await?;
    let owner_role = roles.iter()
        .find(|r| matches!(r.builtin, Some(core_domain::policy::BuiltinRole::Owner)))
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("owner role missing after seed")))?;
    core_db::users::assign_role_tx(&mut tx, user_id, owner_role.id).await?;

    // Registration itself is the org's first audit event.
    core_db::audit::insert_tx(&mut tx, &AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "auth.register".into(),
        entity_type: Some("org".into()),
        entity_id: Some(org_id.to_string()),
        detail: serde_json::json!({ "email": req.email }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    }).await?;

    tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

    let tokens = issue_tokens(&state, user_id, org_id, None).await?;
    Ok(Json(tokens))
}

/// POST /api/auth/login — resolves the org by email (SECURITY DEFINER
/// bridge), verifies the argon2 hash under RLS scope, issues tokens.
#[utoipa::path(post, path = "/api/auth/login", tag = "auth",
    request_body = LoginRequest, responses(
        (status = 200, body = TokenPair),
        (status = 401, description = "bad credentials")
))]
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<TokenPair>> {
    // Narrow RLS bridge: (user_id, org_id) only — no hashes cross this line.
    let resolved: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT user_id, org_id FROM app_resolve_login($1)",
    )
    .bind(&req.email)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let (user_id, org_id) = resolved.ok_or_else(|| {
        // Uniform error: never reveal whether the email exists.
        ApiError::Unauthorized("invalid credentials".into())
    })?;

    let mut tx = state.pool.begin().await.map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, &req.email)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid credentials".into()))?;

    if !password::verify(&req.password, &creds.password_hash) {
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }
    tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

    let tokens = issue_tokens(&state, user_id, org_id, None).await?;

    // Audit the login (best-effort: never block auth on audit failure).
    let _ = core_db::audit::insert(&state.pool, &AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "auth.login".into(),
        entity_type: Some("user".into()),
        entity_id: Some(user_id.to_string()),
        detail: serde_json::json!({}),
        ip: None,
        occurred_at: chrono::Utc::now(),
    }).await;

    Ok(Json(tokens))
}

/// POST /api/auth/refresh — rotates the refresh token (old one dies with the
/// transaction; replay of a rotated token fails).
#[utoipa::path(post, path = "/api/auth/refresh", tag = "auth",
    request_body = RefreshRequest, responses((status = 200, body = TokenPair)))]
pub async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> ApiResult<Json<TokenPair>> {
    let presented_hash = jwt::hash_refresh_token(&req.refresh_token);
    let mut tx = state.pool.begin().await.map_err(|e| ApiError::Internal(e.into()))?;
    let session = core_db::users::find_active_session(&mut *tx, req.session_id, &presented_hash)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid refresh token".into()))?;

    let user = core_db::users::get_user_in_tx(&mut tx, session.user_id, session.user_id).await?;
    // get_user needs org scope; sessions store the user, resolve org via user row.
    let _ = user;

    let new_refresh = jwt::new_refresh_token();
    let new_hash = jwt::hash_refresh_token(&new_refresh);
    let new_expiry = chrono::Utc::now() + chrono::Duration::seconds(state.config.refresh_ttl_secs);
    core_db::users::rotate_session(&mut *tx, session.id, &new_hash, new_expiry).await?;
    tx.commit().await.map_err(|e| ApiError::Internal(e.into()))?;

    // Resolve the user's org for the new access token.
    let org_id: Uuid = sqlx::query_scalar("SELECT org_id FROM users WHERE id = $1")
        .bind(session.user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let access = jwt::issue_access_token(&state.config, session.user_id, org_id, session.id)
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(TokenPair {
        access_token: access,
        refresh_token: new_refresh,
        session_id: session.id,
        expires_in: state.config.access_ttl_secs,
    }))
}

/// POST /api/auth/logout — revokes the refresh session.
#[utoipa::path(post, path = "/api/auth/logout", tag = "auth", security(("bearer" = [])),
    request_body = RefreshRequest, responses((status = 200)))]
pub async fn logout(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> ApiResult<axum::http::StatusCode> {
    core_db::users::revoke_session(&state.pool, req.session_id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// GET /api/me — the authenticated principal with resolved permissions.
#[utoipa::path(get, path = "/api/me", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = MeResponse)))]
pub async fn me(extensions: Extensions) -> ApiResult<Json<MeResponse>> {
    let user: AuthedUser = crate::auth::middleware::authed(&extensions)?;
    Ok(Json(MeResponse {
        user_id: principal(&extensions)?.user.id,
        org_id: principal(&extensions)?.user.org_id,
        email: principal(&extensions)?.user.email.clone(),
        display_name: principal(&extensions)?.user.display_name.clone(),
        timezone: principal(&extensions)?.user.timezone.clone(),
        permissions: principal(&extensions)?
            .permissions
            .iter()
            .map(|p| p.key())
            .collect(),
    }))
}

fn principal(extensions: &Extensions) -> ApiResult<&core_domain::entities::Principal> {
    extensions
        .get::<AuthedUser>()
        .map(|a| &a.principal)
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))
}

/// Issues an access token + a fresh refresh session.
async fn issue_tokens(
    state: &AppState,
    user_id: Uuid,
    org_id: Uuid,
    user_agent: Option<&str>,
) -> ApiResult<TokenPair> {
    let session_id = Uuid::now_v7();
    let refresh = jwt::new_refresh_token();
    let refresh_hash = jwt::hash_refresh_token(&refresh);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(state.config.refresh_ttl_secs);
    core_db::users::create_session(&state.pool, session_id, user_id, &refresh_hash, user_agent, expires_at)
        .await?;
    let access = jwt::issue_access_token(state.config_access(), user_id, org_id, session_id)
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(TokenPair {
        access_token: access,
        refresh_token: refresh,
        session_id,
        expires_in: state.config.access_ttl_secs,
    })
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct MeResponse {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub timezone: String,
    pub permissions: Vec<String>,
}

// Silence unused-import lint for AccessClaims (used by jwt module re-export).
#[allow(unused)]
fn _claims_type_check(_c: AccessClaims) {}
