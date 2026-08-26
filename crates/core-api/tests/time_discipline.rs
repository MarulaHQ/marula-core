//! End-to-end time-discipline flows (TASKLIST 2.9/2.10) driven through the
//! REAL core-api router against a real Postgres.
//!
//! Purpose: prove the single-active-timer switch semantics over HTTP
//! (start/switch auto-pauses with a preserved `switched` segment, one
//! active session per user, stop materializes an entry), the internal
//! gap-audit run's once-per user/day nudge idempotency, the notification
//! center wiring (`missing_time` kind), the self/manager gap reports, and
//! the org time-settings round-trip.
//!
//! Non-purpose: WS hub delivery (realtime crate's own suite) and pure
//! domain rules (core-domain unit/proptest suites).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "time-e2e-internal-token";

async fn test_state() -> (AppState, Router) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("time e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env();
    config.database_url = admin_url;
    config.email_internal_token = Some(TOKEN.into());
    let state = AppState {
        pool,
        config: Arc::new(config),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&Config::from_env())),
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
    internal_token: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if let Some(token) = internal_token {
        builder = builder.header("X-Internal-Token", token);
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

async fn register_org(app: &Router, slug: &str) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        None,
        Some(json!({
            "org_name": "Time Co",
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

#[tokio::test]
async fn switch_auto_pauses_with_switched_segment_and_single_active_holds() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, &format!("timer-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    let boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let board = boards[0]["id"].as_str().unwrap().to_string();

    let mk_ticket = |title: String| {
        json!({
            "board_id": board,
            "title": title,
            "priority": "p2",
        })
    };
    let t1 = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            None,
            Some(mk_ticket("first".into())),
        )
        .await,
    )
    .await;
    let t2 = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            None,
            Some(mk_ticket("second".into())),
        )
        .await,
    )
    .await;

    // Start on t1, then switch to t2: t1 must be auto-paused with a
    // `switched` segment and stay in the working set.
    let s1 = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/timers/start",
            Some(&bearer),
            None,
            Some(json!({ "ticket_id": t1["id"] })),
        )
        .await,
    )
    .await;
    assert_eq!(s1["stopped_at"], Value::Null);

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let s2 = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/timers/start",
            Some(&bearer),
            None,
            Some(json!({ "ticket_id": t2["id"] })),
        )
        .await,
    )
    .await;
    assert_ne!(s1["id"], s2["id"], "switch starts a NEW session");

    let active = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/timers/active",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(active["id"], s2["id"], "exactly one active session");

    let working_set = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/timers/working-set",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let paused = working_set
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == s1["id"])
        .expect("switched session stays in the docked working set");
    let segment = &paused["segments"][0];
    assert_eq!(segment["end_kind"], "switched", "pause reason preserved");
    assert!(paused["stopped_at"].is_null(), "switch pauses, never stops");

    // Stopping the switched session materializes its entry (elapsed > 0).
    let entry = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/timers/stop",
            Some(&bearer),
            None,
            Some(json!({ "session_id": s1["id"], "notes": "from switch" })),
        )
        .await,
    )
    .await;
    assert_eq!(entry["timer_session_id"], s1["id"]);
    assert!(entry["minutes"].as_i64().unwrap() >= 1);
}

