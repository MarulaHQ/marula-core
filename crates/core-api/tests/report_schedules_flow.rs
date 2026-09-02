//! Report-schedule CRUD flow (TASKLIST R4) through the REAL core-api
//! router.
//!
//! Purpose: the scheduled-delivery admin contract over HTTP —
//! 1. Admin CRUD round-trip riding the instant-first write path: create
//!    returns read-your-write `mutation` metadata and a materialized
//!    next-run preview; updates enforce the version gate (typed
//!    `version_conflict`); deletes echo the removed schedule.
//! 2. Idempotency: same key + same body replays the SAME response; only
//!    one row lands.
//! 3. Surface is owner/admin ONLY: builtin technicians (reporting:view)
//!    and even custom roles carrying the full reporting write matrix are
//!    both rejected — scheduling is org-wide delivery configuration.
//! 4. Validation fails closed with typed reasons: cadence shape, IANA
//!    zone, wall-clock time, window span, recipient and definition
//!    existence.
//! 5. The listing is cursor-paged and every row carries its next-run
//!    preview.
//! 6. Lifecycle events land in the outbox (`report_schedule.created` /
//!    `.updated` / `.deleted`).
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
        .expect("report_schedules e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([31; 32]);
    AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
        store: Arc::new(core_store::memory::MemoryStore::new()),
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
            "org_name": "Delivery Co",
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

async fn me_id(app: &Router, token: &str) -> String {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "me must succeed");
    body_json(response).await["user_id"]
        .as_str()
        .expect("user id")
        .into()
}

async fn create_definition(app: &Router, token: &str, name: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/report-definitions",
        &[auth(token)],
        Some(json!({
            "name": name,
            "spec": {
                "view": "tickets",
                "filters": [],
                "group_by": ["status_name"],
                "measures": [{"agg": "count"}],
                "viz": "bar"
            },
            "scope": "org"
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "definition create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("definition id")
        .into()
}

fn schedule_body(
    name: &str,
    definition_id: &str,
    recipients: &[String],
    mut patch: Value,
) -> Value {
    let mut body = json!({
        "name": name,
        "report_definition_id": definition_id,
        "recipients": recipients,
        "cadence": "weekly",
        "time_of_day": "08:00",
        "day_of_week": 0,
        "timezone": "UTC",
        "window_days": 14,
        "enabled": true
    });
    if let Some(map) = patch.as_object_mut() {
        for (k, v) in map {
            body[k] = v.clone();
        }
    }
    body
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

/// Creates a user with a CUSTOM role carrying the full reporting write
/// matrix but NO admin standing — the case that pins the extra
/// owner/admin gate on this surface.
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

async fn org_id_of(state: &AppState, slug: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row")
}

#[tokio::test]
async fn admin_crud_round_trip_with_next_run_preview() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rsched-crud-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];
    let owner_id = me_id(&app, &token).await;
    let definition = create_definition(&app, &token, "Weekly ops digest").await;

    // CREATE: read-your-write metadata + materialized next-run preview.
    let (schedule_id, next_run) = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-schedules",
            &bearer,
            Some(schedule_body(
                "Monday digest",
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "idempotency_key": "sched-create-1" }),
            )),
        )
        .await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, 200, "create must succeed: {body}");
        assert_eq!(body["name"], "Monday digest");
        assert_eq!(body["cadence"], "weekly");
        assert_eq!(body["day_of_week"], 0);
        assert_eq!(body["definition_name"], "Weekly ops digest");
        assert_eq!(body["mutation"]["seq"], 1, "first write is seq 1");
        assert_eq!(body["mutation"]["version"], 1);
        assert!(body["next_run_at"].is_string(), "preview materialized");
        let next_run = body["next_run_at"].as_str().unwrap().to_string();
        assert!(
            next_run.as_str() > "2",
            "preview is an RFC3339 instant: {next_run}"
        );
        (body["id"].as_str().unwrap().to_string(), next_run)
    };

    // GET one: the same view, matching preview.
    {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/v1/report-schedules/{schedule_id}"),
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        let body = body_json(response).await;
        assert_eq!(body["next_run_at"].as_str().unwrap(), next_run);
    }

    // UPDATE: stale version is a typed conflict; current version applies.
    {
        let stale = schedule_body(
            "Monday digest",
            &definition,
            std::slice::from_ref(&owner_id),
            json!({ "version": 0, "cadence": "monthly", "day_of_week": null, "day_of_month": 1 }),
        );
        let response = send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/v1/report-schedules/{schedule_id}"),
            &bearer,
            Some(stale),
        )
        .await;
        assert_eq!(response.status(), 409, "stale version must conflict");
        let body = body_json(response).await;
        assert_eq!(body["error"]["reason"], "version_conflict");

        let fresh = schedule_body(
            "Monday digest",
            &definition,
            std::slice::from_ref(&owner_id),
            json!({ "version": 1, "cadence": "monthly", "day_of_week": null, "day_of_month": 1 }),
        );
        let response = send(
            &app,
            axum::http::Method::PUT,
            &format!("/api/v1/report-schedules/{schedule_id}"),
            &bearer,
            Some(fresh),
        )
        .await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, 200, "current version applies: {body}");
        assert_eq!(body["cadence"], "monthly");
        assert_eq!(body["mutation"]["seq"], 2);
    }

    // DELETE: echo of the removed schedule.
    {
        let response = send(
            &app,
            axum::http::Method::DELETE,
            &format!("/api/v1/report-schedules/{schedule_id}"),
            &bearer,
            None,
        )
        .await;
        assert_eq!(response.status(), 200);
        let body = body_json(response).await;
        assert_eq!(body["id"].as_str().unwrap(), schedule_id);
        assert_eq!(body["mutation"]["seq"], 3, "delete is the third write");
    }

    // Lifecycle events in the outbox, one each.
    let org = org_id_of(&state, &slug).await;
    let events: (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE event_type='report_schedule.created'), \
                count(*) FILTER (WHERE event_type='report_schedule.updated'), \
                count(*) FILTER (WHERE event_type='report_schedule.deleted') \
         FROM outbox_events WHERE org_id = $1",
    )
    .bind(org)
    .fetch_one(&state.pool)
    .await
    .expect("events aggregate");
    assert_eq!(events, (1, 1, 1), "one created/updated/deleted event each");
}

