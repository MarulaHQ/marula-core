//! End-to-end import framework flows (TASKLIST D2) through the REAL
//! core-api router against a real Postgres.
//!
//! Purpose: prove the D2 surface over HTTP — dry-run report accuracy with no
//! writes to data tables, the conflict-policy matrix (skip/update/fail) over
//! missing/identical/different content, re-push idempotency (zero duplicate
//! rows AND zero duplicate outbox events), fail-policy chunk atomicity via
//! the typed 409, CSV quoting round-trips, cross-family key resolution
//! (clients → sites → contacts), org-admin + api-key gating, per-chunk row
//! limits, and audit provenance under actor_kind "import".
//!
//! Non-purpose: pure planning laws (core-domain suite), persistence-level RLS
//! probes (core-db import_queries suite).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const MAX_ROWS: usize = core_domain::import_core::MAX_ROWS_PER_CHUNK;

/// One-shot schema reset per process. Pools are built PER CALL afterwards
/// because each `#[tokio::test]` owns its own runtime (see core-db common).
static BOOTSTRAP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    BOOTSTRAP
        .get_or_init(|| async {
            let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect("imports e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
            let boot = connect_admin(&admin_url).await.expect("bootstrap pool");
            // Fresh schema keeps slugs/keys collision-free no matter which
            // suite ran before this binary.
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
                .expect("RLS coverage holds with import surfaces");
        })
        .await;
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("imports e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url;
    let state = AppState {
        pool,
        reporting: core_api::state::AppState::reporting_pool_lazy(&config.database_url)
            .expect("reporting pool"),
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
    };
    let app = core_api::routes::build_router(state.clone());
    (state, app)
}

async fn connect_admin(url: &str) -> Result<PgPool, sqlx::Error> {
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
            "org_name": "Import Co",
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

/// Adds a same-org user with NO roles (no org-admin rights) directly through
/// the admin pool — there is no invite API yet — then logs them in.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@imports.test");
    core_db::users::create_user(
        &state.pool,
        Uuid::now_v7(),
        org.org_id,
        &email,
        label,
        &core_api::auth::password::hash("correct-horse-battery").unwrap(),
        "UTC",
    )
    .await
    .expect("seed limited user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            None,
            Some(json!({ "email": email, "password": "correct-horse-battery" })),
        )
        .await,
    )
    .await;
    login["access_token"]
        .as_str()
        .expect("login token")
        .to_string()
}

async fn event_count(pool: &PgPool, org: Uuid, event_type: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE org_id = $1 AND event_type = $2")
        .bind(org)
        .bind(event_type)
        .fetch_one(pool)
        .await
        .expect("outbox count")
}

async fn open_batch(app: &Router, org: &Org, kind: &str, policy: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/imports/batches",
        Some(&org.token),
        None,
        Some(json!({
            "entity_kind": kind,
            "conflict_policy": policy,
            "label": format!("{kind}-{policy}-test"),
        })),
    )
    .await;
    assert!(response.status().is_success(), "open batch: {response:?}");
    let body = body_json(response).await;
    Uuid::parse_str(body["batch_id"].as_str().expect("batch id")).expect("uuid")
}

async fn push_chunk(
    app: &Router,
    org: &Org,
    batch: Uuid,
    payload: Value,
    api_key: Option<&str>,
) -> (axum::http::StatusCode, Value) {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/imports/batches/{batch}/chunks"),
        if api_key.is_none() {
            Some(org.token.as_str())
        } else {
            None
        },
        api_key,
        Some(payload),
    )
    .await;
    let status = response.status();
    (status, body_json(response).await)
}

fn json_rows(rows: Value) -> Value {
    json!({ "format": "json", "json_rows": rows })
}

// --- T1: dry-run accuracy without touching data --------------------------------

