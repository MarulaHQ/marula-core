//! The deploy descriptor (Wave 49 lane R7) — the interface marula-hq consumes.
//!
//! One image runs every role, and a deployment is parameterized by runtime env
//! vars. This module is the machine-readable description of that contract: the
//! image and its roles, the deployment profiles, and the full catalogue of
//! runtime parameters with their kinds, defaults and grouping.
//!
//! Rust is the single source of truth. `marula deploy describe --json` emits
//! the descriptor for hq to pin; `marula deploy profiles --write` regenerates
//! `deploy/profiles/*.yaml`, and a test asserts the checked-in files match this
//! code so the human-readable copy cannot drift from the contract.
//!
//! No hq logic lives here. marula-core states the interface; hq decides what to
//! render from it.

use std::path::{Path, PathBuf};

/// Schema version of the emitted descriptor. Bump only on a breaking shape
/// change, so a consumer can refuse a version it does not understand.
pub const SCHEMA_VERSION: u32 = 1;

/// The one image and the roles it can run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageSpec {
    pub name: &'static str,
    /// Role names, one per entrypoint (`BIN` build arg).
    pub roles: Vec<&'static str>,
    pub dockerfile: &'static str,
    /// Build arg that selects the role.
    pub role_arg: &'static str,
}

/// A process in a profile's topology.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Service {
    pub name: &'static str,
    /// The image role this service runs.
    pub role: &'static str,
    pub ports: Vec<u16>,
    /// Infrastructure it needs up first.
    pub needs: Vec<&'static str>,
}

/// A named deployment shape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Profile {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub services: Vec<Service>,
    /// Parameters this profile presets, name to value. Everything not listed
    /// keeps its catalogue default.
    pub sets: std::collections::BTreeMap<&'static str, &'static str>,
}

/// The kind of a runtime parameter, for a consumer to render a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    String,
    Integer,
    Boolean,
    Url,
    Secret,
}

/// One runtime parameter in the deployment surface.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Param {
    pub name: &'static str,
    pub group: &'static str,
    pub kind: ParamKind,
    /// `true` when the process refuses to boot without it (no default).
    pub required: bool,
    /// `None` when there is no default.
    pub default: Option<&'static str>,
    /// `true` for key material a consumer must mask.
    pub secret: bool,
    pub description: &'static str,
}

/// The whole descriptor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeployDescriptor {
    pub schema_version: u32,
    /// Core release this descriptor was built from; a consumer pins against it.
    pub core_version: &'static str,
    pub image: ImageSpec,
    pub profiles: Vec<Profile>,
    pub params: Vec<Param>,
}

fn param(
    name: &'static str,
    group: &'static str,
    kind: ParamKind,
    default: Option<&'static str>,
    secret: bool,
    description: &'static str,
) -> Param {
    Param {
        name,
        group,
        kind,
        // Not derived from `secret`: most secrets are conditional (only
        // needed for the store, transport or provider they configure).
        required: false,
        default,
        secret,
        description,
    }
}

/// A parameter the process refuses to boot without. Today that is the two
/// boot-critical secrets, checked in `auth`/`secrets`; every other secret is
/// conditional on the feature it configures.
fn requires(
    name: &'static str,
    group: &'static str,
    kind: ParamKind,
    default: Option<&'static str>,
    secret: bool,
    description: &'static str,
) -> Param {
    Param {
        required: true,
        ..param(name, group, kind, default, secret, description)
    }
}

/// The full descriptor, built fresh (cheap, all static).
#[must_use]
pub fn descriptor() -> DeployDescriptor {
    DeployDescriptor {
        schema_version: SCHEMA_VERSION,
        core_version: env!("CARGO_PKG_VERSION"),
        image: ImageSpec {
            name: "marula-core",
            roles: vec!["core-api", "worker", "marula-core-cli"],
            dockerfile: "deploy/Dockerfile.rust",
            role_arg: "BIN",
        },
        profiles: profiles(),
        params: params(),
    }
}

