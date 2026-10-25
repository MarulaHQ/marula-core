//! Live-database proof for Wave 55 lane `p55-workflow-approval-binding`.
//!
//! The record (`worker.workflow.flow-await-approval-gate-inert`) said the gate
//! was inert. What this suite pins is what replaced it:
//!
//! 1. a run parked at `flow.awaitApproval` files ONE pending approval and its
//!    cursor binds the run to that row, so the waiters index actually finds
//!    the run when the decision event is published;
//! 2. only the gate's OWN approving decision, from a principal the gate
//!    entitles, continues the run (a new `resumed_from` row);
//! 3. a decision for a different gate, a decision from a principal the gate
//!    did not name, and a declined decision all leave the run parked, and each
//!    writes the reason to the org's audit trail against that run;
//! 4. the expiry still resolves the run, honouring `onExpiry`;
//! 5. a late decision cannot revive a run that was abandoned.
//!
//! Non-purpose: the pure decision rule (core-domain unit tests), the HTTP
//! decision route (`core-api/tests/agent_platform_flow.rs`) and the walk
//! itself (core-db and worker unit tests).
//!
//! Database selection follows the worker suite contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL, schema reset ONCE per process.

use core_db::agent_platform;
use core_db::queue::ClaimedJob;
use core_db::workflow_runs as runs;
use core_db::{automations, orgs, users};
use core_domain::automations::AutomationSubjectKind;
use core_domain::workflow_document::{WorkflowCursor, DOCUMENT_VERSION};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use worker::effects::{EffectOutcome, RecordingEffectApi};
use worker::workflow::{process_resume_job, JobOutcome};

/// Slot base for this suite: orgs, users and rules are minted from it, so the
/// suite never collides with another test binary's fixtures.
const SLOT: u128 = 0xC0DE_5500_0000_0000_0000_0000_0000_0000;

async fn test_pool() -> PgPool {
    static BOOTSTRAP: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let url = BOOTSTRAP
        .get_or_init(|| async {
            let url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MARULA_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
                     Postgres for the workflow approval suite",
                );
            let admin = core_db::connect(&url).await.expect("connect");
            sqlx::query(
                "DO $$ BEGIN DROP SCHEMA IF EXISTS ops CASCADE; \
                 DROP SCHEMA IF EXISTS public CASCADE; END $$",
            )
            .execute(&admin)
            .await
            .expect("drop schema");
            sqlx::query("CREATE SCHEMA public")
                .execute(&admin)
                .await
                .expect("create schema");
            core_db::run_migrations(&admin).await.expect("migrations");
            url
        })
        .await
        .clone();
    core_db::connect(&url).await.expect("pool")
}

struct Fixture {
    org: Uuid,
    rule: Uuid,
    /// The principal the gate names.
    approver: Uuid,
    /// A member the gate does not name.
    outsider: Uuid,
}

/// One org, three users and one gated workflow document.
async fn seed(pool: &PgPool, slot: u128) -> Fixture {
    let org = Uuid::from_u128(SLOT + slot * 10);
    let author = Uuid::from_u128(SLOT + slot * 10 + 1);
    let approver = Uuid::from_u128(SLOT + slot * 10 + 2);
    let outsider = Uuid::from_u128(SLOT + slot * 10 + 3);
    let rule = Uuid::from_u128(SLOT + slot * 10 + 4);
    let slug = format!("gate-{slot}");

    let mut conn = pool.acquire().await.expect("connection");
    orgs::insert_org(&mut *conn, org, "Gate Co", &slug, "GAT")
        .await
        .expect("org");
    orgs::seed_default_roles(&mut conn, org)
        .await
        .expect("roles");
    for (id, label) in [
        (author, "author"),
        (approver, "approver"),
        (outsider, "outsider"),
    ] {
        users::create_user(
            &mut *conn,
            id,
            org,
            &format!("{label}@{slug}.test"),
            label,
            "argon2hash",
            "UTC",
        )
        .await
        .expect("user");
    }
    automations::insert_workflow(
        &mut *conn,
        rule,
        org,
        Some(author),
        "gated",
        "ticket.created",
        AutomationSubjectKind::default(),
        gated_document(approver),
    )
    .await
    .expect("rule");

    Fixture {
        org,
        rule,
        approver,
        outsider,
    }
}

