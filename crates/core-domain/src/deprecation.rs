//! Deprecation metadata and the removal clock (policy §3/§8, the private build tracker U4).
//!
//! Purpose: the typed model behind `deprecations.json` — every deprecated
//! construct (API surface, domain event, migration construct, or anything
//! else greppable) records when it was deprecated and the version it may be
//! removed in. The clock rule is mechanical: a construct is removed no
//! earlier than its `removal_target`, and nothing survives past it.
//!
//! This module is PURE (crate doctrine): parsing, version math, and verdict
//! logic only. Probing the tree for a construct's presence is injected as a
//! closure, so the build-time gate (core-events/tests/deprecation_clock.rs)
//! owns the file I/O and this module stays unit-testable without a tree.
//!
//! Registry source of truth: `deprecations.json` at the repo root. The
//! release procedure for a deprecation is (policy §3): record it here +
//! release notes, keep it working through the window, remove in the target.

use serde::{Deserialize, Serialize};

/// Minimal semver triple. We deliberately do not depend on a semver crate:
/// the policy only ever compares releases and reads the 0.x boundary.
/// Serializes as a plain `"X.Y.Z"` string — the registry document format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl From<Version> for String {
    fn from(v: Version) -> Self {
        v.to_string()
    }
}

impl TryFrom<String> for Version {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses `X.Y.Z` (whitespace-tolerant, no pre-release/build tags —
    /// the policy's release tags are plain triples).
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let fail = || format!("invalid version `{s}` (expected X.Y.Z)");
        let mut it = s.split('.');
        let major = it.next().ok_or_else(fail)?;
        let minor = it.next().ok_or_else(fail)?;
        let patch = it.next().ok_or_else(fail)?;
        if it.next().is_some() {
            return Err(fail());
        }
        let num = |p: &str| -> Result<u64, String> {
            p.parse::<u64>()
                .map_err(|_| format!("invalid version `{s}` (expected X.Y.Z)"))
        };
        Ok(Self {
            major: num(major)?,
            minor: num(minor)?,
            patch: num(patch)?,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Which stability surface (policy §2) the construct lives on. The kind
/// carries no behavior by itself — it documents the registry and lets the
/// gate tailor diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstructKind {
    /// A REST route / operationId on the versioned API surface.
    Api,
    /// A domain event type on the events & webhooks surface.
    Event,
    /// A schema construct (table/column) reachable through migrations.
    Migration,
    /// Anything else greppable (settings key, CLI flag, …).
    Generic,
}

/// How the gate decides whether a construct is still present: some file in
/// the tree, matched by a tiny glob (`*` wildcard in the last path segment
/// only), must contain `needle`. Keep needles distinctive — a probe that
/// matches unrelated text makes the clock lie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    /// Repo-root-relative glob, e.g. `crates/core-db/migrations/*.sql`.
    pub path_glob: String,
    /// Substring that must appear in at least one matching file.
    pub needle: String,
}

impl Probe {
    /// True when `path` (repo-root-relative, `/`-separated) matches this
    /// probe's glob: literal prefix, single `*`, literal suffix.
    pub fn matches_path(&self, path: &str) -> bool {
        let pat = &self.path_glob;
        match pat.split_once('*') {
            None => pat == path,
            Some((prefix, suffix)) => {
                path.len() >= prefix.len() + suffix.len()
                    && path.starts_with(prefix)
                    && path.ends_with(suffix)
            }
        }
    }
}

/// Convenience probe constructors with the conventional default globs —
/// entries may still override `path_glob` explicitly.
impl Probe {
    pub fn api(needle: impl Into<String>) -> Self {
        Self {
            path_glob: "packages/sdk/openapi.json".into(),
            needle: needle.into(),
        }
    }

    pub fn event(needle: impl Into<String>) -> Self {
        Self {
            path_glob: "crates/core-domain/src/events.rs".into(),
            needle: needle.into(),
        }
    }

    pub fn migration(needle: impl Into<String>) -> Self {
        Self {
            path_glob: "crates/core-db/migrations/*.sql".into(),
            needle: needle.into(),
        }
    }
}

/// One deprecated construct (policy §3): announced in `deprecated_in`,
/// removable no earlier than `removal_target` — always a later generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deprecation {
    /// Stable identifier (for events, the event type string; otherwise a
    /// short kebab-case handle). Unique across the registry.
    pub id: String,
    pub kind: ConstructKind,
    /// Version that announced the deprecation (release-notes entry, §3a).
    pub deprecated_in: Version,
    /// Earliest version allowed to remove it (§3c).
    pub removal_target: Version,
    /// What replaces it, if anything. Events: the co-emitting replacement
    /// type (U2). Free text for other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    pub probe: Probe,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Top-level registry document (`deprecations.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecationRegistry {
    /// Format version, currently 1.
    pub version: u32,
    #[serde(default)]
    pub deprecations: Vec<Deprecation>,
}

impl DeprecationRegistry {
    pub fn parse(json: &str) -> Result<Self, String> {
        let reg: Self =
            serde_json::from_str(json).map_err(|e| format!("deprecations.json: {e}"))?;
        reg.validate()?;
        Ok(reg)
    }