fn profiles() -> Vec<Profile> {
    use std::collections::BTreeMap;

    let split = Profile {
        name: "split",
        title: "Split services",
        description: "The default topology: core-api and worker run as separate processes so each scales and restarts on its own. The compose stack ships this shape.",
        services: vec![
            Service {
                name: "core-api",
                role: "core-api",
                ports: vec![8080],
                needs: vec!["postgres", "redis"],
            },
            Service {
                name: "worker",
                role: "worker",
                ports: vec![8082],
                needs: vec!["postgres", "redis"],
            },
        ],
        sets: BTreeMap::new(),
    };

    let all_in_one = Profile {
        name: "all-in-one",
        title: "All-in-one (demo / one-box)",
        description: "One core-api process hosts the worker loops and serves the SSE live-push stream, so a small box runs a single application container. See docs/all-in-one.md.",
        services: vec![Service {
            name: "core-api",
            role: "core-api",
            ports: vec![8080, 8082],
            needs: vec!["postgres", "redis"],
        }],
        sets: BTreeMap::from([
            ("MARULA_ALL_IN_ONE", "1"),
            (
                "MARULA_WORKER_DATABASE_URL",
                "postgres://marula:marula@postgres:5432/marula",
            ),
            ("EMAIL_POLL_SECS", "60"),
            ("QUEUE_IDLE_MS", "60000"),
            ("EVAL_INTERVAL_SECS", "300"),
            ("TIME_AUDIT_POLL_SECS", "300"),
            ("DELIVERY_POLL_SECS", "300"),
            ("USAGE_REFRESH_SECS", "900"),
            ("DIGEST_POLL_SECS", "3600"),
            ("RETENTION_SWEEP_SECS", "86400"),
        ]),
    };

    vec![split, all_in_one]
}

