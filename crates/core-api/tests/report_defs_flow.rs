//! Report-definitions CRUD flow (TASKLIST R3) through the REAL core-api
//! router.
//!
//! Purpose: the stored-report-spec contract over HTTP —
//! 1. Admin CRUD round-trip riding the instant-first write path: create
//!    returns read-your-write `mutation` metadata (seq/version), updates
//!    enforce the version gate (typed `version_conflict`), deletes echo
//!    the removed definition.
//! 2. Registry validation: specs outside the typed catalog are rejected
//!    with the typed `validation_failed` reason — unknown views, hostile
//!    identifier-shaped columns, type-incompatible aggregates, tenancy
//!    dimensions, duplicate names.
//! 3. Idempotency: same key + same body replays the SAME response; only
//!    one row lands.
//! 4. RBAC: technicians (reporting:view only) can read org-published
//!    definitions but never create; custom-role authors (reporting
//!    create/edit/delete, non-admin) are forced onto `self` scope, cannot
//!    see or touch each other's private definitions, while admins see all.
//! 5. Dual-mount parity: /api/v1 (canonical) and /api (legacy alias) hit
//!    the same handlers.
//!
//! Database selection follows manifest_flow's contract:
//! MERIDIAN_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

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
        .expect("report_defs e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([21; 32]);
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

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        &[],
        Some(json!({
            "org_name": "Reports Co",
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

/// Creates a user with the builtin technician role (reporting:view only).
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    create_user_with_role(
        state,
        app,
        org_id,
        email,
        "SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'",
    )
    .await
}

/// Creates a user with a CUSTOM role carrying the reporting write matrix
/// (create/edit/delete) but NO admin standing — the non-admin author case
/// the builtin roles don't cover (builtin write grants stop at owner/
/// admin).
async fn provision_report_author(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    email: &str,
) -> String {
    create_user_with_role(
        state,
        app,
        org_id,
        email,
        "SELECT id FROM roles WHERE org_id = $1 AND builtin IS NULL AND name = 'Report Author'",
    )
    .await
}

async fn create_user_with_role(
    state: &AppState,
    app: &Router,
    org_id: Uuid,
    email: &str,
    role_query: &str,
) -> String {
    let password_hash = core_api::auth::password::hash("author-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let role_id: Uuid = sqlx::query_scalar(role_query)
        .bind(org_id)
        .fetch_one(&mut *tx)
        .await
        .expect("role exists");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Author User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, role_id)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": "author-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "user login must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Inserts the custom "Report Author" role for an org. The permission
/// encoding is the Permission enum's serde shape.
async fn seed_report_author_role(state: &AppState, org_id: Uuid) {
    let permissions = json!([
        {"matrix": {"module": "reporting", "action": "view"}},
        {"matrix": {"module": "reporting", "action": "create"}},
        {"matrix": {"module": "reporting", "action": "edit"}},
        {"matrix": {"module": "reporting", "action": "delete"}},
    ]);
    sqlx::query(
        "INSERT INTO roles (id, org_id, name, builtin, permissions, is_system) \
         VALUES ($1, $2, 'Report Author', NULL, $3, false)",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(&permissions)
    .execute(&state.pool)
    .await
    .expect("custom role insert");
}

fn create_body(name: &str, scope: &str) -> Value {
    json!({
        "name": name,
        "description": "tickets grouped by status",
        "spec": {
            "view": "tickets",
            "filters": [{"column": "priority", "op": "not_eq", "value": {"text": "p4"}}],
            "group_by": ["status_name"],
            "measures": [{"agg": "count"}, {"agg": "min", "column": "created_at"}],
            "viz": "bar"
        },
        "scope": scope,
        "idempotency_key": format!("def-create-{name}")
    })
}

#[tokio::test]
async fn admin_crud_round_trip_rides_the_instant_path() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("repdefs-crud-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // CREATE: read-your-write metadata + echoed spec.
    let created = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &bearer,
            Some(create_body("Open by status", "org")),
        )
        .await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, 200, "create must succeed");
        assert_eq!(body["name"], "Open by status");
        assert_eq!(body["scope"], "org", "admins publish org-wide");
        assert_eq!(body["spec"]["view"], "tickets");
        assert_eq!(
            body["spec"]["measures"][1],
            json!({"agg": "min", "column": "created_at"}),
            "the typed spec round-trips"
        );
        assert_eq!(body["mutation"]["seq"], 1);
        assert_eq!(body["mutation"]["version"], 1);
        assert_eq!(body["mutation"]["before"], Value::Null);
        assert_eq!(
            body["mutation"]["idempotency_key"], "def-create-Open by status",
            "the claimed key is echoed"
        );
        body["id"].as_str().unwrap().to_string()
    };

    // LIST + GET.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-definitions",
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let arr = body_json(response).await["definitions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"].as_str().unwrap(), created);

    // UPDATE with the CURRENT version: seq advances.
    {
        let response = send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/v1/report-definitions/{created}"),
            &bearer,
            Some(json!({
                "name": "Open by status v2",
                "description": null,
                "spec": {
                    "view": "tickets",
                    "filters": [],
                    "group_by": ["priority"],
                    "measures": [{"agg": "count"}],
                    "viz": "pie"
                },
                "scope": "org",
                "version": 1
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "update with current version lands");
        let body = body_json(response).await;
        assert_eq!(body["name"], "Open by status v2");
        assert_eq!(body["description"], Value::Null, "null clears");
        assert_eq!(body["mutation"]["seq"], 2);
        assert_eq!(
            body["mutation"]["before"]["name"], "Open by status",
            "the trail carries the prior snapshot"
        );
    }

    // UPDATE with a STALE version: typed 409 version_conflict.
    {
        let response = send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/v1/report-definitions/{created}"),
            &bearer,
            Some(json!({
                "name": "should not land",
                "spec": {
                    "view": "tickets", "filters": [], "group_by": [],
                    "measures": [{"agg": "count"}], "viz": "table"
                },
                "version": 1
            })),
        )
        .await;
        assert_eq!(response.status(), 409, "stale version must conflict");
        let body = body_json(response).await;
        assert_eq!(body["error"]["reason"], "version_conflict");
        assert_eq!(body["error"]["expected"], 2);
        assert_eq!(body["error"]["provided"], 1);
    }

    // Duplicate name: typed 409 conflict.
    {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &bearer,
            Some(json!({
                "name": "Open by status v2",
                "spec": {
                    "view": "tickets", "filters": [], "group_by": [],
                    "measures": [{"agg": "count"}], "viz": "table"
                }
            })),
        )
        .await;
        assert_eq!(response.status(), 409, "per-org name uniqueness");
        assert_eq!(body_json(response).await["error"]["reason"], "conflict");
    }

    // DELETE echoes the removed definition; the read then 404s.
    {
        let response = send(
            &app,
            axum::http::Method::DELETE,
            &format!("/api/v1/report-definitions/{created}"),
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200, "delete must succeed");
        let body = body_json(response).await;
        assert_eq!(
            body["name"], "Open by status v2",
            "the echo is the last state"
        );
        assert_eq!(body["mutation"]["seq"], 3);
        assert_eq!(body["mutation"]["after"], Value::Null);
    }
    {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/report-definitions/{created}"),
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 404);
        assert_eq!(body_json(response).await["error"]["reason"], "not_found");
    }
}

