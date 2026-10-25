//! Document endpoint flows (the private build tracker D4, agent-platform §6) through the
//! REAL core-api router.
//!
//! Purpose: the /api/documents contract over HTTP —
//! 1. Owner round-trip: create (version 1) → edit (version 2) → inspect
//!    history (newest-first, full bodies) → restore version 1 (creates
//!    version 3, `restored_from_version` set, content copied exactly).
//! 2. Append-only: after every write, earlier version rows are unchanged
//!    and reachable; the head pointer moved, nothing else.
//! 3. Instant-first: idempotent replay, typed `idempotency_key_reused` /
//!    `version_conflict` reasons, read-your-write `mutation` metadata.
//! 4. Authz: technicians (no documents grants) are rejected (403);
//!    unauthenticated requests (401). Read-only users still read.
//! 5. Dual-mount parity: /api/v1 (canonical) and /api (legacy alias).
//!
//! Non-purpose: query-layer semantics (core-db/tests/document_queries.rs),
//! restore/diff policy units (core-documents crate tests).
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("documents flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([22; 32]);
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
            "org_name": "Docs Co",
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

/// Creates a second user through core_db with the builtin technician role
/// (technicians hold NO documents grants) and logs in over HTTP.
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Tech User",
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
        Some(json!({ "email": email, "password": "tech-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login must succeed");
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

/// Creates a custom role holding exactly `permissions` (the `Permission`
/// enum's serde shape), puts a user on it, and logs that user in over HTTP.
async fn provision_role_user(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    role_name: &str,
    permissions: Value,
    email: &str,
    password: &str,
) -> String {
    let password_hash = core_api::auth::password::hash(password).expect("hash");
    let role_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO roles (id, org_id, name, builtin, permissions, is_system) \
         VALUES ($1, $2, $3, NULL, $4, false)",
    )
    .bind(role_id)
    .bind(org_id)
    .bind(role_name)
    .bind(&permissions)
    .execute(&state.pool)
    .await
    .expect("custom role insert");

    let user_id = Uuid::now_v7();
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Docs Viewer",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role_id)
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
    assert_eq!(response.status(), 200, "custom-role login must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn documents_flow_append_only_history_and_restore_creates_new_version() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("docs-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- create (version 1) with idempotent replay ----------------------
    let create_body = json!({
        "title": "Escalation policy",
        "body": "P1 pages on-call.",
        "idempotency_key": "doc-create-1"
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/documents",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200, "document create must succeed");
    let applied = body_json(first).await;
    let doc_id = applied["id"].as_str().unwrap().to_string();
    assert_eq!(applied["current_version"], 1);
    assert_eq!(applied["mutation"]["seq"], 1);

    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/documents",
        &bearer,
        Some(create_body),
    )
    .await;
    assert_eq!(replay.status(), 200);
    assert_eq!(
        body_json(replay).await["id"],
        applied["id"],
        "same key + same payload replays the SAME document"
    );

    // --- edit appends version 2 (read-your-write metadata) --------------
    let edit = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/documents/{doc_id}"),
        &bearer,
        Some(json!({
            "body": "P1 pages on-call. P2 alerts in-app.",
            "change_summary": "added P2 path",
            "version": 1
        })),
    )
    .await;
    assert_eq!(edit.status(), 200);
    let edited = body_json(edit).await;
    assert_eq!(edited["current_version"], 2, "edit APPENDS a version");
    assert_eq!(edited["mutation"]["version"], 2);
    assert_eq!(edited["mutation"]["before"]["current_version"], 1);

    // Stale version rejected with the typed conflict.
    let stale = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/documents/{doc_id}"),
        &bearer,
        Some(json!({ "body": "stale", "version": 1 })),
    )
    .await;
    assert_eq!(stale.status(), 409);
    assert_eq!(
        body_json(stale).await["error"]["reason"],
        "version_conflict"
    );

    // Same key + different payload on a mutation: typed reuse conflict.
    let reused = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/documents/{doc_id}"),
        &bearer,
        Some(json!({ "body": "different", "version": 2, "idempotency_key": "doc-create-1" })),
    )
    .await;
    assert_eq!(reused.status(), 409);
    assert_eq!(
        body_json(reused).await["error"]["reason"],
        "idempotency_key_reused"
    );

    // --- GET one carries the CURRENT content ----------------------------
    let view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(view.status(), 200);
    let doc_view = body_json(view).await;
    assert_eq!(doc_view["body"], "P1 pages on-call. P2 alerts in-app.");
    assert_eq!(doc_view["change_summary"], "added P2 path");
    // Legacy alias parity.
    let alias = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/documents/{doc_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(alias.status(), 200, "legacy alias serves documents");
    assert_eq!(body_json(alias).await["body"], doc_view["body"]);

    // --- history: newest-first, full bodies, append-only -----------------
    let history = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/versions"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(history.status(), 200);
    let versions = body_json(history).await;
    let arr = versions.as_array().unwrap();
    assert_eq!(
        arr.iter()
            .map(|v| v["version"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 1],
        "newest first"
    );
    assert_eq!(arr[0]["body"], "P1 pages on-call. P2 alerts in-app.");
    assert_eq!(
        arr[1]["body"], "P1 pages on-call.",
        "old versions never mutate"
    );
    assert_eq!(arr[1]["restored_from_version"], Value::Null);

    // Single version fetch.
    let v1 = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/versions/1"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(v1.status(), 200);
    assert_eq!(body_json(v1).await["body"], "P1 pages on-call.");

    // Missing version: typed 404, not an empty shell.
    let missing = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/versions/99"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(missing.status(), 404);
    assert_eq!(body_json(missing).await["error"]["reason"], "not_found");

    // --- restore version 1 → creates version 3 ---------------------------
    let restore = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/documents/{doc_id}/restore"),
        &bearer,
        Some(json!({ "version": 1 })),
    )
    .await;
    assert_eq!(restore.status(), 200, "restore must succeed");
    let restored = body_json(restore).await;
    assert_eq!(
        restored["current_version"], 3,
        "restore creates a NEW version"
    );

    let head = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}"),
        &bearer,
        None,
    )
    .await;
    let head_view = body_json(head).await;
    assert_eq!(
        head_view["body"], "P1 pages on-call.",
        "restore copies target content"
    );
    assert_eq!(head_view["restored_from_version"], 1);

    let history = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/versions"),
        &bearer,
        None,
    )
    .await;
    let versions = body_json(history).await;
    let arr = versions.as_array().unwrap();
    assert_eq!(
        arr.len(),
        3,
        "append-only: nothing was removed by the restore"
    );
    assert_eq!(arr[0]["restored_from_version"], 1);
    assert_eq!(arr[0]["change_summary"], "Restored from version 1");
}

