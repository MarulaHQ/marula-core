//! Migration connector conformance suite (the private build tracker D3, Wave 9 lane 6).
//!
//! Purpose: exercise the FULL contract a third-party migration tool sees —
//! not the implementation's internals. The suite plays the role of an
//! external tool over plain HTTP against the real core-api router:
//!
//! 1. key lifecycle gates — an admin key with `migration:write`/`migration:read`
//!    scopes; unknown scopes are refused at creation; scopes without the
//!    migration grant get 403s; the canonical `/api/v1` mount accepts the
//!    same key identically (two-mount parity).
//! 2. push session contract — 201 session creation with a starting
//!    checkpoint, sequential chunk commits, cursor-mismatch 409s, exact
//!    re-send replay (no duplicate rows), content drift under a committed
//!    index (`migration_chunk_reused`), resume-after-crash via the GET
//!    session view, finish closing BOTH lifecycles, and double-finish 409.
//! 3. conflict-policy 409s — exactly the D2 `import_chunk_rejected`
//!    envelope on a fail-policy conflict, checkpoint untouched, chunk
//!    retryable after correction.
//! 4. data idempotency — stable external ids make a re-push across a NEW
//!    session update rather than duplicate.
//! 5. pull — every family exports id + external_key + canonical
//!    projection, cursor-paged with no skips/repeats, keyset cursor walks
//!    pages until exhausted.
//! 6. rate limiting — per-key fixed window, 429 `rate_limited` with
//!    `retry_after_secs` body and `Retry-After` header.
//!
//! Non-purpose: pure engine laws (core-domain), RLS probes (core-db), and
//! the D2 HTTP surface itself (imports_flow) are covered elsewhere.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

/// One-shot schema reset per process; pools are built per call afterwards.
static BOOTSTRAP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn connect_admin(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await
        .expect("admin pool")
}

