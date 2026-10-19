//! End-to-end dispatch board flows (TASKLIST 2.19) driven through the
//! REAL core-api router against a real Postgres.
//!
//! Purpose: prove the dispatch surface over HTTP — day-view assembly
//! (capacity math from availability minus exclusions, per-technician
//! booking rollup), unassigned-queue ordering (priority rank, then
//! effective SLA/due deadline), the bulk-move conflict rejection matrix
//! (typed 409 naming the offending move, atomic all-or-nothing, legal
//! swaps), assignment flipping the ticket assignee + creating the booking,
//! outbox emission for every mutation, and RBAC gating (no scheduling
//! grant ⇒ 403) with cross-org isolation.
//!
//! Non-purpose: pure capacity math (core-domain suite), live-stream
//! delivery of the org channel (the SSE suite in `realtime_flow.rs`),
//! drag-state reducer logic (apps/web vitest).

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
        .expect("dispatch e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with dispatch surfaces");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
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

struct Org {
    token: String,
    org_id: Uuid,
    user_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Dispatch Co",
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
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
    }
}

/// Inserts a same-org user with NO role (hence no permissions) directly
/// through the admin pool — there is no org-user invite API yet — and
/// logs them in for a bearer token.
async fn add_roleless_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}@dispatch.test");
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
    .expect("seed technician user");
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"email": email, "password": "correct-horse-battery"})),
        )
        .await,
    )
    .await;
    login["access_token"].as_str().unwrap().to_string()
}

async fn default_board(app: &Router, org: &Org) -> Uuid {
    let nav = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards/nav",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    nav[0]["id"]
        .as_str()
        .expect("default board seeded")
        .parse()
        .unwrap()
}

async fn create_ticket(
    app: &Router,
    org: &Org,
    board: Uuid,
    title: &str,
    priority: &str,
    due_at: Option<&str>,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(&org.token),
        Some(json!({
            "board_id": board,
            "title": title,
            "priority": priority,
            "due_at": due_at,
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "ticket create must succeed");
    body_json(response).await
}

async fn create_appointment(
    app: &Router,
    org: &Org,
    title: &str,
    day: u32,
    start_h: u32,
    end_h: u32,
) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&org.token),
        Some(json!({
            "title": title,
            "starts_at": at(day, start_h),
            "ends_at": at(day, end_h),
        })),
    )
    .await;
    assert_eq!(response.status(), 201, "appointment create must succeed");
    body_json(response).await
}

async fn day_view(app: &Router, org: &Org) -> Value {
    body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/dispatch/day?date=2026-06-01",
            Some(&org.token),
            None,
        )
        .await,
    )
    .await
}

/// June 2026: day 1 is a Monday. All times UTC.
fn at(day: u32, hour: u32) -> String {
    format!("2026-06-{day:02}T{hour:02}:00:00Z")
}

fn technician<'v>(view: &'v Value, user_id: &Uuid) -> &'v Value {
    view["technicians"]
        .as_array()
        .expect("technicians array")
        .iter()
        .find(|t| t["user_id"].as_str() == Some(&user_id.to_string()))
        .expect("own technician column present")
}

// --- capacity view -------------------------------------------------------

