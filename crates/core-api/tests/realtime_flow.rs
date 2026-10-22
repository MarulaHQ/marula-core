//! Live-delivery + presence contract tests (Wave 12 lane 12), driven
//! through the REAL core-api router against a real Postgres with the
//! migration-0068 NOTIFY trigger active and the dedicated PgListener task
//! running — the full commit→NOTIFY→fan-out→SSE pipeline.
//!
//! Pinned behavior:
//!
//! * **Fan-out** — a mutation's contract event reaches an open SSE
//!   subscriber in the SAME org within commit latency, carrying the
//!   `{type, org_id, entity, entity_id, seq, at}` wire frame with the
//!   per-entity monotonic seq.
//! * **Org isolation** — org filtering happens at fan-out: a subscriber
//!   never receives another org's frames, no matter what that org
//!   commits.
//! * **Per-user connection cap** — the 9th concurrent stream for one user
//!   is a typed 429; closed streams free capacity.
//! * **Presence TTL** — heartbeats register viewers; entries expire after
//!   the TTL (this suite runs with a 1s TTL); roster reads are pruned;
//!   transitions ride the stream as ephemeral `type: "presence"` frames.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

// The presence TTL is read once per process (realtime::presence is a
// global): every test in this binary runs with the 1-second TTL.
fn init_env() {
    std::env::set_var("MARULA_PRESENCE_TTL_SECS", "1");
}

async fn test_state() -> (AppState, Router, String) {
    init_env();
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("realtime e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url.clone();
    let state = AppState {
        plugin_pool: Default::default(),
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
    (state, app, admin_url)
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
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("body_json parse failed ({e})"))
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
            "org_name": "Realtime Co",
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

/// A second, non-owner user in the same org with a staff role, logged in.
async fn add_same_org_user(app: &Router, state: &AppState, org: &Org, label: &str) -> String {
    let email = format!("{label}-{}@realtime.test", Uuid::now_v7().simple());
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
    .expect("seed user");
    // Owner role row so the roster read is authorized; presence needs no
    // specific permission beyond auth.
    let _ = sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT $1, id FROM roles WHERE org_id = $2 LIMIT 1")
        .bind(core_db::users::find_by_email_in_org(&state.pool, org.org_id, &email)
            .await
            .expect("find user")
            .expect("user exists")
            .id)
        .bind(org.org_id)
        .execute(&state.pool)
        .await;
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

async fn create_ticket(app: &Router, token: &str, title: &str) -> Uuid {
    let boards = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/boards",
            Some(token),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().expect("default board").to_string();
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/tickets",
        Some(token),
        Some(json!({
            "board_id": board_id,
            "title": title,
            "body": "rt body",
            "priority": "p2",
        })),
    )
    .await;
    let body = body_json(response).await;
    body["ticket"]["id"]
        .as_str()
        .or(body["id"].as_str())
        .unwrap()
        .parse()
        .unwrap()
}

/// Opens the SSE stream (ticket-scoped) and returns the streaming body
/// once the subscriber is registered inside the hub.
async fn open_stream(app: &Router, token: &str, ticket: Uuid) -> axum::body::Body {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/events/stream?ticket={ticket}"),
        Some(token),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "stream subscribe must succeed within the connection cap"
    );
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "text/event-stream",
        "SSE content type"
    );
    response.into_body()
}

/// Reads data frames off the SSE stream until one decodes as JSON matching
/// `predicate` (keepalive comment bytes are skipped). Fails after
/// `timeout`.
async fn next_frame_matching(
    stream: &mut axum::body::BodyDataStream,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut buffer = String::new();
    loop {
        let chunk = tokio::time::timeout_at(deadline, stream.next())
            .await
            .expect("timed out waiting for a matching stream frame")
            .expect("stream ended before a matching frame arrived")
            .expect("stream read error");
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(line_end) = buffer.find("\n\n") {
            let raw_frame: String = buffer.drain(..line_end + 2).collect();
            for line in raw_frame.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if predicate(&v) {
                            return v;
                        }
                    }
                }
            }
        }
    }
}

