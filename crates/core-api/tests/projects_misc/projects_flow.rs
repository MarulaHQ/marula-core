//! Projects-line endpoint flow (the private build tracker Phase 3) through the REAL
//! core-api router.
//!
//! Purpose: the public contract over HTTP —
//! 1. Project lifecycle: create (idempotency replay = byte-identical),
//!    version-gated patch (typed 409 `version_conflict`), phases, tasks
//!    with dependencies, and the guarded completion path.
//! 2. Dependency gating: premature completion returns the typed 409
//!    `task_dependencies_unsatisfied` naming the blockers; completing in
//!    order succeeds.
//! 3. Milestone billing seam: completing a milestone task emits exactly
//!    ONE `project.milestone_completed` outbox event carrying the amount;
//!    a retried completion (same idempotency key) replays without
//!    double-emitting.
//! 4. Templates: spec validation (422), instantiation expands phases +
//!    tasks + deps, template deletion never touches instantiated projects.
//! 5. Views: scoped CRUD, duplicate-name 409.
//! 6. Authz + envelope: 401 unauthenticated, 403 technician
//!    (`reason: "forbidden"`), 404 `reason: "not_found"`, 422
//!    `reason: "validation_failed"`; dual-mount parity on /api/v1 and /api.
//!
//! Database selection follows settings_flow's contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (no schema reset).

use crate::common::{auth, body_json, register_org_token, send_headers as send, state_with};
use axum::Router;
use core_api::state::AppState;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn test_state() -> AppState {
    state_with(|config| config.secret_key = core_api::config::SecretKey([31; 32])).await
}

async fn register_org(app: &Router, slug: &str) -> String {
    register_org_token(app, "Projects Co", slug).await
}

