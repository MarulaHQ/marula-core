//! SSE resume + cross-replica presence relay contract tests (Wave 13
//! lane 10), driven through the REAL core-api router against a real
//! Postgres — mirroring `realtime_flow.rs` (same commit→NOTIFY→fan-out→SSE
//! pipeline, plus the outbox replay path and the `meridian_presence`
//! relay channel).
//!
//! Pinned behavior:
//!
//! * **SSE `id:` on every frame** — contract-event frames carry the
//!   outbox row's uuid as the SSE `id:` field; presence frames a
//!   per-frame uuid. The JSON `data:` contract is unchanged.
//! * **Last-Event-ID resume** — reconnecting with the last seen id
//!   replays the missed events from the outbox (bounded replay buffer),
//!   each with its `id:` field, before live delivery resumes.
//! * **Cutoff** — a cursor further back than the replay buffer gets NO
//!   replay (an explicit comment marks it); the client falls back to
//!   full snapshot rehydration. Unknown/presence uuids degrade the same
//!   way, without the cutoff comment.
//! * **Presence relay** — a heartbeat publishes a `meridian_presence`
//!   NOTIFY (cross-replica relay); applying a relayed heartbeat (what
//!   another replica's listener does) fans out the IDENTICAL presence
//!   frame shape and converges the roster, with TTL semantics intact.
//!
//! This binary runs with a 1s presence TTL and an 8-event replay buffer
//! (both read once per process) so the cutoff is testable.

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

fn init_env() {
    std::env::set_var("MERIDIAN_PRESENCE_TTL_SECS", "1");
    std::env::set_var("MERIDIAN_SSE_REPLAY_BUFFER", "8");
}

/// The hub, presence store, and every spawned listener are PROCESS-GLOBAL
/// (each test spawns its own run_listener, and every listener publishes
/// everything it receives to the shared hub) — parallel tests would feed
/// each other stale notifications mid-drain. Serialize the suite.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn test_state() -> (AppState, Router, String) {
    init_env();
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MERIDIAN_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("realtime e2e needs MERIDIAN_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url.clone();
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
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(method.clone())
        .uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
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
    user_id: Uuid,
}

async fn register_org(app: &Router, slug: String) -> Org {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        &[],
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
    let me = body_json(
        send(
            app,
            axum::http::Method::GET,
            "/api/me",
            Some(&token),
            &[],
            None,
        )
        .await,
    )
    .await;
    Org {
        token,
        org_id: me["org_id"].as_str().unwrap().parse().unwrap(),
        user_id: me["user_id"].as_str().unwrap().parse().unwrap(),
    }
}

