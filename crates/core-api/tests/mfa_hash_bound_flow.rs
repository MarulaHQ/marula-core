//! The password-hash admission bound on the signed-in MFA doors (Wave 59 lane
//! `p59-mfa-hash`).
//!
//! Five paths used to run argon2 with an organization transaction open and
//! without asking the process for a hash slot: `totp/disable`,
//! `recovery-codes/regenerate`, `mfa/step-up`, a passkey removal, and the
//! recovery-code loop that verifies one candidate per unused code. This suite
//! pins the guarantee the login doors already carry:
//!
//! * a saturated gate answers the typed 503 with `Retry-After` instead of
//!   hashing, and the same request goes through the moment a slot is freed;
//! * the recovery-code loop hashes with NO pooled connection held, measured on
//!   the shared pool while the request is in flight — that is the observable
//!   "one request no longer pays N hashes on a connection" names;
//! * each door still behaves on its success and failure arms.
//!
//! The gate is process-wide by design, so every case is `serial` and installs
//! the shape it wants (one slot, no queue) before the first request.
//!
//! Needs `MARULA_TEST_DATABASE_URL` (or `DATABASE_URL`).

use axum::http::{Method, StatusCode};
use core_api::auth::hash_gate::{self, HashGate};
use core_api::config::Config;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "correct-horse-battery-11";

// --- harness ----------------------------------------------------------------

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("mfa hash bound needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
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
    config.jwt_secret = format!("mfa-hash-bound-{}", Uuid::now_v7());
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

/// Registers an org and returns its owner's bearer token, session id, ids and
/// address.
async fn register_org(app: &axum::Router, slug: &str) -> Value {
    let tokens = body_json(
        send(
            app,
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({
                "org_name": "Hash Bound Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": PASSWORD,
                "display_name": "Owner",
            })),
        )
        .await,
    )
    .await;
    let bearer = tokens["access_token"]
        .as_str()
        .expect("register issues tokens")
        .to_string();
    let me = body_json(send(app, Method::GET, "/api/me", Some(&bearer), None).await).await;
    json!({
        "access_token": bearer,
        "session_id": tokens["session_id"],
        "user_id": me["user_id"],
        "org_id": me["org_id"],
        "email": format!("owner@{slug}.test"),
    })
}

fn totp_code(secret_base32: &str, offset_steps: i64) -> String {
    use totp_rs::{Algorithm, TOTP};
    let bytes = data_encoding::BASE32_NOPAD
        .decode(secret_base32.to_ascii_uppercase().as_bytes())
        .expect("stored secret is base32");
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("Marula".to_string()),
        "bound@mfa.test".to_string(),
    )
    .expect("totp");
    let step = (chrono::Utc::now().timestamp() / 30) + offset_steps;
    totp.generate(u64::try_from(step).expect("positive step") * 30)
}

/// Enrols TOTP over HTTP the way a client does; returns the secret and the
/// one-time recovery batch.
async fn enroll_totp(app: &axum::Router, bearer: &str) -> (String, Vec<String>) {
    let began = send(
        app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(began.status(), StatusCode::OK, "first enrolment is allowed");
    let secret = body_json(began).await["secret_base32"]
        .as_str()
        .expect("secret")
        .to_string();
    let verified = send(
        app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-verify",
        Some(bearer),
        Some(json!({ "code": totp_code(&secret, 0) })),
    )
    .await;
    assert_eq!(verified.status(), StatusCode::OK, "first code confirms");
    let codes = body_json(verified).await["recovery_codes"]
        .as_array()
        .expect("codes")
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();
    (secret, codes)
}

/// Seeds a passkey row. The credential JSON is never parsed on the removal
/// path, so a placeholder body is enough.
async fn seed_passkey(pool: &PgPool, org_id: Uuid, user_id: Uuid) -> (Uuid, String) {
    let row_id = Uuid::now_v7();
    let cred_id = format!("cred-{}", row_id);
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    core_db::mfa::insert_webauthn_credential(
        &mut *tx,
        row_id,
        org_id,
        user_id,
        &cred_id,
        &json!({ "placeholder": true }),
        "laptop",
    )
    .await
    .expect("passkey row");
    tx.commit().await.expect("commit");
    (row_id, cred_id)
}

/// Writes the durable step-up record a passkey removal asks for.
async fn seed_step_up(pool: &PgPool, org_id: Uuid, user_id: Uuid, session_id: Uuid, method: &str) {
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    core_db::mfa::record_step_up(
        &mut *tx,
        core_db::mfa::StepUpRecord {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            session_id,
            method,
            factor_ref: None,
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(300),
        },
    )
    .await
    .expect("step-up record");
    tx.commit().await.expect("commit");
}

/// Counts rows matching a query whose only bind is the user id, inside the
/// org's RLS scope.
async fn db_count(state: &AppState, org_id: Uuid, sql: &str, user_id: Uuid) -> i64 {
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    let count: i64 = sqlx::query_scalar(sql)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    tx.commit().await.expect("commit");
    count
}

async fn unused_recovery_hashes(state: &AppState, org_id: Uuid, user_id: Uuid) -> Vec<String> {
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    let hashes = core_db::mfa::unused_recovery_hashes(&mut *tx, user_id)
        .await
        .expect("hashes")
        .into_iter()
        .map(|(_, hash)| hash)
        .collect();
    tx.commit().await.expect("commit");
    hashes
}

fn id_of(value: &Value, key: &str) -> Uuid {
    value[key]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("{key} is a uuid"))
}

