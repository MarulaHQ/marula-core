//! Upgrade preflight (TASKLIST U6, upgrade-compatibility-policy.md §5).
//!
//! Purpose: `meridian upgrade --check` runs OFFLINE, before an upgrade, and
//! reports exactly which of this org's customizations touch constructs the
//! target version deprecates — auto-migratable ones flagged as such,
//! judgment-call ones described in plain language. Inputs are the same
//! artifacts operators already have: the customization manifest (U5 JSON,
//! same envelope the API exports) and optionally a database copy (any
//! reachable read-only Postgres snapshot of the install).
//!
//! Sources of truth:
//! - The deprecation registry (`deprecations.json` at the repo root, U4 —
//!   the same file the CI deprecation-clock gate enforces). Each entry
//!   carries `kind` (api | event | migration | generic), `deprecated_in`,
//!   `removal_target`, and a `probe` (`path_glob` + `needle`).
//! - Manifest scanning interprets the probe's `path_glob` against the
//!   manifest's JSON paths (dot-separated; `*` = one segment, `**` = any
//!   tail) and reports every value containing the needle.
//!
//! Classification rule (deliberately simple, documented):
//! - `migration`-kind findings are AUTO-MIGRATABLE: the construct moves
//!   with the migration bundle itself, and policy §4 rule 5 guarantees the
//!   customization-row transform ships in the same transaction.
//! - Everything else is a JUDGMENT CALL: the deprecated construct appears
//!   inside org-authored documents (automation triggers, webhook filters,
//!   settings values, report specs), and a human must decide the
//!   replacement before upgrading.
//!
//! Exit code: 0 when only auto-migratable (or no) findings exist, 1 when
//! any judgment-call finding exists — usable as a gate in upgrade runbooks.
//!
//! Non-purpose: no migration execution, no writes of any kind, no network
//! beyond the optional database copy connection, no opinion about
//! constructs the registry does not list.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The deprecation registry file, U4 format (shared with the CI
/// deprecation-clock gate — one file, two consumers).
#[derive(Debug, Clone, Deserialize)]
pub struct Registry {
    pub version: u32,
    #[serde(default)]
    pub deprecations: Vec<Deprecation>,
}

/// One deprecated construct.
#[derive(Debug, Clone, Deserialize)]
pub struct Deprecation {
    #[serde(default)]
    pub id: Option<String>,
    /// api | event | migration | generic.
    pub kind: String,
    pub deprecated_in: String,
    pub removal_target: String,
    pub probe: Probe,
    #[serde(default)]
    pub doc: Option<String>,
}

/// Where to look and what to look for.
#[derive(Debug, Clone, Deserialize)]
pub struct Probe {
    pub path_glob: String,
    pub needle: String,
}

/// One touched customization, with its manifest path and the matched value.
#[derive(Debug, Clone, Serialize)]
pub struct Touchpoint {
    /// JSON path inside the manifest, e.g.
    /// `sections.automation_rules.0.trigger_event`.
    pub path: String,
    /// The value containing the deprecated needle (string-coerced).
    pub value: String,
    /// Human anchor for the finding (e.g. the rule's name) when one is
    /// available alongside the touched path.
    pub anchor: Option<String>,
}

/// Auto-migratable (transform ships with the migration bundle) or a human
/// judgment call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    AutoMigratable,
    JudgmentCall,
}

/// Everything the operator needs to act on one deprecated construct.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub construct: String,
    pub kind: String,
    pub deprecated_in: String,
    pub removal_target: String,
    pub classification: Classification,
    pub touchpoints: Vec<Touchpoint>,
    pub guidance: String,
    /// Registry documentation for the construct, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("registry {0}: {1}")]
    Registry(String, #[source] serde_json::Error),
    #[error("manifest {0}: {1}")]
    Manifest(String, #[source] serde_json::Error),
    #[error("invalid deprecation entry `{0}`: {1}")]
    Entry(String, String),
}

/// Loads and structurally validates the registry file.
pub fn load_registry(path: &Path) -> Result<Registry, PreflightError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| PreflightError::Entry(path.display().to_string(), e.to_string()))?;
    let registry: Registry = serde_json::from_str(&raw)
        .map_err(|e| PreflightError::Registry(path.display().to_string(), e))?;
    for dep in &registry.deprecations {
        match dep.kind.as_str() {
            "api" | "event" | "migration" | "generic" => {}
            other => {
                return Err(PreflightError::Entry(
                    dep.id.clone().unwrap_or_else(|| dep.needle_anchor()),
                    format!("unknown kind `{other}`"),
                ))
            }
        }
        if dep.probe.needle.is_empty() {
            return Err(PreflightError::Entry(
                dep.id.clone().unwrap_or_else(|| dep.needle_anchor()),
                "probe.needle must be non-empty".to_string(),
            ));
        }
    }
    Ok(registry)
}

