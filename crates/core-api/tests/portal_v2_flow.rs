//! Client-portal v2 end-to-end flow through the REAL core-api router
//! (the private build tracker Phase 3 "Portal v2").
//!
//! Purpose: pin the three v2 self-service blocks over HTTP on the REAL
//! router:
//! * invoices — client-scoped list/detail/document, ledger-only payment
//!   through the none-mode `PaymentProvider` seam (partial → settle →
//!   terminal), IF1 replay/version-conflict, cross-client 404, portal
//!   provenance in the audit log + write trail;
//! * KB — the effective client-facing gate (published AND article flag
//!   AND category flag) end-to-end, incl. FTS search scoping;
//! * appointment requests — slots from published availability, intent
//!   creation gated on live availability, staff approve (books the
//!   calendar)/decline/cancel with forward-only status.
//!
//! Non-purpose: SQL-layer behavior (core-db suites), domain-rule unit
//! tests (core-domain), staff invoicing internals (core-billing
//! invoice_flow), UI rendering.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use axum::Router;
use chrono::Datelike;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

#[path = "support/caller.rs"]
mod caller;

use caller::fresh_caller_ip;

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("portal v2 e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with portal v2 surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
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
    // A caller of its own per request: the portal sign-in gate is durable and
    // per-IP, and a headerless `oneshot` would put every case in this binary in
    // one bucket (Wave 54 follow-up F7).
    let request = fresh_caller_ip().stamp(request);
    app.clone().oneshot(request).await.unwrap()
}

async fn body_json(response: axum::response::Response) -> Value {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => panic!(
            "non-JSON body ({status}): {} :: {e}",
            String::from_utf8_lossy(&bytes)
        ),
    }
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).to_string()
}

