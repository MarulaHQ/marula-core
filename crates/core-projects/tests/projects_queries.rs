//! Projects-line query-layer coverage (the private build tracker Phase 3, migration 0030).
//!
//! Purpose: prove the project/phase/task/dep/view/template substrate at
//! the persistence boundary, under the real RLS role —
//! 1. Project CRUD + filters + tri-state patch + name-uniqueness check.
//! 2. Phase/task lifecycle: completion stamps `completed_at` atomically.
//! 3. Dependency edges gate readiness: `incomplete_deps` names blockers;
//!    PUT-replacement validates through `core_projects::deps`.
//! 4. Template instantiation executes the validated expansion plan
//!    transactionally (phases → tasks → deps) and lands milestone flags.
//! 5. Views: project-scoped + org-wide, listing scope semantics.
//! 6. RLS: another org's rows are invisible through the app role.
//!
//! Non-purpose: HTTP/auth/event-envelope behavior (owned by
//! core-api/tests/projects_flow.rs); pure graph + spec logic (unit-tested
//! in the core-projects lib).
//!
//! Org rows use fixed UUIDs 300–399 (projects-lane slots) so parallel
//! suites sharing one database stay collision-free.

mod common;

use common::{db_now, org_tx, seed_org, test_db, TestDb};
use core_domain::entities::{BillingMethod, PhaseStatus, ProjectStatus, TaskStatus};
use core_projects::deps;
use core_projects::template;
use std::collections::HashSet;
use uuid::Uuid;

async fn make_project(db: &TestDb, org: Uuid, name: &str) -> core_domain::entities::Project {
    let mut tx = org_tx(&db.app, org).await;
    let project = core_db::projects::insert_project(
        &mut *tx,
        core_db::projects::NewProject {
            id: Uuid::now_v7(),
            org_id: org,
            client_id: None,
            name,
            description: Some("seeded"),
            billing_method: BillingMethod::Milestone,
            budget_minor: Some(100_000),
            currency: Some("USD"),
            start_date: None,
            due_date: None,
            billing_contract_id: None,
        },
    )
    .await
    .expect("insert project");
    tx.commit().await.expect("commit");
    project
}

// --- 1. project CRUD --------------------------------------------------------------