/// Warm the LISTEN connection: the hub's subscriber is registered by the
/// time the stream opens, but the dedicated PgListener may still be
/// dialing. Loop a cheap mutation until a frame round-trips, which proves
/// the commit→NOTIFY→fan-out path is live; later assertions are then
/// deterministic.
async fn warm_listener(
    app: &Router,
    token: &str,
    ticket: Uuid,
    stream: &mut axum::body::BodyDataStream,
) {
    for attempt in 0..10 {
        let response = send(
            app,
            axum::http::Method::PATCH,
            &format!("/api/tickets/{ticket}"),
            Some(token),
            Some(json!({"title": format!("warm {attempt}")})),
        )
        .await;
        assert_eq!(response.status(), 200, "warm patch must succeed");
        let seen = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            let mut buffer = String::new();
            loop {
                match tokio::time::timeout_at(deadline, stream.next()).await {
                    Ok(Some(Ok(chunk))) => buffer.push_str(&String::from_utf8_lossy(&chunk)),
                    _ => break,
                }
                if buffer.contains("data: ") {
                    break;
                }
            }
            buffer.contains("data: ")
        };
        if seen {
            return;
        }
    }
    panic!("listener never delivered a warm-up event; NOTIFY pipeline is not live");
}

#[tokio::test]
async fn committed_mutation_should_fan_out_to_same_org_sse_subscriber_with_seq() {
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rt-fanout-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Fanout ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket)
        .await
        .into_data_stream();
    warm_listener(&app, &org.token, ticket, &mut stream_body).await;

    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket}"),
        Some(&org.token),
        Some(json!({"title": "concurrent edit landed"})),
    )
    .await;
    assert_eq!(patch.status(), 200);

    let frame = next_frame_matching(&mut stream_body, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
    })
    .await;
    assert_eq!(frame["org_id"], org.org_id.to_string(), "org on the frame");
    assert_eq!(frame["entity"], "ticket");
    assert!(
        frame["seq"].as_i64().is_some_and(|s| s > 0),
        "contract-event frames carry the per-entity ledger seq: {frame}"
    );
    assert!(
        frame["at"].as_str().is_some(),
        "frames carry the commit timestamp: {frame}"
    );
}

#[tokio::test]
async fn other_orgs_events_should_never_reach_a_subscriber() {
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org_a = register_org(&app, format!("rt-iso-a-{}", Uuid::now_v7().simple())).await;
    let org_b = register_org(&app, format!("rt-iso-b-{}", Uuid::now_v7().simple())).await;
    let ticket_a = create_ticket(&app, &org_a.token, "Org A ticket").await;
    let ticket_b = create_ticket(&app, &org_b.token, "Org B ticket").await;

    let mut stream_b = open_stream(&app, &org_b.token, ticket_b)
        .await
        .into_data_stream();
    warm_listener(&app, &org_b.token, ticket_b, &mut stream_b).await;

    // Org A commits a mutation while org B is subscribed to ITS ticket.
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_a}"),
        Some(&org_a.token),
        Some(json!({"title": "org A private edit"})),
    )
    .await;
    assert_eq!(patch.status(), 200);

    // Drain B's stream for a quiet window: nothing about org A may arrive
    // (org filtering is at fan-out, not at subscribe).
    let mut leak = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, stream_b.next()).await {
        leak.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(
        !leak.contains(&ticket_a.to_string()),
        "cross-org leak: org B stream received org A's event: {leak}"
    );

    // Positive control: B's own commits still arrive after the quiet window.
    let mut stream_b2 = open_stream(&app, &org_b.token, ticket_b)
        .await
        .into_data_stream();
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket_b}"),
        Some(&org_b.token),
        Some(json!({"title": "org B own edit"})),
    )
    .await;
    assert_eq!(response.status(), 200);
    // Either stream may have captured it; assert across both buffers by
    // just waiting on the fresh one.
    let _ = next_frame_matching(&mut stream_b2, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket_b.to_string()
    })
    .await;
}

