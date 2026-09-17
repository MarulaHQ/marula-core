//! Auth HTTP handlers: register (org bootstrap), login (with the MFA
//! bridge), refresh (with chain reuse detection), logout, sessions, me.
//! All auth events are audited (§6).

use crate::auth::cookies;
use crate::auth::hardening::{enforce_gate, record_failure, record_success, GateSubject};
use crate::auth::{jwt, password, AccessClaims, AuthedUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::openapi::TokenPair;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header, Extensions, HeaderMap, HeaderName, HeaderValue},
    response::IntoResponse,
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use serde::{Deserialize, Serialize};
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
    /// Optional: when omitted, the refresh pair rides the httpOnly
    /// session cookie (browser clients); bearer-mode clients keep
    /// sending both fields exactly as before.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

/// Login outcome: tokens directly, or a short-lived MFA bridge token when
/// any second factor is enrolled. Untagged so legacy clients that ignore
/// `mfa_required` keep parsing the plain token pair they always got.
#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum LoginResponse {
    Tokens(TokenPair),
    MfaRequired {
        mfa_required: bool,
        mfa_token: String,
        /// Second factors able to satisfy this login.
        methods: Vec<String>,
    },
}

pub(crate) fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.len() < 10 {
        return Err(ApiError::Validation(
            "password must be at least 10 characters".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_email(email: &str) -> ApiResult<()> {
    let ok = email.contains('@') && email.contains('.') && !email.contains(' ') && email.len() >= 5;
    if ok {
        Ok(())
    } else {
        Err(ApiError::Validation("invalid email".into()))
    }
}

/// Best-effort audit write — never blocks an auth response on audit failure
/// for events OUTSIDE a transaction; in-tx callers use core_db::audit::insert.
pub(crate) async fn audit_best_effort(
    state: &AppState,
    org_id: Uuid,
    actor: Actor,
    action: &str,
    entity_type: Option<&str>,
    entity_id: Option<String>,
    detail: serde_json::Value,
) {
    let event = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor,
        action: action.into(),
        entity_type: entity_type.map(String::from),
        entity_id,
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    let _ = core_db::audit::insert(&state.pool, &event).await;
}

/// Maps an `insert_org` failure onto the typed envelope. An
/// `orgs_slug_key` unique violation becomes reason `org_slug_taken` with
/// a suggested available slug (first free `-2`, `-3`, … suffix, probed on
/// the POOL — the write tx is aborted by the constraint error and cannot
/// answer reads). Any other failure passes through the registry mapping
/// unchanged (e.g. duplicate owner email → `user_email_taken`).
async fn slug_conflict_with_suggestion(
    e: core_db::DbError,
    slug: &str,
    pool: &sqlx::PgPool,
) -> ApiError {
    let is_slug_key = matches!(
        &e,
        core_db::DbError::Sql(sqlx::Error::Database(db))
            if db.constraint() == Some("orgs_slug_key")
    );
    if !is_slug_key {
        return ApiError::from(e);
    }
    let mut suggestion = None;
    for n in 2..10 {
        let candidate = format!("{slug}-{n}");
        let free = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM orgs WHERE slug = $1")
            .bind(candidate.clone())
            .fetch_one(pool)
            .await;
        if matches!(free, Ok(0)) {
            suggestion = Some(candidate);
            break;
        }
    }
    tracing::info!(%slug, suggestion = ?suggestion, "org slug collision; returning typed org_slug_taken");
    ApiError::SlugTaken { suggestion }
}

/// POST /api/auth/register — creates the org, its default roles/board/SLA
/// policy, and the first user as Owner. One transaction: a half-created org
/// can never exist.
#[utoipa::path(operation_id = "handlers_register", post, path = "/api/auth/register", tag = "auth",
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
        && req
            .org_slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !slug_ok {
        return Err(ApiError::Validation(
            "org_slug must be lowercase alphanumeric or '-'".into(),
        ));
    }

    let org_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let password_hash = password::hash(&req.password)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("password hashing failed: {e}")))?;

    // The registration transaction runs as the app role with RLS. We bind
    // the tx to the NEW org id first, so the org row insert satisfies its
    // own isolation policy (id = app.org_id) and everything downstream
    // inherits the scope.
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;

    if let Err(e) = core_db::orgs::insert_org(
        &mut *tx,
        org_id,
        &req.org_name,
        &req.org_slug,
        req.ticket_prefix.as_deref().unwrap_or("MER"),
    )
    .await
    {
        return Err(slug_conflict_with_suggestion(e, &req.org_slug, &state.pool).await);
    }
    let role_ids = core_db::orgs::seed_default_roles(&mut tx, org_id).await?;
    core_db::orgs::seed_default_board(&mut tx, org_id).await?;
    core_db::orgs::seed_default_sla_policy(&mut *tx, org_id).await?;
    // Standard type/category/subcategory taxonomy (Wave 11 lane 3) — the
    // defaults-first baseline so pickers work with zero configuration.
    core_db::orgs::seed_default_taxonomy(&mut tx, org_id).await?;
    core_db::document_layouts::seed_default_document_layouts(&mut tx, org_id).await?;
    core_db::templates::seed_default_templates(&mut tx, org_id).await?;

    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        &req.email,
        &req.display_name,
        &password_hash,
        req.timezone.as_deref().unwrap_or("UTC"),
    )
    .await?;

    // The 5 immutable secret-category defaults (ADR 0006/0004 — the
    // same tree migration 0078 backfilled for orgs existing pre-0078),
    // seeded AFTER the owner user exists so the `created_by` FK lands;
    // the seed also seats the owner/admin restricted-chain grants (the
    // runtime twin of migration 0083's backfill — never self-lockout).
    core_db::secret_vault::seed_default_secret_folders(&mut tx, org_id, user_id).await?;

    // The registering user becomes the org's first Owner.
    let owner_role_id = role_ids
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("owner role missing after seed")))?;
    core_db::users::assign_role(&mut *tx, user_id, *owner_role_id).await?;

    // Registration itself is the org's first audit event.
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "auth.register".into(),
            entity_type: Some("org".into()),
            entity_id: Some(org_id.to_string()),
            detail: serde_json::json!({ "email": req.email }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;

    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    // Wave 19 lane 5: verification mail on register (the fresh owner is
    // born UNVERIFIED — deny-by-default even for org creators; mail rides
    // core-mail, best-effort so registration never blocks on the queue).
    {
        let outcome = super::recovery::email_verification_request(
            State(state.clone()),
            axum::http::HeaderMap::new(),
            Json(super::recovery::EmailVerifyRequest { email: req.email }),
        )
        .await;
        if let Err(e) = outcome {
            tracing::warn!(error = %e, "post-register verification mail enqueue failed");
        }
    }

    let tokens = issue_tokens(&state, user_id, org_id, None).await?;
    Ok(Json(tokens))
}