#[tokio::test]
async fn registry_violations_return_typed_validation_reasons() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("repdefs-registry-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // Specs that cannot even DESERIALIZE (unknown slugs) are rejected by
    // the request layer with 422 before a handler runs.
    let bad_shapes: Vec<(Value, &str)> = vec![
        (
            json!({"view": "ops.v1_tickets", "measures": [{"agg": "count"}], "viz": "bar"}),
            "unknown view slug",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "count"}], "viz": "hologram"}),
            "unknown viz",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "median"}], "viz": "bar"}),
            "unknown aggregate",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar",
                   "filters": [{"column": "priority", "op": "matches", "value": {"text": "x"}}]}),
            "unknown operator",
        ),
    ];
    for (spec, why) in bad_shapes {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &bearer,
            Some(json!({
                "name": format!("bad-{why}"),
                "spec": spec,
            })),
        )
        .await;
        assert_eq!(response.status(), 422, "{why} must be rejected");
    }

    // Specs that deserialize but violate the TYPED REGISTRY come back as
    // the machine-readable `validation_failed` envelope reason.
    let bad_specs: Vec<(Value, &str)> = vec![
        (
            json!({"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar",
                   "filters": [{"column": "user_id IS NULL OR 1=1 --", "op": "eq",
                                "value": {"text": "x"}}]}),
            "identifier-shaped filter column",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "sum", "column": "priority"}],
                   "viz": "bar"}),
            "aggregate over a non-numeric column",
        ),
        (
            json!({"view": "tickets", "measures": [], "viz": "bar"}),
            "empty measures",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar",
                   "group_by": ["org_id"]}),
            "tenancy dimension",
        ),
        (
            json!({"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar",
                   "filters": [{"column": "client_id", "op": "eq", "value": {"bool": true}}]}),
            "operand type mismatch",
        ),
    ];
    for (spec, why) in bad_specs {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &bearer,
            Some(json!({
                "name": format!("bad-{why}"),
                "spec": spec,
            })),
        )
        .await;
        assert_eq!(response.status(), 422, "{why} must be rejected");
        let body = body_json(response).await;
        assert_eq!(
            body["error"]["reason"], "validation_failed",
            "{why}: failures carry the typed reason"
        );
        assert!(
            !body["error"]["message"].as_str().unwrap_or("").is_empty(),
            "{why}: a human message accompanies the code"
        );
    }

    // Blank name is a 422 too.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &bearer,
        Some(json!({
            "name": "   ",
            "spec": {"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar"},
        })),
    )
    .await;
    assert_eq!(response.status(), 422, "blank name is a caller error");
}

