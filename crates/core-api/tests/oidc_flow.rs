//! End-to-end OIDC SSO flow against a REAL in-process mock identity
//! provider (discovery → authorize → code exchange → RS256 id_token via
//! JWKS → conservative linking → single-use token handoff), driven through
//! the actual core-api router.
//!
//! Purpose: prove state binding, nonce echo, PKCE presence, secret-at-rest,
//! link-required conservatism, and JIT provisioning over real HTTP shapes.
//! Non-purpose: browser WebAuthn ceremonies (hardware-bound; verified
//! manually per docs/auth-hardening.md) and unit-level policy matrices.

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
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

async fn test_state() -> AppState {
    // Surface server-side tracing (ApiError logs DB failures) in test output.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("auth e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    // Production parity: deploy/docker-compose.yml runs core-api against the
    // table-owner role (RLS-exempt); the constrained meridian_app role is
    // exercised per-transaction by the core-db suites.
    let mut config = Config::from_env();
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: std::sync::Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
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

/// Registers a fresh org; returns the owner's tokens.
async fn register_org(app: &axum::Router, slug: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "SSO Co",
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

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

fn location_of(response: &axum::response::Response) -> String {
    let status = response.status();
    response
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("expected redirect (got {status}) missing Location"))
}

fn query_map(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .expect("url parses")
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// mock IdP — a real HTTP server speaking discovery/authorize/token/jwks
// ---------------------------------------------------------------------------

struct IdpState {
    issuer: String,
    client_id: String,
    client_secret: String,
    signing_pem: String,
    kid: &'static str,
    n_b64: String,
    e_b64: String,
    /// Identity asserted by every id_token this instance mints.
    email: String,
    /// authorization-code → nonce bound into that code.
    codes: std::sync::Mutex<HashMap<String, String>>,
    /// Next token call lies about the nonce (negative test).
    corrupt_next_nonce: std::sync::Mutex<bool>,
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

async fn spawn_idp(client_id: &str, client_secret: &str, email: &str) -> (String, Arc<IdpState>) {
    let mut rng = rand::rngs::OsRng;
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
        codes: std::sync::Mutex::new(HashMap::new()),
        corrupt_next_nonce: std::sync::Mutex::new(false),
    });

    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(|AxumState(s): AxumState<Arc<IdpState>>| async move {
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
                    // Fake authorization: the code carries the nonce so the
                    // token endpoint can echo it like a real provider.
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
                    // client-secret-basic
                    let expected_basic = data_encoding::BASE64
                        .encode(format!("{}:{}", s.client_id, s.client_secret).as_bytes());
                    let got = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert_eq!(got, format!("Basic {expected_basic}"), "basic auth");

                    let nonce = s
                        .codes
                        .lock()
                        .unwrap()
                        .get(&form.code)
                        .cloned()
                        .expect("unknown or replayed code");

                    let lying = *s.corrupt_next_nonce.lock().unwrap();
                    let issued_nonce = if lying {
                        format!("{nonce}-WRONG")
                    } else {
                        nonce
                    };

                    let now = chrono::Utc::now().timestamp();
                    let claims = json!({
                        "iss": s.issuer,
                        "sub": "subject-xyz",
                        "aud": s.client_id,
                        "exp": now + 300,
                        "iat": now,
                        "email": s.email,
                        "email_verified": true,
                        "name": "SSO User",
                        "nonce": issued_nonce,
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

// ---------------------------------------------------------------------------
// helpers over the API surface
// ---------------------------------------------------------------------------

const CLIENT_ID: &str = "meridian-client";
const CLIENT_SECRET: &str = "idp-s3cret";

async fn configure_provider(app: &axum::Router, bearer: &str, name: &str, issuer: &str, jit: bool) {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/auth/oidc/providers/{name}"),
        Some(bearer),
        Some(json!({
            "display_name": "Mock Corp IdP",
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
}

/// Runs login → authorize → callback; returns (callback status, Location).
async fn drive_sso_login(
    app: &axum::Router,
    idp_base: &str,
    provider_name: &str,
) -> (axum::http::StatusCode, String) {
    // 1. Our redirect: signed state + nonce + PKCE challenge present.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/auth/oidc/{provider_name}/login"),
        None,
        None,
    )
    .await;
    if response.status() != 303 {
        let status = response.status();
        let body = body_json(response).await;
        panic!("login ({provider_name}) did not redirect: {status} {body}");
    }
    let authorize_url = location_of(&response);
    let q = query_map(&authorize_url);
    assert_eq!(q.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert!(authorize_url.starts_with(idp_base));
    assert!(q.contains_key("state") && q.contains_key("nonce"));
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256"),
        "PKCE S256 is always on"
    );
    assert!(q
        .get("redirect_uri")
        .map(String::as_str)
        .is_some_and(|r| r.ends_with(&format!("/api/auth/oidc/{provider_name}/callback"))));

    // 2. Real round trip to the mock IdP authorize endpoint (no follow).
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
        .unwrap_or_else(|| panic!("mock IdP authorize lost redirect; url={authorize_url}"));
    assert!(
        callback_url.contains("/api/auth/oidc/"),
        "returns to our callback"
    );
    assert!(
        !callback_url.contains("provider="),
        "no stray params in callback"
    );

    // 3. Our callback consumes code+state.
    let path_and_query = callback_url
        .split_once("//")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(_, pq)| format!("/{pq}"))
        .expect("callback url shape");
    let callback_response = send(app, axum::http::Method::GET, &path_and_query, None, None).await;
    let cb_status = callback_response.status();
    if !cb_status.is_redirection() {
        let body = body_json(callback_response).await;
        panic!("callback did not redirect: {cb_status} {body}");
    }
    (cb_status, location_of(&callback_response))
}

async fn exchange_code(app: &axum::Router, one_time: &str) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/sso/exchange",
        None,
        Some(json!({ "code": one_time })),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// The happy path end to end: explicit link flow binds the subject, then a
/// plain SSO login yields tokens through the single-use handoff code.
#[tokio::test]
async fn oidc_explicit_link_then_sso_login_issues_tokens() {
    let state = test_state().await;
    let app = core_api::router(state);
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("ssolink-{suffix}");
    let provider_name = format!("corp-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    let owner_email = format!("owner@{slug}.test");
    let (issuer, _idp) = spawn_idp(CLIENT_ID, CLIENT_SECRET, &owner_email).await;
    configure_provider(&app, owner, &provider_name, &issuer, false).await;

    // --- explicit link: authenticate first, then link ----------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/auth/oidc/{provider_name}/link"),
        Some(owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "link begin returns authorize URL");
    let link_body = body_json(response).await;
    let authorize_url = link_body["authorize_url"].as_str().unwrap().to_string();

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let authorize_response = http.get(&authorize_url).send().await.unwrap();
    let callback_url = authorize_response.headers()["location"].to_str().unwrap();
    let path = format!(
        "/{}",
        callback_url
            .split_once("//")
            .unwrap()
            .1
            .split_once('/')
            .unwrap()
            .1
    );
    let linked_callback = send(&app, axum::http::Method::GET, &path, None, None).await;
    assert_eq!(linked_callback.status(), 303, "link flow completes");
    let linked_location = location_of(&linked_callback);
    assert!(
        linked_location.contains("/sso"),
        "lands on web handoff page"
    );
    let handoff_q = query_map(&linked_location);
    let (_status, body) = exchange_code(&app, &handoff_q["code"]).await;
    assert_eq!(body["linked"], json!(true), "link confirmation handed back");

    // --- plain SSO login for the now-linked subject ------------------------
    let (status, location) = drive_sso_login(&app, &issuer, &provider_name).await;
    assert_eq!(status, 303, "callback redirects to web");
    let q = query_map(&location);
    assert!(q.contains_key("code"), "one-time code present");
    assert!(!location.contains("access_token"), "tokens never ride URLs");

    let (exchange_status, exchange_body) = exchange_code(&app, &q["code"]).await;
    assert_eq!(exchange_status, 200, "handoff code exchanges for tokens");
    assert!(exchange_body["access_token"].is_string());
    assert!(exchange_body["refresh_token"].is_string());

    // Tokens actually work against /api/me.
    let me = send(
        &app,
        axum::http::Method::GET,
        "/api/me",
        Some(exchange_body["access_token"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(me.status(), 200);
    let me_body = body_json(me).await;
    assert_eq!(me_body["email"], json!(owner_email));

    // The handoff code was single-use.
    let (replay_status, _) = exchange_code(&app, &q["code"]).await;
    assert_eq!(replay_status, 401, "second exchange of same code rejected");
}

/// Unlinked subject with JIT disabled ⇒ conservative LinkRequired, never a
/// surprise account.
#[tokio::test]
async fn oidc_unlinked_subject_with_jit_off_is_refused() {
    let state = test_state().await;
    let app = core_api::router(state);
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("ssonojit-{suffix}");
    let provider_name = format!("strict-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    let (issuer, _idp) = spawn_idp(CLIENT_ID, CLIENT_SECRET, "sso.user@example.test").await;
    configure_provider(&app, owner, &provider_name, &issuer, false).await;

    let (_status, location) = drive_sso_login(&app, &issuer, &provider_name).await;
    let q = query_map(&location);
    assert_eq!(
        q.get("error").map(String::as_str),
        Some("link_required"),
        "no account is created without an explicit link"
    );

    // The same subject under JIT-enabled configuration IS provisioned.
    let jit_provider = format!("jit-{provider_name}");
    configure_provider(&app, owner, &jit_provider, &issuer, true).await;
    let (status, jit_location) = drive_sso_login(&app, &issuer, &jit_provider).await;
    assert_eq!(status, 303);
    let jq = query_map(&jit_location);
    assert!(jq.contains_key("code"), "JIT login hands back tokens");
    let (xs, xbody) = exchange_code(&app, &jq["code"]).await;
    assert_eq!(xs, 200);
    let me = send(
        &app,
        axum::http::Method::GET,
        "/api/me",
        Some(xbody["access_token"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(me.status(), 200);
    let me_body = body_json(me).await;
    assert_eq!(me_body["email"], json!("sso.user@example.test"));
}

/// Tampered state ⇒ hard fail at the callback; wrong-nonce id_token ⇒ 401.
#[tokio::test]
async fn oidc_state_and_nonce_validation_reject_attacks() {
    let state = test_state().await;
    let app = core_api::router(state);
    let suffix = Uuid::now_v7().simple().to_string()[16..].to_string();
    let slug = format!("ssosec-{suffix}");
    let provider_name = format!("sec-{suffix}");
    let tokens = register_org(&app, &slug).await;
    let owner = tokens["access_token"].as_str().unwrap();

    let (issuer, idp) = spawn_idp(CLIENT_ID, CLIENT_SECRET, "sso.user@example.test").await;
    configure_provider(&app, owner, &provider_name, &issuer, false).await;

    // A. Attack: attacker supplies their OWN state (different signature).
    let foreign_state = "attacker-controlled-value";
    let callback =
        format!("/api/auth/oidc/{provider_name}/callback?code=whatever&state={foreign_state}");
    let forged = send(&app, axum::http::Method::GET, &callback, None, None).await;
    assert_eq!(forged.status(), 303);
    assert_eq!(
        query_map(&location_of(&forged))
            .get("error")
            .map(String::as_str),
        Some("bad_state"),
        "unsigned/garbage state is refused"
    );

    // B. Attack: valid envelope but the provider echoes a DIFFERENT nonce
    //    (authorization/code mixup).
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/auth/oidc/{provider_name}/login"),
        None,
        None,
    )
    .await;
    let authorize_url = location_of(&response);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    *idp.corrupt_next_nonce.lock().unwrap() = true;
    let authorize_response = http.get(&authorize_url).send().await.unwrap();
    let callback_url = authorize_response.headers()["location"].to_str().unwrap();
    let path = format!(
        "/{}",
        callback_url
            .split_once("//")
            .unwrap()
            .1
            .split_once('/')
            .unwrap()
            .1
    );
    let mismatched = send(&app, axum::http::Method::GET, &path, None, None).await;
    assert_eq!(
        mismatched.status(),
        401,
        "nonce mismatch aborts authentication outright"
    );
    let err_body = body_json(mismatched).await;
    assert!(err_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("nonce"));

    // C. Provider secret is stored encrypted: the raw plaintext never shows
    //    up in any read of provider metadata.
    let list = send(
        &app,
        axum::http::Method::GET,
        "/api/auth/oidc/providers",
        Some(owner),
        None,
    )
    .await;
    let raw = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    assert!(
        !text.contains(CLIENT_SECRET),
        "secret must never be returned"
    );
}