#[tokio::test]
async fn day_view_computes_capacity_and_books_rollup() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("disp-cap-{}", Uuid::now_v7().simple())).await;

    // Mon 09:00–17:00 availability, minus a 12:00–13:00 exclusion
    // ⇒ 420 available minutes for 2026-06-01.
    let window = send(
        &app,
        axum::http::Method::POST,
        "/api/availability",
        Some(&org.token),
        Some(json!({
            "weekday": 0, "start_local": "09:00", "end_local": "17:00", "timezone": "UTC",
        })),
    )
    .await;
    assert_eq!(window.status(), 201);
    let exclusion = send(
        &app,
        axum::http::Method::POST,
        "/api/calendar-exclusions",
        Some(&org.token),
        Some(json!({
            "starts_at": at(1, 12), "ends_at": at(1, 13), "reason": "lunch",
        })),
    )
    .await;
    assert_eq!(exclusion.status(), 201);

    // Empty board: 420 available, 0 booked.
    let view = day_view(&app, &org).await;
    let me = technician(&view, &org.user_id);
    assert_eq!(me["available_minutes"], 420);
    assert_eq!(me["booked_minutes"], 0);
    assert_eq!(me["utilization_percent"], 0);
    assert_eq!(view["date"], "2026-06-01");

    // A 09:00–11:00 booking ⇒ 120 booked, 28% utilization; the exclusion
    // and window ride along for board shading.
    let appt = create_appointment(&app, &org, "site visit", 1, 9, 11).await;
    let view = day_view(&app, &org).await;
    let me = technician(&view, &org.user_id);
    assert_eq!(me["available_minutes"], 420);
    assert_eq!(me["booked_minutes"], 120);
    assert_eq!(me["utilization_percent"], 28);
    assert_eq!(me["exclusions"].as_array().unwrap().len(), 1);
    assert_eq!(me["availability_windows"].as_array().unwrap().len(), 1);
    assert_eq!(me["appointments"].as_array().unwrap().len(), 1);
    assert_eq!(me["appointments"][0]["id"], appt["id"]);

    // Cross-org isolation: another org's board never sees this booking.
    let other = register_org(&app, format!("disp-cap-other-{}", Uuid::now_v7().simple())).await;
    let other_view = day_view(&app, &other).await;
    let other_me = technician(&other_view, &other.user_id);
    assert_eq!(other_me["booked_minutes"], 0);
    assert_eq!(other_me["appointments"].as_array().unwrap().len(), 0);

    let _ = state; // pool lives in app state
}

// --- queue ordering ------------------------------------------------------

#[tokio::test]
async fn unassigned_queue_orders_priority_then_deadline() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("disp-queue-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &org).await;

    // Deliberately out of order: the queue must re-rank by priority,
    // then earliest effective deadline (due_at when no SLA state).
    let p3 = create_ticket(&app, &org, board, "low", "p3", None).await;
    let p1_late = create_ticket(
        &app,
        &org,
        board,
        "urgent late",
        "p1",
        Some("2026-06-01T14:00:00Z"),
    )
    .await;
    let p2 = create_ticket(&app, &org, board, "medium", "p2", None).await;
    let p1_early = create_ticket(
        &app,
        &org,
        board,
        "urgent early",
        "p1",
        Some("2026-06-01T12:00:00Z"),
    )
    .await;

    // The seeded default SLA policy auto-provisions resolution_due per
    // ticket (creation-time based), which legitimately outranks manual
    // due_at in the my-work contract — this test pins the MANUAL-deadline
    // tiebreak, so drop the auto state and re-run with due_at effective.
    sqlx::query("DELETE FROM ticket_sla_state WHERE org_id = $1")
        .bind(org.org_id)
        .execute(&state.pool)
        .await
        .expect("clear sla state");

    let view = day_view(&app, &org).await;
    let queue = view["unassigned"].as_array().expect("unassigned array");
    let numbers: Vec<&str> = queue
        .iter()
        .map(|t| t["number"].as_str().expect("number"))
        .collect();
    assert_eq!(
        numbers,
        vec![
            p1_early["number"].as_str().unwrap(),
            p1_late["number"].as_str().unwrap(),
            p2["number"].as_str().unwrap(),
            p3["number"].as_str().unwrap(),
        ],
        "queue order must be p1(12:00), p1(14:00), p2, p3"
    );

    // A deadline-less p1 keeps its priority rank but loses the
    // within-priority tiebreak to timed siblings (NULLS LAST mirrors the
    // my-work SQL this queue reuses).
    let plain_p1 = create_ticket(&app, &org, board, "urgent no due", "p1", None).await;
    sqlx::query("DELETE FROM ticket_sla_state WHERE org_id = $1")
        .bind(org.org_id)
        .execute(&state.pool)
        .await
        .expect("clear sla state again");
    let view = day_view(&app, &org).await;
    let numbers: Vec<&str> = view["unassigned"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["number"].as_str().unwrap())
        .collect();
    assert_eq!(
        numbers,
        vec![
            p1_early["number"].as_str().unwrap(),
            p1_late["number"].as_str().unwrap(),
            plain_p1["number"].as_str().unwrap(),
            p2["number"].as_str().unwrap(),
            p3["number"].as_str().unwrap(),
        ],
        "priority ranks dominate; deadline-less p1 slots after timed p1s"
    );
}

// --- bulk-move conflict matrix -------------------------------------------

