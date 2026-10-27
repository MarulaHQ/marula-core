//! The golden conformance battery: one table, two engines, one verdict.
//!
//! `plugins/wasm/golden/expectations.json` names every golden guest, the grants
//! the invocation carries, and the exact answer it must produce. This test runs
//! each guest twice — once through [`Sandbox`], which reports fuel and captured
//! logs, and once through a real [`PluginPool`] worker process, which is the
//! transport a deployment runs — and fails when the two disagree with the table
//! or with each other.
//!
//! The corpus is DATA, not strings in this file. A host change that moves a
//! verdict, a refusal code or a limit shows up as a diff against a table a
//! reviewer can read without opening Rust, and adding a case is one `.wat` file
//! and one table row.

use std::sync::Arc;

use plugin_host::abi::{self, InvocationOutcome, InvocationRequest};
use plugin_host::host_fns::register_all;
use plugin_host::isolation::{LoadError, ModuleArtifact, Sandbox, SandboxLimits};
use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
use serde_json::Value;
use uuid::Uuid;

const CORPUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../plugins/wasm/golden/expectations.json"
));

fn corpus_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/wasm/golden")
}

/// One table row, resolved to the bytes it names.
struct Case {
    name: String,
    claim: String,
    artifact: ModuleArtifact,
    grants: Vec<String>,
    declared_events: Vec<String>,
    payload: Vec<u8>,
    expect: Value,
}

fn cases() -> Vec<Case> {
    let table: Value = serde_json::from_str(CORPUS).expect("the golden table is valid JSON");
    assert_eq!(
        table["abi_version"],
        Value::from(abi::PLUGIN_ABI_VERSION),
        "the corpus was written against a different ABI version"
    );
    let modules = table["modules"].as_array().expect("modules is an array");
    assert!(!modules.is_empty(), "an empty corpus proves nothing");
    modules
        .iter()
        .map(|row| {
            let name = row["name"].as_str().expect("name").to_string();
            let file = row["file"]
                .as_str()
                .unwrap_or_else(|| panic!("{name}: file"));
            let source = std::fs::read_to_string(corpus_root().join(file))
                .unwrap_or_else(|error| panic!("{name}: {file} is not readable: {error}"));
            let bytes = wat::parse_str(&source)
                .unwrap_or_else(|error| panic!("{name}: {file} is not valid WAT: {error}"));
            let payload = match &row["payload"] {
                Value::Null => Vec::new(),
                Value::Object(map) if map.contains_key("fill") => {
                    let fill = map["fill"].as_u64().expect("fill") as u8;
                    let len = map["len"].as_u64().expect("len") as usize;
                    vec![fill; len]
                }
                Value::String(text) => text.as_bytes().to_vec(),
                other => panic!("{name}: payload {other} is not a string or a fill"),
            };
            Case {
                claim: row["claim"].as_str().expect("claim").to_string(),
                name,
                artifact: ModuleArtifact {
                    digest: ModuleArtifact::digest_of(&bytes),
                    bytes,
                },
                grants: strings(&row["grants"]),
                declared_events: strings(&row["declared_events"]),
                payload,
                expect: row["expect"].clone(),
            }
        })
        .collect()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().expect("a string").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn sandbox() -> Sandbox {
    Sandbox::with_limits(Arc::new(register_all()), SandboxLimits::default())
        .expect("the sandbox builds with the shipped limits")
}

/// Which refusals the table allows for one case. `reason` is exact;
/// `reason_any_of` is the set a case genuinely has more than one of (a spin may
/// reach fuel or the clock first, and both mean the invocation's allowance ran
/// out). An empty result means the table is malformed, which fails loudly.
fn allowed_reasons(expect: &Value) -> Vec<String> {
    if let Some(reason) = expect["reason"].as_str() {
        return vec![reason.to_string()];
    }
    strings(&expect["reason_any_of"])
}

fn assert_reason(label: &str, expect: &Value, actual: &str) {
    let allowed = allowed_reasons(expect);
    assert!(!allowed.is_empty(), "{label}: the table names no refusal");
    assert!(
        allowed.iter().any(|reason| reason == actual),
        "{label}: {actual} is not one of {allowed:?}"
    );
}

fn request(case: &Case) -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        install_id: Uuid::now_v7(),
        entry: format!("plugin.golden.{}.entry", case.name),
        payload: case.payload.clone(),
        granted: case.grants.clone(),
        declared_events: case.declared_events.clone(),
        egress: Default::default(),
    }
}