    /// Structural validation independent of the clock: unique ids, sane
    /// versions, and the one-generation rule between the two stamps.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for d in &self.deprecations {
            if d.id.trim().is_empty() {
                return Err("deprecation entry with empty id".into());
            }
            if !seen.insert(d.id.as_str()) {
                return Err(format!("duplicate deprecation id `{}`", d.id));
            }
            if d.probe.needle.is_empty() {
                return Err(format!("deprecation `{}` has an empty probe needle", d.id));
            }
            if !generation_ok(&d.deprecated_in, &d.removal_target) {
                return Err(format!(
                    "deprecation `{}`: removal_target {} is not a later generation than deprecated_in {} (policy §3: a subsequent major; pre-GA 0.x may step one minor)",
                    d.id, d.removal_target, d.deprecated_in
                ));
            }
        }
        Ok(())
    }

    /// The clock check (policy §8). `current` is the shipping version;
    /// `present` reports whether the probe still matches the tree.
    /// Returns one violation per construct in the wrong half of its window.
    pub fn check_clock(
        &self,
        current: &Version,
        mut present: impl FnMut(&Probe) -> bool,
    ) -> Vec<ClockViolation> {
        let mut out = Vec::new();
        for d in &self.deprecations {
            let here = present(&d.probe);
            let due = current >= &d.removal_target;
            match (here, due) {
                // Window open, construct still shipped: correct.
                (true, false) => {}
                // Window elapsed and construct removed: correct (prune the
                // entry in the same PR that removes the construct).
                (false, true) => {}
                (true, true) => out.push(ClockViolation {
                    id: d.id.clone(),
                    reason: ClockReason::Overdue,
                    detail: format!(
                        "removal_target {} has shipped but the construct is still present — remove it now (or, if the removal slipped a release on purpose, move removal_target forward with a release-notes entry)",
                        d.removal_target
                    ),
                }),
                (false, false) => out.push(ClockViolation {
                    id: d.id.clone(),
                    reason: ClockReason::RemovedEarly,
                    detail: format!(
                        "construct is gone but its removal_target is {} and this build is {} — removals before the target break the one-generation promise (policy §1/§3); restore it or move the target with an explicit policy exception",
                        d.removal_target, current
                    ),
                }),
            }
        }
        out
    }
}

/// A construct on the wrong side of its window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockViolation {
    pub id: String,
    pub reason: ClockReason,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockReason {
    /// Still present after `removal_target` shipped.
    Overdue,
    /// Gone before `removal_target`.
    RemovedEarly,
}