#[allow(clippy::too_many_lines)]
fn params() -> Vec<Param> {
    use ParamKind::{Boolean, Integer, Secret, String, Url};
    vec![
        // topology
        param(
            "MARULA_ALL_IN_ONE",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Host the worker loops inside core-api.",
        ),
        param(
            "BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8080"),
            false,
            "core-api HTTP listen address.",
        ),
        param(
            "MARULA_WORKER_BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8082"),
            false,
            "Worker health-endpoint address in all-in-one mode.",
        ),
        param(
            "PUBLIC_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:8080"),
            false,
            "Public base URL of the API, used to build OIDC redirect URIs.",
        ),
        param(
            "WEB_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:3000"),
            false,
            "Web app base URL; the SSO callback lands the browser here.",
        ),
        param(
            "CORS_ORIGIN",
            "topology",
            String,
            None,
            false,
            "Allowed browser origin for the API. Unset disables CORS.",
        ),
        // database
        param(
            "DATABASE_URL",
            "database",
            Url,
            Some("postgres://marula_app:marula_app@localhost:15432/marula"),
            false,
            "RLS-scoped app-role Postgres URL. Set it in production.",
        ),
        param(
            "MARULA_WORKER_DATABASE_URL",
            "database",
            Url,
            None,
            false,
            "Owner-role Postgres URL for the worker loops. Required when MARULA_ALL_IN_ONE=1.",
        ),
        param(
            "MARULA_DB_POOL_MAX",
            "database",
            Integer,
            Some("20"),
            false,
            "App-pool ceiling. Sizes both the API app pool and, in all-in-one, the worker pool.",
        ),
        param(
            "MARULA_REPORTING_POOL_MAX",
            "database",
            Integer,
            Some("25"),
            false,
            "Reporting-pool ceiling (app role under RLS).",
        ),
        param(
            "DELIVERY_POOL_MAX",
            "database",
            Integer,
            Some("5"),
            false,
            "Worker delivery reporting-pool ceiling.",
        ),
        // cache
        param(
            "REDIS_URL",
            "cache",
            Url,
            Some("redis://localhost:16379/0"),
            false,
            "Redis/Valkey URL for the outbox relay and stream consumers.",
        ),
        // secrets
        requires(
            "JWT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. Access/refresh token signing. Boot refuses without it outside dev.",
        ),
        requires(
            "MARULA_SECRET_KEY",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. AES-256 at-rest key for sealed integration secrets.",
        ),
        param(
            "MARULA_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Bearer token for /api/internal/* (worker to API machine surface).",
        ),
        param(
            "EMAIL_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Shared token for the email claim/report loop.",
        ),
        param(
            "EMAIL_INBOUND_HMAC_SECRET",
            "secrets",
            Secret,
            None,
            false,
            "HMAC secret for the inbound email webhook. Unset disables it.",
        ),
        param(
            "MARULA_PLUGIN_SIGNING_KEYS",
            "secrets",
            String,
            None,
            false,
            "Trusted plugin-signing public keys (key_id=hex, comma-separated).",
        ),
        // objects
        param(
            "OBJECT_STORE",
            "objects",
            String,
            Some("local"),
            false,
            "Attachment store: local or s3.",
        ),
        param(
            "OBJECT_STORE_LOCAL_DIR",
            "objects",
            String,
            Some("./data/objects"),
            false,
            "Local object-store directory.",
        ),
        param(
            "S3_ENDPOINT",
            "objects",
            Url,
            None,
            false,
            "S3/MinIO endpoint when OBJECT_STORE=s3.",
        ),
        param(
            "S3_BUCKET",
            "objects",
            String,
            Some("marula"),
            false,
            "S3 bucket name.",
        ),
        param(
            "S3_REGION",
            "objects",
            String,
            Some("us-east-1"),
            false,
            "S3 region.",
        ),
        param(
            "S3_ACCESS_KEY_ID",
            "objects",
            Secret,
            None,
            true,
            "S3 access key id.",
        ),
        param(
            "S3_SECRET_ACCESS_KEY",
            "objects",
            Secret,
            None,
            true,
            "S3 secret access key.",
        ),
        // email
        param(
            "EMAIL_TRANSPORT",
            "email",
            String,
            Some("none"),
            false,
            "Outbound transport: none, smtp, postmark or ses.",
        ),
        param(
            "EMAIL_FROM_ADDRESS",
            "email",
            String,
            Some("support@marula.local"),
            false,
            "From address on Marula-generated mail.",
        ),
        param(
            "EMAIL_FROM_NAME",
            "email",
            String,
            None,
            false,
            "From display name.",
        ),
        param(
            "POSTMARK_SERVER_TOKEN",
            "email",
            Secret,
            None,
            true,
            "Postmark server token.",
        ),
        param("SMTP_HOST", "email", String, None, false, "SMTP host."),
        param(
            "SMTP_PORT",
            "email",
            Integer,
            Some("25"),
            false,
            "SMTP port.",
        ),
        param(
            "SMTP_USERNAME",
            "email",
            String,
            None,
            false,
            "SMTP username.",
        ),
        param(
            "SMTP_PASSWORD",
            "email",
            Secret,
            None,
            true,
            "SMTP password.",
        ),
        param(
            "AWS_SES_ACCESS_KEY_ID",
            "email",
            Secret,
            None,
            true,
            "SES access key id.",
        ),
        param(
            "AWS_SES_SECRET_ACCESS_KEY",
            "email",
            Secret,
            None,
            true,
            "SES secret access key.",
        ),
        param(
            "AWS_SES_REGION",
            "email",
            String,
            Some("eu-west-1"),
            false,
            "SES region. Part of the endpoint, not a preference.",
        ),
        // auth
        param(
            "WEBAUTHN_RP_ID",
            "auth",
            String,
            Some("localhost"),
            false,
            "WebAuthn relying-party id (effective domain of the origin).",
        ),
        param(
            "WEBAUTHN_RP_ORIGIN",
            "auth",
            Url,
            Some("http://localhost:3000"),
            false,
            "WebAuthn relying-party origin.",
        ),
        param(
            "ACCESS_TTL_SECS",
            "auth",
            Integer,
            Some("900"),
            false,
            "Access-token TTL.",
        ),
        param(
            "REFRESH_TTL_SECS",
            "auth",
            Integer,
            Some("2592000"),
            false,
            "Refresh-token TTL.",
        ),
        // cadence
        param(
            "EMAIL_POLL_SECS",
            "cadence",
            Integer,
            Some("2"),
            false,
            "Outbound email idle poll interval. See docs/worker-cadence.md.",
        ),
        param(
            "QUEUE_IDLE_MS",
            "cadence",
            Integer,
            Some("500"),
            false,
            "General job-queue idle sleep.",
        ),
        param(
            "EVAL_INTERVAL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "SLA, project-billing, rotation and dedupe sweep interval.",
        ),
        param(
            "TIME_AUDIT_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Missing-time nudge sweep interval.",
        ),
        param(
            "DELIVERY_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Scheduled report delivery sweep interval.",
        ),
        param(
            "USAGE_REFRESH_SECS",
            "cadence",
            Integer,
            Some("300"),
            false,
            "Org usage refresh sweep interval.",
        ),
        param(
            "DIGEST_POLL_SECS",
            "cadence",
            Integer,
            Some("900"),
            false,
            "Daily digest sweep interval.",
        ),
        param(
            "RETENTION_SWEEP_SECS",
            "cadence",
            Integer,
            Some("21600"),
            false,
            "Workflow run-retention sweep interval.",
        ),
        // observability
        param(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "observability",
            Url,
            None,
            false,
            "OTLP endpoint. Unset runs the none mode (no export).",
        ),
        param(
            "GLITCHTIP_DSN_CORE_API",
            "observability",
            Secret,
            None,
            false,
            "GlitchTip DSN presence for the self-host pane.",
        ),
    ]
}

