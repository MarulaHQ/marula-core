//! p57 adversarial sweep — the enrollment arms of the factor set.
//!
//! Steps up what `qa-p54-security-decisions/DECISIONS.md` record 2 fixed for
//! REMOVAL, applied to what no record had named: ADDING a second factor.
//!
//! - a session on an account that already holds a confirmed factor cannot
//!   mint a NEW factor it controls (enroll TOTP on a passkey account, or
//!   register a passkey on any protected account) without proving one first;
//! - an enrolment after an administrator reset still needs no proof — the
//!   enrolment IS the sanctioned way out;
//! - magic-link sign-in refuses tokens to an account with an enrolled
//!   factor: the link proves the mailbox, never the factor.

use axum::http::{Method, StatusCode};
use core_api::{config::Config, state::AppState};
use serde_json::{json, Value};
use sha2::Digest;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "correct-horse-battery-11";

// --- harness (same shape as mfa_step_up_flow) --------------------------------

fn database_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
}

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
        plugin_pool: Default::default(),
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

/// Registers an org and returns its owner's bearer token plus the identity
/// triple (`user_id`, `org_id`, `session_id`) resolved from /api/me.
async fn register_owner(app: &axum::Router, slug: &str) -> (String, Uuid, Uuid, Uuid) {
    let tokens = body_json(
        send(
            app,
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({
                "org_name": "Enroll Gate Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": PASSWORD,
                "display_name": "Owner",
            })),
        )
        .await,
    )
    .await;
    let bearer = tokens["access_token"].as_str().expect("token").to_string();
    let me = body_json(send(app, Method::GET, "/api/me", Some(&bearer), None).await).await;
    let user_id = me["user_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
    let org_id = me["org_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
    let session_id = tokens["session_id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());
    (
        bearer,
        user_id.expect("owner user id"),
        org_id.expect("owner org id"),
        session_id.expect("owner session id"),
    )
}

fn totp_code(secret_base32: &str, offset_steps: i64) -> String {
    use totp_rs::{Algorithm, Builder};
    let bytes = data_encoding::BASE32_NOPAD
        .decode(secret_base32.to_ascii_uppercase().as_bytes())
        .expect("stored secret is base32");
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(bytes)
        .with_issuer(Some("Marula"))
        .with_account_name("gate@enroll.test")
        .build()
        .expect("totp");
    let step = (chrono::Utc::now().timestamp() / 30) + offset_steps;
    totp.generate(u64::try_from(step).expect("positive step") * 30)
        .to_string()
}

/// Enrols TOTP over HTTP the way a client does.
async fn enroll_totp(app: &axum::Router, bearer: &str) -> String {
    let began = send(
        app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(began.status(), StatusCode::OK, "enrolment allowed");
    let began = body_json(began).await;
    let secret = began["secret_base32"].as_str().expect("secret").to_string();
    let verified = send(
        app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-verify",
        Some(bearer),
        Some(json!({ "code": totp_code(&secret, 0) })),
    )
    .await;
    assert_eq!(verified.status(), StatusCode::OK, "first code confirms");
    secret
}

async fn seed_passkey(pool: &PgPool, org_id: Uuid, user_id: Uuid) {
    let row_id = Uuid::now_v7();
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    core_db::mfa::insert_webauthn_credential(
        &mut *tx,
        row_id,
        org_id,
        user_id,
        &format!("cred-{row_id}"),
        &json!({ "placeholder": true }),
        "laptop",
    )
    .await
    .expect("passkey row");
    tx.commit().await.expect("commit");
}

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

async fn mark_reenroll_required(pool: &PgPool, user_id: Uuid) {
    sqlx::query("UPDATE users SET mfa_reenroll_required = true WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("reenroll flag");
}

/// Seeds an org with the magic-link opt-in plus two users: A (no factors)
/// and B (a confirmed TOTP row). Returns both ids.
async fn seed_magic_link_org(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
    let org = Uuid::now_v7();
    let slug = format!("ml-gate-{}", Uuid::now_v7().simple());
    sqlx::query(
        "INSERT INTO orgs (id, name, slug, ticket_prefix, ticket_sequence, base_currency, magic_link_sign_in) \
         VALUES ($1,'Ml Gate Co',$2,'MLG',1,'USD', true)",
    )
    .bind(org)
    .bind(&slug)
    .execute(pool)
    .await
    .expect("org");

    let user_a = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash) \
         VALUES ($1,$2,$3,'A','argon2')",
    )
    .bind(user_a)
    .bind(org)
    .bind(format!("{slug}-a@test"))
    .execute(pool)
    .await
    .expect("user a");

    let user_b = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash) \
         VALUES ($1,$2,$3,'B','argon2')",
    )
    .bind(user_b)
    .bind(org)
    .bind(format!("{slug}-b@test"))
    .execute(pool)
    .await
    .expect("user b");

    sqlx::query(
        "INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed) \
         VALUES ($1,$2,$3,$4,true)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(user_b)
    .bind([0u8; 40]) // opaque sealed bytes; confirm never decrypts them
    .execute(pool)
    .await
    .expect("totp row");

    (org, user_a, user_b)
}

#[tokio::test]
async fn passkey_only_account_needs_a_step_up_to_enroll_totp() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let (bearer, _user_id, org_id, _session_id) =
        register_owner(&app, &format!("enroll-gate-{}", Uuid::now_v7().simple())).await;

    // The Owner holds ONE working factor: a passkey row. No step-up exists
    // on this password-login session.
    seed_passkey(&state.pool, org_id, _user_id).await;

    let began = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        began.status(),
        StatusCode::PRECONDITION_REQUIRED,
        "enrolling TOTP under a confirmed passkey needs a fresh step-up"
    );
    let body = body_json(began).await;
    assert_eq!(body["error"]["reason"], "step_up_required");

    // The same session, once a passkey login stamped its proof, may enroll.
    seed_step_up(&state.pool, org_id, _user_id, _session_id, "webauthn").await;
    let proved = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        proved.status(),
        StatusCode::OK,
        "a proved session enrolls TOTP"
    );
}

