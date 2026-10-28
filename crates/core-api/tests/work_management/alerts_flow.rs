//! Alert-ingestor endpoint flows (the private build tracker Phase 3) through the REAL
//! core-api router — the Phase-3 acceptance for this lane:
//!
//! **HMAC webhook ingest → dedup/correlation → incident tickets (a storm
//! collapses to ONE ticket).**
//!
//! Covered here:
//! 1. Ingest auth shell: unknown source (404), disabled source (403),
//!    missing/bad signature (401), tampered body with a stale valid
//!    signature (401), malformed payload (422) — typed reasons throughout.
//! 2. Dedup: replaying the SAME signed delivery returns the stored alert
//!    (`duplicate`) and writes nothing — one ticket, one alert row.
//! 3. Storm collapse: a burst of related alerts (same correlation_key)
//!    folds into the ONE incident ticket; `alert_count` grows; a NEW
//!    correlation key anchors a fresh incident ticket.
//! 4. Wave-8a envelope on the staff surface: source create is
//!    idempotency-keyed with read-your-write mutation metadata; same key +
//!    different payload is a typed 409; stale version is a typed 409
//!    `version_conflict`; the secret is returned exactly once.
//! 5. Audit trail: `alert.ingested` + `alert.storm_collapsed` rows, plus
//!    `alert.ingested` outbox events on the wire.
//! 6. Dual-mount parity for the ingest route (`/api/v1` + legacy `/api`).
//! 7. Profile boundary (Wave 55, decision 8): a source promoted to the
//!    generic M2M profile is refused at this door with a typed 409 that
//!    names `/api/webhooks/ingest`, writes nothing, and records
//!    `alert.refused`; an absent profile stays the classic alert source and
//!    keeps ingesting. The generic door is the mirror (`webhook_receiver_flow`).
//!
//! Non-purpose: worker/queue paths (queue_runner is untouched —
//! correlation is synchronous), SLA ladder math (core-db sla suite).
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{auth, body_json, register_org_token, send_headers as send, test_state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    // Deterministic app-level key: source HMAC secrets are sealed with it
    // on create and opened with it on ingest — same AppState, both sides.
    test_state_with(|config| config.secret_key = core_api::config::SecretKey([37; 32]))
        .await
        .0
}

/// Sends the raw signed ingest body (HMAC over the EXACT bytes, like a
/// real provider would).
async fn send_raw(
    app: &Router,
    uri: &str,
    headers: &[(&str, String)],
    body: &str,
) -> axum::response::Response {
    let mut builder = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, value.clone());
    }
    app.clone()
        .oneshot(
            builder
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// source_ref values are GLOBALLY unique (they route pre-auth), and the
/// test database is not reset between runs — suffix per run. The LAST 8
/// hex chars of a v7 uuid are the random segment (the leading ones are
/// the timestamp and repeat within a ~65s window).
fn unique_ref(prefix: &str) -> String {
    let simple = Uuid::now_v7().simple().to_string();
    format!("{prefix}-{}", &simple[simple.len() - 8..])
}

fn sig(value: &str) -> (&'static str, String) {
    (
        core_api::routes::alerts::SIGNATURE_HEADER,
        value.to_string(),
    )
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Alerts Co", slug).await
}

async fn create_board(app: &Router, token: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/boards",
        &[auth(token)],
        Some(json!({ "name": name, "is_default": false })),
    )
    .await;
    assert_eq!(response.status(), 200, "board create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("board id")
        .parse()
        .expect("uuid")
}

/// Creates an alert source through the API and returns (source_ref, raw
/// secret, source id).
struct CreatedSource {
    source_ref: String,
    secret: String,
    id: Uuid,
}

async fn create_source(
    app: &Router,
    token: &str,
    board_id: Uuid,
    source_ref: &str,
    idempotency_key: Option<&str>,
) -> CreatedSource {
    create_source_with(app, token, board_id, source_ref, None, idempotency_key).await
}

/// Creates a source with an explicit profile. `None` is the classic alert
/// source; a dot-path makes it a generic M2M receiver (the create request's
/// documented switch, `alerts.rs:774-777`).
async fn create_source_with(
    app: &Router,
    token: &str,
    board_id: Uuid,
    source_ref: &str,
    event_type: Option<&str>,
    idempotency_key: Option<&str>,
) -> CreatedSource {
    let mut payload = json!({
        "name": "Acme RMM",
        "source_ref": source_ref,
        "board_id": board_id,
    });
    if let Some(event_type) = event_type {
        payload["event_type"] = json!(event_type);
    }
    if let Some(key) = idempotency_key {
        payload["idempotency_key"] = json!(key);
    }
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(token)],
        Some(payload),
    )
    .await;
    assert_eq!(response.status(), 200, "source create must succeed");
    let body = body_json(response).await;
    CreatedSource {
        source_ref: body["source_ref"].as_str().expect("source_ref").into(),
        secret: body["secret"].as_str().expect("secret shown once").into(),
        id: body["id"].as_str().expect("id").parse().expect("uuid"),
    }
}

