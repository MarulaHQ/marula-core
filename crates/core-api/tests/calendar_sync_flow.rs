//! Two-way calendar sync e2e (Wave 14 lane 2).
//!
//! Covers the provider seam end to end against a REAL in-process iCal
//! fixture server: connection lifecycle (incl. the typed duplicate-name
//! 409), inbound sync import → availability double-check (the public
//! scheduling-link slot preview and the book-time gate must both honor
//! external busy time, exactly like portal bookings), upsert/release on
//! re-poll, the highest-sequence-wins duplicate rule, and the outbound
//! token-scoped iCal feed.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_store::memory::MemoryStore;
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
        .expect("calendar-sync e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with calendar-sync tables");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(MemoryStore::new()),
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

async fn body_text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Sync Co",
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

fn next_monday_date() -> String {
    use chrono::Datelike;
    let mut day = chrono::Utc::now() + chrono::Duration::days(1);
    while day.weekday() != chrono::Weekday::Mon {
        day += chrono::Duration::days(1);
    }
    day.format("%Y-%m-%d").to_string()
}

/// The fixture ICS: one busy VEVENT on `day` from start_h to end_h UTC.
fn fixture_ics(day: &str, uid: &str, start_h: u32, end_h: u32, summary: &str) -> String {
    let day = &day.replace('-', "");
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fixture//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:{day}T080000Z\r\nDTSTART:{day}T{start_h:02}0000Z\r\nDTEND:{day}T{end_h:02}0000Z\r\nSUMMARY:{summary}\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

/// Serves a mutable ICS document at /calendar.ics — the test swaps the
/// document between polls through the watch channel.
async fn spawn_ics_server(
    initial: String,
) -> (std::net::SocketAddr, tokio::sync::watch::Sender<String>) {
    let (tx, rx) = tokio::sync::watch::channel(initial);
    let app = axum::Router::new().route(
        "/calendar.ics",
        axum::routing::get(move || {
            let mut rx = rx.clone();
            async move { rx.borrow_and_update().clone() }
        }),
    );
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, tx)
}

#[tokio::test]
async fn inbound_busy_time_gates_booking_like_portal_bookings() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync1-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    let me: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/me",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let my_id = me["user_id"].as_str().unwrap().to_string();

    // Publish Monday 09:00–17:00 UTC availability.
    let day = next_monday_date();
    let avail = send(
        &app,
        axum::http::Method::POST,
        "/api/availability",
        Some(&bearer),
        Some(json!({
            "weekday": 0,
            "start_local": "09:00",
            "end_local": "17:00",
            "timezone": "UTC",
            "effective_from": day,
        })),
    )
    .await;
    assert_eq!(avail.status(), 201, "{}", body_json(avail).await);

    // The fixture server's ICS: one busy event Monday 11:00–13:00 UTC.
    let uid = "evt-busy-1@external.test";
    let (addr, _ics_tx) = spawn_ics_server(fixture_ics(&day, uid, 11, 13, "External busy")).await;

    // Create a two-way connection pointing at the fixture.
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/calendar-sync",
            Some(&bearer),
            Some(json!({
                "name": "Work calendar",
                "provider": "ical",
                "direction": "two_way",
                "feed_url": format!("http://{addr}/calendar.ics"),
            })),
        )
        .await,
    )
    .await;
    assert_eq!(created["connection"]["status"], "active");
    assert!(
        created["feed_token"].as_str().is_some(),
        "outbound token minted once for outbound-capable directions"
    );
    let connection_id = created["connection"]["id"].as_str().unwrap().to_string();
    let feed_token = created["feed_token"].as_str().unwrap().to_string();

    // Poll: the external event imports as busy time.
    let synced: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/calendar-sync/{connection_id}/sync"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(synced["outcome"]["imported"], 1, "{synced}");
    assert_eq!(synced["outcome"]["overlaps"], 0);

    // The mirror + exclusion exist and feed availability.
    let events: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/calendar-sync/{connection_id}/events"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(events["events"].as_array().unwrap().len(), 1);
    assert!(
        events["events"][0]["exclusion_id"].is_string(),
        "busy event feeds the 2.12 availability foundation"
    );

    // THE book-time double-check: a user-scope meeting type + link; the
    // public slot preview must NOT offer anything overlapping 11:00–13:00.
    let mt: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "Sync Test 60",
                "duration_minutes": 60,
                "buffer_minutes": 0,
                "booking_window_days": 14,
                "assignee_scope": "user",
                "assignee_user_id": my_id,
            })),
        )
        .await,
    )
    .await;
    let mt_id = mt["meeting_type"]["id"].as_str().unwrap().to_string();
    let link: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": mt_id})),
        )
        .await,
    )
    .await;
    let link_token = link["token"].as_str().unwrap().to_string();

    let view: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/public/scheduling-links/{link_token}"),
            None,
            None,
        )
        .await,
    )
    .await;
    let slots = view["slots"].as_array().unwrap().clone();
    assert!(!slots.is_empty(), "free slots exist outside the busy span");
    // No free interval may overlap the external busy event (11:00–13:00).
    let busy_start = format!("{}T11:00:00Z", day);
    let busy_end = format!("{}T13:00:00Z", day);
    for slot in &slots {
        let s = slot["starts_at"].as_str().unwrap();
        let e = slot["ends_at"].as_str().unwrap();
        assert!(
            !(s < busy_end.as_str() && busy_start.as_str() < e),
            "slot {s}..{e} overlaps external busy time {busy_start}..{busy_end}"
        );
    }

    // Booking INSIDE the busy span (12:00–13:00) is refused at book
    // time — the same gate portal bookings run.
    let attempt = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{link_token}/book"),
        None,
        Some(json!({
            "starts_at": format!("{}T12:00:00Z", day),
            "booker_name": "Pam",
        })),
    )
    .await;
    assert_eq!(
        attempt.status(),
        409,
        "a 12:00 slot sits inside external busy time and is refused at book time"
    );

    // A booking fully before the busy span lands fine.
    let ok = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{link_token}/book"),
        None,
        Some(json!({
            "starts_at": format!("{}T09:00:00Z", day),
            "booker_name": "Pam",
        })),
    )
    .await;
    assert_eq!(ok.status(), 201, "{}", body_json(ok).await);

    // The outbound feed now carries the just-created appointment.
    let feed = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/calendar-sync/feed/{feed_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(feed.status(), 200);
    assert_eq!(
        feed.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "text/calendar; charset=utf-8"
    );
    let feed_body = body_text(feed).await;
    assert!(feed_body.starts_with("BEGIN:VCALENDAR"));
    assert!(
        feed_body.contains("BEGIN:VEVENT"),
        "outbound feed carries the created appointment"
    );

    // Unknown token → 404 (indistinguishable from disabled).
    let bad = send(
        &app,
        axum::http::Method::GET,
        "/api/calendar-sync/feed/deadbeef",
        None,
        None,
    )
    .await;
    assert_eq!(bad.status(), 404);
}

