//! The plan's order, over the REAL core-api router.
//!
//! A coordinator reorders phases and tasks by dragging rows, and a task dropped
//! on another phase changes phase. That is one write, not one write per row: a
//! reorder that half-landed would leave the plan in an order nobody chose. These
//! cases state the contract the plan surface relies on:
//!
//! 1. The order is written for phases and for tasks, including a task that
//!    crosses into another phase, and it reads back in the order it was sent.
//! 2. A partial order is refused — a task left out would keep a `sort_order`
//!    that now collides with the ones being written.
//! 3. A duplicated id is a shape error, and an id from another org is out of
//!    scope: the same 404 an invisible project answers.
//!
//! Database selection follows calendar_and_collisions_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{auth, body_json, register_org_token, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([41; 32])).await
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![auth(token)]
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

async fn create_phase(app: &Router, token: &str, project_id: Uuid, name: &str, order: i32) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/phases"),
        &bearer(token),
        Some(json!({ "name": name, "sort_order": order })),
    )
    .await;
    assert_eq!(response.status(), 200, "phase create must succeed");
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
    phase_id: Option<Uuid>,
    title: &str,
    order: i32,
) -> Uuid {
    let response = send(
        app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer(token),
        Some(json!({ "title": title, "phase_id": phase_id, "sort_order": order })),
    )
    .await;
    assert_eq!(response.status(), 200, "task create must succeed");
    body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn phase_names(app: &Router, token: &str, project_id: Uuid) -> Vec<String> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/phases"),
        &bearer(token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response)
        .await
        .as_array()
        .expect("phase list")
        .iter()
        .map(|phase| phase["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn task_titles(app: &Router, token: &str, project_id: Uuid) -> Vec<(String, Option<Uuid>)> {
    let response = send(
        app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/tasks?limit=200"),
        &bearer(token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response)
        .await
        .as_array()
        .expect("task list")
        .iter()
        .map(|task| {
            (
                task["title"].as_str().unwrap_or_default().to_string(),
                task["phase_id"].as_str().and_then(|id| id.parse().ok()),
            )
        })
        .collect()
}

fn titles(tasks: &[(String, Option<Uuid>)]) -> Vec<String> {
    tasks.iter().map(|(title, _)| title.clone()).collect()
}

async fn reorder(app: &Router, token: &str, project_id: Uuid, body: Value) -> (u16, Value) {
    let response = send(
        app,
        axum::http::Method::PUT,
        &format!("/api/v1/projects/{project_id}/order"),
        &bearer(token),
        Some(body),
    )
    .await;
    let status = response.status().as_u16();
    let body = body_json(response).await;
    (status, body)
}

#[tokio::test]
async fn the_order_is_written_for_phases_and_tasks() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org_token(
        &app,
        "Order Co",
        &format!("order-{}", Uuid::now_v7().simple()),
    )
    .await;
    let project = create_project(&app, &token, "Rollout").await;

    let discovery = create_phase(&app, &token, project, "Discovery", 0).await;
    let build = create_phase(&app, &token, project, "Build", 1).await;
    let handover = create_phase(&app, &token, project, "Handover", 2).await;
    let survey = create_task(&app, &token, project, Some(discovery), "Survey", 0).await;
    let install = create_task(&app, &token, project, Some(build), "Install", 1).await;
    let cleanup = create_task(&app, &token, project, Some(build), "Clean up", 2).await;

    // Reversed phases; Install moves to Handover; Clean up leads Build.
    let (status, body) = reorder(
        &app,
        &token,
        project,
        json!({
            "phases": [handover, build, discovery],
            "tasks": [
                { "phase_id": handover, "task_ids": [install] },
                { "phase_id": build, "task_ids": [cleanup] },
                { "phase_id": discovery, "task_ids": [survey] }
            ]
        }),
    )
    .await;
    assert_eq!(status, 200, "the order must be written: {body}");
    assert_eq!(body["phases"], 3);
    assert_eq!(body["tasks"], 3);
    assert_eq!(body["rephased"], 1, "only Install changed phase");

    assert_eq!(
        phase_names(&app, &token, project).await,
        vec!["Handover", "Build", "Discovery"]
    );
    // The list is ordered by sort_order, so each phase's work reads in the
    // order it was sent, and Install now belongs to Handover.
    let tasks = task_titles(&app, &token, project).await;
    let install_phase = tasks
        .iter()
        .find(|(title, _)| title == "Install")
        .and_then(|(_, phase)| *phase);
    assert_eq!(install_phase, Some(handover), "Install moved into Handover");
    assert_eq!(titles(&tasks), vec!["Survey", "Clean up", "Install"]);

    // A second call that sends the same order changes nothing and reports none.
    let (status, body) = reorder(
        &app,
        &token,
        project,
        json!({
            "phases": [handover, build, discovery],
            "tasks": [
                { "phase_id": handover, "task_ids": [install] },
                { "phase_id": build, "task_ids": [cleanup] },
                { "phase_id": discovery, "task_ids": [survey] }
            ]
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["rephased"], 0, "the same order re-parents nothing");
}

#[tokio::test]
async fn a_partial_or_duplicated_order_is_refused() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let token = register_org_token(
        &app,
        "Partial Co",
        &format!("partial-{}", Uuid::now_v7().simple()),
    )
    .await;
    let project = create_project(&app, &token, "Partial").await;
    let first = create_phase(&app, &token, project, "First", 0).await;
    let second = create_phase(&app, &token, project, "Second", 1).await;
    let one = create_task(&app, &token, project, Some(first), "One", 0).await;
    let two = create_task(&app, &token, project, Some(first), "Two", 1).await;

    // One task left out: the request is a page of the plan, not the plan.
    let (status, _) = reorder(
        &app,
        &token,
        project,
        json!({
            "phases": [first, second],
            "tasks": [{ "phase_id": first, "task_ids": [one] }]
        }),
    )
    .await;
    assert_eq!(status, 422, "an order that drops a task is refused");

    // A phase left out is refused the same way.
    let (status, _) = reorder(
        &app,
        &token,
        project,
        json!({
            "phases": [first],
            "tasks": [{ "phase_id": first, "task_ids": [one, two] }]
        }),
    )
    .await;
    assert_eq!(status, 422, "an order that drops a phase is refused");

    // The same task named twice is a client bug, not a user action.
    let (status, _) = reorder(
        &app,
        &token,
        project,
        json!({
            "phases": [second, first],
            "tasks": [
                { "phase_id": first, "task_ids": [one, two] },
                { "phase_id": second, "task_ids": [one] }
            ]
        }),
    )
    .await;
    assert_eq!(status, 422, "a task named twice is a shape error");

    // Nothing above landed.
    assert_eq!(
        phase_names(&app, &token, project).await,
        vec!["First", "Second"]
    );
    assert_eq!(titles(&task_titles(&app, &token, project).await), vec!["One", "Two"]);
}

#[tokio::test]
async fn another_orgs_task_is_out_of_scope() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let mine = register_org_token(
        &app,
        "Scope A",
        &format!("scope-a-{}", Uuid::now_v7().simple()),
    )
    .await;
    let theirs = register_org_token(
        &app,
        "Scope B",
        &format!("scope-b-{}", Uuid::now_v7().simple()),
    )
    .await;

    let my_project = create_project(&app, &mine, "Mine").await;
    let my_phase = create_phase(&app, &mine, my_project, "Only", 0).await;
    let my_task = create_task(&app, &mine, my_project, Some(my_phase), "Mine", 0).await;

    let their_project = create_project(&app, &theirs, "Theirs").await;
    let their_phase = create_phase(&app, &theirs, their_project, "Only", 0).await;
    let their_task = create_task(&app, &theirs, their_project, Some(their_phase), "Theirs", 0).await;

    // Their task id inside my order names nothing I can see.
    let (status, _) = reorder(
        &app,
        &mine,
        my_project,
        json!({
            "phases": [my_phase],
            "tasks": [{ "phase_id": my_phase, "task_ids": [my_task, their_task] }]
        }),
    )
    .await;
    assert_eq!(status, 404, "a foreign task is out of scope");

    // And my order did not move.
    assert_eq!(titles(&task_titles(&app, &mine, my_project).await), vec!["Mine"]);
    // Their plan still reads as one task in their phase.
    let their_tasks = task_titles(&app, &theirs, their_project).await;
    assert_eq!(titles(&their_tasks), vec!["Theirs"]);
    assert_eq!(their_tasks[0].1, Some(their_phase));
}
