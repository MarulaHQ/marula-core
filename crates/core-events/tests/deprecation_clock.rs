//! Deprecation clock gate (policy §8, TASKLIST U4).
//!
//! The build-time enforcement of `deprecated_in` / `removal_target`: a
//! change fails this gate when it removes a registered construct before its
//! removal_target, or carries one past it. Wired into CI as its own job
//! (next to the migration-safety and brand-distinctness gates).
//!
//! Registry: `deprecations.json` at the repo root (model in
//! `core_domain::deprecation`, kept pure — this file owns the tree I/O).
//! Empty registries are legal (nothing deprecated yet); the self-tests
//! below prove every failure mode against synthetic trees so the gate is
//! exercised end-to-end before the first real entry exists.
//!
//! Registering a deprecation (policy §3): add an entry to deprecations.json
//! with a distinctive probe, announce it in release notes, keep it working
//! through the window (events: co-emit per U2), remove it in the target
//! version and prune the entry in the same change.

use core_domain::deprecation::{ClockReason, DeprecationRegistry, Probe, Version};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    core_events::contract::repo_root()
}

fn load_registry() -> DeprecationRegistry {
    let path = repo_root().join("deprecations.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    DeprecationRegistry::parse(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Directory walk for a probe: recursive, forward-slash relative paths,
/// skipping VCS/build/output trees that never carry contract surface.
fn walk(root: &Path, skip: &[&str], out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        let path = entry.path();
        if path.is_dir() {
            if skip.contains(&name.as_str()) {
                continue;
            }
            walk(&path, skip, out);
        } else {
            out.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Does any file matching the probe's glob contain the needle?
fn probe_present(root: &Path, probe: &Probe) -> bool {
    let mut files = Vec::new();
    walk(
        root,
        &[".git", "node_modules", "target", "graphify-out", ".kilo"],
        &mut files,
    );
    let root_str = root.to_string_lossy().replace('\\', "/");
    files.iter().any(|abs| {
        let rel = abs
            .strip_prefix(&root_str)
            .unwrap_or(abs)
            .trim_start_matches('/')
            .to_string();
        probe.matches_path(&rel)
            && std::fs::read_to_string(Path::new(abs))
                .map(|body| body.contains(&probe.needle))
                .unwrap_or(false)
    })
}

fn run_gate(root: &Path, registry: &DeprecationRegistry, current: &Version) -> Vec<String> {
    registry
        .check_clock(current, |probe| probe_present(root, probe))
        .into_iter()
        .map(|v| format!("{} [{}]: {}", v.id, reason_name(v.reason), v.detail))
        .collect()
}

fn reason_name(r: ClockReason) -> &'static str {
    match r {
        ClockReason::Overdue => "overdue",
        ClockReason::RemovedEarly => "removed-before-target",
    }
}

// ---------------------------------------------------------------------------
// The real registry, against this build.
// ---------------------------------------------------------------------------

#[test]
fn registry_document_exists_and_is_valid() {
    let registry = load_registry();
    assert_eq!(registry.version, 1, "unknown registry format");
    // Duplicate ids / same-generation targets / empty probes fail loud.
    registry.validate().expect("registry structural validation");
}

#[test]
fn deprecation_clock_passes_for_this_build() {
    let registry = load_registry();
    let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("workspace version");
    let violations = run_gate(&repo_root(), &registry, &current);
    assert!(
        violations.is_empty(),
        "deprecation clock violations:\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Self-tests: every failure mode, against synthetic trees.
// ---------------------------------------------------------------------------

/// Creates a unique throwaway directory with the given files.
fn temp_tree(files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "marula-deprecation-clock-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7().simple()
    ));
    for (rel, body) in files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, body).expect("write temp file");
    }
    root
}

fn synthetic_registry(
    needle: &'static str,
    deprecated_in: &str,
    target: &str,
) -> DeprecationRegistry {
    let doc = format!(
        r#"{{"version":1,"deprecations":[{{
            "id": "event.legacy_ping",
            "kind": "event",
            "deprecated_in": "{deprecated_in}",
            "removal_target": "{target}",
            "replaced_by": "event.pong",
            "probe": {{ "path_glob": "crates/core-domain/src/events.rs", "needle": "{needle}" }}
        }}]}}"#
    );
    DeprecationRegistry::parse(&doc).expect("synthetic registry parses")
}

const EVENTS_RS: &str = "pub const LEGACY_PING: &str = \"legacy.ping\";\n";

#[test]
fn removal_before_target_fails_the_gate() {
    let root = temp_tree(&[(
        "crates/core-domain/src/events.rs",
        "// construct already removed\n",
    )]);
    let registry = synthetic_registry("LEGACY_PING", "0.1.0", "0.3.0");
    let violations = run_gate(&root, &registry, &Version::new(0, 2, 0));
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].contains("removed-before-target"),
        "{violations:?}"
    );
    assert!(violations[0].contains("event.legacy_ping"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn carrying_past_target_fails_the_gate() {
    let root = temp_tree(&[("crates/core-domain/src/events.rs", EVENTS_RS)]);
    let registry = synthetic_registry("LEGACY_PING", "0.1.0", "0.2.0");
    let violations = run_gate(&root, &registry, &Version::new(0, 2, 0));
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(violations[0].contains("overdue"), "{violations:?}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn in_window_presence_and_timely_removal_pass() {
    // Window open, construct still shipped.
    let root = temp_tree(&[("crates/core-domain/src/events.rs", EVENTS_RS)]);
    let registry = synthetic_registry("LEGACY_PING", "0.1.0", "0.3.0");
    assert!(run_gate(&root, &registry, &Version::new(0, 2, 0)).is_empty());
    std::fs::remove_dir_all(&root).ok();

    // Window elapsed, construct removed on time.
    let root = temp_tree(&[("crates/core-domain/src/events.rs", "// removed in 0.3.0\n")]);
    let registry = synthetic_registry("LEGACY_PING", "0.1.0", "0.3.0");
    assert!(run_gate(&root, &registry, &Version::new(0, 3, 0)).is_empty());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn probes_scoped_by_glob_do_not_match_other_files() {
    // The needle exists in the tree but OUTSIDE the probe's glob — the
    // construct counts as removed.
    let root = temp_tree(&[("docs/notes.md", EVENTS_RS)]);
    let registry = synthetic_registry("LEGACY_PING", "0.1.0", "0.3.0");
    let violations = run_gate(&root, &registry, &Version::new(0, 2, 0));
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains("removed-before-target"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn malformed_registry_entries_fail_at_parse() {
    let root = temp_tree(&[]);
    // Same-generation removal target violates the one-generation rule.
    let bad = DeprecationRegistry::parse(
        r#"{"version":1,"deprecations":[{"id":"x","kind":"migration","deprecated_in":"0.2.0","removal_target":"0.2.1","probe":{"path_glob":"crates/core-db/migrations/*.sql","needle":"old_col"}}]}"#,
    );
    assert!(bad.is_err(), "same-generation target must be rejected");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn api_and_migration_probes_target_the_conventional_surfaces() {
    let api = Probe::api("\"/api/v1/legacy-endpoint\"");
    let migration = Probe::migration("legacy_column_name");
    assert_eq!(
        api.path_glob, "packages/sdk/openapi.json",
        "API probes default to the committed OpenAPI snapshot"
    );
    assert_eq!(
        migration.path_glob, "crates/core-db/migrations/*.sql",
        "migration probes default to the migration chain"
    );
}
