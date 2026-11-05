//! The workflow run ledger's persistence tests (the private build tracker Wave 43 lane 14,
//! `p43-workflow-run-ledger`, migration 0129).
//!
//! Purpose: prove the five claims the lane makes, and prove them as tests
//! rather than as intentions:
//!
//! 1. a retry does NOT silently double-apply the steps that already
//!    succeeded — the registry's per-node `idempotent` flag is what refuses
//!    it, and the refusal names the nodes;
//! 2. a resumed run is a NEW row with `resumed_from`, and the parked row is
//!    NEVER mutated — including when something tries to mutate it, which the
//!    database refuses;
//! 3. a parked run shows `waiting` with its wake-at and its cursor;
//! 4. the retention window is enforced and configurable, and only TERMINAL
//!    runs are ever swept;
//! 5. RLS keeps one org's runs invisible to another.
//!
//! Slot discipline: this suite owns fixed-UUID slots 91050–91099 only; every
//! test claims a DISJOINT base so parallel suites sharing one migrated
//! database never collide on fixed UUIDs. (Lane 13's suite owns 91040.)
//!
//! Non-purpose: HTTP behavior (core-api flow test) and the pure retry policy
//! (core-domain unit tests). Database selection follows the documented
//! contract in `tests/common/mod.rs`.

mod common;

use common::{db_now, fixed_uuid, org_tx, test_db};
use core_db::{orgs, users};
use core_domain::settings::WORKFLOW_RUN_RETENTION_DAYS;
use core_domain::workflow_runs::{
    self, RetryMode, RunStatus, RunTrigger, StepStatus, WORKFLOW_RESUME_JOB,
};
use serde_json::{json, Value};
use sqlx::PgConnection;
use std::time::Duration;
use uuid::Uuid;

/// Seeds one org (defaults-first roles) at slot `n`.
async fn seed_org(db: &common::TestDb, n: u64, slug: &str) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Ledger Co", slug, "MER")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    tx.commit().await.expect("commit org");
    org
}

/// Seeds one member of `org` at slot `n` (the retry needs a real actor).
async fn seed_user(db: &common::TestDb, org: Uuid, n: u64, email: &str) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    users::create_user(&mut *tx, user, org, email, "Test User", "argon2hash", "UTC")
        .await
        .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

/// Seeds one automation rule at slot `n`. The run ledger records the
/// document snapshot AND a live `rule_id` pointer at it, so a run row needs
/// a real rule row to point at — the FK is the point, since a rule being
/// deleted must take its runs' pointer with it and not their history.
async fn seed_rule(db: &common::TestDb, org: Uuid, n: u64, name: &str) -> Uuid {
    let rule = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    core_db::automations::insert(
        &mut *tx,
        rule,
        org,
        None,
        core_domain::automations::AutomationSubjectKind::Ticket,
        name,
        "ticket.created",
        serde_json::json!([]),
        serde_json::json!([
            {"id": "step_0", "node": "ticket.create", "input": {"boardId": "b", "title": "t"}},
            {"id": "step_1", "node": "email.send", "input": {"to": "a@b.co", "subject": "s", "body": "x"}}
        ]),
    )
    .await
    .expect("rule insert");
    tx.commit().await.expect("commit rule");
    rule
}

/// A minimal workflow document, as the executor would snapshot it.
fn document() -> Value {
    json!({
        "trigger": {"type": "event", "event": "ticket.created"},
        "conditions": [],
        "actions": [
            {"id": "step_0", "node": "ticket.create", "input": {"boardId": "b", "title": "t"}},
            {"id": "step_1", "node": "email.send", "input": {"to": "a@b.co", "subject": "s", "body": "x"}}
        ]
    })
}

fn new_run(rule_id: Option<Uuid>, trigger: RunTrigger) -> core_db::workflow_runs::NewRun {
    core_db::workflow_runs::NewRun {
        rule_id,
        rule_name: "Escalate overdue tickets".to_string(),
        rule_version: 1,
        rule_definition: document(),
        catalogue_version: core_domain::node_registry::CATALOGUE_VERSION.to_string(),
        trigger,
        trigger_event_id: None,
        trigger_job_id: None,
        trigger_user_id: None,
        resumed_from: None,
        resume_cursor: None,
    }
}

/// Writes a step row and returns its id.
async fn add_step(
    conn: &mut PgConnection,
    org: Uuid,
    run: Uuid,
    index: i32,
    node_id: &str,
    input: Value,
) -> Uuid {
    core_db::workflow_runs::insert_step(
        &mut *conn,
        org,
        run,
        &core_db::workflow_runs::NewStep {
            step_index: index,
            node_id: node_id.to_string(),
            node_label: node_id.to_string(),
            attempt: 1,
            iteration: None,
            input,
        },
    )
    .await
    .expect("step insert")
    .id
}

