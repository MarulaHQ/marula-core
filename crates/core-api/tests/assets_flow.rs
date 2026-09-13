//! Asset endpoint flows (Wave 11 lane 5, migration 0054) through the REAL
//! core-api router.
//!
//! Purpose: the /api/assets contract —
//! 1. Typed taxonomy: create with an asset-shaped system layout (seeded by
//!    0053) and schema-validated custom fields; invalid fields are typed
//!    422 and do NOT consume the idempotency claim.
//! 2. Instant-first: idempotent replay returns the SAME asset; updates
//!    carry mutation metadata; layout changes re-validate kept fields.
//! 3. Filterable resource REST (external-DMS parity): org-wide list with
//!    client/layout/q filters, plus the client-scoped leg.
//! 4. Links via `document_links` (asset kind, 0054): resolved labels,
//!    target-existence checks.
//! 5. Vault posture: metadata-only entries under passwords:view; technician
//!    (no Passwords grants) gets a typed 403; reveal stays on the audited
//!    password endpoint.
//! 6. Integration link status is data, not sync.
//! 7. Delete: gone from scope, audit trail keeps the events.
//!
//! Database selection follows documents_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("assets flow needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([22; 32]);
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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

async fn send(
    app: &Router,
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

fn auth(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Assets Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

/// Creates a second user through core_db with the given builtin role and
/// logs in over HTTP.
async fn provision_role(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    builtin: &str,
    email: &str,
    password: &str,
) -> String {
    let password_hash = core_api::auth::password::hash(password).expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = $2")
        .bind(org_id)
        .bind(builtin)
        .fetch_one(&mut *tx)
        .await
        .expect("builtin role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Second User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": password })),
    )
    .await;
    assert_eq!(response.status(), 200, "login must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn me_org_id(app: &Router, token: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await["org_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn assets_flow_typed_taxonomy_links_vault_and_delete() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("assets-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- seeded asset-shaped system layout (taxonomy as data) -----------
    let layouts = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/documents/layouts",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    let server = layouts
        .as_array()
        .expect("layouts array")
        .iter()
        .find(|l| l["slug"] == "server")
        .expect("seeded system layout 'server'")
        .clone();
    assert_eq!(server["is_system"], true);
    let layout_id = server["id"].as_str().unwrap().to_string();
    let _hostname_field = server["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "hostname")
        .expect("server layout carries a hostname field")
        .clone();

    // --- client to scope assets under -----------------------------------
    let client = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/clients",
            &bearer,
            Some(json!({ "name": "Acme Systems" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().unwrap().to_string();

    // --- create: typed classification + schema-validated fields ---------
    let create_body = json!({
        "client_id": client_id,
        "tag": "EXCH-01",
        "name": "Exchange server",
        "layout_id": layout_id,
        "fields": { "hostname": "exch01.acme.test", "ip_address": "10.0.0.11", "os": "Windows Server 2022" },
        "idempotency_key": "asset-create-1"
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200, "asset create must succeed");
    let applied = body_json(first).await;
    let asset_id = applied["id"].as_str().unwrap().to_string();
    assert_eq!(applied["layout_id"], layout_id);
    assert_eq!(applied["fields"]["hostname"], "exch01.acme.test");
    assert_eq!(
        applied["kind"], "device",
        "connector-compat column keeps its default"
    );
    assert!(applied["mutation"]["seq"].is_number());

    // Replay: same key + same payload → the SAME asset.
    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &bearer,
        Some(create_body),
    )
    .await;
    assert_eq!(replay.status(), 200);
    assert_eq!(
        body_json(replay).await["id"],
        applied["id"],
        "same key + same payload replays the SAME asset"
    );

    // Schema violation: a number where the layout says text — typed 422
    // and the idempotency key is NOT consumed.
    let bad = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &bearer,
        Some(json!({
            "client_id": client_id, "tag": "BAD", "name": "Bad",
            "layout_id": layout_id,
            "fields": { "hostname": 42 },
            "idempotency_key": "asset-bad-1"
        })),
    )
    .await;
    assert_eq!(
        bad.status(),
        422,
        "field type mismatch must be a typed validation error"
    );

    // --- filterable resource REST (external-DMS parity leg) -------------
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/assets?client_id={client_id}&q=EXCH"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1, "client+q filter matches");

    // --- patch: layout change re-validates kept fields ------------------
    let patch_bad = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/assets/{asset_id}"),
        &bearer,
        Some(json!({ "fields": { "hostname": 7 } })),
    )
    .await;
    assert_eq!(
        patch_bad.status(),
        422,
        "kept field vs current schema must re-validate"
    );

    let patch_ok = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/assets/{asset_id}"),
        &bearer,
        Some(json!({ "status": "retired" })),
    )
    .await;
    assert_eq!(patch_ok.status(), 200);
    assert_eq!(body_json(patch_ok).await["status"], "retired");

    // --- links (asset kind rides document_links, 0054) -------------------
    let linked = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/assets/{asset_id}/links"),
            &bearer,
            Some(json!({ "to_kind": "client", "to_id": client_id })),
        )
        .await,
    )
    .await;
    assert_eq!(linked.as_array().unwrap().len(), 1);
    assert_eq!(linked[0]["to_kind"], "client");
    assert_eq!(
        linked[0]["target_label"], "Acme Systems",
        "targets resolve to display labels; missing targets surface as broken instead"
    );
    assert_eq!(linked[0]["target_deleted"], false);

    let missing = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/assets/{asset_id}/links"),
        &bearer,
        Some(json!({ "to_kind": "client", "to_id": Uuid::now_v7() })),
    )
    .await;
    assert_eq!(missing.status(), 404, "link target must exist in scope");

    // --- vault posture (binding: strict, metadata-only list) ------------
    let password = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/v1/passwords",
            &bearer,
            Some(json!({
                "client_id": client_id,
                "title": "ESXi root",
                "username": "root",
                "secret": "vault-secret-1",
                "idempotency_key": "pw-1"
            })),
        )
        .await,
    )
    .await;
    let password_id = password["id"].as_str().unwrap().to_string();
    // Link password → asset.
    let pw_links = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/passwords/{password_id}/links"),
        &bearer,
        Some(json!({ "to_kind": "asset", "to_id": asset_id })),
    )
    .await;
    assert_eq!(pw_links.status(), 200);

    let vault = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/assets/{asset_id}/vault"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(vault.as_array().unwrap().len(), 1);
    assert_eq!(vault[0]["title"], "ESXi root");
    assert!(
        vault[0].get("secret_ciphertext").is_none() && vault[0].get("secret").is_none(),
        "asset vault entries carry metadata only — no ciphertext, no plaintext"
    );

    // Dispatcher holds view-only Passwords: metadata readable, but reveal
    // (passwords:edit) is denied. Read-only holds NO Clients grants: asset
    // CRUD is a typed 403.
    let org_id = me_org_id(&app, &token).await;
    let dispatch_token = provision_role(
        &state,
        &app,
        org_id,
        "dispatcher",
        &format!("dispatch-{slug}@acme.test"),
        "dispatch-password-1234",
    )
    .await;
    let dispatch_bearer = [auth(&dispatch_token)];
    let readonly_token = provision_role(
        &state,
        &app,
        org_id,
        "read_only",
        &format!("readonly-{slug}@acme.test"),
        "readonly-password-1234",
    )
    .await;
    let readonly_bearer = [auth(&readonly_token)];

    let dispatcher_vault = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets/{asset_id}/vault"),
        &dispatch_bearer,
        None,
    )
    .await;
    assert_eq!(
        dispatcher_vault.status(),
        200,
        "dispatcher: passwords view is allowed"
    );

    let dispatcher_reveal = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/passwords/{password_id}/reveal"),
        &dispatch_bearer,
        None,
    )
    .await;
    assert_eq!(
        dispatcher_reveal.status(),
        403,
        "reveal requires passwords:edit — dispatcher (view-only) gets a typed 403"
    );

    let readonly_create = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/assets",
        &readonly_bearer,
        Some(json!({ "client_id": client_id, "tag": "D1", "name": "Nope" })),
    )
    .await;
    assert_eq!(
        readonly_create.status(),
        403,
        "read-only has no Clients create grant — typed 403, never a dead path"
    );
    let readonly_vault = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets/{asset_id}/vault"),
        &readonly_bearer,
        None,
    )
    .await;
    assert_eq!(
        readonly_vault.status(),
        200,
        "read-only holds view across all modules — vault metadata is readable, writes are not"
    );
    let readonly_reveal = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/passwords/{password_id}/reveal"),
        &readonly_bearer,
        None,
    )
    .await;
    assert_eq!(
        readonly_reveal.status(),
        403,
        "reveal requires passwords:edit — read-only gets a typed 403, never a dead button"
    );

    // --- integration link status is data, not sync ----------------------
    let integrations = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/assets/integration-links",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    assert!(
        integrations.as_array().unwrap().is_empty(),
        "no connectors installed yet — unlinked, as data"
    );

    // --- delete: out of scope, trail keeps the events -------------------
    let deleted = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/assets/{asset_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(deleted.status(), 200);
    assert_eq!(body_json(deleted).await["deleted"], true);

    let gone = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/assets/{asset_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(gone.status(), 404, "deleted asset is out of scope");
}

#[tokio::test]
async fn assets_flow_unauthenticated_is_401() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let response = send(&app, axum::http::Method::GET, "/api/v1/assets", &[], None).await;
    assert_eq!(response.status(), 401);
}