#[tokio::test]
async fn documents_authz_rejects_technician_and_anonymous() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("docs-authz-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = me_org_id(&app, &token).await;
    let tech = provision_technician(&state, &app, org_id, &format!("tech-{slug}@docs.test")).await;

    // Technicians hold no documents grants: writes AND reads are denied
    // with the typed forbidden reason.
    let denied_write = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/documents",
        &[auth(&tech)],
        Some(json!({ "title": "Nope" })),
    )
    .await;
    assert_eq!(denied_write.status(), 403);
    assert_eq!(
        body_json(denied_write).await["error"]["reason"],
        "forbidden"
    );

    let denied_read = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/documents",
        &[auth(&tech)],
        None,
    )
    .await;
    assert_eq!(denied_read.status(), 403);

    let anonymous = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/documents",
        &[],
        None,
    )
    .await;
    assert_eq!(anonymous.status(), 401);
    assert_eq!(
        body_json(anonymous).await["error"]["reason"],
        "unauthorized"
    );
}

/// Creates one document over HTTP and returns its id (the panel's owner
/// needs two: a hub and a link target).
async fn create_doc(
    app: &Router,
    bearer: &[(&'static str, String)],
    title: &str,
    key: &str,
) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/documents",
        bearer,
        Some(json!({ "title": title, "idempotency_key": key })),
    )
    .await;
    assert_eq!(response.status(), 200, "document create");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Wave 55 decision 3: a resolved link label is a read of the target. The
/// panel is gated by `documents:view`, but each label inside it must pass
/// the target module's own view gate — so a documents-only caller gets the
/// linked document's title and a typed restricted marker where the vault
/// entry and the ticket title would have been.
#[tokio::test]
async fn link_labels_are_redacted_for_targets_outside_the_callers_gates() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("docs-links-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = me_org_id(&app, &token).await;
    let bearer = [auth(&token)];

    // Owner builds the panel: a document linked to another document, a
    // vault entry, and a ticket.
    let doc_id = create_doc(&app, &bearer, "Incident hub", "hub-1").await;
    let target_doc_id = create_doc(&app, &bearer, "Escalation policy", "policy-1").await;

    let password = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/passwords",
        &bearer,
        Some(json!({
            "title": "Acme domain admin",
            "username": "admin",
            "secret": "vault-secret-1",
            "idempotency_key": "pw-links-1"
        })),
    )
    .await;
    assert_eq!(password.status(), 200, "password create");
    let password_id = body_json(password).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    let boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/v1/boards",
            &bearer,
            None,
        )
        .await,
    )
    .await;
    let board_id = boards
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["is_default"] == json!(true))
        .expect("registration provisions a default board")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let ticket = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/tickets",
        &bearer,
        Some(json!({
            "board_id": board_id,
            "title": "Printer down",
            "priority": "p3"
        })),
    )
    .await;
    assert_eq!(ticket.status(), 200, "ticket create");
    let ticket_id = body_json(ticket).await["id"].as_str().unwrap().to_string();

    for (kind, id) in [
        ("document", target_doc_id.as_str()),
        ("password", password_id.as_str()),
        ("ticket", ticket_id.as_str()),
    ] {
        let linked = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/documents/{doc_id}/links"),
            &bearer,
            Some(json!({ "to_kind": kind, "to_id": id })),
        )
        .await;
        assert_eq!(linked.status(), 200, "link {kind}");
    }

    // A caller holding documents:view and nothing else.
    let viewer = provision_role_user(
        &state,
        &app,
        org_id,
        "Docs Viewer",
        json!([{ "matrix": { "module": "documents", "action": "view" } }]),
        &format!("viewer-{slug}@docs.test"),
        "viewer-password-1234",
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/documents/{doc_id}/links"),
        &[auth(&viewer)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "documents:view opens the panel");
    let rows = body_json(response).await;
    let rows = rows.as_array().expect("link array");
    assert_eq!(rows.len(), 3, "every link row survives the gate");

    let row = |kind: &str| {
        rows.iter()
            .find(|l| l["to_kind"] == json!(kind))
            .unwrap_or_else(|| panic!("row for {kind}"))
    };
    assert_eq!(
        row("document")["target_label"],
        json!("Escalation policy"),
        "a document target answers to documents:view"
    );
    assert_eq!(row("document")["target_denied"], json!(false));
    for kind in ["password", "ticket"] {
        assert_eq!(
            row(kind)["target_label"],
            Value::Null,
            "{kind} title must not cross a module gate the caller lacks"
        );
        assert_eq!(
            row(kind)["target_denied"],
            json!(true),
            "{kind} is restricted"
        );
        assert_eq!(
            row(kind)["target_deleted"],
            json!(false),
            "{kind} is not gone, it is not theirs"
        );
    }
    let raw = serde_json::to_string(rows).unwrap();
    assert!(
        !raw.contains("Acme domain admin") && !raw.contains("Printer down"),
        "no hidden title may survive anywhere in the body"
    );

    // The owner holds both gates: the same rows resolve to labels.
    let owner_rows = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/documents/{doc_id}/links"),
            &bearer,
            None,
        )
        .await,
    )
    .await;
    let owner_rows = owner_rows.as_array().unwrap();
    let owner_row = |kind: &str| {
        owner_rows
            .iter()
            .find(|l| l["to_kind"] == json!(kind))
            .unwrap_or_else(|| panic!("owner row for {kind}"))
    };
    assert_eq!(
        owner_row("document")["target_label"],
        json!("Escalation policy")
    );
    let password_label = owner_row("password");
    assert_eq!(password_label["target_denied"], json!(false));
    assert_eq!(password_label["target_label"], json!("Acme domain admin"));
    let ticket_label = owner_row("ticket");
    assert_eq!(ticket_label["target_denied"], json!(false));
    assert!(
        ticket_label["target_label"]
            .as_str()
            .unwrap_or_default()
            .contains("Printer down"),
        "the ticket label (number + title) resolves for a caller holding tickets:view"
    );
}