/// Creates a second user with the builtin technician role, then logs in
/// over HTTP so authz assertions run against a REAL principal.
async fn provision_technician(state: &AppState, app: &Router, org_id: Uuid, email: &str) -> String {
    let password_hash = core_api::auth::password::hash("tech-password-1234").expect("hash");
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    let tech_role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role exists from registration");
    let user_id = Uuid::now_v7();
    core_db::users::create_user(
        &mut *tx,
        user_id,
        org_id,
        email,
        "Tech User",
        &password_hash,
        "UTC",
    )
    .await
    .expect("user insert");
    core_db::users::assign_role(&mut *tx, user_id, tech_role)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit");

    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/login",
        &[],
        Some(json!({ "email": email, "password": "tech-password-1234" })),
    )
    .await;
    assert_eq!(response.status(), 200, "technician login must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn org_id_of(app: &Router, token: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        &[auth(token)],
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    body_json(response).await["org_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

fn bearer(token: &str) -> Vec<(&'static str, String)> {
    vec![auth(token)]
}

#[tokio::test]
async fn project_lifecycle_dependency_gating_and_milestone_event() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("projects-flow-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = bearer(&token);

    // --- create (instant-gated) -------------------------------------------
    let create_body = json!({
        "name": "HQ Migration",
        "billing_method": "milestone",
        "currency": "USD",
        "budget_minor": 10_000_000,
        "idempotency_key": "create-key-1"
    });
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(response.status(), 200, "create must succeed");
    let created = body_json(response).await;
    let project_id: Uuid = created["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(created["status"], "planning");
    assert_eq!(created["mutation"]["version"], 1, "fresh entity seq = 1");
    assert_eq!(created["mutation"]["seq"], 1);

    // --- idempotent replay: same key + same payload ⇒ SAME stored result ---
    let first = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(first.status(), 200);
    let first = body_json(first).await;
    let second = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer,
        Some(create_body.clone()),
    )
    .await;
    assert_eq!(second.status(), 200);
    let second = body_json(second).await;
    assert_eq!(first, second, "replay must be byte-identical");
    assert_eq!(
        first["id"],
        json!(project_id),
        "replay returns the SAME project"
    );

    // --- version gate: stale version ⇒ typed 409 ---------------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/projects/{project_id}"),
        &bearer,
        Some(json!({
            "name": "HQ Migration 2027",
            "version": 99
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "stale version is a typed conflict");
    let err = body_json(response).await;
    assert_eq!(err["error"]["reason"], "version_conflict");
    assert_eq!(err["error"]["expected"], 1);
    assert_eq!(err["error"]["provided"], 99);

    // --- correct patch ⇒ version increments ---------------------------------
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/projects/{project_id}"),
        &bearer,
        Some(json!({
            "name": "HQ Migration 2027",
            "version": 1
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let patched = body_json(response).await;
    assert_eq!(patched["name"], "HQ Migration 2027");
    assert_eq!(patched["mutation"]["version"], 2);
    assert_eq!(
        patched["mutation"]["before"]["name"], "HQ Migration",
        "the mutation carries the prior value (undo substrate)"
    );

    // --- budget without currency is a typed 422 -----------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer,
        Some(json!({ "name": "No Currency", "budget_minor": 500 })),
    )
    .await;
    assert_eq!(response.status(), 422);
    let err = body_json(response).await;
    assert_eq!(err["error"]["reason"], "validation_failed");

    // --- phase (milestone) ----------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/phases"),
        &bearer,
        Some(json!({
            "name": "Delivery",
            "is_milestone": true,
            "milestone_amount_minor": 4_000_000
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let phase = body_json(response).await;
    let phase_id: Uuid = phase["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(phase["status"], "pending");

    // --- tasks with dependencies ----------------------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer,
        Some(json!({ "title": "Survey", "phase_id": phase_id })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let survey = body_json(response).await;
    let survey_id: Uuid = survey["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer,
        Some(json!({
            "title": "Migrate",
            "phase_id": phase_id,
            "depends_on": [survey_id]
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let migrate = body_json(response).await;
    let migrate_id: Uuid = migrate["id"].as_str().unwrap().parse().unwrap();

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer,
        Some(json!({
            "title": "Go-live",
            "is_milestone": true,
            "milestone_amount_minor": 2_500_000,
            "depends_on": [migrate_id]
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let golive = body_json(response).await;
    let golive_id: Uuid = golive["id"].as_str().unwrap().parse().unwrap();

    // Cross-project dependency is rejected (422, not a silent edge).
    let other = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer,
        Some(json!({ "name": "Other Project" })),
    )
    .await;
    assert_eq!(other.status(), 200);
    let other_id: Uuid = body_json(other).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{other_id}/tasks"),
        &bearer,
        Some(json!({
            "title": "Foreign dep",
            "depends_on": [survey_id]
        })),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "validation_failed"
    );

    // --- dependency gating: premature completion is a typed 409 ---------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{migrate_id}/complete"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "blocked by unfinished Survey");
    let err = body_json(response).await;
    assert_eq!(err["error"]["reason"], "task_dependencies_unsatisfied");
    assert_eq!(
        err["error"]["blocking"].as_array().expect("blocking list")[0],
        json!(survey_id),
        "the 409 names the exact blocker"
    );

    // PATCH cannot skip the gate either.
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/projects/{project_id}/tasks/{migrate_id}"),
        &bearer,
        Some(json!({ "status": "completed" })),
    )
    .await;
    assert_eq!(response.status(), 422);

    // --- complete in order ------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{survey_id}/complete"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"], "completed");

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{migrate_id}/complete"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 200);
    let done = body_json(response).await;
    assert_eq!(done["status"], "completed");
    assert!(
        done["completed_at"].is_string(),
        "completion stamps completed_at"
    );

    // --- milestone completion emits EXACTLY ONE billing event --------------------
    let count_milestone_events = |pool: &PgPool, org: Uuid| {
        let pool = pool.clone();
        async move {
            let (count,): (i64,) = sqlx::query_as(
                "SELECT count(*) FROM outbox_events \
                 WHERE org_id = $1 AND event_type = 'project.milestone_completed'",
            )
            .bind(org)
            .fetch_one(&pool)
            .await
            .expect("outbox query");
            count
        }
    };
    let org = org_id_of(&app, &token).await;
    assert_eq!(
        count_milestone_events(&state.pool, org).await,
        0,
        "no billing events before any milestone completes"
    );

    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{golive_id}/complete"),
        &bearer,
        Some(json!({ "idempotency_key": "golive-complete" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let golive_done = body_json(response).await;
    assert_eq!(golive_done["status"], "completed");

    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT payload::text FROM outbox_events \
         WHERE org_id = $1 AND event_type = 'project.milestone_completed' \
         ORDER BY occurred_at",
    )
    .bind(org)
    .fetch_all(&state.pool)
    .await
    .expect("events");
    assert_eq!(events.len(), 1, "exactly one milestone_completed event");
    let event: Value = serde_json::from_str(&events[0].0).unwrap();
    assert_eq!(event["project_id"], json!(project_id));
    assert_eq!(event["task_id"], json!(golive_id));
    assert_eq!(event["amount"]["minor"], 2_500_000);
    assert_eq!(event["amount"]["currency"], "USD");
    assert_eq!(event["billing_method"], "milestone");

    // --- retried completion REPLAYS (no double-billing) ---------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{golive_id}/complete"),
        &bearer,
        Some(json!({ "idempotency_key": "golive-complete" })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let replayed = body_json(response).await;
    assert_eq!(replayed, golive_done, "replay is byte-identical");
    assert_eq!(
        count_milestone_events(&state.pool, org).await,
        1,
        "retry must not double-emit the billing event"
    );

    // Second completion attempt without the key is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{golive_id}/complete"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409);
    assert_eq!(body_json(response).await["error"]["reason"], "conflict");

    // --- phase completion carries the phase milestone ------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/phases/{phase_id}/complete"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["status"], "completed");
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT payload::text FROM outbox_events \
         WHERE org_id = $1 AND event_type = 'project.milestone_completed' \
         ORDER BY occurred_at",
    )
    .bind(org)
    .fetch_all(&state.pool)
    .await
    .expect("events");
    assert_eq!(events.len(), 2, "phase milestone adds one event");
    let event: Value = serde_json::from_str(&events[1].0).unwrap();
    assert_eq!(event["phase_id"], json!(phase_id));
    assert_eq!(event["amount"]["minor"], 4_000_000);

    // --- dependencies read + PUT replace ---------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/tasks/{golive_id}/dependencies"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let deps = body_json(response).await;
    assert_eq!(deps["depends_on"].as_array().unwrap().len(), 1);

    let response = send(
        &app,
        axum::http::Method::PUT,
        &format!("/api/v1/projects/{project_id}/tasks/{golive_id}/dependencies"),
        &bearer,
        Some(json!({ "depends_on": [] })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["depends_on"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // --- list + get + dual mount --------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/projects?status=planning",
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let listed = body_json(response).await;
    assert!(listed.as_array().unwrap().len() >= 2);

    let canonical = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(canonical.status(), 200);
    let alias = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/projects/{project_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(alias.status(), 200);
    assert_eq!(
        body_json(canonical).await,
        body_json(alias).await,
        "canonical and legacy mounts serve the same handler"
    );
}

#[tokio::test]
async fn templates_validate_instantiate_and_survive_their_own_deletion() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("projects-tmpl-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = bearer(&token);

    // Invalid spec is rejected EAGERLY at create (422, never persisted).
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-templates",
        &bearer,
        Some(json!({
            "name": "Broken",
            "spec": { "spec_version": 1, "tasks": [
                { "key": "a", "title": "A", "depends_on": ["missing"] }
            ]}
        })),
    )
    .await;
    assert_eq!(response.status(), 422);
    assert_eq!(
        body_json(response).await["error"]["reason"],
        "validation_failed"
    );

    // Cyclic spec rejected too.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-templates",
        &bearer,
        Some(json!({
            "name": "Cyclic",
            "spec": { "spec_version": 1, "tasks": [
                { "key": "a", "title": "A", "depends_on": ["b"] },
                { "key": "b", "title": "B", "depends_on": ["a"] }
            ]}
        })),
    )
    .await;
    assert_eq!(response.status(), 422);

    let spec = json!({
        "spec_version": 1,
        "billing_method": "milestone",
        "currency": "USD",
        "phases": [
            { "key": "discovery", "name": "Discovery", "sort_order": 0,
              "tasks": [
                { "key": "kickoff", "title": "Kickoff" },
                { "key": "plan", "title": "Plan", "sort_order": 1 }
              ] }
        ],
        "tasks": [
            { "key": "cutover", "title": "Cutover", "phase": "discovery",
              "is_milestone": true, "milestone_amount_minor": 999_900,
              "depends_on": ["kickoff", "plan"] }
        ]
    });
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-templates",
        &bearer,
        Some(json!({ "name": "Migration Standard", "description": "Two-phase", "spec": spec })),
    )
    .await;
    assert_eq!(response.status(), 200, "valid spec persists");
    let template = body_json(response).await;
    let template_id: Uuid = template["id"].as_str().unwrap().parse().unwrap();

    // Duplicate template name is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-templates",
        &bearer,
        Some(json!({ "name": "migration standard", "spec": spec })),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Instantiate.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/project-templates/{template_id}/instantiate"),
        &bearer,
        Some(json!({
            "name": "Acme Migration",
            "idempotency_key": format!("inst-{}", Uuid::now_v7())
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let instantiated = body_json(response).await;
    let project_id: Uuid = instantiated["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(instantiated["phase_count"], 1);
    assert_eq!(instantiated["task_count"], 3);
    assert_eq!(instantiated["billing_method"], "milestone");

    // The expansion landed: phases, tasks, and the dependency gate.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/phases"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await.as_array().unwrap().len(), 1);

    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}/tasks"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let tasks = body_json(response).await;
    let tasks_arr = tasks.as_array().unwrap();
    assert_eq!(tasks_arr.len(), 3);
    let cutover = tasks_arr
        .iter()
        .find(|t| t["title"] == "Cutover")
        .expect("cutover in referenced phase");
    assert!(cutover["phase_id"].is_string(), "phase reference resolved");
    assert_eq!(cutover["milestone_amount_minor"], 999_900);

    let cutover_id: Uuid = cutover["id"].as_str().unwrap().parse().unwrap();
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/projects/{project_id}/tasks/{cutover_id}/complete"),
        &bearer,
        Some(json!({})),
    )
    .await;
    assert_eq!(response.status(), 409, "cutover blocked by template deps");
    let err = body_json(response).await;
    assert_eq!(err["error"]["reason"], "task_dependencies_unsatisfied");
    assert_eq!(err["error"]["blocking"].as_array().unwrap().len(), 2);

    // Deleting the template leaves the instantiated project untouched.
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/project-templates/{template_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{project_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "project survives template deletion");

    // Template list reflects the deletion.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/project-templates",
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert!(body_json(response).await.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn views_crud_with_scoped_duplicate_detection() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state);
    let slug = format!("projects-views-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let bearer = bearer(&token);

    let project_id: Uuid = {
        let response = send(
            &app,
            axum::http::Method::POST,
            "/api/v1/projects",
            &bearer,
            Some(json!({ "name": "Viewed Project" })),
        )
        .await;
        assert_eq!(response.status(), 200);
        body_json(response).await["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    };

    // Project-scoped view.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-views",
        &bearer,
        Some(json!({
            "name": "Board",
            "project_id": project_id,
            "spec": { "group_by": "phase", "filters": { "status": "pending" } },
            "is_default": true
        })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let view = body_json(response).await;
    let view_id: Uuid = view["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(view["spec"]["group_by"], "phase");

    // Duplicate name in the SAME scope is a typed 409.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-views",
        &bearer,
        Some(json!({ "name": "board", "project_id": project_id, "spec": {} })),
    )
    .await;
    assert_eq!(response.status(), 409);

    // Org-wide view (project_id absent) may share the name — different scope.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/project-views",
        &bearer,
        Some(json!({ "name": "All work", "spec": {} })),
    )
    .await;
    assert_eq!(response.status(), 200);
    let org_view_id: Uuid = body_json(response).await["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Scoped listing sees one; unscoped sees both.
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/project-views?project_id={project_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await.as_array().unwrap().len(), 1);
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/project-views",
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await.as_array().unwrap().len(), 2);

    // Patch + delete + post-delete 404 (rls_sparse on a mutation).
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/project-views/{view_id}"),
        &bearer,
        Some(json!({ "spec": { "group_by": "assignee" }, "is_default": false })),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(body_json(response).await["spec"]["group_by"], "assignee");

    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/project-views/{org_view_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let response = send(
        &app,
        axum::http::Method::DELETE,
        &format!("/api/v1/project-views/{org_view_id}"),
        &bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(body_json(response).await["error"]["reason"], "rls_sparse");
}

#[tokio::test]
async fn projects_authz_and_typed_error_envelope() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("projects-authz-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org = org_id_of(&app, &token).await;
    let tech_token = provision_technician(&state, &app, org, &format!("tech@{slug}.test")).await;

    // Unauthenticated: 401.
    let response = send(&app, axum::http::Method::GET, "/api/v1/projects", &[], None).await;
    assert_eq!(response.status(), 401);

    // Technician lacks projects:create: 403 with the typed reason.
    let response = send(
        &app,
        axum::http::Method::POST,
        "/api/v1/projects",
        &bearer(&tech_token),
        Some(json!({ "name": "Forbidden Project" })),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(body_json(response).await["error"]["reason"], "forbidden");

    // Technician also cannot read the list (view is matrix-gated for techs).
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/projects",
        &bearer(&tech_token),
        None,
    )
    .await;
    assert_eq!(response.status(), 403);

    // Unknown project: 404 not_found on reads.
    let missing = Uuid::now_v7();
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/projects/{missing}"),
        &bearer(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(body_json(response).await["error"]["reason"], "not_found");

    // Mutation on a missing target surfaces rls_sparse (IF3).
    let response = send(
        &app,
        axum::http::Method::PATCH,
        &format!("/api/v1/projects/{missing}"),
        &bearer(&token),
        Some(json!({ "name": "Ghost" })),
    )
    .await;
    assert_eq!(response.status(), 404);
    assert_eq!(body_json(response).await["error"]["reason"], "rls_sparse");
}
