//! End-to-end per-client workflow overrides flow through the REAL core-api
//! router (Wave 12 lane 5, migration 0063).
//!
//! Purpose: the four contract surfaces over HTTP —
//! 1. Resolution precedence: the resolved document defaults to the org's
//!    constructs (default board, entry status with its semantic category,
//!    default SLA policy, org toggle defaults), a client `set` wins over the
//!    org default, an explicit `disabled` denies an org-enabled default, and
//!    clear reverts to inherit.
//! 2. Typed-registry / manifest validation: unknown keys, mistyped payloads,
//!    out-of-org references, non-entry statuses, cross-board statuses, and
//!    disallowed disables are typed 422s; valid rows export verbatim through
//!    GET /api/org/manifest (U5 coverage) in deterministic order.
//! 3. Authorization: mutations are ManageBoards (owner/admin/dispatcher),
//!    reads need client view; a foreign org's client is 404 on reads and
//!    rls_sparse inside mutations.
//! 4. Audit + instant-first envelope: mutations carry `mutation` metadata,
//!    replay by idempotency key is byte-identical, and audit rows carry the
//!    `client.workflow_override_*` actions.
//!
//! Non-purpose: SQL-layer behavior in isolation (covered against the shared
//! core-db test DB by the registry unit tests + RLS coverage gate), the event
//! contract (core-events conformance), UI.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("overrides e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    config.database_url = admin_url.clone();
    let reporting_url = admin_url;
    AppState {
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

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Overrides Co",
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

async fn create_client(app: &Router, token: &str, name: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/clients",
        Some(token),
        Some(json!({ "name": name, "idempotency_key": Uuid::now_v7().to_string() })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");
    body_json(response).await
}

async fn default_board_id(app: &Router, token: &str) -> String {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/boards/nav",
        Some(token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    body.as_array()
        .unwrap()
        .iter()
        .find(|b| b["is_default"] == true)
        .expect("default board")["id"]
        .as_str()
        .expect("board id")
        .into()
}

/// Creates a second user with the builtin technician role and logs in.

// --- tests -----------------------------------------------------------------------

#[tokio::test]
async fn registry_serves_the_closed_set_and_defaults_resolve_to_org_constructs() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("ovr-reg-{}", Uuid::now_v7().simple())).await;
    let client = create_client(app, &token, "Registry Client").await;
    let client_id = client["id"].as_str().unwrap();

    // The registry is served with its five closed declarations.
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/workflow-overrides/registry",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let reg = body_json(response).await;
    let keys: Vec<&str> = reg["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        vec![
            "workflow.intake_board",
            "workflow.intake_status",
            "sla.policy",
            "notifications.enabled",
            "automations.enabled",
        ],
        "the registry is the closed declaration set"
    );

    // A fresh client resolves EVERY construct to the org defaults.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let doc = body_json(response).await;
    assert_eq!(doc["client_id"], json!(client_id));
    let entries = doc["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 5);

    let board = &entries[0];
    assert_eq!(board["key"], "workflow.intake_board");
    assert_eq!(board["source"], "org_default");
    assert_eq!(board["resolution"]["state"], "set");
    assert_eq!(
        board["resolution"]["value"],
        json!(default_board_id(app, &token).await),
        "org default = the org's default board"
    );
    assert_eq!(board["detail"]["kind"], "board");
    assert_eq!(board["org_default"], board["resolution"]["value"]);

    let status = &entries[1];
    assert_eq!(status["key"], "workflow.intake_status");
    assert_eq!(status["source"], "org_default");
    // Semantic-category binding rides the resolution.
    assert_eq!(board["detail"]["kind"], "board");
    let status_detail = &status["detail"];
    assert_eq!(status_detail["kind"], "status");
    let status_categories = ["open", "in_progress", "waiting"];
    assert!(
        status_categories.contains(&status_detail["category"].as_str().unwrap()),
        "intake status carries its semantic category"
    );

    let sla = &entries[2];
    assert_eq!(sla["key"], "sla.policy");
    assert_eq!(
        sla["source"], "org_default",
        "fresh org seeds a default policy"
    );
    assert_eq!(sla["detail"]["kind"], "sla_policy");

    for (idx, key) in [
        (3usize, "notifications.enabled"),
        (4, "automations.enabled"),
    ] {
        let toggle = &entries[idx];
        assert_eq!(toggle["key"], key);
        assert_eq!(toggle["source"], "org_default");
        assert_eq!(toggle["resolution"]["state"], "set");
        assert_eq!(toggle["resolution"]["value"], json!(true));
        assert_eq!(toggle["detail"]["kind"], "toggle");
        assert_eq!(toggle["detail"]["enabled"], json!(true));
        assert_eq!(toggle["org_default"], json!(true), "org default rides");
        assert_eq!(toggle["allow_disable"], json!(true));
    }
}

#[tokio::test]
async fn client_override_wins_disable_denies_and_clear_reverts() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("ovr-prec-{}", Uuid::now_v7().simple())).await;
    let client_id = create_client(app, &token, "Precision Client").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Second board to override intake with.
    let board2 = {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/boards",
            Some(&token),
            Some(json!({ "name": "Client Intake", "is_default": false })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let board2_id = board2["id"].as_str().unwrap().to_string();

    // SET: the client override replaces the org default board.
    let key = Uuid::now_v7().to_string();
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/workflow.intake_board"),
        Some(&token),
        Some(json!({
            "state": "set",
            "value": board2_id,
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "set must succeed");
    let body = body_json(response).await;
    assert_eq!(body["source"], "client_override");
    assert_eq!(body["resolution"]["value"], json!(board2_id));
    assert_eq!(body["mutation"]["seq"], json!(1));
    assert_eq!(body["mutation"]["version"], json!(1));
    let version = body["mutation"]["version"].as_i64().unwrap();

    // If-Match: a stale version is a typed 409.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/workflow.intake_board"),
        Some(&token),
        Some(json!({
            "state": "set",
            "value": board2_id,
            "version": version + 5,
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "stale version must conflict");
    let conflict = body_json(response).await;
    assert_eq!(conflict["error"]["reason"], "version_conflict");

    // Replay with the SAME key + payload returns the identical result.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/workflow.intake_board"),
        Some(&token),
        Some(json!({
            "state": "set",
            "value": board2_id,
            "idempotency_key": key,
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["mutation"]["seq"], json!(1));

    // TOGGLE: explicit set(false) — deny semantics at the client scope.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/notifications.enabled"),
        Some(&token),
        Some(json!({ "state": "set", "value": false })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["source"], "client_override");
    assert_eq!(body["resolution"]["value"], json!(false));
    assert_eq!(body["detail"]["enabled"], json!(false));

    // DISABLE: an explicit deny of the org DEFAULT SLA policy — deny wins.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/sla.policy"),
        Some(&token),
        Some(json!({ "state": "disabled" })),
    )
    .await;
    assert_eq!(response.status(), 200, "sla.policy allows explicit disable");
    let body = body_json(response).await;
    assert_eq!(body["resolution"]["state"], "disabled");
    assert_eq!(body["source"], "client_override");
    assert!(
        body["detail"].is_null(),
        "disabled resolves to no construct"
    );

    // The document reflects all three precedence outcomes.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        Some(&token),
        None,
    )
    .await;
    let doc = body_json(response).await;
    let entries = doc["entries"].as_array().unwrap();
    let board_entry = entries
        .iter()
        .find(|e| e["key"] == "workflow.intake_board")
        .unwrap();
    assert_eq!(board_entry["source"], "client_override");
    assert_eq!(
        board_entry["org_default"],
        json!(default_board_id(app, &token).await)
    );
    let sla_entry = entries.iter().find(|e| e["key"] == "sla.policy").unwrap();
    assert_eq!(sla_entry["resolution"]["state"], "disabled");
    assert!(
        !sla_entry["org_default"].is_null(),
        "the org default is still exported for the audit view"
    );
    // The toggle carries its raw provenance row.
    assert_eq!(
        entries
            .iter()
            .find(|e| e["key"] == "notifications.enabled")
            .unwrap()["client_override"]["state"],
        "set"
    );

    // CLEAR: the board reverts to the org default; a second clear is a no-op.
    let response = send(
        app,
        axum::http::Method::DELETE,
        &format!("/api/clients/{client_id}/workflow-overrides/workflow.intake_board"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_json(response).await;
    assert_eq!(body["source"], "org_default");
    assert_eq!(
        body["resolution"]["value"],
        json!(default_board_id(app, &token).await)
    );

    let response = send(
        app,
        axum::http::Method::DELETE,
        &format!("/api/clients/{client_id}/workflow-overrides/workflow.intake_board"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "clearing an absent row is a no-op");
    assert_eq!(body_json(response).await["source"], "org_default");
}

#[tokio::test]
async fn typed_registry_validation_rejects_bad_payloads_before_persistence() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("ovr-valid-{}", Uuid::now_v7().simple())).await;
    let client_id = create_client(app, &token, "Validated Client").await["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Another board + its statuses for cross-board/membership probes.
    let board2 = {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/boards",
            Some(&token),
            Some(json!({ "name": "Validation Board", "is_default": false })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    let board2_id = board2["id"].as_str().unwrap().to_string();

    let put = |uri_key: String, body: Value| {
        let token = token.clone();
        let client_id = client_id.clone();
        async move {
            let response = send(
                app,
                axum::http::Method::PUT,
                &format!("/api/clients/{client_id}/workflow-overrides/{uri_key}"),
                Some(&token),
                Some(body),
            )
            .await;
            let status = response.status();
            let body = body_json(response).await;
            (status, body)
        }
    };

    // Unknown key: the registry is a closed set.
    let (status, body) = put(
        "nope.key".to_string(),
        json!({ "state": "set", "value": "x" }),
    )
    .await;
    assert_eq!(status, 422, "unknown keys are rejected, never stored");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not a registered workflow override"),
        "typed reason: {}",
        body["error"]["message"]
    );

    // Mistyped payload: a boolean for a board ref.
    let (status, body) = put(
        "workflow.intake_board".to_string(),
        json!({ "state": "set", "value": true }),
    )
    .await;
    assert_eq!(status, 422);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("board_ref"),
        "kind-named reason: {}",
        body["error"]["message"]
    );

    // Out-of-org reference: a random uuid names no board here.
    let (status, _) = put(
        "workflow.intake_board".to_string(),
        json!({ "state": "set", "value": Uuid::now_v7().to_string() }),
    )
    .await;
    assert_eq!(status, 422, "references must exist in the caller's org");

    // Disallowed disable: the intake board cannot be denied.
    let (status, body) = put(
        "workflow.intake_board".to_string(),
        json!({ "state": "disabled" }),
    )
    .await;
    assert_eq!(status, 422);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot be disabled"),
        "clear-to-inherit reference constructs refuse disable: {}",
        body["error"]["message"]
    );

    // A status from ANOTHER board than the effective intake board.
    let statuses2 = send(
        app,
        axum::http::Method::GET,
        &format!("/api/boards/{board2_id}/statuses"),
        Some(&token),
        None,
    )
    .await;
    let statuses2 = body_json(statuses2).await;
    let board2_status = statuses2.as_array().unwrap()[0]["id"].as_str().unwrap();
    let (status, body) = put(
        "workflow.intake_status".to_string(),
        json!({ "state": "set", "value": board2_status }),
    )
    .await;
    assert_eq!(status, 422, "cross-board statuses are rejected");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different board"),
        "membership reason: {}",
        body["error"]["message"]
    );

    // A CLOSED-category status is not an entry status.
    let default_board_id = default_board_id(app, &token).await;
    let statuses1 = send(
        app,
        axum::http::Method::GET,
        &format!("/api/boards/{default_board_id}/statuses"),
        Some(&token),
        None,
    )
    .await;
    let statuses1 = body_json(statuses1).await;
    let closed = statuses1
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "closed")
        .expect("closed status");
    let (status, body) = put(
        "workflow.intake_status".to_string(),
        json!({ "state": "set", "value": closed["id"] }),
    )
    .await;
    assert_eq!(status, 422, "non-entry statuses are rejected");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not an entry status"),
        "entry reason: {}",
        body["error"]["message"]
    );

    // An OPEN status on the default board IS valid — the happy path.
    let open = statuses1
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["category"] == "open" && s["is_entry"] == true)
        .expect("entry open status");
    let (status, body) = put(
        "workflow.intake_status".to_string(),
        json!({ "state": "set", "value": open["id"] }),
    )
    .await;
    assert_eq!(status, 200, "valid entry status on the effective board");
    assert_eq!(body["detail"]["category"], "open");
    assert_eq!(body["source"], "client_override");

    // Nothing above persisted: only the last write exists.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        Some(&token),
        None,
    )
    .await;
    let doc = body_json(response).await;
    let board_entry = doc["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"] == "workflow.intake_board")
        .unwrap();
    assert_eq!(
        board_entry["source"], "org_default",
        "rejected payloads never persisted"
    );
}

