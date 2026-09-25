//! Invoicing-lane integration tests (TASKLIST Phase 3, Wave 9): the
//! contract → invoice run → document → payments → dunning chain through
//! the REAL core-api router against a real Postgres
//! (`MERIDIAN_TEST_DATABASE_URL` or `DATABASE_URL`).
//!
//! Pinned behavior:
//! * Runs — one invoice per billable contract with candidate lines over
//!   the window, issued instantly (`issued` + due terms), with IF2
//!   `mutation` metadata and byte-identical replay under the same
//!   idempotency key; mixed-currency runs are a typed 409 that persists
//!   nothing.
//! * Double-billing — a re-run over the same window creates an empty
//!   run (no invoices, no duplicate lines); an overlapping window bills
//!   only newly-arrived labor; fixed fees bill once per (due, label).
//! * Documents — the print-ready HTML document renders the invoice.
//! * Payments — partial then settling payments flip `issued` → `paid`;
//!   overpayments and payments on settled/void invoices are typed 409s;
//!   version conflicts ride the typed envelope.
//! * Dunning — forward-only ladder (skipping allowed, backwards 422),
//!   issued-only.
//! * Permissions/RLS — roleless users get `forbidden`; cross-org reads
//!   are `not_found` and mutations `rls_sparse`.
//!
//! Non-purpose: engine math edge cases (core-billing unit tests),
//! contract CRUD (billing_flow.rs), consumption-link uniqueness at the
//! query layer (core-db).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("invoicing e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with invoicing surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
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
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
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

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("body_json parse failed ({e}); last SEND lines identify the endpoint")
    })
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}

fn error_reason(body: &Value) -> String {
    body["error"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected typed error envelope, got: {body}"))
        .to_string()
}

struct Org {
    token: String,
    org_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Invoicing Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let tokens = body_json(response).await;
    let token = tokens["access_token"].as_str().unwrap().to_string();
    let me =
        body_json(send(app, axum::http::Method::GET, "/api/me", Some(&token), None).await).await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
    }
}

async fn create_client(app: &Router, org: &Org, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/clients",
        Some(&org.token),
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");
    let body = body_json(response).await;
    body["id"].as_str().unwrap().parse().unwrap()
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool and logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@invoicing.test");
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
    .expect("seed roleless user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({ "email": email, "password": "correct-horse-battery" })),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
}

async fn create_contract(app: &Router, org: &Org, client_id: Uuid, mut body: Value) -> Value {
    body["client_id"] = json!(client_id);
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body),
    )
    .await;
    assert_eq!(response.status(), 200, "contract create must succeed");
    body_json(response).await
}

async fn run_invoices(app: &Router, org: &Org, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/billing/invoice-runs",
        Some(&org.token),
        Some(body),
    )
    .await
}

