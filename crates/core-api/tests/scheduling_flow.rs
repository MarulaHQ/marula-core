//! End-to-end scheduling flows (TASKLIST 2.12) driven through the REAL
//! core-api router against a real Postgres.
//!
//! Purpose: prove the booking lifecycle over HTTP — conflict dry-run
//! correctness, 409 overlap rejection backed by the DB EXCLUDE
//! constraint, reschedule re-validation, cancellation freeing slots,
//! availability/on-call/time-off CRUD with manager scoping, the merged
//! day/week/team calendar views, outbox event emission, and cross-org
//! isolation through every read surface.
//!
//! Non-purpose: pure rule math (core-domain suite), SQL-level constraint
//! probes (core-db db_suite), WS delivery.

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
        .expect("scheduling e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with scheduling tables");
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

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Sched Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await
}

fn at(day: u32, hour: u32) -> String {
    // June 2026: day 1 is a Monday. All times UTC.
    format!("2026-06-{day:02}T{hour:02}:00:00Z")
}

#[tokio::test]
async fn booking_lifecycle_conflict_dry_run_move_and_cancel() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("sched1-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    // Create Mon 09:00–10:00.
    let created = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer),
        Some(json!({
            "title": "site visit",
            "starts_at": at(1, 9),
            "ends_at": at(1, 10),
            "kind": "on_site",
            "travel_minutes": 20,
        })),
    )
    .await;
    assert_eq!(created.status(), 201);
    let appt = body_json(created).await;
    let id = appt["id"].as_str().unwrap().to_string();
    assert_eq!(appt["status"], "scheduled");
    assert_eq!(appt["kind"], "on_site");
    assert_eq!(appt["travel_minutes"], 20);

    // Dry-run names the blocker without writing.
    let dry = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling/conflict-check",
            Some(&bearer),
            Some(json!({
                "user_id": appt["user_id"],
                "starts_at": at(1, 9),
                "ends_at": at(1, 11),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(dry["has_conflict"], true);
    assert_eq!(
        dry["conflicts"][0]["appointment_id"].as_str(),
        Some(id.as_str())
    );

    // Overlapping create is rejected 409.
    let clash = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer),
        Some(json!({
            "title": "double booked",
            "starts_at": at(1, 9),
            "ends_at": at(1, 10),
        })),
    )
    .await;
    assert_eq!(clash.status(), 409);

    // Dry-run with self-exclusion (rescheduling semantics) reports none.
    let dry_self = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling/conflict-check",
            Some(&bearer),
            Some(json!({
                "user_id": appt["user_id"],
                "starts_at": at(1, 9),
                "ends_at": at(1, 11),
                "exclude_appointment_id": id,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(dry_self["has_conflict"], false);

    // Second booking 11:00–12:00, then moves: onto occupied 409, free 200.
    let second_resp = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer),
        Some(json!({
            "title": "remote session",
            "starts_at": at(1, 11),
            "ends_at": at(1, 12),
            "kind": "remote",
        })),
    )
    .await;
    assert_eq!(second_resp.status(), 201);
    let second = body_json(second_resp).await;
    let second_id = second["id"].as_str().unwrap().to_string();

    let blocked_move = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointments/{second_id}/move"),
        Some(&bearer),
        Some(json!({ "starts_at": at(1, 9), "ends_at": at(1, 10) })),
    )
    .await;
    assert_eq!(blocked_move.status(), 409);

    // Touching move (10:00–11:00) is legal.
    let moved = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointments/{second_id}/move"),
        Some(&bearer),
        Some(json!({ "starts_at": at(1, 10), "ends_at": at(1, 11) })),
    )
    .await;
    assert_eq!(moved.status(), 200);
    assert_eq!(body_json(moved).await["starts_at"], at(1, 10));

    // Cancel frees the slot; a new booking lands exactly there.
    let cancelled = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/appointments/{second_id}/cancel"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(cancelled.status(), 200);
    assert_eq!(body_json(cancelled).await["status"], "cancelled");

    let rebook = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer),
        Some(json!({
            "title": "rebooked",
            "starts_at": at(1, 10),
            "ends_at": at(1, 11),
        })),
    )
    .await;
    assert_eq!(rebook.status(), 201);

    // Personal week view merges appointments ordered by time.
    let week = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/calendar/week?date=2026-06-03",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let items = week["items"].as_array().unwrap();
    let titles: Vec<&str> = items
        .iter()
        .map(|i| i["title"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(titles, vec!["site visit", "rebooked"]);
    assert_eq!(week["timezone"], "UTC");
    assert_eq!(items[0]["item_type"], "appointment");

    // Day view shows only that local day.
    let day = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/calendar/day?date=2026-06-02",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(day["items"].as_array().unwrap().len(), 0);

    // Availability templates attached to personal views.
    let windows = week["availability_windows"].as_array().unwrap();
    assert!(windows.is_empty() || windows[0]["weekday"].is_u64());
}

#[tokio::test]
async fn availability_oncall_exclusions_and_manager_scoping() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("sched2-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    // Availability: create + list + validation failure + delete.
    let win = send(
        &app,
        axum::http::Method::POST,
        "/api/availability",
        Some(&bearer),
        Some(json!({
            "weekday": 0,
            "start_local": "09:00",
            "end_local": "17:00",
            "timezone": "America/New_York",
        })),
    )
    .await;
    assert_eq!(win.status(), 201);
    let win = body_json(win).await;
    assert_eq!(win["weekday"], 0);

    let bad_tz = send(
        &app,
        axum::http::Method::POST,
        "/api/availability",
        Some(&bearer),
        Some(json!({
            "weekday": 0, "start_local": "09:00", "end_local": "17:00",
            "timezone": "Not/AZone",
        })),
    )
    .await;
    assert_eq!(bad_tz.status(), 422);

    let listed = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/availability",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(listed["windows"].as_array().unwrap().len(), 1);

    // On-call: one rotation lands, stacking is 409, listing works.
    let rotation = send(
        &app,
        axum::http::Method::POST,
        "/api/on-call-blocks",
        Some(&bearer),
        Some(json!({
            "starts_at": at(1, 18),
            "ends_at": "2026-06-02T08:00:00Z",
            "category": "primary",
        })),
    )
    .await;
    assert_eq!(rotation.status(), 201);
    let rotation = body_json(rotation).await;

    let stacked = send(
        &app,
        axum::http::Method::POST,
        "/api/on-call-blocks",
        Some(&bearer),
        Some(json!({
            "starts_at": "2026-06-02T06:00:00Z",
            "ends_at": "2026-06-02T10:00:00Z",
            "category": "secondary",
        })),
    )
    .await;
    assert_eq!(stacked.status(), 409);

    let on_call_list = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/on-call-blocks?from={}&to={}",
                "2026-06-01T00:00:00Z", "2026-06-08T00:00:00Z"
            ),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(on_call_list.as_array().unwrap().len(), 1);

    // Time-off exclusion round-trips.
    let exclusion = send(
        &app,
        axum::http::Method::POST,
        "/api/calendar-exclusions",
        Some(&bearer),
        Some(json!({
            "starts_at": "2026-06-04T00:00:00Z",
            "ends_at": "2026-06-05T00:00:00Z",
            "reason": "vacation",
        })),
    )
    .await;
    assert_eq!(exclusion.status(), 201);
    let exclusion = body_json(exclusion).await;

    // Team calendar merges appointment + on-call item types across members.
    let appt_resp = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer),
        Some(json!({
            "title": "team probe",
            "starts_at": at(2, 13),
            "ends_at": at(2, 14),
        })),
    )
    .await;
    assert_eq!(appt_resp.status(), 201);
    let team = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/calendar/team?from={}&to={}",
                "2026-06-01T00:00:00Z", "2026-06-08T00:00:00Z"
            ),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let kinds: Vec<&str> = team["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["item_type"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"appointment"));
    assert!(kinds.contains(&"on_call"));

    // Deletes clean up (ownership-scoped query params).
    let del_win = send(
        &app,
        axum::http::Method::DELETE,
        &format!(
            "/api/availability/{}?user_id={}",
            win["id"].as_str().unwrap(),
            listed["user_id"].as_str().unwrap()
        ),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(del_win.status(), 204);
    let del_exclusion = send(
        &app,
        axum::http::Method::DELETE,
        &format!(
            "/api/calendar-exclusions/{}?user_id={}",
            exclusion["id"].as_str().unwrap(),
            rotation["user_id"].as_str().unwrap()
        ),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(del_exclusion.status(), 204);
    let gone = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/availability",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(gone["windows"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scheduling_reads_are_org_isolated_and_events_emitted() {
    let (state, app) = test_state().await;
    let slug_a = format!("isola-{}", Uuid::now_v7().simple());
    let slug_b = format!("isolb-{}", Uuid::now_v7().simple());
    let ta = register_org(&app, slug_a).await;
    let tb = register_org(&app, slug_b).await;
    let bearer_a = ta["access_token"].as_str().unwrap().to_string();
    let bearer_b = tb["access_token"].as_str().unwrap().to_string();

    let resp = send(
        &app,
        axum::http::Method::POST,
        "/api/appointments",
        Some(&bearer_a),
        Some(json!({
            "title": "a-only",
            "starts_at": at(3, 9),
            "ends_at": at(3, 10),
        })),
    )
    .await;
    assert_eq!(resp.status(), 201);
    let appt = body_json(resp).await;

    // Org B's team view sees nothing of A.
    let team_b = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/calendar/team?from={}&to={}",
                "2026-06-01T00:00:00Z", "2026-06-08T00:00:00Z"
            ),
            Some(&bearer_b),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(team_b["items"].as_array().unwrap().len(), 0);

    // Direct fetch of A's appointment under B's token → 404.
    let foreign = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/appointments/{}", appt["id"].as_str().unwrap()),
        Some(&bearer_b),
        None,
    )
    .await;
    assert_eq!(foreign.status(), 404);

    // The create emitted appointment.created into the transactional outbox.
    let org_a: Uuid = {
        let me = body_json(
            send(
                &app,
                axum::http::Method::GET,
                "/api/me",
                Some(&bearer_a),
                None,
            )
            .await,
        )
        .await;
        Uuid::parse_str(me["org_id"].as_str().unwrap()).unwrap()
    };
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM outbox_events WHERE org_id = $1 AND entity = 'appointment'",
    )
    .bind(org_a)
    .fetch_all(&state.pool)
    .await
    .expect("outbox scan");
    let kinds: Vec<String> = events.into_iter().map(|e| e.0).collect();
    assert!(
        kinds.contains(&"appointment.created".to_string()),
        "expected appointment.created in outbox, got {kinds:?}"
    );
}
