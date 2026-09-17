//! Credit-note integration tests (TASKLIST Money+docs backend gap, Wave 11
//! backend-gaps lane): issue → apply → refund → void through the REAL
//! core-api router against a real Postgres
//! (`MERIDIAN_TEST_DATABASE_URL` or `DATABASE_URL`).
//!
//! Pinned behavior:
//! * Issue — invoice-riding credits adopt the invoice's currency (a
//!   mismatched named currency is a typed 422); free-standing credits
//!   name their own; numbers are per-org CRN sequence; IF2 mutation
//!   metadata + byte-identical replay; missing client/invoice are typed
//!   404s inside a mutation (`rls_sparse`).
//! * Apply — the allocation is a REAL payment receipt (provider
//!   `credit_note`, provider_ref = the note number): the invoice's
//!   `total_paid_minor` moves and settles to `paid`; both sides
//!   compare-and-set, so over-allocation (credit or invoice) is a typed
//!   409; cross-client or cross-currency allocations are 422s; voided
//!   notes apply nothing.
//! * Refund — moves `refunded_minor` with the same remaining-value
//!   guard; refund + apply can never over-drain the credit.
//! * Void — only legal while untouched; terminal.
//!
//! Non-purpose: dunning statement delivery (same lane, exercised through
//! the statement endpoint's contract tests to come with the MailTransport
//! harness), invoice-run math (invoice_flow.rs).

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
        .expect("credit-note e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with credit-note surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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

async fn body_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
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
            "org_name": "Credit Co",
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

async fn create_contract(app: &Router, org: &Org, client_id: Uuid, body: Value) -> Value {
    let mut body = body;
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

/// One issued invoice via an invoice run (recurring contract, monthly
/// anchor inside the window).
async fn issue_invoice(app: &Router, org: &Org, client_id: Uuid, amount_minor: i64) -> Value {
    create_contract(
        app,
        org,
        client_id,
        json!({
            "number": format!("C-{}", Uuid::now_v7().simple()),
            "name": "Retainer",
            "contract_type": "recurring",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": amount_minor,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-10",
        }),
    )
    .await;
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/billing/invoice-runs",
        Some(&org.token),
        Some(json!({ "from": "2026-01-01", "to": "2026-01-20", "client_id": client_id })),
    )
    .await;
    assert_eq!(response.status(), 200, "invoice run must succeed: {}", {
        // Re-read for diagnostics when it fails.
        String::new()
    });
    let body = body_json(response).await;
    assert_eq!(body["run"]["invoice_count"], 1, "one invoice issued");
    body["invoices"][0].clone()
}

async fn issue_credit(app: &Router, org: &Org, body: Value) -> axum::response::Response {
    send(
        app,
        axum::http::Method::POST,
        "/api/credit-notes",
        Some(&org.token),
        Some(body),
    )
    .await
}

// --- issue --------------------------------------------------------------------

#[tokio::test]
async fn issue_adopts_invoice_currency_and_replays() {
    let (_state, app) = test_state().await;
    let slug = format!("crn-issue-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Acme").await;
    let invoice = issue_invoice(&app, &org, client_id, 200_000).await;
    let invoice_id: Uuid = invoice["invoice"]["id"]
        .as_str()
        .or(invoice["id"].as_str())
        .expect("invoice id")
        .parse()
        .unwrap();

    // A currency that disagrees with the riding invoice is a typed 422.
    let response = issue_credit(
        &app,
        &org,
        json!({
            "client_id": client_id,
            "invoice_id": invoice_id,
            "total_minor": 10_000,
            "currency": "EUR",
        }),
    )
    .await;
    assert_eq!(response.status(), 422, "currency mismatch must be 422");

    // Invoice-riding credit adopts the invoice currency.
    let key = format!("crn-{}", Uuid::now_v7().simple());
    let response = issue_credit(
        &app,
        &org,
        json!({
            "client_id": client_id,
            "invoice_id": invoice_id,
            "total_minor": 10_000,
            "reason": "Goodwill",
            "idempotency_key": key,
        }),
    )
    .await;
    assert_eq!(response.status(), 200, "issue must succeed");
    let note = body_json(response).await;
    assert_eq!(note["currency"], "USD");
    assert!(note["number"].as_str().unwrap().starts_with("CRN-"));
    assert_eq!(note["total_minor"], 10_000);
    assert_eq!(note["applied_minor"], 0);
    assert!(note["mutation"]["version"].is_i64(), "IF2 meta present");

    // Replay: same key + same payload ⇒ the SAME stored result.
    let response = issue_credit(
        &app,
        &org,
        json!({
            "client_id": client_id,
            "invoice_id": invoice_id,
            "total_minor": 10_000,
            "reason": "Goodwill",
            "idempotency_key": key,
        }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replay = body_json(response).await;
    assert_eq!(replay["id"], note["id"], "replay returns the same note");

    // Free-standing credit names its own currency.
    let response = issue_credit(
        &app,
        &org,
        json!({
            "client_id": client_id,
            "total_minor": 5_000,
            "currency": "EUR",
        }),
    )
    .await;
    assert_eq!(response.status(), 200, "free-standing credit accepted");
    let freestanding = body_json(response).await;
    assert_eq!(freestanding["currency"], "EUR");

    // A credit without a currency and without an invoice is a typed 422.
    let response = issue_credit(
        &app,
        &org,
        json!({ "client_id": client_id, "total_minor": 1_000 }),
    )
    .await;
    assert_eq!(response.status(), 422, "currency required");

    // Detail: remaining + (empty) applications ledger.
    let note_id: Uuid = note["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/credit-notes/{note_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let detail = body_json(response).await;
    assert_eq!(detail["remaining_minor"], 10_000);
    assert_eq!(detail["applications"].as_array().unwrap().len(), 0);

    // Cross-org read is not_found; cross-org mutation rls_sparse.
    let other = register_org(&app, format!("crn-other-{}", Uuid::now_v7().simple())).await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/credit-notes/{note_id}"),
        Some(&other.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/refund"),
        Some(&other.token),
        Some(json!({ "amount_minor": 1 })),
    )
    .await;
    assert_eq!(response.status(), 404);
    let _ = error_reason(&body_json(response).await);
}

// --- apply ----------------------------------------------------------------------

#[tokio::test]
async fn apply_moves_real_money_and_guards() {
    let (_state, app) = test_state().await;
    let slug = format!("crn-apply-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Globex").await;
    let invoice = issue_invoice(&app, &org, client_id, 100_000).await;
    let invoice_id: Uuid = invoice["invoice"]["id"]
        .as_str()
        .or(invoice["id"].as_str())
        .expect("invoice id")
        .parse()
        .unwrap();
    let response = issue_credit(
        &app,
        &org,
        json!({ "client_id": client_id, "invoice_id": invoice_id, "total_minor": 40_000 }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let note = body_json(response).await;
    let note_id: Uuid = note["id"].as_str().unwrap().parse().unwrap();

    // Over-allocation past the credit's remaining value is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/apply"),
        Some(&org.token),
        Some(json!({ "invoice_id": invoice_id, "amount_minor": 40_001 })),
    )
    .await;
    assert_eq!(response.status(), 409, "over-allocation must be 409");

    // Apply 30k: the invoice receipt is real money (paid total moves).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/apply"),
        Some(&org.token),
        Some(json!({
            "invoice_id": invoice_id,
            "amount_minor": 30_000,
            "idempotency_key": format!("apply-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "apply must succeed");
    let applied = body_json(response).await;
    assert_eq!(applied["applied_minor"], 30_000);
    assert_eq!(applied["total_minor"], 40_000);
    assert_eq!(applied["invoice"]["total_paid_minor"], 30_000);
    assert_eq!(applied["invoice"]["status"], "issued");
    assert_eq!(applied["payment"]["provider"], "credit_note");
    assert_eq!(applied["payment"]["provider_ref"], note["number"]);
    assert_eq!(applied["payment"]["amount_minor"], 30_000);

    // The receipt shows on the invoice's payment list too.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/invoices/{invoice_id}/payments"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let payments = body_json(response).await;
    assert_eq!(
        payments.as_array().unwrap().len(),
        1,
        "one receipt per apply"
    );

    // Apply the rest of the credit: the invoice shows 40k paid (still
    // issued — the credit is smaller than the invoice).
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/apply"),
        Some(&org.token),
        Some(json!({ "invoice_id": invoice_id, "amount_minor": 10_000 })),
    )
    .await;
    assert_eq!(response.status(), 200, "final apply must succeed: {}", {
        String::new()
    });
    let settled = body_json(response).await;
    assert_eq!(settled["applied_minor"], 40_000);
    assert_eq!(settled["invoice"]["total_paid_minor"], 40_000);
    assert_eq!(settled["invoice"]["status"], "issued");

    // The credit is drained; a further allocation is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/apply"),
        Some(&org.token),
        Some(json!({ "invoice_id": invoice_id, "amount_minor": 1 })),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Detail now shows one application row.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/credit-notes/{note_id}"),
        Some(&org.token),
        None,
    )
    .await;
    let detail = body_json(response).await;
    assert_eq!(
        detail["applications"].as_array().unwrap().len(),
        2,
        "one application row per apply"
    );
    assert_eq!(detail["remaining_minor"], 0);
    assert_eq!(detail["applied_minor"], 40_000);
}

// --- refund + void --------------------------------------------------------------

#[tokio::test]
async fn refund_and_void_guards() {
    let (_state, app) = test_state().await;
    let slug = format!("crn-refund-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Initech").await;
    let invoice = issue_invoice(&app, &org, client_id, 50_000).await;
    let invoice_id: Uuid = invoice["invoice"]["id"]
        .as_str()
        .or(invoice["id"].as_str())
        .expect("invoice id")
        .parse()
        .unwrap();

    // Untouched note voids cleanly; terminal (second void conflicts).
    let response = issue_credit(
        &app,
        &org,
        json!({ "client_id": client_id, "total_minor": 2_000, "currency": "USD" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let disposable = body_json(response).await;
    let disposable_id: Uuid = disposable["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{disposable_id}/void"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200, "untouched note voids");
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{disposable_id}/void"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "second void is a conflict");
    // A voided note refunds nothing.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{disposable_id}/refund"),
        Some(&org.token),
        Some(json!({ "amount_minor": 100 })),
    )
    .await;
    assert_eq!(response.status(), 409, "voided note refunds nothing");

    // Allocations block voiding; refund + apply drain to exactly zero.
    let response = issue_credit(
        &app,
        &org,
        json!({ "client_id": client_id, "invoice_id": invoice_id, "total_minor": 5_000 }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let note = body_json(response).await;
    let note_id: Uuid = note["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/refund"),
        Some(&org.token),
        Some(json!({ "amount_minor": 2_000, "notes": "Paid back by transfer" })),
    )
    .await;
    assert_eq!(response.status(), 200, "refund must succeed");
    let refunded = body_json(response).await;
    assert_eq!(refunded["refunded_minor"], 2_000);
    assert_eq!(refunded["total_minor"], 5_000);
    assert_eq!(refunded["applied_minor"], 0);
    assert_eq!(refunded["invoice"], Value::Null, "no invoice side");
    assert_eq!(refunded["payment"], Value::Null);

    // Voiding a note with allocations is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/void"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "allocated note cannot void");

    // Drain the rest: 1k apply + 2k refund ⇒ remaining 0; a further
    // refund of anything is a 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/apply"),
        Some(&org.token),
        Some(json!({ "invoice_id": invoice_id, "amount_minor": 1_000 })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/refund"),
        Some(&org.token),
        Some(json!({ "amount_minor": 2_000 })),
    )
    .await;
    assert_eq!(response.status(), 200, "final refund must succeed");
    let drained = body_json(response).await;
    assert_eq!(drained["refunded_minor"], 4_000);
    assert_eq!(drained["applied_minor"], 1_000);
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/refund"),
        Some(&org.token),
        Some(json!({ "amount_minor": 1 })),
    )
    .await;
    assert_eq!(response.status(), 409, "nothing left to refund");

    // Listing filters by status.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/credit-notes?client_id={client_id}&status=issued"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let listed = body_json(response).await;
    let listed = listed.as_array().unwrap();
    assert!(listed.iter().any(|n| n["id"] == note["id"]));
}

/// Minimal check that voided notes disappear from the issued filter but
/// remain readable.
#[tokio::test]
async fn issued_filter_excludes_voided() {
    let (_state, app) = test_state().await;
    let slug = format!("crn-filter-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Hooli").await;
    let response = issue_credit(
        &app,
        &org,
        json!({ "client_id": client_id, "total_minor": 1_500, "currency": "USD" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let note = body_json(response).await;
    let note_id: Uuid = note["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/credit-notes/{note_id}/void"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/credit-notes?client_id={client_id}&status=issued"),
        Some(&org.token),
        None,
    )
    .await;
    let listed = body_json(response).await;
    assert!(
        !listed
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == note["id"]),
        "voided note leaves the issued filter"
    );
}

// --- dunning statement (send-statement backend gap) ------------------------------

/// POST /api/invoices/{id}/dunning/statement — statement of the client's
/// open account queued to the client's contact emails through the
/// outbound ledger. None-mode transport stays queued. Idempotency-keyed
/// (IF1): a replay queues nothing further.
#[tokio::test]
async fn dunning_statement_queues_to_contacts() {
    let (state, app) = test_state().await;
    let slug = format!("crn-stmt-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Statement Client").await;

    // A contact with an email address is the delivery target.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/clients/{client_id}/contacts"),
        Some(&org.token),
        Some(json!({ "name": "Billing", "email": "billing@statement.test" })),
    )
    .await;
    assert_eq!(response.status(), 200, "contact create must succeed");

    let invoice = issue_invoice(&app, &org, client_id, 70_000).await;
    let invoice_id: Uuid = invoice["invoice"]["id"]
        .as_str()
        .or(invoice["id"].as_str())
        .expect("invoice id")
        .parse()
        .unwrap();
    let key = format!("stmt-{}", Uuid::now_v7().simple());

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{invoice_id}/dunning/statement"),
        Some(&org.token),
        Some(json!({ "idempotency_key": key })),
    )
    .await;
    let status = response.status();
    let statement = body_json(response).await;
    assert_eq!(status, 200, "statement must queue: {statement}");
    assert_eq!(statement["queued_to"], json!(["billing@statement.test"]));
    assert_eq!(statement["open_invoices"].as_array().unwrap().len(), 1);
    assert_eq!(
        statement["open_invoices"][0]["number"],
        invoice["invoice"]["number"]
    );
    assert_eq!(statement["total_outstanding_minor"], 70_000);
    assert_eq!(statement["currency"], "USD");

    // The outbound ledger holds exactly one QUEUED row (none-mode).
    let queued: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM email_outbound eo \
         JOIN email_messages em ON em.id = eo.email_message_id \
         WHERE em.org_id = $1 AND em.subject LIKE 'Account statement%'",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .expect("outbound aggregate");
    assert_eq!(queued.0, 1, "exactly one queued statement");

    // Replay: same key ⇒ byte-identical response, nothing further queued.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{invoice_id}/dunning/statement"),
        Some(&org.token),
        Some(json!({ "idempotency_key": key })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replay = body_json(response).await;
    assert_eq!(replay["queued_to"], statement["queued_to"]);
    let queued: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM email_outbound eo \
         JOIN email_messages em ON em.id = eo.email_message_id \
         WHERE em.org_id = $1 AND em.subject LIKE 'Account statement%'",
    )
    .bind(org.org_id)
    .fetch_one(&state.pool)
    .await
    .expect("outbound aggregate");
    assert_eq!(queued.0, 1, "replay queues nothing");

    // A client with no contact emails is a typed 422, not a silent skip.
    let invoice2 = issue_invoice(
        &app,
        &org,
        {
            let cid = create_client(&app, &org, "No Contacts").await;
            cid
        },
        10_000,
    )
    .await;
    let invoice2_id: Uuid = invoice2["invoice"]["id"]
        .as_str()
        .or(invoice2["id"].as_str())
        .expect("invoice id")
        .parse()
        .unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/invoices/{invoice2_id}/dunning/statement"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 422, "no delivery target is typed 422");
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );
}