/// Creates a source with every clearable profile field set: a client, a
/// generic M2M profile and a payload pin. The demotion tests need a source
/// that has something to clear.
async fn create_profiled_source(
    app: &Router,
    token: &str,
    board_id: Uuid,
    source_ref: &str,
    client_id: Uuid,
) -> CreatedSource {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(token)],
        Some(json!({
            "name": "Acme RMM",
            "source_ref": source_ref,
            "board_id": board_id,
            "client_id": client_id,
            "event_type": "acme.event.received",
            "payload_schema_ref": "acme/webhook-v1",
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "profiled source create must succeed"
    );
    let body = body_json(response).await;
    CreatedSource {
        source_ref: body["source_ref"].as_str().expect("source_ref").into(),
        secret: body["secret"].as_str().expect("secret shown once").into(),
        id: body["id"].as_str().expect("id").parse().expect("uuid"),
    }
}

async fn create_client(app: &Router, token: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/clients",
        &[auth(token)],
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(response.status(), 200, "client create must succeed");
    body_json(response).await["id"]
        .as_str()
        .expect("client id")
        .parse()
        .expect("uuid")
}

/// The source's row as the list route reads it, so an assertion is about the
/// stored column and not only about the PATCH response.
async fn stored_source(app: &Router, token: &str, id: Uuid) -> Value {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/alert-sources",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|s| s["id"] == id.to_string())
        .cloned()
        .expect("the source is listed")
}

fn signed(source: &CreatedSource, body: &str) -> Vec<(&'static str, String)> {
    vec![sig(&core_mail::hmac::sign(
        source.secret.as_bytes(),
        body.as_bytes(),
    ))]
}

fn alert_body(dedup_key: &str, correlation_key: &str, title: &str, severity: &str) -> String {
    json!({
        "dedup_key": dedup_key,
        "title": title,
        "description": format!("{title} — observed by the RMM probe"),
        "severity": severity,
        "correlation_key": correlation_key,
        "attributes": { "host": "srv-01" },
    })
    .to_string()
}

#[tokio::test]
async fn ingest_auth_shell_rejects_before_any_pipeline_work() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-shell-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Shell Board").await;
    let source = create_source(&app, &token, board, &unique_ref("shell-rmm"), None).await;
    let body = alert_body("evt-shell-1", "host/shell", "Shell alert", "info");

    // Unknown source_ref → 404, not 401 (the ref is routing info).
    let unknown = send_raw(
        &app,
        "/api/v1/alerts/ingest/no-such-ref",
        &[sig("deadbeef")],
        &body,
    )
    .await;
    assert_eq!(unknown.status(), 404);

    // Missing signature → 401.
    let missing = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &[],
        &body,
    )
    .await;
    assert_eq!(missing.status(), 401);
    assert_eq!(
        body_json(missing).await["error"]["reason"],
        "unauthorized",
        "typed error envelope"
    );

    // Wrong secret → 401.
    let bad = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &[sig(&core_mail::hmac::sign(b"wrong", body.as_bytes()))],
        &body,
    )
    .await;
    assert_eq!(bad.status(), 401);

    // Tampered body with a signature over DIFFERENT bytes → 401.
    let tampered = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &signed(&source, "{\"dedup_key\":\"other\"}"),
        &body,
    )
    .await;
    assert_eq!(tampered.status(), 401);

    // Valid signature + malformed payload → 422 validation_failed.
    let broken = "{not json";
    let malformed = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &signed(&source, broken),
        broken,
    )
    .await;
    assert_eq!(malformed.status(), 422);
    assert_eq!(
        body_json(malformed).await["error"]["reason"],
        "validation_failed"
    );

    // Valid signature + missing required field → 422 (serde rejects).
    let incomplete = json!({ "title": "no dedup key", "severity": "info" }).to_string();
    let rejected = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &signed(&source, &incomplete),
        &incomplete,
    )
    .await;
    assert_eq!(rejected.status(), 422);

    // Disabled source → 403 even with a perfect signature.
    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", source.id),
        &[auth(&token)],
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(patch.status(), 200);
    let disabled = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &signed(&source, &body),
        &body,
    )
    .await;
    assert_eq!(disabled.status(), 403);
    assert_eq!(body_json(disabled).await["error"]["reason"], "forbidden");
}

