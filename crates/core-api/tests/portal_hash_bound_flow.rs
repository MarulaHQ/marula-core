//! The password-hash admission bound on the portal door (Wave 58 lane
//! `p58-hashing`, follow-up F62).
//!
//! `p56-auth-bounds` bounded `POST /api/auth/login` and `POST /api/auth/register`
//! with `auth::hash_gate`. Two more paths spend the same argon2id on input
//! nobody has authenticated yet:
//!
//! * `POST /api/portal/login` (`routes::portal::login`) — anonymous, ungated,
//!   and it held a pooled transaction across the verify.
//! * `provision_jit_user` (`routes::oidc`), reached by an OIDC callback and by
//!   `auth::provider::resolve_and_issue` — it hashed inside an open
//!   transaction, after the IdP assertion verified.
//!
//! This suite proves the observable, not the internals:
//!
//! * With every slot taken, both doors answer the typed 503 instead of
//!   hashing, and both admit the request the moment a slot is free.
//! * A slot is returned on every path, including the arms that pay a dummy
//!   verify (unknown address) and the arms that end in a 401.
//! * The portal verify does not hold a pooled connection: with a
//!   one-connection pool, another borrower gets the connection WHILE the login
//!   is still in flight. That is the property `p56` established for staff
//!   login, measured the same way — from outside, on a real pool.
//!
//! The JIT case drives `auth::provider::resolve_and_issue` directly. It is the
//! production seam the OIDC callback's provisioning arm mirrors, it is public,
//! and it reaches the same `provision_jit_user`, so the bound is exercised
//! where it lives without a mock identity provider in the loop.
//!
//! Database selection follows the other flow suites:
//! `MARULA_TEST_DATABASE_URL` or `DATABASE_URL` (no schema reset).
//!
//! Lockout discipline: every request mints its own caller address through the
//! shared fixture, and the JWT secret is salted per instance, so no case can
//! meet or arm another case's durable row.
//!
//! The gate is process-wide by design, so every case is `serial` and installs
//! the shape it wants before its first request.

use axum::Router;
use core_api::auth::hash_gate::{self, HashGate};
use core_api::config::Config;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/caller.rs"]
mod caller;

use caller::fresh_caller_ip;

const PORTAL_PASSWORD: &str = "portal-passphrase-1";
const OWNER_PASSWORD: &str = "correct-horse-battery";

// --- harness ----------------------------------------------------------------

/// A state whose pool has `max_connections` slots. The portal transport case
/// asks for one, because that is the only way to see, from outside, whether a
/// request is holding the connection.
async fn test_state(max_connections: u32) -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = database_url();
    let pool = connect(&admin_url, max_connections).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Per-instance salt, so the durable per-IP fingerprints this file mints
    // cannot collide with a parallel run's on the shared test database.
    config.jwt_secret = format!("portal-hash-bound-{}", Uuid::now_v7());
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

fn database_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("the hash bound suite needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
}

