//! Money-path flow for Wave 54 lane S3c (`p54-money-and-roles`), driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove an SLA service credit is priced only in the contract's
//! currency. The write path refuses a mismatch with the same typed 422 its
//! sibling credit-note write uses, and an evaluation refuses a credit that
//! predates that guard (naming the credit) instead of pricing it in the wrong
//! currency or dropping it silently.
//!
//! The same suite carries the Stripe webhook's out-of-band settlement (Wave 55
//! lane `p55-stripe-payment-cas`): the total the event was priced on rides into
//! the payment statement's predicate, so a settlement whose total moved between
//! the event's read and the write is a typed 409 that writes nothing, a
//! settlement on the total it read lands and records its receipt, and a replay
//! of the same intent does not apply twice.
//!
//! The concurrency half of that idempotence is the database's (Wave 55 lane
//! `p55-stripe-settlement-fence`): migration 0165 makes one provider reference
//! unique per invoice, so two deliveries that both read "absent" cannot both
//! insert. Two overlapping deliveries of one partial settlement end with one
//! receipt and one `acted: false`, and the ordinary manual path — whose
//! reference is derived from the invoice itself, not from a provider — still
//! records two instalments on that invoice.
//!
//! Non-purpose: engine arithmetic (core-billing unit tests, including the
//! FEFO block draw), the credit-note surface, and the invoice run pipeline.

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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("money e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&admin_url)
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Money Co",
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
        .to_string()
}

/// One active T&M contract in `currency`, seeded directly so the test owns
/// the contract currency the credit must match.
async fn seed_contract(state: &AppState, slug: &str, currency: &str) -> Uuid {
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Money Client')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .expect("client row");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, \
         currency, hourly_rate_minor, effective_from) \
         VALUES ($1, $2, $3, $4, 'Money T&M', 'time_and_materials', 'active', $5, 15000, $6)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(format!(
        "C-MONEY-{}",
        &contract_id.simple().to_string()[..8]
    ))
    .bind(currency)
    .bind(chrono::Utc::now().date_naive() - chrono::Duration::days(30))
    .execute(&state.pool)
    .await
    .expect("contract row");
    contract_id
}

#[tokio::test]
async fn sla_credit_in_another_currency_is_refused_with_the_typed_answer() {
    let (state, app) = test_state().await;
    let slug = format!("money-fx-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract_id}/sla-credits"),
        Some(&owner),
        Some(json!({
            "reason": "resolution target breached by 2d",
            "amount_minor": 15_000,
            "currency": "USD",
            "applies_on": today.to_string(),
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        422,
        "a USD credit on a EUR contract is a validation failure"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "validation_failed");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("USD") && message.contains("EUR"),
        "the refusal names both currencies: {message}"
    );

    // Nothing was stored: a refused write leaves no row the engine could
    // later price in the wrong currency.
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM contract_sla_credits WHERE contract_id = $1")
            .bind(contract_id)
            .fetch_one(&state.pool)
            .await
            .expect("credit count");
    assert_eq!(stored, 0, "the refused credit must not be stored");
}

#[tokio::test]
async fn sla_credit_in_the_contract_currency_prices_as_a_negative_line() {
    let (state, app) = test_state().await;
    let slug = format!("money-same-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    // Lower case on the wire, the contract's code on the row: the currency
    // is normalized, so the comparison is by code and not by spelling.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/contracts/{contract_id}/sla-credits"),
        Some(&owner),
        Some(json!({
            "reason": "resolution target breached by 2d",
            "amount_minor": 15_000,
            "currency": "eur",
            "applies_on": today.to_string(),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "a same-currency credit is accepted");
    let credit = body_json(response).await;
    assert_eq!(credit["currency"], "EUR", "stored in the contract's code");

    let from = (today - chrono::Duration::days(30)).to_string();
    let to = today.to_string();
    let preview = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}/billing-preview?from={from}&to={to}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(preview.status(), 200, "preview must succeed");
    let evaluation = body_json(preview).await;
    let credits: Vec<&Value> = evaluation["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .filter(|l| l["kind"] == "sla_credit")
        .collect();
    assert_eq!(credits.len(), 1, "the credit prices once: {evaluation}");
    assert_eq!(credits[0]["amount_minor"], -15_000, "a credit is negative");
}

#[tokio::test]
async fn legacy_mismatched_credit_refuses_the_evaluation_and_names_the_credit() {
    let (state, app) = test_state().await;
    let slug = format!("money-legacy-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let contract_id = seed_contract(&state, &slug, "EUR").await;
    let today = chrono::Utc::now().date_naive();

    // A row written BEFORE the write-path guard: insert it straight into the
    // table, which is the only way to reach this state now.
    let credit_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO contract_sla_credits (id, org_id, contract_id, reason, amount_minor, \
         currency, applies_on, status) \
         SELECT $1, c.org_id, c.id, 'legacy USD credit', 15000, 'USD', $2, 'pending' \
         FROM contracts c WHERE c.id = $3",
    )
    .bind(credit_id)
    .bind(today)
    .bind(contract_id)
    .execute(&state.pool)
    .await
    .expect("legacy credit row");

    let from = (today - chrono::Duration::days(30)).to_string();
    let to = today.to_string();
    let preview = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}/billing-preview?from={from}&to={to}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        preview.status(),
        422,
        "the evaluation refuses a credit it cannot price honestly"
    );
    let body = body_json(preview).await;
    assert_eq!(body["error"]["reason"], "validation_failed");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&credit_id.to_string()),
        "the refusal names the offending credit: {message}"
    );
    assert!(
        message.contains("USD") && message.contains("EUR"),
        "the refusal names both currencies: {message}"
    );
}

