//! End-to-end edge-assertion federation flow (Wave 13 lane 7, ID2–ID4)
//! against a real in-process JWKS server, driven through the actual
//! core-api router.
//!
//! Proves: deny-by-default (unknown + disabled sources uniformly
//! rejected), signature verification against the pinned team JWKS
//! (tampered/wrong-key assertions refused), issuer/audience pinning
//! (byte-for-byte), conservative linking (no link ⇒ typed
//! `edge_identity_unlinked`, never JIT), same session machinery
//! (auth_sessions chain via the standard token pair), and the org MFA
//! gate applying to federated logins.

use axum::{routing::get, Json, Router};
use core_api::{config::Config, state::AppState};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("edge e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
        pool,
        config: Arc::new(config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
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
    body: Option<serde_json::Value>,
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

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn register_org(app: &axum::Router, slug: &str) -> serde_json::Value {
    let tokens = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/register",
            None,
            Some(json!({
                "org_name": "Edge Co",
                "org_slug": slug,
                "email": format!("owner@{slug}.test"),
                "password": "correct-horse-battery",
                "display_name": "Owner",
            })),
        )
        .await,
    )
    .await;
    // The register response is a plain token pair; /me resolves the ids
    // the tests need (user_id, org_id) under the same credentials.
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&bearer), None).await).await;
    json!({
        "access_token": tokens["access_token"],
        "refresh_token": tokens["refresh_token"],
        "session_id": tokens["session_id"],
        "user_id": me["user_id"],
        "org_id": me["org_id"],
    })
}

// ---------------------------------------------------------------------------
// mock team JWKS — a real HTTP server serving the RSA public key the
// assertions are signed with
// ---------------------------------------------------------------------------

struct JwksState {
    n_b64: String,
    e_b64: String,
    kid: &'static str,
}

fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

async fn spawn_jwks() -> (String, String, Arc<JwksState>) {
    let mut rng = rand::rngs::OsRng;
    let private = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa keygen (test-only)");
    let public = rsa::RsaPublicKey::from(&private);
    let signing_pem = private
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .expect("pem")
        .to_string();
    let state = Arc::new(JwksState {
        n_b64: b64(&public.n().to_bytes_be()),
        e_b64: b64(&public.e().to_bytes_be()),
        kid: "edge-test-key",
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");

    let app = Router::new()
        .route(
            "/cdn-cgi/access/certs",
            get(
                |axum::extract::State(s): axum::extract::State<Arc<JwksState>>| async move {
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
                },
            ),
        )
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("{base}/cdn-cgi/access/certs"), signing_pem, state)
}

/// Mints a `Cf-Access-Jwt-Assertion`-shaped signed JWT (RS256, kid bound).
fn mint_assertion(
    signing_pem: &str,
    kid: &str,
    iss: &str,
    aud: &str,
    sub: &str,
    email: &str,
    exp_delta: i64,
) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": iss,
        "sub": sub,
        "aud": aud,
        "exp": now + exp_delta,
        "iat": now,
        "email": email,
        "name": "Edge User",
    });
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(signing_pem.as_bytes()).unwrap();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(kid.to_string());
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
}

// ---------------------------------------------------------------------------
// helpers over the API surface
// ---------------------------------------------------------------------------

fn unique_source() -> String {
    format!("edge-{}", Uuid::now_v7().simple())
}