/// POST /api/auth/login — resolves the org by email (SECURITY DEFINER
/// bridge), verifies the argon2 hash under RLS scope. When any second
/// factor is enrolled, returns a short-lived `mfa_pending` bridge instead
/// of tokens; POST /api/auth/mfa/verify completes it.
#[utoipa::path(operation_id = "handlers_login", post, path = "/api/auth/login", tag = "auth",
    request_body = LoginRequest, responses(
        (status = 200, body = LoginResponse),
        (status = 401, description = "bad credentials")
))]
pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<axum::response::Response> {
    // Durable per-IP + per-actor deny-by-default gate BEFORE any
    // resolution (Wave 19 lane 5): an engaged lock answers with the SAME
    // envelope shape as a wrong password — no lock-state enumeration.
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::PASSWORD_LOGIN,
        &headers,
        GateSubject::Email(&req.email),
        true,
    )
    .await?;
    // Narrow RLS bridge: (user_id, org_id) only — no hashes cross this line.
    let resolved: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT user_id, org_id FROM app_resolve_login($1)")
            .bind(&req.email)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

    let (user_id, org_id) = match resolved {
        Some(pair) => pair,
        None => {
            // Uniform error: never reveal whether the email exists.
            return Err(ApiError::Unauthorized("invalid credentials".into()));
        }
    };

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    let creds = core_db::users::find_credentials_by_email(&mut *tx, &req.email)
        .await
        .map_err(|_| {
            audit_failed_login(&state, org_id);
            ApiError::Unauthorized("invalid credentials".into())
        })?;

    if !password::verify(&req.password, &creds.password_hash) {
        audit_failed_login(&state, org_id);
        let _ = record_failure(
            &state,
            crate::auth::hardening::kinds::PASSWORD_LOGIN,
            &headers,
            GateSubject::Email(&req.email),
        )
        .await;
        return Err(ApiError::Unauthorized("invalid credentials".into()));
    }

    // Org policy: email verification required (Wave 19 lane 5; deny-by-
    // default — off until an admin opts in, and then UNVERIFIED accounts
    // are denied with a typed reason that never mixes with bad-creds).
    if core_db::orgs::require_email_verification(&mut *tx, org_id).await?
        && !core_db::hardening::user_email_verified(&mut *tx, user_id).await?
    {
        return Err(ApiError::AuthDenied {
            reason: "email_unverified",
            message: "this workspace requires email verification; check your inbox",
        });
    }

    let caps = core_db::mfa::mfa_capabilities(&mut *tx, user_id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    if caps.any() {
        let mfa_token = jwt::issue_mfa_pending_token(&state.config, user_id, org_id)
            .map_err(|e| ApiError::Internal(e.into()))?;
        audit_best_effort(
            &state,
            org_id,
            Actor::Human(user_id),
            "auth.mfa_challenge_issued",
            Some("user"),
            Some(user_id.to_string()),
            serde_json::json!({ "methods": caps.methods() }),
        )
        .await;
        return Ok(auth_json_response(

            LoginResponse::MfaRequired {
                mfa_required: true,
                mfa_token,
                methods: caps.methods().into_iter().map(String::from).collect(),
            },
            Vec::new(),
        ));
    }

    let tokens = issue_tokens(&state, user_id, org_id, None).await?;

    // Durable throttle counters cleared after a clean password check.
    let _ = record_success(
        &state,
        crate::auth::hardening::kinds::PASSWORD_LOGIN,
        &headers,
        GateSubject::Email(&req.email),
    )
    .await;

    // Audit the successful password-only login (best-effort).
    audit_best_effort(
        &state,
        org_id,
        Actor::Human(user_id),
        "auth.login",
        Some("user"),
        Some(user_id.to_string()),
        serde_json::json!({ "method": "password", "amr": "pw" }),
    )
    .await;

    let cookies_head = auth_cookie_headers(&state, tokens.session_id, &tokens.refresh_token);
    Ok(auth_json_response(LoginResponse::Tokens(tokens), cookies_head))
}

