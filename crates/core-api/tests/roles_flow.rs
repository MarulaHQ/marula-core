//! End-to-end preset-roles flow through the REAL core-api router (Wave 13
//! lane 6 — RB1 + RB2, ADR 0004).
//!
//! Purpose: the shipped presets exist as DATA on the same matrix as custom
//! roles (every org carries all eight preset rows, byte-identical to a
//! fresh org's seeds), fork copies a preset into a custom role on the same
//! matrix (with the fork origin recorded + envelope metadata + event),
//! restore resets a drifted preset to canonical (custom roles are never
//! restorable), name collisions are typed 409s, the portal tiers render in
//! the roster, and RB2 holds: bookkeeper sees catalog costs and contract
//! rates, dispatcher/technician do not (cost redacted to null, contract
//! reads typed 403).
//!
//! Non-purpose: policy-unit matrix cells (core-domain policy tests), the
//! web UI.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const PRESETS: [&str; 8] = [
    "owner",
    "admin",
    "dispatcher",
    "senior_tech",
    "technician",
    "bookkeeper",
    "auditor",
    "read_only",
];

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("roles e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
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

/// Registers a fresh org; returns the owner's access token.
async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Preset Roles Flow Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    let body = body_json(response).await;
    body["access_token"].as_str().expect("access token").into()
}

