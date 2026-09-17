//! D2-remainder endpoint flow (saved templates, staged async batches, and
//! the board/ticket/time-entry families) through the REAL core-api router,
//! including the worker-called internal runner.
//!
//! Purpose:
//! 1. Template CRUD over HTTP + template application in dry-run (source
//!    headers remap to canonical names before validation).
//! 2. Async giant batches: stage chunks to the ObjectStore, enqueue via
//!    /run (exactly once), replay through /api/internal/imports/batch/run
//!    (the worker's 2.7-queue callback), and see the batch finish with
//!    accurate counters. Re-delivery of a finished batch is an idempotent
//!    no-op.
//! 3. New entity families end-to-end: an imported BOARD (with the semantic
//!    status floor) accepts an imported TICKET, which accepts an imported
//!    TIME ENTRY attributed by user email.
//! 4. Typed error envelope everywhere: unknown template, wrong-kind
//!    template, staging into queued batches, and double enqueues are
//!    machine-readable rejections.
//!
//! Database selection follows settings_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const INTERNAL_TOKEN: &str = "test-internal-token";

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("import async e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([13; 32]);
    // The internal surface is enabled with a known token so the suite can
    // play the worker.
    config.internal_token = Some(INTERNAL_TOKEN.to_string());
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
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

fn internal() -> (&'static str, String) {
    ("authorization", format!("Bearer {INTERNAL_TOKEN}"))
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Async Co",
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

async fn open_batch(app: &Router, token: &str, kind: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/imports/batches",
        &[auth(token)],
        Some(json!({
            "entity_kind": kind,
            "conflict_policy": "skip",
            "label": format!("{kind} wave"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "batch opens");
    body_json(response).await["batch_id"]
        .as_str()
        .unwrap()
        .to_string()
        .parse()
        .unwrap()
}

async fn stage(app: &Router, token: &str, batch_id: Uuid, rows: Value) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{batch_id}/stage"),
        &[auth(token)],
        Some(json!({ "format": "json", "json_rows": rows })),
    )
    .await;
    assert_eq!(response.status(), 200, "chunk stages");
    body_json(response).await
}

async fn run_internal(app: &Router, org_id: Uuid, batch_id: Uuid, job_id: Uuid) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/internal/imports/batch/run",
        &[internal()],
        Some(json!({
            "org_id": org_id,
            "batch_id": batch_id,
            "job_id": job_id,
        })),
    )
    .await;
    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        200,
        "worker replay applies — body: {}",
        String::from_utf8_lossy(&body_bytes)
    );
    serde_json::from_slice(&body_bytes).unwrap()
}

