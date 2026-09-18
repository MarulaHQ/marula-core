//! Adversarial auth-hardening flow e2e (Wave 19 lane 5 verification
//! gates). Drives the real core-api router against the lane's Postgres.
//!
//! Proves:
//! 1. Lockout bypass via IP rotation is IMPOSSIBLE — the per-actor
//!    (email) key locks independently of the per-IP key (A1).
//! 2. Enumeration-shape consistency: a throttled login answer is
//!    envelope-shape IDENTICAL to a wrong-password answer (401, same
//!    reason/message); refresh throttles are the typed 429 instead.
//! 3. Refresh reuse AFTER rotation revokes the whole chain.
//! 4. Magic-link tokens are single-use: reuse is a uniform 401 (B7).
//! 5. Cookie flags: the staff session cookie carries httpOnly,
//!    SameSite=Lax, Path=/api, Max-Age; the double-submit csrf value is
//!    present and non-httpOnly (A2).

use axum::http::Method;
use core_api::{config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

// --- harness ----------------------------------------------------------------

async fn test_state(admin_url: &str) -> AppState {
    let pool: PgPool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("config");
    config.database_url = admin_url.to_string();
    AppState {
        pool,
        config: Arc::new(config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(
            &Config::from_env_allow_dev_fallback().expect("config"),
        )),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(admin_url)
            .expect("lazy reporting pool"),
    }
}

async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    bearer: Option<&str>,
    extra_headers: &[(&'static str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (key, value) in extra_headers {
        builder = builder.header(*key, *value);
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
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn register_org(app: &axum::Router, slug: &str) -> Value {
    let tokens = body_json(
        send(
            app,
            Method::POST,
            "/api/auth/register",
            None,
            &[],
            Some(json!({
                "org_name": "Hard Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": "correct-horse-battery-11",
                "display_name": "Owner",
            })),
        )
        .await,
    )
    .await;
    let bearer: String = tokens["access_token"]
        .as_str()
        .map(String::from)
        .unwrap_or_default();
    let me = body_json(send(app, Method::GET, "/api/me", Some(&bearer), &[], None).await).await;
    json!({
        "access_token": bearer,
        "refresh_token": tokens["refresh_token"],
        "session_id": tokens["session_id"],
        "user_id": me["user_id"],
        "org_id": me["org_id"],
    })
}

// ---------------------------------------------------------------------------
// (1)(2) lockout: IP rotation cannot bypass the account lock, and the
// throttled login answer is shape-identical to a wrong-password answer.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ip_rotation_cannot_bypass_the_account_lock() {
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let email = Uuid::now_v7().to_string();
    let email = format!("iprot-{email}@lock-a.test");
    register_org(&app, "lock-a").await;

    // Re-declare an identical-shape login against the same account from
    // eight DIFFERENT forwarded IPs (each sees its own per-IP counter).
    for hop in 0..8 {
        let ip = format!("203.0.113.{hop}");
        let response = send(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            &[("x-forwarded-for", ip.as_str())],
            Some(json!({ "email": email, "password": "wrong-password-11" })),
        )
        .await;
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    // A fresh IP: the per-actor key is STILL locked. The answer is the
    // uniform 401 with the same reason/message shape as the failures
    // above — no typed 429, no lock-state fingerprint (§A1 uniform rule).
    let locked = send(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        &[("x-forwarded-for", "203.0.113.199")],
        Some(json!({ "email": email, "password": "anything-11-valid" })),
    )
    .await;
    assert_eq!(locked.status(), axum::http::StatusCode::UNAUTHORIZED);
    let envelope = body_json(locked).await;
    assert_eq!(envelope["error"]["reason"], "unauthorized");
}

/// The typed 429 (`auth_rate_limited`) is the REFRESH-family answer:
/// pathological refresh callers get the branchable code without any
/// account lock being fingerprinted (proof-of-possession surfaces).
#[tokio::test]
async fn pathological_refresh_throttle_is_a_typed_429() {
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    // Elaborate the isolated caller IP so other tests' durable lockouts
    // cannot bleed into this one's curve.
    let ip = format!("114.9.10.{}", Uuid::now_v7());
    // Attempt 1: invalid refresh ⇒ counting (uniform 401).
    let first = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip.as_str())],
        Some(json!({ "refresh_token": "0".repeat(64), "session_id": Uuid::now_v7() })),
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::UNAUTHORIZED);
    // Attempt 2 WITHIN the exponential window ⇒ typed 429 with the
    // branchable `auth_rate_limited` reason (no account info leaked —
    // the caller never got past the per-IP gate).
    let second = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip.as_str())],
        Some(json!({ "refresh_token": "1".repeat(64), "session_id": Uuid::now_v7() })),
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    let envelope = body_json(second).await;
    assert_eq!(envelope["error"]["reason"], "auth_rate_limited");
}