// ---------------------------------------------------------------------------
// 1. retry must not double-apply
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_retry_creates_a_new_row_and_refuses_to_repeat_a_non_idempotent_step() {
    // Slots 91050 (org), 91051 (rule), 91052 (actor).
    let db = test_db().await;
    let org = seed_org(&db, 91050, "ledger-91050").await;
    let rule = seed_rule(&db, org, 91051, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91052, "op@ledger50.test").await;
    let mut tx = org_tx(&db.app, org).await;

    // A run that created a ticket, then tried to email about it, and the
    // send broke. `ticket.create` is idempotent=false (a second run is a
    // second ticket) and `email.send` is too (a second run is a second
    // email) — the registry owns both answers.
    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run insert");
    let created = add_step(
        &mut tx,
        org,
        run.id,
        0,
        "ticket.create",
        json!({"title": "t"}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        created,
        StepStatus::Succeeded,
        Some(json!({"ticketId": "abc"})),
        None,
        None,
    )
    .await
    .expect("finish created");
    let send = add_step(
        &mut tx,
        org,
        run.id,
        1,
        "email.send",
        json!({"to": "a@b.co", "subject": "s", "body": "x"}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        send,
        StepStatus::Failed,
        None,
        Some("smtp_unreachable"),
        Some("connection refused"),
    )
    .await
    .expect("finish send");
    core_db::workflow_runs::finish_run(
        &mut tx,
        org,
        run.id,
        RunStatus::Failed,
        Some("step_failed"),
        Some("email.send failed: connection refused"),
    )
    .await
    .expect("finish run");

    // A from-scratch re-run would repeat BOTH non-idempotent steps: the
    // creation and the send. It is refused, and the refusal names them.
    let refusal = core_db::workflow_runs::retry_run(
        &mut tx,
        org,
        Uuid::now_v7(),
        run.id,
        actor,
        None,
        RetryMode::FromStart,
    )
    .await
    .expect_err("a from-start retry must be refused");
    let message = refusal.to_string();
    assert!(
        message.contains("ticket.create"),
        "named the creation: {message}"
    );
    assert!(message.contains("email.send"), "named the send: {message}");

    // Resuming from the failure re-runs ONLY the failed send, which is
    // still non-idempotent — a failed send may already have sent. So the
    // honest answer is still a refusal, not a silent second email.
    let resumed = core_db::workflow_runs::retry_run(
        &mut tx,
        org,
        Uuid::now_v7(),
        run.id,
        actor,
        None,
        RetryMode::Resume,
    )
    .await
    .expect_err("repeating a failed send is not silent-safe either");
    assert!(
        resumed.to_string().contains("email.send"),
        "the refusal names the node that may have half-applied"
    );

    // NEITHER refusal wrote a run: a retry that would double-apply is
    // never recorded as having happened.
    let runs = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    assert_eq!(runs.len(), 1, "only the original run exists");

    // The steps that already succeeded were NOT re-run: their rows are
    // untouched, and a second insert of the same attempt would collide.
    let steps = core_db::workflow_runs::steps_for_run(&mut tx, org, run.id)
        .await
        .expect("steps");
    assert_eq!(
        steps.len(),
        2,
        "the ledger still holds exactly two attempts"
    );
    assert_eq!(steps[0].status, StepStatus::Succeeded);
    assert_eq!(steps[1].status, StepStatus::Failed);
}

#[tokio::test]
async fn a_safe_retry_records_a_new_row_that_points_back_at_the_failed_one() {
    // Slots 91053 (org), 91054 (rule), 91055 (actor).
    // A workflow whose steps are all safe to repeat: a state setter and a
    // tag write, both idempotent per the registry.
    let db = test_db().await;
    let org = seed_org(&db, 91053, "ledger-91053").await;
    let rule = seed_rule(&db, org, 91054, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91055, "op@ledger53.test").await;
    let mut tx = org_tx(&db.app, org).await;

    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let first = add_step(
        &mut tx,
        org,
        run.id,
        0,
        "ticket.update",
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        first,
        StepStatus::Succeeded,
        Some(json!({"changed": ["title"]})),
        None,
        None,
    )
    .await
    .expect("finish first");
    let second = add_step(
        &mut tx,
        org,
        run.id,
        1,
        "ticket.update",
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        second,
        StepStatus::Failed,
        None,
        Some("validation_failed"),
        Some("field 'due_at' rejected"),
    )
    .await
    .expect("finish second");
    core_db::workflow_runs::finish_run(
        &mut tx,
        org,
        run.id,
        RunStatus::Failed,
        Some("step_failed"),
        None,
    )
    .await
    .expect("finish run");

    let retry = core_db::workflow_runs::retry_run(
        &mut tx,
        org,
        Uuid::now_v7(),
        run.id,
        actor,
        None,
        RetryMode::Resume,
    )
    .await
    .expect("a retry of a safe-to-repeat step is allowed");

    // It is a NEW row, pointing back, and it records that it was a retry.
    assert_ne!(retry.id, run.id, "a retry is not an edit of the old run");
    assert_eq!(retry.retry_of, Some(run.id), "the retry names its origin");
    assert_eq!(retry.trigger, RunTrigger::OperatorRetry);
    assert_eq!(retry.trigger_user_id, Some(actor));
    // It re-runs from the failed step, and says so in the cursor.
    assert_eq!(retry.status, RunStatus::Waiting);
    assert_eq!(retry.wake_on.as_deref(), Some("queue"));
    assert_eq!(
        retry
            .resume_cursor
            .as_ref()
            .and_then(|c| c.get("start_at_step")),
        Some(&json!(1)),
        "the retry's cursor names where it starts"
    );
    // The document it re-runs is the SNAPSHOT the failed run carried, not a
    // re-read that could have changed.
    assert_eq!(retry.rule_definition, run.rule_definition);
    assert_eq!(retry.rule_id, run.rule_id);
    assert_eq!(retry.catalogue_version, run.catalogue_version);

    // The failed run is untouched: it still says failed, with its error.
    let after = core_db::workflow_runs::get_run(&mut tx, org, run.id)
        .await
        .expect("original run");
    assert_eq!(
        after.status,
        RunStatus::Failed,
        "the old run keeps its story"
    );
    assert_eq!(after.error_code.as_deref(), Some("step_failed"));
    assert_eq!(after.retry_of, None);
    assert_eq!(after.resumed_from, None);
    assert_eq!(
        after.id, run.id,
        "the failed row is the same row, not a rewritten one"
    );

    // The listing DERIVES "retried" from the child's existence — no stored
    // flag, so the failed row was never edited to say it was retried.
    let page = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    assert_eq!(page.len(), 2);
    let listed_original = page
        .iter()
        .find(|row| row.run.id == run.id)
        .expect("the original is listed");
    assert!(listed_original.retried, "the original reads as retried");
    assert!(!listed_original.superseded);
    let listed_retry = page
        .iter()
        .find(|row| row.run.id == retry.id)
        .expect("the retry is listed");
    assert!(!listed_retry.retried);
    assert!(!listed_retry.superseded);
}

#[tokio::test]
async fn an_in_run_retry_of_a_step_appends_an_attempt_instead_of_overwriting_it() {
    // Slots 91056 (org), 91057 (rule).
    let db = test_db().await;
    let org = seed_org(&db, 91056, "ledger-91056").await;
    let rule = seed_rule(&db, org, 91057, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;

    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let attempt_one = add_step(
        &mut tx,
        org,
        run.id,
        0,
        "ticket.update",
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        attempt_one,
        StepStatus::Failed,
        None,
        Some("transient"),
        Some("timeout"),
    )
    .await
    .expect("finish attempt 1");
    // The executor's own step retry is a NEW row at the same position with
    // attempt 2 — the failed attempt stays visible.
    let attempt_two = core_db::workflow_runs::insert_step(&mut tx,
        org,
        run.id,
        &core_db::workflow_runs::NewStep {
            step_index: 0,
            node_id: "ticket.update".to_string(),
            node_label: "ticket.update".to_string(),
            attempt: 2,
            iteration: None,
            input: json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
        },
    )
    .await
    .expect("attempt 2 insert");

    let steps = core_db::workflow_runs::steps_for_run(&mut tx, org, run.id)
        .await
        .expect("steps");
    assert_eq!(steps.len(), 2, "both attempts are on the record");
    assert_eq!(steps[0].attempt, 1);
    assert_eq!(steps[0].status, StepStatus::Failed);
    assert_eq!(steps[1].id, attempt_two.id);
    assert_eq!(steps[1].attempt, 2);
    assert_eq!(steps[1].status, StepStatus::Running);

    // A step cannot be finished twice: the operator is looking at exactly
    // that row, and the first answer wins.
    let double = core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        attempt_one,
        StepStatus::Succeeded,
        None,
        None,
        None,
    )
    .await
    .expect_err("a finished step cannot be finished again");
    assert!(matches!(double, core_db::DbError::NotFound));

    // The input a step received is immutable: re-writing it to match a later
    // retry would be the same fiction the run guard refuses.
    let rewrite = sqlx::query("UPDATE workflow_run_steps SET input = $2 WHERE id = $1")
        .bind(attempt_one)
        .bind(json!({"operation": "ticket.create"}))
        .execute(&mut *tx)
        .await
        .expect_err("the trigger must refuse an input rewrite");
    let message = rewrite.to_string();
    assert!(message.contains("audit ledger"), "typed refusal: {message}");
}

// ---------------------------------------------------------------------------
// 2. a resumed run is a NEW row, never a mutation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_parked_run_shows_waiting_with_its_wake_at_and_its_cursor() {
    // Slots 91058 (org), 91059 (rule).
    let db = test_db().await;
    let org = seed_org(&db, 91058, "ledger-91058").await;
    let rule = seed_rule(&db, org, 91059, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;

    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");

    let wake_at = db_now() + Duration::from_secs(3600);
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        run.id,
        json!({"node": "flow.delay", "until": wake_at.to_rfc3339()}),
        Some(wake_at),
        Some("flow.delay"),
    )
    .await
    .expect("park");

    let parked = core_db::workflow_runs::get_run(&mut tx, org, run.id)
        .await
        .expect("parked run");
    assert_eq!(parked.status, RunStatus::Waiting, "parking is first-class");
    assert_eq!(parked.wake_at, Some(wake_at), "the wake-at is on the row");
    assert_eq!(parked.wake_on.as_deref(), Some("flow.delay"));
    assert_eq!(
        parked.resume_cursor.as_ref().and_then(|c| c.get("node")),
        Some(&json!("flow.delay")),
        "the row carries where it resumes from"
    );
    assert!(
        parked.finished_at.is_none(),
        "a parked run has not finished"
    );

    // An event-driven park carries NO wake-at but still names what it waits
    // on: `flow.waitFor` wakes on an envelope, not on a clock.
    let waiting = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        waiting.id,
        json!({"node": "flow.waitFor", "event": "ticket.closed"}),
        None,
        Some("ticket.closed"),
    )
    .await
    .expect("park on an event");
    let event_parked = core_db::workflow_runs::get_run(&mut tx, org, waiting.id)
        .await
        .expect("event-parked run");
    assert_eq!(event_parked.status, RunStatus::Waiting);
    assert_eq!(event_parked.wake_at, None);
    assert_eq!(event_parked.wake_on.as_deref(), Some("ticket.closed"));

    // A finished run is never parked, and the guard is a state check rather
    // than a hope.
    core_db::workflow_runs::finish_run(&mut tx, org, run.id, RunStatus::Succeeded, None, None)
        .await
        .expect("finish");
    let re_park = core_db::workflow_runs::park_run(
        &mut tx,
        org,
        run.id,
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {}}),
        Some(wake_at),
        Some("flow.delay"),
    )
    .await
    .expect_err("a succeeded run cannot be parked");
    assert!(matches!(re_park, core_db::DbError::NotFound));
}

