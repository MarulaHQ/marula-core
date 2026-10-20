//! The workflow run ledger's HTTP flows through the REAL core-api router
//! (the private build tracker Wave 43 lane 14, `p43-workflow-run-ledger`).
//!
//! Purpose: prove the operator surface behaves like the rest of the product
//! and unlike a queue flag:
//!
//! 1. the ledger reads work end to end (list with its cursor, detail with the
//!    trace, the paged trace) and the run list is admin-scoped;
//! 2. the RETRY is a first-class action on the IF1–IF3 envelope: an
//!    idempotency key makes a double-submit answer the first retry, a retry
//!    that would double-apply is a typed 409 naming the nodes, and an
//!    allowed retry writes a NEW row that points back and enqueues the job
//!    on the EXISTING queue;
//! 3. one org's runs are invisible to another — a 404, not a 403, because
//!    under RLS the two are indistinguishable.
//!
//! Non-purpose: pure retry policy (core-domain unit tests) and SQL behavior
//! (core-db suite).
//!
//! Database selection follows the documented contract in
//! `crates/core-db/tests/common/mod.rs` (MARULA_TEST_DATABASE_URL or
//! DATABASE_URL; no schema reset).

use axum::Router;
use core_api::{auth::oidc_client::OidcHttpClient, config::Config, state::AppState};
use serde_json::{json, Value};
use sqlx::PgPool;
use sqlx::Row as _;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