#[tokio::test]
async fn same_idempotency_key_replays_the_stored_response() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("repdefs-idem-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    let body = create_body("Replay me", "org");
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &bearer,
        Some(body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first = body_json(first).await;

    let second = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &bearer,
        Some(body),
    )
    .await;
    assert_eq!(second.status(), 200, "same key + same body replays");
    let second = body_json(second).await;
    assert_eq!(first, second, "the replay is byte-identical");

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-definitions",
        &bearer,
        None,
    )
    .await;
    assert_eq!(
        body_json(response).await["definitions"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the replay never double-applies"
    );
}

#[tokio::test]
async fn technicians_read_but_cannot_write_definitions() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("repdefs-gate-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    // The owner publishes one org-scope definition.
    let created = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &bearer,
            Some(create_body("Org wall board", "org")),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

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
    let tech = provision_technician(&state, &app, org_id, &format!("tech@{slug}.test")).await;
    let tech_bearer = [auth(&tech)];

    // READ: technicians hold reporting:view — org-published definitions
    // are visible to them.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-definitions",
        &tech_bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "technicians may list definitions");
    let arr = body_json(response).await["definitions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(arr.len(), 1, "the org-published definition is visible");
    assert_eq!(arr[0]["id"].as_str().unwrap(), created);

    // WRITE: reporting:create is admin-gated — typed 403.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &tech_bearer,
        Some(create_body("tech def", "self")),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");

    // Mutating someone else's org-scope row: also a 403 (after the view
    // permission let them READ it).
    for (method, uri) in [
        (
            axum::http::Method::PUT,
            format!("/api/v1/report-definitions/{created}"),
        ),
        (
            axum::http::Method::DELETE,
            format!("/api/v1/report-definitions/{created}"),
        ),
    ] {
        let body = if method == axum::http::Method::PUT {
            Some(json!({
                "name": "hijacked",
                "spec": {"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar"}
            }))
        } else {
            None
        };
        let response = send(&app, method, &uri, &tech_bearer, body).await;
        assert_eq!(response.status(), 403, "non-admins never mutate org rows");
        assert_eq!(body_json(response).await["error"]["reason"], "forbidden");
    }
}