fn error_reason(body: &Value) -> String {
    body["error"]["reason"]
        .as_str()
        .unwrap_or_default()
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
            "org_name": "Portal V2 Co",
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
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn invite_portal_user(
    app: &Router,
    staff_token: &str,
    client_id: Uuid,
    email: &str,
    role: &str,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/clients/{client_id}/portal-users"),
        Some(staff_token),
        Some(json!({
            "email": email,
            "display_name": "Portal Person",
            "password": "portal-passphrase-1",
            "role": role,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "invite must succeed");
    body_json(response).await
}

async fn portal_login(app: &Router, email: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/portal/login",
        None,
        Some(json!({"email": email, "password": "portal-passphrase-1"})),
    )
    .await;
    assert_eq!(response.status(), 200, "portal login for {email}");
    body_json(response).await
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool and logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@portalv2.test");
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

// --- invoices: view + pay ------------------------------------------------------

#[tokio::test]
async fn portal_invoices_view_pay_document_and_provenance() {
    let (state, app) = test_state().await;
    let slug = format!("portal-inv-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_a = create_client(&app, &org, "Client A").await;
    let client_b = create_client(&app, &org, "Client B").await;
    create_contract_for(&app, &org, client_a).await;
    create_contract_for(&app, &org, client_b).await;

    // One invoice per client over the same window.
    let run = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/billing/invoice-runs",
            Some(&org.token),
            Some(json!({
                "from": "2026-01-01",
                "to": "2026-01-31",
                "idempotency_key": format!("portal-run-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(run["run"]["invoice_count"], json!(2), "both clients billed");
    let invoice_a_id: Uuid = run["invoices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["invoice"]["client_id"] == json!(client_a))
        .expect("client A invoice")["invoice"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // --- portal identities ----------------------------------------------------
    // Portal login resolves the org from the EMAIL alone, so test emails
    // must be unique per run: the suite never resets the shared schema,
    // and a reused email would silently bind to an earlier run's org.
    let email_a = format!("payer-{}@a.test", Uuid::now_v7().simple());
    let email_b = format!("payer-{}@b.test", Uuid::now_v7().simple());
    invite_portal_user(&app, &org.token, client_a, &email_a, "contact").await;
    invite_portal_user(&app, &org.token, client_b, &email_b, "contact").await;
    let portal_token = portal_login(&app, &email_a).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let login_b = portal_login(&app, &email_b).await;

    // JWT family separation still holds for the NEW surface: staff tokens
    // cannot read portal invoices, portal tokens cannot read staff ones.
    let staff_on_portal = send(
        &app,
        axum::http::Method::GET,
        "/api/portal/invoices",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(staff_on_portal.status(), 401);
    let portal_on_staff = send(
        &app,
        axum::http::Method::GET,
        "/api/invoices",
        Some(&portal_token),
        None,
    )
    .await;
    assert_eq!(portal_on_staff.status(), 401);

    // --- list + detail ---------------------------------------------------------
    let list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/invoices",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    let listed = list.as_array().unwrap();
    assert_eq!(listed.len(), 1, "client A sees exactly its own invoice");
    assert_eq!(listed[0]["status"], json!("issued"));
    assert_eq!(listed[0]["outstanding_minor"], json!(200_000));
    assert!(
        listed[0].get("dunning_state").is_none(),
        "dunning internals never reach the portal view"
    );

    let detail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/invoices/{invoice_a_id}"),
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["invoice"]["id"], json!(invoice_a_id));
    assert_eq!(detail["lines"].as_array().unwrap().len(), 1);
    assert_eq!(
        detail["lines"][0]["kind"],
        json!("recurring_base"),
        "one recurring_base line for the single period"
    );
    assert_eq!(detail["payments"].as_array().unwrap().len(), 0);

    // The document endpoint renders the SAME print-ready HTML staff get.
    let document = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/invoices/{invoice_a_id}/document"),
        Some(&portal_token),
        None,
    )
    .await;
    assert_eq!(document.status(), 200);
    assert!(
        document.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"),
        "the document endpoint serves HTML"
    );
    let html = body_text(document).await;
    assert!(html.contains("Invoice"), "document contains the heading");
    assert!(
        html.contains("Client A"),
        "document names the billed client"
    );

    // Cross-client reads are NotFound — both directions.
    let token_b = login_b["access_token"].as_str().unwrap().to_string();
    let cross = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/portal/invoices/{invoice_a_id}"),
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(
        cross.status(),
        404,
        "client B cannot read client A's invoice"
    );
    let b_list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/invoices",
            Some(&token_b),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        b_list.as_array().unwrap().len(),
        1,
        "B's list shows exactly B's own invoice"
    );
    assert_ne!(
        b_list[0]["id"],
        json!(invoice_a_id),
        "B's invoice is never A's document"
    );

    // --- pay: ledger-only, partial then settle ---------------------------------
    let pay_body = json!({
        "amount_minor": 50_000,
        "notes": "Check #101 arrived",
        "idempotency_key": format!("portal-pay-{}", Uuid::now_v7().simple()),
    });
    let pay_response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&portal_token),
        Some(pay_body.clone()),
    )
    .await;
    assert_eq!(pay_response.status(), 200, "partial payment succeeds");
    let paid = body_json(pay_response).await;
    assert_eq!(paid["payment"]["amount_minor"], json!(50_000));
    assert_eq!(
        paid["payment"]["provider"],
        json!("none"),
        "none-mode provider: ledger-only receipt"
    );
    assert!(
        paid["payment"]["created_by"].is_null(),
        "portal identities are not staff rows"
    );
    assert_eq!(paid["invoice"]["total_paid_minor"], json!(50_000));
    assert_eq!(paid["invoice"]["outstanding_minor"], json!(150_000));
    assert_eq!(
        paid["mutation"]["seq"],
        json!(2),
        "the run allocated version 1"
    );
    let first_payment_id: Uuid = paid["payment"]["id"].as_str().unwrap().parse().unwrap();

    // Same key + same body replays byte-identically.
    let replay = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/invoices/{invoice_a_id}/pay"),
            Some(&portal_token),
            Some(pay_body),
        )
        .await,
    )
    .await;
    assert_eq!(replay, paid, "idempotent replay is byte-identical");

    // Overpayment is a typed conflict.
    let overpay = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&portal_token),
        Some(json!({"amount_minor": 999_000})),
    )
    .await;
    assert_eq!(overpay.status(), 409);

    // Version conflict rides the typed envelope.
    let stale = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&portal_token),
        Some(json!({"amount_minor": 1000, "version": 1})),
    )
    .await;
    assert_eq!(stale.status(), 409);
    assert_eq!(error_reason(&body_json(stale).await), "version_conflict");

    // Settle the remainder.
    let settle = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&portal_token),
        Some(json!({
            "amount_minor": 150_000,
            "idempotency_key": format!("settle-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(settle.status(), 200);
    let settled = body_json(settle).await;
    assert_eq!(settled["invoice"]["status"], json!("paid"));
    assert_eq!(settled["invoice"]["outstanding_minor"], json!(0));

    // A paid invoice is no longer payable.
    let again = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&portal_token),
        Some(json!({"amount_minor": 1})),
    )
    .await;
    assert_eq!(again.status(), 409);

    // Cross-client pay is 404 INSIDE the mutation (rls_sparse), never a
    // double-charge on someone else's document.
    let cross_pay = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/invoices/{invoice_a_id}/pay"),
        Some(&token_b),
        Some(json!({"amount_minor": 1})),
    )
    .await;
    assert_eq!(cross_pay.status(), 404);

    // Provenance: audit rows + write trail carry the PORTAL actor kind,
    // and the receipt keeps created_by NULL.
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT actor_kind FROM audit_events WHERE action = 'portal.invoice_payment'",
    )
    .fetch_all(&state.pool)
    .await
    .expect("audit rows");
    assert!(!audit_rows.is_empty());
    assert_eq!(audit_rows[0].0, "portal_contact");
    let trail: Vec<(String, String)> = sqlx::query_as(
        "SELECT actor_kind, op FROM entity_writes WHERE entity = 'invoice' \
         AND op = 'portal_payment_record'",
    )
    .fetch_all(&state.pool)
    .await
    .expect("write trail rows");
    assert_eq!(trail[0].0, "portal_contact");
    let payment_row: (Option<Uuid>, String) =
        sqlx::query_as("SELECT created_by, provider FROM invoice_payments WHERE id = $1")
            .bind(first_payment_id)
            .fetch_one(&state.pool)
            .await
            .expect("payment row");
    assert_eq!(payment_row.0, None, "portal identities are not staff rows");
    assert_eq!(payment_row.1, "none");
}

