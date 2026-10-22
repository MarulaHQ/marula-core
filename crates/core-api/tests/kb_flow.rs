//! Knowledge-base endpoint flows (the private build tracker Phase 3 native KB v1) through
//! the REAL core-api router.
//!
//! Purpose: the /api/kb contract over HTTP —
//! 1. Owner round-trip: category + article CRUD, the client-facing filter,
//!    slugs derived server-side.
//! 2. Instant-first contract: same idempotency key + same payload replays
//!    the SAME result; same key + different payload is a typed 409
//!    `idempotency_key_reused`; a stale `version` is a typed 409
//!    `version_conflict`; archived articles refuse content edits.
//! 3. Unified-search integration: article content indexed via the internal
//!    effect surface is returned by GET /api/search, filtered by kind
//!    permissions (technician sees `kb_article`, not `document`).
//! 4. Authz: read-only users are rejected (403); unauthenticated (401).
//! 5. Dual-mount parity: /api/v1 (canonical) and /api (legacy alias).
//!
//! Non-purpose: query-layer semantics (core-db/tests/kb_queries.rs), KB
//! domain rules (core-kb unit tests), indexer resolution (worker tests).
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
        .expect("kb flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([21; 32]);
    // Enable the internal effect surface so the search-index seeding step
    // exercises the exact indexer write path (2.16 pattern).
    config.internal_token = Some("kb-flow-internal-token".to_string());
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

fn internal() -> (&'static str, String) {
    ("x-internal-token", "kb-flow-internal-token".to_string())
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "KB Co",
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

/// Creates a second user with the builtin read_only role (no kb:create)
/// and logs in over HTTP so the 403 is asserted against a REAL principal.
async fn provision_readonly(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("ro-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'read_only'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("read_only role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "RO User",
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
        Some(json!({ "email": email, "password": "ro-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "readonly login must succeed");
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
async fn kb_owner_flow_idempotency_version_gate_and_filters() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("kb-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // --- category create + dual-mount parity ---------------------------
    let category = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/kb/categories",
            &bearer,
            Some(json!({ "name": "Client How-Tos", "position": 3, "client_visible": true })),
        )
        .await;
        assert_eq!(response.status(), 200, "category create must succeed");
        let body = body_json(response).await;
        assert_eq!(body["slug"], "client-how-tos", "slug derived server-side");
        assert_eq!(body["mutation"]["seq"], 1, "IF2 seq starts at one");
        body
    };
    let category_id = category["id"].as_str().unwrap().to_string();

    // The legacy alias serves the identical handler.
    let alias = send(
        &app,
        axum::http::Method::GET,
        "/api/kb/categories",
        &bearer,
        None,
    )
    .await;
    assert_eq!(alias.status(), 200, "legacy alias must serve KB reads");
    assert_eq!(body_json(alias).await.as_array().unwrap().len(), 1);

    // --- article create with idempotent replay -------------------------
    let create_body = json!({
        "title": "Rotate the Wifi password",
        "body": "# Quarterly\nUse the vault.",
        "category_id": category_id,
        "status": "published",
        "client_visible": true,
        "idempotency_key": "kb-create-1"
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200);
    let applied = body_json(first).await;
    let article_id = applied["id"].as_str().unwrap().to_string();
    assert_eq!(applied["slug"], "rotate-the-wifi-password");
    assert_eq!(applied["mutation"]["version"], 1);

    // Same key + different payload: typed 409 idempotency_key_reused.
    let mut conflicting = create_body.clone();
    conflicting["title"] = json!("Different title");
    conflicting["idempotency_key"] = json!("kb-create-1");
    let reused = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &bearer,
        Some(conflicting),
    )
    .await;
    assert_eq!(reused.status(), 409);
    assert_eq!(
        body_json(reused).await["error"]["reason"],
        "idempotency_key_reused",
        "IF3: typed reason, never prose to parse"
    );

    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &bearer,
        Some(create_body),
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replayed = body_json(replay).await;
    assert_eq!(
        replayed["id"], applied["id"],
        "same key + same payload must replay the SAME article"
    );
    // --- version gate ---------------------------------------------------
    // Stale version: the client presents 0 on an entity at 1.
    let stale = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{article_id}"),
        &bearer,
        Some(json!({ "body": "edited", "version": 0 })),
    )
    .await;
    assert_eq!(stale.status(), 409);
    let conflict = body_json(stale).await;
    assert_eq!(conflict["error"]["reason"], "version_conflict");
    assert_eq!(
        conflict["error"]["expected"], 1,
        "carries the current version"
    );

    // Matching version applies and hands back the next version token.
    let applied_edit = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{article_id}"),
        &bearer,
        Some(json!({ "body": "# Quarterly\nUse the vault. v2", "version": 1 })),
    )
    .await;
    assert_eq!(applied_edit.status(), 200);
    let edited = body_json(applied_edit).await;
    assert_eq!(edited["mutation"]["version"], 2, "read-your-write version");
    assert_eq!(
        edited["mutation"]["before"]["body"],
        "# Quarterly\nUse the vault."
    );
    assert_eq!(
        edited["mutation"]["after"]["body"],
        "# Quarterly\nUse the vault. v2"
    );

    // --- archived content lock ------------------------------------------
    let archive = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{article_id}"),
        &bearer,
        Some(json!({ "status": "archived", "version": 2 })),
    )
    .await;
    assert_eq!(
        archive.status(),
        200,
        "status flips stay legal while active"
    );

    let content_edit = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{article_id}"),
        &bearer,
        Some(json!({ "body": "nope", "version": 3 })),
    )
    .await;
    assert_eq!(content_edit.status(), 409, "archived articles are locked");
    assert_eq!(body_json(content_edit).await["error"]["reason"], "conflict");

    // --- client-facing filter -------------------------------------------
    // Unarchive first (status flips stay legal; the lock only covers
    // title/body), so the article passes the published leg of the gate.
    let unarchive = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/kb/articles/{article_id}"),
        &bearer,
        Some(json!({ "status": "published", "version": 3 })),
    )
    .await;
    assert_eq!(
        unarchive.status(),
        200,
        "archived articles can be unarchived"
    );

    // A fully gated article (published + flags on both levels) exists; add
    // a flagged DRAFT that must NOT pass the gate.
    send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &bearer,
        Some(json!({
            "title": "Draft announcement",
            "status": "draft",
            "client_visible": true,
            "category_id": category_id
        })),
    )
    .await;
    let filtered = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/kb/articles?client_visible=true",
        &bearer,
        None,
    )
    .await;
    assert_eq!(filtered.status(), 200);
    let visible = body_json(filtered).await;
    let arr = visible.as_array().unwrap();
    assert_eq!(arr.len(), 1, "drafts never pass the client gate");
    assert_eq!(arr[0]["id"], applied["id"]);

    // --- search integration over the internal effect surface ------------
    let org_id = me_org_id(&app, &token).await;
    let index = send(
        &app,
        axum::http::Method::POST,
        "/internal/search/upsert",
        &[internal()],
        Some(json!({
            "org_id": org_id,
            "entries": [{
                "entity_kind": "kb_article",
                "entity_id": article_id,
                "parent_id": category_id,
                "title": "Rotate the Wifi password",
                "body": "# Quarterly\nUse the vault. v2"
            }]
        })),
    )
    .await;
    assert_eq!(
        index.status(),
        200,
        "indexer write path must accept kb_article"
    );

    let hit = send(
        &app,
        axum::http::Method::GET,
        "/api/search?q=wifi%20password",
        &bearer,
        None,
    )
    .await;
    assert_eq!(hit.status(), 200);
    let hits = body_json(hit).await;
    assert!(hits["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| { r["entity_kind"] == "kb_article" && r["entity_id"] == applied["id"] }));
}

#[tokio::test]
async fn kb_authz_rejects_readonly_and_anonymous() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("kb-authz-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org_id = me_org_id(&app, &token).await;

    // A real read_only principal: kb:view passes reads, kb:create is
    // denied on writes.
    let ro_token = provision_readonly(&state, &app, org_id, &format!("ro-{}", slug)).await;
    let readable = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/kb/articles",
        &[auth(&ro_token)],
        None,
    )
    .await;
    assert_eq!(readable.status(), 200, "read_only holds kb:view");

    let forbidden = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/kb/articles",
        &[auth(&ro_token)],
        Some(json!({ "title": "Not allowed" })),
    )
    .await;
    assert_eq!(forbidden.status(), 403, "read_only lacks kb:create");
    assert_eq!(body_json(forbidden).await["error"]["reason"], "forbidden");

    // Unauthenticated requests are rejected with the typed envelope.
    let anonymous = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/kb/articles",
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