#[tokio::test]
async fn a_resume_is_a_new_row_and_the_parked_row_is_never_edited() {
    // Slots 91060 (org), 91061 (rule), 91062 (wake job).
    let db = test_db().await;
    let org = seed_org(&db, 91060, "ledger-91060").await;
    let rule = seed_rule(&db, org, 91061, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;

    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let wake_at = db_now() + Duration::from_secs(60);
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        run.id,
        json!({"node": "flow.delay", "resumed": false}),
        Some(wake_at),
        Some("flow.delay"),
    )
    .await
    .expect("park");

    // The wake job rides the EXISTING queue, due at the wake-at.
    let wake_job = core_db::queue::enqueue_after(
        &mut *tx,
        &core_db::queue::NewJob {
            org_id: org,
            kind: WORKFLOW_RESUME_JOB.to_string(),
            payload: json!({"run_id": run.id, "resume": true}),
            max_attempts: 5,
        },
        wake_at,
    )
    .await
    .expect("enqueue the wake");

    let continued = core_db::workflow_runs::begin_resumed(&mut tx, org, run.id, Some(wake_job))
        .await
        .expect("resume");

    // The continuation is a NEW row that points back.
    assert_ne!(continued.id, run.id);
    assert_eq!(continued.resumed_from, Some(run.id));
    assert_eq!(continued.trigger, RunTrigger::Resume);
    assert_eq!(continued.trigger_job_id, Some(wake_job));
    assert_eq!(continued.status, RunStatus::Running);
    // It carries the state it continues from.
    assert_eq!(
        continued.resume_cursor.as_ref().and_then(|c| c.get("node")),
        Some(&json!("flow.delay"))
    );
    // It inherited the document snapshot, so the continuation is the same
    // document without re-reading a table that might have changed.
    assert_eq!(continued.rule_definition, run.rule_definition);
    assert_eq!(continued.rule_id, run.rule_id);

    // The parked row is UNTOUCHED: same id, same status, same cursor, same
    // wake-at, same updated_at. Not "moved on" — parked, exactly as it was.
    let parked = core_db::workflow_runs::get_run(&mut tx, org, run.id)
        .await
        .expect("parked row");
    assert_eq!(parked.id, run.id, "the parked row is the same row");
    assert_eq!(parked.status, RunStatus::Waiting, "and it is still waiting");
    assert_eq!(parked.wake_at, Some(wake_at));
    assert_eq!(
        parked.resume_cursor.as_ref().and_then(|c| c.get("resumed")),
        Some(&json!(false)),
        "its cursor was not rewritten to pretend it continued"
    );
    assert_eq!(parked.updated_at, run.updated_at, "it was not written to");
    assert_eq!(parked.resumed_from, None, "the parked row claims no parent");
    assert_eq!(parked.retry_of, None);

    // The listing shows the parked row as superseded — DERIVED from the
    // child's existence, never stored on it.
    let page = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    let parked_row = page
        .iter()
        .find(|row| row.run.id == run.id)
        .expect("the parked row is listed");
    assert!(parked_row.superseded, "the chain is visible at read time");
    assert!(!parked_row.retried);
    // Commit before the refusal attempts below: each needs its own
    // transaction, because a failed statement aborts the one it runs in and
    // the next refusal would read "transaction is aborted" instead of
    // naming its own constraint.
    tx.commit().await.expect("commit the chain");

    // And the DATABASE refuses the mutation even if a writer tries. Two
    // refusals, each in its OWN transaction: a failed statement aborts the
    // transaction it runs in, so sharing one would make the second read
    // "transaction is aborted" instead of naming its own constraint.
    //
    // Status is lifecycle and may move, so the first refusal is aimed at
    // what is NOT lifecycle — changing which document the row claims to
    // have run. Migration 0129's trigger is the enforcement, not our
    // remembering.
    let mut tx = org_tx(&db.app, org).await;
    let rewrite = sqlx::query(
        "UPDATE workflow_runs SET rule_name = 'A tidier name', status = 'succeeded' WHERE id = $1",
    )
    .bind(run.id)
    .execute(&mut *tx)
    .await
    .expect_err("the row may not be edited into a different story");
    let message = rewrite.to_string();
    assert!(
        message.contains("audit ledger"),
        "the trigger refuses an identity rewrite of the parked row: {message}"
    );

    // The shape CHECK is what guards the recorded park coordinates from a
    // status move that forgets them: succeeding the row while it still
    // holds a cursor and a wake-at never reaches the trigger.
    drop(tx);
    let mut tx = org_tx(&db.app, org).await;
    let shape = sqlx::query("UPDATE workflow_runs SET status = 'succeeded' WHERE id = $1")
        .bind(run.id)
        .execute(&mut *tx)
        .await
        .expect_err("a parked row cannot become terminal while still parked");
    assert!(
        shape.to_string().contains("workflow_runs_status_shape"),
        "the shape CHECK refused it: {shape}"
    );
    // Nothing after this point runs: a failed statement aborts the
    // transaction, so the parked row is unchanged by construction.
}

