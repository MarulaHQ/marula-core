//! Golden OpenAPI conformance suites (TASKLIST U8, policy §8).
//!
//! Purpose: the API contract is a PINNED spec snapshot, one golden per
//! supported API version (`fixtures/openapi/v1/openapi.json` for `/api/v1`).
//! These suites assert, on every change, that the live surface still matches
//! the pin — drift must be a deliberate, reviewed re-pin, never an accident.
//!
//! Three layers of conformance:
//! 1. `derived_matches_pin` — the spec derived from the compiled handlers
//!    (utoipa, the same function the server serves at /openapi.json) equals
//!    the golden snapshot. This is the primary gate.
//! 2. `sdk_snapshot_matches_pin` — `packages/sdk/openapi.json` (from which
//!    the TS SDK is generated) equals the golden, so SDK regeneration
//!    discipline cannot silently drift either.
//! 3. `live_server_matches_pin` — env-gated (`MERIDIAN_API_BASE_URL`): the
//!    running server's served spec equals the golden. Used by the CI
//!    conformance job that boots the real stack; skipped locally.

use serde_json::Value;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../")
}

fn read_json(path: &PathBuf) -> Value {
    let raw =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn pin_path() -> PathBuf {
    repo_root().join("fixtures/openapi/v1/openapi.json")
}

/// Structural deep-equality with an actionable first-difference report.
fn assert_deep_eq(label: &str, expected: &Value, actual: &Value) {
    fn diff(path: String, expected: &Value, actual: &Value) -> Option<String> {
        match (expected, actual) {
            (Value::Object(e), Value::Object(a)) => {
                for (k, ev) in e {
                    match a.get(k) {
                        None => {
                            return Some(format!(
                                "{path}.{k} present in expected, MISSING in actual"
                            ))
                        }
                        Some(av) => {
                            if let Some(d) = diff(format!("{path}.{k}"), ev, av) {
                                return Some(d);
                            }
                        }
                    }
                }
                for k in a.keys() {
                    if !e.contains_key(k) {
                        return Some(format!("{path}.{k} present in actual, MISSING in expected"));
                    }
                }
                None
            }
            (Value::Array(e), Value::Array(a)) => {
                if e.len() != a.len() {
                    return Some(format!("{path}: array length {} vs {}", e.len(), a.len()));
                }
                for (i, (ev, av)) in e.iter().zip(a.iter()).enumerate() {
                    if let Some(d) = diff(format!("{path}[{i}]"), ev, av) {
                        return Some(d);
                    }
                }
                None
            }
            (e, a) if e == a => None,
            (e, a) => Some(format!("{path}: expected {e}, actual {a}")),
        }
    }

    if expected != actual {
        let first =
            diff(String::new(), expected, actual).unwrap_or_else(|| "documents differ".to_string());
        panic!(
            "{label} differs from the pinned golden spec.\nFirst difference: {first}\n\
             If intentional, re-pin with `cp <new spec> fixtures/openapi/v1/openapi.json` \
             and regenerate the SDK snapshot in the same change."
        );
    }
}

/// Sanity: the pin is a well-formed OpenAPI 3.1 document for /api/v1.
#[test]
fn pin_is_a_valid_v1_spec() {
    let pin = read_json(&pin_path());
    assert_eq!(
        pin["openapi"].as_str().unwrap_or_default().get(..2),
        Some("3."),
        "pin must be an OpenAPI 3.x document"
    );
    let paths = pin["paths"].as_object().expect("pin has paths");
    assert!(
        paths.len() >= 100,
        "v1 spec unexpectedly small — pin looks truncated ({} paths)",
        paths.len()
    );
    for key in paths.keys() {
        assert!(
            key.starts_with("/api/v1") || key == "/openapi.json" || key == "/healthz",
            "unexpected non-v1 path in pin: {key}"
        );
    }
    assert_eq!(
        pin["info"]["title"].as_str(),
        Some("Meridian Core API"),
        "pin is not the Meridian Core API spec"
    );
}

/// Primary gate: the spec derived from the compiled route surface equals
/// the golden pin.
#[test]
fn derived_matches_pin() {
    let pin = read_json(&pin_path());
    let derived = core_api::routes::openapi::openapi_json();
    assert_deep_eq("derived spec", &pin, &derived);
}

/// The committed SDK snapshot (input to `openapi-typescript` generation)
/// matches the golden pin.
#[test]
fn sdk_snapshot_matches_pin() {
    let pin = read_json(&pin_path());
    let sdk = read_json(&repo_root().join("packages/sdk/openapi.json"));
    assert_deep_eq("SDK snapshot", &pin, &sdk);
}

/// Live-server leg: boots nothing itself; when MERIDIAN_API_BASE_URL points
/// at a running server (CI conformance job), the served spec must equal the
/// pin byte-for-byte after JSON normalization.
#[tokio::test]
async fn live_server_matches_pin() {
    let base = match std::env::var("MERIDIAN_API_BASE_URL") {
        Ok(url) if !url.is_empty() => url.trim_end_matches('/').to_string(),
        _ => {
            eprintln!("skipped: MERIDIAN_API_BASE_URL not set");
            return;
        }
    };
    let url = format!("{base}/openapi.json");
    let resp = reqwest::get(&url)
        .await
        .unwrap_or_else(|e| panic!("GET {url}: {e}"));
    assert!(resp.status().is_success(), "GET {url} → {}", resp.status());
    let live: Value = resp.json().await.expect("live spec is JSON");
    let pin = read_json(&pin_path());
    assert_deep_eq("live served spec", &pin, &live);
}
