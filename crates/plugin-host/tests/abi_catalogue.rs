//! The v1 ABI is published as DATA in `packages/plugin-kit/abi/host-abi.v1.json`
//! so the JS kit (build-time module validation) and the Rust PDK read the same
//! numbers the host enforces. Two copies of a contract drift; this test is the
//! seam that stops them: every entry, every refusal string and code, every
//! limit and every query id is compared against the Rust source of truth.
//!
//! The file is read at compile time (`include_str!`), so a catalogue that
//! moves or breaks fails the build rather than skipping the check.

use plugin_host::abi::{self as abi_consts, Refusal};
use plugin_host::host_fns::model;
use plugin_host::host_fns::{register_all, AbiParam, AbiReturn, HostGate, ImplStatus};

const CATALOGUE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/plugin-kit/abi/host-abi.v1.json"
));

fn catalogue() -> serde_json::Value {
    serde_json::from_str(CATALOGUE).expect("the shared catalogue is valid JSON")
}

fn param_name(param: AbiParam) -> &'static str {
    match param {
        AbiParam::I32 => "i32",
        AbiParam::U32 => "u32",
    }
}

fn return_name(ret: AbiReturn) -> &'static str {
    match ret {
        AbiReturn::None => "none",
        AbiReturn::Status => "status",
        AbiReturn::Data => "data",
        AbiReturn::Int => "int",
    }
}

fn gate_name(gate: HostGate) -> &'static str {
    match gate {
        HostGate::InstallScope => "install_scope",
        HostGate::Capability(_) => "capability",
        HostGate::Query => "query",
        HostGate::Egress => "egress",
        HostGate::Event => "event",
    }
}

fn status_name(status: ImplStatus) -> String {
    match status {
        ImplStatus::Shipped => "shipped".to_string(),
        ImplStatus::Deferred { owner } => format!("deferred:{owner}"),
    }
}

#[test]
fn the_catalogue_declares_the_namespace_and_version_the_host_pins() {
    let json = catalogue();
    assert_eq!(json["abi_version"], abi_consts::PLUGIN_ABI_VERSION);
    assert_eq!(json["namespace"], abi_consts::HOST_NAMESPACE);
}

#[test]
fn the_published_exports_are_the_three_the_abi_fixes() {
    // The host never calls the guest's exports, so there is no Rust constant to
    // compare against here: the guest side (Rust PDK and JS kit) reads this
    // list from the same file. Pinning it in one place still catches a drive-by
    // rename, which would break every already-built module.
    let json = catalogue();
    let exports = json["exports"].as_array().expect("exports is an array");
    let rows: Vec<(&str, Vec<&str>, Option<&str>)> = exports
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("export name"),
                entry["params"]
                    .as_array()
                    .expect("export params")
                    .iter()
                    .map(|p| p.as_str().expect("export param"))
                    .collect(),
                entry["returns"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("marula_alloc", vec!["i32"], Some("i32")),
            ("marula_entry", vec!["i32", "i32"], Some("i64")),
            ("marula_free", vec!["i32", "i32"], None),
        ]
    );
}

#[test]
fn every_host_function_matches_its_registry_entry() {
    let json = catalogue();
    let declared = json["host_functions"].as_array().expect("host_functions");
    let registry = register_all();
    let registered = registry.catalogue();

    assert_eq!(
        declared.len(),
        registered.len(),
        "the catalogue declares {} entries and the registry registers {}",
        declared.len(),
        registered.len()
    );

    for (declared, entry) in declared.iter().zip(registered) {
        assert_eq!(declared["name"].as_str(), Some(entry.name), "name/order");

        let params: Vec<&str> = entry.params.iter().copied().map(param_name).collect();
        let declared_params: Vec<&str> = declared["params"]
            .as_array()
            .expect("params")
            .iter()
            .map(|p| p.as_str().expect("param"))
            .collect();
        assert_eq!(declared_params, params, "{} params", entry.name);

        assert_eq!(
            declared["returns"].as_str(),
            Some(return_name(entry.returns)),
            "{} return shape",
            entry.name
        );
        assert_eq!(
            declared["gate"].as_str(),
            Some(gate_name(entry.gate)),
            "{} gate",
            entry.name
        );
        assert_eq!(
            declared["status"].as_str(),
            Some(status_name(entry.status).as_str()),
            "{} shipped/deferred",
            entry.name
        );
    }
}

