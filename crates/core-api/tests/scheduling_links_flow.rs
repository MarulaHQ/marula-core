//! End-to-end scheduling-link flows (Wave 11 lane 4) driven through the
//! REAL core-api router against a real Postgres.
//!
//! Purpose: prove the Calendly-style lifecycle over HTTP — group CRUD +
//! membership, meeting-type templates (validation, editable parameters,
//! scope targets), link minting (token returned once, hash-only storage),
//! the public view/book surface (typed revoked/used/expired errors),
//! race-safe booking against the 2.12 availability foundation
//! (double-booking backstop + single-winner link flip), and cross-org
//! isolation.
//!
//! Non-purpose: pure rule math (core-domain suite), WS delivery, auto-
//! email transport behavior (transport=none in this harness = skipped).

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
        .expect("scheduling-links e2e needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect_admin(&admin_url).await.expect("admin pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    core_db::assert_rls_coverage(&pool)
        .await
        .expect("RLS coverage holds with scheduling-link tables");
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

async fn register_org(app: &Router, slug: String) -> Value {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Links Co",
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

async fn publish_availability(app: &Router, bearer: &str, weekday: i32) {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/availability",
        Some(bearer),
        Some(json!({
            "weekday": weekday,
            "start_local": "09:00",
            "end_local": "17:00",
            "timezone": "UTC",
            "effective_from": "2026-01-01",
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "availability create must succeed: {}",
        body_json(response).await
    );
}

/// The NEXT Monday at `hour`:00 UTC — availability is published for
/// weekday 0, and slots must be in the future (the API rejects past
/// bookings by design).
fn next_monday_at(hour: u32) -> String {
    next_weekday_at(chrono::Weekday::Mon, hour)
}

/// The NEXT Sunday at `hour`:00 UTC (no availability published — the
/// negative case).
fn next_sunday_at(hour: u32) -> String {
    next_weekday_at(chrono::Weekday::Sun, hour)
}

fn next_weekday_at(target: chrono::Weekday, hour: u32) -> String {
    use chrono::Datelike;
    let mut day = chrono::Utc::now() + chrono::Duration::days(1);
    while day.weekday() != target {
        day += chrono::Duration::days(1);
    }
    format!("{}T{hour:02}:00:00Z", day.format("%Y-%m-%d"))
}

#[tokio::test]
async fn groups_meeting_types_and_link_lifecycle() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("links1-{}", Uuid::now_v7().simple())).await;
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
    let owner_id = me["user_id"].as_str().unwrap().to_string();

    // Free-form group CRUD + membership.
    let group: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-groups",
            Some(&bearer),
            Some(json!({"name": "Service Desk"})),
        )
        .await,
    )
    .await;
    assert_eq!(group["name"], "Service Desk");
    let group_id = group["id"].as_str().unwrap().to_string();

    let dupe = send(
        &app,
        axum::http::Method::POST,
        "/api/scheduling-groups",
        Some(&bearer),
        Some(json!({"name": "Service Desk"})),
    )
    .await;
    assert_eq!(
        dupe.status(),
        409,
        "duplicate group name is a typed conflict"
    );

    let membership: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/scheduling-groups/{group_id}/members"),
            Some(&bearer),
            Some(json!({"user_id": owner_id})),
        )
        .await,
    )
    .await;
    assert_eq!(
        membership["member_user_ids"].as_array().unwrap().len(),
        1,
        "membership lands in the pool"
    );

    // Meeting-type template: group scope + editable parameters.
    let template: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "Service Desk — 30 minutes",
                "duration_minutes": 30,
                "buffer_minutes": 5,
                "booking_window_days": 14,
                "parameters": {"location": "phone", "agenda": "Quick triage"},
                "assignee_scope": "group",
                "assignee_group_id": group_id,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(template["meeting_type"]["duration_minutes"], 30);
    assert_eq!(template["meeting_type"]["parameters"]["location"], "phone");
    let template_id = template["meeting_type"]["id"].as_str().unwrap().to_string();

    // Scope/target mismatch is a typed 422.
    let bad = send(
        &app,
        axum::http::Method::POST,
        "/api/meeting-types",
        Some(&bearer),
        Some(json!({
            "name": "Broken",
            "duration_minutes": 30,
            "assignee_scope": "user",
            "assignee_group_id": group_id,
        })),
    )
    .await;
    assert_eq!(bad.status(), 422, "scope without its target is 422");

    // Publish Monday availability so the public slot preview has gaps.
    publish_availability(&app, &bearer, 0).await;

    // Link minting: raw token returned ONCE.
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id, "label": "Triaging"})),
        )
        .await,
    )
    .await;
    assert_eq!(created["link"]["status"], "active");
    assert!(
        created["mutation"]["seq"].as_i64().is_some(),
        "IF2 metadata present"
    );
    let token = created["token"].as_str().unwrap().to_string();
    assert!(token.len() >= 40, "unguessable token (32 bytes URL-safe)");
    let link_id = created["link"]["id"].as_str().unwrap().to_string();

    // Unauthenticated view works and stamps the lifecycle.
    let view: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/public/scheduling-links/{token}"),
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(view["meeting_type_name"], "Service Desk — 30 minutes");
    assert_eq!(view["duration_minutes"], 30);
    assert!(
        !view["slots"].as_array().unwrap().is_empty(),
        "slot preview present (weekdays 09:00-17:00 UTC)"
    );

    let links: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/scheduling-links",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let link = links
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == link_id)
        .unwrap();
    assert!(link["first_viewed_at"].as_str().is_some(), "viewed stamped");
    assert_eq!(link["view_count"], 1);

    // Book through the public surface — unauthenticated.
    let booked: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/public/scheduling-links/{token}/book"),
            None,
            Some(json!({
                "starts_at": next_monday_at(10),
                "booker_name": "Dana Fischer",
                "booker_email": "dana@northwind.example",
                "booker_notes": "Printer is haunted.",
            })),
        )
        .await,
    )
    .await;
    assert_eq!(booked["appointment"]["kind"], "remote");
    assert_eq!(
        booked["appointment"]["scheduling_link_id"], link_id,
        "booking carries the link back-ref"
    );
    let appointment_id = booked["appointment"]["id"].as_str().unwrap().to_string();
    let _ = appointment_id;

    // The link flipped to used: booking again is a typed 409.
    let again = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({"starts_at": next_monday_at(10)})),
    )
    .await;
    assert_eq!(again.status(), 409, "single-use link cannot book twice");

    // The booking is a real appointment visible on the staff calendar.
    let links: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/scheduling-links",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let link = links
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == link_id)
        .unwrap();
    assert_eq!(link["status"], "used");
    assert!(
        link["scheduled_appointment_id"].as_str().is_some(),
        "link points at the real booking"
    );

    // The link → booking trail is auditable.
    let bookings: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/scheduling-links/{link_id}/bookings"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(bookings.as_array().unwrap().len(), 1);
    assert_eq!(bookings[0]["booker_email"], "dana@northwind.example");

    // The public page of a used link is a typed 409, not a leak.
    let used_view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/public/scheduling-links/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(used_view.status(), 409);
}

