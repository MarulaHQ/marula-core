//! The signed update manifest, and what "a security update" means
//! (Wave 60 lane W60-5).
//!
//! Purpose: one file the release publishes and the installer reads. It is
//! signed, so an update is an instruction from the project rather than from
//! whatever answered the connection, and it carries advisories, so `--check`
//! can say "a security update is available" and quote the reason.
//!
//! Non-purpose: it does not download anything and it does not decide whether to
//! install. The verifier in [`crate::update::artifact`] checks the bytes, and
//! the caller decides.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::update::UpdateError;

/// The only manifest shape this build understands. A newer schema is refused
/// rather than half-read: a manifest that says something this binary cannot
/// parse is not a document to guess at.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub channel: String,
    /// Every published release. Newest first is the convention; the code sorts.
    pub releases: Vec<Release>,
    #[serde(default)]
    pub advisories: Vec<Advisory>,
    /// A version below this must not run an update, and should not keep running
    /// at all.
    #[serde(default)]
    pub revoked_below: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub url: String,
    /// Lowercase hex SHA-256 of the artifact.
    pub sha256: String,
    #[serde(default)]
    pub published: Option<String>,
}

/// An advisory travels the same channel as a release. Severity is what makes
/// `--check` say the honest sentence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Advisory {
    pub id: String,
    pub severity: Severity,
    pub summary: String,
    /// Versions this advisory applies to. Empty means every release before
    /// `fixed_in`.
    #[serde(default)]
    pub affects: Vec<String>,
    pub fixed_in: String,
}

/// Least to most urgent: the ordering is what `max` picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Normal,
    High,
    Security,
}

impl Severity {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Severity::Normal => "normal",
            Severity::High => "high",
            Severity::Security => "security",
        }
    }
}

impl Manifest {
    /// Parses a manifest that has already had its signature checked.
    pub fn parse(text: &str) -> Result<Self, UpdateError> {
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|error| UpdateError::Manifest(error.to_string()))?;
        if manifest.schema_version != SCHEMA_VERSION {
            return Err(UpdateError::Schema {
                found: manifest.schema_version,
                understood: SCHEMA_VERSION,
            });
        }
        if manifest.releases.is_empty() {
            return Err(UpdateError::Manifest(
                "the manifest describes no releases".to_string(),
            ));
        }
        Ok(manifest)
    }

    /// The newest release for one platform, by the version comparator below.
    #[must_use]
    pub fn latest_for(&self, os: &str, arch: &str) -> Option<&Release> {
        self.releases
            .iter()
            .filter(|release| release.os == os && release.arch == arch)
            .max_by(|left, right| compare_versions(&left.version, &right.version))
    }

    /// Advisories that apply to `installed` and are fixed by `fixed_in`.
    #[must_use]
    pub fn advisories_for(&self, installed: &str) -> Vec<&Advisory> {
        self.advisories
            .iter()
            .filter(|advisory| {
                if compare_versions(&advisory.fixed_in, installed) != Ordering::Greater {
                    return false;
                }
                advisory.affects.is_empty()
                    || advisory
                        .affects
                        .iter()
                        .any(|version| compare_versions(version, installed) == Ordering::Equal)
            })
            .collect()
    }

    /// The most urgent severity among the advisories that apply.
    #[must_use]
    pub fn worst_severity(&self, installed: &str) -> Option<Severity> {
        self.advisories_for(installed)
            .into_iter()
            .map(|advisory| advisory.severity)
            .max()
    }
}

/// Compares two dotted versions numerically.
///
/// Purpose: `0.10.0` is newer than `0.9.0`, which a string comparison gets
/// backwards, and a pre-release suffix (`-rc.1`) sorts before the release it
/// previews.
///
/// Non-purpose: it is not semver's full grammar. This project ships `0.x`
/// releases with numeric components; a build metadatum is ignored, and a
/// component that is not a number is compared as text so an unexpected tag still
/// orders consistently instead of panicking.
#[must_use]
pub fn compare_versions(left: &str, right: &str) -> Ordering {
    let (left_core, left_pre) = split_prerelease(left);
    let (right_core, right_pre) = split_prerelease(right);

    let mut left_parts = left_core.split('.');
    let mut right_parts = right_core.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => break,
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(left), Some(right)) => {
                let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    _ => left.cmp(right),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
        }
    }

    // A release outranks its own pre-release.
    match (left_pre, right_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => left.cmp(right),
    }
}

fn split_prerelease(version: &str) -> (&str, Option<&str>) {
    let version = version.split('+').next().unwrap_or(version);
    match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"{
              "schema_version": 1,
              "channel": "stable",
              "releases": [
                {"version":"0.9.0","os":"macos","arch":"arm64","url":"https://x/0.9.0","sha256":"aa"},
                {"version":"0.10.0","os":"macos","arch":"arm64","url":"https://x/0.10.0","sha256":"bb"},
                {"version":"0.10.0","os":"linux","arch":"x64","url":"https://x/0.10.0-l","sha256":"cc"}
              ],
              "advisories": [
                {"id":"M-1","severity":"security","summary":"token leak in the worker","affects":["0.9.0"],"fixed_in":"0.10.0"},
                {"id":"M-2","severity":"high","summary":"slow sweep","affects":[],"fixed_in":"0.10.0"}
              ]
            }"#,
        )
        .expect("a manifest")
    }

    #[test]
    fn a_dotted_version_compares_numerically() {
        assert_eq!(compare_versions("0.10.0", "0.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("0.3.1", "0.3.1+build.7"), Ordering::Equal);
    }

    #[test]
    fn the_latest_release_is_found_per_platform() {
        let manifest = manifest();
        assert_eq!(
            manifest
                .latest_for("macos", "arm64")
                .map(|r| r.version.as_str()),
            Some("0.10.0")
        );
        assert_eq!(
            manifest
                .latest_for("linux", "x64")
                .map(|r| r.version.as_str()),
            Some("0.10.0")
        );
        assert!(manifest.latest_for("windows", "x64").is_none());
    }

    #[test]
    fn a_security_advisory_outranks_a_high_one() {
        let manifest = manifest();
        assert_eq!(manifest.worst_severity("0.9.0"), Some(Severity::Security));
    }

    #[test]
    fn an_advisory_the_installed_version_is_past_does_not_apply() {
        let manifest = manifest();
        assert!(manifest.advisories_for("0.10.0").is_empty());
        assert!(manifest.worst_severity("0.10.0").is_none());
    }

    #[test]
    fn a_manifest_from_a_newer_schema_is_refused_rather_than_half_read() {
        let refused = Manifest::parse(r#"{"schema_version":9,"channel":"stable","releases":[]}"#);
        assert!(matches!(refused, Err(UpdateError::Schema { found: 9, .. })));
    }

    #[test]
    fn a_manifest_with_no_releases_is_refused() {
        let refused = Manifest::parse(r#"{"schema_version":1,"channel":"stable","releases":[]}"#);
        assert!(refused.is_err());
    }
}
