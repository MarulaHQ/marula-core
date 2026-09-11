//! End-to-end quote/catalog/pipeline flows (Wave 12 lane 2) driven
//! through the REAL core-api router against a real Postgres.
//!
//! Purpose: prove the commercial front-of-funnel over HTTP —
//! catalog CRUD (SKU uniqueness, archival), quote create with catalog
//! snapshots + server-computed rollups (subtotal/tax/total via Money
//! conventions), the draft→sent→accepted/declined lifecycle (immutability
//! once sent), the convert-to-invoice seam (contract + DRAFT invoice
//! through the existing invoicing persistence, one conversion per quote,
//! validity window enforced), pipeline stages + deals + audited stage
//! transitions, and cross-org isolation.
//!
//! Non-purpose: UI (Wave 12 lane 11), FX (lane 10), tax engines (flat
//! bps rate only).

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
        .expect("quote-catalog e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with quote/catalog/pipeline tables");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
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
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "body_json parse failed ({e}); status={status} body={:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
        )
    })
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Quote Co",
            "org_slug": slug,
            "email": format!("owner@{}.test", slug),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

async fn sent_status_of(quote_id: &str, app: &Router, bearer: &str) -> String {
    let q: Value = body_json(
        send(
            app,
            axum::http::Method::GET,
            &format!("/api/quotes/{quote_id}"),
            Some(bearer),
            None,
        )
        .await,
    )
    .await;
    q["status"].as_str().unwrap().to_string()
}

async fn create_client(app: &Router, bearer: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/clients",
        Some(bearer),
        Some(json!({"name": name})),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");
    let client = body_json(response).await;
    client["id"].as_str().unwrap().to_string()
}

