//! The workflow executor's persistence tests (the private build tracker Wave 43 lane 15,
//! `p43-workflow-executor`, migration 0132).
//!
//! Purpose: prove the claims the executor makes about the DATABASE, which
//! the pure engine's unit tests cannot reach because they hold no connection
//! and no stack:
//!
//! 1. a workflow document (an `automation_rules.definition`) is matched by
//!    trigger event, and the simple tier (NULL definition) is NOT — the two
//!    consumers partition the table, so a document is never run twice;
//! 2. the rule carries its author and its revision, which lane 14's
//!    `rule_version` observes;
//! 3. the WAITERS index finds a parked run by event type, and a comma-separated
//!    `wake_on` matches any of its types;
//! 4. a parked STEP is recorded as `waiting` (not finished) by `park_step`;
//! 5. `begin_resumed` refuses a SECOND continuation, so two wakes racing each
//!    other produce one run, not two.
//!
//! Note on reads: every assertion goes through `org_tx`, never the shared
//! app pool. Postgres keeps custom GUCs DEFINED after a transaction-local set
//! ends, so a pooled connection carries the previous test's `app.org_id`
//! (`tests/common/mod.rs` documents this), and an unscoped pool read is not
//! a reliable read.
//!
//! Slot discipline: this suite owns fixed-UUID slots 91200–91249 only; lane
//! 13 owns 91040, lane 14 owns 91050–91099 and agent_proposal owns 91100–91149.
//! `insert_run` mints its own id (now_v7) rather than taking a slot, so the
//! slots here are orgs, users and rules only.
//!
//! Non-purpose: the document model and the walk (core-domain unit tests) and
//! the HTTP seam (core-api flow tests). Database selection follows the
//! documented contract in `tests/common/mod.rs`.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::workflow_runs::{self as runs, NewStep};
use core_db::{automations, orgs, users};
use core_domain::workflow_document::DOCUMENT_VERSION;
use core_domain::workflow_runs::{RunStatus, RunTrigger, StepStatus};
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

async fn seed_org(db: &common::TestDb, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Executor Co", &format!("exec-{n}"), "MER")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    tx.commit().await.expect("commit org");
    org
}

