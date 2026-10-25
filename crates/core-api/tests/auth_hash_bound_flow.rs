//! The password-hash admission bound on the two unauthenticated auth paths
//! (Wave 56 lane `p56-auth-bounds`).
//!
//! Two kinds of test live here, the same split the timing suite uses:
//!
//! * The ordinary suite proves the OBSERVABLE the ruling names: with every
//!   hash slot taken, `POST /api/auth/login` and `POST /api/auth/register`
//!   answer 503 with a `Retry-After` header and the standard
//!   `service_unavailable` envelope — never a 500, never a request parked on
//!   an unbounded wait — and the same request succeeds the moment a slot is
//!   freed. The slots are drained through the same process-wide gate the
//!   handlers take them from, so the saturation is real, not simulated.
//! * `login_burst_against_the_hash_bound` is `#[ignore]`d: it is the
//!   measurement that produces the burst table (latency percentiles, refusals
//!   and the state of an unrelated authenticated request during the burst) for
//!   the lane's evidence. It belongs to the group gate's run.
//!
//! Both need `MARULA_TEST_DATABASE_URL` (or `DATABASE_URL`).
//!
//! Lockout discipline: every request carries its OWN caller address minted by
//! the shared fixture, so no case meets another case's armed per-IP lock.
//!
//! The gate is process-wide by design, so every case here is `serial` and
//! starts by installing the shape it wants (one slot, no queue).

use core_api::auth::hash_gate::{self, HashGate};
use core_api::config::Config;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/caller.rs"]
mod caller;

use caller::fresh_caller_ip;

// --- harness ----------------------------------------------------------------

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("auth hash bound needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool: PgPool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Per-instance salt, so the durable per-IP fingerprints this file mints
    // cannot collide with a parallel run's on the shared test database.
    config.jwt_secret = format!("auth-hash-bound-{}", Uuid::now_v7());
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn send(
    app: &axum::Router,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
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

const OWNER_PASSWORD: &str = "correct-horse-battery-11";

/// Registers an org and returns its owner address plus the access token.
async fn seed(app: &axum::Router, tag: &str) -> (String, String) {
    let slug = format!("auth-bound-{tag}-{}", Uuid::now_v7().simple());
    let owner_email = format!("owner@{slug}.test");
    let response = send(
        app,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Bound Co",
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
    (owner_email, token)
}

/// The one gate this process has, pinned to a single slot with no queue so a
/// saturated call is refused immediately instead of waiting.
fn one_slot_gate() -> &'static HashGate {
    hash_gate::global(1, Duration::ZERO)
}

/// Takes every free slot. The returned permits keep the gate saturated until
/// they are dropped, which is what makes the refusal cases deterministic
/// rather than a race against a burst.
async fn drain_slots() -> Vec<tokio::sync::SemaphorePermit<'static>> {
    let gate = one_slot_gate();
    let mut held = Vec::new();
    while let Ok(permit) = gate.admit().await {
        held.push(permit);
    }
    held
}

/// Asserts the 503 shape the ruling asks for: the standard envelope, a
/// `Retry-After` a client can act on, and no 500.
async fn assert_saturated_503(response: axum::response::Response) {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
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
        body["error"]["message"].is_string()
            && !body["error"]["message"].as_str().unwrap().is_empty(),
        "the 503 must explain itself: {body}"
    );
    let expected_retry = hash_gate::RETRY_AFTER_SECS.to_string();
    assert_eq!(
        retry_after.as_deref(),
        Some(expected_retry.as_str()),
        "a 503 without Retry-After parks the client"
    );
}

// --- login ------------------------------------------------------------------

/// Every slot taken ⇒ login answers the typed 503 instead of hashing. The
/// audit's burst shape exactly: no lock row exists yet, so the durable gate
/// cannot stop the first wave, and this bound is what does.
#[tokio::test]
#[serial_test::serial]
async fn login_answers_503_with_retry_after_when_the_hash_slots_are_taken() {
    let state = test_state().await;
    let app = core_api::router(state);
    let (email, _token) = seed(&app, "login-saturated").await;

    let held = drain_slots().await;
    let refused = send(
        &app,
        "/api/auth/login",
        &[("x-forwarded-for", &fresh_caller_ip().to_string())],
        Some(json!({ "email": email, "password": OWNER_PASSWORD })),
    )
    .await;
    assert_saturated_503(refused).await;
    drop(held);

    // The bound is a queue, not a wall: the same credentials succeed as soon
    // as a slot is free.
    let admitted = send(
        &app,
        "/api/auth/login",
        &[("x-forwarded-for", &fresh_caller_ip().to_string())],
        Some(json!({ "email": email, "password": OWNER_PASSWORD })),
    )
    .await;
    assert_eq!(
        admitted.status(),
        200,
        "a freed slot must admit the same login"
    );
}

// --- register ---------------------------------------------------------------