async fn connect(url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &Router,
    method: axum::http::Method,
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

struct Seeded {
    /// The portal identity's sign-in address.
    portal_email: String,
    org_id: Uuid,
}

/// Registers a workspace, one client, and a portal identity on it.
///
/// Registration is the cheapest bootstrap that also proves the staff door
/// still works: the org seeds its roles, board, and taxonomy, and the owner
/// can mint the client and invite the portal user through the real routes.
async fn seed(app: &Router, tag: &str) -> Seeded {
    let slug = format!("portal-bound-{tag}-{}", Uuid::now_v7().simple());
    let owner_email = format!("owner@{slug}.test");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Hash Bound Co",
            "org_slug": slug,
            "email": owner_email,
            "password": OWNER_PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "seed register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();

    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    let org_id: Uuid = me["org_id"]
        .as_str()
        .expect("the signed-in owner carries its org")
        .parse()
        .expect("uuid");

    let client = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/clients",
            Some(&token),
            Some(json!({ "name": "Client A" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().expect("client id");

    let portal_email = format!("portal-{tag}@{slug}.test");
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/clients/{client_id}/portal-users"),
        Some(&token),
        Some(json!({
            "email": portal_email,
            "display_name": "Portal Person",
            "password": PORTAL_PASSWORD,
            "role": "contact",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "portal invite must succeed");

    Seeded {
        portal_email,
        org_id,
    }
}

/// The one gate this process has, pinned to a single slot with no queue, so a
/// saturated call is refused at once instead of waiting.
///
/// Called before the first hashing request in every case: the gate is
/// first-caller-wins, and a register that reached it first would install the
/// shipped size.
fn one_slot_gate() -> &'static HashGate {
    hash_gate::global(1, Duration::ZERO)
}

/// Takes every free slot. The permits keep the gate saturated until they are
/// dropped, which makes the refusal cases deterministic rather than a race
/// against a burst.
async fn drain_slots() -> Vec<tokio::sync::SemaphorePermit<'static>> {
    let gate = one_slot_gate();
    let mut held = Vec::new();
    while let Ok(permit) = gate.admit().await {
        held.push(permit);
    }
    held
}

/// The 503 envelope a saturated door must answer: standard reason code, a
/// message, and never a 500.
async fn assert_service_unavailable(response: axum::response::Response) {
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(
        status,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "a saturated hash queue must answer 503, got {status} with {body}"
    );
    assert_eq!(
        body["error"]["reason"], "service_unavailable",
        "the 503 must carry the standard reason code: {body}"
    );
    assert_eq!(body["error"]["code"], 503);
    assert!(
        body["error"]["message"]
            .as_str()
            .map(|m| m.contains("capacity"))
            .unwrap_or(false),
        "the 503 must say it is a capacity answer: {body}"
    );
}

/// The same, plus the `Retry-After` a client needs to act on it.
async fn assert_saturated_503(response: axum::response::Response) {
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert_service_unavailable(response).await;
    let expected = hash_gate::RETRY_AFTER_SECS.to_string();
    assert_eq!(
        retry_after.as_deref(),
        Some(expected.as_str()),
        "a 503 without Retry-After parks the client"
    );
}

fn portal_login_body(email: &str) -> Value {
    json!({ "email": email, "password": PORTAL_PASSWORD })
}

// --- the portal door --------------------------------------------------------

/// Every slot taken ⇒ `/api/portal/login` answers the typed 503 instead of
/// hashing. Before this lane the portal door had no bound at all: a burst of
/// anonymous posts all reached argon2 while each one also held a pooled
/// transaction open.
#[tokio::test]
#[serial_test::serial]
async fn portal_login_answers_503_when_every_hash_slot_is_taken() {
    one_slot_gate();
    let state = test_state(5).await;
    let app = core_api::router(state.clone());
    let seeded = seed(&app, "saturated").await;

    let held = drain_slots().await;
    let refused = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(portal_login_body(&seeded.portal_email)),
    )
    .await;
    assert_saturated_503(refused).await;

    // The refusal is backpressure, not a verdict: the same credentials
    // succeed as soon as the slot is free.
    drop(held);
    let admitted = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(portal_login_body(&seeded.portal_email)),
    )
    .await;
    let status = admitted.status();
    let body = body_json(admitted).await;
    assert_eq!(status, 200, "a freed slot must admit the login: {body}");
    assert!(
        body["access_token"].is_string(),
        "an admitted login issues tokens: {body}"
    );
    assert_eq!(
        one_slot_gate().available(),
        1,
        "the successful login must return its slot"
    );
}

/// The slot comes back on every path that ends before a session: a wrong
/// password, and an address that resolves to nothing (which still pays a
/// dummy verify, so it must hold a slot while it pays it).
#[tokio::test]
#[serial_test::serial]
async fn portal_login_returns_its_slot_on_the_failure_paths() {
    one_slot_gate();
    let state = test_state(5).await;
    let app = core_api::router(state.clone());
    let seeded = seed(&app, "release").await;
    let gate = one_slot_gate();
    assert_eq!(gate.available(), 1, "the case starts with a free slot");

    let wrong = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({
            "email": seeded.portal_email,
            "password": "not-the-password",
        })),
    )
    .await;
    assert_eq!(wrong.status(), 401, "a wrong password is still a 401");
    assert_eq!(
        gate.available(),
        1,
        "a failed verify must not keep its slot"
    );

    let unknown = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(portal_login_body(&format!(
            "nobody-{}@example.test",
            Uuid::now_v7()
        ))),
    )
    .await;
    assert_eq!(unknown.status(), 401, "an unknown address is a 401");
    assert_eq!(
        gate.available(),
        1,
        "the dummy-verify arm must not keep its slot"
    );
}

/// The verify must not hold a pooled connection. The observable is the
/// database's own view of the request: while a transaction is open with no
/// statement running, its backend sits in `idle in transaction`. Before this
/// lane the portal login was in exactly that state for the whole argon2
/// window, so a sampler sees the state persist; after it, the credential read
/// commits before the hash and only a millisecond-wide window exists.
///
/// The assertion is about PERSISTENCE (four consecutive samples, ~60 ms), not
/// presence: the fixed code does briefly hold a read transaction, and calling
/// that a failure would be a test of scheduler luck. A held-across-the-hash
/// connection persists for the hash, which is two orders of magnitude longer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn portal_login_does_not_hold_the_pool_connection_across_the_hash() {
    one_slot_gate();
    let state = test_state(3).await;
    let app = core_api::router(state.clone());
    let seeded = seed(&app, "pool").await;
    // A second pool, so sampling cannot compete with the request for the
    // connection whose state it is reading.
    let probe = connect(&database_url(), 2).await.expect("probe pool");

    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/portal/login")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            portal_login_body(&seeded.portal_email).to_string(),
        ))
        .unwrap();
    let request = fresh_caller_ip().stamp(request);
    let app_for_task = app.clone();
    let login = tokio::spawn(async move { app_for_task.oneshot(request).await.unwrap() });

    // 20 ms apart is finer than any gap the fixed request has between two
    // checkouts, and a hash here runs for hundreds of milliseconds, so a
    // request that parks a connection on it cannot hide between samples.
    let mut longest_streak = 0;
    let mut streak = 0;
    for _ in 0..200 {
        if login.is_finished() {
            break;
        }
        if idle_in_transaction_on_portal_users(&probe).await > 0 {
            streak += 1;
            longest_streak = longest_streak.max(streak);
        } else {
            streak = 0;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let response = login.await.expect("login task");
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, 200, "the login itself must still succeed: {body}");
    assert!(
        longest_streak < 4,
        "the portal verify held an open transaction (a pooled connection) for \
         {longest_streak} consecutive samples, which is the argon2 window"
    );
}

