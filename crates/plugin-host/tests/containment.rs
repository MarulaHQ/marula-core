//! Crash containment and quarantine, against a real worker and a real database.
//!
//! Two different failures live next to each other and must not be confused:
//!
//! - a **worker death** is the mechanism breaking. The parent answers
//!   `plugin_worker_failed`, counts a strike, replaces the worker, and after
//!   three consecutive strikes quarantines the install until an operator clears
//!   it;
//! - a **guest trap** is the plugin answering wrongly. It is a typed refusal with
//!   its own code, the worker survives, and no strike is counted — a plugin that
//!   panics on one input must not be able to stop its own install for every
//!   caller, and an operator must not be paged for a bug the plugin caused.
//!
//! ## Why this binary runs its tests one at a time
//!
//! A strike is produced by killing this process's worker children, and the pool
//! replaces a dead member lazily at the next checkout. Two tests doing that at
//! once would each find the other's children, so the whole test holds
//! [`SERIAL`]. The same lock keeps worker boots apart: three concurrent cold
//! starts of the 52 MB debug binary exceeded the pool's two-second hello window
//! in this lane's gate run, which is written up in `REVIEW.md` as a runtime
//! finding rather than papered over here.
//!
//! ```text
//! cargo test -p plugin-host --test containment -- --ignored
//! ```

mod support;

use std::time::Duration;

use core_domain::entities::plugins::PluginCapability;
use core_store::memory::MemoryStore;
use plugin_host::abi::{InvocationOutcome, InvocationRequest, Refusal};
use plugin_host::isolation::ModuleArtifact;
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig, QUARANTINE_AFTER_FAILURES};
use support::{admin_pool, artifact, seed_install, Seed};
use uuid::Uuid;

/// Held for the duration of each test in this binary.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const NOOP: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (i64.const 0)))
"#;

const TRAP: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (unreachable)
        (i64.const 0)))
"#;

/// A guest that burns its whole allowance in a tight loop, so a test has a
/// window in which to kill the worker that is running it. That is what "the
/// plugin takes its worker down on every call" looks like from outside.
const SPIN: &str = r#"
    (module
      (memory (export "memory") 1)
      (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
      (func (export "marula_free") (param i32) (param i32))
      (func (export "marula_entry") (param i32) (param i32) (result i64)
        (loop $spin (br $spin))
        (i64.const 0)))
"#;

fn request(org_id: Uuid, install_id: Uuid) -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id,
        install_id,
        entry: "plugin.conformance.probe".into(),
        payload: Vec::new(),
        granted: vec![PluginCapability::UiWidgets.as_str().to_string()],
        declared_events: Vec::new(),
        egress: Default::default(),
    }
}

/// The pids of this process's worker children.
///
/// `pgrep -P` is the portable way to ask (macOS `ps` has no parent flag), and
/// each candidate is confirmed by its command line before it is a target. A
/// zombie (empty command line) is skipped: the pool already killed that member
/// and has not reaped it, so it is a corpse rather than a target.
fn worker_children() -> Vec<u32> {
    let me = std::process::id().to_string();
    let output = std::process::Command::new("pgrep")
        .args(["-P", &me])
        .output()
        .expect("pgrep runs");
    let mut workers = Vec::new();
    for pid in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
    {
        let comm = std::process::Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .expect("ps runs");
        let comm = String::from_utf8_lossy(&comm.stdout).trim().to_string();
        if comm.is_empty() {
            continue;
        }
        assert!(
            comm.contains("plugin-worker"),
            "child {pid} ({comm}) is not a plugin worker; refusing to signal it"
        );
        workers.push(pid);
    }
    workers
}

/// Kills every worker this process owns, as a crashed child would die.
fn kill_workers() {
    let pids = worker_children();
    assert!(
        !pids.is_empty(),
        "the pool owns no live worker child to kill; the test cannot simulate a crash"
    );
    for pid in pids {
        let status = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .expect("kill runs");
        assert!(status.success(), "kill -9 {pid} failed");
    }
    // The supervisor has to observe the exit before the next invocation asks for
    // a member, or it would hand out a corpse and the failure would land on the
    // wrong attempt.
    std::thread::sleep(Duration::from_millis(50));
}