#[tokio::test]
async fn source_create_is_instant_first_idempotent_and_shows_secret_once() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-src-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Source Board").await;

    // Same key + same payload → byte-identical replay (same secret, same
    // source_ref, same version).
    let source_ref = unique_ref("acme-rmm-src");
    let payload = json!({
        "name": "Acme RMM",
        "source_ref": source_ref,
        "board_id": board,
        "correlation_window_secs": 600,
        "idempotency_key": "src-key-1",
    });
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(&token)],
        Some(payload.clone()),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first_body = body_json(first).await;
    assert_eq!(first_body["mutation"]["idempotency_key"], "src-key-1");
    assert_eq!(first_body["mutation"]["version"], 1);
    assert!(
        first_body["secret"].as_str().is_some(),
        "secret shown at create"
    );

    let replay = send(
        &app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(&token)],
        Some(payload),
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replay_body = body_json(replay).await;
    assert_eq!(
        replay_body["id"], first_body["id"],
        "replay, not a second source"
    );
    assert_eq!(replay_body["secret"], first_body["secret"]);

    // Same key + different payload → typed 409.
    let conflicting = send(
        &app,
        axum::http::Method::POST,
        "/api/alert-sources",
        &[auth(&token)],
        Some(json!({
            "name": "Different name entirely",
            "source_ref": source_ref,
            "board_id": board,
            "idempotency_key": "src-key-1",
        })),
    )
    .await;
    assert_eq!(conflicting.status(), 409);
    assert_eq!(
        body_json(conflicting).await["error"]["reason"],
        "idempotency_key_reused"
    );

    // List never leaks secrets.
    let listed = send(
        &app,
        axum::http::Method::GET,
        "/api/alert-sources",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(listed.status(), 200);
    let listed_body = body_json(listed).await;
    assert_eq!(listed_body["sources"].as_array().expect("sources").len(), 1);
    assert!(
        listed_body["sources"][0].get("secret").is_none(),
        "secrets never appear in list output"
    );

    // Stale version → typed 409 version_conflict.
    let stale = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", first_body["id"].as_str().unwrap()),
        &[auth(&token)],
        Some(json!({
            "enabled": false,
            "version": first_body["mutation"]["version"].as_i64().unwrap(),
        })),
    )
    .await;
    assert_eq!(stale.status(), 200);
    let current = body_json(stale).await["mutation"]["version"]
        .as_i64()
        .unwrap();
    let stale_again = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", first_body["id"].as_str().unwrap()),
        &[auth(&token)],
        Some(json!({
            "enabled": true,
            "version": current - 1,
        })),
    )
    .await;
    assert_eq!(stale_again.status(), 409);
    assert_eq!(
        body_json(stale_again).await["error"]["reason"],
        "version_conflict"
    );
}