async fn configure_edge_source(
    app: &axum::Router,
    bearer: &str,
    name: &str,
    jwks_url: &str,
    enabled: bool,
) -> serde_json::Value {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/auth/edge/sources/{name}"),
        Some(bearer),
        Some(json!({
            "display_name": "Corp Access",
            "issuer": "https://team.cloudflareaccess.com",
            "audience": "edge.aud.tag",
            "jwks_url": jwks_url,
            "enabled": enabled,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "edge source upsert succeeds");
    body_json(response).await
}

async fn link_edge_identity(
    app: &axum::Router,
    bearer: &str,
    name: &str,
    user_id: &str,
    subject: &str,
) -> serde_json::Value {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/auth/edge/sources/{name}/links"),
        Some(bearer),
        Some(json!({
            "user_id": user_id,
            "subject": subject,
            "email": "edge.user@corp.test",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "admin link creation succeeds");
    body_json(response).await
}

async fn exchange(
    app: &axum::Router,
    name: &str,
    assertion: Option<&str>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(format!("/api/auth/edge/{name}/exchange"));
    if let Some(a) = assertion {
        builder = builder.header("cf-access-jwt-assertion", a);
    }
    app.clone()
        .oneshot(builder.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn error_reason(response: axum::response::Response) -> (axum::http::StatusCode, String) {
    let status = response.status();
    let v = body_json(response).await;
    (status, v["error"]["reason"].as_str().unwrap().to_string())
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// Deny-by-default end to end: a source that does not exist and a source
/// that exists but is DISABLED are indistinguishable typed rejections;
/// only the admin's explicit enable turns authentication on.
#[tokio::test(flavor = "multi_thread")]
async fn edge_sources_are_deny_by_default() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("edge-dd-{}", Uuid::now_v7().simple());
    let tokens = register_org(&app, &slug).await;
    let bearer = tokens["access_token"].as_str().unwrap();

    let (jwks_url, signing_pem, _jwks) = spawn_jwks().await;
    let assertion = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        300,
    );

    // Unknown source: typed, uniform rejection.
    let (status, reason) =
        error_reason(exchange(&app, "no-such-source", Some(&assertion)).await).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(reason, "edge_source_unknown");

    // Created DISABLED (deny-by-default): same rejection — the source
    // cannot be enumerated as "exists but off".
    let disabled_source = unique_source();
    let info = configure_edge_source(&app, bearer, &disabled_source, &jwks_url, false).await;
    assert_eq!(info["enabled"], false, "deny-by-default posture");
    let (status, reason) =
        error_reason(exchange(&app, &disabled_source, Some(&assertion)).await).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(reason, "edge_source_unknown");

    // Missing header: typed rejection, still not source enumeration
    // (enabled now, but no assertion at all).
    let enabled_source = unique_source();
    configure_edge_source(&app, bearer, &enabled_source, &jwks_url, true).await;
    let (status, reason) = error_reason(exchange(&app, &enabled_source, None).await).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(reason, "edge_assertion_invalid");
}

/// Signature verification against the pinned team JWKS with BOTH pins
/// enforced; unlinked identities are refused with the typed
/// `edge_identity_unlinked` reason (no JIT for edge sources).
#[tokio::test(flavor = "multi_thread")]
async fn edge_assertion_requires_signature_pins_and_link() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("edge-pin-{}", Uuid::now_v7().simple());
    let tokens = register_org(&app, &slug).await;
    let bearer = tokens["access_token"].as_str().unwrap();

    let (jwks_url, signing_pem, _jwks) = spawn_jwks().await;
    let source = unique_source();
    configure_edge_source(&app, bearer, &source, &jwks_url, true).await;

    // A DIFFERENT key signs this one: signature verification against the
    // team JWKS must refuse it — raw header trust is never valid.
    let (_other_url, other_pem, _o) = spawn_jwks().await;
    let forged = mint_assertion(
        &other_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        300,
    );
    let (status, reason) = error_reason(exchange(&app, &source, Some(&forged)).await).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(reason, "edge_assertion_invalid");

    // Wrong AUDIENCE pin: same collapsed reason, no check fingerprinting.
    let wrong_aud = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "other.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        300,
    );
    let (status, reason) = error_reason(exchange(&app, &source, Some(&wrong_aud)).await).await;
    assert_eq!(reason, "edge_assertion_invalid");
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);

    // Wrong ISSUER pin: same collapse.
    let wrong_iss = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://evil.example.com",
        "edge.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        300,
    );
    let (status, reason) = error_reason(exchange(&app, &source, Some(&wrong_iss)).await).await;
    assert_eq!(reason, "edge_assertion_invalid");
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);

    // Expired assertion: same collapse. (2 min stale — beyond the 60s
    // leeway jsonwebtoken grants, so this is a true expiry refusal.)
    let expired = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        -120,
    );
    let (status, reason) = error_reason(exchange(&app, &source, Some(&expired)).await).await;
    assert_eq!(reason, "edge_assertion_invalid");
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);

    // Valid assertion, NO link: conservative refusal with a distinct,
    // actionable reason — and still no tokens.
    let good = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-1",
        "edge.user@corp.test",
        300,
    );
    let (status, reason) = error_reason(exchange(&app, &source, Some(&good)).await).await;
    assert_eq!(reason, "edge_identity_unlinked");
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
}