// ---------------------------------------------------------------------------
// Stripe webhook settlement (Wave 55 lane `p55-stripe-payment-cas`)
// ---------------------------------------------------------------------------

/// The secret the route reads per request. Every test in this binary sets the
/// same value, so a parallel test cannot change what another one is verifying.
const WEBHOOK_SECRET: &str = "whsec_p55_ledger_cas";

/// The env the receiver reads before it will process an event at all.
fn ensure_webhook_secret() {
    std::env::set_var("MARULA_STRIPE_WEBHOOK_SECRET", WEBHOOK_SECRET);
}

struct PayableInvoice {
    org_id: Uuid,
    invoice_id: Uuid,
    intent_id: String,
}

/// One org with one issued invoice for 100.00 in USD, and the Stripe intent id
/// the event will carry. Seeded directly: this is about the settlement
/// decision, not about how an invoice comes to exist.
async fn seed_payable_invoice(state: &AppState, label: &str) -> PayableInvoice {
    let org_id = Uuid::now_v7();
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'Webhook Money Co', $2)")
        .bind(org_id)
        .bind(label)
        .execute(&state.pool)
        .await
        .expect("org row");
    let invoice_id = seed_issued_invoice_for_org(state, org_id, 10_000).await;
    PayableInvoice {
        org_id,
        invoice_id,
        intent_id: format!("pi_{}", Uuid::now_v7().simple()),
    }
}

/// One issued invoice for 100.00 in USD on an org that already exists, with the
/// client and contract row an invoice needs. Direct inserts, so a test can drive
/// a route that authenticates as a registered owner.
async fn seed_issued_invoice_for_org(state: &AppState, org_id: Uuid, total_minor: i64) -> Uuid {
    let client_id = Uuid::now_v7();
    let contract_id = Uuid::now_v7();
    let invoice_id = Uuid::now_v7();
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Webhook Client')")
        .bind(client_id)
        .bind(org_id)
        .execute(&state.pool)
        .await
        .expect("client row");
    sqlx::query(
        "INSERT INTO contracts (id, org_id, client_id, number, name, contract_type, status, \
         currency, hourly_rate_minor, effective_from) \
         VALUES ($1, $2, $3, $4, 'Webhook T&M', 'time_and_materials', 'active', 'USD', 15000, \
                 CURRENT_DATE)",
    )
    .bind(contract_id)
    .bind(org_id)
    .bind(client_id)
    .bind(format!("C-WH-{}", &contract_id.simple().to_string()[..8]))
    .execute(&state.pool)
    .await
    .expect("contract row");
    sqlx::query(
        "INSERT INTO invoices (id, org_id, client_id, contract_id, number, status, currency, \
         issued_at, due_on, total_minor, total_paid_minor) \
         VALUES ($1, $2, $3, $4, $5, 'issued', 'USD', CURRENT_DATE, CURRENT_DATE + 30, \
                 $6, 0)",
    )
    .bind(invoice_id)
    .bind(org_id)
    .bind(client_id)
    .bind(contract_id)
    .bind(format!("INV-WH-{}", &invoice_id.simple().to_string()[..8]))
    .bind(total_minor)
    .execute(&state.pool)
    .await
    .expect("invoice row");
    invoice_id
}