/// A second, non-owner user in the same org (presence needs no specific
/// permission beyond auth, so no role row is seeded).
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
    let login = body_json(
        send(
            app,
            axum::http::Method::POST,
            "/api/auth/login",
            None,
            &[],
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
            &[],
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
        &[],
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

/// One parsed SSE frame: the `id:` field (None when absent) plus the
/// concatenated `data:` JSON.
#[derive(Debug, Clone)]
struct SseFrame {
    id: Option<String>,
    data: Option<Value>,
}

fn parse_sse_frame(raw: &str) -> SseFrame {
    let mut id = None;
    let mut data = String::new();
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("id: ") {
            id = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("data: ") {
            data.push_str(v);
        }
    }
    SseFrame {
        id,
        data: if data.is_empty() {
            None
        } else {
            serde_json::from_str(&data).ok()
        },
    }
}

/// Opens the SSE stream (ticket-scoped, optional Last-Event-ID) and
/// returns the streaming body once the subscriber is registered.
async fn open_stream(
    app: &Router,
    token: &str,
    ticket: Uuid,
    last_event_id: Option<&str>,
) -> axum::body::Body {
    let mut headers: Vec<(&str, &str)> = Vec::new();
    if let Some(id) = last_event_id {
        headers.push(("last-event-id", id));
    }
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/events/stream?ticket={ticket}"),
        Some(token),
        &headers,
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

/// Reads complete SSE frames until one's data matches `predicate`
/// (keepalive comments are skipped). Returns that parsed frame. Fails
/// after 15s.
async fn next_frame_matching(
    stream: &mut axum::body::BodyDataStream,
    predicate: impl Fn(&Value) -> bool,
) -> SseFrame {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut buffer = String::new();
    loop {
        let chunk = tokio::time::timeout_at(deadline, stream.next())
            .await
            .expect("timed out waiting for a matching stream frame")
            .expect("stream ended before a matching frame arrived")
            .expect("stream read error");
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(frame_end) = buffer.find("\n\n") {
            let raw: String = buffer.drain(..frame_end + 2).collect();
            let frame = parse_sse_frame(&raw);
            if let Some(data) = &frame.data {
                if predicate(data) {
                    return frame;
                }
            }
        }
    }
}

/// Drains the stream for `window`, collecting everything that arrives
/// (frames AND comments) — for "nothing replayed" assertions.
async fn drain(stream: &mut axum::body::BodyDataStream, window: Duration) -> String {
    let deadline = tokio::time::Instant::now() + window;
    let mut collected = String::new();
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, stream.next()).await {
        collected.push_str(&String::from_utf8_lossy(&chunk));
    }
    collected
}

/// Flushes frames already queued for the subscriber: reads until `window`
/// passes with nothing arriving. Call after warm-up so a subsequent
/// cursor capture can't mistake a leftover warm-up frame for the cursor.
async fn flush_pending(stream: &mut axum::body::BodyDataStream, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(_chunk))) = tokio::time::timeout_at(deadline, stream.next()).await {
        // keep the deadline; drain until quiet
    }
}

/// Warm the LISTEN connection: loop a cheap mutation until a frame
/// round-trips, proving the commit→NOTIFY→fan-out path is live.
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
            &[],
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

async fn patch_title(app: &Router, token: &str, ticket: Uuid, title: &str) {
    let response = send(
        app,
        axum::http::Method::PATCH,
        &format!("/api/tickets/{ticket}"),
        Some(token),
        &[],
        Some(json!({"title": title})),
    )
    .await;
    assert_eq!(response.status(), 200, "patch must succeed");
}

#[tokio::test]
async fn stream_should_carry_sse_id_and_replay_missed_events_on_last_event_id() {
    let _serial = SERIAL.lock().await;
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rr-resume-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Resume ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket, None)
        .await
        .into_data_stream();
    warm_listener(&app, &org.token, ticket, &mut stream_body).await;
    flush_pending(&mut stream_body, Duration::from_millis(300)).await;

    // Capture the last delivered frame's SSE id as the resume cursor.
    patch_title(&app, &org.token, ticket, "cursor edit").await;
    let cursor_frame = next_frame_matching(&mut stream_body, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
    })
    .await;
    // The data JSON contract is unchanged (no `id` leaked into data).
    assert!(
        cursor_frame.data.as_ref().unwrap().get("id").is_none(),
        "the event uuid rides the SSE id: field, never the JSON payload"
    );
    let cursor = cursor_frame.id.expect("every frame carries an SSE id");
    assert!(
        Uuid::parse_str(&cursor).is_ok(),
        "the SSE id is the event uuid: {cursor}"
    );
    let cursor_seq = cursor_frame.data.as_ref().unwrap()["seq"].as_i64().unwrap();

    // Disconnect, then commit 3 edits the client misses.
    drop(stream_body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    for n in 1..=3 {
        patch_title(&app, &org.token, ticket, &format!("missed edit {n}")).await;
    }

    // Reconnect with Last-Event-ID: the missed events replay, each with
    // its id: field, before live delivery resumes.
    let mut resumed = open_stream(&app, &org.token, ticket, Some(&cursor))
        .await
        .into_data_stream();
    let mut missed_seqs = Vec::new();
    let mut ids_all_uuid = true;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while missed_seqs.len() < 3 && tokio::time::Instant::now() < deadline {
        let frame = next_frame_matching(&mut resumed, |v| {
            v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
        })
        .await;
        ids_all_uuid &= frame.id.is_some_and(|id| Uuid::parse_str(&id).is_ok());
        let seq = frame.data.as_ref().unwrap()["seq"].as_i64().unwrap();
        if seq > cursor_seq {
            missed_seqs.push(seq);
        }
    }
    assert_eq!(
        missed_seqs.len(),
        3,
        "all three missed edits replay from the outbox (seq > cursor): {missed_seqs:?}"
    );
    assert!(ids_all_uuid, "replayed frames carry uuid SSE ids");
    // Live delivery resumes after the replay: a new commit still arrives.
    patch_title(&app, &org.token, ticket, "live after resume").await;
    let live = next_frame_matching(&mut resumed, |v| {
        v["type"] == "ticket.updated"
            && v["entity_id"] == ticket.to_string()
            && v["seq"]
                .as_i64()
                .is_some_and(|s| s > *missed_seqs.iter().max().unwrap())
    })
    .await;
    assert!(live.id.is_some(), "live frames keep their SSE id");
}

