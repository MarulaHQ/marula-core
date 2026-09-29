//! Billing-lane integration tests (TASKLIST Phase 3, Wave 8b): contract
//! CRUD through the instant-first envelope plus the read-only recurring-
//! billing rule previews, driven through the REAL core-api router against
//! a real Postgres (`MARULA_TEST_DATABASE_URL` or `DATABASE_URL`).
//!
//! Pinned behavior:
//! * CRUD + typed envelope — validation failures are `validation_failed`,
//!   duplicate numbers `conflict`, missing reads `not_found`, out-of-scope
//!   mutations `rls_sparse`, stale versions `version_conflict`, and
//!   missing permission `forbidden`; creates/updates carry the IF2
//!   `mutation` metadata and replay byte-identically under the same
//!   idempotency key (IF1).
//! * Rule previews — recurring period lines, retainer base + overage,
//!   T&M work_type buckets with rate-card override, fixed-fee due dates,
//!   effective dating, and the org-wide preview aggregate.
//!
//! Non-purpose: seq contention internals (core-db), period math edge
//! cases (core-billing unit tests), invoice persistence (Wave 9 — there
//! is deliberately nothing to persist into).

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
        .expect("billing e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with billing surfaces");
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
            "org_name": "Billing Co",
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
    let email = format!("{label}@billing.test");
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

/// Seeds a board, a ticket for `client_id`, and one billable time entry.
async fn add_time_entry(
    app: &Router,
    org: &Org,
    client_id: Uuid,
    minutes: i64,
    work_type: Option<&str>,
    day: &str,
) {
    add_time_entry_billable(app, org, client_id, minutes, work_type, day, true).await;
}

async fn add_time_entry_billable(
    app: &Router,
    org: &Org,
    client_id: Uuid,
    minutes: i64,
    work_type: Option<&str>,
    day: &str,
    billable: bool,
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
            "billable": billable,
            // Wave 39 V3: a non-billable entry carries a typed reason.
            "unbillable_reason": if billable { Value::Null } else { json!("internal") },
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

// --- CRUD + envelope ---------------------------------------------------------

#[tokio::test]
async fn contract_crud_instant_first_envelope() {
    let (state, app) = test_state().await;
    let slug = format!("bill-crud-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;
    let client_id = create_client(&app, &org, "Acme Corp").await;
    let number = format!("C-{}", Uuid::now_v7().simple());

    // Validation failures come back typed, before anything persists.
    let mut body = json!({
        "client_id": client_id,
        "number": number,
        "name": "Managed services",
        "contract_type": "recurring",
        "currency": "USD",
        "effective_from": "2026-01-01",
        "billing_frequency": "monthly",
        "billing_anchor": "2026-01-15",
        "rates": [],
        "fees": [],
    });
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );

    body["amount_minor"] = json!(200_000);
    body["effective_until"] = json!("2025-01-01"); // inverted window
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );

    body.as_object_mut().unwrap().remove("effective_until");
    body["rates"] = json!([{ "work_type": "onsite", "rate_minor": 1 }]); // rates only on T&M
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );
    body["rates"] = json!([]);

    // Currency validation rides the same envelope.
    body["currency"] = json!("usdollars");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );
    body["currency"] = json!("USD");

    // Create with an idempotency key; response carries IF2 metadata.
    body["idempotency_key"] = json!("billing-create-key-1");
    let first = create_contract(&app, &org, client_id, body.clone()).await;
    assert_eq!(first["mutation"]["seq"], json!(1));
    assert_eq!(first["mutation"]["version"], json!(1));
    assert_eq!(
        first["mutation"]["idempotency_key"],
        json!("billing-create-key-1")
    );
    assert_eq!(first["contract_type"], json!("recurring"));
    assert_eq!(first["status"], json!("draft"));
    let contract_id: Uuid = first["id"].as_str().unwrap().parse().unwrap();

    // Same key + same payload replays byte-identically (never re-applies).
    let replay_response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(replay_response.status(), 200);
    let replay = body_json(replay_response).await;
    assert_eq!(replay, first);

    // Same key + different payload is a typed 409.
    let mut tampered = body.clone();
    tampered["name"] = json!("Different name");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(tampered),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(
        error_reason(&body_json(response).await),
        "idempotency_key_reused"
    );

    // Duplicate contract number is a typed conflict (org-scoped unique).
    let mut dup = body.clone();
    dup.as_object_mut().unwrap().remove("idempotency_key");
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&org.token),
        Some(dup),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(error_reason(&body_json(response).await), "conflict");

    // List + get.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts?client_id={client_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let list = body_json(response).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let detail = body_json(response).await;
    assert_eq!(detail["id"], first["id"]);
    assert_eq!(detail["rates"].as_array().unwrap().len(), 0);
    assert_eq!(detail["fees"].as_array().unwrap().len(), 0);

    // PATCH with a stale version is a typed 409 carrying expected/provided.
    let patch = json!({
        "status": "active",
        "version": 99,
    });
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        Some(patch.clone()),
    )
    .await;
    assert_eq!(response.status(), 409);
    let conflict = body_json(response).await;
    assert_eq!(error_reason(&conflict), "version_conflict");
    assert_eq!(conflict["error"]["expected"], json!(1));
    assert_eq!(conflict["error"]["provided"], json!(99));

    // PATCH with the current version applies; seq bumps to 2.
    let mut patch = patch.clone();
    patch["version"] = json!(1);
    patch["idempotency_key"] = json!("billing-patch-key-1");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        Some(patch.clone()),
    )
    .await;
    assert_eq!(response.status(), 200);
    let patched = body_json(response).await;
    assert_eq!(patched["status"], json!("active"));
    assert_eq!(patched["mutation"]["seq"], json!(2));
    assert_eq!(patched["mutation"]["before"]["status"], json!("draft"));
    assert_eq!(patched["mutation"]["after"]["status"], json!("active"));

    // PATCH replay is byte-identical.
    let replay_response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        Some(patch),
    )
    .await;
    assert_eq!(body_json(replay_response).await, patched);

    // A roleless user gets the typed forbidden envelope on the write path.
    let roleless = add_roleless_user(&app, &state, &org, "roleless").await;
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&roleless),
        Some(json!({
            "client_id": client_id,
            "number": format!("C-{}", Uuid::now_v7().simple()),
            "name": "Nope",
            "contract_type": "recurring",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 1,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-15",
        })),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(error_reason(&body_json(response).await), "forbidden");

    // Cross-org: a second org cannot read (not_found) nor mutate
    // (rls_sparse) the first org's contract.
    let other_slug = format!("bill-other-{}", Uuid::now_v7().simple());
    let other = register_org(&app, other_slug).await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&other.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(error_reason(&body_json(response).await), "not_found");
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/contracts/{contract_id}"),
        Some(&other.token),
        Some(json!({ "name": "stolen" })),
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(error_reason(&body_json(response).await), "rls_sparse");

    // DELETE returns the removed aggregate; it is then gone.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let removed = body_json(response).await;
    assert_eq!(removed["id"], json!(contract_id));
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(error_reason(&body_json(response).await), "not_found");
}

