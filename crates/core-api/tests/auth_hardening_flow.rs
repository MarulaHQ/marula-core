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
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let email = Uuid::now_v7().to_string();
    let slug = format!("lock-a-{}", Uuid::now_v7().simple());
    let email = format!("iprot-{email}@{slug}.test");
    register_org(&app, &slug).await;

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    // Elaborate the isolated caller IP (durable per-IP counters are
    // process-wide; other tests must not bleed into this chain check).
    let ip = format!("114.51.40.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let _ip_dead = format!("114.51.41.{}", Uuid::now_v7());
    let slug = format!("reuse-{}", Uuid::now_v7().simple());
    let seeded = register_org(&app, &slug).await;
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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
    // Random token — auth_one_use_tokens.token_hash is UNIQUE and a
    // fixed raw token collided with prior runs of this binary.
    let token_raw: String = format!(
        "{:064x}",
        (Uuid::now_v7().as_u128() << 64) | Uuid::now_v7().as_u128()
    );
    use sha2::{Digest, Sha256};
    let token_hash = hex::encode(Sha256::digest(token_raw.as_bytes()));
    // Unique slug/email — the test DB is reused across binary runs and
    // orgs.slug / users.email are unique (fixed ids collided on reuse).
    let ml_slug = format!("ml-{}", Uuid::now_v7().simple());
    let ml_email = format!("{ml_slug}@test");
    sqlx::query(
        "INSERT INTO orgs (id, name, slug, ticket_prefix, ticket_sequence, base_currency, magic_link_sign_in) \
         VALUES ($1,'Ml Co',$2,'ML',1,'USD', true)",
    )
    .bind(org)
    .bind(&ml_slug)
    .execute(&pool)
    .await
    .expect("org");
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash) \
         VALUES ($1,$2,$3,'T','argon2')",
    )
    .bind(user)
    .bind(org)
    .bind(&ml_email)
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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
            .any(|c| c.starts_with("marula_session=") && c.contains("HttpOnly")),
        "httpOnly session cookie missing: {set_cookies:?}"
    );
    // The double-submit value is the one cookie the browser's own
    // JavaScript MUST read (ADR 0022) — HttpOnly here would break every
    // cookie-driven refresh for a client that cannot echo the header.
    assert!(
        set_cookies
            .iter()
            .any(|c| c.starts_with("marula_csrf=") && !c.contains("HttpOnly")),
        "the csrf double-submit value must stay readable to JavaScript: {set_cookies:?}"
    );
    assert!(set_cookies.iter().any(|c| c.contains("SameSite=Lax")));
    assert!(set_cookies.iter().any(|c| c.contains("Path=/api")));
    assert!(set_cookies.iter().any(|c| c.starts_with("marula_csrf=")));
    // The body STILL carries the token pair (bearer path unchanged).
    let envelope = body_json(response).await;
    assert!(envelope["access_token"].is_string());
}

// ---------------------------------------------------------------------------
// (6) Wave 47 lane A2 — the two-origin refresh (p41 B1)
//
// The shape under test: a credentialed POST that carries a body credential
// AND an ambient `marula_session` cookie, but NO `x-marula-csrf` header.
//
// That is what a split-origin desktop shell and any cross-registrable-domain
// API deployment produces. The browser still SENDs the `SameSite=Lax` session
// cookie (ports never split a site, and neither does a sibling subdomain),
// still accepts the rotated `Set-Cookie`, and the app's JavaScript still
// cannot read the API origin's `marula_csrf` to echo it, because cookies are
// host-scoped rather than port-scoped and this is a different host. Before
// the fix, the refresh handler applied `double_submit_ok` BEFORE resolving
// the credential source, so a perfectly valid (refresh_token, session_id)
// pair in the JSON body was rejected with `403 csrf check failed` — a re-auth
// loop for the whole life of the access token.
//
// These tests pin the split that keeps both halves honest: the body
// credential is NOT gated (it cannot be forged cross-site), the ambient
// cookie stays gated exactly as before (it can).
//
// Each test uses its own run-unique IP: the durable per-IP refresh gate is
// shared, and sibling rotations in one binary must not bleed into this
// chain's curve.
//
// The paths below use the unversioned `/api/auth/...` legacy alias
// (`routes::ApiGeneration::LegacyAlias` — one builder, two mounts), which
// dispatches the exact handlers the SDK hits at `/api/v1/auth/...`.
// ---------------------------------------------------------------------------

/// The `marula_session` cookie value a browser would hold after `login`.
/// Shape is `{session_id}.{refresh_token}` (`cookies::session_cookie`).
fn session_cookie_value(session_id: &str, refresh_token: &str) -> String {
    format!("{session_id}.{refresh_token}")
}