async fn test_state() -> AppState {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let admin_url = std::env::var("MARULA_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("run ledger flow needs MARULA_TEST_DATABASE_URL or DATABASE_URL");
    let pool = connect(&admin_url).await.expect("pool");
    core_db::run_migrations(&pool).await.expect("migrations");
    let mut config = Config::from_env_allow_dev_fallback().expect("test config");
    let reporting_url = config.database_url.clone();
    config.database_url = admin_url;
    config.secret_key = core_api::config::SecretKey([37; 32]);
    AppState {
        pool,
        config: Arc::new(config.clone()),
        oidc: OidcHttpClient::new(),
        mail: Arc::new(core_api::email::MailStack::from_config(&config)),
        store: Arc::new(core_store::memory::MemoryStore::new()),
        routing: std::sync::Arc::new(core_routing::RoutingRegistry::new()),
        reporting: core_api::state::AppState::reporting_pool_lazy(&reporting_url)
            .expect("lazy reporting pool"),
    }
}

async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
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
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
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
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn register_org(app: &Router, slug: &str) -> String {
    let response = send(
        app,
        axum::http::Method::POST,
        "/api/auth/register",
        None,
        Some(json!({
            "org_name": "Run Ledger Co",
            "org_slug": slug,
            "email": format!("owner@{slug}.test"),
            "password": "correct-horse-battery",
            "display_name": "Owner",
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "register must succeed");
    body_json(response).await["access_token"]
        .as_str()
        .expect("access token")
        .into()
}

async fn me_org_id(app: &Router, token: &str) -> Uuid {
    let response = send(
        app,
        axum::http::Method::GET,
        "/api/v1/me",
        Some(token),
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

fn run_definition() -> Value {
    json!({
        "trigger": {"type": "event", "event": "ticket.created"},
        "conditions": [],
        "actions": [
            {"id": "step_0", "node": "ticket.create",
             "input": {"boardId": "11111111-1111-1111-1111-111111111111", "title": "Escalate"}},
            {"id": "step_1", "node": "email.send",
             "input": {"to": "ops@example.test", "subject": "s", "body": "b"}}
        ]
    })
}

fn new_run(
    rule_id: Option<Uuid>,
    trigger: core_domain::workflow_runs::RunTrigger,
) -> core_db::workflow_runs::NewRun {
    core_db::workflow_runs::NewRun {
        rule_id,
        rule_name: "Escalate overdue tickets".to_string(),
        rule_version: 1,
        rule_definition: run_definition(),
        catalogue_version: core_domain::node_registry::CATALOGUE_VERSION.to_string(),
        trigger,
        trigger_event_id: None,
        trigger_job_id: None,
        trigger_user_id: None,
        resumed_from: None,
        resume_cursor: None,
    }
}

/// Seeds a finished, failed run with one succeeded CREATION and one failed
/// send — the shape an operator inspects and wants to retry. The creation is
/// the interesting half: re-running it mints a second ticket, so it is exactly
/// what a retry must not silently repeat.
async fn seed_failed_run(state: &AppState, org: Uuid) -> (Uuid, Uuid) {
    use core_domain::workflow_runs::{RunStatus, RunTrigger, StepStatus};
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(None, RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let created = core_db::workflow_runs::insert_step(
        &mut tx,
        org,
        run.id,
        &core_db::workflow_runs::NewStep {
            step_index: 0,
            node_id: "ticket.create".to_string(),
            node_label: "Create ticket".to_string(),
            attempt: 1,
            iteration: None,
            input: json!({"boardId": "11111111-1111-1111-1111-111111111111", "title": "Escalate"}),
        },
    )
    .await
    .expect("create step");
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        created.id,
        StepStatus::Succeeded,
        Some(json!({"ticketId": "abcd-1234"})),
        None,
        None,
    )
    .await
    .expect("finish create");
    let send = core_db::workflow_runs::insert_step(
        &mut tx,
        org,
        run.id,
        &core_db::workflow_runs::NewStep {
            step_index: 1,
            node_id: "email.send".to_string(),
            node_label: "Send email".to_string(),
            attempt: 1,
            iteration: None,
            input: json!({"to": "ops@example.test", "subject": "s", "body": "b"}),
        },
    )
    .await
    .expect("send step");
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        send.id,
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
        Some("email.send failed"),
    )
    .await
    .expect("finish run");
    tx.commit().await.expect("commit");
    (run.id, created.id)
}

#[tokio::test]
async fn the_ledger_reads_end_to_end_with_the_delivery_ledgers_cursor_shape() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("run-ledger-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org = me_org_id(&app, &token).await;
    let (run_id, _) = seed_failed_run(&state, org).await;

    // --- the list ------------------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/workflows/runs",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let page = body_json(response).await;
    let runs = page["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1, "the seeded run is listed");
    assert_eq!(runs[0]["id"], run_id.to_string());
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["rule_name"], "Escalate overdue tickets");
    assert_eq!(runs[0]["error_code"], "step_failed");
    assert!(
        page["next_cursor"].is_null(),
        "a one-page ledger has no next cursor"
    );

    // --- the detail: the run plus the trace it explains itself with --------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/workflows/runs/{run_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let detail = body_json(response).await;
    assert_eq!(detail["id"], run_id.to_string());
    // The document that ran rides the detail response, so the run explains
    // itself without a second table.
    assert_eq!(
        detail["rule_definition"]["trigger"]["event"],
        "ticket.created"
    );
    let steps = detail["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 2, "the whole trace, in execution order");
    assert_eq!(steps[0]["node_id"], "ticket.create");
    assert_eq!(steps[0]["status"], "succeeded");
    assert_eq!(steps[0]["output"]["ticketId"], "abcd-1234");
    assert_eq!(steps[1]["node_id"], "email.send");
    assert_eq!(steps[1]["status"], "failed");
    assert_eq!(steps[1]["error_code"], "smtp_unreachable");
    assert!(steps[1]["finished_at"].is_string(), "timing is on the row");

    // --- the paged trace -----------------------------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/workflows/runs/{run_id}/steps"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    let trace = body_json(response).await;
    assert_eq!(trace["steps"].as_array().expect("steps").len(), 2);
    assert!(trace["next_cursor"].is_null());

    // --- an unknown run is a 404, not a 500 ---------------------------------
    let response = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/workflows/runs/{}", Uuid::now_v7()),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn a_retry_that_would_double_apply_is_a_typed_refusal_naming_the_node() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("run-ledger-refuse-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org = me_org_id(&app, &token).await;
    let (run_id, _) = seed_failed_run(&state, org).await;

    // A from-scratch re-run would repeat the succeeded CREATION and the
    // failed SEND, and the registry declares both unsafe to repeat. The
    // refusal is typed and names them.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/workflows/runs/{run_id}/retry"),
        Some(&token),
        Some(json!({
            "mode": "from_start",
            "idempotency_key": format!("retry-{run_id}-fs"),
        })),
    )
    .await;
    assert_eq!(response.status(), 409, "a double-apply retry is refused");
    let body = body_json(response).await;
    assert_eq!(body["error"]["reason"], "run_retry_refused");
    let message = body["error"]["message"].as_str().expect("message");
    assert!(message.contains("ticket.create"), "named the creation");
    assert!(message.contains("email.send"), "named the send");

    // Resuming from the failure point re-runs only the failed send, which is
    // still unsafe to repeat — a failed send may already have sent. So the
    // honest answer is still a refusal.
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/workflows/runs/{run_id}/retry"),
        Some(&token),
        Some(json!({
            "mode": "resume",
            "idempotency_key": format!("retry-{run_id}-rs"),
        })),
    )
    .await;
    assert_eq!(response.status(), 409);
    let body = body_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("email.send"),
        "the refusal names the node that may have half-applied"
    );

    // Nothing was recorded: no retry row, no job.
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let runs = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    assert_eq!(runs.len(), 1, "a refused retry is never recorded");
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE org_id = $1")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .expect("job count");
    assert_eq!(pending, 0, "a refused retry enqueues nothing");
}

#[tokio::test]
async fn an_allowed_retry_records_a_new_row_and_replays_under_the_same_key() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug = format!("run-ledger-retry-{}", Uuid::now_v7().simple());
    let token = register_org(&app, &slug).await;
    let org = me_org_id(&app, &token).await;

    // A run whose only step is a safe-to-repeat state setter, and it failed.
    use core_domain::workflow_runs::{RunStatus, RunTrigger, StepStatus};
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let run = core_db::workflow_runs::insert_run(
        &mut tx,
        org,
        &new_run(None, RunTrigger::Event),
        RunStatus::Running,
    )
    .await
    .expect("run");
    let step = core_db::workflow_runs::insert_step(
        &mut tx,
        org,
        run.id,
        &core_db::workflow_runs::NewStep {
            step_index: 0,
            node_id: "ticket.update".to_string(),
            node_label: "Update ticket".to_string(),
            attempt: 1,
            iteration: None,
            input: json!({"ticketId": "11111111-1111-1111-1111-111111111111", "fields": {"title": "Escalate"}}),
        },
    )
    .await
    .expect("step");
    core_db::workflow_runs::finish_step(
        &mut tx,
        org,
        step.id,
        StepStatus::Failed,
        None,
        Some("validation_failed"),
        Some("field rejected"),
    )
    .await
    .expect("finish step");
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
    tx.commit().await.expect("commit");
    let run_id = run.id;

    // --- the retry -----------------------------------------------------------
    let retry_uri = format!("/api/v1/workflows/runs/{run_id}/retry");
    let response = send(
        &app,
        axum::http::Method::POST,
        &retry_uri,
        Some(&token),
        Some(json!({
            "mode": "resume",
            "idempotency_key": format!("retry-{run_id}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "a safe retry is applied");
    let body = body_json(response).await;
    let retry_id: Uuid = body["run"]["id"].as_str().unwrap().parse().unwrap();
    assert_ne!(retry_id, run_id, "the retry is a NEW row");
    assert_eq!(body["run"]["retry_of"], run_id.to_string());
    assert_eq!(body["run"]["trigger"], "operator_retry");
    assert_eq!(body["run"]["status"], "waiting");
    assert_eq!(body["run"]["wake_on"], "queue");
    // The cursor and the document snapshot are NOT on the listing row: they
    // live on the detail response, which is where an operator who wants to
    // know exactly what ran reads them. Check it there.
    let detail = send(
        &app,
        axum::http::Method::GET,
        &format!("/api/v1/workflows/runs/{retry_id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(detail.status(), 200);
    let detail = body_json(detail).await;
    assert_eq!(
        detail["resume_cursor"]["start_at_step"], 0,
        "the retry's cursor names where it starts"
    );
    assert_eq!(detail["resume_cursor"]["mode"], "resume");
    assert_eq!(
        detail["rule_definition"]["trigger"]["event"], "ticket.created",
        "the retry re-runs the SAME document the failed run carried"
    );
    // IF2: the read-your-write metadata rides the envelope.
    assert!(body["mutation"]["seq"].is_i64(), "mutation meta present");
    assert_eq!(
        body["mutation"]["idempotency_key"],
        format!("retry-{run_id}")
    );

    // --- the same key replays the same answer --------------------------------
    let response = send(
        &app,
        axum::http::Method::POST,
        &retry_uri,
        Some(&token),
        Some(json!({
            "mode": "resume",
            "idempotency_key": format!("retry-{run_id}"),
        })),
    )
    .await;
    assert_eq!(response.status(), 200, "a replay is not a second retry");
    let replay = body_json(response).await;
    assert_eq!(
        replay["run"]["id"],
        retry_id.to_string(),
        "the same key answers the same retry"
    );
    assert_eq!(
        replay["mutation"]["idempotency_key"],
        format!("retry-{run_id}")
    );

    // --- and the database agrees --------------------------------------------
    let mut tx = state.pool.begin().await.expect("tx");
    core_db::set_org(&mut tx, org).await.expect("set_org");
    let page = core_db::workflow_runs::list_runs(
        &mut tx,
        org,
        &core_db::workflow_runs::RunFilter::default(),
        100,
    )
    .await
    .expect("list");
    assert_eq!(page.len(), 2, "the failed run and the retry");
    let listed_original = page.iter().find(|r| r.run.id == run_id).expect("original");
    assert!(listed_original.retried, "the original reads as retried");
    assert_eq!(listed_original.run.status, RunStatus::Failed, "untouched");
    let listed_retry = page.iter().find(|r| r.run.id == retry_id).expect("retry");
    assert_eq!(listed_retry.run.retry_of, Some(run_id));
    let wake_job = listed_retry
        .run
        .trigger_job_id
        .expect("the retry records the job that will run it");
    // The job is on the EXISTING queue, with the documented kind.
    let job = sqlx::query("SELECT kind, payload FROM jobs WHERE id = $1")
        .bind(wake_job)
        .fetch_optional(&mut *tx)
        .await
        .expect("job read")
        .expect("the job exists");
    assert_eq!(
        job.get::<String, _>("kind"),
        core_db::workflow_runs::WORKFLOW_RESUME_JOB
    );
    assert_eq!(job.get::<Value, _>("payload")["retry"], json!(true));

    // --- the audit trail names the action ------------------------------------
    let audit = sqlx::query(
        "SELECT action FROM audit_events WHERE entity_type = 'workflow_run' AND entity_id = $1",
    )
    .bind(run_id.to_string())
    .fetch_optional(&mut *tx)
    .await
    .expect("audit read")
    .expect("the retry is audited");
    assert_eq!(audit.get::<String, _>("action"), "workflow.run_retried");
}

#[tokio::test]
async fn one_orgs_runs_are_invisible_to_another() {
    let state = test_state().await;
    let app = core_api::routes::build_router(state.clone());
    let slug_a = format!("run-ledger-a-{}", Uuid::now_v7().simple());
    let slug_b = format!("run-ledger-b-{}", Uuid::now_v7().simple());
    let token_a = register_org(&app, &slug_a).await;
    let token_b = register_org(&app, &slug_b).await;
    let org_a = me_org_id(&app, &token_a).await;
    let org_b = me_org_id(&app, &token_b).await;
    let (run_id, _) = seed_failed_run(&state, org_a).await;

    // Org B's own ledger is empty — org A's run never leaked into it.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/workflows/runs",
        Some(&token_b),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_json(response).await["runs"]
            .as_array()
            .expect("runs")
            .len(),
        0,
        "org B sees only its own runs"
    );

    // Org B cannot read org A's run, and cannot retry it. Both are 404
    // rather than 403: under RLS the run is indistinguishable from one
    // that never existed, and the read posture keeps that honest.
    for uri in [
        format!("/api/v1/workflows/runs/{run_id}"),
        format!("/api/v1/workflows/runs/{run_id}/steps"),
    ] {
        let response = send(&app, axum::http::Method::GET, &uri, Some(&token_b), None).await;
        assert_eq!(response.status(), 404, "{uri}");
    }
    let response = send(
        &app,
        axum::http::Method::POST,
        &format!("/api/v1/workflows/runs/{run_id}/retry"),
        Some(&token_b),
        Some(json!({"mode": "resume", "idempotency_key": "cross-org"})),
    )
    .await;
    assert_eq!(response.status(), 404, "a cross-org retry is invisible");

    // Org B's own ledger still works, proving the 404s were the isolation
    // and not a broken route.
    let response = send(
        &app,
        axum::http::Method::GET,
        "/api/v1/workflows/runs",
        Some(&token_a),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert!(org_b != org_a, "the two orgs are distinct");
    assert_eq!(
        body_json(response).await["runs"]
            .as_array()
            .expect("runs")
            .len(),
        1,
        "org A still sees its own run"
    );
}