/// The one-generation rule (policy §1/§3): the target must be a subsequent
/// major. Pre-GA (0.x), where majors do not exist yet, breaking changes ride
/// minor boundaries (§3), so a 0.x deprecation may target a later 0.x minor
/// or anything from 1.0 on.
pub fn generation_ok(deprecated_in: &Version, removal_target: &Version) -> bool {
    if removal_target <= deprecated_in {
        return false;
    }
    if deprecated_in.major == 0 {
        removal_target.major > 0
            || (removal_target.major == 0 && removal_target.minor > deprecated_in.minor)
    } else {
        removal_target.major > deprecated_in.major
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(entries: Vec<Deprecation>) -> DeprecationRegistry {
        DeprecationRegistry {
            version: 1,
            deprecations: entries,
        }
    }

    fn dep(id: &str, dep_in: Version, target: Version, needle: &str) -> Deprecation {
        Deprecation {
            id: id.into(),
            kind: ConstructKind::Event,
            deprecated_in: dep_in,
            removal_target: target,
            replaced_by: None,
            probe: Probe::event(needle),
            notes: None,
        }
    }

    #[test]
    fn version_parse_and_order() {
        assert_eq!(Version::parse("0.1.0").unwrap(), Version::new(0, 1, 0));
        assert_eq!(Version::parse(" 1.10.3 ").unwrap(), Version::new(1, 10, 3));
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
        assert!(Version::parse("x.y.z").is_err());
        assert!(Version::new(1, 0, 0) > Version::new(0, 99, 99));
        assert!(Version::new(0, 2, 0) > Version::new(0, 1, 9));
    }

    #[test]
    fn generation_rule_matches_policy() {
        let v = Version::new;
        // Same version or backwards: never.
        assert!(!generation_ok(&v(1, 0, 0), &v(1, 0, 0)));
        assert!(!generation_ok(&v(1, 2, 0), &v(1, 1, 0)));
        // Post-GA: must step the major.
        assert!(generation_ok(&v(1, 0, 0), &v(2, 0, 0)));
        assert!(!generation_ok(&v(1, 0, 0), &v(1, 9, 0)));
        // Pre-GA: minor boundary counts as a generation, 1.0+ always fine.
        assert!(generation_ok(&v(0, 1, 0), &v(0, 2, 0)));
        assert!(generation_ok(&v(0, 1, 0), &v(1, 0, 0)));
        assert!(!generation_ok(&v(0, 1, 0), &v(0, 1, 5)));
    }

    #[test]
    fn probe_glob_matching_is_prefix_star_suffix() {
        let p = Probe {
            path_glob: "crates/core-db/migrations/*.sql".into(),
            needle: "old_thing".into(),
        };
        assert!(p.matches_path("crates/core-db/migrations/0001_x.sql"));
        assert!(!p.matches_path("crates/core-db/migrations/0001_x.sql.bak"));
        assert!(!p.matches_path("crates/core-db/src/lib.rs"));
        let exact = Probe {
            path_glob: "packages/sdk/openapi.json".into(),
            needle: "x".into(),
        };
        assert!(exact.matches_path("packages/sdk/openapi.json"));
        assert!(!exact.matches_path("packages/sdk/openapi.json.orig"));
    }

    #[test]
    fn validate_rejects_bad_entries() {
        let v = Version::new;
        let mut bad = reg(vec![dep("a", v(0, 1, 0), v(0, 2, 0), "n")]);
        bad.deprecations.push(dep("a", v(0, 1, 0), v(0, 2, 0), "n"));
        assert!(bad.validate().is_err(), "duplicate ids rejected");

        let bad_gen = reg(vec![dep("a", v(0, 1, 0), v(0, 1, 2), "n")]);
        assert!(
            bad_gen.validate().is_err(),
            "same-generation target rejected"
        );

        let mut empty_needle = dep("a", v(0, 1, 0), v(0, 2, 0), "");
        empty_needle.probe.needle = String::new();
        assert!(reg(vec![empty_needle]).validate().is_err());

        assert!(reg(vec![dep("a", v(0, 1, 0), v(0, 2, 0), "n")])
            .validate()
            .is_ok());
    }

    #[test]
    fn clock_enforces_both_directions() {
        let v = Version::new;
        let r = reg(vec![
            dep("in-window", v(0, 1, 0), v(0, 3, 0), "still_here"),
            dep("overdue", v(0, 1, 0), v(0, 2, 0), "should_be_gone"),
            dep("removed-early", v(0, 1, 0), v(0, 4, 0), "already_gone"),
            dep("properly-retired", v(0, 1, 0), v(0, 2, 0), "gone_on_time"),
        ]);
        let current = v(0, 2, 5);
        // present: only the two constructs still in the tree.
        let violations = r.check_clock(&current, |p| {
            matches!(p.needle.as_str(), "still_here" | "should_be_gone")
        });
        let ids: Vec<&str> = violations.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["overdue", "removed-early"]);
        assert_eq!(violations[0].reason, ClockReason::Overdue);
        assert_eq!(violations[1].reason, ClockReason::RemovedEarly);
    }

    #[test]
    fn parse_round_trips_registry_document() {
        let doc = r#"{
            "version": 1,
            "deprecations": [
                {
                    "id": "event.legacy_ping",
                    "kind": "event",
                    "deprecated_in": "0.1.0",
                    "removal_target": "0.3.0",
                    "replaced_by": "event.pong",
                    "probe": { "path_glob": "crates/core-domain/src/events.rs", "needle": "LEGACY_PING" },
                    "notes": "co-emits with event.pong for one generation"
                }
            ]
        }"#;
        let r = DeprecationRegistry::parse(doc).unwrap();
        assert_eq!(r.deprecations.len(), 1);
        assert_eq!(r.deprecations[0].kind, ConstructKind::Event);
        assert_eq!(r.deprecations[0].replaced_by.as_deref(), Some("event.pong"));
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(DeprecationRegistry::parse(&json).unwrap(), r);
    }

    #[test]
    fn parse_rejects_malformed_documents() {
        assert!(DeprecationRegistry::parse("not json").is_err());
        assert!(DeprecationRegistry::parse("{\"version\":2,\"deprecations\":[]}").is_ok());
        let bad_target = r#"{"version":1,"deprecations":[{"id":"x","kind":"api","deprecated_in":"0.2.0","removal_target":"0.1.0","probe":{"path_glob":"a","needle":"b"}}]}"#;
        assert!(DeprecationRegistry::parse(bad_target).is_err());
    }
}