#[tokio::test]
async fn a_resume_of_a_run_that_is_not_parked_is_refused() {
    // Slots 91063 (org), 91064 (rule).
    let db = test_db().await;
    let org = seed_org(&db, 91063, "ledger-91063").await;
    let rule = seed_rule(&db, org, 91064, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;

    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let refusal = core_db::workflow_runs::begin_resumed(&mut tx, org, run.id, None)
        .await
        .expect_err("a running run has nothing to resume");
    assert!(matches!(refusal, core_db::DbError::Conflict(_)));
}

// ---------------------------------------------------------------------------
// 3. the reads: cursor paging, the same shape as the delivery ledger
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_run_ledger_pages_on_a_keyset_cursor_like_the_delivery_ledger() {
    // Slots 91065 (org), 91066 (rule).
    let db = test_db().await;
    let org = seed_org(&db, 91065, "ledger-91065").await;
    let rule = seed_rule(&db, org, 91066, "Escalate overdue tickets").await;

    // Five runs, each in its OWN transaction: `created_at` is the
    // transaction's clock, so paging on the timestamp half of the cursor
    // is only real if the rows carry distinct ones.
    let mut ids = Vec::new();
    for _ in 0..5 {
        let mut tx = org_tx(&db.app, org).await;
        let run = core_db::workflow_runs::insert_run(
            &mut tx,
            org,
            &new_run(Some(rule), RunTrigger::Event),
            RunStatus::Running,
        )
        .await
        .expect("run");
        ids.push(run.id);
        tx.commit().await.expect("commit");
        tokio::time::sleep(Duration::from_millis(60)).await;
    }

    // Page one, two per page, newest first.
    let mut tx = org_tx(&db.app, org).await;
    let page_one = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        2,
    )
    .await
    .expect("page one");
    assert_eq!(page_one.len(), 2);
    assert_eq!(page_one[0].run.id, ids[4], "newest first");
    assert_eq!(page_one[1].run.id, ids[3]);
    assert!(
        page_one[0].run.created_at > page_one[1].run.created_at,
        "the rows carry distinct timestamps, so the cursor's time half matters"
    );

    // The cursor is the last row's (created_at, id) — the webhook delivery
    // ledger's exact shape, so the two logs page alike.
    let cursor = (page_one[1].run.created_at, page_one[1].run.id);
    let page_two = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter {
            cursor: Some(cursor),
            ..Default::default()
        },
        2,
    )
    .await
    .expect("page two");
    assert_eq!(page_two.len(), 2);
    assert_eq!(page_two[0].run.id, ids[2]);
    assert_eq!(page_two[1].run.id, ids[1]);

    // Nothing repeats, and the last page is short.
    let last_cursor = (page_two[1].run.created_at, page_two[1].run.id);
    let page_three = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter {
            cursor: Some(last_cursor),
            ..Default::default()
        },
        2,
    )
    .await
    .expect("page three");
    assert_eq!(page_three.len(), 1);
    assert_eq!(page_three[0].run.id, ids[0]);

    // A status filter narrows the same listing.
    core_db::workflow_runs::finish_run(&mut tx, org, ids[0], RunStatus::Succeeded, None, None)
        .await
        .expect("finish");
    let failed_only = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter {
            status: Some(RunStatus::Failed),
            ..Default::default()
        },
        100,
    )
    .await
    .expect("filtered");
    assert!(failed_only.is_empty(), "none of them failed");
    let succeeded = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter {
            status: Some(RunStatus::Succeeded),
            ..Default::default()
        },
        100,
    )
    .await
    .expect("filtered");
    assert_eq!(succeeded.len(), 1);
}