/// entry → gate → stop. The gate names one user; `onExpiry: fail` so the
/// expiry test can prove the protected edge does not fire.
fn gated_document(approver: Uuid) -> Value {
    json!({
        "version": DOCUMENT_VERSION,
        "nodes": [
            {
                "id": "gate",
                "node_type": "flow.awaitApproval",
                "params": {
                    "approvers": [approver.to_string()],
                    "timeout": "1h",
                    "onExpiry": "fail",
                    "prompt": "Release this run?"
                }
            },
            { "id": "after", "node_type": "flow.stop", "params": {} }
        ],
        "edges": [ { "from": "gate", "to": "after" } ]
    })
}

fn job(org: Uuid, payload: Value) -> ClaimedJob {
    ClaimedJob {
        id: Uuid::now_v7(),
        org_id: org,
        kind: core_domain::workflow_runs::WORKFLOW_RESUME_JOB.to_string(),
        payload,
        attempts: 1,
        max_attempts: 5,
        claimed_by: "workflow-approval-suite".to_string(),
    }
}

/// Inserts the queue row a claimed job represents.
///
/// `workflow_runs.trigger_job_id` is a foreign key, so a run started by a job
/// the queue never held cannot be written. The executor normally consumes a
/// row the dispatcher inserted; here the suite stands in for the dispatcher.
async fn enqueue(pool: &PgPool, j: &ClaimedJob) {
    sqlx::query(
        "INSERT INTO jobs (id, org_id, kind, payload, status, attempts, max_attempts, updated_at) \
         VALUES ($1, $2, $3, $4, 'running', $5, $6, now())",
    )
    .bind(j.id)
    .bind(j.org_id)
    .bind(&j.kind)
    .bind(&j.payload)
    .bind(j.attempts)
    .bind(j.max_attempts)
    .execute(pool)
    .await
    .expect("queue row");
}

/// Starts the document and returns the run it parked at the gate.
async fn start_at_gate(pool: &PgPool, fx: &Fixture) -> Uuid {
    let effects = RecordingEffectApi::new(EffectOutcome::Applied);
    let start_job = job(
        fx.org,
        json!({
            "op": "start",
            "rule_id": fx.rule,
            "event": { "ticket_id": null },
            "event_type": "ticket.created",
            "event_entity": "ticket",
        }),
    );
    enqueue(pool, &start_job).await;
    let result = process_resume_job(pool, &effects, &start_job).await;
    assert!(
        matches!(result.outcome, JobOutcome::Done),
        "the start job must settle: {}",
        result.error
    );
    let mut conn = pool.acquire().await.expect("connection");
    let run_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM workflow_runs WHERE org_id = $1 AND rule_id = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(fx.org)
    .bind(fx.rule)
    .fetch_one(&mut *conn)
    .await
    .expect("run");
    let run = runs::get_run(&mut conn, fx.org, run_id)
        .await
        .expect("run row");
    assert_eq!(run.status.as_str(), "waiting", "the gate parks the run");
    run_id
}

/// The approval the parked run filed, and the cursor that binds it.
async fn parked_gate(pool: &PgPool, org: Uuid, run_id: Uuid) -> (Uuid, WorkflowCursor) {
    let mut conn = pool.acquire().await.expect("connection");
    let run = runs::get_run(&mut conn, org, run_id)
        .await
        .expect("parked run");
    let cursor: WorkflowCursor = serde_json::from_value(
        run.resume_cursor
            .clone()
            .expect("a parked run carries its cursor"),
    )
    .expect("cursor");
    let bound = cursor
        .approvals
        .get("gate")
        .expect("the gate is bound to an approval")
        .clone();
    (bound.parse().expect("proposal id"), cursor)
}

/// Decides one proposal the way the route does: compare-and-set on `proposed`.
async fn decide(pool: &PgPool, org: Uuid, proposal: Uuid, decider: Uuid, status: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    agent_platform::decide_proposal(&mut conn, org, proposal, status, decider, None)
        .await
        .expect("decision")
        .expect("the proposal was still proposed");
}

