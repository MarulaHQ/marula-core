//! `marula doctor` (Wave 60 lane W60-2).
//!
//! The same facts the planner uses, read back as a verdict. Every check says
//! what it looked at and what it found, and a failing check names the fix
//! rather than the symptom: "MARULA_SECRET_KEY is empty in marula.env" is
//! actionable, "configuration invalid" is not.

use std::collections::BTreeMap;
use std::net::TcpListener;

use crate::deploy::{DeployDescriptor, Profile};

use super::host::HostFacts;
use super::plan::{embedding_fits, missing_for};

/// One check's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            detail: detail.into(),
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            detail: detail.into(),
        }
    }

    /// One line, prefixed so a reader can scan a list of them.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "  {} {:<28} {}",
            if self.ok { "ok  " } else { "FAIL" },
            self.name,
            self.detail
        )
    }
}

/// The ports a profile expects to be free before it starts.
#[must_use]
pub fn profile_ports(profile: &Profile) -> Vec<u16> {
    let mut ports: Vec<u16> = profile
        .services
        .iter()
        .flat_map(|service| service.ports.clone())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Reads a `KEY=value` env file the way a shell would for the subset this file
/// contains: comments and blank lines skipped, no expansion.
#[must_use]
pub fn parse_env_file(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

/// Whether a port can be bound. Injected so a test does not depend on what the
/// machine running it happens to be listening on.
pub type PortProbe = fn(u16) -> Result<(), String>;

/// The real probe: try to bind loopback.
pub fn probe_port(port: u16) -> Result<(), String> {
    TcpListener::bind(("127.0.0.1", port))
        .map(|_| ())
        .map_err(|error| format!("held by another process ({error})"))
}

/// Runs every check that needs no network call, against the real ports.
#[must_use]
pub fn run_default(
    desc: &DeployDescriptor,
    profile: &Profile,
    facts: &HostFacts,
    env: &BTreeMap<String, String>,
) -> Vec<Check> {
    run(desc, profile, facts, env, probe_port)
}

/// Runs every check, with the port probe supplied by the caller.
///
/// `env` is the environment the install will run with: the parsed file when one
/// exists, or the process environment for a deployment that already runs.
#[must_use]
pub fn run(
    desc: &DeployDescriptor,
    profile: &Profile,
    facts: &HostFacts,
    env: &BTreeMap<String, String>,
    probe: PortProbe,
) -> Vec<Check> {
    let mut checks = Vec::new();

    let missing = missing_for(profile, facts);
    checks.push(if missing.is_empty() {
        Check::pass(
            "host floors",
            format!(
                "{} cores, {} memory, {} free disk",
                facts.cores,
                facts
                    .ram_mb
                    .map_or("unknown".to_string(), |mb| format!("{mb} MB")),
                facts
                    .free_disk_gb
                    .map_or("unknown".to_string(), |gb| format!("{gb} GB"))
            ),
        )
    } else {
        Check::fail("host floors", missing.join("; "))
    });

    checks.push(match &facts.docker {
        Some(version) => Check::pass("docker", format!("server {version}")),
        None => Check::fail(
            "docker",
            "`docker version` did not answer; run the daemon, or install a binary target",
        ),
    });

    let required: Vec<&str> = desc
        .params
        .iter()
        .filter(|param| param.required)
        .map(|param| param.name)
        .collect();
    let empty: Vec<&str> = required
        .iter()
        .copied()
        .filter(|name| env.get(*name).is_none_or(|value| value.trim().is_empty()))
        .collect();
    checks.push(if empty.is_empty() {
        Check::pass("required values", format!("{} present", required.len()))
    } else {
        Check::fail(
            "required values",
            format!(
                "empty and the process will refuse to boot: {}",
                empty.join(", ")
            ),
        )
    });

    for port in profile_ports(profile) {
        checks.push(match probe(port) {
            Ok(()) => Check::pass(format!("port {port}"), "free"),
            Err(reason) => Check::fail(
                format!("port {port}"),
                format!("{reason}; stop it or choose another host"),
            ),
        });
    }

    checks.push(if embedding_fits(profile, facts) {
        Check::pass(
            "embedding model",
            "the profile's local model fits this host",
        )
    } else {
        Check::fail(
            "embedding model",
            "the profile's local model needs more memory than this host reports; \
             set MARULA_EMBEDDING_PROVIDER=openai and an instance key, or raise the host",
        )
    });

    checks
}

/// Whether every check passed.
#[must_use]
pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|check| check.ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy;

    fn profile(name: &str) -> crate::deploy::Profile {
        deploy::descriptor()
            .profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .expect("a shipped profile")
    }

    fn facts(cores: u16, ram_mb: u32, docker: bool) -> HostFacts {
        HostFacts {
            cores,
            ram_mb: Some(ram_mb),
            free_disk_gb: Some(200),
            docker: docker.then(|| "27.0.0".to_string()),
            gpu: None,
        }
    }

    fn env_with_required() -> BTreeMap<String, String> {
        let desc = deploy::descriptor();
        desc.params
            .iter()
            .filter(|param| param.required)
            .map(|param| (param.name.to_string(), "x".repeat(64)))
            .collect()
    }

    #[test]
    fn an_env_file_parses_without_comments_or_expansion() {
        let parsed = parse_env_file(
            "# a comment\n\nMARULA_ALL_IN_ONE=1\nDATABASE_URL=postgres://x:${Y}@h/db\n",
        );
        assert_eq!(
            parsed.get("MARULA_ALL_IN_ONE").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            parsed.get("DATABASE_URL").map(String::as_str),
            Some("postgres://x:${Y}@h/db"),
            "the value is data, not a template to expand"
        );
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn a_healthy_install_passes_every_check() {
        let desc = deploy::descriptor();
        let checks = run(
            &desc,
            &profile("all-in-one"),
            &facts(8, 16384, true),
            &env_with_required(),
            |_| Ok(()),
        );
        assert!(
            all_ok(&checks),
            "{:?}",
            checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_empty_required_value_fails_and_names_it() {
        let desc = deploy::descriptor();
        let mut env = env_with_required();
        env.insert("MARULA_SECRET_KEY".to_string(), String::new());
        let checks = run(
            &desc,
            &profile("all-in-one"),
            &facts(8, 16384, true),
            &env,
            |_| Ok(()),
        );
        let failed = checks.iter().find(|check| !check.ok).expect("a failure");
        assert!(
            failed.detail.contains("MARULA_SECRET_KEY"),
            "{}",
            failed.detail
        );
    }

    #[test]
    fn a_held_port_is_a_named_failure_with_the_way_out() {
        let desc = deploy::descriptor();
        let checks = run(
            &desc,
            &profile("all-in-one"),
            &facts(8, 16384, true),
            &env_with_required(),
            |port| Err(format!("held by test ({port})")),
        );
        let port = checks
            .iter()
            .find(|check| check.name == "port 8080")
            .expect("the 8080 check");
        assert!(!port.ok);
        assert!(port.detail.contains("stop it or choose another host"));
    }

    #[test]
    fn a_two_core_host_fails_the_floor_check_with_the_numbers() {
        let desc = deploy::descriptor();
        let checks = run(
            &desc,
            &profile("split"),
            &facts(2, 1024, true),
            &env_with_required(),
            |_| Ok(()),
        );
        let floors = checks
            .iter()
            .find(|check| check.name == "host floors")
            .expect("the floors check");
        assert!(!floors.ok);
        assert!(floors.detail.contains("cores"));
    }

    #[test]
    fn no_docker_is_a_named_failure_not_a_crash() {
        let desc = deploy::descriptor();
        let checks = run(
            &desc,
            &profile("all-in-one"),
            &facts(8, 16384, false),
            &env_with_required(),
            |_| Ok(()),
        );
        let docker = checks
            .iter()
            .find(|check| check.name == "docker")
            .expect("the docker check");
        assert!(!docker.ok);
        assert!(docker.detail.contains("daemon"));
    }
}