/// The full happy path: valid assertion → admin link → tokens from the
/// SAME session machinery as password login (a real auth_sessions chain
/// root that the standard /auth/refresh accepts).
#[tokio::test(flavor = "multi_thread")]
async fn edge_exchange_resolves_into_the_same_session_machinery() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("edge-ok-{}", Uuid::now_v7().simple());
    let tokens = register_org(&app, &slug).await;
    let bearer = tokens["access_token"].as_str().unwrap();
    let user_id = tokens["user_id"].as_str().unwrap().to_string();

    let (jwks_url, signing_pem, _jwks) = spawn_jwks().await;
    let source = unique_source();
    configure_edge_source(&app, bearer, &source, &jwks_url, true).await;

    let good = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-ok",
        "edge.user@corp.test",
        300,
    );

    // Link the edge subject to the local owner, then exchange.
    link_edge_identity(&app, bearer, &source, &user_id, "edge-sub-ok").await;
    let response = exchange(&app, &source, Some(&good)).await;
    assert_eq!(response.status(), 200, "linked identity gets tokens");
    let v = body_json(response).await;
    let access = v["access_token"].as_str().expect("token pair shape");
    let refresh = v["refresh_token"].as_str().unwrap();
    let session_id = v["session_id"].as_str().unwrap();
    assert!(!access.is_empty() && !refresh.is_empty());

    // Same session machinery: the refresh chain accepts the pair and
    // rotates (proving auth_sessions lineage, not a parallel system).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/auth/refresh",
        None,
        Some(json!({
            "refresh_token": refresh,
            "session_id": session_id,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "refresh chain works for edge login");
    let rotated = body_json(response).await;
    assert_ne!(
        rotated["access_token"].as_str().unwrap(),
        access,
        "access token rotates"
    );
}

/// Org MFA policy applies to federated logins: with a second factor
/// enrolled, the exchange returns the SAME `mfa_pending` bridge password
/// login returns — never tokens.
#[tokio::test(flavor = "multi_thread")]
async fn edge_login_respects_the_org_mfa_gate() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("edge-mfa-{}", Uuid::now_v7().simple());
    let tokens = register_org(&app, &slug).await;
    let bearer = tokens["access_token"].as_str().unwrap();
    let user_id = tokens["user_id"].as_str().unwrap().to_string();
    let org_id = tokens["org_id"].as_str().unwrap().to_string();

    let (jwks_url, signing_pem, _jwks) = spawn_jwks().await;
    let source = unique_source();
    configure_edge_source(&app, bearer, &source, &jwks_url, true).await;
    link_edge_identity(&app, bearer, &source, &user_id, "edge-sub-mfa").await;

    // Enroll a (test-only) confirmed TOTP factor: the org MFA gate keys
    // on any confirmed enrollment, exactly as password login does.
    let sealed = core_api::crypto::seal(&state.config.secret_key, b"test-secret").unwrap();
    sqlx::query(
        "INSERT INTO totp_secrets (id, org_id, user_id, secret_enc, confirmed) \
                 VALUES ($1, $2, $3, $4, true)",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::parse_str(&org_id).unwrap())
    .bind(Uuid::parse_str(&user_id).unwrap())
    .bind(&sealed)
    .execute(&state.pool)
    .await
    .expect("enroll totp row");

    let good = mint_assertion(
        &signing_pem,
        "edge-test-key",
        "https://team.cloudflareaccess.com",
        "edge.aud.tag",
        "edge-sub-mfa",
        "edge.user@corp.test",
        300,
    );
    let response = exchange(&app, &source, Some(&good)).await;
    assert_eq!(response.status(), 200, "MFA bridge is a 200, not an error");
    let v = body_json(response).await;
    assert_eq!(v["mfa_required"], true, "mfa bridge returned");
    assert!(
        v["mfa_token"].as_str().is_some(),
        "mfa_pending bridge token present"
    );
    assert!(
        v["access_token"].is_null(),
        "no tokens while a second factor is enrolled"
    );
}