fn audit_failed_login(state: &AppState, org_id: Uuid) {
    // Fire-and-forget; uniform shape without revealing which half failed.
    let state = state.clone();
    tokio::spawn(async move {
        let _ = core_db::audit::insert(
            &state.pool,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::System,
                action: "auth.login_failed".into(),
                entity_type: Some("user".into()),
                entity_id: None,
                detail: serde_json::json!({}),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await;
    });
}

/// Shared shapers: JSON body + arbitrary Set-Cookie heads on one response.
pub(crate) fn auth_json_response<B: serde::Serialize>(
    body: B,
    cookie_headers: Vec<(HeaderName, String)>,
) -> axum::response::Response {
    let mut response = Json(body).into_response();
    for (name, value) in cookie_headers {
        add_header(&mut response, (name, value));
    }
    response
}

fn add_header(response: &mut axum::response::Response, header: (HeaderName, String)) {
    let (name, value) = header;
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().append(name, value);
    }
}

/// The session-cookie trio set on every token-issuing auth response:
/// the (session id, refresh secret) pair in one httpOnly cookie + the
/// double-submit csrf value. Secure is derived from the API base URL.
pub(crate) fn auth_cookie_headers(
    state: &AppState,
    session_id: Uuid,
    refresh: &str,
) -> Vec<(HeaderName, String)> {
    let secure = state.config.public_base_url.starts_with("https");
    let csrf = cookies::new_csrf_token();
    vec![
        (
            header::SET_COOKIE,
            cookies::session_cookie(
                secure,
                &format!("{session_id}.{refresh}"),
                state.config.refresh_ttl_secs,
            ),
        ),
        (header::SET_COOKIE, cookies::csrf_cookie(secure, &csrf)),
    ]
}