#[tokio::test]
async fn project_crud_filters_and_tri_state_patch() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 300).await;
    let project = make_project(&db, org, "Migrate HQ").await;

    // Read-back agrees.
    let mut tx = org_tx(&db.app, org).await;
    let fetched = core_db::projects::get_project(&mut *tx, org, project.id)
        .await
        .expect("get");
    assert_eq!(fetched.name, "Migrate HQ");
    assert_eq!(fetched.status, ProjectStatus::Planning);
    assert_eq!(fetched.billing_method, BillingMethod::Milestone);
    assert_eq!(fetched.budget_minor, Some(100_000));

    // Filters.
    let active = make_project(&db, org, "Second").await;
    core_db::projects::update_project(
        &mut *tx,
        org,
        active.id,
        core_db::projects::ProjectPatch {
            name: None,
            description: None,
            client_id: None,
            status: Some(ProjectStatus::Active),
            billing_method: None,
            budget_minor: None,
            currency: None,
            start_date: None,
            due_date: None,
            billing_contract_id: None,
        },
    )
    .await
    .expect("activate");
    let by_status = core_db::projects::list_projects(
        &mut *tx,
        org,
        core_db::projects::ProjectFilter {
            status: Some(ProjectStatus::Active),
            client_id: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("list");
    assert_eq!(by_status.len(), 1);
    assert_eq!(by_status[0].id, active.id);
    let all = core_db::projects::list_projects(
        &mut *tx,
        org,
        core_db::projects::ProjectFilter {
            status: None,
            client_id: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("list all");
    assert_eq!(all.len(), 2);

    // Tri-state patch: Some(None) clears, None preserves.
    let patched = core_db::projects::update_project(
        &mut *tx,
        org,
        project.id,
        core_db::projects::ProjectPatch {
            name: Some("Migrate HQ (2027)"),
            description: Some(None),
            client_id: None,
            status: None,
            billing_method: None,
            budget_minor: Some(None),
            currency: Some(None),
            start_date: None,
            due_date: None,
            billing_contract_id: None,
        },
    )
    .await
    .expect("patch");
    assert_eq!(patched.name, "Migrate HQ (2027)");
    assert_eq!(patched.description, None, "Some(None) clears");
    assert_eq!(patched.budget_minor, None, "Some(None) clears the budget");
    assert_eq!(patched.currency, None);
    assert_eq!(
        patched.status,
        ProjectStatus::Planning,
        "None preserves status"
    );

    // Name-uniqueness pre-check + status filter are lane helpers.
    assert!(
        core_db::projects::project_name_taken(&mut *tx, org, "Migrate HQ (2027)")
            .await
            .expect("name check")
    );
    assert!(
        !core_db::projects::project_name_taken(&mut *tx, org, "unused name")
            .await
            .expect("name check")
    );
}

// --- 2. phase/task lifecycle ------------------------------------------------------

#[tokio::test]
async fn phase_and_task_completion_stamp_timestamps() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 302).await;
    let project = make_project(&db, org, "Onboarding").await;
    let mut tx = org_tx(&db.app, org).await;

    let phase = core_db::projects::insert_phase(
        &mut *tx,
        core_db::projects::NewPhase {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            name: "Discovery",
            description: None,
            sort_order: 0,
            is_milestone: true,
            milestone_amount_minor: Some(250_000),
            milestone_currency: Some("USD"),
            start_date: None,
            end_date: None,
        },
    )
    .await
    .expect("phase");
    assert_eq!(phase.status, PhaseStatus::Pending);

    let task = core_db::projects::insert_task(
        &mut *tx,
        core_db::projects::NewTask {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            phase_id: Some(phase.id),
            title: "Kickoff",
            description: None,
            assignee_id: None,
            sort_order: 0,
            is_milestone: true,
            milestone_amount_minor: Some(75_000),
            milestone_currency: Some("USD"),
            due_at: None,
        },
    )
    .await
    .expect("task");
    assert_eq!(task.status, TaskStatus::Pending);

    let at = db_now();
    let completed_task = core_db::projects::complete_task(&mut *tx, org, task.id, at)
        .await
        .expect("complete task");
    assert_eq!(completed_task.status, TaskStatus::Completed);
    assert_eq!(completed_task.completed_at, Some(at));

    let completed_phase = core_db::projects::complete_phase(&mut *tx, org, phase.id, at)
        .await
        .expect("complete phase");
    assert_eq!(completed_phase.status, PhaseStatus::Completed);
    assert_eq!(completed_phase.completed_at, Some(at));

    // The entity's own enum round-trips survive the text round-trip.
    let tasks = core_db::projects::list_tasks(
        &mut *tx,
        org,
        project.id,
        core_db::projects::TaskFilter {
            phase_id: None,
            status: Some(TaskStatus::Completed),
            assignee_id: None,
            milestone: Some(true),
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("filtered tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, task.id);
}

// --- 3. dependency edges ------------------------------------------------------------

#[tokio::test]
async fn dependency_edges_gate_readiness_and_replace_validates() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 304).await;
    let project = make_project(&db, org, "Cutover").await;
    let mut tx = org_tx(&db.app, org).await;

    let a = core_db::projects::insert_task(
        &mut *tx,
        core_db::projects::NewTask {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            phase_id: None,
            title: "A",
            description: None,
            assignee_id: None,
            sort_order: 0,
            is_milestone: false,
            milestone_amount_minor: None,
            milestone_currency: None,
            due_at: None,
        },
    )
    .await
    .expect("task a");
    let b = core_db::projects::insert_task(
        &mut *tx,
        core_db::projects::NewTask {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            phase_id: None,
            title: "B",
            description: None,
            assignee_id: None,
            sort_order: 1,
            is_milestone: false,
            milestone_amount_minor: None,
            milestone_currency: None,
            due_at: None,
        },
    )
    .await
    .expect("task b");
    let c = core_db::projects::insert_task(
        &mut *tx,
        core_db::projects::NewTask {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            phase_id: None,
            title: "C",
            description: None,
            assignee_id: None,
            sort_order: 2,
            is_milestone: false,
            milestone_amount_minor: None,
            milestone_currency: None,
            due_at: None,
        },
    )
    .await
    .expect("task c");

    // B waits on A and C.
    core_db::projects::set_task_deps(&mut tx, org, b.id, &[a.id, c.id])
        .await
        .expect("set deps");

    // Readiness: both blockers listed while A/C are open.
    let mut blocking = core_db::projects::incomplete_deps(&mut *tx, org, b.id)
        .await
        .expect("incomplete deps");
    blocking.sort();
    let mut expected = vec![a.id, c.id];
    expected.sort();
    assert_eq!(blocking, expected, "both open deps block completion");

    // Completing A frees exactly that edge.
    core_db::projects::complete_task(&mut *tx, org, a.id, chrono::Utc::now())
        .await
        .expect("complete a");
    let blocking = core_db::projects::incomplete_deps(&mut *tx, org, b.id)
        .await
        .expect("incomplete deps");
    assert_eq!(blocking, vec![c.id], "only C still blocks");

    // The dependency rows carry full task rows for the blocked-by UI.
    let deps_of_b = core_db::projects::list_task_deps(&mut *tx, org, b.id)
        .await
        .expect("deps");
    let ids: HashSet<Uuid> = deps_of_b.iter().map(|t| t.id).collect();
    assert_eq!(ids, HashSet::from([a.id, c.id]));

    // PUT replacement validated through the pure layer: replacing B's set
    // with {C} drops A; introducing B→A again while A→… nothing — but a
    // cycle A→B would be rejected.
    let edges = core_db::projects::list_project_dep_edges(&mut *tx, org, project.id)
        .await
        .expect("edges");
    let members: HashSet<Uuid> =
        core_db::projects::list_project_task_ids(&mut *tx, org, project.id)
            .await
            .expect("members")
            .into_iter()
            .collect();
    let validated = deps::validate_replace(&edges, b.id, &[c.id], &members).expect("valid set");
    core_db::projects::set_task_deps(&mut tx, org, b.id, &validated)
        .await
        .expect("replace deps");
    let deps_of_b = core_db::projects::list_task_deps(&mut *tx, org, b.id)
        .await
        .expect("deps after replace");
    assert_eq!(deps_of_b.len(), 1);
    assert_eq!(deps_of_b[0].id, c.id);

    // A cycle through the NEW set is rejected before any write.
    core_db::projects::add_task_dep(&mut *tx, org, a.id, b.id)
        .await
        .expect("a waits on b (no cycle yet)");
    let edges = core_db::projects::list_project_dep_edges(&mut *tx, org, project.id)
        .await
        .expect("edges");
    let err = deps::validate_replace(&edges, b.id, &[a.id], &members).unwrap_err();
    assert_eq!(err.reason, deps::RejectionReason::Cycle);
}

// --- 4. template instantiation --------------------------------------------------------

#[tokio::test]
async fn template_instantiation_expands_transactionally() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 306).await;

    let spec = serde_json::json!({
        "spec_version": 1,
        "billing_method": "milestone",
        "currency": "USD",
        "phases": [
            { "key": "discovery", "name": "Discovery", "sort_order": 0,
              "tasks": [
                { "key": "kickoff", "title": "Kickoff" },
                { "key": "audit", "title": "Current-state audit", "sort_order": 1 }
              ] },
            { "key": "delivery", "name": "Delivery", "sort_order": 1,
              "is_milestone": true,
              "milestone_amount_minor": 500_000,
              "tasks": [
                { "key": "rollout", "title": "Rollout",
                  "depends_on": ["kickoff", "audit"] }
              ] }
        ],
        "tasks": [
            { "key": "closeout", "title": "Close-out", "phase": "delivery",
              "depends_on": ["rollout"] }
        ]
    });
    let parsed = template::parse_spec(&spec).expect("valid spec");
    let plan = parsed.expand();

    let mut tx = org_tx(&db.app, org).await;
    let template_row = core_db::projects::insert_template(
        &mut *tx,
        core_db::projects::NewTemplate {
            id: Uuid::now_v7(),
            org_id: org,
            name: "Migration Standard",
            description: Some("Two-phase migration"),
            spec: &spec,
        },
    )
    .await
    .expect("template");

    // The route executes this same plan in one transaction.
    let project = core_db::projects::insert_project(
        &mut *tx,
        core_db::projects::NewProject {
            id: Uuid::now_v7(),
            org_id: org,
            client_id: None,
            name: "Acme Migration",
            description: template_row.description.as_deref(),
            billing_method: plan
                .billing_method
                .as_deref()
                .map(|s| s.parse().expect("billing method"))
                .expect("spec declares milestone"),
            budget_minor: None,
            currency: plan.currency.as_deref(),
            start_date: None,
            due_date: None,
            billing_contract_id: None,
        },
    )
    .await
    .expect("project");

    let mut phase_ids = std::collections::HashMap::new();
    for phase in &plan.phases {
        let phase_id = Uuid::now_v7();
        core_db::projects::insert_phase(
            &mut *tx,
            core_db::projects::NewPhase {
                id: phase_id,
                org_id: org,
                project_id: project.id,
                name: &phase.name,
                description: phase.description.as_deref(),
                sort_order: phase.sort_order,
                is_milestone: phase.is_milestone,
                milestone_amount_minor: phase.milestone_amount_minor,
                milestone_currency: phase.milestone_currency.as_deref(),
                start_date: None,
                end_date: None,
            },
        )
        .await
        .expect("planned phase");
        phase_ids.insert(phase.key.clone(), phase_id);
    }
    let mut task_ids = std::collections::HashMap::new();
    for task in &plan.tasks {
        let phase_id = task.phase_key.as_deref().map(|k| phase_ids[k]);
        let task_id = Uuid::now_v7();
        core_db::projects::insert_task(
            &mut *tx,
            core_db::projects::NewTask {
                id: task_id,
                org_id: org,
                project_id: project.id,
                phase_id,
                title: &task.title,
                description: task.description.as_deref(),
                assignee_id: None,
                sort_order: task.sort_order,
                is_milestone: task.is_milestone,
                milestone_amount_minor: task.milestone_amount_minor,
                milestone_currency: task.milestone_currency.as_deref(),
                due_at: None,
            },
        )
        .await
        .expect("planned task");
        task_ids.insert(task.key.clone(), task_id);
    }
    for task in &plan.tasks {
        let task_id = task_ids[&task.key];
        for dep_key in &task.depends_on {
            core_db::projects::add_task_dep(&mut *tx, org, task_id, task_ids[dep_key])
                .await
                .expect("planned dep");
        }
    }
    tx.commit().await.expect("commit");

    // Verify the expansion landed faithfully.
    let mut tx = org_tx(&db.app, org).await;
    let phases = core_db::projects::list_phases(&mut *tx, org, project.id)
        .await
        .expect("phases");
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].name, "Discovery");
    assert!(!phases[0].is_milestone);
    assert_eq!(phases[1].name, "Delivery");
    assert!(phases[1].is_milestone);
    assert_eq!(phases[1].milestone_amount_minor, Some(500_000));

    let tasks = core_db::projects::list_tasks(
        &mut *tx,
        org,
        project.id,
        core_db::projects::TaskFilter {
            phase_id: None,
            status: None,
            assignee_id: None,
            milestone: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("tasks");
    assert_eq!(tasks.len(), 4, "3 phase-block + 1 project-level task");
    let rollout = tasks
        .iter()
        .find(|t| t.title == "Rollout")
        .expect("rollout");
    assert!(
        !rollout.is_milestone,
        "only the delivery PHASE carries the milestone flag"
    );
    let rollout_deps = core_db::projects::list_task_deps(&mut *tx, org, rollout.id)
        .await
        .expect("rollout deps");
    assert_eq!(rollout_deps.len(), 2, "kickoff + audit block rollout");

    let closeout = tasks
        .iter()
        .find(|t| t.title == "Close-out")
        .expect("close-out");
    assert_eq!(
        closeout.phase_id,
        Some(phase_ids["delivery"]),
        "project-level task landed in its referenced phase"
    );

    // Deleting the template leaves the instantiated project untouched.
    assert!(
        core_db::projects::delete_template(&mut *tx, org, template_row.id)
            .await
            .expect("delete template")
    );
    let still = core_db::projects::get_project(&mut *tx, org, project.id)
        .await
        .expect("project survives template deletion");
    assert_eq!(still.id, project.id);
}

// --- 5. views ---------------------------------------------------------------------------

#[tokio::test]
async fn views_scope_listing_and_lifecycle() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 308).await;
    let project = make_project(&db, org, "Viewed").await;
    let mut tx = org_tx(&db.app, org).await;

    let spec = serde_json::json!({"group_by": "phase", "filters": {"status": "pending"}});
    let project_view = core_db::projects::insert_view(
        &mut *tx,
        core_db::projects::NewView {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: Some(project.id),
            name: "Board",
            spec: &spec,
            is_default: true,
        },
    )
    .await
    .expect("project view");
    let org_view = core_db::projects::insert_view(
        &mut *tx,
        core_db::projects::NewView {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: None,
            name: "All work",
            spec: &serde_json::json!({}),
            is_default: false,
        },
    )
    .await
    .expect("org view");

    // Scoped listing sees only the project's views; unscoped sees both.
    let scoped = core_db::projects::list_views(&mut *tx, org, Some(project.id))
        .await
        .expect("scoped");
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].id, project_view.id);
    assert_eq!(scoped[0].spec, spec);
    let all = core_db::projects::list_views(&mut *tx, org, None)
        .await
        .expect("all");
    assert_eq!(all.len(), 2);
    // Default views sort first within the scope.
    assert!(all[0].is_default);

    let updated = core_db::projects::update_view(
        &mut *tx,
        org,
        project_view.id,
        core_db::projects::ViewPatch {
            name: Some("Board v2"),
            spec: Some(&serde_json::json!({"group_by": "assignee"})),
            is_default: Some(false),
        },
    )
    .await
    .expect("update");
    assert_eq!(updated.name, "Board v2");
    assert_eq!(updated.spec["group_by"], "assignee");
    assert!(!updated.is_default);

    assert!(core_db::projects::delete_view(&mut *tx, org, org_view.id)
        .await
        .expect("delete"));
    assert!(!core_db::projects::delete_view(&mut *tx, org, org_view.id)
        .await
        .expect("delete twice is false"));
}