const TOTP_CONFIRMED: &str = "SELECT count(*) FROM totp_secrets WHERE user_id = $1 AND confirmed";
const UNUSED_CODES: &str =
    "SELECT count(*) FROM recovery_codes WHERE user_id = $1 AND used_at IS NULL";
const PASSKEYS: &str = "SELECT count(*) FROM webauthn_credentials WHERE user_id = $1";
const STEP_UPS: &str = "SELECT count(*) FROM mfa_step_ups WHERE user_id = $1";

// --- the bound --------------------------------------------------------------

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

/// The 503 shape the login doors answer with, asserted here for the
/// signed-in doors: standard envelope, `Retry-After`, never a 500.
async fn assert_saturated_503(response: axum::response::Response) {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a saturated hash queue must answer 503, not {status}"
    );
    let body = body_json(response).await;
    assert_eq!(
        body["error"]["reason"], "service_unavailable",
        "the 503 must carry the standard reason code: {body}"
    );
    assert_eq!(body["error"]["code"], 503);
    let expected_retry = hash_gate::RETRY_AFTER_SECS.to_string();
    assert_eq!(
        retry_after.as_deref(),
        Some(expected_retry.as_str()),
        "a 503 without Retry-After parks the client"
    );
}

// --- totp/disable -----------------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn totp_disable_refuses_at_the_bound_and_still_turns_the_factor_off() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-disable-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let (secret, _codes) = enroll_totp(&app, &bearer).await;
    let code = totp_code(&secret, 1);

    let disable = |password: &'static str, code: String| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            send(
                &app,
                Method::POST,
                "/api/auth/mfa/totp/disable",
                Some(&bearer),
                Some(json!({ "password": password, "code": code })),
            )
            .await
        }
    };

    // Failure arm: a wrong password is refused and the factor survives.
    let wrong = disable("wrong-password-11", code.clone()).await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(db_count(&state, org_id, TOTP_CONFIRMED, user_id).await, 1);

    // Saturated: the typed 503, before any hash or write.
    let held = drain_slots().await;
    let saturated = disable(PASSWORD, code.clone()).await;
    assert_saturated_503(saturated).await;
    drop(held);
    assert_eq!(
        db_count(&state, org_id, TOTP_CONFIRMED, user_id).await,
        1,
        "a refused request must not touch the factor"
    );
    assert_eq!(db_count(&state, org_id, UNUSED_CODES, user_id).await, 8);

    // Success arm: the same call, the same code, a freed slot.
    let ok = disable(PASSWORD, code).await;
    assert_eq!(ok.status(), StatusCode::NO_CONTENT);
    assert_eq!(db_count(&state, org_id, TOTP_CONFIRMED, user_id).await, 0);
    assert_eq!(
        db_count(&state, org_id, UNUSED_CODES, user_id).await,
        0,
        "disabling wipes the recovery batch too"
    );
}

// --- recovery-codes/regenerate ----------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn recovery_code_regeneration_refuses_at_the_bound_and_replaces_the_batch() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-regen-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let (secret, old_codes) = enroll_totp(&app, &bearer).await;
    let code = totp_code(&secret, 1);
    let old_hashes = unused_recovery_hashes(&state, org_id, user_id).await;
    assert_eq!(old_hashes.len(), 8, "enrolment issued one batch");

    let regenerate = |password: &'static str, code: String| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            send(
                &app,
                Method::POST,
                "/api/auth/mfa/recovery-codes/regenerate",
                Some(&bearer),
                Some(json!({ "password": password, "code": code })),
            )
            .await
        }
    };

    // Failure arm: a wrong password is refused, and the old batch still holds.
    let wrong = regenerate("wrong-password-11", code.clone()).await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unused_recovery_hashes(&state, org_id, user_id).await,
        old_hashes
    );

    // Saturated: typed 503, batch untouched.
    let held = drain_slots().await;
    assert_saturated_503(regenerate(PASSWORD, code.clone()).await).await;
    drop(held);
    assert_eq!(
        unused_recovery_hashes(&state, org_id, user_id).await,
        old_hashes,
        "a refused regeneration must not invalidate the batch the user holds"
    );

    // Success arm: a fresh batch of eight, and the old plaintext is dead.
    let ok = regenerate(PASSWORD, code).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let issued: Vec<String> = body_json(ok).await["recovery_codes"]
        .as_array()
        .expect("codes")
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();
    assert_eq!(issued.len(), 8);
    let new_hashes = unused_recovery_hashes(&state, org_id, user_id).await;
    assert_eq!(new_hashes.len(), 8, "the batch is replaced, not appended");
    for hash in &new_hashes {
        assert!(
            !core_api::auth::password::verify_async(old_codes[0].clone(), hash.clone()).await,
            "an old code must not verify against the new batch"
        );
    }
}