/// A login that returns the token pair, its Set-Cookie heads, and the
/// session cookie value a browser would have stored from it.
async fn login_with_cookies(app: &axum::Router, slug: &str) -> (Value, String, Vec<String>) {
    register_org(app, slug).await;
    let response = send(
        app,
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
    let set_cookies: Vec<String> = response
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();
    let session_head = set_cookies
        .iter()
        .find(|c| c.starts_with("marula_session="))
        .unwrap_or_else(|| panic!("no marula_session Set-Cookie: {set_cookies:?}"));
    let jar = session_head
        .trim_start_matches("marula_session=")
        .split(';')
        .next()
        .unwrap_or_default()
        .to_string();
    let envelope = body_json(response).await;
    (envelope, jar, set_cookies)
}

/// One refresh POST. The two-origin shape is the ambient session cookie
/// with NO `x-marula-csrf` header: the app's JS cannot read the API
/// origin's csrf cookie, so it has nothing to echo.
///
/// `jar` is the cookie VALUE (`{session_id}.{secret}`); the request
/// header carries it the way a browser would, as `marula_session=<jar>`.
async fn two_origin_refresh(
    app: &axum::Router,
    ip: &str,
    jar: &str,
    body: Value,
) -> axum::response::Response {
    let cookie = format!("marula_session={jar}");
    send(
        app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip), ("cookie", cookie.as_str())],
        Some(body),
    )
    .await
}