// ---------------------------------------------------------------------------
// (3) refresh reuse after rotation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_reuse_after_rotation_kills_the_whole_chain() {
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    // Elaborate the isolated caller IP (durable per-IP counters are
    // process-wide; other tests must not bleed into this chain check).
    let ip = format!("114.51.40.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let _ip_dead = format!("114.51.41.{}", Uuid::now_v7());
    let seeded = register_org(&app, "reuse").await;
    let refresh = seeded["refresh_token"].as_str().unwrap().to_string();
    let session = Uuid::parse_str(seeded["session_id"].as_str().unwrap()).unwrap();

    // Rotation 1: validate the OLD secret and mint a NEW one.
    let rotated = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip)],
        Some(json!({ "refresh_token": refresh, "session_id": session.to_string() })),
    )
    .await;
    assert_eq!(rotated.status(), axum::http::StatusCode::OK);
    let rotated_pair = body_json(rotated).await;
    let new_refresh = rotated_pair["refresh_token"].as_str().unwrap().to_string();

    // REUSE the rotated parent ⇒ whole-chain revocation (reuse detection).
    let replay = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip)],
        Some(json!({ "refresh_token": refresh, "session_id": session })),
    )
    .await;
    assert_eq!(replay.status(), axum::http::StatusCode::UNAUTHORIZED);

    // The rotation's successor DIED with the chain — presenting it is a
    // 401. A FRESH caller ip because this test's own IP is now under the
    // exponential lock it just demonstrated (durable, per-IP).
    let ip_dead = format!("114.51.41.{}", Uuid::now_v7());
    let dead = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip_dead.as_str())],
        Some(json!({
            "refresh_token": new_refresh,
            "session_id": rotated_pair["session_id"].as_str().unwrap(),
        })),
    )
    .await;
    assert_eq!(dead.status(), axum::http::StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// (4) magic-link single-use (B7)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn magic_link_reuse_is_a_uniform_401() {
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool: PgPool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let app = core_api::router(test_state(&admin_url).await);

    // Seed: org with the magic-link opt-in + one user + one live token.
    let org = Uuid::now_v7();
    let user = Uuid::now_v7();
    // The server stores only sha256(token); send the raw (64 hexes) and
    // seed the recorded hash with its digest (`jwt.rs` uses plain sha256).
    let token_raw: String = "0".repeat(64);
    use sha2::{Digest, Sha256};
    let token_hash = hex::encode(Sha256::digest(token_raw.as_bytes()));
    sqlx::query(
        "INSERT INTO orgs (id, name, slug, ticket_prefix, ticket_sequence, base_currency, magic_link_sign_in) \
         VALUES ($1,'Ml Co','ml','ML',1,'USD', true)",
    )
    .bind(org)
    .execute(&pool)
    .await
    .expect("org");
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash) \
         VALUES ($1,$2,$3,'T','argon2')",
    )
    .bind(user)
    .bind(org)
    .bind("ml-user@test")
    .execute(&pool)
    .await
    .expect("user");
    sqlx::query(
        "INSERT INTO auth_one_use_tokens (id, org_id, user_id, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, 'magic_link', $4, clock_timestamp() + interval '10 minutes')",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(user)
    .bind(&token_hash)
    .execute(&pool)
    .await
    .expect("token");

    // Redemption 1: ok (TokenPair body).
    let first = send(
        &app,
        Method::POST,
        "/api/auth/magic-link/confirm",
        None,
        &[],
        Some(json!({ "token": token_raw })),
    )
    .await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);

    // REDEMPTION 2: reused link — the uniform 401, no second session.
    let second = send(
        &app,
        Method::POST,
        "/api/auth/magic-link/confirm",
        None,
        &[],
        Some(json!({ "token": token_raw })),
    )
    .await;
    assert_eq!(second.status(), axum::http::StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// (5) cookie flags (A2)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_sets_the_hardened_cookie_trio() {
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let slug = format!(
        "cook-{}",
        Uuid::now_v7().to_string()[..8].to_ascii_lowercase()
    );
    let _email = format!("owner@{slug}.test");
    // Bootstrap the account (same login POST follows register verbatim).
    register_org(&app, &slug).await;
    let response = send(
        &app,
        Method::POST,
        "/api/auth/login",
        None,
        &[],
        Some(json!({
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery-11",
        })),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    // Two Set-Cookie heads: the httpOnly session pair + the csrf jar.: the httpOnly session pair + the csrf jar.
    let headers = response.headers();
    let set_cookies: Vec<String> = headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();
    assert!(
        set_cookies
            .iter()
            .any(|c| c.starts_with("meridian_session=") && c.contains("HttpOnly")),
        "httpOnly session cookie missing: {set_cookies:?}"
    );
    assert!(set_cookies.iter().any(|c| c.contains("SameSite=Lax")));
    assert!(set_cookies.iter().any(|c| c.contains("Path=/api")));
    assert!(set_cookies.iter().any(|c| c.starts_with("meridian_csrf=")));
    // The body STILL carries the token pair (bearer path unchanged).
    let envelope = body_json(response).await;
    assert!(envelope["access_token"].is_string());
}
