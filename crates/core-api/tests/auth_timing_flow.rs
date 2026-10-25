//! Account-existence timing parity on BOTH password login arms (Wave 54 lane
//! S2c).
//!
//! Two kinds of test live here and the difference is deliberate:
//!
//! * The `#[tokio::test]` pair runs in the ordinary DB suite and asserts an
//!   OBSERVABLE: an address that resolves to nothing performs exactly one
//!   dummy argon2 verify and answers the same envelope a wrong password
//!   answers. No wall clock is asserted — a millisecond bound in CI is a
//!   flake, not a security property.
//! * `password_login_timing_pairs_both_arms` is `#[ignore]`d. It is the
//!   measurement that produces the n / median / p95 / ratio table for the
//!   lane's evidence, and it belongs to the group gate's run, never to the
//!   unit suite. Exact command: internal/design/qa-p54-argon2-timing/GATES.md.
//!
//! Lockout discipline: every request below carries its OWN caller address,
//! minted by the shared fixture (`support/caller.rs`, Wave 54 follow-up F7).
//! `app_auth_backoff_secs(1)` is 30 seconds, so a single failed login locks the
//! per-IP key it used — a shared bucket would gate the next request and the
//! harness would measure the gate instead of argon2. The per-actor key is
//! cleared between samples with the same `gate_clear` the login path calls
//! after a success, and the pre-sample assertion proves it is clear.

use core_api::{auth::password, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
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
        .expect("auth timing needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool: PgPool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&admin_url)
        .await
        .expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    // Per-instance salt, so the durable per-IP fingerprints this file mints
    // cannot collide with a parallel run's on the shared test database.
    config.jwt_secret = format!("auth-timing-{}", Uuid::now_v7());
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
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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

fn email_key(email: &str) -> String {
    // Mirrors `hardening::email_fingerprint`, which is private: the per-actor
    // key is the trimmed, lowercased address (the IP half is salted and is
    // never reused here — every request carries a fresh bucket).
    format!("email:{}", email.trim().to_lowercase())
}

const OWNER_PASSWORD: &str = "correct-horse-battery-11";
const PORTAL_PASSWORD: &str = "portal-passphrase-1";
/// Sent to BOTH arms: the same string, the same shape. To the resolved arm it
/// is a wrong password; to the unresolved arm it is all the dummy verify gets.
const WALK_IN_PASSWORD: &str = "wrong-password-11";

struct Seeded {
    owner_email: String,
    portal_email: String,
}

async fn seed(app: &axum::Router, tag: &str) -> Seeded {
    let slug = format!("auth-timing-{tag}-{}", Uuid::now_v7().simple());
    let owner_email = format!("owner@{slug}.test");
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Timing Co",
            "org_slug": slug,
            "email": owner_email,
            "password": OWNER_PASSWORD,
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();

    let client = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/clients",
            &[("authorization", &format!("Bearer {token}"))],
            Some(json!({"name": "Timing Client"})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .expect("client id")
        .to_string();

    let portal_email = format!("contact@{slug}.test");
    let invite = send(
        app,
        axum::http::Method::POST,
        &format!("/api/clients/{client}/portal-users"),
        &[("authorization", &format!("Bearer {token}"))],
        Some(json!({
            "email": portal_email,
            "display_name": "Portal Person",
            "password": PORTAL_PASSWORD,
            "role": "contact",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200, "portal invite must succeed");

    Seeded {
        owner_email,
        portal_email,
    }
}

/// One login attempt from its own IP bucket, returning the wall clock in
/// milliseconds and pinning the uniform 401 on the way out.
async fn timed_login(app: &axum::Router, path: &str, ip: &str, email: &str, password: &str) -> f64 {
    let started = std::time::Instant::now();
    let response = send(
        app,
        axum::http::Method::POST,
        path,
        &[("x-forwarded-for", ip)],
        Some(json!({"email": email, "password": password})),
    )
    .await;
    let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNAUTHORIZED,
        "{path} must answer the uniform 401 for {email} from {ip}"
    );
    elapsed
}

// --- assertions that run in the ordinary suite ------------------------------

/// The unresolved staff arm pays one verify. The `+1` is the observable that
/// makes this a test of the equalisation rather than of the source comment,
/// and the body comparison proves the typed answer did not move.
#[tokio::test]
#[serial_test::serial]
async fn unregistered_staff_login_spends_a_dummy_verify_and_answers_the_uniform_401() {
    let state = test_state().await;
    let app = core_api::router(state);
    let seeded = seed(&app, "staff-absent").await;
    let absent = format!("absent-{}@timing.invalid", Uuid::now_v7().simple());
    let unresolved_caller = fresh_caller_ip();

    let before = password::dummy_verify_count();
    let unresolved = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[unresolved_caller.forwarded_for()],
        Some(json!({"email": absent, "password": WALK_IN_PASSWORD})),
    )
    .await;
    assert_eq!(unresolved.status(), axum::http::StatusCode::UNAUTHORIZED);
    let unresolved_body = body_json(unresolved).await;
    assert_eq!(
        password::dummy_verify_count(),
        before + 1,
        "an address that resolves to nothing must still pay exactly one argon2 verify"
    );

    let resolved_caller = fresh_caller_ip();
    let resolved = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[resolved_caller.forwarded_for()],
        Some(json!({"email": seeded.owner_email, "password": WALK_IN_PASSWORD})),
    )
    .await;
    assert_eq!(resolved.status(), axum::http::StatusCode::UNAUTHORIZED);
    let resolved_body = body_json(resolved).await;

    assert_eq!(
        unresolved_body["error"]["reason"], "unauthorized",
        "the unresolved arm keeps the typed answer"
    );
    assert_eq!(
        unresolved_body, resolved_body,
        "an address that does not exist and a wrong password for one that does must be \
         byte-identical answers"
    );
    assert_eq!(
        password::dummy_verify_count(),
        before + 1,
        "the resolved arm verifies the real hash, never the stand-in"
    );
}

/// Same shape on the portal leg that Wave 54 lane S2a gated.
#[tokio::test]
#[serial_test::serial]
async fn unregistered_portal_login_spends_a_dummy_verify_and_answers_the_uniform_401() {
    let state = test_state().await;
    let app = core_api::router(state);
    let seeded = seed(&app, "portal-absent").await;
    let absent = format!("absent-{}@timing.invalid", Uuid::now_v7().simple());
    let unresolved_caller = fresh_caller_ip();

    let before = password::dummy_verify_count();
    let unresolved = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        &[unresolved_caller.forwarded_for()],
        Some(json!({"email": absent, "password": WALK_IN_PASSWORD})),
    )
    .await;
    assert_eq!(unresolved.status(), axum::http::StatusCode::UNAUTHORIZED);
    let unresolved_body = body_json(unresolved).await;
    assert_eq!(
        password::dummy_verify_count(),
        before + 1,
        "the portal's unresolved arm must still pay exactly one argon2 verify"
    );

    let resolved_caller = fresh_caller_ip();
    let resolved = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/login",
        &[resolved_caller.forwarded_for()],
        Some(json!({"email": seeded.portal_email, "password": WALK_IN_PASSWORD})),
    )
    .await;
    assert_eq!(resolved.status(), axum::http::StatusCode::UNAUTHORIZED);
    let resolved_body = body_json(resolved).await;

    assert_eq!(
        unresolved_body, resolved_body,
        "the portal's two 401 arms must stay byte-identical"
    );
    assert_eq!(
        password::dummy_verify_count(),
        before + 1,
        "the portal's resolved arm verifies the real hash, never the stand-in"
    );
}

// --- the measurement (group gate only) --------------------------------------

fn percentile_ms(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p * sorted.len() as f64).ceil() as usize).saturating_sub(1);
    sorted[rank.min(sorted.len() - 1)]
}