// --- rule previews -----------------------------------------------------------

#[tokio::test]
async fn billing_rule_previews_all_types() {
    let (_state, app) = test_state().await;
    let slug = format!("bill-prev-{}", Uuid::now_v7().simple());
    let org = register_org(&app, slug).await;

    // Window validation rides the typed envelope.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/billing/preview?to=2026-03-01",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        error_reason(&body_json(response).await),
        "validation_failed"
    );
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/billing/preview?from=2026-03-01&to=2026-02-01",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 422);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/billing/preview?from=2025-01-01&to=2027-01-01",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 422);

    // Recurring: three period starts inside Jan 1 .. Mar 20 (anchor
    // Jan 15: Jan/Feb/Mar 15).
    let client_id = create_client(&app, &org, "Recurring Client").await;
    let recurring = create_contract(
        &app,
        &org,
        client_id,
        json!({
            "number": format!("C-R-{}", Uuid::now_v7().simple()),
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
    let recurring_id: Uuid = recurring["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{recurring_id}/billing-preview?from=2026-01-01&to=2026-03-20"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let evaluation = body_json(response).await;
    assert_eq!(evaluation["lines"].as_array().unwrap().len(), 3);
    assert_eq!(evaluation["total_minor"], json!(600_000));
    assert_eq!(evaluation["lines"][0]["kind"], json!("recurring_base"));
    assert_eq!(evaluation["lines"][0]["period_start"], json!("2026-01-15"));

    // Retainer: base lines plus overage beyond the included block.
    let retainer_client = create_client(&app, &org, "Retainer Client").await;
    let retainer = create_contract(
        &app,
        &org,
        retainer_client,
        json!({
            "number": format!("C-T-{}", Uuid::now_v7().simple()),
            "name": "Support retainer",
            "contract_type": "retainer",
            "status": "active",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 100_000,
            "included_minutes": 600,
            "overage_rate_minor": 15_000,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-15",
        }),
    )
    .await;
    let retainer_id: Uuid = retainer["id"].as_str().unwrap().parse().unwrap();
    // Period 1 (Jan 15 .. Feb 15): 750 min consumed -> 150 min overage.
    add_time_entry(&app, &org, retainer_client, 400, None, "2026-01-20").await;
    add_time_entry(&app, &org, retainer_client, 350, None, "2026-02-02").await;
    // Period 2 (Feb 15 .. Mar 15): 100 min -> under the block.
    add_time_entry(&app, &org, retainer_client, 100, None, "2026-02-20").await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{retainer_id}/billing-preview?from=2026-01-01&to=2026-03-20"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let evaluation = body_json(response).await;
    let lines = evaluation["lines"].as_array().unwrap();
    let base: Vec<&Value> = lines
        .iter()
        .filter(|l| l["kind"] == json!("retainer_base"))
        .collect();
    let overage: Vec<&Value> = lines
        .iter()
        .filter(|l| l["kind"] == json!("retainer_overage"))
        .collect();
    assert_eq!(base.len(), 3, "one base line per started period");
    assert_eq!(overage.len(), 1, "only period 1 exceeds the block");
    assert_eq!(overage[0]["quantity_minutes"], json!(150));
    assert_eq!(overage[0]["amount_minor"], json!(37_500));
    assert_eq!(evaluation["total_minor"], json!(337_500));

    // T&M: rate-card override per work_type, default rate otherwise.
    let tm_client = create_client(&app, &org, "T&M Client").await;
    let tm = create_contract(
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
            "rates": [{ "work_type": "onsite", "rate_minor": 18_000 }],
        }),
    )
    .await;
    let tm_id: Uuid = tm["id"].as_str().unwrap().parse().unwrap();
    add_time_entry(&app, &org, tm_client, 60, Some("remote"), "2026-01-05").await;
    add_time_entry(&app, &org, tm_client, 30, Some("onsite"), "2026-01-06").await;
    add_time_entry(&app, &org, tm_client, 90, None, "2026-01-07").await;
    // Non-billable labor never reaches the engine.
    add_time_entry_billable(
        &app,
        &org,
        tm_client,
        120,
        Some("remote"),
        "2026-01-08",
        false,
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{tm_id}/billing-preview?from=2026-01-01&to=2026-01-31"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let evaluation = body_json(response).await;
    let lines = evaluation["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3, "onsite + remote + unclassified buckets");
    assert_eq!(lines[0]["unit_amount_minor"], json!(18_000));
    assert_eq!(lines[0]["amount_minor"], json!(9_000));
    assert_eq!(lines[1]["quantity_minutes"], json!(60));
    assert_eq!(lines[1]["unit_amount_minor"], json!(10_000));
    assert_eq!(lines[2]["quantity_minutes"], json!(90));
    assert_eq!(evaluation["total_minor"], json!(34_000));

    // Effective dating excludes labor before effective_from.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{tm_id}/billing-preview?from=2026-02-01&to=2026-02-28"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(body_json(response).await["total_minor"], json!(0));

    // Fixed fee: only dues inside the window, ordered by due date.
    let fee_client = create_client(&app, &org, "Project Client").await;
    let fee = create_contract(
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
                { "label": "Assessment", "amount_minor": 250_000, "due_on": "2026-01-10" },
                { "label": "Migration", "amount_minor": 500_000, "due_on": "2026-03-01" },
                { "label": "Too early", "amount_minor": 999, "due_on": "2025-12-01" }
            ],
        }),
    )
    .await;
    let fee_id: Uuid = fee["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{fee_id}/billing-preview?from=2026-01-01&to=2026-03-31"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let evaluation = body_json(response).await;
    let lines = evaluation["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["description"], json!("Assessment"));
    assert_eq!(lines[1]["description"], json!("Migration"));
    assert_eq!(evaluation["total_minor"], json!(750_000));

    // Cancelled contracts never evaluate; the org-wide preview sums the
    // rest.
    create_contract(
        &app,
        &org,
        client_id,
        json!({
            "number": format!("C-X-{}", Uuid::now_v7().simple()),
            "name": "Cancelled deal",
            "contract_type": "recurring",
            "status": "cancelled",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "amount_minor": 999_999,
            "billing_frequency": "monthly",
            "billing_anchor": "2026-01-15",
        }),
    )
    .await;
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/billing/preview?from=2026-01-01&to=2026-03-20",
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let preview = body_json(response).await;
    let evaluations = preview["evaluations"].as_array().unwrap();
    assert_eq!(evaluations.len(), 4, "cancelled contract excluded");
    // 600_000 + 337_500 + 34_000 + 750_000.
    assert_eq!(preview["total_minor"], json!(1_721_500));
}