#[tokio::test]
async fn cursor_past_the_replay_buffer_should_replay_nothing_and_mark_the_cutoff() {
    let _serial = SERIAL.lock().await;
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rr-cutoff-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Cutoff ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket, None)
        .await
        .into_data_stream();
    warm_listener(&app, &org.token, ticket, &mut stream_body).await;
    flush_pending(&mut stream_body, Duration::from_millis(300)).await;

    patch_title(&app, &org.token, ticket, "cutoff cursor edit").await;
    let cursor_frame = next_frame_matching(&mut stream_body, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
    })
    .await;
    let cursor = cursor_frame.id.expect("cursor id");
    drop(stream_body);

    // 10 edits separate the cursor from now — past the 8-event buffer
    // this binary runs with.
    for n in 0..10 {
        patch_title(&app, &org.token, ticket, &format!("deep history {n}")).await;
    }

    let mut resumed = open_stream(&app, &org.token, ticket, Some(&cursor))
        .await
        .into_data_stream();
    let collected = drain(&mut resumed, Duration::from_millis(1500)).await;
    assert!(
        !collected.contains("data: "),
        "a cursor past the replay buffer must NOT replay (a partial replay \
         that skips events would corrupt reconciliation): {collected}"
    );
    assert!(
        collected.contains("replay buffer"),
        "the cutoff is explicitly marked so the client rehydrates: {collected}"
    );

    // The stream is still live after the cutoff.
    patch_title(&app, &org.token, ticket, "live after cutoff").await;
    let _ = next_frame_matching(&mut resumed, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
    })
    .await;
}

#[tokio::test]
async fn unknown_or_presence_cursor_should_degrade_to_no_replay_without_cutoff_mark() {
    let _serial = SERIAL.lock().await;
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rr-unknown-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Unknown cursor ticket").await;

    // An id that never existed in this org's outbox: no replay, and NOT
    // a buffer cutoff (a plain nothing-missed resume).
    let unknown = Uuid::now_v7().to_string();
    let mut resumed = open_stream(&app, &org.token, ticket, Some(&unknown))
        .await
        .into_data_stream();
    let collected = drain(&mut resumed, Duration::from_millis(1200)).await;
    assert!(
        !collected.contains("data: "),
        "unknown cursor replays nothing: {collected}"
    );
    assert!(
        !collected.contains("replay buffer"),
        "an unknown cursor is not a buffer cutoff: {collected}"
    );

    // A presence-frame uuid (ephemeral, never in the outbox): same
    // degradation — replaying presence would be meaningless.
    let mut resumed2 = open_stream(&app, &org.token, ticket, Some(&Uuid::now_v7().to_string()))
        .await
        .into_data_stream();
    let collected = drain(&mut resumed2, Duration::from_millis(800)).await;
    assert!(
        !collected.contains("data: "),
        "presence cursor: {collected}"
    );

    // Streams stay live after both degradations.
    patch_title(&app, &org.token, ticket, "live after unknown").await;
    let _ = next_frame_matching(&mut resumed, |v| {
        v["type"] == "ticket.updated" && v["entity_id"] == ticket.to_string()
    })
    .await;
}