/// Runs one invocation and kills its worker while the guest is inside it.
///
/// This is the only way to produce a *consecutive* failure series: the pool
/// replaces a dead member lazily, at the next checkout, so killing an idle
/// child yields one failure and then nothing to kill. Killing mid-flight yields
/// one failure per attempt.
async fn invoke_and_kill(
    pool: &std::sync::Arc<PluginPool>,
    request: &InvocationRequest,
    artifact: &ModuleArtifact,
) -> plugin_host::InvocationResult {
    let task = {
        let pool = std::sync::Arc::clone(pool);
        let request = request.clone();
        let artifact = artifact.clone();
        tokio::spawn(async move { pool.invoke(&request, &artifact).await })
    };

    // Wait for the replacement to exist before killing anything: the spawn
    // happens at checkout, inside the invocation this task just started.
    let mut waited = 0;
    while worker_children().is_empty() {
        assert!(
            waited < 2_000,
            "no worker child appeared within two seconds"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
        waited += 5;
    }
    // Let the frame reach the guest, so the death is mid-invocation rather than
    // a member that died before it was asked anything.
    tokio::time::sleep(Duration::from_millis(20)).await;
    kill_workers();

    task.await.expect("the invocation task")
}

async fn pooled(pool: &sqlx::PgPool) -> std::sync::Arc<PluginPool> {
    PluginPool::start(PoolConfig {
        mode: IsolationMode::Pool,
        workers: 1,
        worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
        health: Some(pool.clone()),
        sandbox_limits: Default::default(),
    })
    .await
    .expect("the pool starts")
}

/// One crash is one failed invocation, the worker is replaced, and a success
/// clears the strike. The install never notices a gap it has to interpret.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_dead_worker_is_one_failed_invocation_and_a_success_clears_the_strike() {
    let _serial = SERIAL.lock().await;
    let admin = admin_pool(5).await;
    let store = MemoryStore::new();
    let module = artifact(NOOP);
    let seed = Seed::widget(
        &format!("crash-{}", Uuid::now_v7().simple()),
        &module,
        &[PluginCapability::UiWidgets],
    );
    let seeded = seed_install(&admin, &store, &seed).await;
    let org_id = seeded.org_id;
    let install_id = seeded.install.id;
    let request = request(org_id, install_id);

    let pool = pooled(&admin).await;
    let first = pool.invoke(&request, &module).await;
    assert_eq!(first.reply.outcome, InvocationOutcome::Ok);
    let dead = first.worker.expect("a pooled reply names its worker");

    kill_workers();
    let failed = pool.invoke(&request, &module).await;
    assert_eq!(failed.reply.outcome, InvocationOutcome::WorkerFailed);
    assert_eq!(failed.reply.reason, Some(Refusal::WorkerFailed));
    assert_eq!(failed.reply.validate(), Ok(()));
    assert_eq!(
        failed.reply.fuel_used, None,
        "a dead worker has no fuel figure to report (ADR 0023 Decision 7)"
    );
    assert_eq!(
        failed.worker.as_deref(),
        Some(dead.as_str()),
        "the audit row names the worker that died"
    );

    let health = pool
        .quarantine_state(org_id, install_id)
        .await
        .expect("health readable")
        .expect("a strike is recorded");
    assert_eq!(health.consecutive_failures, 1);
    assert!(!health.is_quarantined());

    let after = pool.invoke(&request, &module).await;
    assert_eq!(after.reply.outcome, InvocationOutcome::Ok);
    assert_ne!(after.worker.as_deref(), Some(dead.as_str()));
    let health = pool
        .quarantine_state(org_id, install_id)
        .await
        .expect("health readable")
        .expect("the row survives");
    assert_eq!(
        health.consecutive_failures, 0,
        "a successful invocation resets the counter, or a flaky install quarantines"
    );
    pool.shutdown().await;
}