#[tokio::test]
async fn authorization_gates_the_surface_and_scopes_to_the_client() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let app = &app;
    let token_a = register_org(app, &format!("ovr-auth-{}", Uuid::now_v7().simple())).await;
    let client_id = create_client(app, &token_a, "Scoped Client").await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Unauthenticated reads are 401.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), 401);

    // Mutations need ManageBoards: a technician is 403 even though they can
    // view the client.
    let tech_email = format!("tech-{}@override.test", Uuid::now_v7().simple());
    // Resolve org A's id through the authed me endpoint — never guess the
    // latest org row (tests share the database and run concurrently).
    let org_id: Uuid = {
        let me = send(
            app,
            axum::http::Method::GET,
            "/api/me",
            Some(&token_a),
            None,
        )
        .await;
        assert_eq!(me.status(), 200);
        let body = body_json(me).await;
        Uuid::parse_str(body["org_id"].as_str().unwrap()).expect("org id")
    };
    let tech_token = {
        let password_hash = core_api::auth::password::hash("tech-password-1234").unwrap();
        let mut tx = state.pool.begin().await.expect("tx");
        core_db::set_org(&mut tx, org_id).await.expect("set_org");
        let tech_role: Uuid =
            sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
                .bind(org_id)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        let user_id = Uuid::now_v7();
        core_db::users::create_user(
            &mut *tx,
            user_id,
            org_id,
            &tech_email,
            "Tech",
            &password_hash,
            "UTC",
        )
        .await
        .expect("user insert");
        core_db::users::assign_role(&mut *tx, user_id, tech_role)
            .await
            .expect("role assign");
        tx.commit().await.expect("commit");
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({ "email": tech_email, "password": "tech-password-1234" })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await["access_token"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Read: technicians hold Clients:View.
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        Some(&tech_token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "technician may read the resolution");

    // Write: technicians do NOT hold ManageBoards.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/notifications.enabled"),
        Some(&tech_token),
        Some(json!({ "state": "set", "value": false })),
    )
    .await;
    assert_eq!(response.status(), 403, "mutation needs ManageBoards");

    // Cross-org: org B's owner cannot see or mutate org A's client.
    let token_b = register_org(app, &format!("ovr-other-{}", Uuid::now_v7().simple())).await;
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/clients/{client_id}/workflow-overrides"),
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org reads are not_found");
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/notifications.enabled"),
        Some(&token_b),
        Some(json!({ "state": "set", "value": false })),
    )
    .await;
    assert_eq!(response.status(), 404, "cross-org mutations are rls_sparse");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "rls_sparse");

    // Unknown override key never depends on the client.
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_id}/workflow-overrides/bogus.key"),
        Some(&token_a),
        Some(json!({ "state": "set", "value": true })),
    )
    .await;
    assert_eq!(response.status(), 422);
}