#[tokio::test]
async fn dry_run_reports_row_fates_and_records_a_probe() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-dryrun").await;

    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/imports/dry-run",
        Some(&org.token),
        None,
        Some(json!({
            "entity_kind": "client",
            "conflict_policy": "update",
            "label": "probe-1",
            "format": "json",
            "json_rows": [
                { "external_key": "ACME", "name": "Acme" },
                { "external_key": "NOPE", "name": "" },
                { "external_key": "WEIRD", "name": "W", "bogus_column": "?" }
            ],
        })),
    )
    .await;
    assert!(response.status().is_success(), "{response:?}");
    let body = body_json(response).await;
    assert_eq!(body["dry_run"], json!(true));
    assert_eq!(body["summary"]["total"], json!(3));
    assert_eq!(body["summary"]["ok"], json!(1));
    assert_eq!(body["summary"]["failed"], json!(2));
    assert_eq!(body["rows"][0]["outcome"]["status"], json!("ok"));
    assert_eq!(body["rows"][1]["outcome"]["status"], json!("failed"));
    assert_eq!(
        body["rows"][1]["outcome"]["errors"],
        json!(["name: required"])
    );
    assert_eq!(body["rows"][2]["outcome"]["status"], json!("failed"));

    // Nothing was created.
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.created").await,
        0,
        "dry-run must not emit mutation events"
    );

    // The probe is recorded and finished for operator history.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/imports/batches",
        Some(&org.token),
        None,
        None,
    )
    .await;
    let list = body_json(response).await;
    let probe = list
        .as_array()
        .expect("list")
        .iter()
        .find(|b| b["label"] == json!("probe-1"))
        .expect("probe batch recorded");
    assert_eq!(probe["dry_run"], json!(true));
    assert!(probe["finished_at"].is_object() || probe["finished_at"].is_string());
    drop(state);
}

// --- T2: policy matrix, idempotency, event hygiene ------------------------------

#[tokio::test]
async fn update_policy_idempotent_repush_and_skip_then_update_matrix() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-policy").await;

    // INSERT two clients under update policy.
    let batch = open_batch(&app, &org, "client", "update").await;
    let payload = json_rows(json!([
        { "external_key": "ACME-EXT", "name": "Acme Inc" },
        { "external_key": "GLOB-EXT", "name": "Glob Ltd" },
    ]));
    let (status, report) = push_chunk(&app, &org, batch, payload, None).await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["ok"], json!(2));
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.created").await,
        2,
        "one created event per inserted row"
    );

    // RE-PUSH identical content: skip ×2, zero new events, zero new rows.
    let again = json_rows(json!([
        { "external_key": "ACME-EXT", "name": "Acme Inc" },
        { "external_key": "GLOB-EXT", "name": "Glob Ltd" },
    ]));
    let (status, report) = push_chunk(&app, &org, batch, again, None).await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_skip"], json!(2));
    assert_eq!(
        report["rows"][0]["outcome"]["reason"],
        json!("identical to existing row")
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.created").await,
        2,
        "re-push must be eventless"
    );

    // Skip policy keeps differing remote content at bay.
    let skip_batch = open_batch(&app, &org, "client", "skip").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        skip_batch,
        json_rows(json!([{ "external_key": "ACME-EXT", "name": "Renamed Under Skip" }])),
        None,
    )
    .await;
    assert!(status.is_success());
    assert_eq!(report["summary"]["would_skip"], json!(1));
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.updated").await,
        0
    );

    // Update policy applies the same diff.
    let upd_batch = open_batch(&app, &org, "client", "update").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        upd_batch,
        json_rows(json!([{ "external_key": "ACME-EXT", "name": "Acme Holdings" }])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_update"], json!(1));
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.updated").await,
        1,
        "exactly one update event"
    );

    // The rename is live through the ordinary read surface; GLOB untouched.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/clients",
        Some(&org.token),
        None,
        None,
    )
    .await;
    let clients = body_json(response).await;
    let names: Vec<&str> = clients
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"Acme Holdings"), "{names:?}");
    assert!(names.contains(&"Glob Ltd"), "{names:?}");
    drop(state);
}

// --- T3: fail policy atomicity ---------------------------------------------------