/// Every guest in the corpus, judged by the sandbox: load first, then run, then
/// compare to the table.
#[test]
fn the_golden_corpus_holds_in_the_sandbox() {
    let sandbox = sandbox();
    for case in cases() {
        let label = format!("[{}] {}", case.name, case.claim);
        let module = match sandbox.load(&case.artifact) {
            Ok(module) => {
                assert!(
                    case.expect.get("load_rejected_contains").is_none(),
                    "{label}: the table says this module must not load, and it did"
                );
                module
            }
            Err(LoadError::Rejected(detail)) => {
                match case.expect["load_rejected_contains"].as_str() {
                    Some(needle) => assert!(
                        detail.contains(needle),
                        "{label}: refusal {detail:?} does not mention {needle:?}"
                    ),
                    None => panic!("{label}: refused to load: {detail}"),
                }
                continue;
            }
            Err(other) => panic!("{label}: refused to load: {other}"),
        };

        let outcome = sandbox.run(&module, &request(&case));
        match (case.expect["outcome"].as_str(), outcome) {
            (Some("ok"), Ok(output)) => {
                if case.expect["output_absent"].as_bool() == Some(true) {
                    assert!(output.output.is_none(), "{label}: expected no output");
                }
                if let Some(bytes) = case.expect["output_bytes"].as_array() {
                    let expected: Vec<u8> = bytes
                        .iter()
                        .map(|byte| byte.as_u64().expect("a byte") as u8)
                        .collect();
                    assert_eq!(
                        output.output.as_deref(),
                        Some(expected.as_slice()),
                        "{label}: output bytes"
                    );
                }
                if let Some(text) = case.expect["output_text"].as_str() {
                    assert_eq!(
                        output.output.as_deref(),
                        Some(text.as_bytes()),
                        "{label}: output text"
                    );
                }
                if case.expect["output_equals_payload"].as_bool() == Some(true) {
                    assert_eq!(
                        output.output.as_deref(),
                        Some(case.payload.as_slice()),
                        "{label}: the payload did not survive the round trip"
                    );
                }
                if let Some(lines) = case.expect["log_lines"].as_u64() {
                    assert_eq!(
                        output.logs.len() as u64,
                        lines,
                        "{label}: captured log lines"
                    );
                }
            }
            (Some("refused"), Err(failure)) => {
                assert_reason(&label, &case.expect, failure.refusal.as_str());
                if let Some(fuel) = case.expect["fuel_at_least"].as_u64() {
                    assert!(
                        failure.fuel_used >= fuel,
                        "{label}: {} fuel used, the cap is {fuel}",
                        failure.fuel_used
                    );
                }
                if case.expect["fuel_used_present"].as_bool() == Some(true) {
                    assert!(
                        failure.fuel_used > 0,
                        "{label}: a refused invocation still reports the fuel it spent"
                    );
                }
            }
            (expected, actual) => {
                panic!("{label}: the table says {expected:?}, the sandbox answered {actual:?}")
            }
        }
    }
}

/// The same table through a real worker process. The worker is the transport a
/// deployment runs, and a host that agreed with the sandbox but not with the
/// worker would be two implementations of one contract.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_golden_corpus_holds_through_a_real_worker() {
    let pool = PluginPool::start(PoolConfig {
        mode: IsolationMode::Pool,
        workers: 1,
        worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
        health: None,
        sandbox_limits: SandboxLimits::default(),
    })
    .await
    .expect("the pool starts");

    for case in cases().into_iter().filter(|case| {
        // A module the table expects to be refused at load never reaches a
        // worker: the parent cannot name an artifact it could not verify, and
        // the worker refuses it the same way the sandbox does. Loading is
        // covered above; running it here would only re-prove the sandbox.
        case.expect.get("load_rejected_contains").is_none()
    }) {
        let label = format!("[{}] {}", case.name, case.claim);
        let result = pool.invoke(&request(&case), &case.artifact).await;
        let reply = &result.reply;
        assert_eq!(
            reply.validate(),
            Ok(()),
            "{label}: the worker answered a frame the ABI rejects"
        );
        assert!(
            result.worker.is_some(),
            "{label}: a worker answered and its label is missing"
        );

        match case.expect["outcome"].as_str() {
            Some("ok") => {
                assert_eq!(
                    reply.outcome,
                    InvocationOutcome::Ok,
                    "{label}: {:?}",
                    reply.reason
                );
                if case.expect["output_absent"].as_bool() == Some(true) {
                    assert!(reply.output.is_none(), "{label}: expected no output");
                }
                if let Some(bytes) = case.expect["output_bytes"].as_array() {
                    let expected: Vec<u8> = bytes
                        .iter()
                        .map(|byte| byte.as_u64().expect("a byte") as u8)
                        .collect();
                    assert_eq!(
                        reply.output.as_deref(),
                        Some(expected.as_slice()),
                        "{label}: output bytes differ from the sandbox's answer"
                    );
                }
                if let Some(text) = case.expect["output_text"].as_str() {
                    assert_eq!(reply.output.as_deref(), Some(text.as_bytes()), "{label}");
                }
                if case.expect["output_equals_payload"].as_bool() == Some(true) {
                    assert_eq!(
                        reply.output.as_deref(),
                        Some(case.payload.as_slice()),
                        "{label}: the payload did not survive the worker"
                    );
                }
            }
            Some("refused") => {
                assert_eq!(reply.outcome, InvocationOutcome::Refused, "{label}");
                let actual = reply.reason.expect("a refusal carries a reason").as_str();
                assert_reason(&label, &case.expect, actual);
            }
            other => panic!("{label}: the table has no usable outcome ({other:?})"),
        }
    }
    pool.shutdown().await;
}

/// The three committed first-party artifacts are guests this host accepts. They
/// are what an operator installs, so a drift between the PDK and the host shows
/// up here rather than on a customer's first invocation.
#[test]
fn every_committed_first_party_artifact_loads() {
    let sandbox = sandbox();
    let plugins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/wasm");
    let mut checked = 0;
    for entry in std::fs::read_dir(&plugins).expect("the plugin directory is readable") {
        let path = entry.expect("a directory entry").path();
        let module_path = path.join("plugin.wasm");
        if !module_path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&module_path).expect("the artifact is readable");
        let artifact = ModuleArtifact {
            digest: ModuleArtifact::digest_of(&bytes),
            bytes,
        };
        sandbox.load(&artifact).unwrap_or_else(|error| {
            panic!(
                "{} does not satisfy the ABI: {error}",
                module_path.display()
            )
        });
        checked += 1;
    }
    assert!(
        checked >= 3,
        "found {checked} first-party artifacts, expected 3"
    );
}
