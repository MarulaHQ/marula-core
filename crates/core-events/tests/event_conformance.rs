//! Event-evolution conformance suite (policy §2, TASKLIST U2).
//!
//! Proves the three promises of the domain-events contract against the
//! published-payload registry (`core_events::contract`) and the committed
//! golden fixtures (`fixtures/events/v1/`):
//!
//! 1. **Unknown-field tolerance** — every golden payload, and the envelope
//!    itself, deserializes with fields present that today's structs do not
//!    know about. Consumers are REQUIRED to tolerate unknown fields; the
//!    producer side proves they can by never being able to deny them.
//! 2. **Additive-only payloads** — golden fixtures are the pinned shape of
//!    every published payload per event type + variant + version. A change
//!    that drops or renames a field fails here: required fields break
//!    deserialization of the old golden, Option fields shrink the struct's
//!    key set below the golden's.
//! 3. **Retired types co-emit for one generation** — the retired registry
//!    only accepts entries with a published replacement and a sane
//!    generation window; goldens of retired types are retained until the
//!    deprecation clock (U4, `deprecation_clock` suite) allows removal.
//!
//! Golden maintenance: regenerate with
//! `cargo test -p core-events --test event_conformance generate_golden_fixtures -- --ignored`
//! and review the diff — a regeneration that removes keys is a BREAKING
//! change and must carry a release-notes "Breaking & deprecated" entry
//! (policy §3a) plus, for event types, a U4 registry entry.

use core_domain::deprecation::Version;
use core_events::contract::{self, GoldenFixture, PublishedEvent, Shape, RETIRED_EVENTS};
use core_events::Envelope;
use serde_json::{json, Value};

fn workspace_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("workspace version parses")
}

fn load_golden(entry: &PublishedEvent) -> GoldenFixture {
    contract::load_fixture(&entry.fixture_file())
        .unwrap_or_else(|e| panic!("golden for {} ({}): {e}", entry.event_type, entry.variant))
}

/// Injects fields today's structs have never heard of — the shape of a
/// payload emitted by a FUTURE Marula.
fn with_unknown_fields(payload: &Value) -> Value {
    let mut obj = payload
        .as_object()
        .cloned()
        .expect("golden payload is an object");
    obj.insert(
        "marula_future_field".into(),
        json!({ "nested": [1, 2, 3] }),
    );
    obj.insert("also_unknown".into(), json!(null));
    Value::Object(obj)
}

fn envelope_wire(golden: &GoldenFixture, payload: Value) -> Value {
    json!({
        "id": uuid::Uuid::from_u128(1).to_string(),
        "type": golden.event_type,
        "org_id": uuid::Uuid::from_u128(2).to_string(),
        "entity": golden.entity,
        "entity_id": uuid::Uuid::from_u128(3).to_string(),
        "occurred_at": "2026-01-01T00:00:00Z",
        "payload_schema_ref": contract::PAYLOAD_SCHEMA_V1,
        "payload": payload,
        // Fields a future envelope revision may add:
        "trace_id": "future-envelope-field",
    })
}

// ---------------------------------------------------------------------------
// Promise 0: every published payload has a committed, coherent golden.
// ---------------------------------------------------------------------------

#[test]
fn every_published_entry_has_a_committed_golden() {
    assert!(
        !contract::PUBLISHED_EVENTS.is_empty(),
        "registry must not be emptied silently"
    );
    for entry in contract::PUBLISHED_EVENTS {
        let golden = load_golden(entry);
        assert_eq!(golden.event_type, entry.event_type, "fixture/type mismatch");
        assert_eq!(golden.entity, entry.entity, "fixture/entity mismatch");
        assert_eq!(golden.variant, entry.variant, "fixture/variant mismatch");
        assert_eq!(
            golden.payload_schema_ref,
            contract::PAYLOAD_SCHEMA_V1,
            "golden pins a different payload schema generation"
        );
        Version::parse(&golden.pinned_in)
            .unwrap_or_else(|e| panic!("golden {} pinned_in: {e}", entry.fixture_file()));
        golden
            .payload
            .as_object()
            .unwrap_or_else(|| panic!("golden {} payload must be an object", entry.fixture_file()));
    }
}

// ---------------------------------------------------------------------------
// Promise 1: consumers tolerate unknown fields.
// ---------------------------------------------------------------------------

#[test]
fn golden_payloads_deserialize_with_unknown_fields_present() {
    for entry in contract::PUBLISHED_EVENTS {
        let golden = load_golden(entry);
        let fut = with_unknown_fields(&golden.payload);
        entry
            .shape
            .accept(&fut)
            .unwrap_or_else(|e| {
                panic!(
                    "{} ({}): payload no longer tolerates unknown fields — contract types must never deny them: {e}",
                    entry.event_type, entry.variant
                )
            });
    }
}