#[tokio::test]
async fn reenroll_required_enrollment_still_needs_no_step_up() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let (bearer, user_id, _org_id, _session_id) =
        register_owner(&app, &format!("reenroll-exit-{}", Uuid::now_v7().simple())).await;

    seed_passkey(&state.pool, _org_id, user_id).await;
    mark_reenroll_required(&state.pool, user_id).await;

    let began = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        began.status(),
        StatusCode::OK,
        "an administrator reset leaves enrolment as the no-proof exit"
    );
}

#[tokio::test]
async fn passkey_register_begin_needs_a_step_up_on_a_protected_account() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let (bearer, user_id, org_id, session_id) =
        register_owner(&app, &format!("passkey-gate-{}", Uuid::now_v7().simple())).await;

    enroll_totp(&app, &bearer).await;

    let began = send(
        &app,
        Method::POST,
        "/api/auth/mfa/webauthn/register/begin",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        began.status(),
        StatusCode::PRECONDITION_REQUIRED,
        "registering a passkey under a confirmed factor needs a fresh step-up"
    );
    let body = body_json(began).await;
    assert_eq!(body["error"]["reason"], "step_up_required");

    seed_step_up(&state.pool, org_id, user_id, session_id, "totp").await;
    let proved = send(
        &app,
        Method::POST,
        "/api/auth/mfa/webauthn/register/begin",
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(
        proved.status(),
        StatusCode::OK,
        "a proved session may register"
    );
}

#[tokio::test]
async fn magic_link_confirm_refuses_tokens_when_a_factor_is_enrolled() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let (org, user_a, user_b) = seed_magic_link_org(&state.pool).await;

    async fn confirm_link(
        app: &axum::Router,
        pool: &PgPool,
        org: Uuid,
        user_id: Uuid,
    ) -> (StatusCode, Value) {
        let raw = format!(
            "{:064x}",
            (Uuid::now_v7().as_u128() << 64) | Uuid::now_v7().as_u128()
        );
        let hash = hex::encode(sha2::Sha256::digest(raw.as_bytes()));
        sqlx::query(
            "INSERT INTO auth_one_use_tokens (id, org_id, user_id, kind, token_hash, expires_at) \
             VALUES ($1,$2,$3,'magic_link',$4, clock_timestamp() + interval '10 minutes')",
        )
        .bind(Uuid::now_v7())
        .bind(org)
        .bind(user_id)
        .bind(&hash)
        .execute(pool)
        .await
        .expect("token");
        let first = send(
            app,
            Method::POST,
            "/api/auth/magic-link/confirm",
            None,
            Some(json!({ "token": raw })),
        )
        .await;
        (first.status(), body_json(first).await)
    }

    let (status_a, body_a) = confirm_link(&app, &state.pool, org, user_a).await;
    assert_eq!(
        status_a,
        StatusCode::OK,
        "a factorless account keeps link login"
    );
    assert!(
        body_a["access_token"].is_string(),
        "tokens are issued as before"
    );

    let (status_b, body_b) = confirm_link(&app, &state.pool, org, user_b).await;
    assert_eq!(
        status_b,
        StatusCode::UNAUTHORIZED,
        "an enrolled factor blocks token issue on the link"
    );
    assert_eq!(body_b["error"]["reason"], "mfa_required");
}