// --- step-up ----------------------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn step_up_refuses_at_the_bound_and_records_the_proof_when_admitted() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-stepup-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let (secret, _codes) = enroll_totp(&app, &bearer).await;
    let code = totp_code(&secret, 1);

    let step_up = |password: &'static str, code: String| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            send(
                &app,
                Method::POST,
                "/api/auth/mfa/step-up",
                Some(&bearer),
                Some(json!({ "password": password, "code": code })),
            )
            .await
        }
    };

    // Saturated: the typed 503, before any hash and without recording a proof.
    let held = drain_slots().await;
    assert_saturated_503(step_up(PASSWORD, code.clone()).await).await;
    drop(held);
    assert_eq!(
        db_count(&state, org_id, STEP_UPS, user_id).await,
        0,
        "a refusal records nothing"
    );

    // Success arm: one proof recorded for this session.
    let ok = step_up(PASSWORD, code.clone()).await;
    assert_eq!(ok.status(), StatusCode::OK);
    let proof = body_json(ok).await;
    assert_eq!(proof["method"], "totp");
    assert_eq!(proof["ttl_secs"], 300);
    assert_eq!(db_count(&state, org_id, STEP_UPS, user_id).await, 1);

    // Failure arm last: a wrong password is refused and records no proof. The
    // durable MFA_VERIFY throttle arms itself on that failure, so this arm has
    // to follow the ones that expect to reach the hash gate.
    let wrong = step_up("wrong-password-11", code).await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(db_count(&state, org_id, STEP_UPS, user_id).await, 1);
}

// --- passkey removal --------------------------------------------------------

#[tokio::test]
#[serial_test::serial]
async fn passkey_removal_refuses_at_the_bound_and_still_deletes_the_credential() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-remove-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let _ = enroll_totp(&app, &bearer).await;
    let (row_id, _cred_id) = seed_passkey(&state.pool, org_id, user_id).await;
    seed_step_up(
        &state.pool,
        org_id,
        user_id,
        id_of(&owner, "session_id"),
        "totp",
    )
    .await;

    let remove = |password: &'static str| {
        let app = app.clone();
        let bearer = bearer.clone();
        async move {
            send(
                &app,
                Method::POST,
                &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
                Some(&bearer),
                Some(json!({ "password": password })),
            )
            .await
        }
    };

    let wrong = remove("wrong-password-11").await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(db_count(&state, org_id, PASSKEYS, user_id).await, 1);

    let held = drain_slots().await;
    assert_saturated_503(remove(PASSWORD).await).await;
    drop(held);
    assert_eq!(
        db_count(&state, org_id, PASSKEYS, user_id).await,
        1,
        "a refused removal deletes nothing"
    );

    let ok = remove(PASSWORD).await;
    assert_eq!(ok.status(), StatusCode::NO_CONTENT);
    assert_eq!(db_count(&state, org_id, PASSKEYS, user_id).await, 0);
}

// --- the recovery-code loop -------------------------------------------------