#[tokio::test]
async fn template_crud_and_dry_run_application() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("import-tpl-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // A template mapping a fictional exporter's headers to canonical names.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/imports/templates",
        &[auth(&token)],
        Some(json!({
            "name": "CW companies",
            "source_system": "connectwise",
            "entity_kind": "client",
            "mapping": { "Company Name": "name", "Company ID": "external_key", "Comment": "notes" }
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "template saves");
    let template = body_json(response).await;
    let template_id = template["id"].as_str().unwrap().to_string();
    assert_eq!(template["entity_kind"], "client");

    // A template whose target is not a canonical column is a typed 422.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/imports/templates",
        &[auth(&token)],
        Some(json!({
            "name": "Bad target",
            "source_system": "connectwise",
            "entity_kind": "client",
            "mapping": { "Company Name": "not_a_column" }
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "bad mapping target rejected");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed");

    // Dry-run through the template: source headers remap before validation.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/imports/dry-run",
        &[auth(&token)],
        Some(json!({
            "entity_kind": "client",
            "conflict_policy": "skip",
            "template_id": template_id,
            "format": "json",
            "json_rows": [
                { "Company ID": "CW-1", "Company Name": "Acme (mapped)", "Comment": "hello" }
            ]
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "dry-run applies the template");
    let report = body_json(response).await;
    assert_eq!(report["summary"]["ok"], 1, "mapped row validates clean");
    assert_eq!(report["rows"][0]["external_key"], "CW-1");

    // A template for the WRONG kind is refused loudly.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/imports/dry-run",
        &[auth(&token)],
        Some(json!({
            "entity_kind": "site",
            "conflict_policy": "skip",
            "template_id": template_id,
            "format": "json",
            "json_rows": []
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "kind mismatch is a validation error"
    );

    // Unknown template id → typed not_found.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/imports/dry-run",
        &[auth(&token)],
        Some(json!({
            "entity_kind": "client",
            "conflict_policy": "skip",
            "template_id": Uuid::now_v7(),
            "format": "json",
            "json_rows": []
        })),
    )
    .await;
    assert_eq!(response.status(), 404);
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "not_found");

    // Listing + delete round-trip.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/imports/templates?source_system=connectwise",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let listed = body_json(response).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/imports/templates/{template_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn staged_batch_runs_through_the_worker_surface() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("import-async-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    let org_id: Uuid = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&token)],
            None,
        )
        .await;
        body_json(response).await["org_id"]
            .as_str()
            .unwrap()
            .to_string()
            .parse()
            .unwrap()
    };
    let batch_id = open_batch(&app, &token, "client").await;

    // Two staged chunks — the deterministic keys accumulate.
    let staged = stage(
        &app,
        &token,
        batch_id,
        json!([
            { "external_key": "A-1", "name": "Async Client A" },
            { "external_key": "A-2", "name": "Async Client B" }
        ]),
    )
    .await;
    assert_eq!(staged["chunk_index"], 0);
    assert_eq!(staged["staged_chunks"], 1);
    let staged = stage(
        &app,
        &token,
        batch_id,
        json!([{ "external_key": "A-3", "name": "Async Client C" }]),
    )
    .await;
    assert_eq!(staged["chunk_index"], 1);
    assert_eq!(staged["staged_chunks"], 2);

    // Enqueue exactly once.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{batch_id}/run"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "enqueue applies");
    let run = body_json(response).await;
    assert_eq!(run["staged_chunks"], 2);
    let job_id: Uuid = run["job_id"].as_str().unwrap().to_string().parse().unwrap();

    // Staging and re-enqueueing are both typed conflicts now.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{batch_id}/stage"),
        &[auth(&token)],
        Some(json!({ "format": "json", "json_rows": [] })),
    )
    .await;
    assert_eq!(response.status(), 409, "staging closed after enqueue");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "conflict");

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{batch_id}/run"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 409, "double enqueue rejected");

    // The worker surface replays the staged chunks and finishes the batch.
    let result = run_internal(&app, org_id, batch_id, job_id).await;
    assert_eq!(result["applied"], true);
    assert_eq!(result["rows_created"], 3);

    // Batch view: finished, accurate counters.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/imports/batches/{batch_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    let batch = body_json(response).await;
    assert!(batch["finished_at"].is_string(), "batch finished");
    assert_eq!(batch["rows_total"], 3);
    assert_eq!(batch["rows_created"], 3);

    // Idempotent redelivery: the worker retrying a finished batch is a
    // no-op, not an error.
    let result = run_internal(&app, org_id, batch_id, job_id).await;
    assert_eq!(result["applied"], true);
    assert_eq!(result["skipped"], "already finished");

    // The internal surface rejects a bad token with the typed envelope.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/internal/imports/batch/run",
        &[("authorization", "Bearer wrong-token".to_string())],
        Some(json!({ "org_id": Uuid::now_v7(), "batch_id": batch_id })),
    )
    .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn board_ticket_time_entry_families_import_end_to_end() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("import-fam-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;

    // 1. Import a board asynchronously (worker surface).
    let org_id: Uuid = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&token)],
            None,
        )
        .await;
        body_json(response).await["org_id"]
            .as_str()
            .unwrap()
            .to_string()
            .parse()
            .unwrap()
    };
    let board_batch = open_batch(&app, &token, "board").await;
    stage(
        &app,
        &token,
        board_batch,
        json!([{ "external_key": "SRV-BOARD", "name": "Imported Service Desk", "is_default": "false" }]),
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{board_batch}/run"),
        &[auth(&token)],
        None,
    )
    .await;
    let run = body_json(response).await;
    let job_id: Uuid = run["job_id"].as_str().unwrap().to_string().parse().unwrap();
    let result = run_internal(&app, org_id, board_batch, job_id).await;
    assert_eq!(result["rows_created"], 1);

    // The imported board exposes the semantic status floor through the
    // public boards surface (defaults-first doctrine holds for imports).
    let imported_board_id: Uuid = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let id: Uuid = sqlx::query_scalar(
            "SELECT internal_id FROM external_ids WHERE org_id = $1 AND entity_kind = 'board' AND external_key = 'SRV-BOARD'",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("board mapping");
        tx.commit().await.expect("commit");
        id
    };
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/boards/{imported_board_id}/statuses"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "imported board is readable");
    let statuses = body_json(response).await;
    let categories: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["category"].as_str().unwrap())
        .collect();
    assert_eq!(
        categories,
        vec!["open", "in_progress", "waiting", "closed"],
        "semantic floor seeded on import"
    );

    // 2. Import a TICKET synchronously, parented on the imported board.
    let ticket_batch = open_batch(&app, &token, "ticket").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{ticket_batch}/chunks"),
        &[auth(&token)],
        Some(json!({
            "format": "json",
            "json_rows": [
                {
                    "external_key": "T-100",
                    "board_key": "SRV-BOARD",
                    "title": "Migrated ticket",
                    "body": "from the old system",
                    "priority": "p2"
                }
            ]
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket chunk commits");
    let report = body_json(response).await;
    assert_eq!(report["summary"]["ok"], 1);

    let imported_ticket_id: Uuid = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let id: Uuid = sqlx::query_scalar(
            "SELECT internal_id FROM external_ids WHERE org_id = $1 AND entity_kind = 'ticket' AND external_key = 'T-100'",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("ticket mapping");
        tx.commit().await.expect("commit");
        id
    };

    // 3. Import a TIME ENTRY for that ticket, attributed by user email.
    let entry_batch = open_batch(&app, &token, "time_entry").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{entry_batch}/chunks"),
        &[auth(&token)],
        Some(json!({
            "format": "json",
            "json_rows": [
                {
                    "external_key": "TE-1",
                    "ticket_key": "T-100",
                    "user_email": format!("owner@{slug}.test"),
                    "started_at": "2026-08-30T09:00:00Z",
                    "minutes": "90",
                    "billable": "true",
                    "notes": "migrated work"
                }
            ]
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "time entry chunk commits");
    let report = body_json(response).await;
    assert_eq!(report["summary"]["ok"], 1);

    // Everything landed through the canonical mutators: one ticket with a
    // 90-minute entry attributed to the resolved owner user.
    let counts: (i64, i64, i64) = {
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let tickets: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tickets WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        let entries: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM time_entries WHERE org_id = $1 AND ticket_id = $2",
        )
        .bind(org_id)
        .bind(imported_ticket_id)
        .fetch_one(&mut *tx)
        .await
        .expect("entries");
        let minutes: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(minutes), 0) FROM time_entries WHERE org_id = $1",
        )
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("minutes");
        tx.commit().await.expect("commit");
        (tickets, entries, minutes)
    };
    assert_eq!(counts.0, 1, "one imported ticket");
    assert_eq!(counts.1, 1, "one imported time entry");
    assert_eq!(counts.2, 90, "minutes landed");

    // Re-pushing the identical time-entry chunk is a no-op skip (the
    // idempotency doctrine extends to the new families).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{entry_batch}/chunks"),
        &[auth(&token)],
        Some(json!({
            "format": "json",
            "json_rows": [
                {
                    "external_key": "TE-1",
                    "ticket_key": "T-100",
                    "user_email": format!("owner@{slug}.test"),
                    "started_at": "2026-08-30T09:00:00Z",
                    "minutes": "90",
                    "billable": "true",
                    "notes": "migrated work"
                }
            ]
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let report = body_json(response).await;
    assert_eq!(
        report["summary"]["would_skip"], 1,
        "identical re-push skips"
    );

    // A ticket row referencing an UNKNOWN board key fails per-row without
    // touching the rest.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/imports/batches/{ticket_batch}/chunks"),
        &[auth(&token)],
        Some(json!({
            "format": "json",
            "json_rows": [
                { "external_key": "T-200", "board_key": "NOPE", "title": "Orphan" }
            ]
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let report = body_json(response).await;
    assert_eq!(report["summary"]["failed"], 1);
    assert_eq!(
        report["rows"][0]["outcome"]["status"], "failed",
        "per-row report names the missing parent"
    );
}