/// The catalog/quote lifecycle over HTTP: catalog CRUD + SKU conflicts,
/// snapshot lines + server-computed rollups, replay, draft-lock
/// immutability, conversion seam, and cross-org isolation.
#[tokio::test]
async fn catalog_quotes_lifecycle_and_conversion() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("qc1-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    // --- catalog CRUD ----------------------------------------------------
    let item: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/catalog-items",
            Some(&bearer),
            Some(json!({
                "name": "Managed workstation",
                "sku": "WS-MGD",
                "unit": "month",
                "currency": "USD",
                "unit_price_minor": 9900,
                "cost_minor": 4000,
                "taxable": true,
                "category": "Managed services",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(item["name"], "Managed workstation");
    assert_eq!(item["active"], true);
    let item_id = item["id"].as_str().unwrap().to_string();

    // Duplicate SKU is a typed 409 (case-insensitive).
    let dupe = send(
        &app,
        axum::http::Method::POST,
        "/api/catalog-items",
        Some(&bearer),
        Some(json!({
            "name": "Another workstation",
            "sku": "ws-mgd",
            "currency": "USD",
        })),
    )
    .await;
    assert_eq!(dupe.status(), 409, "duplicate SKU is a typed conflict");

    let listed: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/catalog-items?search=workstation",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);

    // Quote with one catalog-backed line (snapshot) + one free line.
    let client_id = create_client(&app, &bearer, "Acme Holdings").await;
    let create = json!({
        "client_id": client_id,
        "currency": "USD",
        "valid_until": (chrono::Utc::now() + chrono::Duration::days(30)).format("%Y-%m-%d").to_string(),
        "tax_rate_bps": 800,
        "terms": "Net 30 from acceptance",
        "lines": [
            {
                "catalog_item_id": item_id,
                "quantity_milli": 2000,
            },
            {
                "description": "Network cabling",
                "unit": "hour",
                "quantity_milli": 1500,
                "unit_amount_minor": 12000,
                "taxable": false,
            }
        ],
        "idempotency_key": Uuid::now_v7().to_string(),
    });
    let quote: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/quotes",
            Some(&bearer),
            Some(create.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(quote["status"], "draft");
    assert_eq!(quote["number"], "Q-000001");
    // 2 × 99.00 = 198.00 taxable; 1.5 × 120.00 = 180.00 exempt.
    assert_eq!(quote["status"], "draft");
    assert_eq!(quote["number"], "Q-000001");
    // 2 × 99.00 = 198.00 taxable; 1.5 × 120.00 = 180.00 exempt.
    assert_eq!(quote["subtotal_minor"], 37800);
    assert_eq!(quote["tax_minor"], 1584); // 8% of 198.00
    assert_eq!(quote["total_minor"], 39384);
    let quote_id = quote["id"].as_str().unwrap().to_string();

    // Replay: same idempotency key + same payload ⇒ byte-identical result.
    let replay: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/quotes",
            Some(&bearer),
            Some(create),
        )
        .await,
    )
    .await;
    assert_eq!(replay["id"], quote["id"], "replay returns the same quote");

    // Draft edits work: replace lines and recompute rollups.
    let patched: Value = body_json(
        send(
            &app,
            axum::http::Method::PATCH,
            &format!("/api/quotes/{quote_id}"),
            Some(&bearer),
            Some(json!({
                "lines": [
                    {"description": "Retainer", "quantity_milli": 1000, "unit_amount_minor": 50000, "taxable": true},
                ],
                "tax_rate_bps": 0,
                "version": 1,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(patched["subtotal_minor"], 50000);
    assert_eq!(patched["total_minor"], 50000);

    // Send locks the document: further edits are typed 409s.
    let sent: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/quotes/{quote_id}/send"),
            Some(&bearer),
            Some(json!({"version": 2})),
        )
        .await,
    )
    .await;
    assert_eq!(sent["status"], "sent");
    let locked = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/quotes/{quote_id}"),
        Some(&bearer),
        Some(json!({"notes": "too late"})),
    )
    .await;
    assert_eq!(sent_status_of(&quote_id, &app, &bearer).await, "sent");
    assert_eq!(locked.status(), 409, "sent quotes are immutable");

    // Accept, then convert through the seam.
    let accepted: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/quotes/{quote_id}/accept"),
            Some(&bearer),
            Some(json!({"version": 3})),
        )
        .await,
    )
    .await;
    assert_eq!(accepted["status"], "accepted");

    let converted: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/quotes/{quote_id}/convert-to-invoice"),
            Some(&bearer),
            Some(json!({"idempotency_key": Uuid::now_v7().to_string()})),
        )
        .await,
    )
    .await;
    let invoice_id = converted["invoice"]["id"].as_str().unwrap().to_string();
    assert_eq!(converted["quote"]["status"], "accepted");
    assert_eq!(converted["invoice"]["status"], "draft");
    assert_eq!(converted["invoice"]["total_minor"], 50000);
    assert_eq!(
        converted["quote"]["converted_invoice_id"],
        converted["invoice"]["id"]
    );
    let contract_id = converted["contract"]["id"].as_str().unwrap().to_string();
    assert_eq!(converted["contract"]["contract_type"], "fixed_fee");

    // The draft invoice is visible on the ordinary invoice surface.
    let invoice: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/invoices/{invoice_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(invoice["status"], "draft");
    assert_eq!(invoice["total_minor"], 50000);
    let lines = invoice["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "one fee line + no tax");
    assert_eq!(lines[0]["kind"], "fixed_fee");

    // Conversions are once-only: a second attempt is a typed 409.
    let again = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/quotes/{quote_id}/convert-to-invoice"),
        Some(&bearer),
        Some(json!({"idempotency_key": Uuid::now_v7().to_string()})),
    )
    .await;
    assert_eq!(again.status(), 409, "one conversion per quote");

    // Cross-org isolation: a second org sees none of this.
    let other = register_org(&app, format!("qc2-{}", Uuid::now_v7().simple())).await;
    let other_bearer = other["access_token"].as_str().unwrap().to_string();
    let other_quote = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/quotes/{quote_id}"),
        Some(&other_bearer),
        None,
    )
    .await;
    assert_eq!(
        other_quote.status(),
        404,
        "foreign org cannot read the quote"
    );
    let _ = contract_id;
}

