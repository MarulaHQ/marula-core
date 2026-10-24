//! Two-way calendar sync e2e (Wave 14 lane 2).
//!
//! Covers the provider seam end to end: connection lifecycle (incl. the typed
//! duplicate-name 409), inbound snapshot → availability double-check (the public
//! scheduling-link slot preview and the book-time gate must both honor external
//! busy time, exactly like portal bookings), upsert/release on re-poll, the
//! highest-sequence-wins duplicate rule, the outbound token-scoped iCal feed,
//! and — since Wave 54 lane S1a — the poll's refusal of a feed URL the outbound
//! egress guard rejects.
//!
//! The poll's wire is guarded (https only, public host, judged resolution), so a
//! test cannot point it at an in-process loopback fixture. The mirror half runs
//! through `apply_sync`, the same call the route makes after a poll returns, and
//! the wire half is covered by the refusal test here plus the module's own
//! redirect test.

use axum::Router;
use core_api::routes::calendar_sync::{CalendarSyncProvider, ProviderError, ProviderPull};
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use core_domain::entities::CalendarSyncConnection;
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
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("calendar-sync e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with calendar-sync tables");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    let state = AppState {
        plugin_pool: Default::default(),
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(MemoryStore::new()),
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

/// A loopback listener the guard must never reach. It counts what it accepts,
/// so "refused before the request" is a connection count rather than an
/// inference from a status code.
fn counting_listener() -> (
    std::net::SocketAddr,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let addr = listener.local_addr().expect("local addr");
    let seen = std::sync::Arc::new(AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() {
                count.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    (addr, seen)
}

/// Applies one provider snapshot through the mirror — the call the sync route
/// makes after a poll returns.
///
/// The poll's wire is screened by the outbound guard, so a test cannot point it
/// at a loopback fixture. The mirror rules this suite asserts (duplicate,
/// move-in-place, highest-sequence-wins, release) run in the same place either
/// way; the poll's refusal has its own test below.
async fn apply_snapshot(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    connection_id: Uuid,
    events: &[core_domain::entities::ExternalCalendarEvent],
) -> core_domain::entities::SyncOutcome {
    let mut tx = core_api::org_tx::begin(state, org_id, user_id)
        .await
        .expect("org transaction");
    let outcome =
        core_db::calendar_sync::apply_sync(&mut tx.tx, org_id, connection_id, user_id, events)
            .await
            .expect("snapshot applies");
    tx.commit().await.expect("commit");
    outcome
}

#[tokio::test]
async fn inbound_busy_time_gates_booking_like_portal_bookings() {
    let (state, app) = test_state().await;
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
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = my_id.parse().unwrap();

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

    // The external calendar's ICS: one busy event Monday 11:00–13:00 UTC.
    let uid = "evt-busy-1@external.test";
    let snapshot = core_api::routes::calendar_sync::parse_ical(&fixture_ics(
        &day,
        uid,
        11,
        13,
        "External busy",
    ));

    // Create a two-way connection. The feed URL is a public name: a loopback one
    // would be refused by the poll's egress guard, and that refusal has its own
    // test below.
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
                "feed_url": "https://calendar.example.test/feed.ics",
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

    // The snapshot imports as busy time through the same mirror the poll uses.
    let outcome = apply_snapshot(
        &state,
        org_id,
        user_id,
        Uuid::parse_str(&connection_id).unwrap(),
        &snapshot,
    )
    .await;
    assert_eq!(outcome.imported, 1, "the external busy event imports");
    assert_eq!(outcome.overlaps, 0);

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
    let (state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync2-{}", Uuid::now_v7().simple())).await;
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
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();

    let day = next_monday_date();
    let uid = "evt-move@external.test";

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
                "feed_url": "https://calendar.example.test/feed.ics",
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
    let connection_uuid = Uuid::parse_str(&connection_id).unwrap();

    // First snapshot: the event imports as busy time.
    let s1 = apply_snapshot(
        &state,
        org_id,
        user_id,
        connection_uuid,
        &core_api::routes::calendar_sync::parse_ical(&fixture_ics(
            &day,
            uid,
            11,
            12,
            "External busy",
        )),
    )
    .await;
    assert_eq!(s1.imported, 1);

    // Unchanged re-poll: the duplicate rule counts it, nothing re-applies.
    let s2 = apply_snapshot(
        &state,
        org_id,
        user_id,
        connection_uuid,
        &core_api::routes::calendar_sync::parse_ical(&fixture_ics(
            &day,
            uid,
            11,
            12,
            "External busy",
        )),
    )
    .await;
    assert_eq!(s2.imported, 0);
    assert_eq!(s2.duplicates, 1);

    // Moved event with a higher sequence: the exclusion moves in place.
    let s2b = apply_snapshot(
        &state,
        org_id,
        user_id,
        connection_uuid,
        &core_api::routes::calendar_sync::parse_ical(
            &fixture_ics(&day, uid, 14, 15, "External busy").replace("SEQUENCE:0", "SEQUENCE:1"),
        ),
    )
    .await;
    assert_eq!(s2b.updated, 1);
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
    let s3 = apply_snapshot(
        &state,
        org_id,
        user_id,
        connection_uuid,
        &core_api::routes::calendar_sync::parse_ical(
            &fixture_ics(&day, uid, 8, 9, "Stale").replace("SEQUENCE:1", "SEQUENCE:0"),
        ),
    )
    .await;
    assert_eq!(s3.imported, 0);
    assert_eq!(s3.duplicates, 1);
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
    let s4 = apply_snapshot(
        &state,
        org_id,
        user_id,
        connection_uuid,
        &core_api::routes::calendar_sync::parse_ical(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
        ),
    )
    .await;
    assert_eq!(s4.released, 1);
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

/// A feed URL the outbound guard refuses is never requested.
///
/// The loopback listener is the probe: it counts what it accepts, so the
/// assertion is that the poll refused before a socket existed rather than that
/// it happened to fail. The metadata address is the second spelling of the same
/// refusal — a literal the guard classifies without any lookup.
#[tokio::test]
async fn a_loopback_feed_is_refused_before_the_request() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync4-{}", Uuid::now_v7().simple())).await;
    let bearer = tokens["access_token"].as_str().unwrap().to_string();
    let (addr, connections) = counting_listener();

    for (index, feed_url) in [
        format!("http://{addr}/calendar.ics"),
        "https://169.254.169.254/latest/meta-data/".to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        let created: Value = body_json(
            send(
                &app,
                axum::http::Method::POST,
                "/api/calendar-sync",
                Some(&bearer),
                Some(json!({
                    "name": format!("Refused feed {index}"),
                    "provider": "ical",
                    "direction": "inbound",
                    "feed_url": feed_url,
                })),
            )
            .await,
        )
        .await;
        let connection_id = created["connection"]["id"].as_str().unwrap().to_string();

        let synced = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/calendar-sync/{connection_id}/sync"),
            Some(&bearer),
            None,
        )
        .await;
        assert_eq!(
            synced.status(),
            503,
            "a refused feed is the typed unavailable answer, not a panic or a hop: {feed_url}"
        );

        let listed: Value = body_json(
            send(
                &app,
                axum::http::Method::GET,
                "/api/calendar-sync",
                Some(&bearer),
                None,
            )
            .await,
        )
        .await;
        let row = listed
            .as_array()
            .expect("connections array")
            .iter()
            .find(|c| c["id"] == connection_id.as_str())
            .expect("the connection stays listed");
        assert!(
            row["last_error"].as_str().is_some_and(|e| !e.is_empty()),
            "the refusal is recorded on the connection: {row}"
        );
    }

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        connections.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the refused target was never connected to"
    );
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

/// A provider that answers with a parsed fixture and never touches a socket.
///
/// The parsing is the real one (`parse_ical`), so the snapshot this returns is
/// byte-for-byte what a guarded poll would return; only the wire is absent.
struct FixtureProvider {
    body: String,
    token: Option<String>,
}

#[async_trait::async_trait]
impl CalendarSyncProvider for FixtureProvider {
    fn kind(&self) -> &'static str {
        "ical"
    }

    async fn fetch_events(
        &self,
        _connection: &CalendarSyncConnection,
    ) -> Result<ProviderPull, ProviderError> {
        Ok(ProviderPull {
            events: core_api::routes::calendar_sync::parse_ical(&self.body),
            sync_token: self.token.clone(),
        })
    }
}

/// The success branch of a poll, through the same seam the route calls.
///
/// The refusal path is covered above; this covers what happens when a pull
/// returns: the mirror, the recorded result and the emitted event. The
/// connection is read with the same org-scoped query the route uses, so the
/// only thing this test replaces is the network.
#[tokio::test]
async fn a_successful_pull_mirrors_events_through_the_seam() {
    let (state, app) = test_state().await;
    let tokens = register_org(&app, format!("csync5-{}", Uuid::now_v7().simple())).await;
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
    let org_id: Uuid = me["org_id"].as_str().unwrap().parse().unwrap();
    let user_id: Uuid = me["user_id"].as_str().unwrap().parse().unwrap();

    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/calendar-sync",
            Some(&bearer),
            Some(json!({
                "name": "Seam calendar",
                "provider": "ical",
                "direction": "inbound",
                "feed_url": "https://calendar.example.test/seam.ics",
            })),
        )
        .await,
    )
    .await;
    let connection_id: Uuid = created["connection"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let connection = {
        let mut tx = state.pool.begin().await.unwrap();
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .unwrap();
        let connection = core_db::calendar_sync::get_connection(&mut tx, org_id, connection_id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        connection
    };

    let day = next_monday_date();
    let provider = FixtureProvider {
        body: fixture_ics(&day, "seam-evt@external.test", 10, 11, "Seam busy"),
        token: Some("fixture-etag".to_string()),
    };

    let outcome = core_api::routes::calendar_sync::sync_with_provider(
        &state,
        org_id,
        user_id,
        &connection,
        &provider,
    )
    .await
    .expect("a successful pull");
    assert_eq!(outcome.connection_id, connection_id);
    assert_eq!(outcome.outcome.imported, 1);
    assert_eq!(outcome.outcome.updated, 0);

    let listed: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/calendar-sync",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let row = listed
        .as_array()
        .expect("connections array")
        .iter()
        .find(|c| c["id"] == connection_id.to_string().as_str())
        .expect("the connection is listed");
    assert!(
        row["last_synced_at"].is_string(),
        "the poll recorded its success: {row}"
    );
    assert!(
        row["last_error"].is_null(),
        "a good pull leaves no error copy: {row}"
    );

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
    assert_eq!(events["events"][0]["title"], "Seam busy");

    // The same snapshot again is the update path, not a second import.
    let again = core_api::routes::calendar_sync::sync_with_provider(
        &state,
        org_id,
        user_id,
        &connection,
        &provider,
    )
    .await
    .expect("a second pull");
    assert_eq!(again.outcome.imported, 0);
    assert_eq!(again.outcome.duplicates, 1);
}
