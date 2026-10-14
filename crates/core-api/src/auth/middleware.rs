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
use axum::extract::connect_info::ConnectInfo;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{BuiltinRole, Permission};
use std::net::SocketAddr;

/// Header carrying the SHA-256 fingerprint of the client certificate the
/// edge proxy verified on the (m)TLS connection and forwarded.
const CLIENT_CERT_FP_HEADER: &str = "x-marula-client-cert";

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

/// Axum middleware: `Authorization: Bearer <jwt>` for humans, a scoped
/// machine path via `x-api-key` (TASKLIST 2.15), and client-certificate
/// authentication for service accounts (Wave 47 A9).
///
/// The API-key branch is deliberately NARROW: only read-only public data
/// prefixes accept keys, scopes are enforced against plugin capability
/// names, and the synthesized principal is read-only by construction
/// (`is_machine` + empty roles). Self-scoped surfaces (notifications,
/// timers) reject keys outright — a key id is not a user id.
///
/// The client-certificate branch runs FIRST and is the ONE machine path
/// that is NOT narrowed to the API-key prefix allowlist: it is a distinct
/// principal type — an org-enrolled service account whose authority is
/// EXACTLY its write-time permission grants (resolved through the standard
/// policy matrix), so a cert account is authorized by its grants, not by a
/// URL prefix. For attribution it reuses the same `via_api_key` machine
/// flag an API-key principal sets, so pre-existing import/connector paths
/// label the machine actor `api-key:<id>` / `created_by_kind = "api_key"`
/// while the service-account module's own audit rows label it
/// `service-account:<id>`. That label split is intentional and recorded in
/// design/qa-p47-client-cert-auth/LANDING.md (a dedicated machine-kind
/// discriminator is a follow-up).
pub async fn require_auth(
    State(state): State<crate::state::AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Client-certificate authentication runs FIRST (before x-api-key and
    // JWT) per Wave 47 A9. The cert header value and peer are lifted into
    // OWNED values by `cert_presentation` BEFORE any `.await` —
    // `axum::body::Body` is not `Sync`, so a live `&Request` borrow held
    // across an await would make this future `!Send`.
    if let Some(presentation) = cert_presentation(&request) {
        if let Some(authed) = client_cert_identity(&state, presentation).await? {
            request.extensions_mut().insert(authed);
            return Ok(next.run(request).await);
        }
    }

    if let Some(raw_key) = request
        .headers()
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        let path = request.uri().path();
        // Both mounts promise identical auth behavior (routes/mod.rs): the
        // canonical /api/v1 generation matches the allowlist through its
        // legacy-form twin so a scoped key is accepted identically on
        // either mount. Legacy paths match as-is.
        let candidate = path
            .strip_prefix("/api/v1")
            .map(|rest| format!("/api{rest}"))
            .unwrap_or_else(|| path.to_string());
        let allowed = API_KEY_PATHS.iter().any(|p| {
            candidate == *p
                || candidate
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

    // SSE browser clients (native EventSource) cannot set request
    // headers, so the events stream accepts `?token=` as a fallback —
    // the same posture as the /ws upgrade (crates/realtime ws.rs).
    // Scoped to the stream path only: every other endpoint keeps
    // header-only auth so tokens never leak into URLs/ logs elsewhere.
    let query_token = if request.uri().path() == "/api/events/stream" {
        request
            .uri()
            .query()
            .and_then(|q| {
                q.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    (k == "token").then(|| v.to_string())
                })
            })
            .filter(|v| !v.is_empty())
    } else {
        None
    };
    let token = query_token
        .or_else(|| {
            request
                .headers()
                .get(AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|v| v.to_string())
        })
        .ok_or_else(|| ApiError::Unauthorized("missing bearer token".into()))?;

    let claims = crate::auth::jwt::verify_access_token(&state.config, &token)
        .ok_or_else(|| ApiError::Unauthorized("invalid or expired token".into()))?;

    // Load user + roles fresh per request. Two cheap indexed queries; the
    // correctness win (instant revocation) outweighs the cost at v1 scale.
    // The reads run inside an org-bound transaction: `users`/`user_roles`
    // are RLS-scoped by `app.org_id`, and the pool connection carries no
    // scope of its own — an unscoped read hides every row (fresh
    // connection) or dies casting "" to uuid (post-scoped connection).
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
    core_db::set_org_and_user(&mut tx, claims.org, claims.sub)
        .await
        .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
    let user = core_db::users::get_user(&mut *tx, claims.org, claims.sub)
        .await
        .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
    if !user.is_active {
        return Err(ApiError::Unauthorized("account disabled".into()));
    }
    let roles = core_db::users::roles_for_user(&mut *tx, claims.sub, claims.org).await?;
    tx.commit()
        .await
        .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
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
/// by scope checks on top of this prefix allowance. `/api/agent-proposals`
/// is the IF8 machine flow: agents file proposals (never direct writes)
/// through keys, gated by the capability registry inside the handlers.
/// `/api/migration` is the D3 connector surface (push + pull), scoped by
/// `migration:*` keys.
/// Canonical `/api/v1/...` requests match through their legacy-form twin.
const API_KEY_PATHS: &[&str] = &[
    "/api/tickets",
    "/api/clients",
    "/api/imports",
    // The IF8 machine flow accepts keys on BOTH mounts (legacy + v1).
    "/api/agent-proposals",
    "/api/v1/agent-proposals",
    "/api/migration",
    // The dispatch connector (Wave 9 lane 9): scoped schedule reads +
    // field-service status pushes; handlers gate on `dispatch:*` scopes.
    "/api/dispatch-connector",
    // The CI feeders (Wave 12 lane 8, ADR 0003 CI7): RMM/NMS/docs feeders
    // write through /api/config-items/feed-upsert with `clients:write`
    // keys — the handler's Clients-matrix permission check stays the
    // gate (a read-scoped key still gets typed 403s on writes).
    "/api/config-items",
];

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
            // Migration connector (TASKLIST D3): deliberately broad data
            // grants — a migration tool writes whole entity families
            // through the D2 engine. The engine's gate checks the Clients
            // matrix (the import framework's machine contract), so the
            // scope resolves onto exactly that.
            // Dispatch connector (TASKLIST Phase 3, Wave 9 lane 9):
            // read = schedule visibility; write = status pushes ride the
            // Scheduling module's create+edit matrix like booking writes.
            "dispatch:read" => push(Module::Scheduling, &[Action::View]),
            "dispatch:write" => push(
                Module::Scheduling,
                &[Action::View, Action::Create, Action::Edit],
            ),
            "migration:read" => push(Module::Clients, &[Action::View]),
            "migration:write" => push(
                Module::Clients,
                &[Action::View, Action::Create, Action::Edit],
            ),
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

/// The client-certificate presentation lifted into OWNED values so no
/// borrow of `Request` (whose body type is `!Sync`) is held across an
/// `.await`. Extracted synchronously from the request before any await.
struct CertPresentation {
    /// Raw forwarded fingerprint header value (owned).
    raw: String,
    /// The connecting peer IP, if `ConnectInfo` is present.
    peer: Option<std::net::IpAddr>,
}

/// Synchronously lifts the cert header value and peer out of the request
/// into an owned [`CertPresentation`]. Returns `None` when no cert header
/// is presented (⇒ not a cert-auth request ⇒ fall through unchanged).
fn cert_presentation(request: &Request) -> Option<CertPresentation> {
    let raw = request
        .headers()
        .get(CLIENT_CERT_FP_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    Some(CertPresentation { raw, peer })
}

/// Resolves the client-certificate presentation to an authenticated
/// service-account principal, or `None` when the request is not a
/// certificate-authentication attempt (no header) or the forwarding peer is
/// untrusted (the header is ignored so a forged fingerprint cannot
/// authenticate). Takes OWNED data so the future is `Send`.
async fn client_cert_identity(
    state: &crate::state::AppState,
    presentation: CertPresentation,
) -> ApiResult<Option<AuthedUser>> {
    // Trust gate: only a peer inside the operator's trusted-proxy set may
    // assert the client's certificate. No `ConnectInfo` (raw `oneshot`
    // tests) means no trustworthy peer, so the assertion is ignored rather
    // than believed. The predicate is the trusted-proxy lane's shared one
    // (`crate::client_ip::TrustedProxies::is_trusted`), so the forwarded
    // client-cert trust and the real-client-IP trust cannot drift apart.
    let trusted = presentation
        .peer
        .is_some_and(|ip| state.config.trusted_proxies.is_trusted(ip));
    if !trusted {
        tracing::debug!("client-certificate header from an untrusted peer ignored");
        return Ok(None);
    }

    // Normalize to lowercase hex SHA-256; a well-formed fingerprint from a
    // trusted peer is REQUIRED (a malformed value is a 401, not a silent
    // fall-through — the proxy verified a cert, not this string). One shared
    // normalizer owns enroll-vs-auth parity (routes::service_accounts).
    let Some(fingerprint) = crate::routes::service_accounts::normalize_fingerprint(&presentation.raw)
    else {
        return Err(ApiError::Unauthorized(
            "malformed client certificate fingerprint".into(),
        ));
    };

    // One transaction carries lookup → touch → audit, so a request is a
    // single pool checkout rather than three. Lookup and touch ride their
    // SECURITY DEFINER seams (no org scope needed); the audit row is written
    // under the resolved org's RLS. Any early return drops the tx (rollback),
    // so a rejected fingerprint writes nothing.
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(%error, "cert-auth: could not acquire a connection");
            return Err(ApiError::Unauthorized("client certificate rejected".into()));
        }
    };

    let account = match core_db::service_accounts::find_active_service_account_by_fingerprint(
        &mut *tx,
        &fingerprint,
    )
    .await
    {
        Ok(Some(account)) => account,
        Ok(None) => {
            return Err(ApiError::Unauthorized(
                "client certificate not enrolled".into(),
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "cert-auth: lookup failed");
            return Err(ApiError::Unauthorized("client certificate rejected".into()));
        }
    };

    // Defense-in-depth: the SQL lookup already filters revoked/suspended/
    // expired rows; re-assert against the row so a schema drift cannot
    // silently widen acceptance.
    if !account.is_authenticatable(chrono::Utc::now()) {
        return Err(ApiError::Unauthorized(
            "client certificate is not active".into(),
        ));
    }

    let permissions = crate::routes::service_accounts::resolve_scope_permissions(&account.scopes);
    if permissions.is_empty() {
        return Err(ApiError::Forbidden(
            "service account carries no usable scopes".into(),
        ));
    }

    // The last-used stamp and the audit row SHARE one throttle window: audit
    // at most once per ~60s per account, so `audit_events` grows at ~1 row
    // per minute per machine account rather than per request (matching the
    // API-key seam, which does not audit per request). The decision reads the
    // row's previous stamp, so it adds no query.
    let now = chrono::Utc::now();
    let audit_this_auth = account
        .last_used_at
        .is_none_or(|t| now - t >= chrono::Duration::seconds(60));

    if let Err(error) =
        core_db::service_accounts::touch_service_account_last_used(&mut *tx, account.id, 60).await
    {
        tracing::warn!(%error, "service account last-used touch failed");
    }

    if audit_this_auth {
        let event = AuditEvent {
            id: uuid::Uuid::now_v7(),
            org_id: account.org_id,
            actor: Actor::Agent(format!("service-account:{}", account.id)),
            action: "service_account.authenticated".into(),
            entity_type: Some("service_account".into()),
            entity_id: Some(account.id.to_string()),
            // The fingerprint is an identifier, never a secret; the cert body
            // is out of scope here.
            detail: serde_json::json!({ "cert_fingerprint": fingerprint }),
            ip: None,
            occurred_at: now,
        };
        if let Err(error) = core_db::set_org(&mut tx, account.org_id).await {
            tracing::warn!(%error, "cert-auth audit: could not bind org scope");
        } else if let Err(error) = core_db::audit::insert(&mut *tx, &event).await {
            tracing::warn!(%error, "cert-auth audit: insert failed");
        }
    }

    if let Err(error) = tx.commit().await {
        tracing::warn!(%error, "cert-auth: commit failed");
    }

    Ok(Some(AuthedUser {
        principal: core_domain::entities::Principal {
            user: service_account_user(account.id, account.org_id, &account.name),
            roles: vec![],
            permissions,
        },
        session_id: None,
        via_api_key: true,
    }))
}

/// Synthetic principal user for a certified service account: the account
/// id stands in as the actor id (audit rows attribute to it, exactly like
/// the API-key principal); no email/timezone exist for a machine identity.
fn service_account_user(
    account_id: uuid::Uuid,
    org_id: uuid::Uuid,
    name: &str,
) -> core_domain::entities::User {
    core_domain::entities::User {
        id: account_id,
        org_id,
        email: String::new(),
        display_name: format!("service-account:{name}"),
        timezone: "UTC".into(),
        is_active: true,
        mfa_totp_secret: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}
