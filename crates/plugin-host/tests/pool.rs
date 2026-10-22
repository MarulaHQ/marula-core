//! The pool against a REAL worker process: the reference plugin, a killed
//! worker, a runaway guest, the install rate bucket, and the two transports.
//!
//! These are integration tests because the thing under test is a child process
//! and a pipe. `cargo` builds `marula-plugin-worker` for this target and hands
//! its path over in `CARGO_BIN_EXE_<name>`.

use std::time::{Duration, Instant};

use plugin_host::abi::{InvocationOutcome, InvocationRequest, Refusal, MAX_INPUT_BYTES};
use plugin_host::isolation::{ModuleArtifact, SandboxLimits};
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig, PoolError};
use plugin_host::wire;
use plugin_host::{InvocationResult, PluginRuntime};
use uuid::Uuid;

fn worker_bin() -> &'static str {
    env!("CARGO_BIN_EXE_marula-plugin-worker")
}

fn pool_config(mode: IsolationMode) -> PoolConfig {
    PoolConfig {
        mode,
        workers: 1,
        worker_bin: Some(worker_bin().into()),
        health: None,
        sandbox_limits: SandboxLimits::default(),
    }
}

async fn pool() -> std::sync::Arc<PluginPool> {
    PluginPool::start(pool_config(IsolationMode::Pool))
        .await
        .expect("the pool starts")
}

/// The committed reference plugin, byte for byte: the same artifact an operator
/// installs, not a fixture that could drift from it.
fn reference_plugin() -> ModuleArtifact {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/wasm/escalation-widget/plugin.wasm"
    );
    let bytes = std::fs::read(path).expect("the reference plugin is committed");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn artifact_from_wat(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn request(install_id: Uuid) -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        install_id,
        entry: "plugin.escalation-widget.escalation_policy".into(),
        payload: br#"{"ticket":"01J"}"#.to_vec(),
        granted: vec!["ui.widgets".into()],
        declared_events: Vec::new(),
        egress: Default::default(),
    }
}

fn install() -> Uuid {
    Uuid::now_v7()
}

/// A guest that will not stop is stopped by whichever cap comes first. For a
/// pure spin that is fuel: 200,000,000 fuel is about 200 ms of Cranelift code,
/// which lands before the 250 ms epoch — the ADR's own arithmetic. The epoch's
/// own path is proven in `isolation`'s unit test, where the limits are narrow.
fn is_a_limit_refusal(reason: Option<Refusal>) -> bool {
    matches!(
        reason,
        Some(Refusal::FuelExhausted) | Some(Refusal::WallClockExceeded)
    )
}

fn output_text(result: &InvocationResult) -> String {
    String::from_utf8(
        result
            .reply
            .output
            .clone()
            .expect("the reference plugin returns its props"),
    )
    .expect("the props are UTF-8")
}

