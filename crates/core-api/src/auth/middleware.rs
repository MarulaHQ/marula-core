//! Request middleware: extracts the Bearer token, verifies the JWT, and
//! loads a fresh Principal (user + roles + unioned permissions) from the DB.
//! Permissions are never cached in the token — revocation applies instantly.

use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use axum::extract::connect_info::ConnectInfo;
use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};
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

/// The one place a session principal is assembled, and therefore the one place
/// that answers "what may this principal do": the user row, the effective roles
/// it holds, and the permission union those roles expand to.
///
/// `roles` must come from `core_db::users::effective_roles_for_user` — the
/// additive union of `user_roles` and the roles granted by the directory groups
/// the user belongs to — not from the direct-assignment read. Both the request
/// middleware and the workflow-run seam build their principal here, so an
/// authorization path cannot exist that misses a group-granted role.
pub fn principal_for(
    user: core_domain::entities::User,
    roles: Vec<core_domain::entities::Role>,
) -> core_domain::entities::Principal {
    let permissions = resolve_permissions(&roles);
    core_domain::entities::Principal {
        user,
        roles,
        permissions,
    }
}

/// Axum middleware: `Authorization: Bearer <jwt>` for humans, a scoped
/// machine path via `x-api-key` (the private build tracker 2.15), and client-certificate
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
/// internal/design/qa-p47-client-cert-auth/LANDING.md (a dedicated machine-kind
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
        }) || (candidate == "/api/me" && request.method() == axum::http::Method::GET)
            // The IF8 filing route: an agent files a proposal, and filing is not
            // a write — approval does that. The capability grant in the handler
            // is the gate (deny-by-default), and it answers a typed 403 that
            // names the missing capability. The two read routes on this path
            // stay closed: they require the Admin matrix, which no mintable
            // scope grants.
            || (candidate == "/api/agent-proposals"
                && request.method() == axum::http::Method::POST);
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

        // A key bound to a service account is that agent: its permissions are
        // the account's own keys, and the account's status, expiry and
        // revocation are re-checked on every use. An unbound key keeps the
        // connector vocabulary it was minted with.
        let (permissions, user) = match row.service_account_id {
            Some(account_id) => {
                let account = core_db::service_accounts::find_active_service_account_by_id(
                    &state.pool,
                    account_id,
                )
                .await
                .map_err(|_| ApiError::Unauthorized("api key rejected".into()))?
                .ok_or_else(|| ApiError::Forbidden("service account is not active".into()))?;
                if account.org_id != row.org_id {
                    return Err(ApiError::Forbidden("service account is not active".into()));
                }
                let permissions =
                    crate::routes::service_accounts::resolve_scope_permissions(&account.scopes);
                if permissions.is_empty() {
                    return Err(ApiError::Forbidden(
                        "service account carries no usable scopes".into(),
                    ));
                }
                (
                    permissions,
                    service_account_user(account.id, account.org_id, &account.name),
                )
            }
            None => {
                let permissions = permissions_from_scopes(&row.scopes);
                if permissions.is_empty() {
                    return Err(ApiError::Forbidden(
                        "api key carries no usable scopes".into(),
                    ));
                }
                (permissions, machine_user(row.id, row.org_id))
            }
        };
        request.extensions_mut().insert(AuthedUser {
            principal: core_domain::entities::Principal {
                user,
                roles: vec![],
                permissions,
            },
            session_id: None,
            via_api_key: true,
        });
        return Ok(next.run(request).await);
    }

    // SSE browser clients (native EventSource) cannot set request
    // headers, so the events stream accepts `?token=` as a fallback.
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

    // Wave 57 (p57 finding 3): the session row is the thing revocation writes,
    // and nothing on this path read it. Logout, an admin factor reset and
    // refresh-token reuse all killed the chain while an already-issued access
    // token kept working for its full TTL. One primary-key probe through the
    // SECURITY DEFINER bridge (the pool carries no org scope).
    //
    // `rotated` passes: a refresh retires the parent while the parent's access
    // token is still in the client's hands, and refusing that would turn every
    // rotation into a burst of 401s. Every deliberate reason refuses.
    if !core_db::users::session_active(&state.pool, claims.sid)
        .await
        .map_err(|_| ApiError::Unauthorized("session not found".into()))?
    {
        return Err(ApiError::Unauthorized("session ended".into()));
    }

    // Wave 52 lane P3: the principal is cached per replica for
    // `ref_cache_ttl_secs` and dropped the moment a `user.*` or `role.*` event
    // lands (`crate::cache_invalidation`), so the two indexed queries below run
    // once per (org, user) per TTL window rather than once per request.
    // Deactivation stays honest for the same reason revocation always was: it
    // is a mutation like any other, so it emits an event like any other.
    let caches = crate::cache::caches();
    let cache_key = (claims.org, claims.sub);
    let cached = if caches.enabled() {
        caches.principal.get(&cache_key)
    } else {
        None
    };
    let principal = match cached {
        Some(cached) => cached,
        None => {
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
            let roles =
                core_db::users::effective_roles_for_user(&mut *tx, claims.sub, claims.org).await?;
            tx.commit()
                .await
                .map_err(|_| ApiError::Unauthorized("user not found or inactive".into()))?;
            let principal = principal_for(user, roles);
            if caches.enabled() {
                caches.principal.insert(cache_key, principal.clone());
            }
            principal
        }
    };

    request.extensions_mut().insert(AuthedUser {
        principal,
        session_id: Some(claims.sid),
        via_api_key: false,
    });

    Ok(next.run(request).await)
}