/// Creates a client and one asset under the seeded `server` layout; returns
/// `(client_id, asset_id)` — the asset panel's mount gate and its link
/// targets each need one.
async fn create_client_and_asset(
    app: &Router,
    bearer: &[(&'static str, String)],
    key: &str,
) -> (String, String) {
    let client = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/v1/clients",
            bearer,
            Some(json!({ "name": "Acme Systems" })),
        )
        .await,
    )
    .await;
    let client_id = client["id"].as_str().unwrap().to_string();
    let layouts = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/v1/documents/layouts",
            bearer,
            None,
        )
        .await,
    )
    .await;
    let layout_id = layouts
        .as_array()
        .expect("layouts array")
        .iter()
        .find(|l| l["slug"] == "server")
        .expect("seeded server layout")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let created = send(
        app,
        axum::http::Method::POST,
        "/api/v1/assets",
        bearer,
        Some(json!({
            "client_id": client_id,
            "tag": "EXCH-01",
            "name": "Exchange server",
            "layout_id": layout_id,
            "fields": { "hostname": "exch01.acme.test" },
            "idempotency_key": format!("asset-{key}")
        })),
    )
    .await;
    assert_eq!(created.status(), 200, "asset create");
    (
        client_id,
        body_json(created).await["id"].as_str().unwrap().to_string(),
    )
}