/// Registration has no throttle at all, so the bound is the only thing
/// between an anonymous flood and the CPU. Saturated ⇒ 503 with Retry-After,
/// and nothing half-created in the database.
#[tokio::test]
#[serial_test::serial]
async fn register_answers_503_with_retry_after_when_the_hash_slots_are_taken() {
    let state = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::router(state);

    let held = drain_slots().await;
    let slug = format!("auth-bound-refused-{}", Uuid::now_v7().simple());
    let refused = send(
        &app,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Never Created",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": OWNER_PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_saturated_503(refused).await;
    drop(held);

    // A refusal happens before the transaction opens, so no org row exists.
    let created: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_optional(&pool)
        .await
        .expect("org lookup");
    assert!(
        created.is_none(),
        "a refused registration must not leave an org behind"
    );

    // And the same registration goes through once a slot is free.
    let admitted = send(
        &app,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Created Now",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": OWNER_PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(
        admitted.status(),
        200,
        "a freed slot must admit the same registration"
    );
}

// --- measurement (ignored) --------------------------------------------------

/// The burst table: N simultaneous password logins through the real router,
/// timed, plus an unrelated AUTHENTICATED request sampled throughout, which
/// is the pool-starvation observable the audit asked for.
///
///     MARULA_TEST_DATABASE_URL=... cargo test -p core-api --test auth_hash_bound_flow \
///       login_burst_against_the_hash_bound -- --ignored --nocapture
///
/// Two arms, because the bound is installed once per process:
///
/// * `after`  — the shipped bound (`MARULA_AUTH_HASH_MAX_CONCURRENCY` unset ⇒
///   cores × 2, queue wait 250 ms).
/// * `before` — the pre-change shape, simulated by an effectively unbounded
///   gate: `MARULA_AUTH_HASH_MAX_CONCURRENCY=1024 MARULA_AUTH_HASH_QUEUE_WAIT_MS=0`.
///
/// `BURST` (default the bound + its third) sets how many logins fire at once.
#[ignore = "measurement (Wave 56 lane auth-bounds): needs MARULA_TEST_DATABASE_URL; see internal/design/qa-p56-auth-bounds/GATES.md"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn login_burst_against_the_hash_bound() {
    let state = test_state().await;
    let bound = state.config.auth_hash_max_concurrency;
    let wait = state.config.auth_hash_queue_wait_ms;
    let burst: usize = std::env::var("BURST")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(bound + bound / 3);
    let app = core_api::router(state);
    let (email, token) = seed(&app, "burst").await;
    // The gate the handlers will take slots from: same process, same one.
    let gate = state_gate();
    println!(
        "bound {} slots, queue wait {} ms, burst {burst}, installed {} slots",
        bound,
        wait,
        gate.slots()
    );

    let probe = {
        let app = app.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let mut samples: Vec<Duration> = Vec::new();
            for _ in 0..40 {
                let started = Instant::now();
                let response = app
                    .clone()
                    .oneshot(
                        axum::http::Request::builder()
                            .method(axum::http::Method::GET)
                            .uri("/api/me")
                            .header("authorization", format!("Bearer {token}"))
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .expect("probe request");
                assert_eq!(response.status(), 200, "the probe must stay authenticated");
                samples.push(started.elapsed());
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            samples
        })
    };

    let started = Instant::now();
    let mut tasks = Vec::with_capacity(burst);
    for _ in 0..burst {
        let app = app.clone();
        let email = email.clone();
        let caller = fresh_caller_ip();
        tasks.push(tokio::spawn(async move {
            let request_started = Instant::now();
            // A wrong password keeps the arm out of token issuance: what is
            // measured is the hash, not the session write.
            let response = send(
                &app,
                "/api/auth/login",
                &[("x-forwarded-for", &caller.to_string())],
                Some(json!({ "email": email, "password": "wrong-password-11" })),
            )
            .await;
            (response.status(), request_started.elapsed())
        }));
    }

    let mut admitted: Vec<Duration> = Vec::new();
    let mut refused = 0usize;
    let mut other = 0usize;
    for task in tasks {
        let (status, elapsed) = task.await.expect("login task");
        match status {
            axum::http::StatusCode::UNAUTHORIZED => admitted.push(elapsed),
            axum::http::StatusCode::SERVICE_UNAVAILABLE => refused += 1,
            _ => other += 1,
        }
    }
    let wall = started.elapsed();
    let mut probe_samples = probe.await.expect("probe task");
    probe_samples.sort();
    admitted.sort();

    let pct = |values: &[Duration], p: f64| -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let index = (((values.len() as f64 - 1.0) * p).round() as usize).min(values.len() - 1);
        values[index].as_secs_f64() * 1000.0
    };
    let max = |values: &[Duration]| -> f64 {
        values
            .last()
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    };
    println!(
        "logins admitted {:>3} refused(503) {:>3} other {:>3} wall {:>7.1} ms",
        admitted.len(),
        refused,
        other,
        wall.as_secs_f64() * 1000.0
    );
    println!(
        "login latency p50 {:>7.1} ms p95 {:>7.1} ms max {:>7.1} ms",
        pct(&admitted, 0.50),
        pct(&admitted, 0.95),
        max(&admitted)
    );
    println!(
        "unrelated GET /api/me during the burst: p50 {:>7.1} ms p95 {:>7.1} ms max {:>7.1} ms",
        pct(&probe_samples, 0.50),
        pct(&probe_samples, 0.95),
        max(&probe_samples)
    );
}

/// The config-installed gate, for the measurement's banner.
fn state_gate() -> &'static HashGate {
    hash_gate::installed().unwrap_or_else(|| hash_gate::global(1, Duration::ZERO))
}