#[tokio::test]
async fn a_pool_runs_the_reference_plugin_in_a_real_worker() {
    let pool = pool().await;
    let result = pool.invoke(&request(install()), &reference_plugin()).await;

    assert_eq!(result.reply.outcome, InvocationOutcome::Ok);
    assert_eq!(result.reply.validate(), Ok(()), "{:?}", result.reply);
    assert_eq!(result.reply.invocation_id, result.reply.invocation_id);
    assert!(
        result.reply.fuel_used.expect("ok replies carry fuel") > 0,
        "a real guest burns fuel"
    );
    assert!(output_text(&result).contains("escalation_policy"));
    let worker = result.worker.expect("a pool result names its worker");
    assert!(
        worker.starts_with("marula-plugin-worker:"),
        "worker label was {worker}"
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn a_dead_worker_yields_a_reply_and_a_replacement() {
    let pool = pool().await;
    let plugin = reference_plugin();

    let first = pool.invoke(&request(install()), &plugin).await;
    assert_eq!(first.reply.outcome, InvocationOutcome::Ok);
    let worker = first.worker.clone().expect("worker label");
    let pid: u32 = worker
        .rsplit(':')
        .next()
        .and_then(|pid| pid.parse().ok())
        .expect("the label ends in a pid");

    // The one way to reach the worker-failure path from outside: kill the child.
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("kill runs");
    assert!(killed.success(), "kill -9 {pid} failed");

    let second = pool.invoke(&request(install()), &plugin).await;
    assert_eq!(
        second.reply.outcome,
        InvocationOutcome::WorkerFailed,
        "{:?}",
        second.reply
    );
    assert_eq!(second.reply.reason, Some(Refusal::WorkerFailed));
    assert_eq!(
        second.reply.fuel_used, None,
        "a dead worker has no fuel figure to report (Decision 7)"
    );
    assert_eq!(second.reply.validate(), Ok(()));
    assert_eq!(
        second.worker.as_deref(),
        Some(worker.as_str()),
        "the audit row names the worker that died"
    );

    // Crash containment: the next invocation gets a fresh worker and succeeds.
    let third = pool.invoke(&request(install()), &plugin).await;
    assert_eq!(
        third.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        third.reply
    );
    assert_ne!(third.worker, second.worker);
    pool.shutdown().await;
}

#[tokio::test]
async fn a_runaway_guest_is_stopped_and_its_worker_survives_it() {
    let pool = pool().await;
    let spinner = artifact_from_wat(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (loop $spin (br $spin))
            (i64.const 0)))
        "#,
    );

    let started = Instant::now();
    let runaway = pool.invoke(&request(install()), &spinner).await;
    assert_eq!(runaway.reply.outcome, InvocationOutcome::Refused);
    assert!(
        is_a_limit_refusal(runaway.reply.reason),
        "{:?}",
        runaway.reply.reason
    );
    assert!(
        started.elapsed() < Duration::from_millis(2_000),
        "the epoch deadline is 250 ms; {:?} means it did not fire",
        started.elapsed()
    );

    let after = pool.invoke(&request(install()), &reference_plugin()).await;
    assert_eq!(
        after.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        after.reply
    );
    assert_eq!(
        after.worker, runaway.worker,
        "a guest that burns its wall clock must not take the worker with it"
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn an_artifact_whose_bytes_do_not_match_its_digest_is_refused_by_the_worker() {
    let pool = pool().await;
    let mut tampered = reference_plugin();
    tampered.bytes.push(0);

    let result = pool.invoke(&request(install()), &tampered).await;
    assert_eq!(result.reply.outcome, InvocationOutcome::Refused);
    assert_eq!(result.reply.reason, Some(Refusal::FrameInvalid));
    assert!(
        result.worker.is_some(),
        "the worker answered; it did not die"
    );

    // And it is still healthy for the honest artifact.
    let honest = pool.invoke(&request(install()), &reference_plugin()).await;
    assert_eq!(honest.reply.outcome, InvocationOutcome::Ok);
    pool.shutdown().await;
}

#[tokio::test]
async fn frames_that_never_reach_a_worker_are_refused_with_no_worker_label() {
    let pool = pool().await;

    let mut oversized = request(install());
    oversized.payload = vec![0; MAX_INPUT_BYTES + 1];
    let result = pool.invoke(&oversized, &reference_plugin()).await;
    assert_eq!(result.reply.reason, Some(Refusal::InputTooLarge));
    assert_eq!(result.worker, None);
    assert_eq!(result.reply.validate(), Ok(()));

    let mut bad_digest = reference_plugin();
    bad_digest.digest = "not-a-sha256".into();
    let result = pool.invoke(&request(install()), &bad_digest).await;
    assert_eq!(result.reply.reason, Some(Refusal::FrameInvalid));
    assert_eq!(result.worker, None);

    // A grant the closed vocabulary does not define never leaves this process.
    let mut smuggled = request(install());
    smuggled.granted = vec!["sudo.everything".into()];
    let result = pool.invoke(&smuggled, &reference_plugin()).await;
    assert_eq!(result.reply.reason, Some(Refusal::FrameInvalid));
    assert_eq!(result.worker, None);
    pool.shutdown().await;
}

#[tokio::test]
async fn the_install_rate_bucket_refuses_sustained_load() {
    let pool = pool().await;
    let plugin = reference_plugin();
    let request = request(install());

    let mut first_refusal = None;
    for index in 0..90 {
        let result = pool.invoke(&request, &plugin).await;
        if result.reply.reason == Some(Refusal::RateLimited) {
            first_refusal = Some(index);
            break;
        }
    }
    let first_refusal = first_refusal.expect("a sustained loop must be refused");
    assert!(
        first_refusal >= usize::try_from(plugin_host::abi::INVOCATION_BURST_PER_INSTALL).unwrap(),
        "the burst of {} was served before the refusal at {first_refusal}",
        plugin_host::abi::INVOCATION_BURST_PER_INSTALL
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn a_disabled_runtime_refuses_everything_without_spawning_a_worker() {
    let pool = PluginPool::start(pool_config(IsolationMode::Disabled))
        .await
        .expect("a disabled pool starts without a worker");
    assert!(!pool.enabled());
    let result = pool.invoke(&request(install()), &reference_plugin()).await;
    assert_eq!(result.reply.outcome, InvocationOutcome::Refused);
    assert_eq!(result.reply.reason, Some(Refusal::RuntimeDisabled));
    assert_eq!(result.worker, None);
}

#[tokio::test]
async fn in_process_runs_the_same_plugin_under_the_same_limits() {
    let pool = PluginPool::start(pool_config(IsolationMode::InProcess))
        .await
        .expect("in-process sandbox builds");
    let result = pool.invoke(&request(install()), &reference_plugin()).await;
    assert_eq!(
        result.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        result.reply
    );
    assert_eq!(result.worker.as_deref(), Some("in-process"));
    assert!(output_text(&result).contains("escalation_policy"));

    let spinner = artifact_from_wat(
        r#"
        (module
          (memory (export "memory") 1)
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (loop $spin (br $spin))
            (i64.const 0)))
        "#,
    );
    let runaway = pool.invoke(&request(install()), &spinner).await;
    assert!(
        is_a_limit_refusal(runaway.reply.reason),
        "{:?}",
        runaway.reply.reason
    );
}

#[tokio::test]
async fn a_pool_without_its_worker_binary_fails_to_start() {
    let error = PluginPool::start(
        PoolConfig::default().with_worker_bin("/nonexistent/marula-plugin-worker"),
    )
    .await
    .expect_err("a missing worker must fail the boot");
    assert!(
        matches!(error, PoolError::WorkerUnavailable { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn a_binary_that_does_not_speak_the_protocol_fails_to_start() {
    // `/bin/cat` starts, stays alive and answers nothing: exactly the wrong
    // binary. The pool must refuse it at boot, not hang the first invocation.
    let error = PluginPool::start(PoolConfig::default().with_worker_bin("/bin/cat"))
        .await
        .expect_err("a foreign binary must fail the boot");
    assert!(
        matches!(error, PoolError::WorkerUnavailable { .. }),
        "{error}"
    );
}

const _: () = assert!(
    wire::MAX_FRAME_BYTES > wire::MAX_MODULE_BYTES + MAX_INPUT_BYTES,
    "a full module plus a full payload must fit"
);

// ---------------------------------------------------------------------------
// The host-call bridge (W53-11): a guest's KV verbs, served by the parent
// ---------------------------------------------------------------------------

/// A `HostCallService` backed by `MemoryKvStore` — the parent's half of the
/// bridge, with the quota rules and without a database. The shipped service is
/// `exec::TransactionHostCalls` on the invocation's transaction; this one exists
/// so the transport (two frames, two correlators, the cap) is provable in a test
/// that needs no stack.
struct MemoryKvService {
    store: plugin_host::kv::MemoryKvStore,
}

impl plugin_host::runtime::HostCallService for MemoryKvService {
    fn serve<'a>(
        &'a mut self,
        context: plugin_host::runtime::HostCallContext,
        op: wire::HostCallOp,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = wire::HostCallOutcome> + Send + 'a>>
    {
        use plugin_host::kv::KvStore;
        Box::pin(async move {
            let org = context.org_id;
            let install = context.install_id;
            let result = match op {
                wire::HostCallOp::KvGet { namespace, key } => self
                    .store
                    .get(org, install, &namespace, &key)
                    .map(|value| match value {
                        Some(value) => wire::HostCallValue::Value(value),
                        None => wire::HostCallValue::Absent,
                    }),
                wire::HostCallOp::KvPut {
                    namespace,
                    key,
                    value,
                    expires_at,
                } => self
                    .store
                    .put(org, install, &namespace, &key, &value, expires_at)
                    .map(|()| wire::HostCallValue::Unit),
                wire::HostCallOp::KvDelete { namespace, key } => self
                    .store
                    .delete(org, install, &namespace, &key)
                    .map(wire::HostCallValue::Existed),
                wire::HostCallOp::KvList {
                    namespace,
                    prefix,
                    cursor,
                } => self
                    .store
                    .list(org, install, &namespace, &prefix, cursor.as_deref())
                    .map(|page| wire::HostCallValue::Page {
                        keys: page.keys,
                        cursor: page.next_cursor,
                    }),
            };
            match result {
                Ok(value) => wire::HostCallOutcome::Ok(value),
                Err(plugin_host::kv::KvError::QuotaExceeded) => {
                    wire::HostCallOutcome::Refused(wire::HostCallRefusal::QuotaExceeded)
                }
                Err(_) => wire::HostCallOutcome::Refused(wire::HostCallRefusal::Failed),
            }
        })
    }
}

fn memory_service() -> MemoryKvService {
    MemoryKvService {
        store: plugin_host::kv::MemoryKvStore::new(),
    }
}

/// A guest that stores a value with `kv_put` and then reads it back with
/// `kv_get`, returning the `kv_get` reply frame as its own output.
///
/// Layout in linear memory: namespace "cache" at 0, key "k" at 16, value
/// "bridged" at 32. `marula_alloc` hands out fixed space from 1024, which is
/// where the host writes the data frame the guest returns.
fn kv_round_trip_guest() -> ModuleArtifact {
    artifact_from_wat(
        r#"
        (module
          (import "marula" "kv_put" (func $put (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "marula" "kv_get" (func $get (param i32 i32 i32 i32) (result i64)))
          (memory (export "memory") 1)
          (data (i32.const 0) "cache")
          (data (i32.const 16) "k")
          (data (i32.const 32) "bridged")
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            ;; A refused put is a failed test, not a quiet zero.
            (if (i32.ne
                  (call $put (i32.const 0) (i32.const 5)
                             (i32.const 16) (i32.const 1)
                             (i32.const 32) (i32.const 7))
                  (i32.const 0))
                (then (unreachable)))
            (call $get (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 1))))
        "#,
    )
}

#[tokio::test]
async fn a_guest_reads_through_the_parent_bridge_in_the_pool_profile() {
    let pool = pool().await;
    let mut service = memory_service();
    let request = request(install());
    let result = pool
        .invoke_with(&request, &kv_round_trip_guest(), &mut service)
        .await;

    assert_eq!(
        result.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        result.reply
    );
    let output = result.reply.output.clone().expect("a data frame");
    // The host's in-band frame: `svc::ok_frame` wraps the value, so the bytes
    // the guest got back are the value it stored, not an empty answer and not a
    // refusal.
    let value = plugin_host::abi::parse_frame(&output).expect("an ok frame");
    assert_eq!(value, b"bridged");
    let worker = result.worker.expect("a pool result names its worker");
    assert!(worker.starts_with("marula-plugin-worker:"));
    pool.shutdown().await;
}

/// A guest that calls `kv_put` 70 times and returns a 4-byte output: the number
/// of calls the parent refused. 64 are served, the rest are past
/// `MAX_HOST_CALLS_PER_INVOCATION`.
fn kv_loop_guest(calls: u32) -> ModuleArtifact {
    artifact_from_wat(&format!(
        r#"
        (module
          (import "marula" "kv_put" (func $put (param i32 i32 i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (data (i32.const 0) "cache")
          (data (i32.const 16) "k")
          (data (i32.const 32) "v")
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (local $left i32)
            (local $refused i32)
            (local.set $left (i32.const {calls}))
            (block $done
              (loop $again
                (br_if $done (i32.eqz (local.get $left)))
                (if (i32.ne
                      (call $put (i32.const 0) (i32.const 5)
                                 (i32.const 16) (i32.const 1)
                                 (i32.const 32) (i32.const 1))
                      (i32.const 0))
                    (then (local.set $refused (i32.add (local.get $refused) (i32.const 1)))))
                (local.set $left (i32.sub (local.get $left) (i32.const 1)))
                (br $again)))
            ;; 4 raw bytes at 1024 = the refusal count, little-endian.
            (i32.store (i32.const 1024) (local.get $refused))
            ;; `abi::pack`: the pointer is the high half, the length the low one.
            (i64.or
              (i64.shl (i64.const 1024) (i64.const 32))
              (i64.const 4))))
        "#
    ))
}

#[tokio::test]
async fn a_guest_that_ignores_the_host_call_cap_is_refused_and_cannot_extend_its_budget() {
    let pool = pool().await;
    let mut service = memory_service();
    let calls = u32::try_from(plugin_host::runtime::MAX_HOST_CALLS_PER_INVOCATION + 6).unwrap();
    let result = pool
        .invoke_with(&request(install()), &kv_loop_guest(calls), &mut service)
        .await;

    assert_eq!(
        result.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        result.reply
    );
    let output = result.reply.output.clone().expect("a 4-byte count");
    assert_eq!(output.len(), 4);
    let refused = u32::from_le_bytes([output[0], output[1], output[2], output[3]]);
    assert_eq!(
        refused,
        6,
        "{calls} calls against a cap of {} must serve exactly the cap",
        plugin_host::runtime::MAX_HOST_CALLS_PER_INVOCATION
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn a_guest_deletes_and_lists_through_the_same_bridge() {
    let pool = pool().await;
    let mut service = memory_service();

    // put → delete → get: the get must be the empty ok frame, which is how the
    // ABI says "absent".
    let deleted = pool
        .invoke_with(&request(install()), &kv_delete_guest(), &mut service)
        .await;
    assert_eq!(
        deleted.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        deleted.reply
    );
    let output = deleted.reply.output.expect("a frame");
    let value = plugin_host::abi::parse_frame(&output).expect("an ok frame");
    assert!(
        value.is_empty(),
        "the key was deleted, so kv_get answers the empty value"
    );

    // put → list: the page is the PDK's JSON and carries the key.
    let listed = pool
        .invoke_with(&request(install()), &kv_list_guest(), &mut service)
        .await;
    assert_eq!(
        listed.reply.outcome,
        InvocationOutcome::Ok,
        "{:?}",
        listed.reply
    );
    let output = listed.reply.output.expect("a frame");
    let page = plugin_host::abi::parse_frame(&output).expect("an ok frame");
    let page: serde_json::Value = serde_json::from_slice(page).expect("the documented JSON page");
    assert_eq!(page["keys"], serde_json::json!(["a1", "a2"]));
    assert_eq!(page["cursor"], "");
    pool.shutdown().await;
}

/// put, then delete, then get the same key.
fn kv_delete_guest() -> ModuleArtifact {
    artifact_from_wat(
        r#"
        (module
          (import "marula" "kv_put" (func $put (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "marula" "kv_delete" (func $del (param i32 i32 i32 i32) (result i32)))
          (import "marula" "kv_get" (func $get (param i32 i32 i32 i32) (result i64)))
          (memory (export "memory") 1)
          (data (i32.const 0) "cache")
          (data (i32.const 16) "k")
          (data (i32.const 32) "v")
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (if (i32.ne
                  (call $put (i32.const 0) (i32.const 5)
                             (i32.const 16) (i32.const 1)
                             (i32.const 32) (i32.const 1))
                  (i32.const 0))
                (then (unreachable)))
            (if (i32.ne (call $del (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 1))
                        (i32.const 0))
                (then (unreachable)))
            (call $get (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 1))))
        "#,
    )
}

/// put two keys, then list the namespace under the empty prefix.
fn kv_list_guest() -> ModuleArtifact {
    artifact_from_wat(
        r#"
        (module
          (import "marula" "kv_put" (func $put (param i32 i32 i32 i32 i32 i32) (result i32)))
          (import "marula" "kv_list" (func $list (param i32 i32 i32 i32 i32 i32) (result i64)))
          (memory (export "memory") 1)
          (data (i32.const 0) "cache")
          (data (i32.const 16) "a1")
          (data (i32.const 32) "a2")
          (data (i32.const 48) "v")
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (if (i32.ne
                  (call $put (i32.const 0) (i32.const 5)
                             (i32.const 16) (i32.const 2)
                             (i32.const 48) (i32.const 1))
                  (i32.const 0))
                (then (unreachable)))
            (if (i32.ne
                  (call $put (i32.const 0) (i32.const 5)
                             (i32.const 32) (i32.const 2)
                             (i32.const 48) (i32.const 1))
                  (i32.const 0))
                (then (unreachable)))
            ;; Empty prefix and empty cursor: a legal empty label each.
            (call $list (i32.const 0) (i32.const 5) (i32.const 0) (i32.const 0)
                        (i32.const 0) (i32.const 0))))
        "#,
    )
}

#[tokio::test]
async fn a_pool_with_no_service_answers_kv_as_unavailable_rather_than_guessing() {
    // The plain `invoke` entry point serves no host calls. A guest that asks is
    // told `plugin_not_implemented` — the honest answer — and its invocation
    // still succeeds if it ignores the refusal needed to. This guest traps on a
    // refused put, so the outcome is a guest trap, which is exactly the point:
    // the refusal reached the guest.
    let pool = pool().await;
    let result = pool
        .invoke(&request(install()), &kv_round_trip_guest())
        .await;
    assert_eq!(result.reply.outcome, InvocationOutcome::Refused);
    assert_eq!(result.reply.reason, Some(Refusal::GuestTrap));
    pool.shutdown().await;
}