/// A PgListener on the presence relay channel, collecting NOTIFY
/// payloads — stands in for ANOTHER core-api replica's listener.
struct PresenceRelayListener {
    rx: tokio::sync::mpsc::UnboundedReceiver<Value>,
}

impl PresenceRelayListener {
    async fn connect(database_url: &str) -> Self {
        let mut listener = sqlx::postgres::PgListener::connect(database_url)
            .await
            .expect("presence relay listener");
        listener
            .listen(core_api::realtime::PRESENCE_CHANNEL)
            .await
            .expect("listen meridian_presence");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(n) => {
                        let payload: Value =
                            serde_json::from_str(n.payload()).expect("heartbeat payload json");
                        tx.send(payload).expect("collector alive");
                    }
                    Err(e) => panic!("presence relay listener error: {e}"),
                }
            }
        });
        Self { rx }
    }

    async fn next_payload(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(10), self.rx.recv())
            .await
            .expect("timed out waiting for a presence relay NOTIFY")
            .expect("relay collector dropped")
    }
}

#[tokio::test]
async fn presence_heartbeat_should_relay_and_remote_apply_should_fan_out_identical_frames() {
    let _serial = SERIAL.lock().await;
    let (state, app, admin_url) = test_state().await;
    let listener_url = admin_url.clone();
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rr-relay-{}", Uuid::now_v7().simple())).await;
    let peer = add_same_org_user(&app, &state, &org, "peer").await;
    let ticket = create_ticket(&app, &org.token, "Relay ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket, None)
        .await
        .into_data_stream();

    // Stand-in for another replica's listener: collect the relay NOTIFY.
    let mut relay = PresenceRelayListener::connect(&listener_url).await;

    // Heartbeat through the ROUTE (replica A): the local roster answers,
    // and a relay NOTIFY lands on meridian_presence.
    let hb = send(
        &app,
        axum::http::Method::POST,
        "/api/presence/heartbeat",
        Some(&peer),
        &[],
        Some(json!({"entity": "ticket", "entity_id": ticket.to_string(), "editing": false})),
    )
    .await;
    assert_eq!(hb.status(), 200);
    let roster = body_json(hb).await;
    let peer_user = roster["viewers"][0]["user_id"]
        .as_str()
        .unwrap()
        .to_string();

    let payload = relay.next_payload().await;
    assert_eq!(
        payload["org_id"],
        org.org_id.to_string(),
        "relay NOTIFY is org-keyed: {payload}"
    );
    assert_eq!(payload["user_id"], peer_user, "relay carries the viewer");
    assert_eq!(payload["entity"], "ticket");
    assert_eq!(payload["entity_id"], ticket.to_string());
    assert_eq!(payload["editing"], false);
    assert!(payload["name"].is_string() && payload["at"].is_string());

    // Replica B's listener applies the relayed heartbeat: the transition
    // fans out with the IDENTICAL frame shape to B's subscribers —
    // simulated here by the same apply seam the listener uses, for a
    // user replica B has not seen yet (the owner).
    core_api::realtime::presence().apply_remote_heartbeat(
        serde_json::from_value(json!({
            "org_id": org.org_id,
            "user_id": org.user_id,
            "name": "Owner",
            "entity": "ticket",
            "entity_id": ticket.to_string(),
            "editing": true,
            "at": chrono::Utc::now()
        }))
        .expect("HeartbeatNotify"),
    );
    let frame = next_frame_matching(&mut stream_body, |v| {
        v["type"] == "presence" && v["viewers"].as_array().is_some_and(|vs| vs.len() == 2)
    })
    .await;
    let viewers = frame.data.as_ref().unwrap()["viewers"]
        .as_array()
        .expect("presence roster");
    assert!(
        viewers.iter().any(|v| v["user_id"] == peer_user.as_str()),
        "rostered viewer converges across replicas: {viewers:?}"
    );
    assert!(
        viewers
            .iter()
            .any(|v| v["user_id"] == org.user_id.to_string() && v["editing"] == true),
        "the remote-applied viewer joins with its editing flag: {viewers:?}"
    );
    assert!(
        viewers
            .iter()
            .all(|v| v["name"].is_string() && v["at"].is_string()),
        "identical frame shape to a local transition: {viewers:?}"
    );
    assert!(
        frame.data.as_ref().unwrap().get("seq").is_none(),
        "presence frames carry no seq (ephemeral, never the ledger)"
    );

    // The converged roster reads identically on "replica B".
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket}/presence"),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert_eq!(roster["viewers"].as_array().unwrap().len(), 2);

    // TTL semantics unchanged: no further heartbeat => expiry on BOTH
    // sides of the relay (this binary runs a 1s TTL).
    tokio::time::sleep(Duration::from_millis(1400)).await;
    let roster = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/tickets/{ticket}/presence"),
            Some(&org.token),
            &[],
            None,
        )
        .await,
    )
    .await;
    assert!(
        roster["viewers"].as_array().unwrap().is_empty(),
        "relayed presence entries expire on the same TTL: {roster}"
    );
}

