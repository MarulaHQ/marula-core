//! The Wave 57 sweep's three rowed findings, driven through the real router
//! against the lane's Postgres.
//!
//! 1. A revoked session's access token is refused on the next request —
//!    logout, an admin revocation and the revoke-every-session path all bite
//!    before the token's TTL runs out.
//! 2. A rotation does NOT refuse the parent's access token: the parent is
//!    retired while its token is still in the client's hands, and refusing it
//!    would turn every refresh into a burst of 401s.
//! 3. A magic link refused for a missing second factor is not spent, and the
//!    same link signs in once the factor is gone.
//! 4. A slug collision names a suffix that is actually free.

use axum::http::{Method, StatusCode};
use core_api::{config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/caller.rs"]
mod caller;

use caller::fresh_caller_ip;

// --- harness ----------------------------------------------------------------

fn admin_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
}

async fn owner_pool(url: &str) -> PgPool {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    pool
}

/// Rewrites owner credentials to the RLS-constrained application role — the
/// role a deployed core-api connects as, so the SECURITY DEFINER grants this
/// lane added are exercised the way a deployment exercises them.
fn app_role_url(admin_url: &str) -> String {
    let (scheme, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_, host)) => format!("{scheme}://marula_app:marula_app@{host}"),
        None => format!("{scheme}://marula_app:marula_app@{rest}"),
    }
}

fn state_on(pool: PgPool, url: &str) -> AppState {
    let mut config = Config::from_env_allow_dev_fallback().expect("config");
    config.database_url = url.to_string();
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(
            &Config::from_env_allow_dev_fallback().expect("config"),
        )),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(url)
            .expect("lazy reporting pool"),
    }
}

/// Owner pool: most cases do not need RLS in the way, and the owner can read
/// the rows the assertions are about.
async fn plain_app(url: &str) -> axum::Router {
    core_api::router(state_on(owner_pool(url).await, url))
}

/// App-role pool: the one case that proves the new bridge's grant.
async fn rls_app(url: &str) -> axum::Router {
    let _owner = owner_pool(url).await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&app_role_url(url))
        .await
        .expect("app-role pool");
    core_api::router(state_on(pool, url))
}

async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    let request = fresh_caller_ip().stamp(request);
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn unique_slug(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::now_v7().simple())
}

async fn register(app: &axum::Router, slug: &str, email: &str) -> Value {
    let response = send(
        app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Sweep Co",
            "org_slug": slug,
            "email": email,
            "password": "correct-horse-battery-11",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "registration");
    body_json(response).await
}

async fn me(app: &axum::Router, access: &str) -> StatusCode {
    send(app, Method::GET, "/api/me", Some(access), None)
        .await
        .status()
}

fn token(pair: &Value, field: &str) -> String {
    pair[field]
        .as_str()
        .unwrap_or_else(|| panic!("missing {field}: {pair}"))
        .to_string()
}

fn session_uuid(pair: &Value) -> Uuid {
    token(pair, "session_id").parse().expect("session id")
}

async fn owner_user_of(pool: &PgPool, session: Uuid) -> (Uuid, Uuid) {
    sqlx::query_as(
        "SELECT s.user_id, u.org_id FROM auth_sessions s \
         JOIN users u ON u.id = s.user_id WHERE s.id = $1",
    )
    .bind(session)
    .fetch_one(pool)
    .await
    .expect("session row")
}

// --- 1. revocation ----------------------------------------------------------