#[tokio::test]
async fn idempotent_create_replays_the_same_schedule() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rsched-idem-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];
    let owner_id = me_id(&app, &token).await;
    let definition = create_definition(&app, &token, "Daily digest").await;

    let body = schedule_body(
        "Daily digest",
        &definition,
        &[owner_id],
        json!({ "cadence": "daily", "day_of_week": null, "idempotency_key": "sched-idem-1" }),
    );
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-schedules",
        &bearer,
        Some(body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first_body = body_json(first).await;

    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-schedules",
        &bearer,
        Some(body),
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replay_body = body_json(replay).await;

    assert_eq!(
        first_body["id"].as_str().unwrap(),
        replay_body["id"].as_str().unwrap(),
        "same key + same body replays the SAME schedule"
    );
    assert_eq!(
        first_body["mutation"]["seq"].as_i64().unwrap(),
        replay_body["mutation"]["seq"].as_i64().unwrap(),
        "replay carries the original write's sequence"
    );
}

#[tokio::test]
async fn surface_rejects_non_admins_even_with_reporting_grants() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rsched-rbac-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let owner_id = me_id(&app, &token).await;
    let definition = create_definition(&app, &token, "RBAC digest").await;

    let org: Uuid = sqlx::query_scalar("SELECT id FROM orgs WHERE slug = $1")
        .bind(&slug)
        .fetch_one(&state.pool)
        .await
        .expect("org row");
    seed_report_author_role(&state, org).await;
    let technician = provision_technician(&state, &app, org, &format!("tech-{slug}.test")).await;
    let author = provision_report_author(&state, &app, org, &format!("author-{slug}.test")).await;

    // The body a reporting-granted non-admin would submit (their OWN id
    // is a valid recipient — the rejection is the ADMIN gate, not data).
    let author_user_id: String = {
        let response = send(
            &app,
            axum::http::Method::GET,
            "/api/v1/me",
            &[auth(&author)],
            None,
        )
        .await;
        body_json(response).await["user_id"]
            .as_str()
            .expect("author user id")
            .to_string()
    };
    let body = schedule_body(
        "author digest",
        &definition,
        std::slice::from_ref(&author_user_id),
        json!({ "idempotency_key": "sched-rbac-1" }),
    );

    // Builtin technician (reporting:view): reads and writes both 403.
    for (method, uri, body) in [
        (
            axum::http::Method::GET,
            "/api/v1/report-schedules".to_string(),
            None,
        ),
        (
            axum::http::Method::POST,
            "/api/v1/report-schedules".to_string(),
            Some(body.clone()),
        ),
    ] {
        let response = send(&app, method, &uri, &[auth(&technician)], body).await;
        assert_eq!(
            response.status(),
            403,
            "technician must not touch schedules"
        );
    }

    // Custom role with the FULL reporting write matrix, still not an
    // admin: the owner/admin gate is the second lock on this surface.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-schedules",
        &[auth(&author)],
        Some(body),
    )
    .await;
    assert_eq!(
        response.status(),
        403,
        "reporting:create without admin standing must 403"
    );
    let parsed = body_json(response).await;
    assert_eq!(parsed["error"]["reason"], "forbidden");

    // The admin's own writes remain fine (definition + recipient valid).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/report-schedules",
        &[auth(&token)],
        Some(schedule_body(
            "admin digest",
            &definition,
            &[owner_id],
            json!({}),
        )),
    )
    .await;
    assert_eq!(response.status(), 200, "admin creates cleanly");
}