fn round3(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
}

struct Leg {
    n: usize,
    median_ms: f64,
    p95_ms: f64,
}

impl Leg {
    fn from_samples(mut samples: Vec<f64>) -> Self {
        samples.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        Leg {
            n: samples.len(),
            median_ms: percentile_ms(&samples, 0.5),
            p95_ms: percentile_ms(&samples, 0.95),
        }
    }

    fn json(&self) -> Value {
        json!({
            "n": self.n,
            "median_ms": round3(self.median_ms),
            "p95_ms": round3(self.p95_ms),
        })
    }
}

/// Paired samples on one arm: for each pair, one login for an address that
/// exists (wrong password) and one for an address that does not (the same
/// wrong password). Returns the resolved and unresolved durations in
/// milliseconds.
struct Arm<'a> {
    app: &'a axum::Router,
    pool: &'a PgPool,
    kind: &'a str,
    path: &'a str,
    resolved_email: &'a str,
    unresolved_email: &'a str,
    samples: usize,
    warmup: usize,
}

async fn measure_arm(arm: Arm<'_>) -> (Vec<f64>, Vec<f64>) {
    let Arm {
        app,
        pool,
        kind,
        path,
        resolved_email,
        unresolved_email,
        samples,
        warmup,
    } = arm;
    let keys = vec![email_key(resolved_email), email_key(unresolved_email)];
    let mut resolved_ms = Vec::with_capacity(samples);
    let mut unresolved_ms = Vec::with_capacity(samples);

    for sample in 0..(warmup + samples) {
        // Validity gate: if the previous pair's clear missed (a changed key
        // shape, say), the lock would still be engaged and this pair would
        // measure the gate's short-circuit instead of argon2.
        let engaged = core_db::hardening::gate_check(pool, kind, &keys)
            .await
            .expect("gate_check");
        assert_eq!(
            engaged, 0,
            "sample {sample} starts with an engaged gate on {keys:?} — the harness is \
             measuring the lockout, not the verify"
        );

        let ip_resolved = fresh_caller_ip();
        let ip_unresolved = fresh_caller_ip();
        // Alternate which arm goes first so a monotonic drift (CPU heat, pool
        // warm-up) lands on both arms instead of on one.
        let (resolved, unresolved) = if sample % 2 == 0 {
            let r = timed_login(
                app,
                path,
                ip_resolved.as_str(),
                resolved_email,
                WALK_IN_PASSWORD,
            )
            .await;
            let u = timed_login(
                app,
                path,
                ip_unresolved.as_str(),
                unresolved_email,
                WALK_IN_PASSWORD,
            )
            .await;
            (r, u)
        } else {
            let u = timed_login(
                app,
                path,
                ip_unresolved.as_str(),
                unresolved_email,
                WALK_IN_PASSWORD,
            )
            .await;
            let r = timed_login(
                app,
                path,
                ip_resolved.as_str(),
                resolved_email,
                WALK_IN_PASSWORD,
            )
            .await;
            (r, u)
        };

        // The resolved arm must have RECORDED its failure: that is how the
        // harness knows the request reached the verify and was not answered
        // by the gate.
        let armed = core_db::hardening::gate_check(pool, kind, &[email_key(resolved_email)])
            .await
            .expect("gate_check");
        assert!(
            armed > 0,
            "the resolved arm did not record a failure on sample {sample}"
        );
        core_db::hardening::gate_clear(pool, kind, &keys)
            .await
            .expect("gate_clear");

        if sample >= warmup {
            resolved_ms.push(resolved);
            unresolved_ms.push(unresolved);
        }
    }
    (resolved_ms, unresolved_ms)
}