/// Delivers one wake for a parked run. `event: None` is the expiry wake.
///
/// The job is enqueued first, exactly as the dispatcher inserts the row it
/// claims: a continuation records the wake job as its `trigger_job_id`, which
/// is a foreign key, so a job the queue never held cannot write its run.
async fn wake(
    pool: &PgPool,
    fx: &Fixture,
    run_id: Uuid,
    event: Option<Value>,
    event_type: Option<&str>,
) {
    let mut payload = json!({ "op": "resume", "run_id": run_id });
    if let Some(event) = event {
        payload["event"] = event;
    }
    if let Some(event_type) = event_type {
        payload["event_type"] = json!(event_type);
        payload["event_entity"] = json!("agent_proposal");
    }
    let wake_job = job(fx.org, payload);
    enqueue(pool, &wake_job).await;
    let effects = RecordingEffectApi::new(EffectOutcome::Applied);
    let result = process_resume_job(pool, &effects, &wake_job).await;
    assert!(
        matches!(result.outcome, JobOutcome::Done),
        "the wake must settle: {}",
        result.error
    );
}

/// The continuation a wake created, if it created one.
async fn continuation(
    pool: &PgPool,
    org: Uuid,
    parent: Uuid,
) -> Option<(Uuid, String, Option<String>)> {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query_as(
        "SELECT id, status, error_code FROM workflow_runs \
         WHERE org_id = $1 AND resumed_from = $2",
    )
    .bind(org)
    .bind(parent)
    .fetch_optional(&mut *conn)
    .await
    .expect("continuation read")
}

async fn status_of(pool: &PgPool, org: Uuid, run_id: Uuid) -> String {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query_scalar("SELECT status FROM workflow_runs WHERE org_id = $1 AND id = $2")
        .bind(org)
        .bind(run_id)
        .fetch_one(&mut *conn)
        .await
        .expect("status")
}

