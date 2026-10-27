//! The four limit bombs, each pinned to the cap that has to stop it.
//!
//! Every guard lives in `isolation`, so a bomb is not an attack on a defence
//! that has a second copy: the test narrows one limit at a time through
//! [`PoolConfig::sandbox_limits`] (the `in_process` transport, same executor and
//! same gates) and asserts the exact refusal, then asserts the process is still
//! healthy and the next honest invocation answers.
//!
//! Fire: a spin loop (fuel), the same loop with fuel priced out of reach (wall
//! clock), a growth loop (memory), a claimed length (output), and the log
//! budget, which has no refusal channel and is proven by what is captured.

use std::sync::Arc;

use plugin_host::abi::{self, InvocationOutcome, InvocationRequest, Refusal};
use plugin_host::host_fns::register_all;
use plugin_host::isolation::{ModuleArtifact, Sandbox, SandboxLimits};
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use uuid::Uuid;

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../plugins/wasm/golden/src");

fn guest(name: &str) -> ModuleArtifact {
    let path = format!("{CORPUS}/{name}.wat");
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    let bytes = wat::parse_str(&source).unwrap_or_else(|error| panic!("{path}: {error}"));
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

fn request() -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        install_id: Uuid::now_v7(),
        entry: "plugin.golden.bomb".into(),
        payload: Vec::new(),
        granted: Vec::new(),
        declared_events: Vec::new(),
        egress: Default::default(),
    }
}

/// One in-process runtime at the given limits. `in_process` is the transport
/// that carries a narrowed limit; a pooled worker always runs the shipped
/// numbers, which is why the shipped numbers get their own assertions further
/// down and in `conformance.rs`.
async fn in_process(limits: SandboxLimits) -> Arc<PluginPool> {
    PluginPool::start(PoolConfig {
        mode: IsolationMode::InProcess,
        workers: 1,
        worker_bin: None,
        health: None,
        sandbox_limits: limits,
    })
    .await
    .expect("the in-process runtime starts")
}

/// The one shape every bomb has: an exact refusal, inside the budget it was
/// given, and a runtime that answers the next honest call.
async fn bomb(
    pool: &Arc<PluginPool>,
    name: &str,
    expected: Refusal,
    fuel_budget: u64,
) -> InvocationRequest {
    let result = pool.invoke(&request(), &guest(name)).await;
    assert_eq!(
        result.reply.outcome,
        InvocationOutcome::Refused,
        "[{name}] {:?}",
        result.reply
    );
    assert_eq!(result.reply.reason, Some(expected), "[{name}]");
    let fuel = result.reply.fuel_used.expect("a refusal reports its fuel");
    assert!(
        fuel <= fuel_budget,
        "[{name}] spent {fuel} fuel of a {fuel_budget} budget"
    );

    request()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spin_loop_is_stopped_by_fuel_and_the_next_call_answers() {
    let fuel = 50_000;
    let pool = in_process(SandboxLimits {
        fuel,
        // The clock cannot win this race, so the refusal is the fuel cap's and
        // nothing else's.
        wall_clock_ms: 60_000,
        ..SandboxLimits::default()
    })
    .await;

    let mut next = bomb(&pool, "spin", Refusal::FuelExhausted, fuel).await;
    next.entry = "plugin.golden.noop".into();
    let after = pool.invoke(&next, &guest("noop")).await;
    assert_eq!(
        after.reply.outcome,
        InvocationOutcome::Ok,
        "a runtime that survives one bomb must answer the next call"
    );
    pool.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spin_loop_is_stopped_by_the_clock_when_fuel_is_out_of_reach() {
    let wall_clock_ms = 20;
    let pool = in_process(SandboxLimits {
        // Priced so the fuel cap cannot bind inside the window: 2^62 fuel is
        // years of compiled code, and the epoch fires at 20 ms.
        fuel: 1 << 62,
        wall_clock_ms,
        epoch_tick_ms: 5,
        ..SandboxLimits::default()
    })
    .await;

    let started = std::time::Instant::now();
    let mut next = bomb(&pool, "spin", Refusal::WallClockExceeded, 1 << 62).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the deadline is {wall_clock_ms} ms; {:?} means it did not fire",
        started.elapsed()
    );

    next.entry = "plugin.golden.noop".into();
    assert_eq!(
        pool.invoke(&next, &guest("noop")).await.reply.outcome,
        InvocationOutcome::Ok
    );
    pool.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_growth_past_the_instance_cap_is_a_trap_not_a_grow_failure() {
    let pool = in_process(SandboxLimits::default()).await;
    let mut next = bomb(
        &pool,
        "memory-bomb",
        Refusal::GuestTrap,
        abi::FUEL_PER_INVOCATION,
    )
    .await;

    next.entry = "plugin.golden.noop".into();
    assert_eq!(
        pool.invoke(&next, &guest("noop")).await.reply.outcome,
        InvocationOutcome::Ok
    );
    pool.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversize_output_is_refused_before_the_host_allocates() {
    let pool = in_process(SandboxLimits::default()).await;
    let mut next = bomb(
        &pool,
        "output-bomb",
        Refusal::OutputTooLarge,
        abi::FUEL_PER_INVOCATION,
    )
    .await;

    next.entry = "plugin.golden.noop".into();
    assert_eq!(
        pool.invoke(&next, &guest("noop")).await.reply.outcome,
        InvocationOutcome::Ok
    );
    pool.shutdown().await;
}

/// The log budget has no return value to refuse in, so the proof is what the
/// sandbox captured: 100 lines in, 100 lines out, and the 101st is dropped
/// whole. A line over the per-line cap is dropped rather than truncated, because
/// half a log line is worse than none.
#[test]
fn the_log_budget_drops_lines_instead_of_growing() {
    let sandbox = Sandbox::with_limits(
        Arc::new(register_all()),
        SandboxLimits {
            // The guests here log in a tight loop; the budget is not the cap
            // under test.
            fuel: abi::FUEL_PER_INVOCATION,
            ..SandboxLimits::default()
        },
    )
    .expect("sandbox");

    let fill = guest("log-fill");
    let module = sandbox.load(&fill).expect("log-fill loads");
    let output = sandbox
        .run(&module, &request())
        .expect("100 lines is legal");
    assert_eq!(output.logs.len(), abi::MAX_LOG_LINES as usize);
    assert!(
        output
            .logs
            .iter()
            .all(|line| line.message.len() <= abi::MAX_LOG_LINE_BYTES),
        "a captured line is over the per-line cap"
    );

    let overflow = guest("log-overflow");
    let module = sandbox.load(&overflow).expect("log-overflow loads");
    let output = sandbox
        .run(&module, &request())
        .expect("101 lines still runs");
    assert_eq!(
        output.logs.len(),
        abi::MAX_LOG_LINES as usize,
        "the 101st line must be dropped, not queued"
    );

    let long = guest("log-over-line");
    let module = sandbox.load(&long).expect("log-over-line loads");
    let output = sandbox
        .run(&module, &request())
        .expect("the invocation still runs");
    assert!(
        output.logs.is_empty(),
        "a 4097-byte line must be dropped whole, and {} line(s) were kept",
        output.logs.len()
    );
}
