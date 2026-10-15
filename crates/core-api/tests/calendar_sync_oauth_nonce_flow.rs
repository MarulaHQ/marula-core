//! Wave 43 lane 20 `p43-calendar-sync-oauth-scope` — the OAuth state-nonce
//! write, tested against the RLS ENFORCING role.
//!
//! Why this file exists. Every other core-api test builds its `AppState.pool`
//! from the admin URL (`connect_admin`), which connects as the schema owner
//! and therefore BYPASSES row-level security. `calendar_sync_flow.rs` does
//! exactly that, so the whole suite ran with RLS off. Wave 43 lane 14 found
//! that `authorize_start` wrote its OAuth nonce off that same bare pool, where
//! `app.org_id` is never set: under RLS the `WHERE org_id = $1` clause resolved
//! against NULL, the UPDATE matched zero rows and the nonce was silently never
//! persisted. The admin-pool tests could not see it — the write "succeeded"
//! against every row because the policy was not applied at all.
//!
//! So this file lives against `marula_app` (the role the app actually runs as),
//! which is the only configuration in which the defect is visible. Test 1 is
//! the (C) regression test for the specific defect: it fails if the nonce write
//! ever leaves the org-bound transaction again. Test 2 is the (A) round-trip
//! test on the callback leg: it consumes the nonce with no session, through the
//! `app_consume_calendar_oauth_state` bridge (migration 0132) the callback now
//! uses, and guards against the consume ever returning to a bare-pool read.
//! See `design/qa-p43-calendar-sync-oauth-scope/`.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::memory::MemoryStore;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// Credentials for the delegated provider. `oauth_client_config` fails closed
/// when they are absent, so the flow never reaches the write without them.
fn set_google_client_env() {
    std::env::set_var("GOOGLE_CALENDAR_CLIENT_ID", "lane20-test-client");
    std::env::set_var("GOOGLE_CALENDAR_CLIENT_SECRET", "lane20-test-secret");
}

/// Rewrites the configured (privileged) URL onto the `marula_app` role, which
/// runs under RLS exactly like production (`crates/core-api/src/config.rs`
/// defaults `DATABASE_URL` to `marula_app`; migrations run as the owner).
fn app_url(admin_url: &str) -> String {
    let (scheme, rest) = admin_url
        .split_once("://")
        .unwrap_or(("postgres", admin_url));
    match rest.split_once('@') {
        Some((_, host)) => format!("{scheme}://marula_app:marula_app@{host}"),
        None => format!("{scheme}://marula_app:marula_app@{rest}"),
    }
}

async fn connect(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
        .expect("connect")
}

/// Admin pool (migrations, RLS-bypassing assertions) + a `marula_app` pool that
/// IS the app state's `pool`, so every route runs under RLS.
async fn rls_state() -> (AppState, Router, PgPool, String) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("calendar-sync oauth test needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let admin = connect(&admin_url).await;
    core_db::run_migrations(&admin).await.expect("migrations");
    core_db::assert_rls_coverage(&admin)
        .await
        .expect("RLS coverage holds with calendar-sync tables");

    let app = connect(&app_url(&admin_url)).await;
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url.clone();
    let state = AppState {
        pool: app,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    };
    let router = core_api::routes::build_router(state.clone());
    (state, router, admin, admin_url)
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
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
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({e}); status={status} body={:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        )
    })
}

/// Registers an org through the real route — which runs on the `marula_app`
/// pool and binds `app.org_id` to the new org before inserting — and returns
/// its bearer token.
async fn register_bearer(app: &Router, slug: String) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "OAuth Scope Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed under RLS");
    let tokens = body_json(response).await;
    tokens["access_token"]
        .as_str()
        .expect("register returns an access token")
        .to_string()
}