#[tokio::test]
async fn validation_fails_closed_typed() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rsched-valid-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];
    let owner_id = me_id(&app, &token).await;
    let definition = create_definition(&app, &token, "Valid digest").await;

    let cases: Vec<(Value, &str)> = vec![
        // Weekly without its day field.
        (
            schedule_body(
                "bad-weekly",
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "day_of_week": null }),
            ),
            "cadence weekly requires day_of_week",
        ),
        // Unknown IANA zone.
        (
            schedule_body(
                "bad-tz",
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "timezone": "Mars/Olympus" }),
            ),
            "unknown timezone",
        ),
        // Garbage wall clock.
        (
            schedule_body(
                "bad-time",
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "time_of_day": "25:99" }),
            ),
            "time_of_day",
        ),
        // Window span out of range.
        (
            schedule_body(
                "bad-window",
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "window_days": 400 }),
            ),
            "window_days",
        ),
        // Unknown definition reference.
        (
            schedule_body(
                "bad-def",
                &Uuid::now_v7().to_string(),
                std::slice::from_ref(&owner_id),
                json!({}),
            ),
            "report_definition_id",
        ),
        // Recipient outside the org.
        (
            schedule_body(
                "bad-recipient",
                &definition,
                &[Uuid::now_v7().to_string()],
                json!({}),
            ),
            "not a user of this org",
        ),
        // No recipients at all.
        (
            schedule_body("no-recipients", &definition, &[], json!({})),
            "at least one recipient",
        ),
    ];

    for (body, expect_fragment) in cases {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-schedules",
            &bearer,
            Some(body),
        )
        .await;
        let status = response.status();
        let parsed = body_json(response).await;
        assert_eq!(
            status, 422,
            "expected typed 422 for {expect_fragment}: {parsed}"
        );
        assert_eq!(parsed["error"]["reason"], "validation_failed");
        let message = parsed["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(expect_fragment),
            "message {message:?} should mention {expect_fragment}"
        );
    }
}

#[tokio::test]
async fn listing_is_cursor_paged_with_previews() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("rsched-list-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = [auth(&token)];
    let owner_id = me_id(&app, &token).await;
    let definition = create_definition(&app, &token, "Listed digest").await;

    for name in ["alpha", "beta", "gamma"] {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/report-schedules",
            &bearer,
            Some(schedule_body(
                name,
                &definition,
                std::slice::from_ref(&owner_id),
                json!({ "cadence": "daily", "day_of_week": null, "time_of_day": "07:00" }),
            )),
        )
        .await;
        assert_eq!(response.status(), 200, "{name} must create");
    }

    // Page 1: two rows in (name, id) order + a cursor.
    let page1 = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/report-schedules?limit=2",
        &bearer,
        None,
    )
    .await;
    assert_eq!(page1.status(), 200);
    let page1 = body_json(page1).await;
    let names: Vec<&str> = page1["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["alpha", "beta"]);
    assert!(
        page1["schedules"][0]["next_run_at"].is_string(),
        "preview present"
    );
    let cursor = page1["next_cursor"].as_str().expect("page 1 has a cursor");

    // Page 2: the remainder, no further cursor.
    let page2 = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/report-schedules?limit=2&cursor={cursor}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(page2.status(), 200);
    let page2 = body_json(page2).await;
    let names: Vec<&str> = page2["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["gamma"]);
    assert!(
        page2["next_cursor"].is_null(),
        "exhausted page carries no cursor"
    );
}