/// Issues tokens AND the cookie trio in one call (magic-link confirm
/// rides the same shape as password login).
pub(crate) async fn issue_tokens_with_cookies(
    state: &AppState,
    user_id: Uuid,
    org_id: Uuid,
    user_agent: Option<&str>,
) -> ApiResult<(TokenPair, Vec<(HeaderName, String)>)> {
    let tokens = issue_tokens(state, user_id, org_id, user_agent).await?;
    let head = auth_cookie_headers(state, tokens.session_id, &tokens.refresh_token);
    Ok((tokens, head))
}

/// Token-pair responses issued after any second-factor finish share the
/// same session-cookie trio + shaper as the password login (§A2 parity).
pub(crate) fn mfa_cookie_response(
    state: &AppState,
    tokens: TokenPair,
) -> axum::response::Response {
    let head = auth_cookie_headers(state, tokens.session_id, &tokens.refresh_token);
    auth_json_response(tokens, head)
}

/// POST /api/auth/refresh — rotates the refresh secret as a NEW chain node.
/// Presenting an already-consumed (rotated/revoked/expired) token proves
/// replay: the WHOLE chain is revoked and the event is audited.
#[utoipa::path(operation_id = "handlers_refresh", post, path = "/api/auth/refresh", tag = "auth",
    request_body = RefreshRequest, responses((status = 200, body = TokenPair)))]
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Option<Json<RefreshRequest>>,
) -> ApiResult<axum::response::Response> {
    // CSRF double-submit for the cookie-only path (bearer callers carry
    // no ambient credential and pass by construction).
    if !cookies::double_submit_ok(&headers) {
        return Err(ApiError::Forbidden("csrf check failed".into()));
    }
    // Durable per-IP throttle pre-check, typed 429 — proves nothing about
    // any account (no user identity exists yet), so the code cannot
    // fingerprint lockout state.
    enforce_gate(
        &state,
        crate::auth::hardening::kinds::REFRESH,
        &headers,
        GateSubject::IpOnly,
        false,
    )
    .await?;

    // Cookie fallback: browser clients may POST with no (or empty) body.
    let refresh_token: Option<(Uuid, String)> = match req {
        Some(Json(RefreshRequest { refresh_token: Some(t), session_id: Some(sid) })) => {
            Some((sid, t))
        }
        _ => cookies::session_refresh_pair_from(&headers),
    };
    let Some((sid, secret)) = refresh_token else {
        // Consistent pre-throttle answer: need a body or a cookie.
        return Err(ApiError::Unauthorized("invalid refresh token".into()));
    };

    let presented_hash = jwt::hash_refresh_token(&secret);

    // SECURITY DEFINER bridge resolves even rotated rows — seeing one is
    // exactly how reuse is detected before org scope exists.
    let resolution = core_db::users::resolve_refresh(&state.pool, sid, &presented_hash)
        .await
        .map_err(ApiError::from)?;

    let resolution = match resolution {
        Some(r) => r,
        None => {
            let _ = record_failure(
                &state,
                crate::auth::hardening::kinds::REFRESH,
                &headers,
                GateSubject::IpOnly,
            )
            .await;
            return Err(ApiError::Unauthorized("invalid refresh token".into()));
        }
    };

    if !resolution.active {
        let mut tx = state
            .pool
            .begin()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        core_db::set_org(&mut tx, resolution.org_id).await?;
        let killed = core_db::users::revoke_chain(&mut *tx, resolution.chain_id).await?;
        core_db::audit::insert(
            &mut *tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: resolution.org_id,
                actor: Actor::System,
                action: "auth.refresh_reuse_detected".into(),
                entity_type: Some("auth_session".into()),
                entity_id: Some(resolution.chain_id.to_string()),
                detail: serde_json::json!({
                    "chain_id": resolution.chain_id,
                    "sessions_revoked": killed,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let _ = record_failure(
            &state,
            crate::auth::hardening::kinds::REFRESH,
            &headers,
            GateSubject::IpOnly,
        )
        .await;
        return Err(ApiError::Unauthorized("invalid refresh token".into()));
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, resolution.org_id).await?;
    let parent = core_db::users::find_session_any(&mut *tx, sid, &presented_hash)
        .await
        .map_err(|_| ApiError::Unauthorized("invalid refresh token".into()))?;
    if parent.revoked_at.is_some() || parent.expires_at <= chrono::Utc::now() {
        // Lost a race against a concurrent replay — same treatment.
        core_db::users::revoke_chain(&mut *tx, resolution.chain_id).await?;
        tx.commit()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        return Err(ApiError::Unauthorized("invalid refresh token".into()));
    }

    let new_refresh = jwt::new_refresh_token();
    let new_hash = jwt::hash_refresh_token(&new_refresh);
    let new_session = Uuid::now_v7();
    let new_expiry = chrono::Utc::now() + chrono::Duration::seconds(state.config.refresh_ttl_secs);
    core_db::users::rotate_session_chain(&mut *tx, &parent, new_session, &new_hash, new_expiry)
        .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let access = jwt::issue_access_token(
        &state.config,
        parent.user_id,
        resolution.org_id,
        new_session,
    )
    .map_err(|e| ApiError::Internal(e.into()))?;
    let cookies_head =
        auth_cookie_headers(&state, new_session, &new_refresh);
    Ok(auth_json_response(
        TokenPair {
            access_token: access,
            refresh_token: new_refresh,
            session_id: new_session,
            expires_in: state.config.access_ttl_secs,
        },
        cookies_head,
    ))
}

/// POST /api/auth/logout — revokes the refresh session and audits the event.
#[utoipa::path(operation_id = "handlers_logout", post, path = "/api/auth/logout", tag = "auth",
    request_body = RefreshRequest, responses((status = 200)))]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: Option<Json<RefreshRequest>>,
) -> ApiResult<axum::response::Response> {
    // Cookie fallback: browser clients may POST with no (or empty) body
    // — the httpOnly session cookie carries (session, secret).
    let refresh_token = match req {
        Some(Json(RefreshRequest { refresh_token: Some(t), session_id: Some(sid) })) => {
            Some((sid, t))
        }
        _ => cookies::session_refresh_pair_from(&headers),
    };
    if !cookies::double_submit_ok(&headers) {
        return Err(ApiError::Forbidden("csrf check failed".into()));
    }
    if let Some((sid, secret)) = refresh_token {
        let presented_hash = jwt::hash_refresh_token(&secret);
        if let Ok(Some(resolution)) =
            core_db::users::resolve_refresh(&state.pool, sid, &presented_hash).await
        {
            let _ = core_db::users::revoke_session(&state.pool, sid).await;
            audit_best_effort(
                &state,
                resolution.org_id,
                Actor::Human(resolution.user_id),
                "auth.logout",
                Some("auth_session"),
                Some(sid.to_string()),
                serde_json::json!({}),
            )
            .await;
        } else {
            let _ = core_db::users::revoke_session(&state.pool, sid).await;
        }
    }
    let secure = state.config.public_base_url.starts_with("https");
    let clearing = vec![
        (header::SET_COOKIE, cookies::clear_session_cookie(secure)),
        (header::SET_COOKIE, cookies::clear_csrf_cookie(secure)),
    ];
    Ok(clearing.into_iter().fold(
        axum::http::StatusCode::NO_CONTENT.into_response(),
        |mut response, hdr| {
            add_header(&mut response, hdr);
            response
        },
    ))
}