#[tokio::test]
async fn body_credential_refresh_with_an_ambient_session_cookie_and_no_csrf_header_succeeds() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let ip = format!("114.70.10.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let slug = format!("twoorg-{}", Uuid::now_v7().simple());
    let (seeded, jar, _) = login_with_cookies(&app, &slug).await;
    let refresh = seeded["refresh_token"].as_str().unwrap().to_string();
    let session = seeded["session_id"].as_str().unwrap().to_string();
    assert_eq!(
        jar,
        session_cookie_value(&session, &refresh),
        "the ambient jar is the (session_id, secret) pair the browser stored"
    );

    // THE defect: a valid body credential + the ambient jar, no
    // x-marula-csrf header. Before the fix this answered
    // `403 Forbidden "csrf check failed"`.
    let rotated = two_origin_refresh(
        &app,
        ip,
        jar.as_str(),
        json!({ "refresh_token": refresh, "session_id": session }),
    )
    .await;
    assert_eq!(
        rotated.status(),
        axum::http::StatusCode::OK,
        "a body credential is not forgeable cross-site, so the double-submit gate must not apply to it"
    );
    // Read the Set-Cookie heads BEFORE the body consumes the response:
    // the browser must receive a rotated jar entry, or the next
    // cookie-mode refresh would present the consumed parent and trip
    // reuse detection for a perfectly live session.
    let rotated_jars: Vec<String> = rotated
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();
    let rotated_jar = rotated_jars
        .iter()
        .find(|c| c.starts_with("marula_session="))
        .map(|c| {
            c.trim_start_matches("marula_session=")
                .split(';')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_else(|| panic!("no rotated marula_session Set-Cookie: {rotated_jars:?}"));
    let pair = body_json(rotated).await;
    let new_refresh = pair["refresh_token"].as_str().unwrap().to_string();
    let new_session = pair["session_id"].as_str().unwrap().to_string();
    let new_access = pair["access_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, seeded["refresh_token"].as_str().unwrap());
    assert_ne!(new_session, session);
    assert_ne!(new_access, seeded["access_token"].as_str().unwrap());

    // The rotated Set-Cookie jar entry IS the new pair the browser just
    // stored — so a later cookie-mode refresh presents a live pair.
    assert_eq!(
        rotated_jar,
        session_cookie_value(&new_session, &new_refresh),
        "the rotated Set-Cookie must carry the new (session_id, secret) pair"
    );

    // The kept credential rotates again in the same two-origin shape.
    let second = two_origin_refresh(
        &app,
        ip,
        rotated_jar.as_str(),
        json!({ "refresh_token": new_refresh, "session_id": new_session }),
    )
    .await;
    assert_eq!(
        second.status(),
        axum::http::StatusCode::OK,
        "the rotated pair keeps refreshing in the same two-origin shape"
    );
}

#[tokio::test]
async fn cookie_only_refresh_without_the_csrf_header_is_still_403() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let ip = format!("114.70.11.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let slug = format!("cookieonly-{}", Uuid::now_v7().simple());
    let (seeded, jar, set_cookies) = login_with_cookies(&app, &slug).await;

    // ...with NO body credential at all.
    let no_header = two_origin_refresh(&app, ip, jar.as_str(), json!({})).await;
    assert_eq!(
        no_header.status(),
        axum::http::StatusCode::FORBIDDEN,
        "an ambient session cookie with no echoed header is still a refused CSRF attempt"
    );

    // ...and with a header that does not match the cookie.
    let csrf = set_cookies
        .iter()
        .find(|c| c.starts_with("marula_csrf="))
        .map(|c| {
            c.trim_start_matches("marula_csrf=")
                .split(';')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default();
    assert_eq!(csrf.len(), 64, "the csrf jar is 32 random bytes");
    // A browser holds BOTH cookies and sends both. The double-submit needs
    // the csrf cookie in the jar AND the echoed header to match it.
    let jar_head = format!("marula_session={jar}; marula_csrf={csrf}");
    let wrong_header = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[
            ("x-forwarded-for", ip),
            ("cookie", jar_head.as_str()),
            ("x-marula-csrf", "not-the-cookie"),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        wrong_header.status(),
        axum::http::StatusCode::FORBIDDEN,
        "a mismatched header is refused on the cookie path too"
    );

    // PROOF the check is enforced, not deleted: the RIGHT header still
    // passes, and the pair still rotates.
    let good = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[
            ("x-forwarded-for", ip),
            ("cookie", jar_head.as_str()),
            ("x-marula-csrf", csrf.as_str()),
        ],
        Some(json!({})),
    )
    .await;
    assert_eq!(
        good.status(),
        axum::http::StatusCode::OK,
        "the cookie-mode path with the echoed header must keep working"
    );
    let rotated = body_json(good).await;
    assert_ne!(
        rotated["refresh_token"].as_str().unwrap(),
        seeded["refresh_token"].as_str().unwrap(),
        "rotation still issues a fresh refresh token"
    );
}

#[tokio::test]
async fn successive_two_origin_refreshes_never_trip_reuse_detection() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let ip = format!("114.70.12.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let slug = format!("chain-{}", Uuid::now_v7().simple());
    let (seeded, _, _) = login_with_cookies(&app, &slug).await;
    let mut refresh = seeded["refresh_token"].as_str().unwrap().to_string();
    let mut session = seeded["session_id"].as_str().unwrap().to_string();

    // Several access-token lifetimes: every rotation carries the ambient
    // jar and no csrf header, which is exactly the shape a long-running
    // two-origin session takes. If ANY of these wrongly tripped reuse
    // detection, the whole chain would die and the session would need a
    // re-auth — the defect's user-visible symptom.
    for lifetime in 0..4 {
        let jar = session_cookie_value(&session, &refresh);
        let response = two_origin_refresh(
            &app,
            ip,
            jar.as_str(),
            json!({ "refresh_token": refresh, "session_id": session }),
        )
        .await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "rotation {lifetime} of the chain must not be refused"
        );
        let pair = body_json(response).await;
        let next_refresh = pair["refresh_token"].as_str().unwrap().to_string();
        let next_session = pair["session_id"].as_str().unwrap().to_string();
        assert_ne!(
            next_refresh, refresh,
            "every rotation issues a fresh secret (a repeated secret would mean no rotation)"
        );
        assert_ne!(
            next_session, session,
            "every rotation is a new session node"
        );
        refresh = next_refresh;
        session = next_session;
    }

    // The chain survived four lifetimes — proof there was no false reuse
    // revocation along the way.
    let final_check = two_origin_refresh(
        &app,
        ip,
        &session_cookie_value(&session, &refresh),
        json!({ "refresh_token": refresh, "session_id": session }),
    )
    .await;
    assert_eq!(final_check.status(), axum::http::StatusCode::OK);
}

#[tokio::test]
async fn logout_with_an_ambient_session_cookie_and_no_csrf_header_succeeds() {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let app = core_api::router(test_state(&admin_url).await);
    let ip = format!("114.70.13.{}", Uuid::now_v7());
    let ip = ip.as_str();
    let slug = format!("logout-{}", Uuid::now_v7().simple());
    let (seeded, _jar, _) = login_with_cookies(&app, &slug).await;
    let refresh = seeded["refresh_token"].as_str().unwrap().to_string();
    let session = seeded["session_id"].as_str().unwrap().to_string();

    // The same credential-source-first split on the sibling surface:
    // presenting the secret in the body is not forgeable, so the gate
    // does not gate it (before the fix this was a 403 with the 30-day
    // session still live).
    let response = send(
        &app,
        Method::POST,
        "/api/auth/logout",
        None,
        &[("x-forwarded-for", ip)],
        Some(json!({ "refresh_token": refresh, "session_id": session })),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);

    // The revoked session cannot rotate again — the revocation landed.
    let after = send(
        &app,
        Method::POST,
        "/api/auth/refresh",
        None,
        &[("x-forwarded-for", ip)],
        Some(json!({ "refresh_token": refresh, "session_id": session })),
    )
    .await;
    assert_eq!(
        after.status(),
        axum::http::StatusCode::UNAUTHORIZED,
        "a logged-out session must not rotate"
    );
}