/// Creates a delegated (google) inbound connection and returns its id. No
/// token is required at create time — consent supplies it.
async fn create_google_connection(app: &Router, bearer: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/calendar-sync",
        Some(bearer),
        Some(json!({
            "name": "Work calendar",
            "provider": "google",
            "direction": "inbound",
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "google connection create must succeed under RLS"
    );
    let created = body_json(response).await;
    created["connection"]["id"]
        .as_str()
        .expect("connection id")
        .parse()
        .expect("uuid")
}

/// The `state=` query parameter the vendor will echo back on redirect.
fn nonce_from_authorize_url(url: &str) -> String {
    let after = url
        .split_once("state=")
        .map(|(_, v)| v)
        .expect("authorize URL carries a state nonce");
    after.split('&').next().expect("state value").to_string()
}

/// (C) THE SPECIFIC-DEFECT REGRESSION TEST.
///
/// Drives `POST /api/calendar-sync/{id}/authorize` on the `marula_app` pool,
/// then reads the row back with the ADMIN pool (which bypasses RLS) and asserts
/// the nonce the handler returned to the caller is the nonce actually stored.
///
/// This fails in exactly one way — the failure lane 14 fixed: if the write ever
/// moves back off `&mut *tx` onto the bare `&state.pool`, `app.org_id` is unset
/// for that statement, the RLS policy hides the row, the UPDATE matches zero
/// rows and `oauth_state` stays NULL. (On a warm pooled session it instead
/// raises 22P02 and the handler 500s; either outcome fails here.) It does not
/// test the general shape of OAuth — it pins the one invariant that regressed.
#[tokio::test]
async fn authorize_start_persists_the_nonce_inside_the_org_bound_transaction() {
    set_google_client_env();
    let (_state, app, admin, _admin_url) = rls_state().await;
    let bearer = register_bearer(&app, format!("oauth1-{}", Uuid::now_v7().simple())).await;
    let connection_id = create_google_connection(&app, &bearer).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/calendar-sync/{connection_id}/authorize"),
        Some(&bearer),
        None,
    )
    .await;
    let status = response.status();
    let started = body_json(response).await;
    assert_eq!(
        status, 200,
        "authorize must succeed under RLS, got {started}"
    );
    let url = started["authorize_url"].as_str().expect("authorize_url");
    assert!(
        url.contains("accounts.google.com"),
        "google consent URL, got {url}"
    );
    let returned_nonce = nonce_from_authorize_url(url);

    // ADMIN pool: RLS is bypassed, so this is the raw stored truth.
    let stored: Option<String> =
        sqlx::query_scalar("SELECT oauth_state FROM calendar_sync_connections WHERE id = $1")
            .bind(connection_id)
            .fetch_one(&admin)
            .await
            .expect("read stored nonce");

    assert_eq!(
        stored.as_deref(),
        Some(returned_nonce.as_str()),
        "the nonce handed to the operator MUST be the nonce persisted — a NULL here means the \
         write left the org-bound transaction and RLS matched zero rows (the lane-14 defect)"
    );
}

/// (A) ROUND-TRIP TEST — the callback leg consumes the nonce with no session.
///
/// The callback runs with no bearer (the vendor redirects the browser), so it
/// resolves the connection from the state nonce itself. `oauth_callback` does
/// that through `core_db::calendar_sync::consume_oauth_state`, which rides the
/// `app_consume_calendar_oauth_state` SECURITY DEFINER bridge (migration 0132).
/// This test calls that same core-db function on the bare `marula_app` pool
/// with no `set_org_and_user`, which is exactly the callback's situation.
///
/// It also exists as a regression guard on the bridge: before 0132 the callback
/// read the row off the bare pool directly, where RLS is default-deny, so it
/// matched zero rows and every callback answered "unknown or expired". If the
/// bridge call is ever replaced by a bare-pool read again, this fails.
#[tokio::test]
async fn oauth_callback_consumes_the_nonce_without_a_session() {
    set_google_client_env();
    let (state, app, _admin, _admin_url) = rls_state().await;
    let bearer = register_bearer(&app, format!("oauth2-{}", Uuid::now_v7().simple())).await;
    let connection_id = create_google_connection(&app, &bearer).await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/calendar-sync/{connection_id}/authorize"),
        Some(&bearer),
        None,
    )
    .await;
    let status = response.status();
    let started = body_json(response).await;
    assert_eq!(status, 200, "authorize failed: {started}");
    let nonce = nonce_from_authorize_url(started["authorize_url"].as_str().unwrap());

    // The callback's own resolution path, on the bare app pool, no session.
    let resolved = core_db::calendar_sync::consume_oauth_state(&state.pool, "google", &nonce)
        .await
        .expect("consume runs")
        .expect(
            "the callback must resolve a freshly-stored nonce without a session — it cannot via \
             a bare-pool read, which RLS hides",
        );
    assert_eq!(resolved.connection_id, connection_id);

    // Single-use: the atomic claim clears the nonce, so a replay resolves nothing.
    let replay = core_db::calendar_sync::consume_oauth_state(&state.pool, "google", &nonce)
        .await
        .expect("consume runs");
    assert!(
        replay.is_none(),
        "a consumed nonce must not resolve again (replay protection)"
    );
}