/// The quarantine rule, end to end: three consecutive worker deaths stop the
/// install, the row says why, every later call is refused without a worker, and
/// an operator's clear restores it.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn three_consecutive_worker_deaths_quarantine_and_an_operator_clear_lifts_it() {
    let _serial = SERIAL.lock().await;
    let admin = admin_pool(5).await;
    let store = MemoryStore::new();
    let module = artifact(NOOP);
    let seed = Seed::widget(
        &format!("quarantine-{}", Uuid::now_v7().simple()),
        &module,
        &[PluginCapability::UiWidgets],
    );
    let seeded = seed_install(&admin, &store, &seed).await;
    let org_id = seeded.org_id;
    let install_id = seeded.install.id;
    let request = request(org_id, install_id);

    let pool = pooled(&admin).await;
    // Each attempt is killed while the guest is inside it, so the failures are
    // consecutive by construction and no success ever resets the counter.
    let spinner = artifact(SPIN);

    for strike in 1..=QUARANTINE_AFTER_FAILURES {
        let failed = invoke_and_kill(&pool, &request, &spinner).await;
        assert_eq!(
            failed.reply.outcome,
            InvocationOutcome::WorkerFailed,
            "strike {strike}: {:?}",
            failed.reply.reason
        );
        let health = pool
            .quarantine_state(org_id, install_id)
            .await
            .expect("health readable")
            .expect("a strike is recorded");
        assert_eq!(health.consecutive_failures as u32, strike);
        if strike < QUARANTINE_AFTER_FAILURES {
            assert!(
                !health.is_quarantined(),
                "strike {strike} must not quarantine; the threshold is {QUARANTINE_AFTER_FAILURES}"
            );
        }
    }

    let health = pool
        .quarantine_state(org_id, install_id)
        .await
        .expect("health readable")
        .expect("the row exists");
    assert!(
        health.is_quarantined(),
        "the third consecutive strike stops it"
    );
    assert_eq!(
        health.quarantine_reason.as_deref(),
        Some(Refusal::WorkerFailed.as_str()),
        "the reason is a machine-readable refusal string"
    );

    // Quarantined: refused before a worker exists, so the row names none and the
    // counter stops moving while an operator has the install stopped.
    let refused = pool.invoke(&request, &module).await;
    assert_eq!(refused.reply.outcome, InvocationOutcome::Refused);
    assert_eq!(refused.reply.reason, Some(Refusal::Quarantined));
    assert_eq!(
        refused.worker, None,
        "a quarantined install refuses before a worker is reached"
    );
    let unchanged = pool
        .quarantine_state(org_id, install_id)
        .await
        .expect("health readable")
        .expect("the row survives a refusal");
    assert_eq!(
        unchanged.consecutive_failures as u32, QUARANTINE_AFTER_FAILURES,
        "a refusal must not keep counting"
    );

    // The operator's clear, and the install runs again.
    let cleared = pool
        .clear_quarantine(org_id, install_id, None)
        .await
        .expect("the clear commits");
    assert!(cleared, "a quarantine was lifted");
    let health = pool
        .quarantine_state(org_id, install_id)
        .await
        .expect("health readable")
        .expect("the row survives the clear");
    assert!(!health.is_quarantined());
    assert_eq!(health.consecutive_failures, 0);
    assert!(
        health.cleared_at.is_some(),
        "the clear is attributed in time"
    );
    assert_eq!(health.quarantine_reason, None);

    let after = pool.invoke(&request, &module).await;
    assert_eq!(after.reply.outcome, InvocationOutcome::Ok);
    pool.shutdown().await;
}

/// A guest that traps is a refusal, not a strike. Five traps in a row leave the
/// install healthy and running on the same worker.
#[tokio::test]
#[ignore = "needs Postgres via MARULA_TEST_DATABASE_URL or DATABASE_URL; run with --ignored at the gate"]
async fn a_trapping_guest_is_a_refusal_and_never_a_strike() {
    let _serial = SERIAL.lock().await;
    let admin = admin_pool(5).await;
    let store = MemoryStore::new();
    let module = artifact(TRAP);
    let seed = Seed::widget(
        &format!("trap-{}", Uuid::now_v7().simple()),
        &module,
        &[PluginCapability::UiWidgets],
    );
    let seeded = seed_install(&admin, &store, &seed).await;
    let request = request(seeded.org_id, seeded.install.id);
    let pool = pooled(&admin).await;

    let mut worker = None;
    for _ in 0..(QUARANTINE_AFTER_FAILURES + 2) {
        let result = pool.invoke(&request, &module).await;
        assert_eq!(result.reply.outcome, InvocationOutcome::Refused);
        assert_eq!(result.reply.reason, Some(Refusal::GuestTrap));
        assert!(
            result.reply.fuel_used.is_some(),
            "a refusal reports the fuel it spent"
        );
        match &worker {
            None => worker = result.worker.clone(),
            Some(first) => assert_eq!(
                result.worker.as_deref(),
                Some(first.as_str()),
                "the worker that ran the trap is the worker that runs the next one"
            ),
        }
    }

    let health = pool
        .quarantine_state(seeded.org_id, seeded.install.id)
        .await
        .expect("health readable");
    assert!(
        health.as_ref().map(|row| row.is_quarantined()) != Some(true),
        "a guest trap never quarantines"
    );

    // And the install still runs an honest module afterwards.
    let honest = artifact(NOOP);
    assert_eq!(
        pool.invoke(&request, &honest).await.reply.outcome,
        InvocationOutcome::Ok
    );
    pool.shutdown().await;
}