/// JSON rendering of the descriptor, optionally narrowed to one profile.
pub fn describe_json(profile: Option<&str>) -> anyhow::Result<String> {
    let mut desc = descriptor();
    if let Some(name) = profile {
        let kept: Vec<Profile> = desc
            .profiles
            .iter()
            .filter(|p| p.name == name)
            .cloned()
            .collect();
        anyhow::ensure!(
            !kept.is_empty(),
            "unknown profile {name:?} — known: {}",
            desc.profiles
                .iter()
                .map(|p| p.name)
                .collect::<Vec<_>>()
                .join(", ")
        );
        desc.profiles = kept;
    }
    Ok(serde_json::to_string_pretty(&desc)?)
}

/// A short human summary for `marula deploy describe` without `--json`.
#[must_use]
pub fn describe_summary() -> String {
    let d = descriptor();
    let mut out = String::new();
    out.push_str(&format!(
        "marula-core {} — deploy descriptor schema {}\n",
        d.core_version, d.schema_version
    ));
    out.push_str(&format!(
        "image {} (roles: {}) via {} [{}]\n\n",
        d.image.name,
        d.image.roles.join(", "),
        d.image.dockerfile,
        d.image.role_arg
    ));
    for p in &d.profiles {
        out.push_str(&format!("profile {} — {}\n", p.name, p.title));
        for s in &p.services {
            out.push_str(&format!(
                "  service {:<10} role {:<12} ports {:?} needs {:?}\n",
                s.name, s.role, s.ports, s.needs
            ));
        }
        if !p.sets.is_empty() {
            out.push_str(&format!("  presets {} parameters\n", p.sets.len()));
        }
    }
    out.push_str(&format!(
        "\n{} runtime parameters; emit JSON with `marula deploy describe --json`\n",
        d.params.len()
    ));
    out
}

/// The repo root, from this crate's manifest dir.
#[cfg(test)]
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Regenerates `deploy/profiles/*.yaml` from this module. Returns the paths
/// written.
pub fn write_profiles(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    for profile in profiles() {
        let path = dir.join(format!("{}.yaml", profile.name));
        std::fs::write(&path, profile_yaml(&profile))?;
        written.push(path);
    }
    Ok(written)
}

/// The checked-in YAML for one profile, as `profile_yaml` renders it. Used by
/// the drift test.
#[cfg(test)]
#[must_use]
pub fn checked_in_profile_yaml(name: &str) -> Option<String> {
    let path = repo_root()
        .join("deploy/profiles")
        .join(format!("{name}.yaml"));
    std::fs::read_to_string(path).ok()
}