// --- 6. RLS -------------------------------------------------------------------------------

#[tokio::test]
async fn rls_hides_other_orgs_project_rows() {
    let db = test_db().await;
    let (org_a, _user_a) = seed_org(&db, 310).await;
    let (org_b, _user_b) = seed_org(&db, 311).await;
    let project_a = make_project(&db, org_a, "Org A Secret").await;

    // Org B's app-role transaction cannot read org A's project…
    let mut tx_b = org_tx(&db.app, org_b).await;
    let missed = core_db::projects::get_project(&mut *tx_b, org_b, project_a.id).await;
    assert!(
        matches!(missed, Err(core_db::DbError::NotFound)),
        "cross-org reads must be rls_sparse/not-found, got {missed:?}"
    );
    let visible = core_db::projects::list_projects(
        &mut *tx_b,
        org_b,
        core_db::projects::ProjectFilter {
            status: None,
            client_id: None,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("list");
    assert!(visible.is_empty(), "org B sees none of org A's projects");

    // …and org A still sees its own row through the same role.
    let mut tx_a = org_tx(&db.app, org_a).await;
    let seen = core_db::projects::get_project(&mut *tx_a, org_a, project_a.id)
        .await
        .expect("own org visible");
    assert_eq!(seen.id, project_a.id);
}

/// Migration 0030 covers every new table with at least one RLS policy —
/// the CI gate query re-run against the projects tables specifically.
#[tokio::test]
async fn migration_0030_tables_pass_rls_coverage() {
    let db = test_db().await;
    core_db::assert_rls_coverage(&db.admin)
        .await
        .expect("all user-data tables carry RLS policies");
    for table in [
        "projects",
        "project_phases",
        "project_tasks",
        "project_task_deps",
        "project_views",
        "project_templates",
    ] {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_policies WHERE tablename = $1")
                .bind(table)
                .fetch_one(&db.admin)
                .await
                .expect("policy query");
        assert!(count >= 1, "{table} has no RLS policy");
    }
}

/// Milestone rows round-trip their amount + currency text faithfully, and
/// a direct outbox read proves the table exists for the seam (the event
/// itself is emitted at the route layer).
#[tokio::test]
async fn milestone_amounts_round_trip() {
    let db = test_db().await;
    let (org, _user) = seed_org(&db, 312).await;
    let project = make_project(&db, org, "Billing").await;
    let mut tx = org_tx(&db.app, org).await;
    let task = core_db::projects::insert_task(
        &mut *tx,
        core_db::projects::NewTask {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            phase_id: None,
            title: "Invoice trigger",
            description: None,
            assignee_id: None,
            sort_order: 0,
            is_milestone: true,
            milestone_amount_minor: Some(123_456),
            milestone_currency: Some("EUR"),
            due_at: None,
        },
    )
    .await
    .expect("task");
    let fetched = core_db::projects::get_task(&mut *tx, org, task.id)
        .await
        .expect("fetch");
    assert_eq!(fetched.milestone_amount_minor, Some(123_456));
    assert_eq!(fetched.milestone_currency.as_deref(), Some("EUR"));
    assert!(fetched.is_milestone);
    // Sanity: the outbox table is present for the seam (the
    // milestone event itself is emitted at the route layer).
    let _count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox_events")
        .fetch_one(&mut *tx)
        .await
        .expect("outbox exists");
}

/// Wave 34 claims close-out (claim 6): a phase created WITH dates is drawn
/// on the Gantt; one created without is not. The asymmetry is the feature —
/// a NOT NULL default would draw a schedule nobody chose.
#[tokio::test]
async fn phase_dates_roundtrip_and_absent_dates_stay_absent() {
    let db = test_db().await;
    // 311 is already used by `rls_hides_other_orgs_project_rows` in this
    // same file, and `seed_org` derives a DETERMINISTIC id from `n` — so two
    // tests sharing a value collide on the orgs primary key. 330 is free.
    let (org, _user) = seed_org(&db, 330).await;
    let project = make_project(&db, org, "Dated Gantt").await;
    let mut tx = org_tx(&db.app, org).await;

    let dated = core_db::projects::insert_phase(
        &mut *tx,
        core_db::projects::NewPhase {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            name: "Site survey",
            description: None,
            sort_order: 0,
            is_milestone: false,
            milestone_amount_minor: None,
            milestone_currency: None,
            start_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 1),
            end_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 14),
        },
    )
    .await
    .expect("dated phase");
    let undated = core_db::projects::insert_phase(
        &mut *tx,
        core_db::projects::NewPhase {
            id: Uuid::now_v7(),
            org_id: org,
            project_id: project.id,
            name: "Handover",
            description: None,
            sort_order: 1,
            is_milestone: false,
            milestone_amount_minor: None,
            milestone_currency: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
    .expect("undated phase");

    // The entity's own predicates, not a re-implementation of them.
    assert!(dated.is_dated());
    assert!(!undated.is_dated());
    // Inclusive of both ends: 1st to 14th is 14 days.
    assert_eq!(dated.planned_days(), Some(14));
    assert_eq!(undated.planned_days(), None);

    // The Gantt read model partitions on exactly the same rule, and carries
    // budget columns as NULL for an unbudgeted phase rather than zero.
    let gantt = core_db::projects::phase_gantt(&mut *tx, org, project.id)
        .await
        .expect("gantt");
    assert_eq!(
        gantt.len(),
        2,
        "both phases are returned; the API splits them"
    );
    let g_dated = gantt.iter().find(|g| g.id == dated.id).expect("dated");
    let g_undated = gantt.iter().find(|g| g.id == undated.id).expect("undated");
    assert!(g_dated.start_date.is_some() && g_dated.end_date.is_some());
    assert!(g_undated.start_date.is_none() && g_undated.end_date.is_none());
    assert_eq!(
        g_dated.consumed_minor, 0,
        "no time logged ⇒ nothing consumed"
    );
    assert!(g_dated.budget_minor.is_none(), "unbudgeted is None, not 0");

    // Tri-state patch: absent leaves the window alone, Some(None) clears it.
    let renamed = core_db::projects::update_phase(
        &mut *tx,
        org,
        dated.id,
        core_db::projects::PhasePatch {
            name: Some("Site survey (renamed)"),
            description: None,
            sort_order: None,
            status: None,
            is_milestone: None,
            milestone_amount_minor: None,
            milestone_currency: None,
            start_date: None,
            end_date: None,
        },
    )
    .await
    .expect("rename");
    assert_eq!(renamed.name, "Site survey (renamed)");
    assert!(renamed.is_dated(), "a rename must not undate a phase");

    let cleared = core_db::projects::update_phase(
        &mut *tx,
        org,
        dated.id,
        core_db::projects::PhasePatch {
            name: None,
            description: None,
            sort_order: None,
            status: None,
            is_milestone: None,
            milestone_amount_minor: None,
            milestone_currency: None,
            start_date: Some(None),
            end_date: Some(None),
        },
    )
    .await
    .expect("clear dates");
    assert!(
        !cleared.is_dated(),
        "Some(None) takes the phase off the Gantt"
    );

    // The database refuses a backwards window whatever the API sends, so a
    // future caller bypassing the handler still cannot write one.
    let backwards =
        sqlx::query("UPDATE project_phases SET start_date = $2, end_date = $1 WHERE id = $3")
            .bind(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())
            .bind(chrono::NaiveDate::from_ymd_opt(2026, 10, 14).unwrap())
            .bind(undated.id)
            .execute(&mut *tx)
            .await;
    assert!(
        backwards.is_err(),
        "end < start must be a constraint violation"
    );

    tx.commit().await.expect("commit");
}