// ---------------------------------------------------------------------------
// 4. retention is enforced and configurable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_retention_window_is_configurable_and_only_ever_sweeps_finished_runs() {
    // Slots 91067 (org), 91068 (rule), 91069 (actor).
    let db = test_db().await;
    let org = seed_org(&db, 91067, "ledger-91067").await;
    let rule = seed_rule(&db, org, 91068, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91069, "op@ledger67.test").await;

    // An untouched org gets the declared default.
    let mut tx = org_tx(&db.app, org).await;
    assert_eq!(
        core_db::workflow_runs::retention_days(&mut tx, org)
            .await
            .expect("default window"),
        workflow_runs::RETENTION_DEFAULT_DAYS
    );

    // The org narrows its window to the floor through the SAME typed
    // settings path every other org setting rides.
    core_db::settings::upsert_org_setting(
        &mut tx,
        org,
        WORKFLOW_RUN_RETENTION_DAYS,
        &json!(workflow_runs::RETENTION_MIN_DAYS),
        actor,
    )
    .await
    .expect("set the window");
    assert_eq!(
        core_db::workflow_runs::retention_days(&mut tx, org)
            .await
            .expect("configured window"),
        workflow_runs::RETENTION_MIN_DAYS,
        "the setting is read back as the window"
    );

    // Three runs: an old finished one, a recent finished one, and one still
    // PARKED. Only the old finished one is ever swept.
    let old = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("old");
    let recent = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("recent");
    let parked = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("parked");
    let running = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("running");

    // The old run finished long ago; its STEP goes with it.
    let old_step = add_step(&mut tx, org, old.id, 0, "ticket.update", json!({})).await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        old_step,
        StepStatus::Succeeded,
        None,
        None,
        None,
    )
    .await
    .expect("finish old step");
    for (run, status) in [
        (old.clone(), RunStatus::Succeeded),
        (recent.clone(), RunStatus::Failed),
    ] {
        core_db::workflow_runs::finish_run(&mut tx, org, run.id, status, None, None)
            .await
            .expect("finish");
    }
    // Age the old run past a week, and park the parked one in the past: it
    // is owed a wake, not a delete.
    sqlx::query("UPDATE workflow_runs SET finished_at = now() - interval '30 days' WHERE id = $1")
        .bind(old.id)
        .execute(&mut *tx)
        .await
        .expect("age the old run");
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        parked.id,
        json!({"node": "flow.delay"}),
        Some(chrono::Utc::now() - Duration::from_secs(3600)),
        Some("flow.delay"),
    )
    .await
    .expect("park in the past");
    let recent_step = add_step(&mut tx, org, recent.id, 0, "ticket.update", json!({})).await;
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        recent_step,
        StepStatus::Failed,
        None,
        Some("x"),
        None,
    )
    .await
    .expect("finish recent step");

    // The sweep, at the org's own window.
    let purged =
        core_db::workflow_runs::purge_expired(&mut tx, org, workflow_runs::RETENTION_MIN_DAYS)
            .await
            .expect("purge");
    assert_eq!(purged, 1, "one run is past the window");

    let remaining = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    let remaining_ids: Vec<Uuid> = remaining.iter().map(|r| r.run.id).collect();
    assert!(!remaining_ids.contains(&old.id), "the old run is gone");
    assert!(
        remaining_ids.contains(&recent.id),
        "the recent run is still there"
    );
    assert!(
        remaining_ids.contains(&parked.id),
        "a parked run is owed a wake, never swept"
    );
    assert!(
        remaining_ids.contains(&running.id),
        "a running run is work, never swept"
    );
    // Its steps cascaded with it.
    let old_steps = core_db::workflow_runs::steps_for_run(&mut tx, org, old.id)
        .await
        .expect("old steps");
    assert!(old_steps.is_empty(), "the swept run's steps went with it");
    // ...and the other run kept its own.
    assert_eq!(
        core_db::workflow_runs::steps_for_run(&mut tx, org, recent.id)
            .await
            .expect("recent steps")
            .len(),
        1
    );
}