#[tokio::test]
async fn bulk_move_rejects_conflicts_atomically_and_allows_swaps() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("disp-move-{}", Uuid::now_v7().simple())).await;

    let a = create_appointment(&app, &org, "booking A", 1, 9, 10).await;
    let b = create_appointment(&app, &org, "booking B", 1, 10, 11).await;
    let c = create_appointment(&app, &org, "booking C", 1, 15, 16).await;
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    let c_id = c["id"].as_str().unwrap();
    let owner_user = org.user_id;

    // 1. Simple overlap → typed 409 naming move 0 and the blocker.
    let rejected = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/bulk-move",
            Some(&org.token),
            Some(json!({"moves": [
                {"appointment_id": a_id, "user_id": owner_user, "starts_at": at(1, 10), "ends_at": at(1, 11)},
            ]})),
        )
        .await,
    )
    .await;
    assert_eq!(rejected["error"]["code"], 409);
    assert_eq!(rejected["error"]["index"], 0);
    assert_eq!(rejected["error"]["appointment_id"], a["id"]);
    assert_eq!(rejected["error"]["blocker_title"], "booking B");

    // 2. Batch where only the SECOND move conflicts → index 1; and the
    //    accepted first move must NOT have landed (atomicity).
    let rejected = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/bulk-move",
            Some(&org.token),
            Some(json!({"moves": [
                {"appointment_id": a_id, "user_id": owner_user, "starts_at": at(1, 13), "ends_at": at(1, 14)},
                {"appointment_id": b_id, "user_id": owner_user, "starts_at": at(1, 15), "ends_at": at(1, 16)},
            ]})),
        )
        .await,
    )
    .await;
    assert_eq!(rejected["error"]["code"], 409);
    assert_eq!(rejected["error"]["index"], 1, "second move names B vs C");
    let a_after = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/appointments/{a_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(a_after["starts_at"], at(1, 9), "atomic batch: A unmoved");

    // 3. Legal swap: A↔B slots (moving bookings vacate their own slots,
    //    so A may land on B's old 10:00–11:00 and B on A's 09:00–10:00).
    let swapped = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/bulk-move",
            Some(&org.token),
            Some(json!({"moves": [
                {"appointment_id": a_id, "user_id": owner_user, "starts_at": at(1, 10), "ends_at": at(1, 11)},
                {"appointment_id": b_id, "user_id": owner_user, "starts_at": at(1, 9), "ends_at": at(1, 10)},
            ]})),
        )
        .await,
    )
    .await;
    assert_eq!(
        swapped["moved"].as_array().expect("moved list").len(),
        2,
        "swap lands atomically"
    );

    // 4. Invalid span → 422; empty batch → 422; over-limit batch → 422.
    let bad_span = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/bulk-move",
        Some(&org.token),
        Some(json!({"moves": [
            {"appointment_id": a_id, "user_id": owner_user, "starts_at": at(2, 9), "ends_at": at(2, 9)},
        ]})),
    )
    .await;
    assert_eq!(bad_span.status(), 422);

    let empty = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/bulk-move",
        Some(&org.token),
        Some(json!({"moves": []})),
    )
    .await;
    assert_eq!(empty.status(), 422);

    let oversized: Vec<Value> = (0..65)
        .map(|_| {
            json!({"appointment_id": a_id, "user_id": owner_user, "starts_at": at(2, 9), "ends_at": at(2, 10)})
        })
        .collect();
    let over = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/bulk-move",
        Some(&org.token),
        Some(json!({"moves": oversized})),
    )
    .await;
    assert_eq!(over.status(), 422);

    // 5. Foreign-org appointment id → 404 (RLS/org scoping).
    let other = register_org(&app, format!("disp-move-other-{}", Uuid::now_v7().simple())).await;
    let cross = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/bulk-move",
        Some(&other.token),
        Some(json!({"moves": [
            {"appointment_id": c_id, "user_id": other.user_id, "starts_at": at(1, 13), "ends_at": at(1, 14)},
        ]})),
    )
    .await;
    assert_eq!(cross.status(), 404, "cross-org move must be NotFound");

    // 6. Successful single move updates the slot and reports it.
    let moved = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/bulk-move",
            Some(&org.token),
            Some(json!({"moves": [
                {"appointment_id": c_id, "user_id": owner_user, "starts_at": at(1, 13), "ends_at": at(1, 14)},
            ]})),
        )
        .await,
    )
    .await;
    assert_eq!(moved["moved"][0]["starts_at"], at(1, 13));

    // Every landed batch emitted appointment.updated events.
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM outbox_events \
         WHERE org_id = $1 AND entity = 'appointment' AND event_type = 'appointment.updated'",
    )
    .bind(org.org_id)
    .fetch_all(&state.pool)
    .await
    .expect("outbox scan");
    assert!(
        !events.is_empty(),
        "expected appointment.updated events in outbox"
    );
}