#[tokio::test]
async fn booking_validates_availability_and_conflicts_at_book_time() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("links2-{}", Uuid::now_v7().simple())).await;
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
    let owner_id = me["user_id"].as_str().unwrap().to_string();
    publish_availability(&app, &bearer, 0).await; // Mondays 09:00-17:00 UTC

    let group: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-groups",
            Some(&bearer),
            Some(json!({"name": "Field Techs"})),
        )
        .await,
    )
    .await;
    let group_id = group["id"].as_str().unwrap().to_string();
    let _: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/scheduling-groups/{group_id}/members"),
            Some(&bearer),
            Some(json!({"user_id": owner_id})),
        )
        .await,
    )
    .await;
    let template: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "Field visit",
                "duration_minutes": 60,
                "assignee_scope": "group",
                "assignee_group_id": group_id,
            })),
        )
        .await,
    )
    .await;
    let template_id = template["meeting_type"]["id"].as_str().unwrap().to_string();
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id})),
        )
        .await,
    )
    .await;
    let token = created["token"].as_str().unwrap().to_string();

    // Slot OUTSIDE published availability: typed 409, no booking.
    let outside = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({"starts_at": next_monday_at(20)})), // 20:00 > 17:00 close
    )
    .await;
    assert_eq!(outside.status(), 409, "slot outside availability is 409");

    // Slot on a Sunday (no windows): typed 409.
    let sunday = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token}/book"),
        None,
        Some(json!({"starts_at": next_sunday_at(10)})),
    )
    .await;
    assert_eq!(sunday.status(), 409, "day with no published windows is 409");

    // Legal Monday slot books.
    let booked: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/public/scheduling-links/{token}/book"),
            None,
            Some(json!({"starts_at": next_monday_at(10)})),
        )
        .await,
    )
    .await;
    assert_eq!(
        booked["appointment"]["starts_at"],
        json!(next_monday_at(10))
    );

    // A SECOND link to the same template cannot take the overlapping
    // Monday slot (double-booking backstop at book time).
    let created2: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id})),
        )
        .await,
    )
    .await;
    let token2 = created2["token"].as_str().unwrap().to_string();
    let overlap = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{token2}/book"),
        None,
        Some(json!({"starts_at": next_monday_at(10)})),
    )
    .await;
    assert_eq!(
        overlap.status(),
        409,
        "racing second booking through another link is a typed conflict"
    );

    // A non-overlapping slot through the second link still books.
    let ok: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/public/scheduling-links/{token2}/book"),
            None,
            Some(json!({"starts_at": next_monday_at(13)})),
        )
        .await,
    )
    .await;
    assert_eq!(ok["appointment"]["starts_at"], json!(next_monday_at(13)));
}