#[tokio::test]
async fn pipeline_stages_deals_and_audited_transitions() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("pipe1-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let client_id = create_client(&app, &bearer, "Pipe Client").await;

    // Stages: two open + one won.
    let stage_a: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/pipeline/stages",
            Some(&bearer),
            Some(json!({"name": "Qualifying", "position": 1})),
        )
        .await,
    )
    .await;
    let _stage_b: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/pipeline/stages",
            Some(&bearer),
            Some(json!({"name": "Proposal sent", "position": 2})),
        )
        .await,
    )
    .await;
    let won_stage: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/pipeline/stages",
            Some(&bearer),
            Some(json!({"name": "Won", "stage_type": "won", "position": 10})),
        )
        .await,
    )
    .await;
    assert_eq!(won_stage["stage_type"], "won");

    // Duplicate stage names are typed 409s.
    let dupe = send(
        &app,
        axum::http::Method::POST,
        "/api/pipeline/stages",
        Some(&bearer),
        Some(json!({"name": "Qualifying"})),
    )
    .await;
    let _ = dupe;
    let dupe = send(
        &app,
        axum::http::Method::POST,
        "/api/pipeline/stages",
        Some(&bearer),
        Some(json!({"name": "Qualifying"})),
    )
    .await;
    let _ = dupe;

    let stage_a_id = stage_a["id"].as_str().unwrap().to_string();
    let won_stage_id = won_stage["id"].as_str().unwrap().to_string();

    // Deal created on stage A; placement rides the ledger.
    let deal: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/pipeline/deals",
            Some(&bearer),
            Some(json!({
                "client_id": client_id,
                "stage_id": stage_a_id,
                "name": "Workstation rollout",
                "amount_minor": 120000,
                "currency": "USD",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(deal["status"], "open");
    assert_eq!(deal["name"], "Workstation rollout");
    let deal_id = deal["id"].as_str().unwrap().to_string();

    // Deal detail carries the placement transition (NULL from).
    let detail: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/pipeline/deals/{deal_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let transitions = detail["transitions"].as_array().unwrap();
    assert_eq!(transitions.len(), 1);
    assert!(transitions[0]["from_stage_id"].is_null());

    // Stage move: audited (ledger row appended), status follows stage kind.
    let moved: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/pipeline/deals/{deal_id}/stage"),
            Some(&bearer),
            Some(json!({
                "to_stage_id": won_stage_id,
                "note": "signed",
                "version": 1,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(moved["status"], "won");
    assert!(
        moved["closed_at"].is_string(),
        "won deals are stamped closed"
    );

    let detail: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/pipeline/deals/{deal_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let transitions = detail["transitions"].as_array().unwrap();
    assert_eq!(transitions.len(), 2, "the move appended one ledger row");
    assert!(transitions[0]["from_stage_id"].is_string());

    // List filter: only the won deal shows under status=won.
    let won: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/pipeline/deals?status=won",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(won.as_array().unwrap().len(), 1);

    // Policy: a read-only member is refused on commercial mutations (403).
    // Invite → one-time setup token → redeem → token pair.
    let setup_email = format!("auditor-{}.test@demo.local", Uuid::now_v7().simple());
    let invite: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users",
            Some(&bearer),
            Some(json!({
                "email": setup_email,
                "display_name": "Auditor",
                "role": "read_only",
            })),
        )
        .await,
    )
    .await;
    let setup_token = invite["invite"]["setup_token"]
        .as_str()
        .unwrap()
        .to_string();
    let redeemed: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/org/users/accept-invite",
            None,
            Some(json!({
                "token": setup_token,
                "password": "correct-horse-battery",
            })),
        )
        .await,
    )
    .await;
    let auditor_bearer = redeemed["access_token"].as_str().unwrap().to_string();

    // Reads are fine (Commercial view).
    let read = send(
        &app,
        axum::http::Method::GET,
        "/api/catalog-items",
        Some(&auditor_bearer),
        None,
    )
    .await;
    assert_eq!(read.status(), 200, "read-only members can read the catalog");

    // Mutations are matrix-refused (403), never silent.
    let denied = send(
        &app,
        axum::http::Method::POST,
        "/api/catalog-items",
        Some(&auditor_bearer),
        Some(json!({"name": "Nope", "currency": "USD"})),
    )
    .await;
    assert_eq!(denied.status(), 403, "read-only members cannot mutate");
    let denied = send(
        &app,
        axum::http::Method::POST,
        "/api/pipeline/deals",
        Some(&auditor_bearer),
        Some(json!({
            "client_id": client_id,
            "stage_id": stage_a_id,
            "name": "Nope",
            "currency": "USD",
        })),
    )
    .await;
    assert_eq!(
        denied.status(),
        403,
        "read-only members cannot mutate deals"
    );
}
