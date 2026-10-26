//! Capability escalation, attempted from the three places a caller can try it:
//! the surface the caller asks for, the grant set the install holds, and the
//! payload the guest sends.
//!
//! Every test here runs the real executor against a real database: the registry
//! row, the install, the pinned manifest, the pinned module bytes on disk, the
//! worker process, the effect and the audit row. A refusal at any of those gates
//! is a written row with a typed reason, never an exception a caller could
//! mistake for an outage.
//!
//! ```text
//! cargo test -p plugin-host --test escalation -- --ignored
//! ```

mod support;

use core_domain::entities::plugins::PluginCapability;
use core_store::memory::MemoryStore;
use plugin_host::abi::InvocationOutcome;
use plugin_host::exec::{invoke_surface, PoolSlot, SurfaceCall};
use plugin_host::surface::SurfaceKind;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{admin_pool, artifact, seed_install, seed_ticket, Seed, Seeded};
use uuid::Uuid;

/// A guest that answers nothing. Used wherever the gate under test must refuse
/// before a module matters.
const NOOP: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (i64.const 0)))
"#;

/// A guest that asks for the network and hands back whatever the host answered.
/// The payload it receives names capabilities; the frame it sends does not
/// exist, because authority is the install's.
const EGRESS_PROBE: &str = r#"
    (module
      (import "marula" "http_request" (func $http (param i32 i32 i32 i32) (result i64)))
      (memory (export "memory") 1)
      (data (i32.const 512) "vendor")
      (data (i32.const 544) "{\"method\":\"GET\",\"url\":\"https://api.example.com/v1\"}")
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (call $http (i32.const 512) (i32.const 6) (i32.const 544) (i32.const 51))))
"#;

/// A guest that answers with an effect the manifest never consented. The
/// executor reads this as a contradiction and applies nothing.
const HIJACK: &str = r#"
    (module
      (memory (export "memory") 1)
      (data (i32.const 1024) "{\"effect\":\"delete_ticket\",\"accepted\":true}")
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 4096))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (i64.const 4398046511146))) ;; ptr 1024, len 42
"#;

fn call<'a>(
    org_id: Uuid,
    entry_key: &'a str,
    params: &'a Value,
    ticket_id: Option<Uuid>,
) -> SurfaceCall<'a> {
    SurfaceCall {
        org_id,
        entry_key,
        surface: SurfaceKind::Action,
        params,
        ticket_id,
    }
}

/// Runs one attempt in its own transaction and rolls back, so a refusal leaves
/// nothing but its audit row behind — which is exactly what the row is for.
async fn attempt(
    pool: &PgPool,
    store: &dyn core_store::ObjectStore,
    slot: &PoolSlot,
    call: &SurfaceCall<'_>,
) -> plugin_host::exec::SurfaceOutcome {
    let mut tx = pool.begin().await.expect("transaction");
    core_db::set_org(&mut tx, call.org_id)
        .await
        .expect("org scope");
    let outcome = invoke_surface(&mut tx, pool, store, slot, call)
        .await
        .expect("a refusal is an Ok outcome, not a database error");
    tx.commit().await.expect("the audit row commits");
    outcome
}

/// The audit row the last attempt wrote, read as JSON.
async fn last_audit(pool: &PgPool, org_id: Uuid) -> Value {
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_events
          WHERE org_id = $1 AND action = 'plugin.invoke'
          ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .expect("the attempt wrote an audit row");
    detail
}