/// A `payment_intent.succeeded` event tagged for this invoice, shaped the way
/// Stripe sends one: the intent id is the provider ref the settlement records,
/// and the org and invoice ride in the metadata.
fn settlement_event(invoice: &PayableInvoice, amount_minor: i64) -> Value {
    json!({
        "id": format!("evt_{}", Uuid::now_v7().simple()),
        "type": "payment_intent.succeeded",
        "data": {
            "object": {
                "id": invoice.intent_id,
                "amount_received": amount_minor,
                "currency": "usd",
                "metadata": {
                    "marula_org_id": invoice.org_id.to_string(),
                    "marula_invoice_id": invoice.invoice_id.to_string(),
                },
            }
        },
    })
}

/// Delivers one signed event to the receiver. The signature covers the exact
/// bytes in the body, and a fresh timestamp keeps it inside the skew window,
/// which is what lets the replay case re-sign the same event.
async fn deliver(app: &Router, event: &Value) -> axum::response::Response {
    let raw = event.to_string();
    let timestamp = chrono::Utc::now().timestamp();
    let signed = format!("{timestamp}.{raw}");
    let signature = core_mail::hmac::sign(WEBHOOK_SECRET.as_bytes(), signed.as_bytes());
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/webhooks/stripe")
        .header("content-type", "application/json")
        .header("stripe-signature", format!("t={timestamp},v1={signature}"))
        .body(axum::body::Body::from(raw))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn invoice_paid_and_status(state: &AppState, invoice_id: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT total_paid_minor, status FROM invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&state.pool)
        .await
        .expect("invoice row")
}

async fn receipts_for(state: &AppState, invoice_id: Uuid) -> Vec<(i64, String, Option<String>)> {
    sqlx::query_as(
        "SELECT amount_minor, provider, provider_ref FROM invoice_payments \
         WHERE invoice_id = $1 ORDER BY received_at, id",
    )
    .bind(invoice_id)
    .fetch_all(&state.pool)
    .await
    .expect("receipt rows")
}

async fn webhook_audit_rows(state: &AppState, org_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM audit_events \
         WHERE org_id = $1 AND action = 'invoice.payment_recorded_webhook'",
    )
    .bind(org_id)
    .fetch_one(&state.pool)
    .await
    .expect("audit count")
}

/// Waits until `want` backends of this database are parked on a lock while
/// running a statement against `invoice_payments`. That is the state the
/// settlement reaches after it has read the invoice and before it writes the
/// paid total — the window under test.
///
/// The probe matches the insert by table rather than by its first clause: a
/// backend executing a prepared statement can report its text differently
/// between server versions, and `invoice_payments` survives that, while the
/// poll query itself cannot match because it is never in a `Lock` wait.
async fn wait_for_the_parked_receipt_inserts(pool: &PgPool, want: i64) {
    for _ in 0..200 {
        let parked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() \
               AND wait_event_type = 'Lock' \
               AND query LIKE '%invoice_payments%'",
        )
        .fetch_one(pool)
        .await
        .expect("activity probe");
        if parked >= want {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the settlement never reached its receipt insert");
}

#[tokio::test]
async fn a_webhook_settlement_on_a_moved_total_is_refused_and_writes_nothing() {
    let (state, app) = test_state().await;
    ensure_webhook_secret();
    let invoice =
        seed_payable_invoice(&state, &format!("wh-moved-{}", Uuid::now_v7().simple())).await;

    // The amend that moves the total, holding the invoice row so the
    // settlement cannot reach its own write until this transaction commits.
    let mut mover = state.pool.begin().await.expect("mover transaction");
    sqlx::query("SELECT total_minor FROM invoices WHERE id = $1 FOR UPDATE")
        .bind(invoice.invoice_id)
        .fetch_one(&mut *mover)
        .await
        .expect("hold the invoice row");

    let event = settlement_event(&invoice, 2_500);
    let app_for_delivery = app.clone();
    let delivery = tokio::spawn(async move { deliver(&app_for_delivery, &event).await });

    // The settlement reads 100.00, then parks on the receipt insert behind the
    // row lock. The total moves and commits while it waits.
    wait_for_the_parked_receipt_inserts(&state.pool, 1).await;
    sqlx::query("UPDATE invoices SET total_minor = 12_500, updated_at = now() WHERE id = $1")
        .bind(invoice.invoice_id)
        .execute(&mut *mover)
        .await
        .expect("total edit");
    mover.commit().await.expect("commit the moved total");

    let response = delivery.await.expect("delivery task");
    assert_eq!(
        response.status(),
        409,
        "a settlement priced on a total that moved is a conflict, not a write"
    );
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "conflict");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("10000") && message.contains("12500"),
        "the refusal names both totals: {message}"
    );

    // Nothing was written: no paid total, no status flip, no receipt row, and
    // no audit row claiming a settlement that did not happen.
    let (paid, status) = invoice_paid_and_status(&state, invoice.invoice_id).await;
    assert_eq!(paid, 0, "no payment was applied against the moved total");
    assert_eq!(status, "issued", "the invoice keeps its payable status");
    assert_eq!(
        receipts_for(&state, invoice.invoice_id).await.len(),
        0,
        "the rolled-back receipt left nothing behind"
    );
    assert_eq!(
        webhook_audit_rows(&state, invoice.org_id).await,
        0,
        "no settlement is audited that the ledger refused"
    );
}