#[tokio::test]
async fn storm_collapses_to_one_ticket_and_replays_are_inert() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-storm-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Storm Board").await;
    let source = create_source(&app, &token, board, &unique_ref("storm-rmm"), None).await;
    let org_id: Uuid = {
        let row: (Uuid,) = sqlx::query_as("SELECT id FROM orgs WHERE slug = $1")
            .bind(&slug)
            .fetch_one(&state.pool)
            .await
            .expect("org row");
        row.0
    };
    let ingest_uri = format!("/api/v1/alerts/ingest/{}", source.source_ref);

    // 1. The anchor alert creates ONE incident ticket (critical → P1).
    let anchor_body = alert_body("evt-1", "srv-01/disk-c", "Disk usage critical", "critical");
    let anchor = send_raw(
        &app,
        &ingest_uri,
        &signed(&source, &anchor_body),
        &anchor_body,
    )
    .await;
    assert_eq!(anchor.status(), 200);
    let anchor_res = body_json(anchor).await;
    assert_eq!(anchor_res["status"], "incident_created");
    let incident_id = anchor_res["incident"]["ticket_id"]
        .as_str()
        .expect("ticket id")
        .to_string();
    let incident_number = anchor_res["incident"]["ticket_number"]
        .as_str()
        .expect("ticket number")
        .to_string();
    assert_eq!(anchor_res["incident"]["alert_count"], 1);

    let ticket = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/tickets/{incident_id}"),
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(ticket.status(), 200);
    let ticket_body = body_json(ticket).await;
    assert_eq!(ticket_body["number"], incident_number.as_str());
    assert_eq!(
        ticket_body["priority"], "p1",
        "critical severity maps to P1"
    );
    assert_eq!(ticket_body["board_id"], board.to_string());

    // 2. EXACT replay of the same signed delivery → duplicate; nothing
    //    written: still one alert, one ticket, no new events.
    let replay = send_raw(
        &app,
        &ingest_uri,
        &signed(&source, &anchor_body),
        &anchor_body,
    )
    .await;
    assert_eq!(replay.status(), 200);
    let replay_res = body_json(replay).await;
    assert_eq!(replay_res["status"], "duplicate");
    assert_eq!(replay_res["alert"]["id"], anchor_res["alert"]["id"]);
    assert_eq!(replay_res["incident"]["ticket_id"], incident_id.as_str());
    assert_eq!(
        replay_res["incident"]["alert_count"], 1,
        "replay never bumps the count"
    );

    // 3. Storm: two more related alerts (same correlation_key, fresh
    //    dedup_keys) COLLAPSE into the same ticket.
    for (i, expected_count) in [(2, 2i64), (3, 3i64)] {
        let body = alert_body(
            &format!("evt-{i}"),
            "srv-01/disk-c",
            "Disk usage critical (repeat)",
            "warning",
        );
        let res = send_raw(&app, &ingest_uri, &signed(&source, &body), &body).await;
        assert_eq!(res.status(), 200);
        let res_body = body_json(res).await;
        assert_eq!(res_body["status"], "collapsed", "storm member must fold in");
        assert_eq!(
            res_body["incident"]["ticket_id"],
            incident_id.as_str(),
            "ONE ticket for the whole storm"
        );
        assert_eq!(res_body["incident"]["alert_count"], expected_count);
    }

    // 4. An unrelated correlation key anchors a SECOND ticket.
    let other_body = alert_body(
        "evt-other",
        "srv-02/raid",
        "RAID degraded on SRV-02",
        "warning",
    );
    let other = send_raw(
        &app,
        &ingest_uri,
        &signed(&source, &other_body),
        &other_body,
    )
    .await;
    assert_eq!(other.status(), 200);
    let other_res = body_json(other).await;
    assert_eq!(other_res["status"], "incident_created");
    let second_id = other_res["incident"]["ticket_id"].as_str().unwrap();
    assert_ne!(second_id, incident_id, "different key ⇒ new incident");

    // 5. Ledger: 4 alerts (1 replay excluded), two distinct incidents.
    let alerts = send(
        &app,
        axum::http::Method::GET,
        "/api/alerts",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(alerts.status(), 200);
    let alert_rows = body_json(alerts).await["alerts"]
        .as_array()
        .expect("alerts")
        .clone();
    assert_eq!(alert_rows.len(), 4, "duplicate replay added no alert row");
    let storm_incidents: std::collections::HashSet<&str> = alert_rows
        .iter()
        .filter(|a| a["correlation_key"] == "srv-01/disk-c")
        .map(|a| a["incident_ticket_id"].as_str().expect("incident"))
        .collect();
    assert_eq!(
        storm_incidents.len(),
        1,
        "the whole storm maps to exactly ONE incident ticket"
    );

    // 6. Audit trail documents ingest + collapse (actor = the connector).
    let audit = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=alert.",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(audit.status(), 200);
    let events = body_json(audit).await["events"]
        .as_array()
        .expect("events")
        .clone();
    let actions: Vec<&str> = events
        .iter()
        .map(|e| e["action"].as_str().expect("action"))
        .collect();
    assert!(
        actions.contains(&"alert.ingested"),
        "ingest audited: {actions:?}"
    );
    assert!(
        actions.contains(&"alert.storm_collapsed"),
        "collapse audited: {actions:?}"
    );
    let connectors: std::collections::HashSet<&str> = events
        .iter()
        .filter(|e| e["action"] == "alert.ingested")
        .map(|e| e["actor"]["id"].as_str().unwrap_or(""))
        .collect();
    let expected_connector = format!("alert-source:{}", source.source_ref);
    assert!(
        connectors.contains(&expected_connector.as_str()),
        "connector actor names the source"
    );

    // 7. The wire carries `alert.ingested` (deduped: 3 events for 3
    //    accepted alerts) and `ticket.created` (2 anchored tickets).
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM outbox_events WHERE org_id = $1 AND event_type IN \
         ('alert.ingested','ticket.created')",
    )
    .bind(org_id)
    .fetch_all(&state.pool)
    .await
    .expect("outbox rows");
    let ingested = rows.iter().filter(|(t,)| t == "alert.ingested").count();
    let created = rows.iter().filter(|(t,)| t == "ticket.created").count();
    assert_eq!(
        ingested, 4,
        "one alert.ingested per ACCEPTED alert (3 storm + 1 other)"
    );
    assert_eq!(created, 2, "one ticket.created per anchored incident");
}