async fn create_contract_for(app: &Router, org: &Org, client_id: Uuid) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(json!({
            "client_id": client_id,
            "number": format!("C-{}", Uuid::now_v7().simple()),
            "name": "Managed services",
            "contract_type": "recurring",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 200_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-01",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "contract create must succeed");
}

// --- KB self-service -------------------------------------------------------------

#[tokio::test]
async fn portal_kb_honors_the_effective_client_gate() {
    let (_state, app) = test_state().await;
    let slug = format!("portal-kb-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_a = create_client(&app, &org, "Reader Co").await;

    // Staff-side content: two categories (one client-visible), five
    // articles spanning every gate combination.
    let visible_cat: Uuid = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/kb/categories",
            Some(&org.token),
            Some(json!({"name": "How-to guides", "client_visible": true})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let hidden_cat: Uuid = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/kb/categories",
            Some(&org.token),
            Some(json!({"name": "Internal runbooks", "client_visible": false})),
        )
        .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    async fn create_article(app: &Router, token: &str, body: Value) -> Value {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/kb/articles",
            Some(token),
            Some(body),
        )
        .await;
        assert_eq!(response.status(), 200, "article create");
        body_json(response).await
    }

    let visible_published = create_article(
        &app,
        &org.token,
        json!({
            "category_id": visible_cat,
            "title": "How to restart the backup agent",
            "body": "Open the dashboard, choose Services, press Restart. The whole restart procedure takes two minutes.",
            "status": "published",
            "client_visible": true,
        }),
    )
    .await;
    let gated_out = [
        // Flagged article in a HIDDEN category (category half fails).
        create_article(
            &app,
            &org.token,
            json!({
                "category_id": hidden_cat,
                "title": "Internal failover steps",
                "body": "failover steps for staff only",
                "status": "published",
                "client_visible": true,
            }),
        )
        .await,
        // Published but article flag off.
        create_article(
            &app,
            &org.token,
            json!({
                "category_id": visible_cat,
                "title": "Costing notes",
                "body": "internal costing",
                "status": "published",
                "client_visible": false,
            }),
        )
        .await,
        // Draft with the flag on (status half fails).
        create_article(
            &app,
            &org.token,
            json!({
                "category_id": visible_cat,
                "title": "Draft migration guide",
                "body": "not ready",
                "status": "draft",
                "client_visible": true,
            }),
        )
        .await,
        create_article(
            &app,
            &org.token,
            json!({
                "category_id": visible_cat,
                "title": "Old onboarding guide",
                "body": "archived content",
                "status": "archived",
                "client_visible": true,
            }),
        )
        .await,
    ];

    // --- portal identity --------------------------------------------------------
    let email_a = format!("reader-{}@a.test", Uuid::now_v7().simple());
    invite_portal_user(&app, &org.token, client_a, &email_a, "contact").await;
    let portal_token = portal_login(&app, &email_a).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Categories: only the client-visible one exists.
    let categories = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/kb/categories",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    let category_ids: Vec<&str> = categories
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(category_ids, vec![visible_cat.to_string().as_str()]);

    // Articles: exactly the effective gate — published + article flag +
    // category flag.
    let articles = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/kb/articles",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    let article_ids: Vec<&str> = articles
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(article_ids, vec![visible_published["id"].as_str().unwrap()]);

    // Hidden-by-gate rows resolve to 404, indistinguishable from absent.
    for hidden in &gated_out {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/kb/articles/{}", hidden["id"].as_str().unwrap()),
            Some(&portal_token),
            None,
        )
        .await;
        assert_eq!(response.status(), 404, "gated-out articles are 404s");
    }
    let found = send(
        &app,
        axum::http::Method::GET,
        &format!(
            "/api/portal/kb/articles/{}",
            visible_published["id"].as_str().unwrap()
        ),
        Some(&portal_token),
        None,
    )
    .await;
    assert_eq!(found.status(), 200);

    // Search rides FTS and respects the same gate: the flagged published
    // article hits; every gated-out sibling never does.
    let search = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/kb/search?q=restart%20backup",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    let hits: Vec<&str> = search
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["article"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(hits, vec![visible_published["id"].as_str().unwrap()]);
    let no_leak = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/kb/search?q=failover",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        no_leak.as_array().unwrap().len(),
        0,
        "hidden article never surfaces"
    );
}

/// Unified-search scoping check (Wave 15 lane 4, carried from the Wave 13
/// docs follow-ups row): search_index is org-scoped only (no client
/// scoping columns exist), so the guarantee for client contexts is the
/// STRUCTURAL one — portal principals must never reach the staff unified
/// /api/search at all, and the portal KB search re-resolves from
/// kb_articles with its own published + client-visible gate.
#[tokio::test]
async fn portal_principals_never_reach_the_unified_search_seam() {
    let (_state, app) = test_state().await;
    let slug = format!("portal-scope-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id: Uuid = create_client(&app, &org, "Scoped Co").await;

    let email = format!("reader-scope-{}@example.test", Uuid::now_v7().simple());
    send(
        &app,
        axum::http::Method::POST,
        &format!("/api/clients/{client_id}/portal-users"),
        Some(&org.token),
        Some(json!({
            "email": email,
            "display_name": "Portal Reader",
            "password": "portal-passphrase-1",
            "role": "contact",
        })),
    )
    .await;
    let portal_token: String = portal_login(&app, &email).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/search?q=scoped",
        Some(&portal_token),
        None,
    )
    .await;
    assert!(
        response.status() == 401 || response.status() == 403,
        "portal principal must not search staff documents/org-wide index"
    );

    // Staff fencing too: a staff token still works — the failure is about
    // the PRINCIPAL kind, not the route.
    let staff = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/search?q=scoped",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(staff.status(), 200);
}

// --- appointment requests -------------------------------------------------------

fn tomorrow() -> chrono::NaiveDate {
    chrono::Utc::now().date_naive().succ_opt().unwrap()
}

#[tokio::test]
async fn portal_appointment_requests_create_decide_cancel() {
    let (state, app) = test_state().await;
    let slug = format!("portal-appt-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_a = create_client(&app, &org, "Sched Co").await;
    let client_b = create_client(&app, &org, "Other Co").await;

    // The owner acts as the staff member whose calendar is requested.
    let staff_user: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE org_id = $1 ORDER BY email LIMIT 1")
            .bind(org.org_id)
            .fetch_one(&state.pool)
            .await
            .expect("staff user");
    let day = tomorrow();
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/availability",
        Some(&org.token),
        Some(json!({
            "user_id": staff_user,
            "weekday": day.weekday().num_days_from_monday() as i16,
            "start_local": "00:00",
            "end_local": "23:59",
            "timezone": "UTC",
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "seed availability");

    let email_a = format!("planner-{}@a.test", Uuid::now_v7().simple());
    invite_portal_user(&app, &org.token, client_a, &email_a, "contact").await;
    let portal_token = portal_login(&app, &email_a).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // --- slots: published availability minus bookings --------------------------
    let slots = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/schedule/slots?from={day}&to={day}"),
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        slots.as_array().unwrap().len(),
        1,
        "one staff member publishes slots"
    );
    assert!(!slots[0]["slots"].as_array().unwrap().is_empty());
    let listed_user: Uuid = slots[0]["user_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(listed_user, staff_user);

    // --- create: inside availability wins, outside availability is 422 --------
    let create_key = format!("req-{}", Uuid::now_v7().simple());
    let create_body = json!({
        "user_id": staff_user,
        "title": "Quarterly security review",
        "starts_at": format!("{day}T10:00:00Z"),
        "ends_at": format!("{day}T11:00:00Z"),
        "kind": "remote",
        "idempotency_key": create_key,
    });
    let created = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/appointment-requests",
            Some(&portal_token),
            Some(create_body.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(created["status"], json!("pending"));
    assert_eq!(created["kind"], json!("remote"));
    assert_eq!(
        created["mutation"]["idempotency_key"],
        json!(create_key.clone())
    );
    let request_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    // Replay is byte-identical (the intent never double-books).
    let replay = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/appointment-requests",
            Some(&portal_token),
            Some(create_body),
        )
        .await,
    )
    .await;
    assert_eq!(replay, created);

    // A request for a slot that is already BOOKED is refused: staff books
    // a slot directly, then the portal ask for that exact time is 422.
    let booked_start = format!("{day}T14:00:00Z");
    let booked_end = format!("{day}T15:00:00Z");
    let staff_booking = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&org.token),
        Some(json!({
            "user_id": staff_user,
            "title": "Existing on-site visit",
            "starts_at": booked_start,
            "ends_at": booked_end,
        })),
    )
    .await;
    assert_eq!(staff_booking.status(), 201, "staff books a slot directly");
    let clash = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/appointment-requests",
        Some(&portal_token),
        Some(json!({
            "user_id": staff_user,
            "title": "Overlapping ask",
            "starts_at": booked_start,
            "ends_at": booked_end,
        })),
    )
    .await;
    assert_eq!(
        clash.status(),
        422,
        "booked time is outside free availability"
    );

    // A slot in the past is refused.
    let past = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/appointment-requests",
        Some(&portal_token),
        Some(json!({
            "user_id": staff_user,
            "title": "Time travel",
            "starts_at": "2026-01-01T10:00:00Z",
            "ends_at": "2026-01-01T11:00:00Z",
        })),
    )
    .await;
    assert_eq!(past.status(), 422);

    // --- staff queue + approve ---------------------------------------------------
    let pending = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/appointment-requests?status=pending",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(pending.as_array().unwrap().len(), 1, "only the pending one");

    // An unprivileged staff member cannot decide.
    let roleless = add_roleless_user(&app, &state, &org, "norole").await;
    let forbidden = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointment-requests/{request_id}/approve"),
        Some(&roleless),
        Some(json!({})),
    )
    .await;
    assert_eq!(forbidden.status(), 403);

    let approve = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointment-requests/{request_id}/approve"),
        Some(&org.token),
        Some(json!({"decision_note": "Confirmed for the morning."})),
    )
    .await;
    assert_eq!(approve.status(), 200);
    let approved = body_json(approve).await;
    assert_eq!(approved["request"]["status"], json!("approved"));
    assert_eq!(approved["appointment"]["status"], json!("scheduled"));
    assert_eq!(
        approved["appointment"]["client_id"],
        json!(client_a),
        "the booking is client-bound"
    );

    // Deciding again is refused once decided.
    let re_approve = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointment-requests/{request_id}/approve"),
        Some(&org.token),
        Some(json!({})),
    )
    .await;
    assert_eq!(re_approve.status(), 409);

    // The approval re-validates against the live calendar: the slot is
    // now a real booking, so a NEW portal request overlapping it is 422.
    let overlap = send(
        &app,
        axum::http::Method::POST,
        "/api/portal/appointment-requests",
        Some(&portal_token),
        Some(json!({
            "user_id": staff_user,
            "title": "Overlaps the approved booking",
            "starts_at": format!("{day}T10:30:00Z"),
            "ends_at": format!("{day}T11:30:00Z"),
        })),
    )
    .await;
    assert_eq!(
        overlap.status(),
        422,
        "approval re-validation mirrors the create gate"
    );

    // --- decline + cancel ---------------------------------------------------------
    let second_key = format!("req-{}", Uuid::now_v7().simple());
    let second = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/appointment-requests",
            Some(&portal_token),
            Some(json!({
                "user_id": staff_user,
                "title": "Evening walkthrough",
                "starts_at": format!("{day}T18:00:00Z"),
                "ends_at": format!("{day}T19:00:00Z"),
                "idempotency_key": second_key,
            })),
        )
        .await,
    )
    .await;
    let second_id: Uuid = second["status"]
        .as_str()
        .map(|_| second["id"].as_str().unwrap())
        .unwrap()
        .parse()
        .unwrap();
    let declined = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointment-requests/{second_id}/decline"),
        Some(&org.token),
        Some(json!({"decision_note": "That hour is reserved for maintenance."})),
    )
    .await;
    assert_eq!(declined.status(), 200);
    assert_eq!(
        body_json(declined).await["request"]["status"],
        json!("declined")
    );

    // The client sees decisions and may cancel a still-pending request.
    body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/portal/appointment-requests",
            Some(&portal_token),
            Some(json!({
                "user_id": staff_user,
                "title": "Late-day sync",
                "starts_at": format!("{day}T20:00:00Z"),
                "ends_at": format!("{day}T21:00:00Z"),
                "idempotency_key": format!("req-{}", Uuid::now_v7().simple()),
            })),
        )
        .await,
    )
    .await;
    let mine = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/appointment-requests",
            Some(&portal_token),
            None,
        )
        .await,
    )
    .await;
    let mine = mine.as_array().unwrap();
    assert!(mine.iter().any(|r| r["status"] == json!("approved")));
    assert!(mine.iter().any(|r| r["status"] == json!("declined")));
    let pending_id = mine
        .iter()
        .find(|r| r["status"] == json!("pending"))
        .expect("the pending third request")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/portal/appointment-requests/{pending_id}/cancel"),
        Some(&portal_token),
        Some(json!({})),
    )
    .await;
    assert_eq!(cancelled.status(), 200);
    assert_eq!(body_json(cancelled).await["status"], json!("cancelled"));

    // Cross-client isolation: client B sees nothing of client A's requests.
    let email_b = format!("other-{}@b.test", Uuid::now_v7().simple());
    invite_portal_user(&app, &org.token, client_b, &email_b, "contact").await;
    let b_token = portal_login(&app, &email_b).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let b_list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/appointment-requests",
            Some(&b_token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(b_list.as_array().unwrap().len(), 0);

    // Events: every request lifecycle type fires; the approval ALSO
    // emits the standard appointment.created through the normal
    // booking path.
    let request_types: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT event_type FROM outbox_events WHERE entity = 'appointment_request'",
    )
    .fetch_all(&state.pool)
    .await
    .expect("outbox rows");
    let types: Vec<&str> = request_types.iter().map(|(t,)| t.as_str()).collect();
    for expected in [
        "appointment_request.created",
        "appointment_request.approved",
        "appointment_request.declined",
        "appointment_request.cancelled",
    ] {
        assert!(types.contains(&expected), "missing event {expected}");
    }
    let booking_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM outbox_events WHERE entity = 'appointment' AND event_type = 'appointment.created'",
    )
    .fetch_all(&state.pool)
    .await
    .expect("outbox rows");
    assert!(!booking_events.is_empty());
}