/// Fresh schema + migrations + router. `rate_limit_per_min` overrides the
/// per-principal migration rate limit for the returned state (the 429
/// probe uses a tiny budget; every other test keeps the 600 default).
async fn test_state(rate_limit_per_min: u32) -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    BOOTSTRAP
        .get_or_init(|| async {
            let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect("connector e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
            let boot = connect_admin(&admin_url).await;
            sqlx::query("DROP SCHEMA public CASCADE")
                .execute(&boot)
                .await
                .expect("drop schema");
            sqlx::query("CREATE SCHEMA public")
                .execute(&boot)
                .await
                .expect("create schema");
            core_db::run_migrations(&boot).await.expect("migrations");
            core_db::assert_rls_coverage(&boot)
                .await
                .expect("RLS coverage holds with migration connector surfaces");
        })
        .await;
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("connector e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await;
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url;
    config.migration_rate_limit_per_min = rate_limit_per_min;
    let state = AppState {
        pool,
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
            .expect("reporting pool"),
        config: std::sync::Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: std::sync::Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn send(
    app: &Router,
    method: axum::http::Method,
    uri: &str,
    bearer: Option<&str>,
    api_key: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = api_key {
        builder = builder.header("x-api-key", key);
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines identify the endpoint")
    })
}

async fn event_count(pool: &PgPool, org_id: Uuid, event_type: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE org_id = $1 AND event_type = $2")
        .bind(org_id)
        .bind(event_type)
        .fetch_one(pool)
        .await
        .expect("outbox count")
}

async fn audit_count(pool: &PgPool, org_id: Uuid, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE org_id = $1 AND action = $2")
        .bind(org_id)
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("audit count")
}

async fn table_count(pool: &PgPool, org_id: Uuid, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE org_id = $1");
    sqlx::query_scalar(&sql)
        .bind(org_id)
        .fetch_one(pool)
        .await
        .expect("table count")
}

struct Org {
    token: String,
    org_id: Uuid,
}

async fn register_org(app: &Router, slug: &str) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        None,
        Some(json!({
            "org_name": "Migration Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert!(status.is_success(), "register failed: {status} {body}");
    let token = body["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    let me = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/me",
            Some(&token),
            None,
            None,
        )
        .await,
    )
    .await;
    Org {
        token,
        org_id: Uuid::parse_str(me["org_id"].as_str().expect("org_id")).expect("uuid"),
    }
}

/// Creates an org API key and returns its raw material.
async fn create_key(app: &Router, org: &Org, name: &str, scopes: &[&str]) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(&org.token),
        None,
        Some(json!({ "name": name, "scopes": scopes })),
    )
    .await;
    let status = response.status();
    let body = body_json(response).await;
    assert!(status.is_success(), "create key failed: {status} {body}");
    body["raw_key"].as_str().expect("raw key").to_string()
}

fn json_rows(rows: Value) -> Value {
    json!({ "format": "json", "json_rows": rows })
}

async fn open_session(
    app: &Router,
    key: &str,
    entity_kind: &str,
    conflict_policy: &str,
) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/migration/push/sessions",
        None,
        Some(key),
        Some(json!({
            "entity_kind": entity_kind,
            "conflict_policy": conflict_policy,
            "label": "conformance",
        })),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

async fn push_chunk(
    app: &Router,
    key: &str,
    session: Uuid,
    index: i64,
    rows: Value,
) -> (axum::http::StatusCode, Value) {
    let mut body = rows;
    body["chunk_index"] = json!(index);
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/migration/push/sessions/{session}/chunks"),
        None,
        Some(key),
        Some(body),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

async fn finish(app: &Router, key: &str, session: Uuid) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/migration/push/sessions/{session}/finish"),
        None,
        Some(key),
        None,
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

fn reason_of(body: &Value) -> &str {
    body["error"]["reason"].as_str().unwrap_or_default()
}

// --- T1: key + scope gating ------------------------------------------------------

#[tokio::test]
async fn migration_scopes_gate_the_connector_surface() {
    let (state, app) = test_state(600).await;
    let org = register_org(&app, "d3-gating").await;
    let _ = state;

    // Unknown scopes are refused at key creation.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(&org.token),
        None,
        Some(json!({ "name": "bad", "scopes": ["migration:admin"] })),
    )
    .await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );

    // A key without any migration scope cannot touch the surface.
    let plain_key = create_key(&app, &org, "plain", &["tickets:read"]).await;
    let (status, body) = open_session(&app, &plain_key, "client", "update").await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body:?}");
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/migration/pull/client",
        None,
        Some(&plain_key),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

    // read-only scope pulls but never pushes.
    let read_key = create_key(&app, &org, "reader", &["migration:read"]).await;
    let (status, body) = open_session(&app, &read_key, "client", "update").await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body:?}");
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/migration/pull/client",
        None,
        Some(&read_key),
        None,
    )
    .await;
    assert!(response.status().is_success(), "read scope pulls");

    // write scope pushes and pulls.
    let write_key = create_key(&app, &org, "writer", &["migration:write"]).await;
    let (status, body) = open_session(&app, &write_key, "client", "update").await;
    assert!(status.is_success(), "{body:?}");

    // The same key works identically on the legacy alias mount.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/migration/pull/client",
        None,
        Some(&write_key),
        None,
    )
    .await;
    assert!(
        response.status().is_success(),
        "legacy mount accepts the key"
    );

    // JWT humans stay owner/admin gated (the owner is allowed).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/migration/push/sessions",
        Some(&org.token),
        None,
        Some(json!({ "entity_kind": "client", "conflict_policy": "skip" })),
    )
    .await;
    assert!(response.status().is_success(), "org owner may push");
}

// --- T2: push lifecycle + checkpoint resume --------------------------------------