#[tokio::test]
async fn the_cross_org_purge_reads_each_orgs_own_window() {
    // Slots 91070 (org A), 91071 (org B), 91072 (rule), 91073 (org B's actor).
    let db = test_db().await;
    let org_a = seed_org(&db, 91070, "ledger-91070").await;
    let org_b = seed_org(&db, 91071, "ledger-91071").await;
    let rule = seed_rule(&db, org_a, 91072, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org_b, 91073, "op@ledger71.test").await;

    // Org A keeps the default 30 days; org B narrowed to 7. Both have a run
    // that finished 10 days ago: A keeps it, B loses it.
    let mut tx_a = org_tx(&db.app, org_a).await;
    let a_run = core_db::workflow_runs::insert_run(
        &mut tx_a,
        org_a,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("a run");
    core_db::workflow_runs::finish_run(
        &mut tx_a,
        org_a,
        a_run.id,
        RunStatus::Succeeded,
        None,
        None,
    )
    .await
    .expect("a finish");
    sqlx::query("UPDATE workflow_runs SET finished_at = now() - interval '10 days' WHERE id = $1")
        .bind(a_run.id)
        .execute(&mut *tx_a)
        .await
        .expect("age a");
    tx_a.commit().await.expect("commit a");

    let mut tx_b = org_tx(&db.app, org_b).await;
    core_db::settings::upsert_org_setting(
        &mut tx_b,
        org_b,
        WORKFLOW_RUN_RETENTION_DAYS,
        &json!(7),
        actor,
    )
    .await
    .expect("b window");
    let b_run = core_db::workflow_runs::insert_run(
        &mut tx_b,
        org_b,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("b run");
    core_db::workflow_runs::finish_run(
        &mut tx_b,
        org_b,
        b_run.id,
        RunStatus::Succeeded,
        None,
        None,
    )
    .await
    .expect("b finish");
    sqlx::query("UPDATE workflow_runs SET finished_at = now() - interval '10 days' WHERE id = $1")
        .bind(b_run.id)
        .execute(&mut *tx_b)
        .await
        .expect("age b");
    tx_b.commit().await.expect("commit b");

    // The cross-org purge (what the worker's sweep calls) resolves each
    // org's window inside the one statement.
    let purged = core_db::workflow_runs::purge_expired_everywhere(&db.admin)
        .await
        .expect("purge everywhere");
    assert_eq!(purged, 1, "only org B's ten-day-old run is past its window");

    let mut tx = org_tx(&db.app, org_a).await;
    assert!(
        core_db::workflow_runs::get_run(&mut tx, org_a, a_run.id)
            .await
            .is_ok(),
        "org A's ten-day-old run survives its 30-day window"
    );
    let mut tx = org_tx(&db.app, org_b).await;
    assert!(
        core_db::workflow_runs::get_run(&mut tx, org_b, b_run.id)
            .await
            .is_err(),
        "org B's ten-day-old run is past its 7-day window"
    );
}

// ---------------------------------------------------------------------------
// 5. RLS keeps one org's runs invisible to another
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rls_keeps_one_orgs_runs_invisible_to_another() {
    // Slots 91074 (org A), 91075 (org B), 91076 (rule).
    let db = test_db().await;
    let org_a = seed_org(&db, 91074, "ledger-91074").await;
    let org_b = seed_org(&db, 91075, "ledger-91075").await;
    let rule = seed_rule(&db, org_a, 91076, "Escalate overdue tickets").await;

    let mut tx = org_tx(&db.app, org_a).await;
    let a_run = core_db::workflow_runs::insert_run(
        &mut tx,
        org_a,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("a run");
    add_step(&mut tx, org_a, a_run.id, 0, "ticket.update", json!({})).await;
    tx.commit().await.expect("commit a");

    let mut tx = org_tx(&db.app, org_b).await;
    let b_run = core_db::workflow_runs::insert_run(
        &mut tx,
        org_b,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("b run");
    tx.commit().await.expect("commit b");

    // Org A sees only its own run.
    let mut tx = org_tx(&db.app, org_a).await;
    let visible = core_db::workflow_runs::list_runs(
        &mut tx,
        org_a,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].run.id, a_run.id);

    // A direct read of the other org's run is indistinguishable from a run
    // that never existed — the RLS security property, and the read posture
    // that keeps it honest.
    let cross = core_db::workflow_runs::get_run(&mut tx, org_a, b_run.id).await;
    assert!(
        matches!(cross, Err(core_db::DbError::NotFound)),
        "another org's run is not visible: {cross:?}"
    );
    // Its steps are not visible either.
    let cross_steps = core_db::workflow_runs::steps_for_run(&mut tx, org_a, b_run.id)
        .await
        .expect("the empty read succeeds");
    assert!(cross_steps.is_empty(), "and its trace is not leaked");
    // The retry of another org's run is refused by the same posture.
    let actor = seed_user(&db, org_a, 91077, "op@ledger74.test").await;
    let cross_retry = core_db::workflow_runs::retry_run(
        &mut tx,
        org_a,
        Uuid::now_v7(),
        b_run.id,
        actor,
        None,
        RetryMode::Resume,
    )
    .await;
    assert!(
        matches!(cross_retry, Err(core_db::DbError::NotFound)),
        "an operator cannot retry another org's run: {cross_retry:?}"
    );
}

// ---------------------------------------------------------------------------
// the status vocabulary is enforced by the DATABASE, not only in Rust
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_waiting_run_without_a_resume_cursor_cannot_land() {
    // Slots 91078 (org), 91079 (rule).
    //
    // A parked run always says where it resumes from and what it waits for.
    // The shape CHECK spells the whole rule, so a half-shaped row cannot
    // land from ANY writer — hand-written SQL included. Each refusal runs
    // in its OWN transaction below, because a failed statement aborts the
    // one that follows it and the next refusal would read as "transaction
    // aborted" instead of naming its constraint.
    let db = test_db().await;
    let org = seed_org(&db, 91078, "ledger-91078").await;
    let _rule = seed_rule(&db, org, 91079, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;
    let malformed = sqlx::query(
        "INSERT INTO workflow_runs
           (id, org_id, rule_name, rule_version, rule_definition, catalogue_version,
            status, trigger_kind)
         VALUES ($1, $2, 'x', 1, $3, '1.0.0', 'waiting', 'event')",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(json!({"trigger": {}}))
    .execute(&mut *tx)
    .await
    .expect_err("a waiting run must carry a resume cursor");
    assert!(
        malformed.to_string().contains("workflow_runs_status_shape"),
        "the shape CHECK refused it: {malformed}"
    );
    drop(tx);

    // ...and neither can a status the code does not know about. Build drift
    // is a database error here rather than a row the UI has to interpret.
    let mut tx = org_tx(&db.app, org).await;
    let unknown = sqlx::query(
        "INSERT INTO workflow_runs
           (id, org_id, rule_name, rule_version, rule_definition, catalogue_version,
            status, trigger_kind)
         VALUES ($1, $2, 'x', 1, $3, '1.0.0', 'parked', 'event')",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(json!({"trigger": {}}))
    .execute(&mut *tx)
    .await
    .expect_err("an unknown status is build drift");
    assert!(
        unknown.to_string().contains("workflow_runs_status_check"),
        "the vocabulary CHECK refused it: {unknown}"
    );
    drop(tx);

    // The chain shape too, exercised on an INSERT rather than an UPDATE:
    // the trigger already makes the chain columns immutable, so updating
    // them is refused before the CHECK is ever consulted. A row that tries
    // to claim both a resume and a retry at write time is what the CHECK
    // catches — which is why the row points its own id at itself.
    let mut tx = org_tx(&db.app, org).await;
    let both = sqlx::query(
        "INSERT INTO workflow_runs
           (id, org_id, rule_name, rule_version, rule_definition, catalogue_version,
            status, trigger_kind, retry_of, resumed_from, resume_cursor, wake_on)
         VALUES ($1, $2, 'x', 1, $3, '1.0.0', 'waiting', 'resume', $1, $1, $4, 'queue')",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(json!({"trigger": {}}))
    .bind(json!({}))
    .execute(&mut *tx)
    .await
    .expect_err("a row cannot be both a resume and a retry");
    assert!(
        both.to_string().contains("workflow_runs_trigger_shape"),
        "the chain CHECK refused it: {both}"
    );
}

#[tokio::test]
async fn a_finished_run_carries_no_park_coordinates() {
    // Slots 91080 (org), 91081 (rule).
    //
    // `finish_run` consumes the park coordinates, so a finished run reads as
    // terminal rather than as one still owed a wake. The CHECK enforces the
    // same thing from the database side.
    let db = test_db().await;
    let org = seed_org(&db, 91080, "ledger-91080").await;
    let rule = seed_rule(&db, org, 91081, "Escalate overdue tickets").await;
    let mut tx = org_tx(&db.app, org).await;

    let parked = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        parked.id,
        json!({"node": "flow.delay"}),
        Some(chrono::Utc::now()),
        Some("flow.delay"),
    )
    .await
    .expect("park");
    core_db::workflow_runs::finish_run(&mut tx, org, parked.id, RunStatus::Failed, None, None)
        .await
        .expect("a parked run can still fail");

    let finished = core_db::workflow_runs::get_run(&mut tx, org, parked.id)
        .await
        .expect("run");
    assert_eq!(finished.status, RunStatus::Failed);
    assert!(
        finished.resume_cursor.is_none(),
        "the cursor is consumed with the park"
    );
    assert!(finished.wake_at.is_none());
    assert!(finished.wake_on.is_none());
    assert!(finished.finished_at.is_some());
}

// ---------------------------------------------------------------------------
// 6. the purge never aborts on the ledger's declared references (0169)
// ---------------------------------------------------------------------------

/// Ages one run's finish time, which is a lifecycle column and stays
/// writable: only the provenance is immutable.
async fn age_finished_at(conn: &mut PgConnection, run: Uuid, days: i64) {
    sqlx::query(
        "UPDATE workflow_runs
            SET finished_at = now() - make_interval(days => $2::int)
          WHERE id = $1",
    )
    .bind(run)
    .bind(days as i32)
    .execute(&mut *conn)
    .await
    .expect("age the finish time");
}

/// One failed run with a succeeded first step and a failed second one, so an
/// operator retry of it is allowed by the double-apply gate.
async fn a_failed_run(conn: &mut PgConnection, org: Uuid, rule: Uuid) -> Uuid {
    let run = core_db::workflow_runs::insert_run(
        conn,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let first = add_step(
        conn,
        org,
        run.id,
        0,
        "ticket.update",
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        conn,
        org,
        first,
        StepStatus::Succeeded,
        Some(json!({"changed": ["title"]})),
        None,
        None,
    )
    .await
    .expect("finish first");
    let second = add_step(
        conn,
        org,
        run.id,
        1,
        "ticket.update",
        json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
    )
    .await;
    core_db::workflow_runs::finish_step(
        conn,
        org,
        second,
        StepStatus::Failed,
        None,
        Some("validation_failed"),
        Some("field 'due_at' rejected"),
    )
    .await
    .expect("finish second");
    core_db::workflow_runs::finish_run(
        conn,
        org,
        run.id,
        RunStatus::Failed,
        Some("step_failed"),
        None,
    )
    .await
    .expect("finish run");
    run.id
}

#[tokio::test]
async fn purging_a_retried_run_releases_the_retry_instead_of_aborting() {
    // Slots 91082 (org), 91083 (rule), 91084 (actor).
    //
    // The record this test closes: the parent is past the window and the
    // retry is inside it, so the delete fires the FK's own
    // `UPDATE ... SET retry_of = NULL` on the SURVIVING row. The guard used
    // to refuse that write, and the sweep aborted with
    // `workflow_runs is an audit ledger: identity and provenance columns are
    // immutable` — retrying a workflow poisoned the purge for its org.
    //
    // The sweep here is the WORKER's function (`purge_expired_everywhere`,
    // cross-org on the admin pool), so the assertions are about the rows this
    // test owns rather than about a global count other suites share.
    let db = test_db().await;
    let org = seed_org(&db, 91082, "ledger-91082").await;
    let rule = seed_rule(&db, org, 91083, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91084, "op@ledger82.test").await;

    let (parent, retry) = {
        let mut tx = org_tx(&db.app, org).await;
        let parent = a_failed_run(&mut tx, org, rule).await;
        let retry = core_db::workflow_runs::retry_run(
            &mut tx,
            org,
            Uuid::now_v7(),
            parent,
            actor,
            None,
            RetryMode::Resume,
        )
        .await
        .expect("a retry of the failed run");
        core_db::workflow_runs::finish_run(
            &mut tx,
            org,
            retry.id,
            RunStatus::Succeeded,
            None,
            None,
        )
        .await
        .expect("the retry finished inside the window");
        // The parent finished 40 days ago, its retry 10 days ago: the parent
        // is past the 30-day default and the retry is not.
        age_finished_at(&mut tx, parent, 40).await;
        age_finished_at(&mut tx, retry.id, 10).await;
        tx.commit().await.expect("commit the chain");
        (parent, retry.id)
    };

    core_db::workflow_runs::purge_expired_everywhere(&db.admin)
        .await
        .expect("the purge must not abort on the chain's own reference");

    let mut tx = org_tx(&db.app, org).await;
    assert!(
        core_db::workflow_runs::get_run(&mut tx, org, parent)
            .await
            .is_err(),
        "the parent is past the window and gone"
    );
    let survivor = core_db::workflow_runs::get_run(&mut tx, org, retry)
        .await
        .expect("the retry is inside its window and stays");
    assert!(
        survivor.retry_of.is_none(),
        "the reference is released rather than left dangling"
    );
    assert_eq!(survivor.status, RunStatus::Succeeded);
    assert_eq!(
        survivor.rule_definition,
        document(),
        "the surviving run keeps the document it ran"
    );
    assert_eq!(survivor.rule_name, "Escalate overdue tickets");
    assert_eq!(survivor.trigger, RunTrigger::OperatorRetry);
    assert_eq!(survivor.trigger_user_id, Some(actor));
}

#[tokio::test]
async fn the_purge_is_idempotent_and_keeps_every_row_inside_a_window() {
    // Slots 91085 (org), 91086 (rule), 91087 (actor).
    //
    // Two claims in one pass, on one transaction so the counts are the rows
    // this test wrote: a second sweep removes nothing and does not abort, and
    // the rows the windows keep are untouched — a parked run is owed a wake,
    // a recent run is inside its window, and neither has its provenance
    // rewritten.
    let db = test_db().await;
    let org = seed_org(&db, 91085, "ledger-91085").await;
    let rule = seed_rule(&db, org, 91086, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91087, "op@ledger85.test").await;
    let mut tx = org_tx(&db.app, org).await;

    let parent = a_failed_run(&mut tx, org, rule).await;
    let retry = core_db::workflow_runs::retry_run(
        &mut tx,
        org,
        Uuid::now_v7(),
        parent,
        actor,
        None,
        RetryMode::Resume,
    )
    .await
    .expect("retry");
    core_db::workflow_runs::finish_run(&mut tx, org, retry.id, RunStatus::Succeeded, None, None)
        .await
        .expect("finish retry");
    age_finished_at(&mut tx, parent, 40).await;
    age_finished_at(&mut tx, retry.id, 10).await;

    // A parked run whose wake-at is long past: owed a wake, never swept.
    let parked = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("parked");
    core_db::workflow_runs::park_run(
        &mut tx,
        org,
        parked.id,
        json!({"node": "flow.delay"}),
        Some(chrono::Utc::now() - Duration::from_secs(86_400)),
        Some("flow.delay"),
    )
    .await
    .expect("park in the past");

    // A terminal run inside the window.
    let recent = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("recent");
    core_db::workflow_runs::finish_run(&mut tx, org, recent.id, RunStatus::Failed, None, None)
        .await
        .expect("finish recent");

    let first =
        core_db::workflow_runs::purge_expired(&mut tx, org, workflow_runs::RETENTION_DEFAULT_DAYS)
            .await
            .expect("the first sweep must not abort on the chain");
    assert_eq!(first, 1, "only the parent is past the window");

    let second =
        core_db::workflow_runs::purge_expired(&mut tx, org, workflow_runs::RETENTION_DEFAULT_DAYS)
            .await
            .expect("the second sweep must not abort and must not re-delete");
    assert_eq!(second, 0, "the purge is idempotent");

    assert!(
        core_db::workflow_runs::get_run(&mut tx, org, parent)
            .await
            .is_err(),
        "the parent stays gone"
    );
    let kept_retry = core_db::workflow_runs::get_run(&mut tx, org, retry.id)
        .await
        .expect("the retry is kept");
    assert!(kept_retry.retry_of.is_none(), "released on the first sweep");
    assert_eq!(kept_retry.status, RunStatus::Succeeded);

    let kept_parked = core_db::workflow_runs::get_run(&mut tx, org, parked.id)
        .await
        .expect("a parked run is never swept");
    assert_eq!(kept_parked.status, RunStatus::Waiting);
    assert!(
        kept_parked.resume_cursor.is_some(),
        "its cursor is still where it parked"
    );

    let kept_recent = core_db::workflow_runs::get_run(&mut tx, org, recent.id)
        .await
        .expect("a run inside the window stays");
    assert_eq!(kept_recent.status, RunStatus::Failed);
}

#[tokio::test]
async fn the_guard_still_refuses_a_hand_written_provenance_edit() {
    // Slots 91088 (org), 91089 (rule), 91090 (actor).
    //
    // 0169 lets the referential action RELEASE a pointer; it does not let a
    // statement do it. Each refusal is its own transaction because a failed
    // statement aborts the one after it, and the next refusal would then
    // read as "transaction aborted" instead of naming the guard.
    let db = test_db().await;
    let org = seed_org(&db, 91088, "ledger-91088").await;
    let rule = seed_rule(&db, org, 91089, "Escalate overdue tickets").await;
    let actor = seed_user(&db, org, 91090, "op@ledger88.test").await;

    let (parent, retry) = {
        let mut tx = org_tx(&db.app, org).await;
        let parent = a_failed_run(&mut tx, org, rule).await;
        let retry = core_db::workflow_runs::retry_run(
            &mut tx,
            org,
            Uuid::now_v7(),
            parent,
            actor,
            None,
            RetryMode::Resume,
        )
        .await
        .expect("retry");
        core_db::workflow_runs::finish_run(
            &mut tx,
            org,
            retry.id,
            RunStatus::Succeeded,
            None,
            None,
        )
        .await
        .expect("finish retry");
        tx.commit().await.expect("commit");
        (parent, retry.id)
    };

    // (a) a release written by hand is still a provenance edit.
    let mut tx = org_tx(&db.app, org).await;
    let release = sqlx::query("UPDATE workflow_runs SET retry_of = NULL WHERE id = $1")
        .bind(retry)
        .execute(&mut *tx)
        .await
        .expect_err("a hand-written release is refused");
    assert!(
        release.to_string().contains("audit ledger"),
        "the guard names itself: {release}"
    );
    drop(tx);

    // (b) so is a rewrite to a different run.
    let mut tx = org_tx(&db.app, org).await;
    let rewrite = sqlx::query("UPDATE workflow_runs SET retry_of = $2 WHERE id = $1")
        .bind(retry)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .expect_err("a hand-written rewrite is refused");
    assert!(
        rewrite.to_string().contains("audit ledger"),
        "the guard names itself: {rewrite}"
    );
    drop(tx);

    // (c) and so is the identity a run must never change.
    let mut tx = org_tx(&db.app, org).await;
    let renamed = sqlx::query("UPDATE workflow_runs SET rule_name = 'Rewritten' WHERE id = $1")
        .bind(retry)
        .execute(&mut *tx)
        .await
        .expect_err("the run's name is immutable");
    assert!(
        renamed.to_string().contains("audit ledger"),
        "the guard names itself: {renamed}"
    );
    drop(tx);

    // Nothing above landed: the chain still reads as it did.
    let mut tx = org_tx(&db.app, org).await;
    let intact = core_db::workflow_runs::get_run(&mut tx, org, retry)
        .await
        .expect("the retry still exists");
    assert_eq!(intact.retry_of, Some(parent), "the chain pointer is intact");
    assert_eq!(intact.rule_name, "Escalate overdue tickets");
}

#[tokio::test]
async fn deleting_a_rule_releases_its_runs_instead_of_aborting() {
    // Slots 91091 (org), 91092 (rule).
    //
    // The same collision, on the record's sibling path: `workflow_runs.rule_id`
    // is declared `ON DELETE SET NULL`, so deleting a rule that has ever run
    // fired the same guard. The rule goes, the run keeps its story and only
    // loses the pointer to a workspace row that no longer exists.
    let db = test_db().await;
    let org = seed_org(&db, 91091, "ledger-91091").await;
    let rule = seed_rule(&db, org, 91092, "Escalate overdue tickets").await;

    let mut tx = org_tx(&db.app, org).await;
    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(Some(rule), RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    assert_eq!(run.rule_id, Some(rule));

    core_db::automations::delete(&mut *tx, org, rule)
        .await
        .expect("deleting a rule that has run is not blocked by the ledger guard");

    let survivor = core_db::workflow_runs::get_run(&mut tx, org, run.id)
        .await
        .expect("the run keeps its row");
    assert!(
        survivor.rule_id.is_none(),
        "the pointer to the deleted rule is released"
    );
    assert_eq!(
        survivor.rule_name, "Escalate overdue tickets",
        "the run still says what it ran"
    );
    assert_eq!(survivor.rule_definition, document());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn purging_an_outbox_event_releases_the_run_that_came_from_it() {
    // Slots 91093 (org), 91094 (rule).
    //
    // The third leg of the same collision. `trigger_event_id` is declared
    // `ON DELETE SET NULL`, so the outbox retention pass
    // (`outbox::prune_published`) releasing a purged event hit the same
    // guard and aborted. The statement below is the delete that pass issues,
    // without its global blast radius: one event, this test's.
    let db = test_db().await;
    let org = seed_org(&db, 91093, "ledger-91093").await;
    let rule = seed_rule(&db, org, 91094, "Escalate overdue tickets").await;
    let event = Uuid::now_v7();

    let run = {
        let mut tx = db.admin.begin().await.expect("begin admin");
        core_db::outbox::insert(
            &mut tx,
            event,
            org,
            "ticket.created",
            "ticket",
            None,
            &json!({"id": "11111111-1111-1111-1111-111111111111"}),
        )
        .await
        .expect("the event a run is started from");
        let mut new = new_run(Some(rule), RunTrigger::Event);
        new.trigger_event_id = Some(event);
        let run = core_db::workflow_runs::insert_run(&mut tx, org, &new, RunStatus::Running)
            .await
            .expect("run");
        tx.commit().await.expect("commit");
        run.id
    };

    sqlx::query("DELETE FROM outbox_events WHERE id = $1")
        .bind(event)
        .execute(&db.admin)
        .await
        .expect("purging an event a run came from is not blocked by the ledger guard");

    let mut tx = org_tx(&db.app, org).await;
    let survivor = core_db::workflow_runs::get_run(&mut tx, org, run)
        .await
        .expect("the run keeps its row");
    assert!(
        survivor.trigger_event_id.is_none(),
        "the pointer to the purged event is released"
    );
    assert_eq!(survivor.rule_name, "Escalate overdue tickets");
    assert_eq!(survivor.status, RunStatus::Running);
}