/// The claim this lane rests on: verifying a recovery code no longer spends a
/// pooled connection on argon2.
///
/// The measurement is the pool itself, sampled every 2 ms while the request is
/// in flight. Every unused code is a candidate, so a wrong code costs one
/// verify per code — the longest hash-only section in this module. While that
/// runs, nothing may hold a connection; the case requires at least 100 ms of
/// sustained "every established connection idle" samples, which a request that
/// kept its transaction open for the loop cannot produce at any point.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn the_recovery_code_loop_hashes_without_holding_a_connection() {
    let state = test_state().await;
    let pool = state.pool.clone();
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-loop-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let (_secret, _codes) = enroll_totp(&app, &bearer).await;
    assert_eq!(
        unused_recovery_hashes(&state, org_id, user_id).await.len(),
        8
    );

    let in_flight = Arc::new(AtomicBool::new(true));
    let sampler = {
        let pool = pool.clone();
        let in_flight = in_flight.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let mut samples = 0u32;
            let mut idle_samples = 0u32;
            while in_flight.load(Ordering::Relaxed) {
                samples += 1;
                if pool.num_idle() as usize == pool.size() as usize {
                    idle_samples += 1;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            (started.elapsed(), samples, idle_samples)
        })
    };

    // Not a TOTP code, so the TOTP arm declines and every unused recovery code
    // is verified. The answer is a plain 401.
    let started = Instant::now();
    let response = send(
        &app,
        Method::POST,
        "/api/auth/mfa/step-up",
        Some(&bearer),
        Some(json!({ "password": PASSWORD, "code": "AAAAA-BBBBB" })),
    )
    .await;
    let elapsed = started.elapsed();
    in_flight.store(false, Ordering::Relaxed);
    let (sampled_for, samples, idle_samples) = sampler.await.expect("sampler");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let idle_ms = (idle_samples * 2) as u128;
    println!(
        "recovery-code loop: request {} ms, sampled {} ms in {} samples, all-idle {} ms",
        elapsed.as_millis(),
        sampled_for.as_millis(),
        samples,
        idle_ms
    );
    assert!(
        idle_ms >= 100,
        "the hash loop held a pooled connection: all-idle for only {idle_ms} ms of {} sampled \
         ({idle_samples}/{samples} samples)",
        sampled_for.as_millis()
    );

    // A wrong code matches nothing and consumes nothing.
    assert_eq!(
        db_count(&state, org_id, UNUSED_CODES, user_id).await,
        8,
        "the loop must not spend a code it did not match"
    );
}

// --- burst ------------------------------------------------------------------

/// A burst at one bounded door: the callers that fit in the slot are admitted,
/// every other caller gets the typed 503 with `Retry-After`, and the door works
/// again once the burst drains.
///
/// The wrong password keeps the admitted arm out of the delete, so what is
/// measured is admission. The passkey-removal door is the one measured here on
/// purpose: `step-up` sits behind the durable `MFA_VERIFY` throttle, which arms
/// itself on the first failed password and would answer 429 before the hash
/// gate is ever reached.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[serial_test::serial]
async fn a_burst_on_passkey_removal_admits_a_bounded_number_and_refuses_the_rest() {
    let state = test_state().await;
    let app = core_api::router(state.clone());
    let _ = one_slot_gate();
    let slug = format!("bound-burst-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer").to_string();
    let _ = enroll_totp(&app, &bearer).await;
    let (row_id, _cred_id) = seed_passkey(&state.pool, org_id, user_id).await;
    seed_step_up(
        &state.pool,
        org_id,
        user_id,
        id_of(&owner, "session_id"),
        "totp",
    )
    .await;
    assert_eq!(one_slot_gate().slots(), 1, "this case measures one slot");

    let burst = 6usize;
    let mut tasks = Vec::with_capacity(burst);
    for _ in 0..burst {
        let app = app.clone();
        let bearer = bearer.clone();
        tasks.push(tokio::spawn(async move {
            let response = send(
                &app,
                Method::POST,
                &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
                Some(&bearer),
                Some(json!({ "password": "wrong-password-11" })),
            )
            .await;
            let status = response.status();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            (status, retry_after)
        }));
    }

    let expected_retry = hash_gate::RETRY_AFTER_SECS.to_string();
    let mut admitted = 0usize;
    let mut refused = 0usize;
    for task in tasks {
        let (status, retry_after) = task.await.expect("removal task");
        match status {
            StatusCode::UNAUTHORIZED => admitted += 1,
            StatusCode::SERVICE_UNAVAILABLE => {
                assert_eq!(
                    retry_after.as_deref(),
                    Some(expected_retry.as_str()),
                    "every refusal must carry Retry-After"
                );
                refused += 1;
            }
            other => panic!("burst request answered {other}"),
        }
    }
    println!("burst {burst}: admitted {admitted}, refused(503) {refused}");
    assert!(admitted >= 1, "the burst must not refuse every caller");
    assert!(
        admitted <= 2,
        "one slot admits at most the callers that run through it; {admitted} got in"
    );
    assert_eq!(admitted + refused, burst);

    // The bound is backpressure, not an outage: the door answers normally once
    // the burst has drained.
    let ok = send(
        &app,
        Method::POST,
        &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
        Some(&bearer),
        Some(json!({ "password": PASSWORD })),
    )
    .await;
    assert_eq!(
        ok.status(),
        StatusCode::NO_CONTENT,
        "a drained gate admits again"
    );
}