#[tokio::test]
async fn gap_audit_run_nudges_once_and_reports_show_the_gap() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, &format!("gap-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let me = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let org_id = me["org_id"].as_str().unwrap().to_string();
    let user_id = me["user_id"].as_str().unwrap().to_string();

    let boards = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let board = boards[0]["id"].as_str().unwrap().to_string();
    let ticket = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            None,
            Some(json!({
                "board_id": board,
                "title": "gap probe",
                "priority": "p3",
            })),
        )
        .await,
    )
    .await;

    // Work with a timer, then stop it into a 1-minute entry.
    let session = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/timers/start",
            Some(&bearer),
            None,
            Some(json!({ "ticket_id": ticket["id"] })),
        )
        .await,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let entry = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/timers/stop",
            Some(&bearer),
            None,
            Some(json!({ "session_id": session["id"] })),
        )
        .await,
    )
    .await;
    assert!(
        entry["minutes"].as_i64().unwrap() >= 1,
        "entry materialized"
    );

    // Internal run for TODAY (UTC — the owner registered with UTC): timer
    // activity + 1 logged minute vs 60 expected ⇒ gap ⇒ one nudge.
    let today = chrono::Utc::now().date_naive();
    let item = json!({
        "items": [{
            "org_id": org_id,
            "user_id": user_id,
            "day": today.to_string(),
        }]
    });
    let run = async |token: Option<&str>| {
        body_json(
            send(
                &app,
                axum::http::Method::POST,
                "/api/internal/time/gap-audit/run",
                None,
                token,
                Some(item.clone()),
            )
            .await,
        )
        .await
    };

    let unauthorized = send(
        &app,
        axum::http::Method::POST,
        "/api/internal/time/gap-audit/run",
        None,
        Some("wrong-token"),
        Some(item.clone()),
    )
    .await;
    assert_eq!(unauthorized.status(), axum::http::StatusCode::UNAUTHORIZED);

    let first = run(Some(TOKEN)).await;
    let r1 = &first["results"][0];
    assert_eq!(r1["error"], Value::Null, "clean run: {r1}");
    assert_eq!(r1["activity"], true, "timer overlap counts as activity");
    assert_eq!(r1["has_gap"], true, "1 logged < 60 expected");
    assert_eq!(r1["nudged"], true, "first run wins the nudge claim");

    let second = run(Some(TOKEN)).await;
    assert_eq!(
        second["results"][0]["nudged"], false,
        "replay never re-nudges"
    );

    // The notification center received exactly one missing_time nudge.
    let notifications = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/notifications?limit=50",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let nudges = notifications["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "missing_time")
        .count();
    assert_eq!(nudges, 1, "exactly one nudge row for the day");

    // Self report shows the gap + the nudge timestamp.
    let report = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-gaps/me?from={today}&to={today}"),
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    let day = &report["days"][0];
    assert_eq!(day["day"], today.to_string());
    assert_eq!(day["activity"], true);
    assert_eq!(day["has_gap"], true);
    assert!(
        day["nudge_sent_at"].is_string(),
        "ledger enrichment present"
    );

    // Manager view (owner) for a specific member works; missing user_id is
    // a validation error, not a silent self-report.
    let managed = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/time-gaps?user_id={user_id}&from={today}&to={today}"),
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(managed["days"][0]["has_gap"], true);
    let missing = send(
        &app,
        axum::http::Method::GET,
        "/api/time-gaps",
        Some(&bearer),
        None,
        None,
    )
    .await;
    assert_eq!(
        missing.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY
    );

    // Settings round-trip (admin) — and the lowered bar changes the rule.
    let settings = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/time-settings",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(settings["eod_nudge_hour"], 18);
    assert_eq!(settings["time_gap_min_minutes"], 60);
    let put = send(
        &app,
        axum::http::Method::PUT,
        "/api/time-settings",
        Some(&bearer),
        None,
        Some(json!({
            "eod_nudges_enabled": false,
            "eod_nudge_hour": 17,
            "time_gap_min_minutes": 0,
        })),
    )
    .await;
    assert_eq!(put.status(), 200);
    let reread = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/time-settings",
            Some(&bearer),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(reread["eod_nudges_enabled"], false);
    assert_eq!(reread["eod_nudge_hour"], 17);
    assert_eq!(reread["time_gap_min_minutes"], 0);

    // Out-of-range knobs are rejected, never clamped silently.
    let bad = send(
        &app,
        axum::http::Method::PUT,
        "/api/time-settings",
        Some(&bearer),
        None,
        Some(json!({
            "eod_nudges_enabled": true,
            "eod_nudge_hour": 24,
            "time_gap_min_minutes": 60,
        })),
    )
    .await;
    assert_eq!(bad.status(), axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}