impl Deprecation {
    fn needle_anchor(&self) -> String {
        format!("{}:{}", self.kind, self.probe.needle)
    }

    /// Display name for reports.
    pub fn construct(&self) -> String {
        self.id.clone().unwrap_or_else(|| self.needle_anchor())
    }

    pub fn classification(&self) -> Classification {
        if self.kind == "migration" {
            Classification::AutoMigratable
        } else {
            Classification::JudgmentCall
        }
    }

    pub fn guidance(&self) -> String {
        match self.classification() {
            Classification::AutoMigratable => format!(
                "Auto-migratable: the migration bundle transforms affected rows; no action needed before upgrading (removed no earlier than {}).",
                self.removal_target
            ),
            Classification::JudgmentCall => format!(
                "Judgment call: this org-authored value references a construct deprecated in {} and removed no earlier than {}. Choose the replacement before upgrading.",
                self.deprecated_in, self.removal_target
            ),
        }
    }
}

/// Segment glob: `*` matches exactly one dot-segment, `**` matches any
/// number of trailing segments, other segments are literal.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    let mut glob_segs: Vec<&str> = glob.split('.').collect();
    let path_segs: Vec<&str> = path.split('.').collect();
    // `**` must be the last segment when present (we only need tail-globs
    // for manifest scanning; anything else falls back to exact matching).
    if let Some(pos) = glob_segs.iter().position(|s| *s == "**") {
        let head = glob_segs.drain(..pos).collect::<Vec<_>>();
        if head.len() > path_segs.len() {
            return false;
        }
        return head
            .iter()
            .zip(path_segs.iter())
            .all(|(g, p)| seg_match(g, p));
    }
    glob_segs.len() == path_segs.len()
        && glob_segs
            .iter()
            .zip(path_segs.iter())
            .all(|(g, p)| seg_match(g, p))
}

fn seg_match(glob_seg: &str, path_seg: &str) -> bool {
    glob_seg == "*" || glob_seg == path_seg
}

/// Walks the manifest JSON and returns every string-coercible value under a
/// glob-matching path that contains the needle. Anchors are resolved later
/// (see [`resolve_anchor`]) because walking does not carry the root.
fn collect_touchpoints(glob: &str, needle: &str, value: &serde_json::Value) -> Vec<Touchpoint> {
    let mut found = Vec::new();
    walk(glob, String::new(), value, needle, &mut found);
    found
}

fn walk(
    glob: &str,
    path: String,
    value: &serde_json::Value,
    needle: &str,
    found: &mut Vec<Touchpoint>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{}{}", prefix(&path), key);
                if let Some(s) = scalar_str(child) {
                    if glob_matches(glob, &child_path) && s.contains(needle) {
                        found.push(Touchpoint {
                            path: child_path.clone(),
                            value: s,
                            anchor: None,
                        });
                    }
                }
                walk(glob, child_path, child, needle, found);
            }
        }
        serde_json::Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                let child_path = format!("{}{}", prefix(&path), i);
                if let Some(s) = scalar_str(child) {
                    if glob_matches(glob, &child_path) && s.contains(needle) {
                        found.push(Touchpoint {
                            path: child_path.clone(),
                            value: s,
                            anchor: None,
                        });
                    }
                }
                walk(glob, child_path, child, needle, found);
            }
        }
        _ => {}
    }
}

fn prefix(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("{path}.")
    }
}