#[tokio::test]
async fn non_admin_authors_are_scoped_to_their_own_definitions() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("repdefs-author-{}", Uuid::now_v7().simple());
    let owner = register_org(&app, &slug).await;

    let org_id: Uuid = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&owner)],
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
    seed_report_author_role(&state, org_id).await;
    let author_a = provision_report_author(&state, &app, org_id, &format!("a@{slug}.test")).await;
    let author_b = provision_report_author(&state, &app, org_id, &format!("b@{slug}.test")).await;
    let a_bearer = [auth(&author_a)];
    let b_bearer = [auth(&author_b)];

    // Author A asks for org scope but is forced onto `self`.
    let created = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-definitions",
            &a_bearer,
            Some(create_body("A's private board", "org")),
        )
        .await;
        assert_eq!(
            response.status(),
            200,
            "custom-role authors may create definitions"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["scope"], "self",
            "org publication requires admin; the grant is capped"
        );
        body["id"].as_str().unwrap().to_string()
    };

    // Author B can neither see nor touch A's private definition.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/report-definitions/{created}"),
        &b_bearer,
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "private rows of others are invisible"
    );
    assert_eq!(body_json(response).await["error"]["reason"], "not_found");

    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-definitions",
        &b_bearer,
        None,
    )
    .await;
    assert_eq!(
        body_json(response).await["definitions"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "A's private definition never shows in B's list"
    );

    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/report-definitions/{created}"),
        &b_bearer,
        Some(json!({
            "name": "stolen",
            "spec": {"view": "tickets", "measures": [{"agg": "count"}], "viz": "bar"}
        })),
    )
    .await;
    assert_eq!(response.status(), 403, "B cannot rewrite A's row");

    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/report-definitions/{created}"),
        &b_bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 403, "B cannot delete A's row");

    // The OWNER (admin) sees the private definition in their list.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-definitions",
        &[auth(&owner)],
        None,
    )
    .await;
    let arr = body_json(response).await["definitions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(arr.len(), 1, "admins see every org definition");
    assert_eq!(arr[0]["id"].as_str().unwrap(), created);

    // Author A updates their OWN row through the version gate.
    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/report-definitions/{created}"),
        &a_bearer,
        Some(json!({
            "name": "A's renamed board",
            "spec": {
                "view": "tickets", "filters": [], "group_by": ["priority"],
                "measures": [{"agg": "count"}], "viz": "table"
            },
            "version": 1
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "authors manage their own rows");
    assert_eq!(body_json(response).await["mutation"]["seq"], 2);
}

#[tokio::test]
async fn definitions_are_served_identically_on_both_mounts() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("repdefs-dual-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];

    let canonical = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &bearer,
        Some(create_body("Dual mount", "org")),
    )
    .await;
    assert_eq!(canonical.status(), 200);

    let alias = send(
        &app,
        axum::http::Method::GET,
        "/api/report-definitions",
        &bearer,
        None,
    )
    .await;
    assert_eq!(
        alias.status(),
        200,
        "the legacy alias mounts the same surface"
    );
    let alias_list = body_json(alias).await;
    let canonical_list = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/report-definitions",
            &bearer,
            None,
        )
        .await;
        body_json(response).await
    };
    assert_eq!(
        canonical_list, alias_list,
        "both mounts serve one handler set"
    );
}