#[tokio::test]
async fn re_poll_upserts_moves_and_releases_and_enforces_sequence_wins() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync2-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();

    let day = next_monday_date();
    let uid = "evt-move@external.test";
    let (addr, ics_tx) = spawn_ics_server(fixture_ics(&day, uid, 11, 12, "External busy")).await;

    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/calendar-sync",
            Some(&bearer),
            Some(json!({
                "name": "Poll calendar",
                "provider": "ical",
                "direction": "inbound",
                "feed_url": format!("http://{addr}/calendar.ics"),
            })),
        )
        .await,
    )
    .await;
    assert!(
        created["feed_token"].is_null(),
        "inbound connections mint no outbound token"
    );
    let connection_id = created["connection"]["id"].as_str().unwrap().to_string();
    let sync_uri = format!("/api/calendar-sync/{connection_id}/sync");

    let s1: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &sync_uri,
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s1["outcome"]["imported"], 1, "{s1}");

    // Unchanged re-poll: the duplicate rule counts it, nothing re-applies.
    let s2: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &sync_uri,
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s2["outcome"]["imported"], 0, "{s2}");
    assert_eq!(s2["outcome"]["duplicates"], 1, "{s2}");

    // Moved event with a higher sequence: the exclusion moves in place.
    ics_tx.send_modify(|v| {
        *v = fixture_ics(&day, uid, 14, 15, "External busy").replace("SEQUENCE:0", "SEQUENCE:1")
    });
    let s2b: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &sync_uri,
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s2b["outcome"]["updated"], 1, "{s2b}");
    let events: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/calendar-sync/{connection_id}/events"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(events["events"][0]["starts_at"], format!("{day}T14:00:00Z"));

    // LOWER sequence re-announcement: ignored (highest sequence wins).
    ics_tx.send_modify(|v| {
        *v = fixture_ics(&day, uid, 8, 9, "Stale").replace("SEQUENCE:1", "SEQUENCE:0")
    });
    let s3: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &sync_uri,
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s3["outcome"]["imported"], 0, "{s3}");
    assert_eq!(s3["outcome"]["duplicates"], 1, "{s3}");
    let events: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/calendar-sync/{connection_id}/events"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        events["events"][0]["starts_at"],
        format!("{day}T14:00:00Z"),
        "the stale lower-sequence re-announcement changed nothing"
    );

    // Event disappears from the snapshot: released, slot freed.
    ics_tx.send_modify(|v| *v = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n".to_string());
    let s4: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &sync_uri,
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(s4["outcome"]["released"], 1, "{s4}");
    let events: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/calendar-sync/{connection_id}/events"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(events["events"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn connection_duplicate_name_is_a_typed_409() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync3-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    for _ in 0..2 {
        let res = send(
            &app,
            axum::http::Method::POST,
            "/api/calendar-sync",
            Some(&bearer),
            Some(json!({
                "name": "Same name",
                "provider": "ical",
                "direction": "outbound",
            })),
        )
        .await;
        let first = res.status();
        if first.as_u16() == 201 {
            continue;
        }
        let body = body_json(res).await;
        assert_eq!(first, 409);
        assert_eq!(
            body["error"]["reason"], "calendar_sync_connection_name_taken",
            "IF3: the raw constraint name never crosses the wire"
        );
        return;
    }
    panic!("the duplicate create should have 409ed on the second attempt");
}