#[tokio::test]
async fn fail_policy_rejects_whole_chunk_and_writes_nothing() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-failgate").await;

    // Seed one mapped key through a clean run.
    let seed_batch = open_batch(&app, &org, "client", "update").await;
    let (_, _) = push_chunk(
        &app,
        &org,
        seed_batch,
        json_rows(json!([{ "external_key": "EXISTING", "name": "Stored Name" }])),
        None,
    )
    .await;

    let fail_batch = open_batch(&app, &org, "client", "fail").await;
    let (status, body) = push_chunk(
        &app,
        &org,
        fail_batch,
        json_rows(json!([
            { "external_key": "NEWOK", "name": "Would Be Fresh" },
            { "external_key": "EXISTING", "name": "Different Content" },
        ])),
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    assert_eq!(
        body["error"]["message"],
        json!("import chunk rejected by conflict_policy=fail")
    );
    let report = &body["error"]["report"];
    assert_eq!(report["summary"]["failed"], json!(1));

    // NOTHING from that chunk persisted: no new row, no renamed existing,
    // no counters on the still-open batch.
    assert_eq!(
        event_count(&state.pool, org.org_id, "client.created").await,
        1,
        "only the seed row exists"
    );
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/imports/batches/{fail_batch}"),
        Some(&org.token),
        None,
        None,
    )
    .await;
    let view = body_json(response).await;
    assert_eq!(view["rows_total"], json!(0));
    assert_eq!(view["finished_at"], json!(null));
    drop(state);
}

// --- T4: CSV quirks + cross-family resolution ------------------------------------

#[tokio::test]
async fn csv_quoting_and_family_chain_sites_contacts() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-csvchain").await;

    let client_batch = open_batch(&app, &org, "client", "skip").await;
    let csv_body = "external_key,name,notes\nCSVCO,\"Acme, Inc.\",from-csv\n";
    let (status, report) = push_chunk(
        &app,
        &org,
        client_batch,
        json!({ "format": "csv", "csv": csv_body }),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["ok"], json!(1));

    // Quoted comma survives into the stored name.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/clients",
        Some(&org.token),
        None,
        None,
    )
    .await;
    let clients = body_json(response).await;
    assert!(
        clients
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == json!("Acme, Inc.")),
        "quoted name intact"
    );

    // Sites resolve their parent by external key; orphans fail cleanly.
    let site_batch = open_batch(&app, &org, "site", "skip").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        site_batch,
        json_rows(json!([
            { "external_key": "HQ-SITE", "client_key": "CSVCO", "name": "HQ",
              "latitude": "41.8781", "longitude": "-87.6298", "timezone": "America/Chicago" },
            { "external_key": "ORPHAN", "client_key": "GHOST-CO", "name": "??" },
        ])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["rows"][0]["outcome"]["status"], json!("ok"));
    assert_eq!(report["rows"][1]["outcome"]["status"], json!("failed"));
    let sites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sites WHERE org_id = $1")
        .bind(org.org_id)
        .fetch_one(&state.pool)
        .await
        .expect("site count");
    assert_eq!(sites, 1);

    // Contacts chain client + optional site keys; the site link lands.
    let contact_batch = open_batch(&app, &org, "contact", "update").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        contact_batch,
        json_rows(json!([
            { "external_key": "PAT", "client_key": "CSVCO", "site_key": "HQ-SITE",
              "name": "Pat Doe", "email": "pat@acme.example.com", "is_primary": true },
        ])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    let contacts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE org_id = $1 AND email IS NOT NULL")
            .bind(org.org_id)
            .fetch_one(&state.pool)
            .await
            .expect("contact count");
    assert_eq!(contacts, 1);

    // finish closes the last batch with correct cumulative counters.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/imports/batches/{contact_batch}/finish"),
        Some(&org.token),
        None,
        None,
    )
    .await;
    assert!(response.status().is_success());
    let view = body_json(response).await;
    assert_eq!(view["rows_total"], json!(1));
    assert_eq!(view["rows_created"], json!(1));
    drop(state);
}

// --- T5: gates (owner/admin humans, scoped machine keys, anonymous) ---------------