/// F52 (Wave 56): a link mount asks the PANEL's own module. A caller holding
/// exactly `passwords:view` reads the password panel, a caller holding exactly
/// `clients:view` reads the asset panel, and both are refused the documents
/// panel they hold no grant for.
#[tokio::test]
async fn link_mounts_ask_their_own_module_not_documents() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("panel-gates-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = me_org_id(&app, &token).await;
    let bearer = [auth(&token)];

    let doc_id = create_doc(&app, &bearer, "Incident hub", "panel-hub").await;
    let password = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/passwords",
        &bearer,
        Some(json!({
            "title": "Acme domain admin",
            "username": "admin",
            "secret": "vault-secret-1",
            "idempotency_key": "panel-pw-1"
        })),
    )
    .await;
    assert_eq!(password.status(), 200, "password create");
    let password_id = body_json(password).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_client_id, asset_id) = create_client_and_asset(&app, &bearer, "panel").await;

    // Exactly passwords:view — nothing else.
    let vault_viewer = provision_role_user(
        &state,
        &app,
        org_id,
        "Vault Viewer",
        json!([{ "matrix": { "module": "passwords", "action": "view" } }]),
        &format!("vault-viewer-{slug}@docs.test"),
        "vault-viewer-1234",
    )
    .await;
    let vault_bearer = [auth(&vault_viewer)];
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/passwords/{password_id}/links"),
            &vault_bearer,
            None,
        )
        .await
        .status(),
        200,
        "passwords:view opens the password panel"
    );
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/documents/{doc_id}/links"),
            &vault_bearer,
            None,
        )
        .await
        .status(),
        403,
        "the documents panel stays closed to a caller without documents:view"
    );

    // Exactly clients:view — assets ride the Clients matrix.
    let asset_viewer = provision_role_user(
        &state,
        &app,
        org_id,
        "Asset Viewer",
        json!([{ "matrix": { "module": "clients", "action": "view" } }]),
        &format!("asset-viewer-{slug}@docs.test"),
        "asset-viewer-1234",
    )
    .await;
    let asset_bearer = [auth(&asset_viewer)];
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/assets/{asset_id}/links"),
            &asset_bearer,
            None,
        )
        .await
        .status(),
        200,
        "clients:view opens the asset panel"
    );
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/documents/{doc_id}/links"),
            &asset_bearer,
            None,
        )
        .await
        .status(),
        403,
        "the documents panel stays closed to a caller without documents:view"
    );
}