/// Read-only prefixes where `x-api-key` auth is accepted. Everything else
/// (mutations, admin surfaces, self-scoped endpoints) is JWT-only.
/// `/api/imports` is the deliberate mutation exception (the private build tracker D2/D3):
/// scoped migration pushes are a documented machine flow, gated in-handler
/// by scope checks on top of this prefix allowance. `/api/agent-proposals`
/// is deliberately absent: it is the IF8 machine flow, and its handlers need
/// the Admin matrix, which no mintable scope grants. A key was accepted there
/// and refused two layers down, so the allowance is gone until a scope exists
/// that the gate actually checks.
/// `/api/migration` is the D3 connector surface (push + pull), scoped by
/// `migration:*` keys. `/api/orgs` is the Wave 47 A6 usage/quota monitoring
/// read (scoped by `org_usage:read`) plus A7's suspension notice
/// (`org_suspension:read`) and A8's custom domain (`org_custom_domain:read`
/// / `:manage`), plus R16's directory surface (`directory_groups:read` /
/// `:write`). An allowance is not a grant: each of
/// these prefixes is JWT-reachable too, and the handlers' permission checks
/// still run.
/// Canonical `/api/v1/...` requests match through their legacy-form twin.
const API_KEY_PATHS: &[&str] = &[
    "/api/tickets",
    "/api/clients",
    "/api/imports",
    "/api/migration",
    // The dispatch connector (Wave 9 lane 9): scoped schedule reads +
    // field-service status pushes; handlers gate on `dispatch:*` scopes.
    "/api/dispatch-connector",
    // Org resource usage + quotas (Wave 47 lane A6): READ-only monitoring
    // reports for hosted-tier plugins. A key on these paths carries
    // `org_usage:read` (Permission::ViewOrgUsage) and nothing else useful
    // — the handler's own permission check is the gate, and the read is
    // org-bound exactly like a human member's.
    "/api/orgs",
    // The CI feeders (Wave 12 lane 8, ADR 0003 CI7): RMM/NMS/docs feeders
    // write through /api/config-items/feed-upsert with `clients:write`
    // keys — the handler's Clients-matrix permission check stays the
    // gate (a read-scoped key still gets typed 403s on writes).
    "/api/config-items",
    // RAG and media vector namespace (Wave 45 lane 1, ADR 0020): connectors,
    // plugins, and self-hosters call ingest/query/delete through `/api/rag`
    // (`/api/v1/rag`) gated by `rag:read` (`kb:view`) and `rag:write`
    // (`kb:edit`).
    "/api/rag",
    // The audit trail (Wave 52 lane R12): a premium pack that exports or
    // forwards the trail calls `GET /api/audit-events` with an
    // `observability:read` key and the ascending `after` cursor. An allowance
    // is not a grant — the handler's own gate still runs, and that gate is what
    // refuses a key holding only matrix reads.
    "/api/audit-events",
    // The directory surface (Wave 52 lane R16): the identity pack's SCIM
    // connector and directory-admin plugin are separate processes and reach
    // groups, members and role mappings through this prefix. The allowance
    // covers the whole family because the connector writes as well as reads;
    // the handler's own gate is what separates a reader from a writer, and a
    // key with `directory_groups:read` still gets a typed 403 on every write.
    "/api/directory-groups",
];