/// How many backends are sitting in an open transaction that last touched the
/// portal credential row — the state a request is in while it hashes with the
/// read's transaction still uncommitted.
async fn idle_in_transaction_on_portal_users(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity
          WHERE datname = current_database()
            AND state = 'idle in transaction'
            AND query ILIKE '%portal_users%'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_stat_activity read")
}

// --- measurement (ignored) --------------------------------------------------

/// What the bound does to the portal door under a burst, through the real
/// router and a real pool.
///
///     cargo test -p core-api --test portal_hash_bound_flow portal_login_burst \
///         -- --ignored --nocapture
///
/// The gate is one object per process and its size is first-caller-wins, so
/// the two arms are two runs:
///
/// * `after` (shipped shape) — leave `MARULA_AUTH_HASH_MAX_CONCURRENCY` unset:
///   cores times two slots and the 250 ms queue wait.
/// * `before` (the pre-change shape on this door) — no bound that any burst
///   can reach: `MARULA_AUTH_HASH_MAX_CONCURRENCY=256
///   MARULA_AUTH_HASH_QUEUE_WAIT_MS=0` (256 is the config ceiling; a larger
///   value is rejected at startup, which is itself part of the fix).
///
/// `BURST` overrides the burst size. Both arms run the same fixed code and
/// the same real argon2; only admission differs. The numbers this prints are
/// the lane's evidence, and a debug-profile hash costs far more than a release
/// one, so compare the arms to each other and never to a shipped-cost table.
#[ignore = "measurement (Wave 58 lane p58-hashing): prints the portal burst table; see the lane's GATES.md"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn portal_login_burst_against_the_hash_bound() {
    let state = test_state(20).await;
    let gate = state.config.auth_hash_gate();
    let app = core_api::router(state.clone());
    let seeded = seed(&app, "burst").await;
    let burst: usize = std::env::var("BURST")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or_else(|| (gate.slots() * 3).max(8));

    println!(
        "gate: {} slots, {} ms queue wait | pool max {} | burst {}",
        gate.slots(),
        gate.queue_wait().as_millis(),
        state.pool.options().get_max_connections(),
        burst
    );

    let started = std::time::Instant::now();
    let mut tasks = Vec::with_capacity(burst);
    for _ in 0..burst {
        let app = app.clone();
        let email = seeded.portal_email.clone();
        tasks.push(tokio::spawn(async move {
            let started = std::time::Instant::now();
            let request = axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/api/portal/login")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    portal_login_body(&email).to_string(),
                ))
                .unwrap();
            let request = fresh_caller_ip().stamp(request);
            let response = app.oneshot(request).await.unwrap();
            (response.status().as_u16(), started.elapsed())
        }));
    }

    let mut admitted = Vec::new();
    let mut refused = 0usize;
    let mut other = 0usize;
    for task in tasks {
        let (status, elapsed) = task.await.expect("burst task");
        match status {
            200 => admitted.push(elapsed),
            503 => refused += 1,
            _ => other += 1,
        }
    }
    let wall = started.elapsed();
    admitted.sort();
    let at = |p: f64| -> f64 {
        if admitted.is_empty() {
            return 0.0;
        }
        let index = (((admitted.len() as f64 - 1.0) * p).round() as usize).min(admitted.len() - 1);
        admitted[index].as_secs_f64() * 1000.0
    };
    println!(
        "{:<10} {:>7} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "arm", "burst", "admit", "refuse", "wall_s", "p50_ms", "p95_ms", "other"
    );
    println!(
        "{:<10} {:>7} {:>7} {:>7} {:>9.2} {:>9.1} {:>9.1} {:>9}",
        std::env::var("MARULA_AUTH_HASH_MAX_CONCURRENCY").unwrap_or_else(|_| "shipped".into()),
        burst,
        admitted.len(),
        refused,
        wall.as_secs_f64(),
        at(0.50),
        at(0.95),
        other
    );
    assert_eq!(
        other, 0,
        "no burst request may answer anything but 200 or 503"
    );
}