#[tokio::test]
async fn a_webhook_settlement_on_the_total_it_read_lands_and_records_its_receipt() {
    let (state, app) = test_state().await;
    ensure_webhook_secret();
    let invoice =
        seed_payable_invoice(&state, &format!("wh-lands-{}", Uuid::now_v7().simple())).await;

    let response = deliver(&app, &settlement_event(&invoice, 2_500)).await;
    assert_eq!(
        response.status(),
        200,
        "a settlement on the read total lands"
    );
    let body = body_json(response).await;
    assert_eq!(body["received"], true);
    assert_eq!(body["acted"], true, "the delivery recorded a receipt");

    let (paid, status) = invoice_paid_and_status(&state, invoice.invoice_id).await;
    assert_eq!(paid, 2_500, "the receipt is on the paid total");
    assert_eq!(status, "issued", "25.00 of 100.00 leaves the invoice open");

    let receipts = receipts_for(&state, invoice.invoice_id).await;
    assert_eq!(receipts.len(), 1, "exactly one receipt");
    assert_eq!(receipts[0].0, 2_500, "the settled amount");
    assert_eq!(receipts[0].1, "stripe", "the provider is recorded");
    assert_eq!(
        receipts[0].2.as_deref(),
        Some(invoice.intent_id.as_str()),
        "the intent id is the provider ref an idempotent retry looks for"
    );
    assert_eq!(
        webhook_audit_rows(&state, invoice.org_id).await,
        1,
        "the settlement is audited once"
    );
}

#[tokio::test]
async fn replaying_the_same_intent_does_not_apply_the_payment_twice() {
    let (state, app) = test_state().await;
    ensure_webhook_secret();
    let invoice =
        seed_payable_invoice(&state, &format!("wh-replay-{}", Uuid::now_v7().simple())).await;

    // A partial settlement, so the second delivery cannot be stopped by the
    // paid status or the balance: only the provider_ref guard decides it.
    let first = deliver(&app, &settlement_event(&invoice, 2_500)).await;
    assert_eq!(first.status(), 200);
    assert_eq!(
        body_json(first).await["acted"],
        true,
        "the first delivery applies"
    );

    let replay = deliver(&app, &settlement_event(&invoice, 2_500)).await;
    assert_eq!(
        replay.status(),
        200,
        "a redelivery is acknowledged, not failed"
    );
    assert_eq!(
        body_json(replay).await["acted"],
        false,
        "the replay finds the existing receipt and does nothing"
    );

    let (paid, status) = invoice_paid_and_status(&state, invoice.invoice_id).await;
    assert_eq!(paid, 2_500, "the payment was applied once, not twice");
    assert_eq!(status, "issued");
    assert_eq!(
        receipts_for(&state, invoice.invoice_id).await.len(),
        1,
        "one receipt for one intent"
    );
    assert_eq!(
        webhook_audit_rows(&state, invoice.org_id).await,
        1,
        "the replay adds no second audit row"
    );
}