/// Wave 55, decision 8. The profile is a property of the source: a source
/// promoted to the generic M2M profile stops opening incident tickets here,
/// the refusal is this module's typed 409 naming the endpoint that serves the
/// profile, nothing is written, and the reason lands on the audit trail so an
/// operator can tell a dropped alert from a broken ingest.
#[tokio::test]
async fn promotion_to_the_generic_profile_refuses_ingest_and_records_it() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-profile-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Profile Board").await;
    // No profile recorded: the classic alert source, which the create request
    // documents as "Leave NULL for the classic alert path".
    let source = create_source(&app, &token, board, &unique_ref("profile-rmm"), None).await;
    let ingest_uri = format!("/api/v1/alerts/ingest/{}", source.source_ref);

    let first = alert_body("evt-1", "srv-01/disk-c", "Disk usage critical", "critical");
    let accepted = send_raw(&app, &ingest_uri, &signed(&source, &first), &first).await;
    assert_eq!(
        accepted.status(),
        200,
        "an absent profile is the alert profile and lands as it always has"
    );
    assert_eq!(body_json(accepted).await["status"], "incident_created");

    // Promote through the admin write. The profile switch stays the explicit,
    // permissioned, audited act it already was.
    let promoted = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", source.id),
        &[auth(&token)],
        Some(json!({ "event_type": "acme_event.received" })),
    )
    .await;
    assert_eq!(promoted.status(), 200);
    assert_eq!(
        body_json(promoted).await["event_type"],
        "acme_event.received"
    );

    // The next signed delivery is refused, and the answer points the operator
    // at the door that serves the profile the source now records.
    let second = alert_body("evt-2", "srv-01/disk-c", "Disk usage critical", "warning");
    let refused = send_raw(&app, &ingest_uri, &signed(&source, &second), &second).await;
    assert_eq!(
        refused.status(),
        409,
        "a generic M2M profile cannot open incident tickets"
    );
    let refusal = body_json(refused).await;
    assert_eq!(
        refusal["error"]["reason"], "conflict",
        "the module's own typed 409, not a generic 400"
    );
    let message = refusal["error"]["message"].as_str().expect("message");
    assert!(
        message.contains(&format!("/api/webhooks/ingest/{}", source.source_ref)),
        "the refusal names the endpoint that serves the profile: {message}"
    );
    assert!(
        message.contains("acme_event.received"),
        "the refusal names the recorded profile: {message}"
    );

    // Nothing was written by the refused delivery.
    let alerts = send(
        &app,
        axum::http::Method::GET,
        "/api/alerts",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(alerts.status(), 200);
    let rows = body_json(alerts).await["alerts"]
        .as_array()
        .expect("alerts")
        .clone();
    assert_eq!(rows.len(), 1, "the refused delivery added no alert row");
    assert_eq!(rows[0]["dedup_key"], "evt-1");

    // The refusal is observable: the trail names the source, the profile and
    // the endpoint it should have used.
    let audit = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=alert.refused",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(audit.status(), 200);
    let events = body_json(audit).await["events"]
        .as_array()
        .expect("events")
        .clone();
    let recorded = events
        .iter()
        .find(|e| e["entity_id"] == source.id.to_string())
        .expect("the refusal is on the audit trail");
    assert_eq!(recorded["action"], "alert.refused");
    assert_eq!(recorded["detail"]["reason"], "source_profile");
    assert_eq!(recorded["detail"]["event_type"], "acme_event.received");
    assert_eq!(recorded["detail"]["served_by"], "/api/webhooks/ingest");
    assert_eq!(
        recorded["actor"]["id"],
        format!("alert-source:{}", source.source_ref)
    );
}