/// Sign-out retires the row; the token issued before it must be dead on the
/// next request, not fifteen minutes later. App-role pool on purpose: the
/// bridge's grant is part of the contract.
#[tokio::test]
async fn a_logged_out_access_token_is_refused_on_the_next_request() {
    let url = admin_url();
    let app = rls_app(&url).await;
    let slug = unique_slug("revoked");
    let pair = register(&app, &slug, &format!("owner@{slug}.test")).await;
    let access = token(&pair, "access_token");
    assert_eq!(me(&app, &access).await, StatusCode::OK);

    let out = send(
        &app,
        Method::POST,
        "/api/auth/logout",
        None,
        Some(json!({
            "refresh_token": token(&pair, "refresh_token"),
            "session_id": token(&pair, "session_id"),
        })),
    )
    .await;
    assert_eq!(out.status(), StatusCode::NO_CONTENT, "logout");

    assert_eq!(
        me(&app, &access).await,
        StatusCode::UNAUTHORIZED,
        "the access token outlived the session it was issued for"
    );
}

/// The admin paths revoke every live session of a user; the same rule has to
/// hold for the tokens already issued.
#[tokio::test]
async fn revoking_every_session_refuses_the_access_token_too() {
    let url = admin_url();
    let app = plain_app(&url).await;
    let pool = owner_pool(&url).await;
    let slug = unique_slug("revoke-all");
    let pair = register(&app, &slug, &format!("owner@{slug}.test")).await;
    let access = token(&pair, "access_token");
    let session = session_uuid(&pair);
    assert_eq!(me(&app, &access).await, StatusCode::OK);

    let (user_id, org_id) = owner_user_of(&pool, session).await;
    let mut tx = pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("org scope");
    let killed = core_db::users::revoke_all_user_sessions(&mut *tx, user_id)
        .await
        .expect("revoke all");
    tx.commit().await.expect("commit");
    assert_eq!(killed, 1, "one live session for this user");

    assert_eq!(
        me(&app, &access).await,
        StatusCode::UNAUTHORIZED,
        "revoke-all left the outstanding access token working"
    );
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoked_reason FROM auth_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(&pool)
            .await
            .expect("reason");
    assert_eq!(reason.as_deref(), Some("revoked"));
}

// --- 2. rotation is not revocation -----------------------------------------

/// A refresh retires the parent and mints a child. The parent's access token
/// is still in the client's hands until it expires, so it must keep working —
/// otherwise every rotation races its own in-flight requests.
#[tokio::test]
async fn a_rotated_sessions_access_token_still_works_until_it_expires() {
    let url = admin_url();
    let app = plain_app(&url).await;
    let pool = owner_pool(&url).await;
    let slug = unique_slug("rotated");
    let pair = register(&app, &slug, &format!("owner@{slug}.test")).await;
    let parent_access = token(&pair, "access_token");

    let response = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({
            "refresh_token": token(&pair, "refresh_token"),
            "session_id": token(&pair, "session_id"),
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "refresh");
    let child = body_json(response).await;
    let child_access = token(&child, "access_token");

    assert_ne!(
        session_uuid(&pair),
        session_uuid(&child),
        "the child is a new chain node"
    );
    let reason: Option<String> =
        sqlx::query_scalar("SELECT revoked_reason FROM auth_sessions WHERE id = $1")
            .bind(session_uuid(&pair))
            .fetch_one(&pool)
            .await
            .expect("reason");
    assert_eq!(reason.as_deref(), Some("rotated"));

    assert_eq!(
        me(&app, &parent_access).await,
        StatusCode::OK,
        "the retired parent's in-flight access token was refused"
    );
    assert_eq!(me(&app, &child_access).await, StatusCode::OK);
}

/// Reuse detection is not a rotation: the whole chain is revoked because the
/// refresh secret leaked, so the tokens already issued go with it.
#[tokio::test]
async fn refresh_reuse_refuses_the_access_token_of_the_whole_chain() {
    let url = admin_url();
    let app = plain_app(&url).await;
    let slug = unique_slug("reuse");
    let pair = register(&app, &slug, &format!("owner@{slug}.test")).await;
    let parent_access = token(&pair, "access_token");

    let first = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({
            "refresh_token": token(&pair, "refresh_token"),
            "session_id": token(&pair, "session_id"),
        })),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);

    // The same parent secret again: a replay, which revokes the chain.
    let replay = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({
            "refresh_token": token(&pair, "refresh_token"),
            "session_id": token(&pair, "session_id"),
        })),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::UNAUTHORIZED, "replay refused");

    assert_eq!(
        me(&app, &parent_access).await,
        StatusCode::UNAUTHORIZED,
        "reuse detection left an access token from the chain working"
    );
}