async fn seed_user(db: &common::TestDb, org: Uuid, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    users::create_user(
        &mut *tx,
        user,
        org,
        &format!("author{n}@exec.test"),
        "The Author",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

/// One workflow document: entry → stop.
fn document() -> Value {
    json!({
        "version": DOCUMENT_VERSION,
        "nodes": [
            { "id": "a", "node_type": "flow.branch", "params": { "condition": "c1" } },
            { "id": "z", "node_type": "flow.stop", "params": {} }
        ],
        "edges": [ { "from": "a", "to": "z" } ],
        "loop_max_depth": 1
    })
}

/// Inserts one rule at slot `n`. `definition` Some means a workflow
/// document; None is the simple tier the automations consumer runs.
async fn insert_rule(
    conn: &mut PgConnection,
    org: Uuid,
    n: u64,
    author: Option<Uuid>,
    definition: Option<Value>,
) -> Uuid {
    let id = fixed_uuid(n);
    automations::insert(
        &mut *conn,
        id,
        org,
        None,
        core_domain::automations::AutomationSubjectKind::default(),
        &format!("rule-{n}"),
        "ticket.created",
        json!([]),
        json!([]),
    )
    .await
    .expect("rule insert");
    if definition.is_some() {
        // The executor's document, author and revision ride migration 0132's
        // columns; the authoring lanes set them and the executor reads them.
        sqlx::query(
            "UPDATE automation_rules SET definition = $2, revision = 3, created_by = $3 WHERE id = $1",
        )
        .bind(id)
        .bind(definition)
        .bind(author)
        .execute(&mut *conn)
        .await
        .expect("rule update");
    }
    id
}

/// One run started (not parked). Returns the minted run id — `insert_run`
/// generates it.
async fn insert_run(db: &common::TestDb, org: Uuid, author: Uuid) -> Uuid {
    let mut tx = org_tx(&db.app, org).await;
    let run = runs::insert_run(
        &mut tx,
        org,
        &runs::NewRun {
            rule_id: None,
            rule_name: "rule".into(),
            rule_version: 1,
            rule_definition: document(),
            catalogue_version: "1.0.0".into(),
            trigger: RunTrigger::Event,
            trigger_event_id: None,
            trigger_job_id: None,
            trigger_user_id: Some(author),
            resumed_from: None,
            resume_cursor: None,
        },
        RunStatus::Running,
    )
    .await
    .expect("run insert");
    tx.commit().await.expect("run commit");
    run.id
}

async fn park(db: &common::TestDb, org: Uuid, run_id: Uuid, wake_on: &str) {
    let mut tx = org_tx(&db.app, org).await;
    runs::park_run(
        &mut tx,
        org,
        run_id,
        json!({ "index": 1, "executed": ["a"] }),
        Some(chrono::Utc::now() + chrono::Duration::hours(4)),
        Some(wake_on),
    )
    .await
    .expect("park");
    tx.commit().await.expect("park commit");
}

#[tokio::test]
async fn only_workflow_documents_are_matched_by_trigger() {
    let db = test_db().await;
    let org = seed_org(&db, 91200).await;
    let author = seed_user(&db, org, 91201).await;
    let document_rule;
    let simple_rule;
    {
        let mut tx = org_tx(&db.app, org).await;
        // A workflow document (definition set).
        document_rule = insert_rule(&mut tx, org, 91202, Some(author), Some(document())).await;
        // A simple-tier rule (definition NULL) on the SAME trigger.
        simple_rule = insert_rule(&mut tx, org, 91203, None, None).await;
        tx.commit().await.expect("commit rules");
    }

    let (workflows, simples) = {
        let mut tx = org_tx(&db.app, org).await;
        let workflows =
            automations::list_enabled_workflows_by_trigger(&mut *tx, org, "ticket.created")
                .await
                .expect("workflow list");
        let simples = automations::list_enabled_by_trigger(&mut *tx, org, "ticket.created")
            .await
            .expect("simple list");
        tx.commit().await.expect("commit reads");
        (workflows, simples)
    };

    assert_eq!(workflows.len(), 1, "only the graph rule is a workflow");
    let rule = &workflows[0];
    assert_eq!(rule.id, document_rule);
    assert_eq!(
        rule.created_by,
        Some(author),
        "the run executes as its author, never as a system user"
    );
    assert_eq!(
        rule.revision, 3,
        "the revision lane 14's rule_version observes"
    );
    assert_eq!(rule.definition, document());
    assert_eq!(
        simples.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![simple_rule],
        "the simple tier sees only itself, so a document is never run twice"
    );
}

#[tokio::test]
async fn the_waiters_index_finds_a_parked_run_by_any_matched_event() {
    let db = test_db().await;
    let org = seed_org(&db, 91210).await;
    let author = seed_user(&db, org, 91211).await;
    let run = insert_run(&db, org, author).await;
    park(&db, org, run, "quote.accepted,quote.declined").await;

    // Either of the comma-separated types matches.
    for event in ["quote.accepted", "quote.declined"] {
        let mut tx = org_tx(&db.app, org).await;
        let hits = runs::list_waiting_on_event(&mut tx, org, event)
            .await
            .expect("waiters read");
        tx.commit().await.expect("commit read");
        assert_eq!(hits.len(), 1, "{event} must wake the run");
        assert_eq!(hits[0].id, run);
        assert_eq!(hits[0].status, RunStatus::Waiting);
        assert_eq!(
            hits[0].wake_on.as_deref(),
            Some("quote.accepted,quote.declined"),
            "the wake-on list both declared types"
        );
    }
    // An unrelated type leaves it parked.
    {
        let mut tx = org_tx(&db.app, org).await;
        let hits = runs::list_waiting_on_event(&mut tx, org, "quote.sent")
            .await
            .expect("waiters read");
        tx.commit().await.expect("commit read");
        assert!(hits.is_empty(), "an unmatched event leaves the run parked");
    }
    // Another org's waiters are not visible here.
    {
        let other = seed_org(&db, 91213).await;
        let mut tx = org_tx(&db.app, other).await;
        let hits = runs::list_waiting_on_event(&mut tx, other, "quote.accepted")
            .await
            .expect("waiters read");
        tx.commit().await.expect("commit read");
        assert!(hits.is_empty(), "one org's waiter is not another's");
    }
}

#[tokio::test]
async fn a_parked_step_is_waiting_not_finished() {
    let db = test_db().await;
    let org = seed_org(&db, 91220).await;
    let author = seed_user(&db, org, 91221).await;
    let run = insert_run(&db, org, author).await;
    let steps = {
        let mut tx = org_tx(&db.app, org).await;
        let step = runs::insert_step(
            &mut tx,
            org,
            run,
            &NewStep {
                step_index: 0,
                node_id: "flow.waitFor".into(),
                node_label: "Wait for an event".into(),
                attempt: 1,
                iteration: None,
                input: json!({ "eventTypes": ["quote.accepted"] }),
            },
        )
        .await
        .expect("step insert");
        runs::park_step(&mut tx, org, step.id, json!({ "parked": true }))
            .await
            .expect("park step");
        let steps = runs::steps_for_run(&mut tx, org, run).await.expect("steps");
        tx.commit().await.expect("commit");
        steps
    };
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0].status,
        StepStatus::Waiting,
        "a parked step waits; it is not finished"
    );
    assert_eq!(steps[0].output, Some(json!({ "parked": true })));
}

#[tokio::test]
async fn a_second_continuation_of_a_parked_run_is_refused() {
    let db = test_db().await;
    let org = seed_org(&db, 91230).await;
    let author = seed_user(&db, org, 91231).await;
    let run = insert_run(&db, org, author).await;
    park(&db, org, run, "quote.accepted").await;

    let (first, second, parent) = {
        let mut tx = org_tx(&db.app, org).await;
        let first = runs::begin_resumed(&mut tx, org, run, None)
            .await
            .expect("the first wake continues the run");
        // A second wake, in the SAME transaction, must be refused rather
        // than minting a second continuation of one parked row.
        let second = runs::begin_resumed(&mut tx, org, run, None).await;
        let parent = runs::get_run(&mut tx, org, run).await.expect("parent read");
        tx.commit().await.expect("commit");
        (first, second, parent)
    };
    match second {
        Err(core_db::DbError::Conflict(_)) => {}
        other => panic!("a second continuation must be refused, got {other:?}"),
    }
    // The parked row was not rewritten: it still reads `waiting` and still
    // carries the cursor the continuation inherited.
    assert_eq!(
        parent.status,
        RunStatus::Waiting,
        "the parked row is never edited"
    );
    assert_eq!(
        parent.resume_cursor,
        Some(json!({ "index": 1, "executed": ["a"] }))
    );
    // The continuation is a NEW row carrying the pointer back.
    assert_ne!(first.id, run);
    assert_eq!(first.resumed_from, Some(run));
    assert_eq!(first.trigger, RunTrigger::Resume);
    assert_eq!(first.status, RunStatus::Running);
    assert_eq!(
        first.resume_cursor,
        Some(json!({ "index": 1, "executed": ["a"] }))
    );
}
