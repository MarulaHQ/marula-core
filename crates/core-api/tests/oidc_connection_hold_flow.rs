//! Proves the OIDC callback holds NO pooled connection across an
//! identity-provider round trip, and that splitting its transaction did not
//! cost the guarantees that transaction was carrying.
//!
//! The measured property is "no backend is `idle in transaction` on
//! `oidc_providers` while a provider is being waited on", sampled directly
//! from `pg_stat_activity` rather than inferred from the code's shape. The
//! stub provider answers slowly enough that the old shape is unmistakable: with
//! ~2s of provider latency and a 20ms sampler, a held connection shows up as
//! roughly a hundred consecutive positive samples, and the assertion reports
//! that count instead of a bare boolean.
//!
//! The other two tests pin what the split could have broken: two concurrent
//! callbacks for one identity still leave one account and no orphan user, and
//! a provider row edited while its provider was thinking refuses the login
//! instead of deciding against the configuration it started with.
//!
//! Non-purpose: the flow's own semantics (state binding, nonce echo, PKCE,
//! link conservatism) live in `oidc_flow.rs` and are re-run unmodified by the
//! lane; this file does not restate them.

use axum::{
    extract::{Query as AxumQuery, State as AxumState},
    routing::{get, post},
    Json, Router,
};
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// The sampler measures a WHOLE-DATABASE property, so the tests in this file
// take turns: a callback from a neighbouring test would be indistinguishable
// from the one under measurement. `#[serial]` is the workspace's tool for
// exactly this (`auth_hash_bound_flow`, `portal_hash_bound_flow`).
const SAMPLE_INTERVAL: Duration = Duration::from_millis(20);
const DISCOVERY_DELAY: Duration = Duration::from_millis(400);
const TOKEN_DELAY: Duration = Duration::from_millis(1200);
const JWKS_DELAY: Duration = Duration::from_millis(400);

// ---------------------------------------------------------------------------
// harness (mirrors oidc_flow's; integration test binaries cannot share code)
// ---------------------------------------------------------------------------

fn database_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("oidc hold flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
}

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let admin_url = database_url();
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: AppState::reporting_pool_lazy(&reporting_url).expect("lazy reporting pool"),
    }
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
}

async fn send(
    app: &axum::Router,
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

fn location_of(response: &axum::response::Response) -> String {
    response
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("expected redirect, got {}", response.status()))
}