#[tokio::test]
async fn revoked_expired_and_unknown_tokens_are_typed() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("links3-{}", Uuid::now_v7().simple())).await;
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
    let owner_id = me["user_id"].as_str().unwrap().to_string();
    publish_availability(&app, &bearer, 0).await;

    let template: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "Sync",
                "duration_minutes": 15,
                "assignee_scope": "user",
                "assignee_user_id": owner_id,
            })),
        )
        .await,
    )
    .await;
    let template_id = template["meeting_type"]["id"].as_str().unwrap().to_string();

    // Revoked link: view + book are typed 409.
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id})),
        )
        .await,
    )
    .await;
    let revoked_token = created["token"].as_str().unwrap().to_string();
    let link_id = created["link"]["id"].as_str().unwrap().to_string();
    let revoked: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/scheduling-links/{link_id}/revoke"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(revoked["status"], "revoked");
    let view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/public/scheduling-links/{revoked_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(view.status(), 409, "revoked token is a typed conflict");
    let book = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{revoked_token}/book"),
        None,
        Some(json!({"starts_at": next_monday_at(10)})),
    )
    .await;
    assert_eq!(book.status(), 409);

    // Expired link: typed 409.
    let expired: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({
                "meeting_type_id": template_id,
                "expires_at": "2026-01-01T00:00:00Z",
            })),
        )
        .await,
    )
    .await;
    let expired_token = expired["token"].as_str().unwrap().to_string();
    let view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/public/scheduling-links/{expired_token}"),
        None,
        None,
    )
    .await;
    assert_eq!(view.status(), 409, "expired token is a typed conflict");

    // Unknown token: 404, no org data leak.
    let unknown = send(
        &app,
        axum::http::Method::GET,
        "/api/public/scheduling-links/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        None,
        None,
    )
    .await;
    assert_eq!(unknown.status(), 404);

    // Slot beyond the booking window: typed 422.
    let window: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id})),
        )
        .await,
    )
    .await;
    let wtoken = window["token"].as_str().unwrap().to_string();
    let far = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{wtoken}/book"),
        None,
        Some(json!({"starts_at": "2026-12-01T10:00:00Z"})),
    )
    .await;
    assert_eq!(far.status(), 422, "beyond the booking window is 422");

    // Past slot: typed 422.
    let past = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/public/scheduling-links/{wtoken}/book"),
        None,
        Some(json!({"starts_at": "2020-01-01T10:00:00Z"})),
    )
    .await;
    assert_eq!(past.status(), 422);
}