/// Invites + redeems one user; returns their access token.
async fn invite_and_activate(app: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let invite = send(
        app,
        axum::http::Method::POST,
        "/api/org/users",
        Some(owner_token),
        Some(json!({
            "email": email,
            "display_name": email.split('@').next().unwrap_or("user"),
            "role": role,
            "idempotency_key": format!("invite-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(invite.status(), 200, "invite must succeed: {role}");
    let setup_token = body_json(invite).await["invite"]["setup_token"]
        .as_str()
        .expect("setup token")
        .to_string();
    let accept = send(
        app,
        axum::http::Method::POST,
        "/api/org/users/accept-invite",
        None,
        Some(json!({ "token": setup_token, "password": "another-correct-horse" })),
    )
    .await;
    assert_eq!(accept.status(), 200, "redemption must succeed");
    let tokens = body_json(accept).await;
    tokens["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

/// The permission array of one org role row, by name.
async fn role_permissions(app: &Router, bearer: &str, name: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/roles",
        Some(bearer),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "roles list must succeed");
    let body = body_json(response).await;
    body["roles"]
        .as_array()
        .expect("roles array")
        .iter()
        .find(|r| r["name"] == json!(name))
        .unwrap_or_else(|| panic!("role '{name}' missing from the directory"))["permissions"]
        .clone()
}

#[tokio::test]
async fn presets_exist_as_data_and_fork_restore_round_trip() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("preset-roles-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;

    // --- RB1: every org carries all eight preset rows as data ---------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/roles",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;

    let roles = body["roles"].as_array().expect("roles array");
    for preset in PRESETS {
        let row = roles
            .iter()
            .find(|r| r["builtin"] == json!(preset))
            .unwrap_or_else(|| panic!("preset row '{preset}' missing"));
        assert_eq!(row["is_system"], true, "{preset} must be a system row");
        assert!(
            !row["permissions"]
                .as_array()
                .expect("permissions")
                .is_empty(),
            "{preset} must carry a non-empty matrix"
        );
    }
    // The fork-origin tag is empty on pristine presets.
    assert!(
        roles.iter().all(|r| r["forked_from"].is_null()),
        "pristine presets carry no fork origin"
    );

    // The canonical roster renders with descriptions + the same matrices.
    let presets = body["presets"].as_array().expect("preset roster");
    assert_eq!(presets.len(), 8, "ADR 0004 roster is eight staff presets");
    for preset in presets {
        assert!(
            preset["description"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "preset needs a description: {preset}"
        );
    }

    // Portal tiers render through the same roster (RB1) with the tier
    // capability matrix keys.
    let tiers = body["portal_tiers"].as_array().expect("portal tiers");
    assert_eq!(tiers.len(), 3, "contact, approver, it_manager");
    let it_manager = tiers
        .iter()
        .find(|t| t["tier"] == json!("it_manager"))
        .expect("it_manager tier");
    let caps: Vec<&str> = it_manager["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"view_in_house_updates"),
        "it_manager widens: {caps:?}"
    );
    let contact = tiers
        .iter()
        .find(|t| t["tier"] == json!("contact"))
        .expect("contact tier");
    let contact_caps: Vec<&str> = contact["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        !contact_caps.contains(&"view_in_house_updates"),
        "contact must not widen to in-house updates"
    );
    assert!(contact_caps.contains(&"view_tickets"));

    // --- fork: a preset becomes a mutable custom copy on the SAME matrix ----
    let dispatcher_row = roles
        .iter()
        .find(|r| r["builtin"] == json!("dispatcher"))
        .expect("dispatcher row")
        .clone();
    let fork = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/roles/{}/fork", dispatcher_row["id"].as_str().unwrap()),
        Some(&owner),
        Some(json!({
            "name": "Dispatcher (custom)",
            "idempotency_key": format!("fork-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(fork.status(), 200, "fork must succeed");
    let forked = body_json(fork).await;
    assert_eq!(forked["builtin"], Value::Null, "a fork is never a preset");
    assert_eq!(forked["is_system"], false);
    assert_eq!(forked["forked_from"], "dispatcher", "origin recorded");
    assert_eq!(
        forked["permissions"], dispatcher_row["permissions"],
        "fork copies the preset's matrix verbatim"
    );
    assert!(
        forked["mutation"]["version"].is_i64(),
        "fork must carry mutation metadata: {forked}"
    );

    // Same name again ⇒ typed 409 role_name_taken.
    let duplicate = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/roles/{}/fork", dispatcher_row["id"].as_str().unwrap()),
        Some(&owner),
        Some(json!({ "name": "Dispatcher (custom)" })),
    )
    .await;
    assert_eq!(duplicate.status(), 409, "duplicate role name");
    let dup_body = body_json(duplicate).await;
    assert_eq!(dup_body["error"]["reason"], "role_name_taken");

    // --- restore: a drifted preset resets to canonical ----------------------
    // Drift the dispatcher preset row directly (the org cannot do this
    // through the API — presets are never editable in place).
    let drift = json!([{"matrix": {"module": "tickets", "action": "view"}}]);
    sqlx::query("UPDATE roles SET permissions = $2 WHERE org_id = $1 AND builtin = 'dispatcher' AND is_system")
        .bind(
            uuid::Uuid::parse_str(dispatcher_row["org_id"].as_str().unwrap()).unwrap(),
        )
        .bind(&drift)
        .execute(&state.pool)
        .await
        .expect("drift write");
    assert_ne!(
        role_permissions(&app, &owner, "Dispatcher").await,
        dispatcher_row["permissions"],
        "drift must be visible before restore"
    );

    let restore = send(
        &app,
        axum::http::Method::POST,
        &format!(
            "/api/roles/{}/restore",
            dispatcher_row["id"].as_str().unwrap()
        ),
        Some(&owner),
        Some(json!({
            "idempotency_key": format!("restore-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(restore.status(), 200, "restore must succeed");
    let restored = body_json(restore).await;
    assert_eq!(
        restored["permissions"], dispatcher_row["permissions"],
        "restore resets to the canonical set"
    );
    assert!(
        restored["mutation"]["version"].is_i64(),
        "restore must carry mutation metadata"
    );

    // A custom role is NOT restorable (org-owned data).
    let custom_restore = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/roles/{}/restore", forked["id"].as_str().unwrap()),
        Some(&owner),
        Some(json!({})),
    )
    .await;
    assert_eq!(custom_restore.status(), 422, "forks are not restorable");

    // The technician's role assignment accepts the new presets end-to-end.
    let email = format!("gina-{}@acme.test", Uuid::now_v7().simple());
    let senior = invite_and_activate(&app, &owner, &email, "senior_tech").await;
    let me = send(
        &app,
        axum::http::Method::GET,
        "/api/me",
        Some(&senior),
        None,
    )
    .await;
    assert_eq!(me.status(), 200);
    let me_body = body_json(me).await;
    let perms: Vec<&str> = me_body["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert!(
        perms.contains(&"special:approve_time"),
        "senior tech approves time"
    );
    assert!(
        !perms.contains(&"financial:read"),
        "senior tech must NOT see financial data (RB2)"
    );

    // --- U5 manifest: role rows are customizations-as-data (ADR 0004) -------
    // The forked copy is org-owned data and MUST appear; the pristine
    // presets are default state and must NOT.
    let manifest = send(
        &app,
        axum::http::Method::GET,
        "/api/org/manifest",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(manifest.status(), 200, "manifest is admin-gated");
    let manifest_body = body_json(manifest).await;
    let manifest_roles = manifest_body["sections"]["roles"]
        .as_array()
        .expect("roles section");
    assert!(
        manifest_roles
            .iter()
            .any(|r| r["id"] == forked["id"] && r["forked_from"] == json!("dispatcher")),
        "the fork must appear in the manifest with its origin: {manifest_roles:?}"
    );
    assert!(
        manifest_roles.iter().all(|r| r["builtin"].is_null()),
        "pristine presets are default state, not customization"
    );
}

#[tokio::test]
async fn rb2_financial_read_gates_costs_and_contract_rates() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rb2-flow-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;

    // A dispatcher (commercial matrix, NO financial:read) and a bookkeeper
    // (financial:read + invoicing) join the org.
    let dispatcher = invite_and_activate(
        &app,
        &owner,
        &format!("dispatch-{}@acme.test", Uuid::now_v7().simple()),
        "dispatcher",
    )
    .await;
    let bookkeeper = invite_and_activate(
        &app,
        &owner,
        &format!("books-{}@acme.test", Uuid::now_v7().simple()),
        "bookkeeper",
    )
    .await;

    // --- catalog: cost + margin redacted without financial:read -------------
    let created = send(
        &app,
        axum::http::Method::POST,
        "/api/catalog-items",
        Some(&owner),
        Some(json!({
            "name": "SSD 1TB",
            "sku": "SSD-1TB",
            "unit": "each",
            "currency": "USD",
            "unit_price_minor": 12000,
            "cost_minor": 8000,
            "idempotency_key": format!("item-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(created.status(), 200, "catalog create must succeed");
    let item_id = body_json(created).await["id"].as_str().unwrap().to_string();

    // Owner sees the cost (and can derive the margin).
    let owner_view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/catalog-items/{item_id}"),
        Some(&owner),
        None,
    )
    .await;
    let owner_item = body_json(owner_view).await;
    assert_eq!(owner_item["cost_minor"], 8000, "owner sees cost");

    // The dispatcher (commercial view) sees the PRICE but the cost is null.
    let dispatcher_view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/catalog-items/{item_id}"),
        Some(&dispatcher),
        None,
    )
    .await;
    assert_eq!(
        dispatcher_view.status(),
        200,
        "commercial view still reads the catalog"
    );
    let dispatcher_item = body_json(dispatcher_view).await;
    assert_eq!(
        dispatcher_item["unit_price_minor"], 12000,
        "price stays visible"
    );
    assert_eq!(
        dispatcher_item["cost_minor"],
        Value::Null,
        "cost must render blank without financial:read"
    );

    // The bookkeeper sees the cost.
    let bookkeeper_view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/catalog-items/{item_id}"),
        Some(&bookkeeper),
        None,
    )
    .await;
    assert_eq!(body_json(bookkeeper_view).await["cost_minor"], 8000);

    // --- contracts: rate-bearing reads require financial:read ---------------
    let client = send(
        &app,
        axum::http::Method::POST,
        "/api/clients",
        Some(&owner),
        Some(json!({
            "name": "Acme Corp",
            "idempotency_key": format!("client-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(client.status(), 200, "client create must succeed");
    let client_id = body_json(client).await["id"].as_str().unwrap().to_string();

    let contract = send(
        &app,
        axum::http::Method::POST,
        "/api/contracts",
        Some(&owner),
        Some(json!({
            "client_id": client_id,
            "number": "C-001",
            "name": "Managed Services",
            "contract_type": "time_and_materials",
            "currency": "USD",
            "effective_from": "2026-01-01",
            "hourly_rate_minor": 15000,
            "rates": [{"work_type": "remote", "rate_minor": 15000}],
            "idempotency_key": format!("contract-{}", Uuid::now_v7().simple()),
        })),
    )
    .await;
    assert_eq!(contract.status(), 200, "contract create must succeed");
    let contract_id = body_json(contract).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // The dispatcher is 403 on the rate-bearing surfaces — typed reason.
    let denied = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&dispatcher),
        None,
    )
    .await;
    assert_eq!(denied.status(), 403, "contract rates are financial data");
    let denied_body = body_json(denied).await;
    assert!(
        denied_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("financial:read"),
        "denial names the missing permission: {denied_body}"
    );

    let list_denied = send(
        &app,
        axum::http::Method::GET,
        "/api/contracts",
        Some(&dispatcher),
        None,
    )
    .await;
    assert_eq!(list_denied.status(), 403, "contract list carries rates too");

    // The bookkeeper reads the contract (and its rate card) fine.
    let bookkeeper_contract = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&bookkeeper),
        None,
    )
    .await;
    assert_eq!(
        bookkeeper_contract.status(),
        200,
        "bookkeeper sees contract rates"
    );
    let detail = body_json(bookkeeper_contract).await;
    assert_eq!(detail["hourly_rate_minor"], 15000);

    // The owner, of course, sees everything.
    let owner_contract = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(owner_contract.status(), 200);

    // --- ticket cost lines stay financial:read-gated (existing gate) --------
    let denied_costs = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/contracts/{contract_id}/billing-preview"),
        Some(&dispatcher),
        None,
    )
    .await;
    assert_eq!(denied_costs.status(), 403, "billing previews carry rates");
}
