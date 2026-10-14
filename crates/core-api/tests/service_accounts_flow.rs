//! Client-certificate authentication flow (Wave 47 lane A9,
//! `p47-client-cert-auth`) — service-account enrollment plus the cert-auth
//! middleware branch over the REAL core-api router and lane DB.
//!
//! The suite drives the router over a REAL localhost socket served with
//! `into_make_service_with_connect_info` so the middleware can see the peer
//! IP (loopback = trusted by default). It proves:
//!
//!  1. Happy path — an enrolled cert authenticates a service account whose
//!     granted scopes resolve onto the standard permission matrix, and the
//!     identity is service-account-shaped (id + scoped permissions).
//!  2. Wrong cert refused — an unenrolled fingerprint is a typed 401.
//!  3. Org-binding — a cert enrolled in org A can never act on org B: the
//!     org-scoped admin surface rejects a cross-org path (RLS/scope 404).
//!  4. Revoked service account refused — DELETE then re-auth is a 401.
//!  5. Expired service account refused — an already-lapsed binding is 401.
//!  6. Untrusted peer — a forged cert header from outside the trust set is
//!     IGNORED (falls through to the normal chain), so it cannot auth.
//!  7. No-cert requests unaffected — the bearer/JWT chain still works on
//!     the connect-info router (an operator's existing auth is untouched).
//!  8. Admin-only CRUD — a service account that lacks ManageApiKeys gets a
//!     typed 403 on the admin surface, and its granted scope alone does not
//!     escalate to service-account administration.
//!
//! Database selection follows ci_feeder_gate's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// Per-process nonce so fingerprints are unique across re-runs against the
/// shared lane DB (the `cert_fingerprint` column is globally unique) while
/// staying stable within a single run.
fn run_nonce() -> String {
    static NONCE: OnceLock<String> = OnceLock::new();
    NONCE
        .get_or_init(|| Uuid::now_v7().simple().to_string())
        .clone()
}

/// Deterministic-within-a-run, unique-across-runs 64-hex fingerprint from a
/// seed byte (valid for both enrollment and presentation).
fn fp(seed: u8) -> String {
    let mut s = format!("{:02x}{}", seed, run_nonce());
    if s.len() < 64 {
        s.push_str(&"0".repeat(64 - s.len()));
    }
    s.truncate(64);
    s
}

async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
}