/// Ignored by default. Prints the paired table for both password login arms.
///
///     MARULA_TEST_DATABASE_URL=... cargo test -p core-api --test auth_timing_flow \
///       password_login_timing_pairs_both_arms -- --ignored --nocapture
///
/// `TIMING_SAMPLES` (default 60) and `TIMING_WARMUP` (default 5) tune it.
#[ignore = "measurement (Wave 54 lane S2c login timing): needs MARULA_TEST_DATABASE_URL; see internal/design/qa-p54-argon2-timing/GATES.md"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn password_login_timing_pairs_both_arms() {
    let samples: usize = std::env::var("TIMING_SAMPLES")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(60);
    let warmup: usize = std::env::var("TIMING_WARMUP")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(5);
    assert!(samples > 0, "TIMING_SAMPLES must be positive");

    let state = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::router(state);
    let seeded = seed(&app, "measure").await;
    let staff_absent = format!("absent-{}@timing.invalid", Uuid::now_v7().simple());
    let portal_absent = format!("absent-portal-{}@timing.invalid", Uuid::now_v7().simple());

    let (staff_resolved, staff_unresolved) = measure_arm(Arm {
        app: &app,
        pool: &pool,
        kind: core_db::hardening::gate_kind::PASSWORD_LOGIN,
        path: "/api/auth/login",
        resolved_email: &seeded.owner_email,
        unresolved_email: &staff_absent,
        samples,
        warmup,
    })
    .await;
    let (portal_resolved, portal_unresolved) = measure_arm(Arm {
        app: &app,
        pool: &pool,
        kind: core_db::hardening::gate_kind::PORTAL_PASSWORD_LOGIN,
        path: "/api/portal/login",
        resolved_email: &seeded.portal_email,
        unresolved_email: &portal_absent,
        samples,
        warmup,
    })
    .await;

    let rows = [
        ("staff/resolved", Leg::from_samples(staff_resolved)),
        ("staff/unresolved", Leg::from_samples(staff_unresolved)),
        ("portal/resolved", Leg::from_samples(portal_resolved)),
        ("portal/unresolved", Leg::from_samples(portal_unresolved)),
    ];
    let staff_ratio_median = rows[1].1.median_ms / rows[0].1.median_ms;
    let staff_ratio_p95 = rows[1].1.p95_ms / rows[0].1.p95_ms;
    let portal_ratio_median = rows[3].1.median_ms / rows[2].1.median_ms;
    let portal_ratio_p95 = rows[3].1.p95_ms / rows[2].1.p95_ms;

    println!(
        "\nAUTH_TIMING_META samples={samples} warmup={warmup} \
         password={WALK_IN_PASSWORD:?} in_process=oneshot"
    );
    println!("AUTH_TIMING_TABLE leg                n   median_ms     p95_ms");
    for (name, leg) in &rows {
        println!(
            "AUTH_TIMING_TABLE {name:<16} {:>4}   {:>9.3}   {:>9.3}",
            leg.n, leg.median_ms, leg.p95_ms
        );
    }
    println!("AUTH_TIMING_RATIO staff  median={staff_ratio_median:.3} p95={staff_ratio_p95:.3}");
    println!("AUTH_TIMING_RATIO portal median={portal_ratio_median:.3} p95={portal_ratio_p95:.3}");
    println!(
        "AUTH_TIMING_JSON {}",
        json!({
            "samples": samples,
            "warmup": warmup,
            "ratio_definition": "unresolved / resolved, 1.0 = the address cannot be timed",
            "staff": {
                "resolved": rows[0].1.json(),
                "unresolved": rows[1].1.json(),
                "ratio": { "median": round3(staff_ratio_median), "p95": round3(staff_ratio_p95) },
            },
            "portal": {
                "resolved": rows[2].1.json(),
                "unresolved": rows[3].1.json(),
                "ratio": { "median": round3(portal_ratio_median), "p95": round3(portal_ratio_p95) },
            },
        })
    );
}
