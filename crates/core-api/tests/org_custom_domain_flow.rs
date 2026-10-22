//! HTTP flow tests for the per-org custom domain API (the private build tracker Wave 47 lane
//! A8, `p47-org-custom-domains`, migration 0135) over the REAL core-api
//! router.
//!
//! Purpose: prove the surface end to end with a DETERMINISTIC resolver
//! (installed via `core_api::dns_verify::install_txt_resolver`, so no real
//! DNS is touched):
//!   * state moves none → pending → verified → cleared, and the TXT token is
//!     the authority (a wrong/absent record leaves it pending).
//!   * the domain is normalized (case + trailing dot) and the manual
//!     instructions name `_marula-verify.<domain>`.
//!   * a domain is unique across orgs (a second claimant gets a typed 409).
//!   * a path org that is not the caller's own is a 404 for BOTH read and
//!     manage — the RLS/`NotFound` posture.
//!   * a non-admin human MEMBER reads their own state but is refused on
//!     mutations (`admin:edit` is the human manage gate).
//!   * a service account needs `org_custom_domain:read` to read and
//!     `org_custom_domain:manage` to mutate.
//!
//! Non-purpose: the hickory system resolver itself (unit-tested in
//! `core_api::dns_verify`), and TLS/routing (Core does neither).
//!
//! Database selection follows the documented contract: MARULA_TEST_DATABASE_URL
//! or DATABASE_URL (no schema reset).

use async_trait::async_trait;
use core_api::dns_verify::{install_txt_resolver, DnsLookupError, TxtResolver};
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

/// A resolver whose answers the test mutates between calls, so the same
/// process-global resolver serves "not published yet" and "published".
#[derive(Clone, Default)]
struct SharedDns {
    txt: Arc<Mutex<HashMap<String, Vec<String>>>>,
    addressable: Arc<Mutex<HashSet<String>>>,
}

impl SharedDns {
    fn publish_txt(&self, name: &str, value: &str) {
        self.txt
            .lock()
            .unwrap()
            .entry(name.to_string())
            .or_default()
            .push(value.to_string());
    }
    fn make_resolvable(&self, domain: &str) {
        self.addressable.lock().unwrap().insert(domain.to_string());
    }
}