#[test]
fn envelope_and_stream_document_tolerate_unknown_fields() {
    // A future envelope revision (and any extra stream metadata) must not
    // break consumers parsing today's envelopes.
    let entry = &contract::PUBLISHED_EVENTS[0];
    let golden = load_golden(entry);
    let wire = envelope_wire(&golden, with_unknown_fields(&golden.payload));
    let env: Envelope = serde_json::from_value(wire).expect("envelope tolerates unknown fields");
    assert_eq!(env.event_type, golden.event_type);
    assert_eq!(env.payload_schema_ref, contract::PAYLOAD_SCHEMA_V1);
}

#[test]
fn envelope_round_trips_payload_verbatim() {
    let entry = &contract::PUBLISHED_EVENTS[0];
    let golden = load_golden(entry);
    let wire = envelope_wire(&golden, golden.payload.clone());
    let env: Envelope = serde_json::from_value(wire).expect("envelope parses");
    assert_eq!(env.payload, golden.payload, "payload must survive verbatim");
    let round: Envelope =
        serde_json::from_value(env.to_json()).expect("envelope round-trips through JSON");
    assert_eq!(round, env);
}

// ---------------------------------------------------------------------------
// Promise 2: payload evolution is additive-only.
// ---------------------------------------------------------------------------

#[test]
fn golden_payloads_still_deserialize_into_current_structs() {
    for entry in contract::PUBLISHED_EVENTS {
        let golden = load_golden(entry);
        entry.shape.accept(&golden.payload).unwrap_or_else(|e| {
            panic!(
                "{} ({}): breaking change detected — the golden pinned by {} no longer deserializes: {e}",
                entry.event_type, entry.variant, golden.pinned_in
            )
        });
    }
}

#[test]
fn struct_key_sets_never_shrink_below_golden() {
    // Catches drops/renames of Option fields, which deserialization alone
    // cannot see (missing Option silently becomes None).
    for entry in contract::PUBLISHED_EVENTS {
        let golden = load_golden(entry);
        let golden_keys: std::collections::BTreeSet<&str> = golden
            .payload
            .as_object()
            .expect("golden payload object")
            .keys()
            .map(String::as_str)
            .collect();
        let sample_keys_set: std::collections::BTreeSet<String> =
            entry.shape.sample_keys().into_iter().collect();
        let sample_keys: std::collections::BTreeSet<&str> =
            sample_keys_set.iter().map(String::as_str).collect();
        let lost: Vec<_> = golden_keys.difference(&sample_keys).collect();
        assert!(
            lost.is_empty(),
            "{} ({}): fields present in the golden ({}) vanished from the current shape — payload evolution is additive-only (policy §2). If this removal is intentional, it is a breaking change: regenerate goldens with a release-notes entry and, for retired types, a deprecation-registry entry.",
            entry.event_type,
            entry.variant,
            entry.fixture_file()
        );
    }
}

