//! Calendar zones and cross-project collisions over the REAL core-api router.
//!
//! Two things a coordinator relies on before moving a bar, both stated by the
//! plan surface and both previously unnamed in the contract:
//!
//! 1. Zones: a project can state an agreed change window and a client closure,
//!    read them back, and delete one. The kind and the range are validated.
//! 2. Collisions: a move that lands on work the same person is already doing in
//!    ANOTHER project comes back in `collisions`, with the project's name and a
//!    working-minute end — not a silent double booking.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{auth, body_json, register_org_token, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::json;
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([37; 32])).await
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![auth(token)]
}

async fn me_id(app: &Router, token: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &bearer(token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "auth/me must resolve");
    let me = body_json(response).await;
    me["user_id"].as_str().unwrap().parse().unwrap()
}

async fn create_project(app: &Router, token: &str, name: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer(token),
        Some(json!({ "name": name, "billing_method": "time_and_materials", "currency": "USD" })),
    )
    .await;
    assert_eq!(response.status(), 200, "project create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn create_task(
    app: &Router,
    token: &str,
    project_id: Uuid,
    title: &str,
    assignee: Uuid,
    start_at: &str,
) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer(token),
        Some(json!({
            "title": title,
            "assignee_id": assignee,
            "start_at": start_at,
            "duration_minutes": 480
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "task create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn calendar_zones_are_stated_listed_and_removed() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org_token(
        &app,
        "Zones Co",
        &format!("zones-{}", Uuid::now_v7().simple()),
    )
    .await;
    let project_id = create_project(&app, &token, "Cutover plan").await;

    // --- a closure and a window, both read back in date order -------------
    for (kind, label, starts_on, ends_on) in [
        (
            "window",
            "Cutover change window",
            "2026-11-27",
            "2026-11-29",
        ),
        ("closed", "Client closure", "2026-11-30", "2026-12-02"),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/projects/{project_id}/zones"),
            &bearer(&token),
            Some(json!({
                "kind": kind,
                "label": label,
                "starts_on": starts_on,
                "ends_on": ends_on,
                "note": "agreed on site"
            })),
        )
        .await;
        assert_eq!(response.status(), 200, "zone create must succeed");
        let zone = body_json(response).await;
        assert_eq!(zone["kind"], kind);
        assert_eq!(zone["label"], label);
    }

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/zones"),
        &bearer(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let listed = body_json(response).await;
    let zones = listed["zones"].as_array().expect("zones array");
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0]["kind"], "window", "soonest first");
    assert_eq!(zones[1]["kind"], "closed");

    // --- an unknown kind and a reversed range are refused, not stored -----
    for body in [
        json!({ "kind": "blackout", "label": "x", "starts_on": "2026-11-27", "ends_on": "2026-11-28" }),
        json!({ "kind": "closed", "label": "x", "starts_on": "2026-11-28", "ends_on": "2026-11-27" }),
        json!({ "kind": "closed", "label": "   ", "starts_on": "2026-11-27", "ends_on": "2026-11-28" }),
    ] {
        let response = send(
            &app,
            axum::http::Method::POST,
            &format!("/api/v1/projects/{project_id}/zones"),
            &bearer(&token),
            Some(body),
        )
        .await;
        assert_eq!(response.status(), 422, "invalid zone must be refused");
    }

    // --- delete removes exactly one ---------------------------------------
    let zone_id = zones[0]["id"].as_str().unwrap().to_string();
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/projects/{project_id}/zones/{zone_id}"),
        &bearer(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "delete must succeed");
    assert_eq!(body_json(response).await["deleted"], true);

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/zones"),
        &bearer(&token),
        None,
    )
    .await;
    let left = body_json(response).await;
    let left = left["zones"].as_array().expect("zones array");
    assert_eq!(left.len(), 1, "only the deleted zone is gone");
    assert_eq!(left[0]["kind"], "closed");
}

#[tokio::test]
async fn a_move_that_double_books_a_person_names_the_other_project() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org_token(
        &app,
        "Collisions Co",
        &format!("collisions-{}", Uuid::now_v7().simple()),
    )
    .await;
    let person = me_id(&app, &token).await;

    let planned = create_project(&app, &token, "Acme refresh").await;
    let other = create_project(&app, &token, "Harbour cutover").await;

    // The move target is 4 May; the other project already owns that morning.
    let moved = create_task(
        &app,
        &token,
        planned,
        "Survey",
        person,
        "2026-05-01T07:00:00Z",
    )
    .await;
    create_task(
        &app,
        &token,
        other,
        "Firewall cutover",
        person,
        "2026-05-04T07:00:00Z",
    )
    .await;

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{planned}/schedule/preview"),
        &bearer(&token),
        Some(json!({ "moves": [{ "task_id": moved, "start_at": "2026-05-04T07:00:00Z" }] })),
    )
    .await;
    assert_eq!(response.status(), 200, "preview must succeed");
    let preview = body_json(response).await;
    assert_eq!(preview["changes"].as_array().unwrap().len(), 1);
    let collisions = preview["collisions"].as_array().expect("collisions array");
    assert_eq!(collisions.len(), 1, "the double booking is reported");
    let collision = &collisions[0];
    assert_eq!(collision["project_id"], other.to_string());
    assert_eq!(collision["project_name"], "Harbour cutover");
    assert_eq!(collision["task_title"], "Firewall cutover");
    assert_eq!(
        collision["end_at"], "2026-05-04T15:00:00Z",
        "480 working minutes from 07:00 ends at 15:00"
    );

    // --- a move with no other project's work around it reports nothing ----
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{planned}/schedule/preview"),
        &bearer(&token),
        Some(json!({ "moves": [{ "task_id": moved, "start_at": "2026-05-11T07:00:00Z" }] })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let clear = body_json(response).await;
    assert_eq!(
        clear["collisions"]
            .as_array()
            .expect("collisions array")
            .len(),
        0,
        "an empty list is the honest answer when nothing overlaps"
    );
}