/// The record this lane closes: two deliveries of ONE partial settlement, in
/// flight at the same time, must land the money once.
///
/// The barrier is the invoice row. A transaction here holds it `FOR UPDATE`, so
/// both deliveries get through their advisory provider-reference read (it takes
/// no lock), then park on the receipt insert — a child-row insert takes a key
/// share on the parent, which waits on `FOR UPDATE`. Both are therefore past the
/// check before either has inserted anything, which is exactly the window the
/// read-then-insert pair left open. Releasing the row lets the two inserts reach
/// the unique index (migration 0165) together: one inserts, the other finds the
/// reference taken and answers `acted: false`.
#[tokio::test]
async fn two_concurrent_deliveries_of_one_partial_settlement_land_once() {
    let (state, app) = test_state().await;
    ensure_webhook_secret();
    let invoice =
        seed_payable_invoice(&state, &format!("wh-race-{}", Uuid::now_v7().simple())).await;

    let mut hold = state.pool.begin().await.expect("hold transaction");
    sqlx::query("SELECT total_minor FROM invoices WHERE id = $1 FOR UPDATE")
        .bind(invoice.invoice_id)
        .fetch_one(&mut *hold)
        .await
        .expect("hold the invoice row");

    let event = settlement_event(&invoice, 2_500);
    let first = tokio::spawn({
        let app = app.clone();
        let event = event.clone();
        async move { deliver(&app, &event).await }
    });
    let second = tokio::spawn({
        let app = app.clone();
        let event = event.clone();
        async move { deliver(&app, &event).await }
    });

    wait_for_the_parked_receipt_inserts(&state.pool, 2).await;
    hold.commit().await.expect("release the invoice row");

    let first = first.await.expect("first delivery task");
    let second = second.await.expect("second delivery task");
    assert_eq!(first.status(), 200, "the winner is acknowledged");
    assert_eq!(
        second.status(),
        200,
        "the loser of the race is acknowledged too, not failed"
    );
    let first_body = body_json(first).await;
    let second_body = body_json(second).await;
    let acted = [
        first_body["acted"].as_bool(),
        second_body["acted"].as_bool(),
    ];
    assert_eq!(
        acted
            .iter()
            .filter(|applied| **applied == Some(true))
            .count(),
        1,
        "exactly one delivery applied the settlement: {acted:?}"
    );
    assert_eq!(
        acted
            .iter()
            .filter(|applied| **applied == Some(false))
            .count(),
        1,
        "the other answered the duplicate outcome: {acted:?}"
    );

    let (paid, status) = invoice_paid_and_status(&state, invoice.invoice_id).await;
    assert_eq!(paid, 2_500, "the paid total moved once for one intent");
    assert_eq!(status, "issued", "25.00 of 100.00 leaves the invoice open");

    let receipts = receipts_for(&state, invoice.invoice_id).await;
    assert_eq!(receipts.len(), 1, "one receipt for one money movement");
    assert_eq!(
        receipts[0].2.as_deref(),
        Some(invoice.intent_id.as_str()),
        "the receipt carries the intent that paid it"
    );
    assert_eq!(
        webhook_audit_rows(&state, invoice.org_id).await,
        1,
        "one settlement is audited, not two"
    );
}