// --- assignment ----------------------------------------------------------

#[tokio::test]
async fn assign_places_ticket_and_flips_assignee() {
    let (_state, app) = test_state().await;
    let org = register_org(&app, format!("disp-assign-{}", Uuid::now_v7().simple())).await;
    let board = default_board(&app, &org).await;
    let ticket = create_ticket(&app, &org, board, "server down", "p1", None).await;
    let ticket_id = ticket["id"].as_str().unwrap();

    let assigned = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/dispatch/assign",
            Some(&org.token),
            Some(json!({
                "ticket_id": ticket_id,
                "user_id": org.user_id,
                "starts_at": at(1, 13),
                "ends_at": at(1, 14),
                "travel_minutes": 20,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(assigned["ticket_id"], ticket["id"]);
    assert_eq!(
        assigned["ticket_number"], ticket["number"],
        "block face carries the ticket number"
    );
    assert_eq!(assigned["travel_minutes"], 20);

    // The ticket's assignee flipped (queue drains)…
    let after = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket_id}"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        after["assignee_id"].as_str(),
        Some(org.user_id.to_string()).as_deref(),
        "assignment must set the ticket assignee"
    );

    // …and the board shows the booking with the joined number.
    let view = day_view(&app, &org).await;
    let me = technician(&view, &org.user_id);
    let blocks = me["appointments"].as_array().unwrap();
    assert!(blocks
        .iter()
        .any(|x| x["ticket_id"] == ticket["id"] && x["ticket_number"] == ticket["number"]));
    assert!(
        !view["unassigned"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == ticket["id"]),
        "assigned ticket left the queue"
    );

    // Conflicting assignment → 409.
    let conflict = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/assign",
        Some(&org.token),
        Some(json!({
            "ticket_id": ticket_id,
            "user_id": org.user_id,
            "starts_at": at(1, 13),
            "ends_at": at(1, 14),
        })),
    )
    .await;
    assert_eq!(conflict.status(), 409);

    // Bad span → 422.
    let bad = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/assign",
        Some(&org.token),
        Some(json!({
            "ticket_id": ticket_id,
            "user_id": org.user_id,
            "starts_at": at(1, 15),
            "ends_at": at(1, 15),
        })),
    )
    .await;
    assert_eq!(bad.status(), 422);

    // Unknown technician → 404.
    let missing = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/assign",
        Some(&org.token),
        Some(json!({
            "ticket_id": ticket_id,
            "user_id": Uuid::now_v7(),
            "starts_at": at(1, 16),
            "ends_at": at(1, 17),
        })),
    )
    .await;
    assert_eq!(missing.status(), 404);
}

// --- RBAC gating ----------------------------------------------------------

#[tokio::test]
async fn dispatch_endpoints_require_permissions() {
    let (state, app) = test_state().await;
    let org = register_org(&app, format!("disp-rbac-{}", Uuid::now_v7().simple())).await;
    let outsider = add_roleless_user(&app, &state, &org, "no-role").await;

    let view = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch/day",
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(view.status(), 403, "day view needs scheduling.view");

    let move_req = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/bulk-move",
        Some(&outsider),
        Some(json!({"moves": []})),
    )
    .await;
    assert_eq!(move_req.status(), 403, "bulk-move needs scheduling.edit");

    let assign = send(
        &app,
        axum::http::Method::POST,
        "/api/dispatch/assign",
        Some(&outsider),
        Some(json!({
            "ticket_id": Uuid::now_v7(),
            "user_id": org.user_id,
            "starts_at": at(1, 9),
            "ends_at": at(1, 10),
        })),
    )
    .await;
    assert_eq!(assign.status(), 403, "assign needs scheduling.create");

    // Unauthenticated → 401 on the read path.
    let anon = send(
        &app,
        axum::http::Method::GET,
        "/api/dispatch/day",
        None,
        None,
    )
    .await;
    assert_eq!(anon.status(), 401);
}