#[tokio::test]
async fn push_lifecycle_checkpoints_and_replays() {
    let (state, app) = test_state(600).await;
    let org = register_org(&app, "d3-push").await;
    let key = create_key(&app, &org, "pusher", &["migration:write"]).await;

    // Session open: 201, starting cursor 0.
    let (status, session) = open_session(&app, &key, "client", "update").await;
    assert_eq!(status, axum::http::StatusCode::CREATED, "{session:?}");
    let session_id: uuid::Uuid = session["session_id"].as_str().unwrap().parse().unwrap();
    let batch_id: uuid::Uuid = session["batch_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(session["checkpoint"]["next_chunk"], json!(0));
    assert_eq!(session["checkpoint"]["last_committed_chunk"], json!(-1));

    // Chunk 0: two clients commit.
    let chunk0 = json_rows(json!([
        { "external_key": "cli-a", "name": "Alpha Co" },
        { "external_key": "cli-b", "name": "Beta LLC" },
    ]));
    let (status, report) = push_chunk(&app, &key, session_id, 0, chunk0.clone()).await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["replay"], json!(false));
    assert_eq!(report["report"]["summary"]["ok"], json!(2));
    assert_eq!(report["report"]["batch_id"], json!(batch_id.to_string()));
    assert_eq!(report["checkpoint"]["next_chunk"], json!(1));
    assert_eq!(report["rows_created"], json!(2));
    assert_eq!(report["checkpoint"]["last_committed_chunk"], json!(0));

    // Chunk 2 out of position: typed cursor mismatch naming the expectation.
    let (status, body) = push_chunk(
        &app,
        &key,
        session_id,
        2,
        json_rows(json!([{ "external_key": "cli-z", "name": "Jump" }])),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    assert_eq!(body["error"]["reason"], json!("migration_cursor_mismatch"));
    assert_eq!(body["error"]["expected"], json!(1));
    assert_eq!(body["error"]["provided"], json!(2));

    // Chunk 1: one more client.
    let (status, report) = push_chunk(&app, &key, session_id, 1, gamma_chunk()).await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["checkpoint"]["next_chunk"], json!(2));

    // Exact re-send of chunk 1: free replay, zero duplicate rows, no new
    // events (the engine's external-id idempotency holds on replay).
    let (status, replayed) = push_chunk(&app, &key, session_id, 1, gamma_chunk()).await;
    assert!(status.is_success(), "{replayed:?}");
    assert_eq!(replayed["replay"], json!(true));
    assert_eq!(replayed["report"]["summary"], report["report"]["summary"]);
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.created").await,
        3
    );
    assert_eq!(table_count(&state.pool, org.org_id, "clients").await, 3);

    // Content drift under a committed index is refused: the index is a
    // replay contract, not a dedupe hint.
    let (status, body) = push_chunk(&app, &key, session_id, 1, gamma_drift()).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    assert_eq!(body["error"]["reason"], json!("migration_chunk_reused"));
    assert_eq!(body["error"]["chunk_index"], json!(1));

    // Resume view: the tool that crashed mid-run asks where to continue.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/migration/push/sessions/{session_id}"),
        None,
        Some(&key),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["checkpoint"]["last_committed_chunk"], json!(1));
    assert_eq!(view["checkpoint"]["next_chunk"], json!(2));
    let indices: Vec<i64> = view["committed_chunks"]
        .as_array()
        .expect("committed_chunks")
        .iter()
        .map(|c| c["chunk_index"].as_i64().expect("index"))
        .collect();
    assert_eq!(indices, vec![0, 1]);

    // Finish: closes the session AND the D2 batch, emits both lifecycles.
    let (status, finished) = finish(&app, &key, session_id).await;
    assert!(status.is_success(), "{finished:?}");
    assert_eq!(finished["chunks_committed"], json!(2));
    assert_eq!(finished["rows_total"], json!(3));
    assert_eq!(finished["rows_created"], json!(3));
    assert_eq!(finished["checkpoint"]["next_chunk"], json!(2));

    assert_eq!(
        event_count(&state.pool, org.org_id, "migration.push_session_started").await,
        1
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "migration.push_session_finished").await,
        1
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "import.batch_started").await,
        1
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "import.batch_finished").await,
        1
    );
    assert_eq!(
        audit_count(&state.pool, org.org_id, "migration.session_started").await,
        1
    );
    assert_eq!(
        audit_count(&state.pool, org.org_id, "migration.chunk_committed").await,
        2
    );

    // Double finish is a typed conflict; so is any further chunk push.
    let (status, body) = finish(&app, &key, session_id).await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    let (status, body) = push_chunk(
        &app,
        &key,
        session_id,
        2,
        json_rows(json!([{ "external_key": "cli-d", "name": "Delta" }])),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
}

fn gamma_chunk() -> Value {
    json_rows(json!([{ "external_key": "cli-c", "name": "Gamma Ltd" }]))
}

fn gamma_drift() -> Value {
    json_rows(json!([{ "external_key": "cli-c", "name": "Gamma Holdings" }]))
}

// --- T3: fail-policy 409s are typed and retryable ---------------------------------