/// The other side of the same index: a manual payment's reference is not a
/// provider's identity.
///
/// The none-mode provider mints `local-{invoice_id}` for every staff payment on
/// an invoice, so two instalments carry the SAME reference by design. The unique
/// index (0165) excludes `provider = 'none'` for exactly this reason, and this
/// test is what would fail if it did not: the ordinary "record the balance in
/// two receipts" flow is not a duplicate delivery and must not be refused.
///
/// No idempotency key is sent, so this is also the current rule for a manual
/// payment stated plainly: without a key two identical POSTs are two receipts.
#[tokio::test]
async fn two_staff_instalments_on_one_invoice_both_land() {
    let (state, app) = test_state().await;
    let slug = format!("wh-manual-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;
    let org_id: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");
    let invoice_id = seed_issued_invoice_for_org(&state, org_id, 10_000).await;

    for notes in ["instalment one", "instalment two"] {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/invoices/{invoice_id}/payments"),
            Some(&owner),
            Some(json!({ "amount_minor": 2_500, "notes": notes })),
        )
        .await;
        assert_eq!(
            response.status(),
            200,
            "a manual instalment is a new receipt, not a conflict: {notes}"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["payment"]["provider"], "none",
            "staff payments stay ledger-only"
        );
    }

    let (paid, status) = invoice_paid_and_status(&state, invoice_id).await;
    assert_eq!(paid, 5_000, "both instalments are on the paid total");
    assert_eq!(status, "issued", "50.00 of 100.00 leaves the invoice open");

    let receipts = receipts_for(&state, invoice_id).await;
    assert_eq!(receipts.len(), 2, "two receipts, two moments of money");
    let expected_ref = Some(format!("local-{invoice_id}"));
    assert_eq!(
        receipts
            .iter()
            .map(|(_, provider, provider_ref)| (provider.as_str(), provider_ref.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("none", expected_ref.clone()),
            ("none", expected_ref.clone()),
        ],
        "the none provider's reference is derived from the invoice, so it repeats"
    );
}

/// What the provider-reference index actually scopes, exercised at the insert
/// rather than through a route: one reference, one receipt, one invoice.
///
/// This is the test that fails without migration 0165. It drives the two
/// answers the key was chosen to give:
///
/// * a second receipt for the same Stripe intent on the SAME invoice is a typed
///   `DbError::Conflict` naming the reference (409 at the wire) — the double
///   credit the audit record describes;
/// * the same credit-note number on a SECOND invoice of the same org is
///   allowed, because a note is applied in parts across invoices, which is why
///   `invoice_id` stays in the key instead of a key on the reference alone;
/// * two `none` receipts carrying the same invoice-derived reference on one
///   invoice are allowed, which is every second staff instalment.
#[tokio::test]
async fn the_provider_reference_index_is_scoped_to_one_invoice() {
    let (state, _app) = test_state().await;
    let invoice =
        seed_payable_invoice(&state, &format!("wh-scope-{}", Uuid::now_v7().simple())).await;
    let other_invoice = seed_issued_invoice_for_org(&state, invoice.org_id, 10_000).await;

    let receipt = |invoice_id: Uuid, provider: &str, provider_ref: &str| {
        core_domain::entities::InvoicePayment {
            id: Uuid::now_v7(),
            org_id: invoice.org_id,
            invoice_id,
            amount_minor: 2_500,
            currency: "USD".into(),
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_amount_minor: None,
            provider: provider.into(),
            provider_ref: Some(provider_ref.into()),
            notes: None,
            received_at: chrono::Utc::now(),
            created_by: None,
        }
    };

    // One intent, one receipt on one invoice.
    let mut tx = state.pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, invoice.org_id)
        .await
        .expect("org bind");
    core_db::invoices::insert_payment(
        &mut *tx,
        &receipt(invoice.invoice_id, "stripe", &invoice.intent_id),
    )
    .await
    .expect("the first receipt for an intent is accepted");
    let refused = core_db::invoices::insert_payment(
        &mut *tx,
        &receipt(invoice.invoice_id, "stripe", &invoice.intent_id),
    )
    .await
    .expect_err("a second receipt for the same intent must be refused");
    match refused {
        core_db::DbError::Conflict(message) => assert!(
            message.contains(&invoice.intent_id),
            "the refusal names the reference: {message}"
        ),
        other => panic!("a duplicate provider reference is a typed conflict, got {other:?}"),
    }
    tx.rollback().await.expect("roll back the refused pair");

    // One credit note, applied to two invoices of its client: not a duplicate.
    let mut tx = state.pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, invoice.org_id)
        .await
        .expect("org bind");
    core_db::invoices::insert_payment(
        &mut *tx,
        &receipt(invoice.invoice_id, "credit_note", "CN-000042"),
    )
    .await
    .expect("a credit note lands on the first invoice");
    core_db::invoices::insert_payment(
        &mut *tx,
        &receipt(other_invoice, "credit_note", "CN-000042"),
    )
    .await
    .expect("the same note application on a second invoice is a different receipt");
    tx.rollback().await.expect("roll back the credit-note pair");

    // Two staff instalments on one invoice: the none provider's reference is
    // derived from the invoice id, so it repeats by design.
    let local_ref = format!("local-{}", invoice.invoice_id);
    let mut tx = state.pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, invoice.org_id)
        .await
        .expect("org bind");
    for _ in 0..2 {
        core_db::invoices::insert_payment(
            &mut *tx,
            &receipt(invoice.invoice_id, "none", &local_ref),
        )
        .await
        .expect("a manual instalment is not a duplicate provider reference");
    }
    tx.commit().await.expect("commit the two instalments");

    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM invoice_payments WHERE invoice_id = $1 AND provider = 'none'",
    )
    .bind(invoice.invoice_id)
    .fetch_one(&state.pool)
    .await
    .expect("receipt count");
    assert_eq!(stored, 2, "both manual receipts are recorded");
}