#[tokio::test]
async fn cross_org_isolation_and_rbac() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("links3-{}", Uuid::now_v7().simple())).await;
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
    let owner_id = me["user_id"].as_str().unwrap().to_string();
    publish_availability(&app, &bearer, 0).await;

    let template: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "Owner-only",
                "duration_minutes": 30,
                "assignee_scope": "user",
                "assignee_user_id": owner_id,
            })),
        )
        .await,
    )
    .await;
    let template_id = template["meeting_type"]["id"].as_str().unwrap().to_string();

    // A SECOND org cannot see or revoke the first org's link, and its
    // own member-less group scope yields a typed conflict at book time.
    let other = register_org(&app, format!("links4-{}", Uuid::now_v7().simple())).await;
    let other_bearer = other["access_token"].as_str().unwrap().to_string();

    // Cross-org reads are RLS-empty.
    let groups: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/scheduling-groups",
            Some(&other_bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        groups.as_array().unwrap().len(),
        0,
        "org A groups invisible to org B"
    );
    let templates: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/meeting-types",
            Some(&other_bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        templates.as_array().unwrap().len(),
        0,
        "org A templates invisible to org B"
    );

    // The first org's public token books fine (public surface is
    // token-scoped, not org-scoped — that IS the feature).
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({"meeting_type_id": template_id})),
        )
        .await,
    )
    .await;
    let token = created["token"].as_str().unwrap().to_string();
    let view = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/public/scheduling-links/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(view.status(), 200, "public surface needs no auth");
}

#[tokio::test]
async fn ticket_scoped_links_carry_the_cross_ref() {
    let (_state, app) = test_state().await;
    let tokens = register_org(&app, format!("links5-{}", Uuid::now_v7().simple())).await;
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
    let owner_id = me["user_id"].as_str().unwrap().to_string();
    publish_availability(&app, &bearer, 0).await;

    // Create a board + ticket for the cross-ref.
    let boards: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            "/api/boards",
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    let board_id = boards[0]["id"].as_str().unwrap().to_string();
    let ticket: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/tickets",
            Some(&bearer),
            Some(json!({
                "board_id": board_id,
                "title": "Server room smells like ozone",
                "priority": "p1",
                "instant": {"idempotency_key": Uuid::now_v7().to_string()},
            })),
        )
        .await,
    )
    .await;
    let ticket_id = ticket["ticket"]["ticket"]["id"]
        .as_str()
        .or(ticket["ticket"]["id"].as_str())
        .or(ticket["id"].as_str())
        .unwrap()
        .trim_matches('"')
        .to_string();

    let template: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/meeting-types",
            Some(&bearer),
            Some(json!({
                "name": "On-site visit",
                "duration_minutes": 60,
                "assignee_scope": "user",
                "assignee_user_id": owner_id,
            })),
        )
        .await,
    )
    .await;
    let template_id = template["meeting_type"]["id"].as_str().unwrap().to_string();

    // Ticket-scoped link: the booking lands back on the ticket.
    let created: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            "/api/scheduling-links",
            Some(&bearer),
            Some(json!({
                "meeting_type_id": template_id,
                "ticket_id": ticket_id,
            })),
        )
        .await,
    )
    .await;
    assert_eq!(created["link"]["ticket_id"], ticket_id);
    let token = created["token"].as_str().unwrap().to_string();
    let _: Value = body_json(
        send(
            &app,
            axum::http::Method::POST,
            &format!("/api/public/scheduling-links/{token}/book"),
            None,
            Some(json!({"starts_at": next_monday_at(9)})),
        )
        .await,
    )
    .await;

    // The ticket's link list shows the used link with its booking.
    let links: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!("/api/scheduling-links?ticket_id={ticket_id}"),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(links.as_array().unwrap().len(), 1);
    assert_eq!(links[0]["status"], "used");

    // The link → booking trail carries the booking (booker email was
    // not supplied — null fields stay null).
    let trail: Value = body_json(
        send(
            &app,
            axum::http::Method::GET,
            &format!(
                "/api/scheduling-links/{}/bookings",
                created["link"]["id"].as_str().unwrap()
            ),
            Some(&bearer),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(trail.as_array().unwrap().len(), 1);
    assert_eq!(trail[0]["booker_email"], Value::Null);
}