// --- 3. a refused link is not spent -----------------------------------------

async fn confirm(app: &axum::Router, secret: &str) -> axum::response::Response {
    send(
        app,
        Method::POST,
        "/api/auth/magic-link/confirm",
        None,
        Some(json!({ "token": secret })),
    )
    .await
}

#[tokio::test]
async fn a_link_refused_for_a_missing_factor_is_not_spent() {
    let url = admin_url();
    let app = plain_app(&url).await;
    let pool = owner_pool(&url).await;
    let slug = unique_slug("magic");
    let pair = register(&app, &slug, &format!("owner@{slug}.test")).await;
    let session = session_uuid(&pair);
    let (user_id, org_id) = owner_user_of(&pool, session).await;

    // The account holds a confirmed factor, so the gate must refuse tokens.
    sqlx::query(
        "INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed) \
         VALUES ($1, $2, $3, $4, true)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(user_id)
    .bind(vec![7u8; 32])
    .execute(&pool)
    .await
    .expect("a confirmed factor");

    // One live link, hashed exactly the way the route hashes it.
    let secret = format!("link-{}", Uuid::now_v7());
    let hash = core_api::auth::jwt::hash_refresh_token(&secret);
    core_db::hardening::put_one_use_token(
        &pool,
        core_db::hardening::PutToken {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            kind: core_db::hardening::gate_kind::MAGIC_LINK,
            token_hash: &hash,
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
        },
    )
    .await
    .expect("link row");

    // Twice: the second answer must still be the gate's, not "invalid link".
    for attempt in 1..=2 {
        let response = confirm(&app, &secret).await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {attempt}"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["error"]["reason"], "mfa_required",
            "attempt {attempt} answered {body}"
        );
    }

    let used: bool = sqlx::query_scalar(
        "SELECT used_at IS NOT NULL FROM auth_one_use_tokens WHERE token_hash = $1",
    )
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .expect("token row");
    assert!(!used, "the refused link was spent");

    // With the factor gone the SAME link signs in: nothing was consumed.
    sqlx::query("DELETE FROM totp_secrets WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("factor removed");
    let response = confirm(&app, &secret).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the unspent link no longer signs in"
    );
}

// --- 4. the slug suggestion can see the collision ---------------------------

#[tokio::test]
async fn the_slug_suggestion_names_a_suffix_that_is_free() {
    let url = admin_url();
    // App-role pool on purpose: `orgs` is RLS-scoped, and the owner bypasses
    // the policy that made the old probe blind.
    let app = rls_app(&url).await;
    let slug = unique_slug("sugg");
    register(&app, &slug, &format!("first@{slug}.test")).await;

    let conflict = send(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Sweep Co Two",
            "org_slug": slug,
            "email": format!("second@{slug}.test"),
            "password": "correct-horse-battery-11",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let body = body_json(conflict).await;
    assert_eq!(body["error"]["reason"], "org_slug_taken");
    assert_eq!(
        body["error"]["suggestion"],
        format!("{slug}-2"),
        "the probe could not see the occupied base slug"
    );

    // Occupy the suggestion too; the next answer has to move on.
    register(&app, &format!("{slug}-2"), &format!("third@{slug}.test")).await;
    let conflict = send(
        &app,
        Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Sweep Co Three",
            "org_slug": slug,
            "email": format!("fourth@{slug}.test"),
            "password": "correct-horse-battery-11",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let body = body_json(conflict).await;
    assert_eq!(
        body["error"]["suggestion"],
        format!("{slug}-3"),
        "the probe suggested a slug that is already taken"
    );
}