#[tokio::test]
async fn human_admin_gate_and_scoped_machine_keys() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-gates").await;

    // Read-only key CANNOT drive imports (needs both write scopes).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(&org.token),
        None,
        Some(json!({ "name": "ro-key", "scopes": ["clients:read"] })),
    )
    .await;
    let ro_key = body_json(response).await["raw_key"]
        .as_str()
        .expect("raw key")
        .to_string();
    let (status, _) = push_chunk(
        &app,
        &org,
        Uuid::now_v7(),
        json!({ "format": "json", "json_rows": [] }),
        Some(&ro_key),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN);

    // Write-scoped machine key drives the whole lifecycle end to end.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/api-keys",
        Some(&org.token),
        None,
        Some(json!({ "name": "mig-bot", "scopes": ["clients:write"] })),
    )
    .await;
    let created = body_json(response).await;
    let raw_key = created["raw_key"].as_str().expect("raw key").to_string();

    let (status, _) = push_chunk(
        &app,
        &org,
        Uuid::now_v7(),
        json!({ "format": "json", "json_rows": [] }),
        Some(&raw_key),
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "unknown batch is 404"
    );

    // Roleless human JWT cannot import either — the admin gate bites.
    let limited_token = add_roleless_user(&app, &state, &org, "roleless").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/imports/dry-run",
        Some(&limited_token),
        None,
        Some(json!({
            "entity_kind": "client", "conflict_policy": "skip",
            "format": "json", "json_rows": [],
        })),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

    // Machine key OPENS its own batch, so driver provenance lands on it.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/imports/batches",
        None,
        Some(&raw_key),
        Some(json!({
            "entity_kind": "client",
            "conflict_policy": "skip",
            "label": "machine-run",
        })),
    )
    .await;
    assert!(response.status().is_success(), "machine open: {response:?}");
    let machine_batch =
        Uuid::parse_str(body_json(response).await["batch_id"].as_str().expect("id")).expect("uuid");
    let (status, _) = push_chunk(
        &app,
        &org,
        machine_batch,
        json_rows(json!([{ "external_key": "MACHINE-CO", "name": "Machine Pushed" }])),
        Some(&raw_key),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    // Provenance: batches carry the api_key driver; audits carry actor_kind
    // 'import' with an api-key descriptor.
    let driver_kind: String =
        sqlx::query_scalar("SELECT created_by_kind FROM import_batches WHERE id = $1")
            .bind(machine_batch)
            .fetch_one(&state.pool)
            .await
            .expect("driver kind");
    assert_eq!(driver_kind, "api_key");
    let import_audits: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE org_id = $1 AND actor_kind = 'import'",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .expect("audit count");
    assert!(import_audits >= 2, "start + chunk + finish lineage");

    // Anonymous requests stay 401.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/imports/batches",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    drop(state);
}

// --- T6: chunk limits -------------------------------------------------------------

#[tokio::test]
async fn chunk_row_limit_is_enforced_at_the_boundary() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, "imports-limits").await;
    let batch = open_batch(&app, &org, "client", "skip").await;

    let oversized: Vec<Value> = (0..=MAX_ROWS)
        .map(|i| json!({ "external_key": format!("K{i}"), "name": "Row" }))
        .collect();
    let (status, body) = push_chunk(&app, &org, batch, json_rows(json!(oversized)), None).await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "{body:?}"
    );
    let message = body["error"]["message"].as_str().expect("message");
    assert!(message.contains("row limit exceeded"), "{message}");
}

// --- T7: KB families (D5) ----------------------------------------------------------