#[tokio::test]
async fn remote_editing_flip_should_relay_as_a_transition_frame() {
    let _serial = SERIAL.lock().await;
    let (_state, app, admin_url) = test_state().await;
    tokio::spawn(core_api::realtime::run_listener(admin_url));

    let org = register_org(&app, format!("rr-flip-{}", Uuid::now_v7().simple())).await;
    let ticket = create_ticket(&app, &org.token, "Flip ticket").await;
    let mut stream_body = open_stream(&app, &org.token, ticket, None)
        .await
        .into_data_stream();

    // Owner joins (editing=false), directly through the store — the
    // local half of the join transition.
    let hb = send(
        &app,
        axum::http::Method::POST,
        "/api/presence/heartbeat",
        Some(&org.token),
        &[],
        Some(json!({"entity": "ticket", "entity_id": ticket.to_string(), "editing": false})),
    )
    .await;
    assert_eq!(hb.status(), 200);
    let _ = next_frame_matching(&mut stream_body, |v| v["type"] == "presence").await;

    // ANOTHER replica relays the same user flipping editing=true: the
    // editing flip is a transition and fans out as a presence frame
    // carrying the flipped roster — identical shape, no duplicates.
    core_api::realtime::presence().apply_remote_heartbeat(core_api::realtime::HeartbeatNotify {
        org_id: org.org_id,
        user_id: org.user_id,
        name: "Owner".into(),
        entity: "ticket".into(),
        entity_id: ticket.to_string(),
        editing: true,
        at: chrono::Utc::now(),
    });
    let frame = next_frame_matching(&mut stream_body, |v| {
        v["type"] == "presence"
            && v["viewers"]
                .as_array()
                .is_some_and(|vs| vs.iter().any(|v| v["editing"] == true))
    })
    .await;
    assert_eq!(
        frame.data.as_ref().unwrap()["viewers"][0]["editing"],
        true,
        "editing flip from the relay fans out: {frame:?}"
    );

    // A re-applied unchanged heartbeat is NOT a transition: no extra
    // frame (the origin's own NOTIFY round-trip must stay silent).
    core_api::realtime::presence().apply_remote_heartbeat(core_api::realtime::HeartbeatNotify {
        org_id: org.org_id,
        user_id: org.user_id,
        name: "Owner".into(),
        entity: "ticket".into(),
        entity_id: ticket.to_string(),
        editing: true,
        at: chrono::Utc::now(),
    });
    let collected = drain(&mut stream_body, Duration::from_millis(800)).await;
    assert!(
        !collected.contains("\"type\":\"presence\""),
        "a no-op relayed heartbeat must not duplicate the transition frame: {collected}"
    );
}
