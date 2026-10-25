//! Wave 55 lane `p55-mfa-step-up` HTTP-layer tests.
//!
//! Records 1 and 2 of `qa-p54-security-decisions/DECISIONS.md`, as behaviour:
//!
//! - a confirmed TOTP factor cannot be replaced without a fresh step-up, and
//!   can be with one proved by a DIFFERENT method;
//! - a passkey cannot be removed on a password alone, and its audit row names
//!   the factor that authorized the removal;
//! - an administrator reset clears the factors, lands an audit row, revokes
//!   the target's sessions, and leaves the account owing a factor.
//!
//! Database-backed. The suite is written but not run in this lane (the gate run
//! owns it); `cargo check --tests` compiles it.

use axum::http::{Method, StatusCode};
use core_api::auth::mfa_handlers;
use core_api::auth::AuthedUser;
use core_api::{config::Config, state::AppState};
use core_domain::policy::Permission;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

const PASSWORD: &str = "correct-horse-battery-11";

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

fn database_url() -> String {
    std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("needs MARULA_TEST_DATABASE_URL or DATABASE_URL")
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

/// Registers an org (which seeds the Owner role) and returns the owner's
/// bearer token, session id, user id and org id.
async fn register_org(app: &axum::Router, slug: &str) -> Value {
    let tokens = body_json(
        send(
            app,
            Method::POST,
            "/api/auth/register",
            None,
            Some(json!({
                "org_name": "Step Up Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": PASSWORD,
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
    let me = body_json(send(app, Method::GET, "/api/me", Some(&bearer), None).await).await;
    json!({
        "access_token": bearer,
        "refresh_token": tokens["refresh_token"],
        "session_id": tokens["session_id"],
        "user_id": me["user_id"],
        "org_id": me["org_id"],
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
        "gate@step.test".to_string(),
    )
    .expect("totp");
    let step = (chrono::Utc::now().timestamp() / 30) + offset_steps;
    totp.generate(u64::try_from(step).expect("positive step") * 30)
}

/// Enrols TOTP over HTTP the way a client does, and returns the secret and the
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
    let verified = body_json(verified).await;
    let codes = verified["recovery_codes"]
        .as_array()
        .expect("codes")
        .iter()
        .filter_map(|c| c.as_str().map(String::from))
        .collect();
    (secret, codes)
}

/// Writes the durable record a step-up leaves, for a session the test already
/// holds. The endpoint that normally writes it is exercised separately.
async fn seed_step_up(
    pool: &PgPool,
    org_id: Uuid,
    user_id: Uuid,
    session_id: Uuid,
    method: &str,
    factor_ref: Option<&str>,
) {
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    core_db::mfa::record_step_up(
        &mut *tx,
        Uuid::now_v7(),
        org_id,
        user_id,
        session_id,
        method,
        factor_ref,
        chrono::Utc::now() + chrono::Duration::seconds(300),
    )
    .await
    .expect("step-up record");
    tx.commit().await.expect("commit");
}

/// Seeds a passkey row. The credential JSON is never parsed on the removal
/// path, so a placeholder body is enough to exercise the gate.
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

async fn seed_user(pool: &PgPool, org_id: Uuid, slug: &str) -> Uuid {
    let id = Uuid::now_v7();
    let email = format!("target-{}@{slug}.test", Uuid::now_v7().simple());
    let hash = core_api::auth::password::hash_async(PASSWORD.to_string())
        .await
        .expect("hash");
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    core_db::users::create_user(&mut *tx, id, org_id, &email, "Target", &hash, "UTC")
        .await
        .expect("target user");
    tx.commit().await.expect("commit");
    id
}

async fn audit_detail(pool: &PgPool, org_id: Uuid, action: &str, entity_id: &str) -> Option<Value> {
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    let detail: Option<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_events WHERE action = $1 AND entity_id = $2 \
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(action)
    .bind(entity_id)
    .fetch_optional(&mut *tx)
    .await
    .expect("audit read");
    tx.commit().await.expect("commit");
    detail
}

fn id_of(value: &Value, key: &str) -> Uuid {
    value[key]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(|| panic!("{key} is a uuid"))
}

// --- records 1 and 2: step-up before a destructive factor change ------------

#[tokio::test]
async fn a_confirmed_totp_cannot_be_replaced_without_a_fresh_step_up() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-a-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let bearer = owner["access_token"].as_str().expect("bearer");
    let (secret, _codes) = enroll_totp(&app, bearer).await;

    // The session was created before the factor existed and has never proved
    // one, so starting a replacement is the same "signed in" fact the bug
    // accepted as authorization.
    let response = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    let envelope = body_json(response).await;
    assert_eq!(envelope["error"]["reason"], "step_up_required");
    assert_eq!(envelope["error"]["endpoint"], "/api/auth/mfa/step-up");
    assert_eq!(envelope["error"]["ttl_secs"], 300);

    // The confirmed factor is still the one that works.
    let verified = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-verify",
        Some(bearer),
        Some(json!({ "code": totp_code(&secret, 1) })),
    )
    .await;
    assert_eq!(
        verified.status(),
        StatusCode::OK,
        "the live secret still holds"
    );
}

#[tokio::test]
async fn a_confirmed_totp_can_be_replaced_after_a_non_totp_step_up() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-b-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let bearer = owner["access_token"].as_str().expect("bearer");
    let (old_secret, codes) = enroll_totp(&app, bearer).await;
    assert_eq!(codes.len(), 8, "the first batch is issued once");

    // A recovery code is the factor a user still holds after losing the phone.
    seed_step_up(
        &state.pool,
        id_of(&owner, "org_id"),
        id_of(&owner, "user_id"),
        id_of(&owner, "session_id"),
        "recovery_code",
        Some("code-row"),
    )
    .await;

    let began = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(
        began.status(),
        StatusCode::OK,
        "a fresh step-up authorizes it"
    );
    let began = body_json(began).await;
    let new_secret = began["secret_base32"].as_str().expect("secret").to_string();
    assert_ne!(new_secret, old_secret, "the secret is genuinely replaced");

    let verified = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-verify",
        Some(bearer),
        Some(json!({ "code": totp_code(&new_secret, 0) })),
    )
    .await;
    assert_eq!(verified.status(), StatusCode::OK);
    let verified = body_json(verified).await;
    assert_eq!(
        verified["recovery_codes"].as_array().map(Vec::len),
        Some(8),
        "the replacement issues a fresh batch, invalidating the old one"
    );

    // The audit row names both halves: the factor replaced and the factor that
    // authorized the replacement.
    let started = audit_detail(
        &state.pool,
        id_of(&owner, "org_id"),
        "auth.mfa_totp_enroll_started",
        &id_of(&owner, "user_id").to_string(),
    )
    .await
    .expect("audit row");
    assert_eq!(started["replaces_confirmed_factor"], true);
    assert_eq!(started["authorized_by"], "recovery_code");
}

#[tokio::test]
async fn a_totp_step_up_cannot_authorize_replacing_the_totp_it_belongs_to() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-c-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let bearer = owner["access_token"].as_str().expect("bearer");
    let (_secret, _codes) = enroll_totp(&app, bearer).await;

    // A live TOTP code is exactly what a lost-phone user cannot produce — and
    // using it here would let a stolen session plus the visible secret rotate
    // the factor it already holds.
    seed_step_up(
        &state.pool,
        id_of(&owner, "org_id"),
        id_of(&owner, "user_id"),
        id_of(&owner, "session_id"),
        "totp",
        None,
    )
    .await;

    let response = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    let envelope = body_json(response).await;
    assert_eq!(envelope["error"]["reason"], "step_up_factor_not_allowed");
}

#[tokio::test]
async fn a_passkey_cannot_be_removed_on_a_password_alone() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-d-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer");
    let _ = enroll_totp(&app, bearer).await;
    let (row_id, _cred_id) = seed_passkey(&state.pool, org_id, user_id).await;

    let response = send(
        &app,
        Method::POST,
        &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
        Some(bearer),
        Some(json!({ "password": PASSWORD })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    let envelope = body_json(response).await;
    assert_eq!(envelope["error"]["reason"], "step_up_required");

    // Nothing was removed: the credential is still listed.
    let listed = body_json(
        send(
            &app,
            Method::GET,
            "/api/auth/mfa/webauthn/credentials",
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_passkey_is_removed_after_a_fresh_step_up_and_the_audit_names_it() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-e-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer");
    let _ = enroll_totp(&app, bearer).await;
    let (row_id, _cred_id) = seed_passkey(&state.pool, org_id, user_id).await;

    seed_step_up(
        &state.pool,
        org_id,
        user_id,
        id_of(&owner, "session_id"),
        "totp",
        None,
    )
    .await;

    let response = send(
        &app,
        Method::POST,
        &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
        Some(bearer),
        Some(json!({ "password": PASSWORD })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let detail = audit_detail(
        &state.pool,
        org_id,
        "auth.mfa_webauthn_credential_removed",
        &user_id.to_string(),
    )
    .await
    .expect("audit row lands with the removal");
    assert_eq!(detail["authorized_by"], "totp");
    assert_eq!(detail["label"], "laptop");
    // The confirmed TOTP and the recovery batch are still there.
    assert_eq!(detail["remaining_factors"], 9);
}

#[tokio::test]
async fn a_passkey_cannot_authorize_its_own_removal() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-f-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let user_id = id_of(&owner, "user_id");
    let bearer = owner["access_token"].as_str().expect("bearer");
    let _ = enroll_totp(&app, bearer).await;
    let (row_id, cred_id) = seed_passkey(&state.pool, org_id, user_id).await;

    seed_step_up(
        &state.pool,
        org_id,
        user_id,
        id_of(&owner, "session_id"),
        "webauthn",
        Some(cred_id.as_str()),
    )
    .await;

    let response = send(
        &app,
        Method::POST,
        &format!("/api/auth/mfa/webauthn/credentials/{row_id}/remove"),
        Some(bearer),
        Some(json!({ "password": PASSWORD })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
    let envelope = body_json(response).await;
    assert_eq!(envelope["error"]["reason"], "step_up_factor_not_allowed");
}

// --- the admin reset --------------------------------------------------------

async fn admin_authed(state: &AppState, owner: &Value) -> AuthedUser {
    let org_id = id_of(owner, "org_id");
    let user_id = id_of(owner, "user_id");
    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    let user = core_db::users::get_user(&mut *tx, org_id, user_id)
        .await
        .expect("owner row");
    let roles = core_db::users::effective_roles_for_user(&mut *tx, user_id, org_id)
        .await
        .expect("owner roles");
    tx.commit().await.expect("commit");
    AuthedUser {
        principal: core_api::auth::middleware::principal_for(user, roles),
        session_id: Some(id_of(owner, "session_id")),
        via_api_key: false,
    }
}

#[tokio::test]
async fn an_admin_reset_clears_the_factors_lands_an_audit_row_and_forces_reenrolment() {
    let admin_url = database_url();
    let state = test_state(&admin_url).await;
    let app = core_api::router(state.clone());
    let slug = format!("step-g-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id = id_of(&owner, "org_id");
    let target_id = seed_user(&state.pool, org_id, &slug).await;
    let (_row_id, _cred_id) = seed_passkey(&state.pool, org_id, target_id).await;

    // The target holds a live TOTP enrolment before the reset.
    {
        let mut tx = state.pool.begin().await.expect("begin");
        core_db::set_org(&mut tx, org_id).await.expect("scope");
        core_db::mfa::upsert_pending_totp(&mut *tx, Uuid::now_v7(), org_id, target_id, &[7])
            .await
            .expect("totp");
        core_db::mfa::confirm_totp(&mut *tx, target_id, 5)
            .await
            .expect("confirm");
        core_db::mfa::replace_recovery_codes(
            &mut *tx,
            org_id,
            target_id,
            &["h1".to_string()],
            &[Uuid::now_v7()],
        )
        .await
        .expect("codes");
        // A live session the reset has to take away.
        core_db::users::create_session(
            &state.pool,
            Uuid::now_v7(),
            target_id,
            "stale-refresh",
            None,
            chrono::Utc::now() + chrono::Duration::days(30),
        )
        .await
        .expect("target session");
        tx.commit().await.expect("commit");
    }

    // An administrator may not use the path on themselves.
    let mut self_extensions = axum::http::Extensions::new();
    self_extensions.insert(admin_authed(&state, &owner).await);
    let self_reset = mfa_handlers::admin_reset_user_factors(
        axum::extract::State(state.clone()),
        self_extensions,
        axum::extract::Path(id_of(&owner, "user_id")),
    )
    .await;
    assert!(
        matches!(self_reset, Err(core_api::error::ApiError::Forbidden(_))),
        "a stolen admin session must not be able to turn its owner's MFA off"
    );

    let mut extensions = axum::http::Extensions::new();
    extensions.insert(admin_authed(&state, &owner).await);
    let reset = mfa_handlers::admin_reset_user_factors(
        axum::extract::State(state.clone()),
        extensions,
        axum::extract::Path(target_id),
    )
    .await
    .expect("reset lands");
    assert_eq!(reset.0.totp_removed, 1);
    assert_eq!(reset.0.passkeys_removed, 1);
    assert_eq!(reset.0.recovery_codes_removed, 1);
    assert_eq!(reset.0.sessions_revoked, 1);
    assert!(reset.0.reenroll_required);

    // Nothing carries a working factor, and the credential row is gone.
    {
        let mut tx = state.pool.begin().await.expect("begin");
        core_db::set_org(&mut tx, org_id).await.expect("scope");
        let caps = core_db::mfa::mfa_capabilities(&mut *tx, target_id)
            .await
            .expect("caps");
        assert!(!caps.any(), "the reset leaves no factor behind");
        assert!(
            core_db::mfa::user_mfa_reenroll_required(&mut *tx, target_id)
                .await
                .expect("flag"),
            "the account owes a factor"
        );
        let listed = core_db::mfa::list_webauthn_credentials(&mut *tx, target_id)
            .await
            .expect("passkeys");
        assert!(listed.is_empty());
        tx.commit().await.expect("commit");
    }

    // The audit row names the administrator, the target and what was removed.
    let detail = audit_detail(
        &state.pool,
        org_id,
        "auth.mfa_factors_reset",
        &target_id.to_string(),
    )
    .await
    .expect("audit row");
    assert_eq!(detail["reenroll_required"], true);
    assert_eq!(detail["totp_removed"], 1);
    assert_eq!(detail["passkeys_removed"], 1);

    // --- "must re-enrol on next sign-in", as far as this lane can enforce it
    //
    // Signing in now yields a password-only session (the login handler is a
    // read-only constant in this lane). On that session every factor change
    // except enrolment is refused with the typed answer, and enrolment is
    // permitted without a step-up because there is no factor left to prove.
    let email = {
        let mut tx = state.pool.begin().await.expect("begin");
        core_db::set_org(&mut tx, org_id).await.expect("scope");
        let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
            .bind(target_id)
            .fetch_one(&mut *tx)
            .await
            .expect("email");
        tx.commit().await.expect("commit");
        email
    };
    let target_tokens = body_json(
        send(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": PASSWORD })),
        )
        .await,
    )
    .await;
    let target_bearer = target_tokens["access_token"]
        .as_str()
        .expect("password-only login after a reset issues tokens")
        .to_string();

    let refused = send(
        &app,
        Method::POST,
        "/api/auth/mfa/recovery-codes/regenerate",
        Some(&target_bearer),
        Some(json!({ "password": PASSWORD, "code": "000000" })),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::PRECONDITION_REQUIRED);
    let envelope = body_json(refused).await;
    assert_eq!(envelope["error"]["reason"], "mfa_reenrollment_required");

    let began = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-begin",
        Some(&target_bearer),
        None,
    )
    .await;
    assert_eq!(
        began.status(),
        StatusCode::OK,
        "enrolment is the way out of the reset state"
    );
    let began = body_json(began).await;
    let secret = began["secret_base32"].as_str().expect("secret").to_string();
    let confirmed = send(
        &app,
        Method::POST,
        "/api/auth/mfa/totp/enroll-verify",
        Some(&target_bearer),
        Some(json!({ "code": totp_code(&secret, 0) })),
    )
    .await;
    assert_eq!(confirmed.status(), StatusCode::OK);

    let mut tx = state.pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("scope");
    assert!(
        !core_db::mfa::user_mfa_reenroll_required(&mut *tx, target_id)
            .await
            .expect("flag"),
        "confirming a factor clears the requirement"
    );
    tx.commit().await.expect("commit");

    // The permission gate is real: a principal without `manage_users` is
    // refused before any factor is touched.
    let mut roleless_extensions = axum::http::Extensions::new();
    roleless_extensions.insert(AuthedUser {
        principal: core_domain::entities::Principal {
            user: admin_authed(&state, &owner).await.principal.user.clone(),
            roles: vec![],
            permissions: vec![Permission::ManageBilling],
        },
        session_id: Some(id_of(&owner, "session_id")),
        via_api_key: false,
    });
    let refused = mfa_handlers::admin_reset_user_factors(
        axum::extract::State(state.clone()),
        roleless_extensions,
        axum::extract::Path(target_id),
    )
    .await;
    assert!(matches!(
        refused,
        Err(core_api::error::ApiError::Forbidden(_))
    ));
}
