//! The update engine (Wave 60 lane W60-5).
//!
//! Purpose: `marula update` reads a signed manifest, says whether the installed
//! version is behind, backs the install up, applies through the same
//! [`DeploymentTarget`] the installer uses, verifies the result, and puts the
//! files back if either the apply or the verdict fails.
//!
//! Non-purpose: it does not fetch. The caller downloads the manifest and the
//! artifact; this module decides what they mean, and refuses a manifest whose
//! signature does not hold.
//!
//! The rollback is honest about what it can restore: this engine owns the files
//! an install writes (`marula.env`, the compose files beside it, and a `VERSION`
//! marker). It cannot restore a database or an object store, and the summary
//! says so rather than implying a time machine.

pub mod artifact;
pub mod manifest;

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::install::plan::Plan;
use crate::install::target::{Applied, DeploymentTarget};

pub use artifact::{
    checksum_file, digest_hex, parse_sums, read_public_key, verify_signature, verify_signed_sums,
    verify_sums, SumEntry,
};
pub use manifest::{compare_versions, Advisory, Manifest, Release, Severity, SCHEMA_VERSION};

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("the manifest is not readable: {0}")]
    Manifest(String),
    #[error("this build understands manifest schema {understood}, and the file says {found}")]
    Schema { found: u32, understood: u32 },
    #[error("the signature does not hold: {0}")]
    Signature(String),
    #[error("no trusted keys were supplied, so nothing can be verified")]
    NoTrustedKeys,
    #[error("a checksum is wrong: {0}")]
    Sums(String),
    #[error("could not read or write {path}: {error}")]
    Io { path: PathBuf, error: String },
    #[error("{0}")]
    Refused(String),
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub installed: String,
    /// The newest release for this platform, when the manifest has one.
    pub latest: Option<String>,
    pub availability: Availability,
    /// Advisories that apply to the installed version, most urgent first.
    pub advisories: Vec<Advisory>,
    pub severity: Option<Severity>,
    /// The installed version is below the manifest's `revoked_below`.
    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    UpToDate,
    Available,
    /// The manifest publishes no artifact for this platform.
    NoRelease,
}

impl Check {
    /// The sentence `--check` prints. A security update says so, because
    /// "an update is available" when the update closes a vulnerability is the
    /// sentence that gets deferred.
    #[must_use]
    pub fn sentence(&self) -> String {
        if self.revoked {
            return format!(
                "{} is revoked by the manifest: nothing below {} may run. Update now.",
                self.installed,
                self.latest.as_deref().unwrap_or("the current release")
            );
        }
        match self.availability {
            Availability::UpToDate => format!("{} is current.", self.installed),
            Availability::NoRelease => format!(
                "the manifest publishes no release for this platform, so {} cannot be updated from it.",
                self.installed
            ),
            Availability::Available => {
                let version = self.latest.as_deref().unwrap_or("a newer release");
                match self.severity {
                    Some(Severity::Security) => format!(
                        "a security update is available: {version} (from {}). {}",
                        self.installed,
                        self.advisories
                            .iter()
                            .map(|advisory| format!("{}: {}", advisory.id, advisory.summary))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    ),
                    Some(Severity::High) => {
                        format!("a high-priority update is available: {version} (from {}).", self.installed)
                    }
                    _ => format!("an update is available: {version} (from {}).", self.installed),
                }
            }
        }
    }
}

/// Whether an update exists, and how urgent it is.
#[must_use]
pub fn check(manifest: &Manifest, installed: &str, os: &str, arch: &str) -> Check {
    let revoked = manifest
        .revoked_below
        .as_deref()
        .is_some_and(|floor| compare_versions(installed, floor).is_lt());
    let latest = manifest.latest_for(os, arch);
    let advisories: Vec<Advisory> = manifest
        .advisories_for(installed)
        .into_iter()
        .cloned()
        .collect();
    let severity = advisories.iter().map(|advisory| advisory.severity).max();

    let availability = match latest {
        None => Availability::NoRelease,
        Some(release) => {
            if compare_versions(&release.version, installed).is_gt() {
                Availability::Available
            } else {
                Availability::UpToDate
            }
        }
    };

    Check {
        installed: installed.to_string(),
        latest: latest.map(|release| release.version.clone()),
        availability,
        advisories,
        severity,
        revoked,
    }
}

/// The operating system this binary runs on, in the manifest's vocabulary.
#[must_use]
pub fn platform() -> (&'static str, &'static str) {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    (os, arch)
}