#[test]
fn every_refusal_named_by_a_function_is_a_real_refusal() {
    let json = catalogue();
    for entry in json["host_functions"].as_array().expect("host_functions") {
        for reason in entry["refusals"].as_array().expect("refusals") {
            let reason = reason.as_str().expect("refusal string");
            assert!(
                Refusal::ALL.iter().any(|r| r.as_str() == reason),
                "{} names {:?}, which no Refusal variant produces",
                entry["name"],
                reason
            );
        }
    }
}

#[test]
fn refusal_strings_and_codes_match_the_rust_enum() {
    let json = catalogue();
    let declared = json["refusals"].as_array().expect("refusals");
    assert_eq!(declared.len(), Refusal::ALL.len(), "refusal count");

    for (index, (declared, refusal)) in declared.iter().zip(Refusal::ALL).enumerate() {
        assert_eq!(
            declared["code"].as_u64(),
            Some(u64::from(refusal.code())),
            "{} code",
            refusal.as_str()
        );
        assert_eq!(
            declared["reason"].as_str(),
            Some(refusal.as_str()),
            "row {index} reason"
        );
        // Codes are append-only and dense from 1: a gap means a variant was
        // removed, which breaks every already-compiled guest switching on it.
        assert_eq!(u32::try_from(index).unwrap() + 1, refusal.code());
    }
    assert_eq!(Refusal::ALL.len(), 17);
}

#[test]
fn limits_match_the_rust_constants() {
    let json = catalogue();
    let limits = &json["limits"];
    let cases: [(&str, serde_json::Value); 16] = [
        (
            "input_bytes",
            serde_json::json!(abi_consts::MAX_INPUT_BYTES),
        ),
        (
            "output_bytes",
            serde_json::json!(abi_consts::MAX_OUTPUT_BYTES),
        ),
        (
            "log_line_bytes",
            serde_json::json!(abi_consts::MAX_LOG_LINE_BYTES),
        ),
        ("log_lines", serde_json::json!(abi_consts::MAX_LOG_LINES)),
        (
            "kv_namespace_bytes",
            serde_json::json!(abi_consts::KV_NAMESPACE_MAX_BYTES),
        ),
        (
            "kv_key_bytes",
            serde_json::json!(abi_consts::KV_KEY_MAX_BYTES),
        ),
        (
            "kv_value_bytes",
            serde_json::json!(abi_consts::KV_VALUE_MAX_BYTES),
        ),
        ("kv_list_page", serde_json::json!(abi_consts::KV_LIST_PAGE)),
        (
            "http_response_bytes",
            serde_json::json!(abi_consts::HTTP_RESPONSE_MAX_BYTES),
        ),
        (
            "http_timeout_ms",
            serde_json::json!(abi_consts::HTTP_TIMEOUT_MS),
        ),
        (
            "http_redirects",
            serde_json::json!(abi_consts::HTTP_MAX_REDIRECTS),
        ),
        ("fuel", serde_json::json!(abi_consts::FUEL_PER_INVOCATION)),
        (
            "wall_clock_ms",
            serde_json::json!(abi_consts::WALL_CLOCK_MS),
        ),
        (
            "epoch_tick_ms",
            serde_json::json!(abi_consts::EPOCH_TICK_MS),
        ),
        (
            "instance_memory_bytes",
            serde_json::json!(abi_consts::INSTANCE_MEMORY_MAX_BYTES),
        ),
        (
            "worker_resident_bytes",
            serde_json::json!(abi_consts::WORKER_RESIDENT_MAX_BYTES),
        ),
    ];
    for (key, expected) in &cases {
        assert_eq!(&limits[*key], expected, "limit {key}");
    }
    assert_eq!(
        limits.as_object().expect("limits object").len(),
        cases.len(),
        "a limit was added to the catalogue without a Rust constant"
    );
}

#[test]
fn the_query_table_matches_the_registry() {
    let json = catalogue();
    let declared = json["query_ids"].as_array().expect("query_ids");
    assert_eq!(declared.len(), model::QUERIES.len());
    for (declared, spec) in declared.iter().zip(model::QUERIES) {
        assert_eq!(declared["id"].as_u64(), Some(u64::from(spec.id)));
        assert_eq!(declared["name"].as_str(), Some(spec.name));
        assert_eq!(
            declared["capability"].as_str(),
            Some(spec.capability.as_str()),
            "{} capability",
            spec.name
        );
        assert_eq!(model::query(spec.id), Some(spec));
    }
    assert!(model::query(5).is_none(), "the table is closed at four");
}