#[tokio::test]
async fn manifest_covers_and_audits_the_overrides() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let app = &app;
    let token = register_org(app, &format!("ovr-manifest-{}", Uuid::now_v7().simple())).await;
    let client_a = create_client(app, &token, "Alpha Client").await;
    let client_b = create_client(app, &token, "Beta Client").await;
    let client_a_id = client_a["id"].as_str().unwrap().to_string();

    // Alpha: a value override + a toggle override.
    let board2 = {
        let response = send(
            app,
            axum::http::Method::POST,
            "/api/boards",
            Some(&token),
            Some(json!({ "name": "Manifest Board", "is_default": false })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await
    };
    for (key, body) in [
        (
            "workflow.intake_board",
            json!({ "state": "set", "value": board2["id"] }),
        ),
        (
            "automations.enabled",
            json!({ "state": "set", "value": false }),
        ),
    ] {
        let response = send(
            app,
            axum::http::Method::PUT,
            &format!("/api/clients/{client_a_id}/workflow-overrides/{key}"),
            Some(&token),
            Some(body),
        )
        .await;
        assert_eq!(response.status(), 200, "set {key}");
    }

    // Beta: an explicit disable.
    let client_b_id = client_b["id"].as_str().unwrap().to_string();
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/clients/{client_b_id}/workflow-overrides/notifications.enabled"),
        Some(&token),
        Some(json!({ "state": "disabled" })),
    )
    .await;
    assert_eq!(response.status(), 200);

    // The manifest exports rows verbatim, (client_name, key) ordered.
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/org/manifest",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let manifest = body_json(response).await;
    let overrides = manifest["sections"]["client_workflow_overrides"]
        .as_array()
        .expect("override section is always serialized");
    assert_eq!(overrides.len(), 3, "2 alpha rows + 1 beta row");
    assert_eq!(overrides[0]["client_name"], "Alpha Client");
    assert_eq!(overrides[0]["key"], "automations.enabled");
    assert_eq!(overrides[0]["state"], "set");
    assert_eq!(overrides[0]["value"], json!(false));
    assert_eq!(overrides[1]["key"], "workflow.intake_board");
    assert_eq!(overrides[1]["value"], json!(board2["id"].as_str().unwrap()));
    assert_eq!(overrides[2]["client_name"], "Beta Client");
    assert_eq!(overrides[2]["state"], "disabled");
    assert!(
        overrides[2]["value"].is_null(),
        "disabled rows carry no value"
    );

    // Manifest validation: every exported row's key/state pair must be a
    // legal registry state (round-trip the rows through the typed parser).
    for row in overrides.iter() {
        let decl = registry::declaration(row["key"].as_str().unwrap())
            .expect("manifest rows carry registered keys");
        let value_opt: Option<&Value> = if row["value"].is_null() {
            None
        } else {
            Some(&row["value"])
        };
        let parsed = registry::parse_state(decl, row["state"].as_str().unwrap(), value_opt);
        assert!(
            parsed.is_ok(),
            "manifest row must re-parse against the typed registry: {row}"
        );
    }

    // Clearing removes the manifest row.
    let response = send(
        app,
        axum::http::Method::DELETE,
        &format!("/api/clients/{client_a_id}/workflow-overrides/automations.enabled"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/org/manifest",
        Some(&token),
        None,
    )
    .await;
    let manifest = body_json(response).await;
    let overrides = manifest["sections"]["client_workflow_overrides"]
        .as_array()
        .unwrap();
    assert_eq!(overrides.len(), 2, "the cleared row is gone");

    // Audit: the audit log carries the override actions.
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/audit-events?action=client.workflow_override",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let audit = body_json(response).await;
    let actions: Vec<String> = audit["events"]
        .as_array()
        .expect("audit page carries events")
        .iter()
        .filter_map(|e| e["action"].as_str().map(str::to_string))
        .collect();
    assert!(
        actions.contains(&"client.workflow_override_set".to_string()),
        "set actions are audited: {actions:?}"
    );
    assert!(
        actions.contains(&"client.workflow_override_cleared".to_string()),
        "clears are audited"
    );
}

use core_domain::workflow_overrides as registry;