/// The refusal is about the source, not about the payload, and it does not
/// brick the integration: the same source keeps emitting through its own door.
#[tokio::test]
async fn a_generic_source_is_refused_at_the_alert_door_and_still_serves_its_own() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-generic-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Generic Board").await;
    let source = create_source_with(
        &app,
        &token,
        board,
        &unique_ref("generic-rmm"),
        Some("acme_event.received"),
        None,
    )
    .await;

    // A real alert document, then a body that would fail validation as one:
    // both get the same typed refusal, because the profile is decided from the
    // source before the payload is read.
    for body in [
        alert_body("evt-generic-1", "host/generic", "Disk critical", "critical"),
        "{}".to_string(),
    ] {
        let refused = send_raw(
            &app,
            &format!("/api/v1/alerts/ingest/{}", source.source_ref),
            &signed(&source, &body),
            &body,
        )
        .await;
        assert_eq!(
            refused.status(),
            409,
            "the profile decides before the payload: {body}"
        );
        assert_eq!(body_json(refused).await["error"]["reason"], "conflict");
    }

    // The integration is not broken — its own door still emits the envelope.
    let envelope =
        json!({ "dedup_key": "evt-generic-1", "payload": { "status": "ok" } }).to_string();
    let emitted = send_raw(
        &app,
        &format!("/api/v1/webhooks/ingest/{}", source.source_ref),
        &signed(&source, &envelope),
        &envelope,
    )
    .await;
    assert_eq!(
        emitted.status(),
        200,
        "the generic profile still serves its own door"
    );
    assert_eq!(body_json(emitted).await["status"], "emitted");

    // No incident tickets came out of the alert door.
    let alerts = send(
        &app,
        axum::http::Method::GET,
        "/api/alerts",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(alerts.status(), 200);
    assert!(
        body_json(alerts).await["alerts"]
            .as_array()
            .expect("alerts")
            .is_empty(),
        "a generic receiver opens no alerts"
    );

    // Both refusals are recorded, not just the first.
    let audit = send(
        &app,
        axum::http::Method::GET,
        "/api/audit-events?action=alert.refused&limit=50",
        &[auth(&token)],
        None,
    )
    .await;
    assert_eq!(audit.status(), 200);
    let refusals = body_json(audit).await["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter(|e| e["entity_id"] == source.id.to_string())
        .count();
    assert_eq!(refusals, 2, "every refused delivery leaves a row");
}

#[tokio::test]
async fn dual_mount_parity_for_ingest_and_alias_path() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-mounts-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Mount Board").await;
    let source = create_source(&app, &token, board, &unique_ref("mount-rmm"), None).await;

    // The unversioned legacy alias hits the same handler: an unsigned
    // request is 401 (auth shell ran), NOT 404 (route missing).
    for path in [
        format!("/api/v1/alerts/ingest/{}", source.source_ref),
        format!("/api/alerts/ingest/{}", source.source_ref),
    ] {
        let res = send_raw(&app, &path, &[], "{}").await;
        assert_eq!(res.status(), 401, "{path} must be mounted and auth-gated");
    }
}