/// The newest refusal recorded against one run, as (code, detail).
async fn latest_refusal(pool: &PgPool, org: Uuid, run_id: Uuid) -> Option<(String, Value)> {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query_as(
        "SELECT detail->>'code', detail FROM audit_events \
         WHERE org_id = $1 AND action = 'workflow.approval_refused' \
           AND entity_type = 'workflow_run' AND entity_id = $2 \
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org)
    .bind(run_id.to_string())
    .fetch_optional(&mut *conn)
    .await
    .expect("refusal read")
}

/// The step rows the continuation recorded, as (node_id, status, output).
async fn steps(pool: &PgPool, org: Uuid, run_id: Uuid) -> Vec<(String, String, Option<Value>)> {
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query_as(
        "SELECT node_id, status, output FROM workflow_run_steps \
         WHERE org_id = $1 AND run_id = $2 ORDER BY step_index, attempt",
    )
    .bind(org)
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await
    .expect("steps read")
}

// ===========================================================================
// 1. The approval that releases the gate
// ===========================================================================

#[tokio::test]
async fn an_entitled_approval_resumes_the_run() {
    let pool = test_pool().await;
    let fx = seed(&pool, 1).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (proposal, _cursor) = parked_gate(&pool, fx.org, parent).await;

    // The parked row is filed in the queue the approver works from, and the
    // waiters index finds the run by the event the decision will carry — the
    // two facts the old literal string broke.
    let mut conn = pool.acquire().await.expect("connection");
    let row = agent_platform::get_proposal(&mut *conn, fx.org, proposal)
        .await
        .expect("gate approval row");
    assert_eq!(row.entity, core_domain::workflow_approvals::GATE_ENTITY);
    assert_eq!(row.entity_id, Some(parent));
    assert_eq!(row.status, "proposed");
    let waiters = runs::list_waiting_on_event(
        &mut conn,
        fx.org,
        core_domain::events::types::AGENT_PROPOSAL_APPROVED,
    )
    .await
    .expect("waiters");
    assert!(
        waiters.iter().any(|run| run.id == parent),
        "the parked run is woken by the approval event type"
    );

    decide(&pool, fx.org, proposal, fx.approver, "approved").await;
    wake(
        &pool,
        &fx,
        parent,
        Some(json!({
            "proposal_id": proposal,
            "entity": core_domain::workflow_approvals::GATE_ENTITY,
            "entity_id": parent,
            "approved_by": fx.approver,
            "applied": false,
        })),
        Some(core_domain::events::types::AGENT_PROPOSAL_APPROVED),
    )
    .await;

    let (child, status, error) = continuation(&pool, fx.org, parent)
        .await
        .expect("an admitted decision continues the run");
    assert_eq!(
        status, "succeeded",
        "the document ran to its stop: {error:?}"
    );
    let gate_step = steps(&pool, fx.org, child)
        .await
        .into_iter()
        .find(|(node, _, _)| node == "flow.awaitApproval")
        .expect("the resumed gate records its step");
    assert_eq!(gate_step.1, "succeeded");
    let output = gate_step.2.expect("gate output");
    assert_eq!(output["outcome"], json!("approved"));
    assert_eq!(output["approverId"], json!(fx.approver));
    assert_eq!(output["proposalId"], json!(proposal.to_string()));
    assert!(
        steps(&pool, fx.org, child)
            .await
            .iter()
            .any(|(node, _, _)| node == "flow.stop"),
        "the branch after the gate fired"
    );
}

// ===========================================================================
// 2. A principal the gate did not name
// ===========================================================================

#[tokio::test]
async fn a_principal_the_gate_did_not_name_cannot_resume_it() {
    let pool = test_pool().await;
    let fx = seed(&pool, 2).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (proposal, _) = parked_gate(&pool, fx.org, parent).await;

    // The route refuses this principal; a decision that got past it anyway
    // (an older build, a direct write) still must not move the run.
    decide(&pool, fx.org, proposal, fx.outsider, "approved").await;
    wake(
        &pool,
        &fx,
        parent,
        Some(json!({
            "proposal_id": proposal,
            "entity": core_domain::workflow_approvals::GATE_ENTITY,
            "entity_id": parent,
            "approved_by": fx.outsider,
            "applied": false,
        })),
        Some(core_domain::events::types::AGENT_PROPOSAL_APPROVED),
    )
    .await;

    assert!(
        continuation(&pool, fx.org, parent).await.is_none(),
        "a non-entitled decision must not continue the run"
    );
    assert_eq!(status_of(&pool, fx.org, parent).await, "waiting");
    let (code, detail) = latest_refusal(&pool, fx.org, parent)
        .await
        .expect("the refusal is recorded");
    assert_eq!(code, "approval_not_entitled");
    assert_eq!(detail["proposal_id"], json!(proposal.to_string()));
    assert_eq!(detail["gate"], json!("gate"));
}

// ===========================================================================
// 3. A decision that satisfies a different gate
// ===========================================================================

#[tokio::test]
async fn a_decision_for_another_gate_does_not_resume_this_one() {
    let pool = test_pool().await;
    let fx = seed(&pool, 3).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (_proposal, _) = parked_gate(&pool, fx.org, parent).await;

    // Another run's gate, decided by its own entitled approver.
    let other_run = Uuid::now_v7();
    let other = Uuid::now_v7();
    let mut conn = pool.acquire().await.expect("connection");
    agent_platform::insert_proposal(
        &mut conn,
        other,
        fx.org,
        &agent_platform::ProposalFields {
            entity: core_domain::workflow_approvals::GATE_ENTITY.to_string(),
            entity_id: Some(other_run),
            op: "update".to_string(),
            before_value: Value::Null,
            proposed_value: json!({
                "gate": "gate",
                "approvers": [fx.approver.to_string()],
                "onExpiry": "fail"
            }),
            classification: "internal".to_string(),
            capability: None,
            dial_mode: "proposed".to_string(),
            base_version: 0,
            agent_kind: "agent".to_string(),
            agent_ref: format!("workflow-run:{other_run}"),
            skill_slug: None,
            skill_version: None,
            rule_set_hash: None,
            session_id: None,
            tool: None,
            for_user: None,
            rationale: None,
        },
    )
    .await
    .expect("other gate approval");
    decide(&pool, fx.org, other, fx.approver, "approved").await;

    wake(
        &pool,
        &fx,
        parent,
        Some(json!({
            "proposal_id": other,
            "entity": core_domain::workflow_approvals::GATE_ENTITY,
            "entity_id": other_run,
            "approved_by": fx.approver,
            "applied": false,
        })),
        Some(core_domain::events::types::AGENT_PROPOSAL_APPROVED),
    )
    .await;

    assert!(continuation(&pool, fx.org, parent).await.is_none());
    assert_eq!(status_of(&pool, fx.org, parent).await, "waiting");
    let (code, _) = latest_refusal(&pool, fx.org, parent)
        .await
        .expect("the refusal is recorded");
    assert_eq!(code, "approval_other_gate");
}

// ===========================================================================
// 4. A declined decision
// ===========================================================================

#[tokio::test]
async fn a_declined_decision_does_not_resume_and_is_recorded() {
    let pool = test_pool().await;
    let fx = seed(&pool, 4).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (proposal, _) = parked_gate(&pool, fx.org, parent).await;

    decide(&pool, fx.org, proposal, fx.approver, "declined").await;
    // The row is decided once: a later approval cannot release the gate.
    let mut conn = pool.acquire().await.expect("connection");
    assert!(
        agent_platform::decide_proposal(&mut conn, fx.org, proposal, "approved", fx.approver, None)
            .await
            .expect("second decision")
            .is_none(),
        "a declined approval cannot be re-decided"
    );

    wake(
        &pool,
        &fx,
        parent,
        Some(json!({
            "proposal_id": proposal,
            "entity": core_domain::workflow_approvals::GATE_ENTITY,
            "entity_id": parent,
            "declined_by": fx.approver,
        })),
        Some(core_domain::events::types::AGENT_PROPOSAL_DECLINED),
    )
    .await;

    assert!(continuation(&pool, fx.org, parent).await.is_none());
    assert_eq!(status_of(&pool, fx.org, parent).await, "waiting");
    let (code, detail) = latest_refusal(&pool, fx.org, parent)
        .await
        .expect("the refusal is recorded");
    assert_eq!(code, "approval_declined");
    assert_eq!(
        detail["event_type"],
        json!(core_domain::events::types::AGENT_PROPOSAL_DECLINED)
    );
}

// ===========================================================================
// 5. A cancelled run and a late decision
// ===========================================================================

#[tokio::test]
async fn a_late_decision_cannot_resume_a_cancelled_run() {
    let pool = test_pool().await;
    let fx = seed(&pool, 5).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (proposal, _) = parked_gate(&pool, fx.org, parent).await;

    // The reaper's shape: a waiting run moved to abandoned, its park
    // coordinates consumed and its finish time written.
    let mut conn = pool.acquire().await.expect("connection");
    sqlx::query(
        "UPDATE workflow_runs SET status = 'abandoned', resume_cursor = NULL, \
         wake_at = NULL, wake_on = NULL, finished_at = now(), error_code = 'wake_lost' \
         WHERE org_id = $1 AND id = $2",
    )
    .bind(fx.org)
    .bind(parent)
    .execute(&mut *conn)
    .await
    .expect("cancel");

    decide(&pool, fx.org, proposal, fx.approver, "approved").await;
    wake(
        &pool,
        &fx,
        parent,
        Some(json!({
            "proposal_id": proposal,
            "entity": core_domain::workflow_approvals::GATE_ENTITY,
            "entity_id": parent,
            "approved_by": fx.approver,
            "applied": false,
        })),
        Some(core_domain::events::types::AGENT_PROPOSAL_APPROVED),
    )
    .await;

    assert!(
        continuation(&pool, fx.org, parent).await.is_none(),
        "a cancelled run has nothing to continue"
    );
    assert_eq!(status_of(&pool, fx.org, parent).await, "abandoned");
}

// ===========================================================================
// 6. The expiry still resolves the run
// ===========================================================================

#[tokio::test]
async fn the_expiry_honours_on_expiry_and_fires_no_guarded_edge() {
    let pool = test_pool().await;
    let fx = seed(&pool, 6).await;
    let parent = start_at_gate(&pool, &fx).await;
    let (proposal, _) = parked_gate(&pool, fx.org, parent).await;

    // No event payload: this is the time wake the park enqueued at `wake_at`.
    wake(&pool, &fx, parent, None, None).await;

    let (child, status, error) = continuation(&pool, fx.org, parent)
        .await
        .expect("the expiry continues the run so it can end");
    assert_eq!(status, "failed");
    assert_eq!(error.as_deref(), Some("approval_expired"));
    assert!(
        !steps(&pool, fx.org, child)
            .await
            .iter()
            .any(|(node, _, _)| node == "flow.stop"),
        "onExpiry: fail must not fire the branch the gate guards"
    );
    assert!(
        latest_refusal(&pool, fx.org, parent).await.is_none(),
        "an expiry is not a refused decision"
    );
    // The approval nobody answered is closed, not left in the queue as a
    // decision someone still owes on a finished run.
    let mut conn = pool.acquire().await.expect("connection");
    let row = agent_platform::get_proposal(&mut *conn, fx.org, proposal)
        .await
        .expect("gate approval row");
    assert_eq!(row.status, "withdrawn");
    assert!(
        row.decided_by.is_none(),
        "nobody decided, so nobody is named"
    );
    let expiries: Vec<String> = sqlx::query_scalar(
        "SELECT detail->>'reason' FROM audit_events \
         WHERE org_id = $1 AND action = 'agent_proposal.expired'",
    )
    .bind(fx.org)
    .fetch_all(&mut *conn)
    .await
    .expect("expiry audit");
    assert_eq!(expiries.len(), 1, "the closure is recorded once");
}

/// F51. The retry route writes one `workflow.resume` job for the row it is
/// inserting. This drives that payload through the executor and pins the two
/// things a wrong payload does instead: no `op` dead-letters the job, and a
/// `run_id` naming the FAILED run settles as "already finished" without ever
/// resuming. The route's own test pins the same three keys, so a regression on
/// either side fails a test rather than a run.
#[tokio::test]
async fn a_retried_run_resumes_instead_of_dying() {
    let pool = test_pool().await;
    let fx = seed(&pool, 7).await;

    // A failed run with one node to reach: nothing whose repeat could apply
    // twice, so the retry is allowed and the continuation has work to do.
    let mut tx = pool.begin().await.expect("tx");
    let failed = runs::insert_run(
        &mut tx,
        fx.org,
        &runs::NewRun {
            rule_id: None,
            rule_name: "retried".to_string(),
            rule_version: 1,
            rule_definition: json!({
                "version": DOCUMENT_VERSION,
                "nodes": [{ "id": "stop", "node_type": "flow.stop", "params": {} }],
            }),
            catalogue_version: core_domain::node_registry::CATALOGUE_VERSION.to_string(),
            trigger: core_domain::workflow_runs::RunTrigger::Event,
            trigger_event_id: None,
            trigger_job_id: None,
            trigger_user_id: Some(fx.approver),
            resumed_from: None,
            resume_cursor: None,
        },
        core_domain::workflow_runs::RunStatus::Running,
    )
    .await
    .expect("failed run");
    runs::finish_run(
        &mut tx,
        fx.org,
        failed.id,
        core_domain::workflow_runs::RunStatus::Failed,
        Some("step_failed"),
        Some("the probe failed"),
    )
    .await
    .expect("finish the failed run");
    tx.commit().await.expect("commit");

    // The retry, in the route's order: the job is enqueued first (the retry
    // row's `trigger_job_id` is a foreign key), its payload names the row the
    // caller is about to insert, and the insert uses that same id.
    let retry_id = Uuid::now_v7();
    let retry_job = job(
        fx.org,
        json!({ "op": "resume", "run_id": retry_id, "retry": true }),
    );
    enqueue(&pool, &retry_job).await;
    let mut tx = pool.begin().await.expect("tx");
    let retry = runs::retry_run(
        &mut tx,
        fx.org,
        retry_id,
        failed.id,
        fx.approver,
        Some(retry_job.id),
        core_domain::workflow_runs::RetryMode::Resume,
    )
    .await
    .expect("a stop-only run may be retried");
    tx.commit().await.expect("commit");
    assert_eq!(retry.id, retry_id, "the payload named this row");
    assert_eq!(
        retry.status.as_str(),
        "waiting",
        "the executor resumes a waiting row"
    );

    let effects = RecordingEffectApi::new(EffectOutcome::Applied);
    let result = process_resume_job(&pool, &effects, &retry_job).await;
    assert!(
        matches!(result.outcome, JobOutcome::Done),
        "a retried run must settle, not dead-letter: {}",
        result.error
    );

    let (_, status, error) = continuation(&pool, fx.org, retry_id)
        .await
        .expect("the retry resumed into a continuation");
    assert_eq!(status, "succeeded", "the continuation ran: {error:?}");
    assert_eq!(
        status_of(&pool, fx.org, failed.id).await,
        "failed",
        "the original is a ledger row, never rewritten by a retry"
    );
}