// --- quotes: scoped view + the client's approve/decline legs (Wave 34) --------

/// Portal quote approval end-to-end: scoped list/detail over the REAL
/// router, draft invisibility, cross-client 404, the SAME sent →
/// accepted | declined lifecycle legs as the staff flow (typed events in
/// the outbox), IF1 replay, and the post-decision 409s.
#[tokio::test]
async fn portal_quotes_scoped_view_and_decide_lifecycle() {
    let (state, app) = test_state().await;
    let slug = format!("portal-quote-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_a = create_client(&app, &org, "Quote Client A").await;
    let client_b = create_client(&app, &org, "Quote Client B").await;

    // One sent quote for A, one DRAFT for A; nothing for B.
    let create_sent = json!({
        "client_id": client_a,
        "currency": "USD",
        "valid_until": tomorrow().to_string(),
        "notes": "Discovery sprint proposal",
        "tax_rate_bps": 800,
        "lines": [
            {"description": "Discovery workshop", "quantity_milli": 2000, "unit_amount_minor": 9900, "taxable": true},
        ],
        "idempotency_key": Uuid::now_v7().to_string(),
    });
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/quotes",
            Some(&org.token),
            Some(create_sent.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(created["status"], "draft");
    assert_eq!(created["subtotal_minor"], 19800);
    assert_eq!(created["tax_minor"], 1584);
    assert_eq!(created["total_minor"], 21384);
    let quote_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    let sent: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/quotes/{quote_id}/send"),
            Some(&org.token),
            Some(json!({})),
        )
        .await,
    )
    .await;
    assert_eq!(sent["status"], "sent");

    let _draft: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/quotes",
            Some(&org.token),
            Some(json!({
                "client_id": client_a,
                "currency": "USD",
                "valid_until": tomorrow().to_string(),
                "lines": [{"description": "Not yet sent", "quantity_milli": 1000, "unit_amount_minor": 10000, "taxable": false}],
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;

    // --- portal identities (unique emails: login resolves the org from
    // the email alone; the suite never resets the shared schema).
    let email_a = format!("quote-{}@a.test", Uuid::now_v7().simple());
    let email_b = format!("quote-{}@b.test", Uuid::now_v7().simple());
    invite_portal_user(&app, &org.token, client_a, &email_a, "contact").await;
    invite_portal_user(&app, &org.token, client_b, &email_b, "contact").await;
    let token_a = portal_login(&app, &email_a).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let token_b = portal_login(&app, &email_b).await["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Staff tokens never pass the portal gate; portal tokens never pass
    // the staff one.
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/quotes",
            Some(&org.token),
            None
        )
        .await
        .status(),
        401
    );
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            "/api/quotes",
            Some(&token_a),
            None
        )
        .await
        .status(),
        401
    );

    // Scoped list: A sees exactly the sent quote (draft hidden); B sees none.
    let list_a = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/quotes",
            Some(&token_a),
            None,
        )
        .await,
    )
    .await;
    let rows_a = list_a.as_array().unwrap();
    assert_eq!(rows_a.len(), 1, "drafts are staff-internal");
    assert_eq!(rows_a[0]["status"], "sent");
    assert_eq!(rows_a[0]["total_minor"], 21384);
    assert!(
        rows_a[0].get("converted_invoice_id").is_none(),
        "conversion seam ids stay staff-internal"
    );
    let list_b = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/portal/quotes",
            Some(&token_b),
            None,
        )
        .await,
    )
    .await;
    assert!(
        list_b.as_array().unwrap().is_empty(),
        "client B sees nothing"
    );

    // Detail: lines + rollup money math visible to the scoped client.
    let detail = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/quotes/{quote_id}"),
            Some(&token_a),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["lines"].as_array().unwrap().len(), 1);
    assert_eq!(detail["lines"][0]["description"], "Discovery workshop");

    // Cross-client reads and decisions are bare 404s (no existence leak).
    assert_eq!(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/portal/quotes/{quote_id}"),
            Some(&token_b),
            None
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/quotes/{quote_id}/approve"),
            Some(&token_b),
            Some(json!({})),
        )
        .await
        .status(),
        404
    );

    // B cannot see it, so B cannot break it — approve from A, with a
    // replay under the same key.
    let key = format!("portal-approve-{}", Uuid::now_v7().simple());
    let approve = json!({"idempotency_key": key});
    let approved: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/quotes/{quote_id}/approve"),
            Some(&token_a),
            Some(approve.clone()),
        )
        .await,
    )
    .await;
    assert_eq!(approved["status"], "accepted");
    assert!(approved["decided_at"].is_string());
    let replayed: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/quotes/{quote_id}/approve"),
            Some(&token_a),
            Some(approve),
        )
        .await,
    )
    .await;
    assert_eq!(
        replayed["status"], "accepted",
        "IF1 replay is byte-faithful"
    );

    // A decided quote is decided: the decline leg is a typed 409.
    assert_eq!(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/quotes/{quote_id}/decline"),
            Some(&token_a),
            Some(json!({"reason": "too late"})),
        )
        .await
        .status(),
        409
    );

    // Decline leg on a fresh quote: sent → declined with the reason in
    // the audit trail, same lifecycle legs.
    let declined_quote: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/quotes",
            Some(&org.token),
            Some(json!({
                "client_id": client_a,
                "currency": "USD",
                "valid_until": tomorrow().to_string(),
                "lines": [{"description": "Revision B", "quantity_milli": 1000, "unit_amount_minor": 5000, "taxable": false}],
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await,
    )
    .await;
    let declined_id: Uuid = declined_quote["id"].as_str().unwrap().parse().unwrap();
    body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/quotes/{declined_id}/send"),
            Some(&org.token),
            Some(json!({})),
        )
        .await,
    )
    .await;
    let declined: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/portal/quotes/{declined_id}/decline"),
            Some(&token_a),
            Some(json!({"reason": "budget cycle"})),
        )
        .await,
    )
    .await;
    assert_eq!(declined["status"], "declined");
    let audit_action: (String,) = sqlx::query_as(
        "SELECT action FROM audit_events WHERE entity_id = $1 AND action = 'portal.quote_decline'",
    )
    .bind(declined_id.to_string())
    .fetch_one(&state.pool)
    .await
    .expect("audit row");
    assert_eq!(audit_action.0, "portal.quote_decline");

    // The SAME typed lifecycle legs as the staff flow, straight from the
    // portal actions.
    let outbox: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM outbox_events WHERE entity = 'quote' AND entity_id = ANY($1)",
    )
    .bind(vec![quote_id.to_string(), declined_id.to_string()])
    .fetch_all(&state.pool)
    .await
    .expect("outbox rows");
    let types: Vec<&str> = outbox.iter().map(|(t,)| t.as_str()).collect();
    assert!(types.contains(&"quote.accepted"), "missing quote.accepted");
    assert!(types.contains(&"quote.declined"), "missing quote.declined");
}