#[async_trait]
impl TxtResolver for SharedDns {
    async fn lookup_txt(&self, name: &str) -> Result<Vec<String>, DnsLookupError> {
        Ok(self
            .txt
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_default())
    }
    async fn resolves_to_address(&self, domain: &str) -> Result<bool, DnsLookupError> {
        Ok(self.addressable.lock().unwrap().contains(domain))
    }
}

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("custom domain flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([41; 32]);
    AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

async fn send(
    app: &axum::Router,
    method: axum::http::Method,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(value.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    app.clone()
        .oneshot(request)
        .await
        .expect("infallible router")
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![("authorization", format!("Bearer {token}"))]
}

async fn register_org(app: &axum::Router, state: &AppState, slug: &str) -> (String, Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Domain Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let token = body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org id");
    (token, org_id)
}

#[tokio::test]
async fn custom_domain_full_lifecycle_and_authorization() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());

    let dns = SharedDns::default();
    assert!(
        install_txt_resolver(Arc::new(dns.clone())),
        "the deterministic resolver is installed once per test binary"
    );

    let slug_a = format!("dom-a-{}", Uuid::now_v7().simple());
    let (owner_a, org_a) = register_org(&app, &state, &slug_a).await;
    let slug_b = format!("dom-b-{}", Uuid::now_v7().simple());
    let (owner_b, org_b) = register_org(&app, &state, &slug_b).await;

    let base_a = format!("/api/v1/orgs/{org_a}/custom-domain");

    // --- initial state: nothing, and no provider plugin ---------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    assert_eq!(view["domain"], Value::Null);
    assert_eq!(view["status"], json!("none"));
    assert_eq!(view["provider_plugin_installed"], json!(false));

    // --- set: normalization + a token + manual instructions ------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &base_a,
        &bearer(&owner_a),
        Some(json!({ "domain": "Support.Example.COM." })),
    )
    .await;
    assert_eq!(response.status(), 200, "owner sets a domain");
    let set = body_json(response).await;
    assert_eq!(
        set["domain"],
        json!("support.example.com"),
        "the domain is normalized (lowercased, trailing dot stripped)"
    );
    let token = set["verification_token"]
        .as_str()
        .expect("token")
        .to_string();
    assert!(token.starts_with("marula-verify-"), "token shape: {token}");
    assert_eq!(
        set["instructions"]["verification_txt"]["name"],
        json!("_marula-verify.support.example.com")
    );
    assert_eq!(
        set["instructions"]["verification_txt"]["kind"],
        json!("TXT")
    );
    assert_eq!(
        set["instructions"]["verification_txt"]["value"],
        json!(token.as_str())
    );

    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["status"], json!("pending"));
    assert_eq!(view["verification_token"], json!(token.as_str()));
    assert!(view["verified_at"].is_null());

    // --- verify before the record exists: still pending, no error -----------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("{base_a}/verify"),
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let verify = body_json(response).await;
    assert_eq!(verify["verified"], json!(false));
    assert_eq!(verify["status"], json!("pending"));
    assert_eq!(
        verify["record_name"],
        json!("_marula-verify.support.example.com")
    );
    assert_eq!(verify["txt"]["observed"], json!([]));
    assert!(
        verify["txt"]["error"].is_null(),
        "an absent record is not a lookup error"
    );

    // --- a WRONG token does not verify even when the edge resolves ----------
    dns.publish_txt("_marula-verify.support.example.com", "marula-verify-wrong");
    dns.make_resolvable("support.example.com");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("{base_a}/verify"),
        &bearer(&owner_a),
        None,
    )
    .await;
    let verify = body_json(response).await;
    assert_eq!(
        verify["verified"],
        json!(false),
        "the TXT token is the authority"
    );
    assert_eq!(verify["edge"]["resolves"], json!(true));

    // --- publish the right token: verified -----------------------------------
    dns.publish_txt("_marula-verify.support.example.com", &token);
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("{base_a}/verify"),
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let verify = body_json(response).await;
    assert_eq!(verify["verified"], json!(true));
    assert_eq!(verify["status"], json!("verified"));

    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["status"], json!("verified"));
    assert!(
        view["verified_at"].is_string(),
        "a verified domain carries its timestamp"
    );

    // --- uniqueness: another org cannot claim the same domain ---------------
    let base_b = format!("/api/v1/orgs/{org_b}/custom-domain");
    let response = send(
        &app,
        axum::http::Method::POST,
        &base_b,
        &bearer(&owner_b),
        Some(json!({ "domain": "support.example.com" })),
    )
    .await;
    assert_eq!(
        response.status(),
        409,
        "a claimed domain is a typed conflict"
    );

    // --- cross-org is a 404 for BOTH read and manage ------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &base_b,
        &bearer(&owner_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "another org's state is a 404");
    let response = send(
        &app,
        axum::http::Method::POST,
        &base_b,
        &bearer(&owner_a),
        Some(json!({ "domain": "evil.example.com" })),
    )
    .await;
    assert_eq!(response.status(), 404, "managing another org is a 404");
    let _ = org_b;

    // --- a human MEMBER reads but cannot manage -----------------------------
    let invite = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users",
        &bearer(&owner_a),
        Some(json!({
            "email": format!("tech@{slug_a}.test"),
            "display_name": "Tech",
            "role": "technician",
        })),
    )
    .await;
    assert_eq!(invite.status(), 200, "invite must succeed");
    let setup_token = body_json(invite).await["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accept = send(
        &app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        &[],
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200);
    let tech = body_json(accept).await["access_token"]
        .as_str()
        .expect("tech token")
        .to_string();

    let response = send(&app, axum::http::Method::GET, &base_a, &bearer(&tech), None).await;
    assert_eq!(response.status(), 200, "a member reads their own state");
    let read = send(
        &app,
        axum::http::Method::POST,
        &base_a,
        &bearer(&tech),
        Some(json!({ "domain": "nope.example.com" })),
    )
    .await;
    assert_eq!(read.status(), 403, "a non-admin cannot set the domain");
    let del = send(
        &app,
        axum::http::Method::DELETE,
        &base_a,
        &bearer(&tech),
        None,
    )
    .await;
    assert_eq!(del.status(), 403, "a non-admin cannot clear the domain");

    // --- service accounts: read grant, wrong scope, manage grant ------------
    let key_resp = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer(&owner_a),
        Some(json!({ "name": "proxy-read", "scopes": ["org_custom_domain:read"] })),
    )
    .await;
    assert_eq!(key_resp.status(), 201, "the read scope is creatable");
    let read_key = body_json(key_resp).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &[("x-api-key", read_key.clone())],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "a read key reads the state");
    // A read key cannot manage.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &base_a,
        &[("x-api-key", read_key.clone())],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "a read key cannot clear the domain");

    let wrong = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer(&owner_a),
        Some(json!({ "name": "wrong", "scopes": ["tickets:read"] })),
    )
    .await;
    let wrong_key = body_json(wrong).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &[("x-api-key", wrong_key)],
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "a key without the scope is refused");

    let manage = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        &bearer(&owner_a),
        Some(json!({ "name": "dns-provider", "scopes": ["org_custom_domain:manage"] })),
    )
    .await;
    assert_eq!(manage.status(), 201, "the manage scope is creatable");
    let manage_key = body_json(manage).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();

    // --- clear (manage key), and the state returns to none ------------------
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &base_a,
        &[("x-api-key", manage_key)],
        None,
    )
    .await;
    assert_eq!(response.status(), 204, "a manage key clears the domain");

    let response = send(
        &app,
        axum::http::Method::GET,
        &base_a,
        &bearer(&owner_a),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["domain"], Value::Null);
    assert_eq!(view["status"], json!("none"));
    assert_eq!(view["verification_token"], Value::Null);
}