fn scalar_str(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Human anchor for a touchpoint path: walk up from the matched scalar to
/// the nearest enclosing object carrying an identity field (name, key, url,
/// slug) — e.g. the automation rule's name or the webhook's URL.
fn resolve_anchor(root: &serde_json::Value, path: &str) -> Option<String> {
    let mut segs: Vec<&str> = path.split('.').collect();
    loop {
        if let Some(v) = navigate(root, &segs) {
            if let Some(id) = ["name", "key", "url", "slug"]
                .iter()
                .find_map(|k| v.get(k).and_then(scalar_str))
            {
                return Some(id);
            }
        }
        segs.pop()?;
    }
}

fn navigate<'a>(root: &'a serde_json::Value, segs: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for seg in segs {
        cur = match cur {
            serde_json::Value::Object(map) => map.get(*seg)?,
            serde_json::Value::Array(items) => items.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Loads the manifest JSON from disk.
pub fn load_manifest(path: &Path) -> Result<serde_json::Value, PreflightError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| PreflightError::Entry(path.display().to_string(), e.to_string()))?;
    serde_json::from_str(&raw).map_err(|e| PreflightError::Manifest(path.display().to_string(), e))
}

/// Default registry location relative to a checkout root.
pub const DEFAULT_REGISTRY_PATH: &str = "deprecations.json";

/// Scans the manifest for every registered deprecation and returns findings
/// with anchors resolved.
pub fn scan_manifest(registry: &Registry, manifest: &serde_json::Value) -> Vec<Finding> {
    let mut findings = Vec::new();
    for dep in &registry.deprecations {
        let mut touchpoints =
            collect_touchpoints(&dep.probe.path_glob, &dep.probe.needle, manifest);
        if touchpoints.is_empty() {
            continue;
        }
        for tp in &mut touchpoints {
            tp.anchor = resolve_anchor(manifest, &tp.path);
        }
        findings.push(Finding {
            construct: dep.construct(),
            kind: dep.kind.clone(),
            deprecated_in: dep.deprecated_in.clone(),
            removal_target: dep.removal_target.clone(),
            classification: dep.classification(),
            touchpoints,
            guidance: dep.guidance(),
            doc: dep.doc.clone(),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry_with(entries: serde_json::Value) -> Registry {
        serde_json::from_value(json!({ "version": 1, "deprecations": entries }))
            .expect("registry parses")
    }

    fn sample_manifest() -> serde_json::Value {
        json!({
            "manifest_version": 1,
            "org_id": "00000000-0000-4000-8000-000000000001",
            "generated_at": "2026-09-03T02:00:00Z",
            "sections": {
                "custom_fields": [
                    { "id": "f1", "entity": "ticket", "name": "Env", "kind": "select",
                      "config": { "options": ["Production", "Legacy Zone"] }, "required": true,
                      "sort_order": 0, "archived_at": null },
                    { "id": "f2", "entity": "client", "name": "Tier", "kind": "select",
                      "config": { "options": ["Gold"] }, "required": false,
                      "sort_order": 0, "archived_at": null }
                ],
                "boards": [
                    { "id": "b1", "name": "Desk", "is_default": true, "require_checklist_close": true,
                      "statuses": [ { "id": "s1", "name": "New", "category": "open", "sort_order": 0, "is_entry": true, "color": null } ],
                      "transitions": [] }
                ],
                "automation_rules": [
                    { "id": "a1", "board_id": null, "name": "Legacy pager", "trigger_event": "ticket.legacy_created",
                      "conditions": [], "actions": [ { "type": "notify" } ], "enabled": true }
                ],
                "webhooks": [
                    { "id": "w1", "url": "https://hooks.example/x", "event_types": ["ticket.legacy_created"], "enabled": true, "created_by": null }
                ],
                "api_keys": [],
                "settings": [
                    { "key": "search.engine", "value": "legacy_tsvector" }
                ],
                "sla": { "calendars": [], "policies": [] },
                "report_definitions": [
                    { "id": "r1", "name": "Old view report", "description": null, "kind": "legacy_tickets_view",
                      "spec": { "filters": {}, "group_by": "priority", "measures": ["count"] },
                      "viz": "bar", "scope": "org", "created_by": "u1",
                      "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z" }
                ]
            }
        })
    }

    #[test]
    fn empty_registry_produces_no_findings() {
        let reg = registry_with(json!([]));
        let findings = scan_manifest(&reg, &sample_manifest());
        assert!(findings.is_empty());
    }

    #[test]
    fn event_kind_finds_automation_trigger_and_webhook_filter() {
        let reg = registry_with(json!([{
            "id": "ticket.legacy_created",
            "kind": "event",
            "deprecated_in": "0.2.0",
            "removal_target": "0.4.0",
            "probe": { "path_glob": "sections.**", "needle": "ticket.legacy_created" }
        }]));
        let findings = scan_manifest(&reg, &sample_manifest());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.classification, Classification::JudgmentCall);
        // Touches BOTH the automation trigger and the webhook filter, with
        // human anchors from the sibling name/url fields.
        let paths: Vec<&str> = f.touchpoints.iter().map(|t| t.path.as_str()).collect();
        assert!(paths.contains(&"sections.automation_rules.0.trigger_event"));
        assert!(paths.contains(&"sections.webhooks.0.event_types.0"));
        let anchors: Vec<Option<&String>> =
            f.touchpoints.iter().map(|t| t.anchor.as_ref()).collect();
        assert!(anchors.contains(&Some(&"Legacy pager".to_string())));
        assert!(anchors.contains(&Some(&"https://hooks.example/x".to_string())));
    }

    #[test]
    fn migration_kind_is_auto_migratable() {
        let reg = registry_with(json!([{
            "id": "legacy_zone_option",
            "kind": "migration",
            "deprecated_in": "0.2.0",
            "removal_target": "0.3.0",
            "probe": { "path_glob": "sections.custom_fields.*.config.options.*", "needle": "Legacy Zone" }
        }]));
        let findings = scan_manifest(&reg, &sample_manifest());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].classification, Classification::AutoMigratable);
        assert_eq!(findings[0].touchpoints.len(), 1);
        assert_eq!(
            findings[0].touchpoints[0].path,
            "sections.custom_fields.0.config.options.1"
        );
    }

    #[test]
    fn generic_kind_scans_settings_values() {
        let reg = registry_with(json!([{
            "id": "legacy_tsvector",
            "kind": "generic",
            "deprecated_in": "0.2.0",
            "removal_target": "0.4.0",
            "probe": { "path_glob": "sections.settings.*.value", "needle": "legacy_tsvector" }
        }]));
        let findings = scan_manifest(&reg, &sample_manifest());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].touchpoints[0].path, "sections.settings.0.value");
    }

    #[test]
    fn probe_outside_glob_does_not_match() {
        let reg = registry_with(json!([{
            "id": "x",
            "kind": "event",
            "deprecated_in": "0.2.0",
            "removal_target": "0.4.0",
            "probe": { "path_glob": "sections.webhooks.*.url", "needle": "ticket.legacy_created" }
        }]));
        let findings = scan_manifest(&reg, &sample_manifest());
        assert!(
            findings.is_empty(),
            "needle exists in the manifest but OUTSIDE the probe glob"
        );
    }

    #[test]
    fn single_segment_glob_requires_exact_depth() {
        assert!(glob_matches("sections.settings", "sections.settings"));
        assert!(!glob_matches(
            "sections.settings",
            "sections.settings.0.value"
        ));
        assert!(glob_matches(
            "sections.settings.*.value",
            "sections.settings.0.value"
        ));
        assert!(!glob_matches(
            "sections.settings.*.value",
            "sections.settings.0.extra.value"
        ));
        assert!(glob_matches(
            "sections.**",
            "sections.sla.policies.0.targets"
        ));
        assert!(!glob_matches("other.**", "sections.settings"));
    }

    #[test]
    fn registry_rejects_unknown_kind_and_empty_needle() {
        let cases = [
            // Unknown kind must be rejected.
            json!({ "version": 1, "deprecations": [ {
                "id": "x", "kind": "widget", "deprecated_in": "0.2.0", "removal_target": "0.4.0",
                "probe": { "path_glob": "a", "needle": "b" } } ] }),
            // Empty needle can never match anything — reject at load.
            json!({ "version": 1, "deprecations": [ {
                "id": "x", "kind": "api", "deprecated_in": "0.2.0", "removal_target": "0.4.0",
                "probe": { "path_glob": "a", "needle": "" } } ] }),
        ];
        for (i, case) in cases.iter().enumerate() {
            let file = std::env::temp_dir().join(format!("reg-{}-{i}.json", std::process::id()));
            std::fs::write(&file, case.to_string()).unwrap();
            assert!(load_registry(&file).is_err(), "case {i} must be rejected");
            std::fs::remove_file(&file).ok();
        }
    }

    #[test]
    fn valid_registry_loads() {
        let good = json!({ "version": 1, "deprecations": [ {
            "id": "ticket.legacy_created", "kind": "event", "deprecated_in": "0.2.0",
            "removal_target": "0.4.0",
            "probe": { "path_glob": "sections.**", "needle": "ticket.legacy_created" } } ] });
        let file = std::env::temp_dir().join(format!("reg-good-{}.json", std::process::id()));
        std::fs::write(&file, good.to_string()).unwrap();
        let reg = load_registry(&file).expect("valid registry loads");
        assert_eq!(reg.deprecations.len(), 1);
        std::fs::remove_file(&file).ok();
    }
}