#[tokio::test]
async fn fail_policy_conflict_keeps_checkpoint_retryable() {
    let (state, app) = test_state(600).await;
    let org = register_org(&app, "d3-fail").await;
    let key = create_key(&app, &org, "strict", &["migration:write"]).await;

    let (status, session) = open_session(&app, &key, "client", "fail").await;
    assert!(status.is_success(), "{session:?}");
    let session_id: uuid::Uuid = session["session_id"].as_str().unwrap().parse().unwrap();

    // Chunk 0 creates the base row.
    let (status, report) = push_chunk(
        &app,
        &key,
        session_id,
        0,
        json_rows(json!([{ "external_key": "ACME", "name": "Acme Inc" }])),
    )
    .await;
    assert!(status.is_success(), "{report:?}");

    // Chunk 1 re-pushes the same key with drifted content under
    // conflict_policy=fail: the chunk is rejected transactionally with the
    // D2 typed 409 and the FULL per-row report; nothing was written.
    let (status, body) = push_chunk(
        &app,
        &key,
        session_id,
        1,
        json_rows(json!([
            { "external_key": "ACME", "name": "Acme Manufacturing" },
        ])),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    assert_eq!(reason_of(&body), "import_chunk_rejected");
    assert_eq!(body["error"]["report"]["rows"][0]["row_index"], json!(0));
    assert_eq!(body["error"]["report"]["conflict_policy"], json!("fail"));

    // The checkpoint did NOT advance: the tool corrects the rows and
    // re-sends the same index, which commits.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/migration/push/sessions/{session_id}"),
        None,
        Some(&key),
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["checkpoint"]["next_chunk"], json!(1));

    let (status, report) = push_chunk(
        &app,
        &key,
        session_id,
        1,
        json_rows(json!([{ "external_key": "GLOB", "name": "Glob Ltd" }])),
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    // Per-chunk fate: GLOB was the only row in this chunk...
    assert_eq!(report["report"]["summary"]["ok"], json!(1));
    // ...while the counters are the CUMULATIVE batch view (chunk 0's row
    // plus this one).
    assert_eq!(report["rows_created"], json!(2));

    let (status, finished) = finish(&app, &key, session_id).await;
    assert!(status.is_success(), "{finished:?}");
    assert_eq!(finished["rows_failed"], json!(0));
    assert_eq!(
        event_count(&state.pool, org.org_id, "import_chunk_rejected").await,
        0,
        "a rejected chunk emits no domain events"
    );
}

// --- T4: external ids make re-pushes across sessions idempotent -------------------

#[tokio::test]
async fn cross_session_repush_updates_instead_of_duplicating() {
    let (state, app) = test_state(600).await;
    let org = register_org(&app, "d3-idempotency").await;
    let key = create_key(&app, &org, "repusher", &["migration:write"]).await;

    let (status, first) = open_session(&app, &key, "client", "update").await;
    assert!(status.is_success());
    let first_id: uuid::Uuid = first["session_id"].as_str().unwrap().parse().unwrap();
    let (status, report) = push_chunk(
        &app,
        &key,
        first_id,
        0,
        json_rows(json!([{ "external_key": "ACME", "name": "Acme Inc" }])),
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    let (status, _) = finish(&app, &key, first_id).await;
    assert!(status.is_success());

    // A NEW session re-pushes the same external key with drifted content.
    let (status, second) = open_session(&app, &key, "client", "update").await;
    assert!(status.is_success());
    let second_id: uuid::Uuid = second["session_id"].as_str().unwrap().parse().unwrap();
    let (status, report) = push_chunk(
        &app,
        &key,
        second_id,
        0,
        json_rows(json!([{ "external_key": "ACME", "name": "Acme Manufacturing" }])),
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["report"]["summary"]["would_update"], json!(1));

    // Still ONE client, now under the new name; the mapping kept the key.
    assert_eq!(table_count(&state.pool, org.org_id, "clients").await, 1);
    let name: String = sqlx::query_scalar(
        "SELECT c.name FROM clients c JOIN external_ids e ON e.internal_id = c.id \
         WHERE e.org_id = $1 AND e.external_key = 'ACME'",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .expect("mapped client");
    assert_eq!(name, "Acme Manufacturing");
}

// --- T5: pull exports walk every family, cursor-paged -----------------------------

#[tokio::test]
async fn pull_exports_all_families_with_cursor_paging() {
    let (_state, app) = test_state(600).await;
    let org = register_org(&app, "d3-pull").await;
    let key = create_key(&app, &org, "puller", &["migration:write"]).await;

    // Seed through the connector itself: client → site → contact → board
    // → ticket → time entry (the documented import order).
    let (status, s) = open_session(&app, &key, "client", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([{ "external_key": "ACME", "name": "Acme Inc" }])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let (status, s) = open_session(&app, &key, "site", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(
            json!([{ "external_key": "HQ", "client_key": "ACME", "name": "HQ", "city": "Lyon" }]),
        ),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let (status, s) = open_session(&app, &key, "contact", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([{ "external_key": "DANA", "client_key": "ACME", "name": "Dana Reyes", "email": "dana@acme.example" }])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let (status, s) = open_session(&app, &key, "board", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([{ "external_key": "SRV", "name": "Migrated Service Desk", "is_default": "false" }])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let (status, s) = open_session(&app, &key, "ticket", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([{ "external_key": "T-1", "board_key": "SRV", "title": "Migrated ticket", "priority": "p2" }])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let (status, s) = open_session(&app, &key, "time_entry", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([{
            "external_key": "TE-1",
            "ticket_key": "T-1",
            // A key-driven entry MUST resolve a real org user (the key id
            // is not a users row) — the D2 fallback only works for human
            // drivers.
            "user_email": format!("owner@{}.test", "d3-pull"),
            "started_at": "2026-08-30T09:00:00Z",
            "minutes": "90",
            "billable": "true"
        }])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    // Every family exports; the org's default "Service Desk" board is part
    // of the board family, so it exports too (no external_key).
    for (family, expected) in [
        ("client", 1i64),
        ("site", 1),
        ("contact", 1),
        ("board", 2),
        ("ticket", 1),
        ("time_entry", 1),
    ] {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/migration/pull/{family}?limit=100"),
            None,
            Some(&key),
            None,
        )
        .await;
        let body = body_json(response).await;
        assert_eq!(
            body["items"].as_array().map(Vec::len),
            Some(expected as usize),
            "{family}: {body:?}"
        );
        assert!(body["next_cursor"].is_null(), "{family} fits one page");
        let items = body["items"].as_array().expect("items");
        assert!(
            items.iter().any(|item| item["external_key"].is_string()),
            "{family} carries the pushed row's stable key"
        );
        // The projection is the canonical import field catalog.
        assert!(
            body["items"][0]["projection"].is_object()
                && !body["items"][0]["projection"]
                    .as_object()
                    .unwrap()
                    .is_empty(),
            "{family} projection: {body:?}"
        );
    }

    // Unknown families are a 422, not a 500.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/migration/pull/gizmos",
        None,
        Some(&key),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );

    // Cursor pagination: walk a client family with limit=1 until
    // exhausted — ids strictly ascend and no row repeats. Seed two more
    // clients first.
    let (status, s) = open_session(&app, &key, "client", "skip").await;
    assert!(status.is_success());
    let sid: uuid::Uuid = s["session_id"].as_str().unwrap().parse().unwrap();
    let (status, r) = push_chunk(
        &app,
        &key,
        sid,
        0,
        json_rows(json!([
            { "external_key": "BETA", "name": "Beta" },
            { "external_key": "GAMMA", "name": "Gamma" }
        ])),
    )
    .await;
    assert!(status.is_success(), "{r:?}");
    let (status, _) = finish(&app, &key, sid).await;
    assert!(status.is_success());

    let mut cursor: Option<String> = None;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let suffix = match &cursor {
            Some(c) => format!("?cursor={c}&limit=1"),
            None => "?limit=1".to_string(),
        };
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/migration/pull/client{suffix}"),
            None,
            Some(&key),
            None,
        )
        .await;
        let body = body_json(response).await;
        let items = body["items"].as_array().expect("items");
        if items.is_empty() {
            panic!("empty page before exhaustion");
        }
        seen.push(items[0]["id"].as_str().unwrap().to_string());
        match body["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    assert_eq!(seen.len(), 3, "every client exactly once: {seen:?}");
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(seen, sorted, "keyset pages ascend");
}

// --- T6: per-key rate limiting ------------------------------------------------------

#[tokio::test]
async fn rate_limit_returns_typed_429_with_retry_after() {
    // Dedicated tiny budget; a fresh key means a fresh bucket.
    let (_state, app) = test_state(2).await;
    let org = register_org(&app, "d3-ratelimit").await;
    let key = create_key(&app, &org, "bursty", &["migration:write"]).await;

    for _ in 0..2 {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/migration/pull/client",
            None,
            Some(&key),
            None,
        )
        .await;
        assert!(response.status().is_success(), "within budget");
    }
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/migration/pull/client",
        None,
        Some(&key),
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    let retry_header = response
        .headers()
        .get(axum::http::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = body_json(response).await;
    assert_eq!(reason_of(&body), "rate_limited");
    let retry_after = body["error"]["retry_after_secs"]
        .as_i64()
        .expect("retry secs");
    assert!((1..=60).contains(&retry_after));
    assert_eq!(
        retry_header.as_deref(),
        Some(retry_after.to_string().as_str()),
        "header and body agree"
    );

    // A different key keeps its own budget.
    let other = create_key(&app, &org, "patient", &["migration:write"]).await;
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/migration/pull/client",
        None,
        Some(&other),
        None,
    )
    .await;
    assert!(
        response.status().is_success(),
        "per-key budgets are independent"
    );
}