fn query_map(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .expect("url parses")
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Registers a fresh org (unique slug); returns the owner's tokens.
async fn register_org(app: &axum::Router, slug: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Hold Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

const CLIENT_ID: &str = "marula-core-client";
const CLIENT_SECRET: &str = "idp-s3cret";

async fn configure_provider(
    app: &axum::Router,
    bearer: &str,
    name: &str,
    issuer: &str,
    jit: bool,
) -> Value {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/auth/oidc/providers/{name}"),
        Some(bearer),
        Some(json!({
            "display_name": "Slow Mock IdP",
            "issuer": issuer,
            "discovery_url": format!("{issuer}/.well-known/openid-configuration"),
            "client_id": CLIENT_ID,
            "client_secret": CLIENT_SECRET,
            "jit_provisioning": jit,
            "enabled": true,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "provider upsert succeeds");
    body_json(response).await
}

// ---------------------------------------------------------------------------
// mock IdP — real HTTP, and it can be slow on purpose
// ---------------------------------------------------------------------------

/// A row edit the stub performs from INSIDE the token round trip: the exact
/// window in which the callback used to be holding its transaction.
struct RotateRequest {
    pool: PgPool,
    provider_id: Uuid,
    sealed: Vec<u8>,
}

struct IdpState {
    issuer: String,
    client_id: String,
    client_secret: String,
    signing_pem: String,
    kid: &'static str,
    n_b64: String,
    e_b64: String,
    email: String,
    codes: Mutex<HashMap<String, String>>,
    discovery_delay: Duration,
    token_delay: Duration,
    jwks_delay: Duration,
    rotate: Mutex<Option<RotateRequest>>,
}

#[derive(serde::Deserialize)]
struct AuthorizeParams {
    client_id: String,
    redirect_uri: String,
    state: String,
    nonce: String,
}

#[derive(serde::Deserialize)]
struct TokenForm {
    grant_type: String,
    code: String,
}

fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

async fn spawn_idp(
    client_id: &str,
    client_secret: &str,
    email: &str,
    delays: (Duration, Duration, Duration),
) -> (String, Arc<IdpState>) {
    // `rsa` 0.9 key generation takes a `rand_core` 0.6 generator, so the
    // OS source comes from rsa's own re-export rather than `rand` 0.10.
    let mut rng = rsa::rand_core::OsRng;
    let private = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen (test-only)");
    let public = rsa::RsaPublicKey::from(&private);
    let pem = private
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .expect("pem")
        .to_string();
    let n_b64 = b64(&public.n().to_bytes_be());
    let e_b64 = b64(&public.e().to_bytes_be());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let issuer = format!("http://{addr}");

    let state = Arc::new(IdpState {
        issuer: issuer.clone(),
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        signing_pem: pem,
        kid: "test-key",
        n_b64,
        e_b64,
        email: email.to_string(),
        codes: Mutex::new(HashMap::new()),
        discovery_delay: delays.0,
        token_delay: delays.1,
        jwks_delay: delays.2,
        rotate: Mutex::new(None),
    });

    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(|AxumState(s): AxumState<Arc<IdpState>>| async move {
                tokio::time::sleep(s.discovery_delay).await;
                Json(json!({
                    "issuer": s.issuer,
                    "authorization_endpoint": format!("{}/authorize", s.issuer),
                    "token_endpoint": format!("{}/token", s.issuer),
                    "jwks_uri": format!("{}/jwks.json", s.issuer),
                }))
            }),
        )
        .route(
            "/jwks.json",
            get(|AxumState(s): AxumState<Arc<IdpState>>| async move {
                tokio::time::sleep(s.jwks_delay).await;
                Json(json!({
                    "keys": [{
                        "kty": "RSA",
                        "use": "sig",
                        "alg": "RS256",
                        "kid": s.kid,
                        "n": s.n_b64,
                        "e": s.e_b64,
                    }]
                }))
            }),
        )
        .route(
            "/authorize",
            get(
                |AxumState(s): AxumState<Arc<IdpState>>,
                 AxumQuery(q): AxumQuery<AuthorizeParams>| async move {
                    assert_eq!(q.client_id, s.client_id, "mock IdP checks client");
                    let code = format!("code-{}", Uuid::now_v7());
                    s.codes
                        .lock()
                        .unwrap()
                        .insert(code.clone(), q.nonce.clone());
                    let target = format!(
                        "{}?code={}&state={}",
                        q.redirect_uri,
                        urlencode(&code),
                        urlencode(&q.state)
                    );
                    axum::response::Redirect::temporary(&target)
                },
            ),
        )
        .route(
            "/token",
            post(
                |AxumState(s): AxumState<Arc<IdpState>>,
                 headers: axum::http::HeaderMap,
                 axum::Form(form): axum::Form<TokenForm>| async move {
                    assert_eq!(form.grant_type, "authorization_code");
                    let expected_basic = data_encoding::BASE64
                        .encode(format!("{}:{}", s.client_id, s.client_secret).as_bytes());
                    let got = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert_eq!(got, format!("Basic {expected_basic}"), "basic auth");

                    // "An admin saved a new secret while the provider was
                    // thinking." The callback is mid-round-trip right now.
                    let rotation = s.rotate.lock().unwrap().take();
                    let nonce = s
                        .codes
                        .lock()
                        .unwrap()
                        .get(&form.code)
                        .cloned()
                        .expect("unknown or replayed code");
                    tokio::time::sleep(s.token_delay).await;
                    if let Some(rotation) = rotation {
                        sqlx::query(
                            "UPDATE oidc_providers SET client_secret_enc = $2, updated_at = now()
                             WHERE id = $1",
                        )
                        .bind(rotation.provider_id)
                        .bind(&rotation.sealed)
                        .execute(&rotation.pool)
                        .await
                        .expect("rotate the provider secret mid-flight");
                    }

                    let now = chrono::Utc::now().timestamp();
                    let claims = json!({
                        "iss": s.issuer,
                        "sub": "subject-hold",
                        "aud": s.client_id,
                        "exp": now + 300,
                        "iat": now,
                        "email": s.email,
                        "email_verified": true,
                        "name": "Slow User",
                        "nonce": nonce,
                    });
                    let key =
                        jsonwebtoken::EncodingKey::from_rsa_pem(s.signing_pem.as_bytes()).unwrap();
                    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
                    header.kid = Some(s.kid.to_string());
                    let id_token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
                    Json(json!({ "id_token": id_token, "token_type": "Bearer" }))
                },
            ),
        );

    let app = app.with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (issuer, state)
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .append_pair("k", s)
        .finish()
        .trim_start_matches("k=")
        .to_string()
}

/// login → authorize → callback, the three hops a browser would make.
async fn drive_sso_login(
    app: &axum::Router,
    idp_base: &str,
    provider_name: &str,
) -> (axum::http::StatusCode, String) {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/auth/oidc/{provider_name}/login"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 303, "login redirects to the provider");
    let authorize_url = location_of(&response);
    assert!(authorize_url.starts_with(idp_base));

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize_response = http
        .get(&authorize_url)
        .send()
        .await
        .expect("reaches mock IdP");
    assert_eq!(authorize_response.status(), 307);
    let callback_url = authorize_response
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string())
        .expect("mock IdP authorize redirects back");

    let path_and_query = callback_url
        .split_once("//")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(_, pq)| format!("/{pq}"))
        .expect("callback url shape");
    let callback_response = send(app, axum::http::Method::GET, &path_and_query, None, None).await;
    let status = callback_response.status();
    if status.is_redirection() {
        (status, location_of(&callback_response))
    } else {
        let body = body_json(callback_response).await;
        (status, body.to_string())
    }
}

// ---------------------------------------------------------------------------
// the sampler
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct HoldWatch {
    /// Longest unbroken run of samples that found a backend parked in a
    /// transaction whose last statement touched `oidc_providers`.
    max_consecutive: usize,
    /// Wall-clock span of that run, measured from the first positive sample to
    /// the last. Reported instead of `max_consecutive × interval` because the
    /// sampler's own query is not free on a loaded machine: the count says how
    /// many ticks landed, this says how long the pose actually lasted.
    max_held: Duration,
    samples: usize,
}

/// Polls `pg_stat_activity` until told to stop.
///
/// `state = 'idle in transaction'` is the backend reporting "I have an open
/// transaction and nothing to do" — exactly the pose a callback holds while it
/// waits on the network. The query text is the LAST statement the backend ran,
/// which is the provider read in the shape this lane replaced, so it both
/// identifies the endpoint's transaction and excludes unrelated open
/// transactions on other tables.
async fn watch_connection_hold(
    pool: &PgPool,
    stop: Arc<AtomicBool>,
    interval: Duration,
) -> HoldWatch {
    let mut conn = pool.acquire().await.expect("sampler connection");
    let mut watch = HoldWatch {
        max_consecutive: 0,
        max_held: Duration::ZERO,
        samples: 0,
    };
    let mut run = 0usize;
    let mut run_started: Option<std::time::Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        // The transaction-age floor is what separates a hold from a gap: a
        // decision transaction reports 'idle in transaction' for the
        // sub-millisecond between two statements of its own, which is not a
        // connection held across anything. The fastest stub delay is 400ms, so
        // a real round-trip hold cannot slip under 200ms.
        let holding: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database()
               AND pid <> pg_backend_pid()
               AND state = 'idle in transaction'
               AND query ILIKE '%oidc_providers%'
               AND now() - xact_start > interval '200 milliseconds'",
        )
        .fetch_one(&mut *conn)
        .await
        .expect("sample pg_stat_activity");
        watch.samples += 1;
        if holding > 0 {
            run += 1;
            let started = *run_started.get_or_insert_with(std::time::Instant::now);
            watch.max_held = watch.max_held.max(started.elapsed());
            watch.max_consecutive = watch.max_consecutive.max(run);
        } else {
            run = 0;
            run_started = None;
        }
        tokio::time::sleep(interval).await;
    }
    watch
}