/// Maps capability scopes onto the permission matrix. Unknown scopes are
/// ignored here (they cannot be created — validated at key creation).
fn permissions_from_scopes(scopes: &[String]) -> Vec<core_domain::policy::Permission> {
    use core_domain::policy::{Action, Module, Permission};
    let mut out = Vec::new();
    // Org usage monitoring (Wave 47 lane A6): the narrowest grant in the
    // table — ONE read-only permission that unlocks ONE read-only report.
    // It deliberately does NOT map onto the Reporting matrix, so a
    // monitoring key gains no report definitions, no schedules, and no
    // delivery of its own. Handled before the scope loop because the loop's
    // `push` closure already borrows `out` mutably.
    if scopes.iter().any(|s| s.as_str() == "org_usage:read")
        && !out.contains(&Permission::ViewOrgUsage)
    {
        out.push(Permission::ViewOrgUsage);
    }
    // Wave 47 lane A8: custom-domain read + manage. Same module-namespaced
    // shape as `org_usage:read` (not a matrix `module:action`), handled in the
    // same pre-loop block for the same borrow reason above.
    if scopes
        .iter()
        .any(|s| s.as_str() == "org_custom_domain:read")
        && !out.contains(&Permission::ViewOrgCustomDomain)
    {
        out.push(Permission::ViewOrgCustomDomain);
    }
    if scopes
        .iter()
        .any(|s| s.as_str() == "org_custom_domain:manage")
        && !out.contains(&Permission::ManageOrgCustomDomain)
    {
        out.push(Permission::ManageOrgCustomDomain);
    }
    // Wave 47 lane A7: the suspension notice read. Same module-namespaced
    // shape again, and read-only by design: setting the flag is an admin
    // action, so a machine key has no manage half to present.
    if scopes.iter().any(|s| s.as_str() == "org_suspension:read")
        && !out.contains(&Permission::ViewOrgSuspension)
    {
        out.push(Permission::ViewOrgSuspension);
    }
    // Wave 52 lane R12: the observability reader. Read-only by construction —
    // the audit trail is append-only and this grant reaches no other route.
    if scopes.iter().any(|s| s.as_str() == "observability:read")
        && !out.contains(&Permission::ViewObservability)
    {
        out.push(Permission::ViewObservability);
    }
    // Wave 52 lane R16: the directory pair. Read and write are separate keys
    // on purpose — a groups-only mirror never receives role mapping — and the
    // write key does not imply the read one, so a writer carries both when it
    // needs to read back what it wrote.
    if scopes.iter().any(|s| s.as_str() == "directory_groups:read")
        && !out.contains(&Permission::ViewDirectoryGroups)
    {
        out.push(Permission::ViewDirectoryGroups);
    }
    if scopes
        .iter()
        .any(|s| s.as_str() == "directory_groups:write")
        && !out.contains(&Permission::ManageDirectoryGroups)
    {
        out.push(Permission::ManageDirectoryGroups);
    }
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
            // The catalog's sensitive ticket-correspondence read (row A29):
            // its own MCP capability so enabling ticket reads does not imply
            // correspondence. At core-api it maps to the ticket-view permission
            // the read handler already enforces — so a KEY minted with only
            // this scope can also call the ordinary ticket reads directly. The
            // tight surface is the MCP adapter, which exposes only the tools
            // this capability declares; keys are connector surface.
            "tickets_correspondence:read" => push(Module::Tickets, &[Action::View]),
            "tickets:write" => push(
                Module::Tickets,
                &[Action::View, Action::Create, Action::Edit],
            ),
            "clients:read" => push(Module::Clients, &[Action::View]),
            "clients:write" => push(
                Module::Clients,
                &[Action::View, Action::Create, Action::Edit],
            ),
            // Migration connector (the private build tracker D3): deliberately broad data
            // grants — a migration tool writes whole entity families
            // through the D2 engine. The engine's gate checks the Clients
            // matrix (the import framework's machine contract), so the
            // scope resolves onto exactly that.
            // Dispatch connector (the private build tracker Phase 3, Wave 9 lane 9):
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
            // RAG and media vector namespace (Wave 45 lane 1, ADR 0020):
            // `rag:read` resolves onto `kb:view` (query + deletion receipt
            // verification); `rag:write` resolves onto `kb:view` + `kb:edit`
            // (batch chunk ingest + source deletion).
            "rag:read" | "rag.read" | "kb:read" | "kb.read" => push(Module::Kb, &[Action::View]),
            "rag:write" | "rag.write" | "kb:write" | "kb.write" => push(
                Module::Kb,
                &[Action::View, Action::Create, Action::Edit, Action::Delete],
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
    let Some(fingerprint) =
        crate::routes::service_accounts::normalize_fingerprint(&presentation.raw)
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

#[cfg(test)]
mod tests {
    use super::principal_for;
    use core_domain::entities::{Role, User};
    use core_domain::policy::{BuiltinRole, Permission};

    fn user() -> User {
        let now = chrono::Utc::now();
        User {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            email: "union@acme.test".into(),
            display_name: "Union Test".into(),
            timezone: "UTC".into(),
            is_active: true,
            mfa_totp_secret: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn role(name: &str, builtin: Option<BuiltinRole>, permissions: Vec<Permission>) -> Role {
        Role {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            name: name.into(),
            builtin,
            permissions,
            is_system: builtin.is_some(),
        }
    }

    /// The assembly point is the union's only consumer: whatever roles the
    /// caller loaded — direct, group-derived, or both — become the permission
    /// list every route check reads. A group-granted role is not a second
    /// authorization path, so it must not need one here either.
    #[test]
    fn a_group_granted_role_reaches_the_permission_union() {
        let direct = role("Technician", None, vec![Permission::ManageUsers]);
        let via_group = role("Admin", None, vec![Permission::ManageRoles]);

        let principal = principal_for(user(), vec![direct, via_group]);

        assert_eq!(principal.roles.len(), 2);
        assert!(principal.permissions.contains(&Permission::ManageUsers));
        assert!(
            principal.permissions.contains(&Permission::ManageRoles),
            "a role reached through a group contributes exactly like a direct one"
        );
    }

    /// The API-key path resolves the directory pair onto exactly the two new
    /// permissions, and a key that names neither gets neither — whatever else
    /// it carries. This is the layer a machine credential meets before any
    /// handler runs, and the one that would silently over-grant if the keys
    /// were mapped to a broader grant.
    #[test]
    fn the_directory_scopes_resolve_onto_their_own_pair_and_nothing_wider() {
        use super::permissions_from_scopes;

        let read = permissions_from_scopes(&["directory_groups:read".to_string()]);
        assert_eq!(read, vec![Permission::ViewDirectoryGroups]);

        let write = permissions_from_scopes(&["directory_groups:write".to_string()]);
        assert_eq!(write, vec![Permission::ManageDirectoryGroups]);

        // The pair together, in the order the key lists them, with no
        // duplicate and no matrix grant invented along the way.
        let both = permissions_from_scopes(&[
            "directory_groups:read".to_string(),
            "directory_groups:write".to_string(),
        ]);
        assert_eq!(
            both,
            vec![
                Permission::ViewDirectoryGroups,
                Permission::ManageDirectoryGroups
            ]
        );

        // A key carrying other scopes is not a directory key.
        let elsewhere = permissions_from_scopes(&[
            "observability:read".to_string(),
            "org_usage:read".to_string(),
            "tickets:write".to_string(),
        ]);
        assert!(!elsewhere.contains(&Permission::ViewDirectoryGroups));
        assert!(!elsewhere.contains(&Permission::ManageDirectoryGroups));
        // And neither new grant is an admin special.
        for grant in both {
            assert!(!matches!(
                grant,
                Permission::ManageUsers
                    | Permission::ManageRoles
                    | Permission::ManagePlugins
                    | Permission::ManageApiKeys
            ));
        }
    }

    /// Every scope the API-key surface hands out must mean exactly what the
    /// registry says it means. `crate::api_scopes` publishes the grant with
    /// the scope; this module decides it at request time. A drift between
    /// them is a documented grant that does not exist (or a wider one than
    /// the document claims), so the parity is a test.
    #[test]
    fn every_mintable_api_key_scope_resolves_to_its_documented_grant() {
        use super::permissions_from_scopes;

        let mut checked = 0;
        for scope in crate::api_scopes::SCOPES {
            let resolved = permissions_from_scopes(&[scope.name.to_string()]);
            assert!(
                !resolved.is_empty(),
                "{} can be written onto an API key but resolves to no \
                 permission — the key would authenticate carrying nothing",
                scope.name
            );
            assert_eq!(
                resolved, scope.grants,
                "{} grants a different permission set than the document \
                 publishes",
                scope.name
            );
            checked += 1;
        }
        assert!(
            checked >= 18,
            "only {checked} scopes were checked; the list moved and this test is \
             now watching almost nothing"
        );
    }

    /// A builtin role expands from its shipped matrix, not from a drifted row.
    #[test]
    fn a_builtin_role_expands_from_the_canonical_matrix() {
        use core_domain::policy::{Action, Module};

        let owner = role("Owner", Some(BuiltinRole::Owner), vec![]);
        let principal = principal_for(user(), vec![owner]);

        assert!(principal.is_owner_or_admin());
        assert!(principal.permissions.contains(&Permission::ManageBilling));
        assert!(principal.permissions.contains(&Permission::Matrix {
            module: Module::Tickets,
            action: Action::Delete,
        }));
    }
}