#[tokio::test]
async fn connection_cap_should_reject_the_ninth_stream_and_free_on_close() {
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rt-cap-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Cap ticket").await;

    let mut streams = Vec::new();
    for _ in 0..8 {
        let response = send(
            &app,
            axum::http::Method::GET,
            &format!("/api/events/stream?ticket={ticket}"),
            Some(&org.token),
            None,
        )
        .await;
        assert_eq!(response.status(), 200, "streams under the cap succeed");
        streams.push(response);
    }
    let ninth = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/events/stream?ticket={ticket}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(
        ninth.status(),
        429,
        "the ninth concurrent stream hits the per-user cap"
    );

    // Dropping the open streams frees capacity for new subscribers.
    drop(streams);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reclaimed = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/events/stream?ticket={ticket}"),
        Some(&org.token),
        None,
    )
    .await;
    assert_eq!(
        reclaimed.status(),
        200,
        "closed streams free per-user capacity"
    );
}

#[tokio::test]
async fn presence_should_heartbeat_roster_expire_and_push_transition_frames() {
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rt-presence-{}", Uuid::now_v7().simple())).await;
    let peer = add_same_org_user(&app, &_state, &org, "peer").await;
    let ticket = create_ticket(&app, &org.token, "Presence ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket)
        .await
        .into_data_stream();
    warm_listener(&app, &org.token, ticket, &mut stream_body).await;

    // Peer starts heartbeating as an editor.
    let hb = send(
        &app,
        axum::http::Method::POST,
        "/api/presence/heartbeat",
        Some(&peer),
        Some(json!({"entity": "ticket", "entity_id": ticket.to_string(), "editing": true})),
    )
    .await;
    assert_eq!(hb.status(), 200, "heartbeat succeeds");
    let roster = body_json(hb).await;
    let viewers = roster["viewers"].as_array().unwrap();
    assert_eq!(viewers.len(), 1, "one viewer right after the heartbeat");
    assert_eq!(viewers[0]["editing"], true, "editing flag surfaces");
    assert!(
        viewers[0]["name"].is_string() && viewers[0]["user_id"].is_string(),
        "roster carries user id + name: {viewers:?}"
    );

    // The join transition rides the stream as an ephemeral presence frame.
    let frame = next_frame_matching(&mut stream_body, |v| v["type"] == "presence").await;
    assert_eq!(frame["entity_id"], ticket.to_string());
    let viewers = frame["viewers"].as_array().expect("presence roster");
    assert!(
        viewers.iter().any(|v| v["editing"] == true),
        "presence frame carries the roster: {frame}"
    );

    // Roster read answers "who else is working on this ticket".
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket}/presence"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(roster["viewers"].as_array().unwrap().len(), 1);

    // No heartbeat => TTL expiry => empty roster (this binary runs with a
    // 1s TTL).
    tokio::time::sleep(Duration::from_millis(1400)).await;
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket}/presence"),
            Some(&org.token),
            None,
        )
        .await,
    )
    .await;
    assert!(
        roster["viewers"].as_array().unwrap().is_empty(),
        "presence entries expire after the TTL: {roster}"
    );

    // Presence stays out of the contract trail: no outbox row was written
    // for presence frames (the trigger only fires on outbox_events).
    let outbox: (i64,) =
        sqlx::query_as("SELECT count(*) FROM outbox_events WHERE event_type = 'presence'")
            .fetch_one(&_state.pool)
            .await
            .expect("outbox count");
    assert_eq!(
        outbox.0, 0,
        "presence is ephemeral — never a contract event"
    );
}

#[tokio::test]
async fn ticket_presence_should_404_for_invisible_tickets() {
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org_a = register_org(&app, format!("rt-vis-a-{}", Uuid::now_v7().simple())).await;
    let org_b = register_org(&app, format!("rt-vis-b-{}", Uuid::now_v7().simple())).await;
    let ticket_a = create_ticket(&app, &org_a.token, "Org A only").await;

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_a}/presence"),
        Some(&org_b.token),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        404,
        "presence roster is org-scoped: another org's ticket is invisible"
    );

    let unauth = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{ticket_a}/presence"),
        None,
        None,
    )
    .await;
    assert_eq!(unauth.status(), 401, "presence requires bearer auth");
}