async fn count_users_by_email(pool: &PgPool, email: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("count users")
}

async fn count_links(pool: &PgPool, provider_id: Uuid, subject: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM oidc_accounts WHERE provider_id = $1 AND subject = $2")
        .bind(provider_id)
        .bind(subject)
        .fetch_one(pool)
        .await
        .expect("count links")
}

/// Users matching an email that no `oidc_accounts` row points at: the orphan a
/// rolled-back provision would leave if the transaction were broken up.
async fn count_orphan_users(pool: &PgPool, email: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM users u
          WHERE u.email = $1
            AND NOT EXISTS (SELECT 1 FROM oidc_accounts a WHERE a.user_id = u.id)",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("count orphans")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// THE property: an anonymous caller cannot park a pooled connection by making
/// an identity provider wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn anonymous_callback_holds_no_pooled_connection_across_provider_round_trips() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("hold-{suffix}");
    let provider_name = format!("slow-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    // JIT ON: the callback runs its longest arm — link read, argon2, user,
    // role, link and audit write — so the measurement covers the whole path,
    // not just the cheap refusal branches.
    let (issuer, _idp) = spawn_idp(
        CLIENT_ID,
        CLIENT_SECRET,
        "slow.user@example.test",
        (DISCOVERY_DELAY, TOKEN_DELAY, JWKS_DELAY),
    )
    .await;
    configure_provider(&app, owner, &provider_name, &issuer, true).await;

    // A sampler pool of its own: one connection, never competing with the
    // app's pool for the thing it is measuring.
    let sampler = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url())
        .await
        .expect("sampler pool");
    let stop = Arc::new(AtomicBool::new(false));
    let handle = {
        let stop = stop.clone();
        let sampler = sampler.clone();
        tokio::spawn(async move { watch_connection_hold(&sampler, stop, SAMPLE_INTERVAL).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (status, location) = drive_sso_login(&app, &issuer, &provider_name).await;

    stop.store(true, Ordering::Relaxed);
    let watch = handle.await.expect("sampler task");

    assert_eq!(status, 303, "the login still completes: {location}");
    assert!(
        query_map(&location).contains_key("code"),
        "the single-use handoff code is still issued: {location}"
    );

    // The instrument proves it was live: the callback spent ~2s inside the
    // provider stubs, which is ~100 sampling ticks at 20ms.
    assert!(
        watch.samples >= 20,
        "sampler took only {} samples; it cannot vouch for the window",
        watch.samples
    );
    assert_eq!(
        watch.max_consecutive,
        0,
        "the callback held a pooled connection in an open transaction across {} \
         consecutive samples, spanning {}ms, while an identity provider was \
         thinking; the provider stubs total {}ms of latency",
        watch.max_consecutive,
        watch.max_held.as_millis(),
        (DISCOVERY_DELAY + TOKEN_DELAY + JWKS_DELAY).as_millis()
    );
}

/// Two callbacks for ONE identity, arriving together, must leave one account
/// and no orphan user. This is the guarantee the single transaction was
/// carrying, and the reason `provision_jit_user` keeps its transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn concurrent_callbacks_for_one_identity_leave_one_account_and_no_orphan() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("race-{suffix}");
    let provider_name = format!("race-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    let race_email = format!("race.{suffix}@example.test");
    let (issuer, _idp) = spawn_idp(
        CLIENT_ID,
        CLIENT_SECRET,
        &race_email,
        (Duration::ZERO, Duration::from_millis(50), Duration::ZERO),
    )
    .await;
    let provider = configure_provider(&app, owner, &provider_name, &issuer, true).await;
    let provider_id: Uuid = provider["id"].as_str().unwrap().parse().unwrap();

    // Both logins run their authorize + callback hops interleaved; the argon2
    // hash inside the decision transaction is what widens the overlap window.
    //
    // Two outcomes are legitimate and both are one account: the loser hits the
    // email uniqueness check inside `create_user` (typed 409, transaction
    // rolls back) or its link read happens after the winner committed and it
    // takes the plain LOGIN arm. What must never happen is a second account, a
    // second link, or a user left behind by the loser's rollback.
    let (first, second) = tokio::join!(
        drive_sso_login(&app, &issuer, &provider_name),
        drive_sso_login(&app, &issuer, &provider_name)
    );
    let outcomes = [&first, &second];
    assert!(
        outcomes.iter().any(|(status, _)| *status == 303),
        "at least one callback completes: {first:?} {second:?}"
    );
    for (status, body) in outcomes {
        assert!(
            *status == 303 || status.is_client_error(),
            "a losing callback is a redirect or a typed refusal, never a 5xx: {status} {body}"
        );
    }

    let users_with_email = count_users_by_email(&state.pool, &race_email).await;
    let links = count_links(&state.pool, provider_id, "subject-hold").await;
    let orphans = count_orphan_users(&state.pool, &race_email).await;

    assert_eq!(users_with_email, 1, "one identity = one account, never two");
    assert_eq!(links, 1, "one provider subject binds exactly one account");
    assert_eq!(orphans, 0, "no provisioned user was left behind unlinked");
}

/// A provider row edited while its own provider was thinking must refuse the
/// login. The old shape decided against the configuration it started with and
/// never noticed; the split makes that staleness reachable, so it is checked
/// rather than assumed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn provider_edited_during_the_round_trip_redeems_nothing() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("stale-{suffix}");
    let provider_name = format!("stale-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    let stale_email = format!("stale.{suffix}@example.test");
    let (issuer, idp) = spawn_idp(
        CLIENT_ID,
        CLIENT_SECRET,
        &stale_email,
        (Duration::ZERO, TOKEN_DELAY, Duration::ZERO),
    )
    .await;
    let provider = configure_provider(&app, owner, &provider_name, &issuer, true).await;
    let provider_id: Uuid = provider["id"].as_str().unwrap().parse().unwrap();

    let sealed = core_api::crypto::seal(
        &core_api::config::SecretKey(state.config.secret_key.0),
        b"rotated-while-you-waited",
    )
    .expect("seal the new secret");
    *idp.rotate.lock().unwrap() = Some(RotateRequest {
        pool: state.pool.clone(),
        provider_id,
        sealed,
    });

    let (status, location) = drive_sso_login(&app, &issuer, &provider_name).await;
    assert_eq!(status, 303, "the callback still answers with a redirect");
    assert_eq!(
        query_map(&location).get("error").map(String::as_str),
        Some("provider_changed"),
        "a row edited mid-flight refuses the login: {location}"
    );

    let users = count_users_by_email(&state.pool, &stale_email).await;
    let links = count_links(&state.pool, provider_id, "subject-hold").await;
    assert_eq!(users, 0, "a refused login provisions nobody");
    assert_eq!(links, 0, "a refused login binds nobody");
}