/// GET /api/auth/sessions — the caller's live refresh sessions (security
/// settings page).
#[utoipa::path(operation_id = "auth_sessions_list", get, path = "/api/auth/sessions", tag = "auth",
    security(("bearer" = [])), responses((status = 200, body = [SessionInfo])))]
pub async fn list_sessions(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<SessionInfo>>> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, authed.principal.user.org_id).await?;
    let sessions = core_db::users::list_user_sessions(&mut *tx, authed.principal.user.id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let current_sid = authed.session_id;
    Ok(Json(
        sessions
            .iter()
            .map(|s| SessionInfo {
                id: s.id,
                user_agent: s.user_agent.clone(),
                created_at: s.created_at,
                expires_at: s.expires_at,
                current: current_sid == Some(s.id),
                chain_root: s.rotated_from.is_none(),
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionInfo {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// True for the session making the request.
    pub current: bool,
    pub chain_root: bool,
}

/// POST /api/auth/sessions/{id}/revoke — kills one of the caller's own
/// sessions (sign-out-everywhere-else primitive).
#[utoipa::path(operation_id = "auth_session_revoke", post, path = "/api/auth/sessions/{id}/revoke", tag = "auth",
    security(("bearer" = [])), params(("id" = uuid::Uuid, Path)),
    responses((status = 204)))]
pub async fn revoke_session(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(session_id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = crate::auth::middleware::authed::<AuthedUser>(&extensions)?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, authed.principal.user.org_id).await?;
    // RLS guarantees ownership: sessions of other users are invisible.
    core_db::users::revoke_session(&mut *tx, session_id).await?;
    core_db::audit::insert(
        &mut *tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: authed.principal.user.org_id,
            actor: Actor::Human(authed.principal.user.id),
            action: "auth.session_revoked".into(),
            entity_type: Some("auth_session".into()),
            entity_id: Some(session_id.to_string()),
            detail: serde_json::json!({}),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// GET /api/me — the authenticated principal with resolved permissions.
#[utoipa::path(operation_id = "handlers_me", get, path = "/api/me", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = MeResponse)))]
pub async fn me(extensions: Extensions) -> ApiResult<Json<MeResponse>> {
    // Validates the authenticated principal is present (middleware-inserted).
    let _: AuthedUser = crate::auth::middleware::authed(&extensions)?;
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

/// Issues an access token + a fresh refresh-session chain root.
pub(crate) async fn issue_tokens(
    state: &AppState,
    user_id: Uuid,
    org_id: Uuid,
    user_agent: Option<&str>,
) -> ApiResult<TokenPair> {
    let session_id = Uuid::now_v7();
    let refresh = jwt::new_refresh_token();
    let refresh_hash = jwt::hash_refresh_token(&refresh);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(state.config.refresh_ttl_secs);
    core_db::users::create_session(
        &state.pool,
        session_id,
        user_id,
        &refresh_hash,
        user_agent,
        expires_at,
    )
    .await?;
    let access = jwt::issue_access_token(&state.config, user_id, org_id, session_id)
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(TokenPair {
        access_token: access,
        refresh_token: refresh,
        session_id,
        expires_in: state.config.access_ttl_secs,
    })
}

/// The federated-login MFA gate (Wave 13 lane 7, invariant ID4): the org
/// MFA policy — any enrolled second factor must be satisfied before
/// tokens are issued — applies REGARDLESS of identity source. Password
/// login and every federation bundle route their token issuance through
/// this same gate; `None` means tokens may be issued directly.
pub(crate) async fn federated_mfa_gate(
    state: &AppState,
    user_id: Uuid,
    org_id: Uuid,
) -> ApiResult<Option<(String, Vec<String>)>> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id).await?;
    let caps = core_db::mfa::mfa_capabilities(&mut *tx, user_id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if !caps.any() {
        return Ok(None);
    }
    let mfa_token = jwt::issue_mfa_pending_token(&state.config, user_id, org_id)
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Some((
        mfa_token,
        caps.methods().into_iter().map(String::from).collect(),
    )))
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