/// Seeds a board, a ticket for `client_id`, and one billable time entry.
async fn add_time_entry(
    app: &Router,
    org: &Org,
    client_id: Uuid,
    minutes: i64,
    work_type: Option<&str>,
    day: &str,
) {
    let board = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/boards",
            Some(&org.token),
            Some(json!({ "name": format!("Ops-{day}-{minutes}"), "is_default": false })),
        )
        .await,
    )
    .await;
    let board_id: Uuid = board["id"].as_str().unwrap().parse().unwrap();
    let ticket = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&org.token),
            Some(json!({
                "board_id": board_id,
                "title": format!("Labor {day}"),
                "priority": "p2",
                "client_id": client_id,
            })),
        )
        .await,
    )
    .await;
    let ticket_id: Uuid = ticket["ticket"]["id"]
        .as_str()
        .or(ticket["id"].as_str())
        .expect("ticket id in response")
        .parse()
        .unwrap();
    let started = format!("{day}T10:00:00Z");
    let ended = format!("{day}T{:02}:{:02}:00Z", 10 + minutes / 60, minutes % 60);
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries",
        Some(&org.token),
        Some(json!({
            "ticket_id": ticket_id,
            "started_at": started,
            "ended_at": ended,
            "billable": true,
            "work_type": work_type,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "time entry create must succeed");
    // 0073 time approvals: entries start in `draft` and are only
    // billable once SUBMITTED and APPROVED — drive the real approval
    // flow here (the owner role carries ApproveTime).
    let entry: Value = body_json(response).await;
    let entry_id: Uuid = entry["id"]
        .as_str()
        .or(entry["entry"]["id"].as_str())
        .expect("time entry id in response")
        .parse()
        .unwrap();
    let submitted = send(
        app,
        axum::http::Method::POST,
        &format!("/api/time-entries/{entry_id}/submit"),
        Some(&org.token),
        Some(json!({ "note": null })),
    )
    .await;
    assert_eq!(submitted.status(), 200, "time entry submit must succeed");
    let decided = send(
        app,
        axum::http::Method::POST,
        "/api/time-entries/approvals/decide",
        Some(&org.token),
        Some(json!({
            "entry_ids": [entry_id],
            "decision": "approve",
            "reason_code": null,
            "note": null
        })),
    )
    .await;
    assert_eq!(decided.status(), 200, "time entry approve must succeed");
}

// --- run → document → payments → dunning -------------------------------------

#[tokio::test]
async fn invoice_run_end_to_end() {
    let (_state, app) = test_state().await;
    let slug = format!("inv-e2e-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Acme Corp").await;
    create_contract(
        &app,
        &org,
        client_id,
        json!({
            "number": format!("C-{}", Uuid::now_v7().simple()),
            "name": "Managed services",
            "contract_type": "recurring",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 200_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-15",
        }),
    )
    .await;

    // Window validation rides the typed envelope.
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-03-01", "to": "2026-02-01" }),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );

    // The run: three monthly periods inside Jan 1 .. Mar 20.
    let run_body = json!({
        "from": "2026-01-01",
        "to": "2026-03-20",
        "idempotency_key": "run-key-1",
    });
    let response = run_invoices(&app, &org, run_body.clone()).await;
    assert_eq!(response.status(), 200, "run must succeed");
    let run = body_json(response).await;
    assert_eq!(run["run"]["invoice_count"], json!(1));
    assert_eq!(run["run"]["total_minor"], json!(600_000));
    assert_eq!(run["run"]["currency"], json!("USD"));
    assert_eq!(run["mutation"]["seq"], json!(1));
    assert_eq!(run["mutation"]["idempotency_key"], json!("run-key-1"));
    let invoices = run["invoices"].as_array().unwrap();
    assert_eq!(invoices.len(), 1);
    let invoice = &invoices[0]["invoice"];
    assert_eq!(invoice["status"], json!("issued"));
    assert_eq!(invoice["total_minor"], json!(600_000));
    assert_eq!(invoice["total_paid_minor"], json!(0));
    assert_eq!(invoice["dunning_state"], json!("none"));
    assert_eq!(invoice["run_id"], run["run"]["id"]);
    // Payment terms: due 30 days after issue.
    let issued: chrono::NaiveDate = invoice["issued_at"].as_str().unwrap().parse().unwrap();
    let due: chrono::NaiveDate = invoice["due_on"].as_str().unwrap().parse().unwrap();
    assert_eq!((due - issued).num_days(), 30);
    let lines = invoices[0]["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3, "one line per monthly period");
    assert_eq!(lines[0]["kind"], json!("recurring_base"));
    assert_eq!(lines[0]["period_start"], json!("2026-01-15"));
    assert_eq!(invoices[0]["lines"][2]["period_start"], json!("2026-03-15"));
    let invoice_id: Uuid = invoice["id"].as_str().unwrap().parse().unwrap();

    // Same key + same body replays byte-identically.
    let replay = body_json(run_invoices(&app, &org, run_body).await).await;
    assert_eq!(replay, run);

    // Re-running the same window with a FRESH key bills nothing new.
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-03-20" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let second = body_json(response).await;
    assert_eq!(second["run"]["invoice_count"], json!(0));
    assert_eq!(second["invoices"].as_array().unwrap().len(), 0);

    // Detail endpoint returns the aggregate (document + lines, no
    // payments yet).
    let detail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/invoices/{invoice_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["id"], json!(invoice_id));
    assert_eq!(detail["lines"].as_array().unwrap().len(), 3);
    assert_eq!(detail["payments"].as_array().unwrap().len(), 0);

    // List endpoint filters by client/status.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/invoices?client_id={client_id}&status=issued"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // The print-ready HTML document renders number, client, and total.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/invoices/{invoice_id}/document"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let html = body_text(response).await;
    assert!(html.contains(&format!("Invoice {}", invoice["number"].as_str().unwrap())));
    assert!(html.contains("Acme Corp"));
    assert!(html.contains("6000.00"));

    // Dunning: forward moves are fine, backwards are typed 422s.
    let dunning = |app: &Router, org: &Org, id: Uuid, body: Value| {
        let app = app.clone();
        let org = Org {
            token: org.token.clone(),
            org_id: org.org_id,
        };
        async move {
            send(
                &app,
                axum::http::Method::POST,
                &format!("/api/invoices/{id}/dunning"),
                Some(&org.token),
                Some(body),
            )
            .await
        }
    };
    let response = dunning(
        &app,
        &org,
        invoice_id,
        json!({ "to_state": "reminder_sent", "note": "friendly nudge" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let moved = body_json(response).await;
    assert_eq!(moved["dunning_state"], json!("reminder_sent"));
    assert_eq!(moved["mutation"]["seq"], json!(2));
    let response = dunning(
        &app,
        &org,
        invoice_id,
        json!({ "to_state": "reminder_sent" }),
    )
    .await;
    assert_eq!(response.status(), 422, "same-state is refused");
    let response = dunning(&app, &org, invoice_id, json!({ "to_state": "overdue" })).await;
    assert_eq!(response.status(), 200, "skipping forward is allowed");
    let response = dunning(
        &app,
        &org,
        invoice_id,
        json!({ "to_state": "reminder_sent" }),
    )
    .await;
    assert_eq!(response.status(), 422, "backwards is refused");
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );

    // Payments: partial, then settling. Overpayments are typed 409s.
    let pay = |app: &Router, org: &Org, id: Uuid, body: Value| {
        let app = app.clone();
        let org = Org {
            token: org.token.clone(),
            org_id: org.org_id,
        };
        async move {
            send(
                &app,
                axum::http::Method::POST,
                &format!("/api/invoices/{id}/payments"),
                Some(&org.token),
                Some(body),
            )
            .await
        }
    };
    let response = pay(&app, &org, invoice_id, json!({ "amount_minor": 0 })).await;
    assert_eq!(response.status(), 422, "non-positive amounts are invalid");
    let response = pay(&app, &org, invoice_id, json!({ "amount_minor": 700_000 })).await;
    assert_eq!(response.status(), 409, "overpayment is refused");
    assert_eq!(error_reason(&body_json(response).await), "conflict");

    let response = pay(
        &app,
        &org,
        invoice_id,
        json!({ "amount_minor": 250_000, "notes": "first instalment", "idempotency_key": "pay-1" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let paid = body_json(response).await;
    assert_eq!(paid["payment"]["amount_minor"], json!(250_000));
    assert_eq!(paid["payment"]["provider"], json!("none"));
    assert_eq!(paid["invoice"]["status"], json!("issued"));
    assert_eq!(paid["invoice"]["total_paid_minor"], json!(250_000));
    let replay = body_json(
        pay(
            &app,
            &org,
            invoice_id,
            json!({ "amount_minor": 250_000, "notes": "first instalment", "idempotency_key": "pay-1" }),
        )
        .await,
    )
    .await;
    assert_eq!(replay, paid, "payment replay is byte-identical");

    // A stale If-Match version is a typed 409 carrying expected/provided.
    let response = pay(
        &app,
        &org,
        invoice_id,
        json!({ "amount_minor": 100, "version": 99 }),
    )
    .await;
    assert_eq!(response.status(), 409);
    let conflict = body_json(response).await;
    assert_eq!(error_reason(&conflict), "version_conflict");

    let response = pay(
        &app,
        &org,
        invoice_id,
        json!({ "amount_minor": 350_000, "idempotency_key": "pay-2" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let settled = body_json(response).await;
    assert_eq!(settled["invoice"]["status"], json!("paid"));
    assert_eq!(settled["invoice"]["total_paid_minor"], json!(600_000));

    // Payments on settled invoices and dunning on settled invoices are
    // both typed conflicts.
    let response = pay(&app, &org, invoice_id, json!({ "amount_minor": 100 })).await;
    assert_eq!(response.status(), 409);
    let response = dunning(
        &app,
        &org,
        invoice_id,
        json!({ "to_state": "final_notice" }),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Payment receipts are queryable.
    let payments = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/invoices/{invoice_id}/payments"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(payments.as_array().unwrap().len(), 2);

    // Void another invoice: run April, void it, then a second void (with
    // a fresh key) is a typed conflict.
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-04-01", "to": "2026-04-30" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let april = body_json(response).await;
    let april_invoices = april["invoices"].as_array().unwrap();
    assert_eq!(april_invoices.len(), 1, "only the April period is new");
    let april_id: Uuid = april_invoices[0]["invoice"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{april_id}/void"),
        Some(&org.token),
        Some(json!({ "idempotency_key": "void-1" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let voided = body_json(response).await;
    assert_eq!(voided["status"], json!("void"));
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{april_id}/void"),
        Some(&org.token),
        Some(json!({ "idempotency_key": "void-2" })),
    )
    .await;
    assert_eq!(response.status(), 409, "void is terminal");

    // Run listing surfaces both runs, newest first.
    let runs = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/billing/invoice-runs",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert!(runs.as_array().unwrap().len() >= 3);
}

// --- double-billing invariants ------------------------------------------------

#[tokio::test]
async fn invoice_run_never_double_bills() {
    let (_state, app) = test_state().await;
    let slug = format!("inv-dedupe-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;

    // T&M: entries bill once, ever — even across overlapping windows.
    let tm_client = create_client(&app, &org, "T&M Client").await;
    create_contract(
        &app,
        &org,
        tm_client,
        json!({
            "number": format!("C-M-{}", Uuid::now_v7().simple()),
            "name": "Project labor",
            "contract_type": "time_and_materials",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "hourly_rate_minor": 10_000,
        }),
    )
    .await;
    add_time_entry(&app, &org, tm_client, 60, None, "2026-01-05").await;
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let first = body_json(response).await;
    let invoices = first["invoices"].as_array().unwrap();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0]["lines"][0]["quantity_minutes"], json!(60));
    assert_eq!(invoices[0]["lines"][0]["amount_minor"], json!(10_000));

    // Overlapping window with one NEW entry: only the new minutes bill.
    add_time_entry(&app, &org, tm_client, 30, None, "2026-02-05").await;
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-15", "to": "2026-02-28" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let second = body_json(response).await;
    let invoices = second["invoices"].as_array().unwrap();
    assert_eq!(invoices.len(), 1, "the new entry bills in a new invoice");
    assert_eq!(invoices[0]["lines"][0]["quantity_minutes"], json!(30));
    assert_eq!(invoices[0]["lines"][0]["amount_minor"], json!(5_000));

    // Fixed fee: one line per (due, label), stable across re-runs.
    let fee_client = create_client(&app, &org, "Project Client").await;
    create_contract(
        &app,
        &org,
        fee_client,
        json!({
            "number": format!("C-F-{}", Uuid::now_v7().simple()),
            "name": "Migration project",
            "contract_type": "fixed_fee",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "fees": [
                { "label": "Assessment", "amount_minor": 250_000, "due_on": "2026-01-10" }
            ],
        }),
    )
    .await;
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let fee_run = body_json(response).await;
    assert_eq!(fee_run["run"]["invoice_count"], json!(1));
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let fee_again = body_json(response).await;
    assert_eq!(
        fee_again["run"]["invoice_count"],
        json!(0),
        "the same fee never bills twice"
    );

    // A fully-empty window produces an empty run.
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2030-01-01", "to": "2030-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let empty = body_json(response).await;
    assert_eq!(empty["run"]["invoice_count"], json!(0));
    assert_eq!(empty["run"]["currency"], json!(null));
}

#[tokio::test]
async fn mixed_currency_run_is_typed_conflict_and_persists_nothing() {
    let (_state, app) = test_state().await;
    let slug = format!("inv-fx-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let usd = create_client(&app, &org, "USD Client").await;
    let eur = create_client(&app, &org, "EUR Client").await;
    create_contract(
        &app,
        &org,
        usd,
        json!({
            "number": format!("C-U-{}", Uuid::now_v7().simple()),
            "name": "USD deal",
            "contract_type": "recurring",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 100_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-01",
        }),
    )
    .await;
    create_contract(
        &app,
        &org,
        eur,
        json!({
            "number": format!("C-E-{}", Uuid::now_v7().simple()),
            "name": "EUR deal",
            "contract_type": "recurring",
            "status": "active",
            "currency": "EUR",
            "effective_from": "2026-01-01",
            "amount_minor": 100_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-01",
        }),
    )
    .await;
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(error_reason(&body_json(response).await), "conflict");
    // The transaction rolled back: no invoices, no partial run.
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/invoices",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 0);
}

// --- permissions / RLS --------------------------------------------------------

#[tokio::test]
async fn invoicing_permissions_and_rls() {
    let (state, app) = test_state().await;
    let slug = format!("inv-rbac-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Acme").await;
    create_contract(
        &app,
        &org,
        client_id,
        json!({
            "number": format!("C-{}", Uuid::now_v7().simple()),
            "name": "Managed services",
            "contract_type": "recurring",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 200_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-15",
        }),
    )
    .await;
    let response = run_invoices(
        &app,
        &org,
        json!({ "from": "2026-01-01", "to": "2026-01-31" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let run = body_json(response).await;
    let invoice_id: Uuid = run["invoices"][0]["invoice"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // A roleless user gets the typed forbidden envelope everywhere.
    let roleless = add_roleless_user(&app, &state, &org, "roleless").await;
    for (method, uri) in [
        (
            axum::http::Method::POST,
            "/api/billing/invoice-runs".to_string(),
        ),
        (axum::http::Method::GET, "/api/invoices".to_string()),
        (
            axum::http::Method::POST,
            format!("/api/invoices/{invoice_id}/payments"),
        ),
        (
            axum::http::Method::POST,
            format!("/api/invoices/{invoice_id}/dunning"),
        ),
    ] {
        let response = send(
            &app,
            method,
            &uri,
            Some(&roleless),
            Some(json!({ "from": "2026-01-01", "to": "2026-01-31", "amount_minor": 1, "to_state": "overdue" })),
        )
        .await;
        assert_eq!(response.status(), 403, "{uri} must be forbidden");
        assert_eq!(error_reason(&body_json(response).await), "forbidden");
    }

    // Cross-org: reads are not_found, mutations rls_sparse.
    let other = register_org(&app, format!("inv-other-{}", Uuid::now_v7().simple())).await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/invoices/{invoice_id}"),
        Some(&other.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(error_reason(&body_json(response).await), "not_found");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{invoice_id}/void"),
        Some(&other.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(error_reason(&body_json(response).await), "rls_sparse");

    // Invoiced contracts are delete-locked (the Wave 9 billing-lock
    // guard invited by the contracts lane).
    let contract_id: Uuid = run["invoices"][0]["invoice"]["contract_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 409, "invoiced contracts are locked");
    assert_eq!(error_reason(&body_json(response).await), "conflict");
}