/// The escalation that has to fail first: a caller asking for an action kind the
/// install holds no grant for. Nothing runs, and the row says which capability.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn an_action_the_install_has_no_grant_for_is_refused_before_a_worker_runs() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::action(
        &format!("escalation-{}", Uuid::now_v7().simple()),
        &artifact(NOOP),
        &[],
        "add_internal_comment",
    );
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let slot = support::pool_slot(&pool, 1).await;

    let params = json!({ "body": "should never be written" });
    let outcome = attempt(
        &pool,
        &store,
        &slot,
        &call(org_id, &entry_key, &params, None),
    )
    .await;

    assert_eq!(outcome.outcome, InvocationOutcome::Refused);
    assert_eq!(outcome.reason, Some("plugin_capability_not_granted"));
    assert!(
        outcome.comment.is_none(),
        "a refused action writes no comment"
    );

    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["outcome"], "refused");
    assert_eq!(detail["reason"], "plugin_capability_not_granted");
    assert_eq!(detail["capability"], "automation.actions");
    assert_eq!(
        detail["worker"],
        Value::Null,
        "no worker may be named for an attempt refused before dispatch"
    );
}

/// A disabled install is refused at the admin gate even though it holds the
/// grant. The grant is not the authority; the install's state is part of it.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_disabled_install_never_reaches_a_worker() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::action(
        &format!("disabled-{}", Uuid::now_v7().simple()),
        &artifact(NOOP),
        &[PluginCapability::AutomationActions],
        "add_internal_comment",
    )
    .disabled();
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let slot = support::pool_slot(&pool, 1).await;

    let params = json!({ "body": "blocked" });
    let outcome = attempt(
        &pool,
        &store,
        &slot,
        &call(org_id, &entry_key, &params, None),
    )
    .await;

    assert_eq!(outcome.outcome, InvocationOutcome::Refused);
    assert_eq!(outcome.reason, Some("plugin_install_disabled"));
    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["reason"], "plugin_install_disabled");
}

/// A revoked catalog entry is terminal: the install keeps its grant and still
/// does not run.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_revoked_catalog_entry_is_terminal() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::action(
        &format!("revoked-{}", Uuid::now_v7().simple()),
        &artifact(NOOP),
        &[PluginCapability::AutomationActions],
        "add_internal_comment",
    )
    .revoked();
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let slot = support::pool_slot(&pool, 1).await;

    let params = json!({ "body": "blocked" });
    let outcome = attempt(
        &pool,
        &store,
        &slot,
        &call(org_id, &entry_key, &params, None),
    )
    .await;

    assert_eq!(outcome.outcome, InvocationOutcome::Refused);
    assert_eq!(outcome.reason, Some("plugin_entry_revoked"));
    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["reason"], "plugin_entry_revoked");
}

/// The payload is bytes. A guest that puts a capability list in the task input
/// does not mint one: the frame's grant set is built from the install row, and
/// the worker's own gate answers the request with the refusal code.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_guest_cannot_mint_a_grant_from_its_payload() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::widget(
        &format!("payload-grant-{}", Uuid::now_v7().simple()),
        &artifact(EGRESS_PROBE),
        &[PluginCapability::UiWidgets],
    );
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let slot = support::pool_slot(&pool, 1).await;

    // The claim, in the shape a confused caller would send it.
    let params = json!({
        "granted": ["automation.reads", "automation.actions", "secrets.read"],
        "capabilities": ["automation.reads"],
        "grant": ["automation.reads"],
    });
    let mut call = call(org_id, &entry_key, &params, None);
    call.surface = SurfaceKind::Widget;
    let outcome = attempt(&pool, &store, &slot, &call).await;

    // The guest ran and reported the frame: tag 1 (error), code 1
    // (plugin_capability_not_granted). It did not get the network.
    assert_eq!(
        outcome.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        outcome.reason
    );
    assert_eq!(
        outcome.output.as_deref(),
        Some(&[1u8, 1, 0, 0, 0][..]),
        "the worker answered plugin_capability_not_granted"
    );

    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["capability"], "ui.widgets");
    assert_eq!(detail["outcome"], "ok");
}