/// The version the install directory records, falling back to this binary's.
pub fn installed_version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("VERSION"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// What a backup holds: the files an install owns, and where each came from.
#[derive(Debug, Clone)]
pub struct Backup {
    pub dir: PathBuf,
    files: Vec<(PathBuf, PathBuf)>,
}

impl Backup {
    /// The files this backup would restore, for the summary a person reads.
    #[must_use]
    pub fn files(&self) -> Vec<String> {
        self.files
            .iter()
            .map(|(original, _)| original.display().to_string())
            .collect()
    }
}

/// Copies the install's own files into `<dir>/backups/<version>-<stamp>/`.
///
/// A directory with no `marula.env` is refused: backing up nothing and then
/// calling it a safety net is worse than saying there is nothing to update.
pub fn backup(dir: &Path, version: &str) -> Result<Backup, UpdateError> {
    let env = dir.join("marula.env");
    if !env.is_file() {
        return Err(UpdateError::Refused(format!(
            "{} holds no marula.env, so there is no install to update",
            dir.display()
        )));
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let target = dir.join("backups").join(format!("{version}-{stamp}"));
    std::fs::create_dir_all(&target).map_err(|error| UpdateError::Io {
        path: target.clone(),
        error: error.to_string(),
    })?;

    let mut files = Vec::new();
    let mut candidates = vec![env];
    for name in [
        "deploy/docker-compose.yml",
        "deploy/docker-compose.all-in-one.yml",
        "VERSION",
    ] {
        let path = dir.join(name);
        if path.is_file() {
            candidates.push(path);
        }
    }
    for original in candidates {
        let relative = original
            .strip_prefix(dir)
            .unwrap_or(&original)
            .to_path_buf();
        let copy = target.join(&relative);
        if let Some(parent) = copy.parent() {
            std::fs::create_dir_all(parent).map_err(|error| UpdateError::Io {
                path: parent.to_path_buf(),
                error: error.to_string(),
            })?;
        }
        std::fs::copy(&original, &copy).map_err(|error| UpdateError::Io {
            path: copy.clone(),
            error: error.to_string(),
        })?;
        files.push((original, copy));
    }

    Ok(Backup { dir: target, files })
}

/// Puts every backed-up file back where it came from.
pub fn restore(backup: &Backup) -> Result<(), UpdateError> {
    for (original, copy) in &backup.files {
        std::fs::copy(copy, original).map_err(|error| UpdateError::Io {
            path: original.clone(),
            error: error.to_string(),
        })?;
    }
    Ok(())
}

/// What an update did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub applied: Applied,
    /// `Some` when the change was undone, with the reason.
    pub rolled_back: Option<String>,
    /// What a rollback restored, when one ran.
    pub restored: Vec<String>,
    /// What still depends on a person, from the plan.
    pub manual: Vec<String>,
}

/// Applies a plan, verifies it, and undoes it if either step fails.
///
/// The order is the point: the backup is taken before the target is touched, the
/// verdict is taken before the update is called done, and a failure restores the
/// files rather than leaving a half-updated install behind a green message.
pub fn run(
    target: &dyn DeploymentTarget,
    plan: &Plan,
    backup: &Backup,
    verify: impl Fn() -> Result<(), String>,
) -> Outcome {
    let applied = target.apply(plan);
    if let Some(failed) = applied.failed.clone() {
        let restored = restore(backup).map(|()| backup.files()).unwrap_or_default();
        return Outcome {
            applied,
            rolled_back: Some(failed),
            restored,
            manual: plan.manual.clone(),
        };
    }

    if let Err(reason) = verify() {
        let restored = restore(backup).map(|()| backup.files()).unwrap_or_default();
        return Outcome {
            applied,
            rolled_back: Some(reason),
            restored,
            manual: plan.manual.clone(),
        };
    }

    Outcome {
        applied,
        rolled_back: None,
        restored: Vec::new(),
        manual: plan.manual.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::host::HostFacts;
    use crate::install::plan::{Plan, Step};
    use crate::install::target::Status;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"{
              "schema_version": 1,
              "channel": "stable",
              "revoked_below": "0.9.0",
              "releases": [
                {"version":"0.10.0","os":"macos","arch":"arm64","url":"https://x/a","sha256":"aa"}
              ],
              "advisories": [
                {"id":"M-1","severity":"security","summary":"token leak","affects":[],"fixed_in":"0.10.0"}
              ]
            }"#,
        )
        .expect("a manifest")
    }

    #[test]
    fn an_older_install_is_told_which_version_and_why_it_matters() {
        let check = check(&manifest(), "0.9.0", "macos", "arm64");
        assert_eq!(check.availability, Availability::Available);
        assert_eq!(check.severity, Some(Severity::Security));
        let sentence = check.sentence();
        assert!(sentence.contains("security update"), "{sentence}");
        assert!(sentence.contains("M-1"), "{sentence}");
    }

    #[test]
    fn a_current_install_says_so_in_five_words() {
        let check = check(&manifest(), "0.10.0", "macos", "arm64");
        assert_eq!(check.availability, Availability::UpToDate);
        assert_eq!(check.sentence(), "0.10.0 is current.");
    }

    #[test]
    fn an_install_below_the_revocation_floor_is_told_to_update_now() {
        let check = check(&manifest(), "0.8.0", "macos", "arm64");
        assert!(check.revoked);
        assert!(check.sentence().contains("revoked"), "{}", check.sentence());
    }

    #[test]
    fn a_platform_with_no_artifact_says_that_rather_than_offering_one() {
        let check = check(&manifest(), "0.9.0", "windows", "x64");
        assert_eq!(check.availability, Availability::NoRelease);
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marula-update-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("deploy")).expect("a scratch dir");
        std::fs::write(dir.join("marula.env"), "MARULA_SECRET_KEY=old\n").expect("env");
        std::fs::write(dir.join("VERSION"), "0.9.0\n").expect("version");
        dir
    }

    fn plan() -> Plan {
        Plan {
            target: "compose",
            profile: "split".to_string(),
            facts: HostFacts {
                cores: 4,
                ram_mb: Some(8192),
                free_disk_gb: Some(80),
                docker: Some("29.0".to_string()),
                gpu: None,
            },
            steps: vec![Step {
                kind: crate::install::plan::StepKind::Run {
                    program: "docker".to_string(),
                    args: vec!["compose".to_string(), "up".to_string()],
                },
                reason: "restart the stack".to_string(),
                source: "profiles.split".to_string(),
            }],
            manual: vec!["the backup destination".to_string()],
            secret_names: vec!["MARULA_SECRET_KEY".to_string()],
        }
    }

    struct FakeTarget {
        fail: bool,
    }

    impl DeploymentTarget for FakeTarget {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn available(&self, _host: &HostFacts) -> Result<(), crate::install::plan::PlanError> {
            Ok(())
        }
        fn plan(
            &self,
            _host: &HostFacts,
            _selection: &crate::install::plan::Selection,
        ) -> Result<Plan, crate::install::plan::PlanError> {
            Ok(plan())
        }
        fn apply(&self, plan: &Plan) -> Applied {
            if self.fail {
                Applied {
                    done: vec!["write marula.env".to_string()],
                    failed: Some("docker compose up exited 1".to_string()),
                    manual: plan.manual.clone(),
                }
            } else {
                Applied {
                    done: vec![
                        "write marula.env".to_string(),
                        "docker compose up".to_string(),
                    ],
                    failed: None,
                    manual: plan.manual.clone(),
                }
            }
        }
        fn status(&self) -> Status {
            Status {
                target: "fake",
                services: Vec::new(),
                error: None,
            }
        }
    }

    #[test]
    fn a_backup_copies_the_install_and_a_restore_puts_it_back() {
        let dir = scratch();
        let backup = backup(&dir, "0.9.0").expect("a backup");
        assert!(backup
            .files()
            .iter()
            .any(|path| path.ends_with("marula.env")));

        std::fs::write(dir.join("marula.env"), "MARULA_SECRET_KEY=new\n").expect("update");
        restore(&backup).expect("restore");
        let restored = std::fs::read_to_string(dir.join("marula.env")).expect("env");
        assert_eq!(restored, "MARULA_SECRET_KEY=old\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_without_an_install_cannot_be_backed_up() {
        let dir = std::env::temp_dir().join(format!("marula-empty-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        assert!(backup(&dir, "0.9.0").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_apply_restores_the_files_and_says_why() {
        let dir = scratch();
        let backup = backup(&dir, "0.9.0").expect("a backup");
        std::fs::write(dir.join("marula.env"), "MARULA_SECRET_KEY=new\n").expect("update");

        let outcome = run(&FakeTarget { fail: true }, &plan(), &backup, || Ok(()));
        assert!(outcome.rolled_back.is_some());
        assert!(!outcome.restored.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("marula.env")).expect("env"),
            "MARULA_SECRET_KEY=old\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_verdict_rolls_back_a_successful_apply() {
        let dir = scratch();
        let backup = backup(&dir, "0.9.0").expect("a backup");
        std::fs::write(dir.join("marula.env"), "MARULA_SECRET_KEY=new\n").expect("update");

        let outcome = run(&FakeTarget { fail: false }, &plan(), &backup, || {
            Err("the doctor says the API is down".to_string())
        });
        assert_eq!(
            outcome.rolled_back.as_deref(),
            Some("the doctor says the API is down")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("marula.env")).expect("env"),
            "MARULA_SECRET_KEY=old\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clean_apply_and_verdict_leaves_the_files_alone() {
        let dir = scratch();
        let backup = backup(&dir, "0.9.0").expect("a backup");
        let outcome = run(&FakeTarget { fail: false }, &plan(), &backup, || Ok(()));
        assert!(outcome.rolled_back.is_none());
        assert!(outcome.restored.is_empty());
        assert_eq!(outcome.manual, vec!["the backup destination".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