#[tokio::test]
async fn kb_categories_and_articles_flow_through_the_import_engine() {
    let (state, app) = test_state().await;
    let org = register_org(&app, "imports-kb").await;

    // Categories first: the parent family rides its own batch, exactly
    // like the client → site chain.
    let category_batch = open_batch(&app, &org, "kb_category", "update").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        category_batch,
        json_rows(json!([
            {
                "external_key": "getting-started",
                "name": "Getting started",
                "slug": "getting-started",
                "description": "First steps",
                "position": "1",
                "client_visible": "true",
            },
        ])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["ok"], json!(1));

    // Articles: one parented + published, one top-level draft.
    let article_batch = open_batch(&app, &org, "kb_article", "update").await;
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([
            {
                "external_key": "article:reset-password",
                "category_slug": "getting-started",
                "title": "Reset a password",
                "slug": "reset-password",
                "body": "Step one.",
                "status": "published",
                "client_visible": "true",
            },
            {
                "external_key": "article:internal-runbook",
                "title": "Internal runbook",
                "slug": "internal-runbook",
            },
        ])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["ok"], json!(2), "rows: {report:?}");
    assert_eq!(
        event_count(&state.pool, org.org_id, "kb_article.created").await,
        2,
        "articles emit the same lifecycle events as the authoring surface"
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "kb_category.created").await,
        0,
        "categories are non-evented, matching the authoring surface"
    );

    // Live through the ordinary read surface, with the parent resolved.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/kb/articles",
        Some(&org.token),
        None,
        None,
    )
    .await;
    let articles = body_json(response).await;
    let by_slug: std::collections::BTreeMap<String, Value> = articles
        .as_array()
        .expect("articles list")
        .iter()
        .map(|a| (a["slug"].as_str().expect("slug").to_string(), a.clone()))
        .collect();
    assert_eq!(by_slug.len(), 2);
    assert_eq!(by_slug["reset-password"]["status"], json!("published"));
    assert_eq!(by_slug["reset-password"]["client_visible"], json!(true));
    assert!(by_slug["reset-password"]["category_id"].is_string());
    assert_eq!(by_slug["internal-runbook"]["status"], json!("draft"));
    assert_eq!(by_slug["internal-runbook"]["category_id"], json!(null));

    // Identical re-push skips both and emits nothing.
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([
            {
                "external_key": "article:reset-password",
                "category_slug": "getting-started",
                "title": "Reset a password",
                "slug": "reset-password",
                "body": "Step one.",
                "status": "published",
                "client_visible": "true",
            },
            {
                "external_key": "article:internal-runbook",
                "title": "Internal runbook",
                "slug": "internal-runbook",
            },
        ])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_skip"], json!(2));
    assert_eq!(
        event_count(&state.pool, org.org_id, "kb_article.updated").await,
        0,
        "identical re-push must be eventless"
    );

    // A drift re-push flows through the regular update path.
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([{
            "external_key": "article:reset-password",
            "category_slug": "getting-started",
            "title": "Reset a password",
            "slug": "reset-password",
            "body": "Step one. Step two.",
            "status": "published",
            "client_visible": "true",
        }])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_update"], json!(1));
    assert_eq!(
        event_count(&state.pool, org.org_id, "kb_article.updated").await,
        1,
        "exactly one update event"
    );

    // Unknown category slug fails the row in the report — never invented.
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([{
            "external_key": "article:orphan",
            "category_slug": "no-such-category",
            "title": "Orphan",
            "slug": "orphan",
        }])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["failed"], json!(1));
    assert_eq!(
        report["rows"][0]["outcome"]["errors"][0],
        json!("category_slug \"no-such-category\": no imported kb category with this slug")
    );

    // Duplicate slug across distinct external keys aborts the chunk with
    // the same typed 409 the authoring POST returns.
    let (status, body) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([
            {
                "external_key": "article:slug-a",
                "title": "Slug A",
                "slug": "collide",
            },
            {
                "external_key": "article:slug-b",
                "title": "Slug B",
                "slug": "collide",
            },
        ])),
        None,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::CONFLICT, "{body:?}");
    assert_eq!(
        body["error"]["message"],
        json!("slug already in use (kb article)")
    );
    assert_eq!(
        event_count(&state.pool, org.org_id, "kb_article.created").await,
        2,
        "the aborted chunk must not have published either row"
    );

    // The archived content lock holds on the bulk path too: archive the
    // runbook, then refuse a body-only drift through import.
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([{
            "external_key": "article:internal-runbook",
            "title": "Internal runbook",
            "slug": "internal-runbook",
            "status": "archived",
        }])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_update"], json!(1));
    let (status, body) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([{
            "external_key": "article:internal-runbook",
            "title": "Internal runbook",
            "slug": "internal-runbook",
            "status": "archived",
            "body": "new content under lock",
        }])),
        None,
    )
    .await;
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "{body:?}"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("archived"),
        "{body:?}"
    );

    // Status flips stay legal — unarchiving through import works.
    let (status, report) = push_chunk(
        &app,
        &org,
        article_batch,
        json_rows(json!([{
            "external_key": "article:internal-runbook",
            "title": "Internal runbook",
            "slug": "internal-runbook",
            "status": "published",
        }])),
        None,
    )
    .await;
    assert!(status.is_success(), "{report:?}");
    assert_eq!(report["summary"]["would_update"], json!(1));

    // The unarchive landed through the ordinary read surface.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/kb/articles",
        Some(&org.token),
        None,
        None,
    )
    .await;
    let articles = body_json(response).await;
    let runbook = articles
        .as_array()
        .expect("articles list")
        .iter()
        .find(|a| a["slug"] == json!("internal-runbook"))
        .expect("runbook article");
    assert_eq!(runbook["status"], json!("published"));
    drop(state);
}