#[test]
fn untyped_required_keys_covered_by_golden_and_sample() {
    for entry in contract::PUBLISHED_EVENTS {
        let Shape::Untyped(untyped) = &entry.shape else {
            continue;
        };
        let golden = load_golden(entry);
        let golden_obj = golden.payload.as_object().unwrap();
        for key in untyped.required_keys {
            assert!(
                golden_obj.contains_key(*key),
                "{} ({}): required key `{key}` missing from its own golden",
                entry.event_type,
                entry.variant
            );
        }
        let sample_keys = entry.shape.sample_keys();
        for key in untyped.required_keys {
            assert!(
                sample_keys.iter().any(|k| k == key),
                "{} ({}): required key `{key}` missing from the registry sample — the registry and the emission site have drifted",
                entry.event_type,
                entry.variant
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Completeness: every event type constant is accounted for.
// ---------------------------------------------------------------------------

/// Parses `pub const X: &str = "t";` lines from core-domain's events.rs so
/// a newly declared constant (new payload) cannot skip registration.
fn declared_event_types() -> Vec<(String, String)> {
    let path = contract::repo_root()
        .join("crates/core-domain/src/events.rs")
        .canonicalize()
        .expect("core-domain events.rs exists");
    let src = std::fs::read_to_string(&path).expect("read events.rs");
    let mut out = Vec::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("pub const ") || !trimmed.contains(": &str =") {
            continue;
        }
        let name = trimmed
            .strip_prefix("pub const ")
            .and_then(|rest| rest.split(':').next())
            .unwrap_or("")
            .trim()
            .to_string();
        let value = match (trimmed.find('"'), trimmed.rfind('"')) {
            (Some(a), Some(b)) if a < b => trimmed[a + 1..b].to_string(),
            _ => continue,
        };
        if !name.is_empty() {
            out.push((name, value));
        }
    }
    out
}

#[test]
fn every_event_type_constant_is_published_or_reserved() {
    let reserved: std::collections::BTreeSet<&str> =
        contract::RESERVED_EVENT_TYPES.iter().copied().collect();
    let published: std::collections::BTreeSet<&str> =
        contract::published_event_types().into_iter().collect();
    let unaccounted: Vec<_> = declared_event_types()
        .into_iter()
        .filter(|(_, value)| {
            !published.contains(value.as_str()) && !reserved.contains(value.as_str())
        })
        .collect();
    assert!(
        unaccounted.is_empty(),
        "event type constants with no contract entry — register a golden in core_events::contract::PUBLISHED_EVENTS (or add to RESERVED_EVENT_TYPES until an emission site ships): {unaccounted:?}"
    );
}

#[test]
fn reserved_event_types_have_no_published_entry() {
    let published: std::collections::BTreeSet<&str> =
        contract::published_event_types().into_iter().collect();
    for r in contract::RESERVED_EVENT_TYPES {
        assert!(
            !published.contains(r),
            "type `{r}` is both reserved and published — remove it from RESERVED_EVENT_TYPES"
        );
    }
}

// ---------------------------------------------------------------------------
// Promise 3: retirement requires a replacement and one co-emit generation.
// ---------------------------------------------------------------------------

#[test]
fn retired_event_registry_is_valid() {
    assert!(
        contract::retired_registry_errors().is_empty(),
        "retired registry invalid: {:?}",
        contract::retired_registry_errors()
    );
    // Goldens of retired types are retained until the clock allows removal —
    // consumers still deserialize them during the whole window.
    for r in RETIRED_EVENTS {
        let entries = contract::entries_for_type(r.event_type);
        assert!(
            !entries.is_empty(),
            "retired event `{}` must stay in the published registry while it co-emits",
            r.event_type
        );
        for entry in entries {
            load_golden(entry);
        }
    }
}

#[test]
fn retired_registry_and_deprecation_registry_agree() {
    // U4's deprecations.json is the cross-surface registry; event-kind
    // entries and core-events' retired registry must list the same set.
    let path = contract::repo_root().join("deprecations.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let registry = core_domain::deprecation::DeprecationRegistry::parse(&raw)
        .unwrap_or_else(|e| panic!("deprecations.json: {e}"));
    let mut clock_events: Vec<&str> = registry
        .deprecations
        .iter()
        .filter(|d| d.kind == core_domain::deprecation::ConstructKind::Event)
        .map(|d| d.id.as_str())
        .collect();
    clock_events.sort_unstable();
    let mut retired: Vec<&str> = RETIRED_EVENTS.iter().map(|r| r.event_type).collect();
    retired.sort_unstable();
    assert_eq!(
        clock_events, retired,
        "deprecations.json event entries and core-events RETIRED_EVENTS disagree — a retirement must be registered in both, in the same change"
    );
}

#[test]
fn co_emit_helper_produces_the_generation_pair() {
    // Mechanism check on a synthetic retirement of a real published type:
    // publishers emit the replacement envelope AND derive the retired one
    // via Envelope::co_emit_as — same payload, fresh id, old type string.
    let entry = &contract::PUBLISHED_EVENTS[0];
    let golden = load_golden(entry);
    let wire = envelope_wire(&golden, golden.payload.clone());
    let replacement: Envelope = serde_json::from_value(wire).expect("envelope parses");
    let retired = replacement.co_emit_as("ticket.created__legacy");
    assert_eq!(retired.event_type, "ticket.created__legacy");
    assert_ne!(
        retired.id, replacement.id,
        "co-emitted events are distinct rows"
    );
    assert_eq!(retired.org_id, replacement.org_id);
    assert_eq!(retired.entity, replacement.entity);
    assert_eq!(retired.occurred_at, replacement.occurred_at);
    assert_eq!(retired.payload, replacement.payload);
    assert_eq!(
        retired.payload_schema_ref, replacement.payload_schema_ref,
        "co-emitted generations share the payload schema ref"
    );
    // Both documents must carry the same required contract fields on the
    // wire (consumers of either type parse one envelope shape).
    for env in [&replacement, &retired] {
        let doc = env.to_json();
        assert!(doc["type"].is_string());
        assert!(doc["payload_schema_ref"].is_string());
    }
}

// ---------------------------------------------------------------------------
// Golden generation — explicit, reviewed, never run by CI.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "writes fixture files; run explicitly with --ignored and review the diff"]
fn generate_golden_fixtures() {
    let dir = contract::repo_root().join(contract::FIXTURES_DIR);
    std::fs::create_dir_all(&dir).expect("create fixtures dir");
    let pinned_in = workspace_version().to_string();
    let mut written = 0;
    for entry in contract::PUBLISHED_EVENTS {
        let golden = GoldenFixture {
            event_type: entry.event_type.to_string(),
            entity: entry.entity.to_string(),
            variant: entry.variant.to_string(),
            payload_schema_ref: contract::PAYLOAD_SCHEMA_V1.to_string(),
            pinned_in: pinned_in.clone(),
            payload: entry.shape.sample(),
        };
        let path = dir.join(entry.fixture_file());
        let body = serde_json::to_string_pretty(&golden).expect("golden serializes") + "\n";
        std::fs::write(&path, body).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        written += 1;
    }
    println!("wrote {written} goldens to {}", dir.display());
}