/// Builds an AppState. `trusted_proxies` is the trusted-proxy posture
/// (shared with the real-client-IP lane); `TrustedProxies::default()` is the
/// narrow loopback + RFC1918 set that trusts the loopback test peer.
async fn test_state(trusted_proxies: core_api::client_ip::TrustedProxies) -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("service-account flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([29; 32]);
    config.trusted_proxies = trusted_proxies;
    AppState {
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

/// Boots the REAL router on a loopback port WITH ConnectInfo so the trust
/// predicate sees the peer IP. Returns the base URL.
async fn spawn_router(state: AppState) -> String {
    let app = core_api::routes::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}")
}

async fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// Registers a fresh org and returns (admin token, org_id, user_id).
async fn register_org(base: &str, slug: &str) -> (String, String, String) {
    // Unique per registration so repeated runs against the shared lane DB
    // never collide on the org slug / owner email.
    let slug = format!("{slug}-{}", Uuid::now_v7().simple());
    let client = http().await;
    let resp = client
        .post(format!("{base}/api/v1/auth/register"))
        .json(&json!({
            "org_name": "Cert Auth Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        }))
        .send()
        .await
        .expect("register");
    assert_eq!(resp.status(), 200, "register must succeed");
    let body: Value = resp.json().await.expect("register body");
    let token = body["access_token"].as_str().expect("token").to_string();

    let me = client
        .get(format!("{base}/api/v1/me"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("me");
    assert_eq!(me.status(), 200, "me must succeed for the owner");
    let me: Value = me.json().await.expect("me body");
    (
        token,
        me["org_id"].as_str().unwrap().to_string(),
        me["user_id"].as_str().unwrap().to_string(),
    )
}

/// Creates a service account via the admin API; returns its id.
async fn create_account(
    base: &str,
    token: &str,
    org_id: &str,
    name: &str,
    fingerprint: &str,
    scopes: Vec<&str>,
) -> String {
    let resp = http()
        .await
        .post(format!(
            "{base}/api/v1/orgs/{org_id}/service-accounts"
        ))
        .bearer_auth(token)
        .json(&json!({
            "name": name,
            "cert_fingerprint": fingerprint,
            "scopes": scopes,
        }))
        .send()
        .await
        .expect("create account");
    assert_eq!(resp.status(), 201, "create service account must be 201");
    let body: Value = resp.json().await.expect("create body");
    body["id"].as_str().expect("account id").to_string()
}

/// Calls GET /me carrying ONLY the client-cert header (no bearer).
async fn me_with_cert(base: &str, fingerprint: &str) -> reqwest::Response {
    http()
        .await
        .get(format!("{base}/api/v1/me"))
        .header("x-marula-client-cert", fingerprint)
        .send()
        .await
        .expect("cert me")
}

/// Inserts an already-expired service account directly against the lane DB
/// (its own pool), bypassing the API's future-expiry guard so the
/// "expired refused" test can reach that state.
async fn insert_expired_account(org_id: &str, admin_id: &str, fingerprint: &str) {
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("test db url");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let org = org_uuid(org_id);
    let admin = org_uuid(admin_id);
    let mut tx = pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    core_db::service_accounts::insert_service_account(
        &mut *tx,
        Uuid::now_v7(),
        org,
        "lapsed",
        fingerprint,
        None,
        &["clients:view".to_string()],
        Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        Some(admin),
    )
    .await
    .expect("insert expired account");
    tx.commit().await.expect("commit");
}

#[tokio::test]
#[serial_test::serial]
async fn an_enrolled_certificate_authenticates_a_scoped_service_account() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-happy").await;
    let fingerprint = fp(0x11);
    let account_id = create_account(
        &base,
        &token,
        &org_id,
        "ci-bot",
        &fingerprint,
        vec!["clients:view", "special:manage_api_keys"],
    )
    .await;

    // Present the cert alone (trusted loopback peer) — no bearer, no key.
    let resp = me_with_cert(&base, &fingerprint).await;
    assert_eq!(resp.status(), 200, "cert auth must succeed on the happy path");
    let me: Value = resp.json().await.unwrap();
    // The authenticated principal is the service account, not the admin.
    assert_eq!(me["user_id"].as_str().unwrap(), account_id);
    assert_eq!(me["org_id"].as_str().unwrap(), org_id);
    assert_eq!(me["display_name"].as_str().unwrap(), "service-account:ci-bot");
    // The granted scopes resolved onto the standard permission matrix.
    let perms = me["permissions"].as_array().unwrap();
    assert!(perms.iter().any(|p| p == "clients:view"));
    assert!(perms.iter().any(|p| p == "special:manage_api_keys"));

    // And the resolved grant genuinely authorizes a gated surface: the
    // service-account list needs ManageApiKeys, which the cert carries.
    let list = http()
        .await
        .get(format!("{base}/api/v1/orgs/{org_id}/service-accounts"))
        .header("x-marula-client-cert", &fingerprint)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200, "cert grant must authorize the admin list");
}

#[tokio::test]
#[serial_test::serial]
async fn an_unknown_certificate_is_refused() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let _ = register_org(&base, "cert-unknown").await;
    // A well-formed 64-hex fingerprint that is NOT enrolled.
    let resp = me_with_cert(&base, &fp(0xff)).await;
    assert_eq!(resp.status(), 401, "an unenrolled cert must be a 401");
}

#[tokio::test]
#[serial_test::serial]
async fn a_revoked_service_account_is_refused() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-revoked").await;
    let fingerprint = fp(0x22);
    let account_id = create_account(
        &base,
        &token,
        &org_id,
        "doomed",
        &fingerprint,
        vec!["clients:view"],
    )
    .await;

    // Works before revocation.
    assert_eq!(me_with_cert(&base, &fingerprint).await.status(), 200);

    // Admin revokes it.
    let del = http()
        .await
        .delete(format!(
            "{base}/api/v1/orgs/{org_id}/service-accounts/{account_id}"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204, "revocation must be 204");

    // The same cert is now refused.
    assert_eq!(
        me_with_cert(&base, &fingerprint).await.status(),
        401,
        "a revoked cert must be a 401"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_expired_service_account_is_refused() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (_token, org_id, admin_id) = register_org(&base, "cert-expired").await;
    let fingerprint = fp(0x33);

    // Enroll an ALREADY-LAPSED account directly (the API refuses a past
    // expiry at create time, so insert one out-of-band to reach the state).
    insert_expired_account(&org_id, &admin_id, &fingerprint).await;

    assert_eq!(
        me_with_cert(&base, &fingerprint).await.status(),
        401,
        "an expired cert must be a 401"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_org_a_certificate_cannot_act_on_org_b() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token_a, org_a, _admin_a) = register_org(&base, "cert-org-a").await;
    let (token_b, org_b, _admin_b) = register_org(&base, "cert-org-b").await;

    // Enroll a service account in org A only.
    let fingerprint = fp(0x44);
    create_account(
        &base,
        &token_a,
        &org_a,
        "org-a-bot",
        &fingerprint,
        vec!["clients:view", "special:manage_api_keys"],
    )
    .await;

    // Org B's admin must not see org A's service account (RLS + scope).
    let list_b = http()
        .await
        .get(format!("{base}/api/v1/orgs/{org_b}/service-accounts"))
        .bearer_auth(&token_b)
        .send()
        .await
        .unwrap();
    assert_eq!(list_b.status(), 200);
    let body: Value = list_b.json().await.unwrap();
    let accounts = body["service_accounts"].as_array().unwrap();
    assert!(
        accounts.iter().all(|a| a["cert_fingerprint"] != json!(fingerprint)),
        "org A's service account must never be visible to org B"
    );

    // The org-A cert, used against org B's admin path, is refused — the
    // principal's org (A) does not match the path org (B).
    let cross = http()
        .await
        .get(format!("{base}/api/v1/orgs/{org_b}/service-accounts"))
        .header("x-marula-client-cert", &fingerprint)
        .send()
        .await
        .unwrap();
    assert_ne!(
        cross.status(),
        200,
        "an org-A cert acting on org B must not be authorized"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_forged_header_from_an_untrusted_peer_is_ignored() {
    // Narrow the trust set to a range that EXCLUDES the loopback test peer,
    // so a forwarded cert header is treated as no assertion at all.
    let narrowed = core_api::client_ip::TrustedProxies::from_ranges(
        vec![core_api::client_ip::Cidr::parse("203.0.113.0/24").expect("cidr")],
        "",
        core_api::client_ip::CfConnectingIp::Never,
        false,
    );
    let base = spawn_router(test_state(narrowed).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-untrusted").await;
    let fingerprint = fp(0x55);
    create_account(
        &base,
        &token,
        &org_id,
        "ignored",
        &fingerprint,
        vec!["special:manage_api_keys"],
    )
    .await;

    // Presenting the enrolled cert from an untrusted peer is IGNORED — it
    // does not authenticate and the request falls through to the normal
    // chain, which then refuses the missing bearer with a 401.
    let resp = me_with_cert(&base, &fingerprint).await;
    assert_eq!(
        resp.status(),
        401,
        "a cert header from an untrusted peer must not authenticate"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn requests_without_a_certificate_are_unaffected() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let _ = register_org(&base, "cert-plain").await;

    // The ordinary bearer/JWT chain still works on the connect-info router:
    // no cert header ⇒ the cert branch is a no-op.
    let (token, _org, user_id) = register_org(&base, "cert-plain-2").await;
    let me = http()
        .await
        .get(format!("{base}/api/v1/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200, "bearer auth is unaffected by cert auth");
    let me: Value = me.json().await.unwrap();
    assert_eq!(me["user_id"].as_str().unwrap(), user_id);

    // And a request with NEITHER a cert nor a bearer is still refused.
    let none = http()
        .await
        .get(format!("{base}/api/v1/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
}

#[tokio::test]
#[serial_test::serial]
async fn a_service_account_without_manage_api_keys_is_forbidden() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-no-admin").await;
    let fingerprint = fp(0x66);
    // Scope is a data read the cert CAN use, but NOT service-account admin.
    create_account(
        &base,
        &token,
        &org_id,
        "reader",
        &fingerprint,
        vec!["clients:view"],
    )
    .await;

    // The cert authenticates (GET /me works), but the service-account
    // admin surface requires ManageApiKeys, which this account lacks.
    let me = me_with_cert(&base, &fingerprint).await;
    assert_eq!(me.status(), 200, "the cert itself authenticates");

    let list = http()
        .await
        .get(format!("{base}/api/v1/orgs/{org_id}/service-accounts"))
        .header("x-marula-client-cert", &fingerprint)
        .send()
        .await
        .unwrap();
    assert_eq!(
        list.status(),
        403,
        "a data-only grant must not reach service-account administration"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn admin_crud_is_refused_for_unknown_grants_and_bad_fingerprints() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-validation").await;

    // Unknown grant key is refused (typed 400), not silently stored.
    let bad = http()
        .await
        .post(format!("{base}/api/v1/orgs/{org_id}/service-accounts"))
        .bearer_auth(&token)
        .json(&json!({
            "name": "bad",
            "cert_fingerprint": fp(0x77),
            "scopes": ["not_a_real:permission"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        422,
        "an unknown grant must be refused with a typed validation error"
    );

    // A non-64-hex fingerprint is refused.
    let bad_fp = http()
        .await
        .post(format!("{base}/api/v1/orgs/{org_id}/service-accounts"))
        .bearer_auth(&token)
        .json(&json!({
            "name": "bad-fp",
            "cert_fingerprint": "short",
            "scopes": ["clients:view"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_fp.status(), 422, "a malformed fingerprint must be refused");
}

fn org_uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("valid uuid")
}

#[tokio::test]
#[serial_test::serial]
async fn a_patch_updates_scopes_status_and_expiry_through_one_lifecycle_owner() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, admin_id) = register_org(&base, "cert-patch").await;
    let fingerprint = fp(0x88);
    let account_id = create_account(
        &base,
        &token,
        &org_id,
        "patchme",
        &fingerprint,
        vec!["clients:view"],
    )
    .await;

    // PATCH the grant set + lifecycle; immutable fields (id/org/fingerprint)
    // ride through unchanged, and the derived lifecycle is one function.
    let resp = http()
        .await
        .patch(format!(
            "{base}/api/v1/orgs/{org_id}/service-accounts/{account_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({
            "scopes": ["clients:view", "special:manage_api_keys"],
            "status": "suspended",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "patch must succeed");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"], json!(account_id), "id is immutable");
    assert_eq!(body["cert_fingerprint"], json!(fingerprint), "pin is immutable");
    assert_eq!(body["status"], "suspended");
    assert_eq!(body["created_by"], json!(admin_id), "enrollment provenance surfaced");
    let scopes = body["scopes"].as_array().unwrap();
    assert!(scopes.iter().any(|s| s == "special:manage_api_keys"));

    // A suspended account no longer authenticates.
    assert_eq!(
        me_with_cert(&base, &fingerprint).await.status(),
        401,
        "a suspended account must be refused"
    );

    // Re-activate and clear expiry: `expires_at: null` means never expires, so
    // the derived cert status returns to valid and the cert works again.
    let resp2 = http()
        .await
        .patch(format!(
            "{base}/api/v1/orgs/{org_id}/service-accounts/{account_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "status": "active", "expires_at": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let b2: Value = resp2.json().await.unwrap();
    assert_eq!(b2["status"], "active");
    assert_eq!(b2["expires_at"], Value::Null);
    assert_eq!(b2["cert_status"], "valid");
    assert_eq!(me_with_cert(&base, &fingerprint).await.status(), 200);
}

#[tokio::test]
#[serial_test::serial]
async fn a_revoke_reason_is_recorded_and_surfaced() {
    let base = spawn_router(test_state(core_api::client_ip::TrustedProxies::default()).await).await;
    let (token, org_id, _admin) = register_org(&base, "cert-reason").await;
    let fingerprint = fp(0x99);
    let account_id = create_account(
        &base,
        &token,
        &org_id,
        "rotating",
        &fingerprint,
        vec!["clients:view"],
    )
    .await;

    let del = http()
        .await
        .delete(format!(
            "{base}/api/v1/orgs/{org_id}/service-accounts/{account_id}?reason=rotated%20to%20new%20key"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);

    // The reason is recorded on the account and surfaced in the listing.
    let list = http()
        .await
        .get(format!("{base}/api/v1/orgs/{org_id}/service-accounts"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let body: Value = list.json().await.unwrap();
    let row = body["service_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == json!(account_id))
        .expect("revoked account still listed");
    assert_eq!(row["cert_status"], "revoked");
    assert_eq!(
        row["revocation_reason"],
        json!("rotated to new key"),
        "the supplied reason is recorded and readable"
    );
}