/// F48. A source could be promoted to the generic M2M profile but never
/// demoted: the web UI sends an explicit `null`, serde read it as "absent",
/// and the SQL COALESCE bound the same NULL for both shapes. This pins the
/// demotion end to end — record cleared, and the alert door open again.
#[tokio::test]
async fn patch_with_explicit_null_demotes_the_source_and_clears_the_pin() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-demote-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Demote Board").await;
    let client = create_client(&app, &token, "Demote Client").await;
    let source =
        create_profiled_source(&app, &token, board, &unique_ref("demote-rmm"), client).await;
    assert_eq!(
        stored_source(&app, &token, source.id).await["event_type"],
        "acme.event.received",
        "the create records the generic profile"
    );

    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", source.id),
        &[auth(&token)],
        Some(json!({
            "event_type": null,
            "payload_schema_ref": null,
            "client_id": null,
        })),
    )
    .await;
    assert_eq!(
        patch.status(),
        200,
        "an explicit null is a clear, not a no-op"
    );
    let body = body_json(patch).await;
    assert!(body["event_type"].is_null(), "the response shows the clear");
    assert!(body["payload_schema_ref"].is_null());
    assert!(body["client_id"].is_null());

    let stored = stored_source(&app, &token, source.id).await;
    assert!(
        stored["event_type"].is_null(),
        "the column is cleared, not just the response: {stored}"
    );
    assert!(stored["payload_schema_ref"].is_null());
    assert!(stored["client_id"].is_null());

    // The demotion is what reopens the alert door: the same source that
    // refused ingest with a generic profile lands an incident again.
    let restored = alert_body("evt-demoted", "srv-demoted/disk-c", "Disk full", "warning");
    let ingest = send_raw(
        &app,
        &format!("/api/v1/alerts/ingest/{}", source.source_ref),
        &signed(&source, &restored),
        &restored,
    )
    .await;
    assert_eq!(
        ingest.status(),
        200,
        "a demoted source is the classic alert source again"
    );
    assert_eq!(body_json(ingest).await["status"], "incident_created");
}

/// The other half of F48: a PATCH that does not name a clearable field must
/// leave it exactly as it was. An empty `Option` and an explicit null are
/// different requests, and only one of them clears.
#[tokio::test]
async fn patch_that_omits_the_profile_fields_leaves_them_untouched() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("alerts-omit-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let board = create_board(&app, &token, "Omit Board").await;
    let client = create_client(&app, &token, "Omit Client").await;
    let source = create_profiled_source(&app, &token, board, &unique_ref("omit-rmm"), client).await;

    let patch = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/alert-sources/{}", source.id),
        &[auth(&token)],
        Some(json!({ "name": "Renamed RMM" })),
    )
    .await;
    assert_eq!(patch.status(), 200);
    let body = body_json(patch).await;
    assert_eq!(body["name"], "Renamed RMM");

    let stored = stored_source(&app, &token, source.id).await;
    assert_eq!(stored["name"], "Renamed RMM", "the named field changed");
    assert_eq!(
        stored["event_type"], "acme.event.received",
        "an omitted profile field is not a clear: {stored}"
    );
    assert_eq!(stored["payload_schema_ref"], "acme/webhook-v1");
    assert_eq!(
        stored["client_id"],
        client.to_string(),
        "an omitted client is not a clear"
    );
}