/// Deterministic YAML for one profile. The shape is fixed and simple, so a
/// small emitter beats a YAML dependency here.
#[must_use]
pub fn profile_yaml(p: &Profile) -> String {
    fn q(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
    let mut s = String::new();
    s.push_str("# Generated by `marula deploy profiles --write` (Wave 49 lane R7).\n");
    s.push_str("# The machine-readable contract is `marula deploy describe --json`;\n");
    s.push_str("# see docs/deploy-descriptor.md. Do not edit by hand.\n");
    s.push_str(&format!("name: {}\n", p.name));
    s.push_str(&format!("title: {}\n", q(p.title)));
    s.push_str(&format!("description: {}\n", q(p.description)));
    s.push_str("services:\n");
    for svc in &p.services {
        s.push_str(&format!("  - name: {}\n", svc.name));
        s.push_str(&format!("    role: {}\n", svc.role));
        let ports: Vec<String> = svc.ports.iter().map(u16::to_string).collect();
        s.push_str(&format!("    ports: [{}]\n", ports.join(", ")));
        s.push_str(&format!("    needs: [{}]\n", svc.needs.join(", ")));
    }
    s.push_str("sets:\n");
    for (k, v) in &p.sets {
        s.push_str(&format!("  {k}: {}\n", q(v)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_names_are_unique_and_groups_are_known() {
        let names: Vec<&str> = params().iter().map(|p| p.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate parameter name");

        let groups: std::collections::HashSet<&str> = params().iter().map(|p| p.group).collect();
        for known in [
            "topology",
            "database",
            "cache",
            "secrets",
            "objects",
            "email",
            "auth",
            "cadence",
            "observability",
        ] {
            assert!(groups.contains(known), "group {known} missing");
        }
    }

    #[test]
    fn only_the_boot_critical_secrets_are_required() {
        // `required` means the process refuses to boot without it. Most secrets
        // are conditional on the feature they configure (an S3 key, a Postmark
        // token), so they are not required here. The two boot-critical secrets
        // are, and neither may carry a default.
        let required: Vec<&str> = params()
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        assert_eq!(required, vec!["JWT_SECRET", "MARULA_SECRET_KEY"]);
        for p in params() {
            if p.required {
                assert!(
                    p.default.is_none(),
                    "{} is required but has a default",
                    p.name
                );
            }
        }
    }

    #[test]
    fn every_profile_preset_names_a_real_param() {
        let known: std::collections::HashSet<&str> = params().iter().map(|p| p.name).collect();
        for profile in profiles() {
            for name in profile.sets.keys() {
                assert!(
                    known.contains(name),
                    "profile {} presets unknown param {name}",
                    profile.name
                );
            }
        }
    }

    #[test]
    fn all_in_one_is_the_profile_the_roster_describes() {
        let p = profiles()
            .into_iter()
            .find(|p| p.name == "all-in-one")
            .expect("all-in-one profile");
        assert_eq!(p.sets.get("MARULA_ALL_IN_ONE"), Some(&"1"));
        assert!(
            p.sets.contains_key("MARULA_WORKER_DATABASE_URL"),
            "all-in-one must preset the owner-role URL"
        );
        // One process, so one service, exposing the API and worker-health ports.
        assert_eq!(p.services.len(), 1);
        assert_eq!(p.services[0].ports, vec![8080, 8082]);
    }

    #[test]
    fn split_profile_has_one_service_per_role() {
        let p = profiles()
            .into_iter()
            .find(|p| p.name == "split")
            .expect("split profile");
        let roles: Vec<&str> = p.services.iter().map(|s| s.role).collect();
        assert_eq!(roles, vec!["core-api", "worker"]);
        assert!(p.sets.is_empty(), "split ships the defaults");
    }

    #[test]
    fn describe_json_is_valid_and_pins_the_schema() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["profiles"].as_array().map(Vec::len), Some(2));
        assert!(!v["core_version"].as_str().unwrap_or("").is_empty());
    }

    #[test]
    fn describe_json_narrows_to_a_named_profile() {
        let json = describe_json(Some("all-in-one")).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["profiles"].as_array().map(Vec::len), Some(1));
        assert_eq!(v["profiles"][0]["name"], "all-in-one");

        assert!(describe_json(Some("nope")).is_err());
    }

    #[test]
    fn checked_in_profile_files_match_this_code() {
        // The contract lives in Rust. The checked-in YAML is a copy, so a drift
        // here means someone edited one side only.
        for profile in profiles() {
            let expected = profile_yaml(&profile);
            let actual = checked_in_profile_yaml(profile.name).unwrap_or_else(|| {
                panic!(
                    "deploy/profiles/{}.yaml is missing — run `marula deploy profiles --write`",
                    profile.name
                )
            });
            assert_eq!(
                actual, expected,
                "deploy/profiles/{}.yaml drifted from the descriptor — run `marula deploy profiles --write`",
                profile.name
            );
        }
    }
}