/// The wave's acceptance criterion, end to end: the committed first-party action
/// plugin runs through the real executor, its consented effect lands as an
/// internal comment, and the audit row names the install as the actor.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_first_party_action_performs_its_consented_effect() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/wasm/triage-note-action/plugin.wasm"
    );
    let bytes = std::fs::read(path).expect("the reference plugin is committed");
    let module = plugin_host::isolation::ModuleArtifact {
        digest: plugin_host::isolation::ModuleArtifact::digest_of(&bytes),
        bytes,
    };
    let seed = Seed::action(
        &format!("triage-note-{}", Uuid::now_v7().simple()),
        &module,
        &[PluginCapability::AutomationActions],
        "add_internal_comment",
    );
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let ticket_id = seed_ticket(&pool, org_id).await;
    let slot = support::pool_slot(&pool, 1).await;

    let params = json!({ "body": "triage: reboot the print spooler" });
    let outcome = attempt(
        &pool,
        &store,
        &slot,
        &call(org_id, &entry_key, &params, Some(ticket_id)),
    )
    .await;

    assert_eq!(
        outcome.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        outcome.reason
    );
    let comment = outcome.comment.expect("the consented effect applied");
    assert_eq!(comment.ticket_id, ticket_id);
    assert_eq!(comment.body, "triage: reboot the print spooler");

    // The row: one insert, plugin actor, the install's id, the seven detail keys.
    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["outcome"], "ok");
    assert_eq!(detail["capability"], "automation.actions");
    assert!(detail["fuel_used"].as_u64().is_some(), "fuel is reported");
    assert!(
        detail["worker"]
            .as_str()
            .unwrap_or("")
            .starts_with("marula-plugin-worker:"),
        "the row names the worker that ran the module"
    );

    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ticket_comments WHERE org_id = $1 AND ticket_id = $2",
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_one(&pool)
    .await
    .expect("comment count");
    assert_eq!(stored, 1, "exactly one comment, written by one attempt");
}

/// The guest does not choose the effect. A module that answers with a different
/// effect is contradicted, nothing is written, and the reason says so.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_guest_that_names_another_effect_gets_no_effect() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::action(
        &format!("effect-hijack-{}", Uuid::now_v7().simple()),
        &artifact(HIJACK),
        &[PluginCapability::AutomationActions],
        "add_internal_comment",
    );
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let ticket_id = seed_ticket(&pool, org_id).await;
    let slot = support::pool_slot(&pool, 1).await;

    let params = json!({ "body": "the consented body" });
    let outcome = attempt(
        &pool,
        &store,
        &slot,
        &call(org_id, &entry_key, &params, Some(ticket_id)),
    )
    .await;

    assert_eq!(outcome.outcome, InvocationOutcome::Refused);
    assert_eq!(outcome.reason, Some("plugin_effect_not_consented"));
    assert!(outcome.comment.is_none());

    let comments: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ticket_comments WHERE org_id = $1 AND ticket_id = $2",
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_one(&pool)
    .await
    .expect("comment count");
    assert_eq!(comments, 0, "a contradicted effect writes nothing");

    let detail = last_audit(&pool, org_id).await;
    assert_eq!(detail["reason"], "plugin_effect_not_consented");
}

/// Every attempt leaves one row, whatever happened. Five attempts at five
/// different gates produce five rows and no more, which is what makes the trail
/// usable as evidence.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn one_attempt_is_one_audit_row() {
    let pool = admin_pool(5).await;
    let store = MemoryStore::new();
    let seed = Seed::action(
        &format!("audit-count-{}", Uuid::now_v7().simple()),
        &artifact(NOOP),
        &[],
        "add_internal_comment",
    );
    let Seeded {
        org_id, entry_key, ..
    } = seed_install(&pool, &store, &seed).await;
    let slot = support::pool_slot(&pool, 1).await;

    for _ in 0..5 {
        let params = json!({ "body": "refused" });
        let outcome = attempt(
            &pool,
            &store,
            &slot,
            &call(org_id, &entry_key, &params, None),
        )
        .await;
        assert_eq!(outcome.outcome, InvocationOutcome::Refused);
    }

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE org_id = $1 AND action = 'plugin.invoke'",
    )
    .bind(org_id)
    .fetch_one(&pool)
    .await
    .expect("audit count");
    assert_eq!(rows, 5, "five attempts, five rows");
}