/// F52 (Wave 56): the add-link response and the mount it refreshes agree.
/// An editor holding `passwords:edit`/`clients:edit` and no documents grant
/// gets the refreshed list back from both POSTs instead of a 403 raised by
/// the shared reader.
#[tokio::test]
async fn add_link_response_rides_the_mounts_own_gate() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("panel-add-link-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = me_org_id(&app, &token).await;
    let bearer = [auth(&token)];

    let doc_id = create_doc(&app, &bearer, "Incident hub", "add-hub").await;
    let password = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/passwords",
        &bearer,
        Some(json!({
            "title": "Acme domain admin",
            "username": "admin",
            "secret": "vault-secret-1",
            "idempotency_key": "add-pw-1"
        })),
    )
    .await;
    assert_eq!(password.status(), 200, "password create");
    let password_id = body_json(password).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (client_id, asset_id) = create_client_and_asset(&app, &bearer, "add").await;

    let editor = provision_role_user(
        &state,
        &app,
        org_id,
        "Vault and Asset Editor",
        json!([
            { "matrix": { "module": "passwords", "action": "view" } },
            { "matrix": { "module": "passwords", "action": "edit" } },
            { "matrix": { "module": "clients", "action": "view" } },
            { "matrix": { "module": "clients", "action": "edit" } }
        ]),
        &format!("vault-editor-{slug}@docs.test"),
        "vault-editor-1234",
    )
    .await;
    let editor_bearer = [auth(&editor)];

    let password_post = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/passwords/{password_id}/links"),
        &editor_bearer,
        Some(json!({ "to_kind": "client", "to_id": client_id })),
    )
    .await;
    assert_eq!(
        password_post.status(),
        200,
        "the add-link response rides passwords:view, the same gate as its mount"
    );
    let password_rows = body_json(password_post).await;
    let password_rows = password_rows.as_array().expect("link array");
    assert_eq!(
        password_rows.len(),
        1,
        "the refreshed panel carries the link"
    );
    assert_eq!(password_rows[0]["target_label"], json!("Acme Systems"));
    assert_eq!(password_rows[0]["target_denied"], json!(false));

    let asset_post = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/assets/{asset_id}/links"),
        &editor_bearer,
        Some(json!({ "to_kind": "client", "to_id": client_id })),
    )
    .await;
    assert_eq!(
        asset_post.status(),
        200,
        "the add-link response rides clients:view, the same gate as its mount"
    );
    let asset_rows = body_json(asset_post).await;
    let asset_rows = asset_rows.as_array().expect("link array");
    assert_eq!(asset_rows.len(), 1, "the refreshed panel carries the link");
    assert_eq!(asset_rows[0]["target_label"], json!("Acme Systems"));

    // The shared reader never widens the mount it serves.
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/documents/{doc_id}/links"),
            &editor_bearer,
            None,
        )
        .await
        .status(),
        403,
        "no documents grant, no documents panel"
    );
}
