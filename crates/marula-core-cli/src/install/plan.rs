//! The plan: data, not prose (Wave 60 lane W60-2).
//!
//! A plan is a list of file writes and command steps. Each one carries the
//! reason it exists and the descriptor field that justifies it, so `--dry-run`
//! and `--apply` read the same object (ADR 0026). The recommendation is
//! separate because it answers a different question: not "what will happen" but
//! "which shape fits this machine, and what would it take to fit the next one".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::deploy::{DeployDescriptor, Param, Profile};

use super::host::HostFacts;

/// Which shape to install and what the operator changed.
#[derive(Debug, Clone)]
pub struct Selection {
    pub profile: String,
    /// Parameter overrides, as `NAME=value`. A name the descriptor does not
    /// publish is refused rather than passed through.
    pub overrides: BTreeMap<String, String>,
    /// Where the deployment's files live.
    pub dir: PathBuf,
    /// Where the application images come from: this checkout, or a registry.
    pub images: ImageSource,
    /// The registry prefix, when `images` is [`ImageSource::Registry`].
    pub image_base: Option<String>,
    /// One digest per application service, as `sha256:<hex>`. A registry
    /// install refuses to plan without one for every service it starts.
    pub image_digests: BTreeMap<String, String>,
    /// What the long-running services do after a crash or a daemon restart.
    pub restart: RestartPolicy,
    /// Whether the plan may ask this host about its docker state.
    ///
    /// Off by default so a plan stays a pure function of the descriptor and the
    /// selection: a test that builds a plan must not depend on which volumes the
    /// machine running it happens to hold. The `install` command turns it on,
    /// because that is where a real volume matters.
    pub probe_volumes: bool,
    /// Whether this plan may run on a host that misses the profile's floors.
    ///
    /// Off by default, because the floors are the installer's answer to "will
    /// this box still be serving a team next month", and a shape that starts on
    /// a box too small for it fails later, somewhere the operator cannot connect
    /// back to this decision. A developer install turns it on to plan a shape
    /// this machine cannot carry; the plan says what it is short of rather than
    /// pretending the floors were met.
    pub allow_unfit: bool,
}

/// Where the application images come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageSource {
    /// Build from this checkout. The shape a developer install uses, and the
    /// only shape that works before a registry has published anything.
    #[default]
    Build,
    /// Pull the release's images by digest. An install that needs no source and
    /// no Rust toolchain on the target, which is what a release bundle is for.
    Registry,
}

impl ImageSource {
    /// Parses the value an operator types.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "build" => Some(Self::Build),
            "registry" => Some(Self::Registry),
            _ => None,
        }
    }
}

/// What Docker does with a service that exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Restarts after a crash and after a daemon restart, but a deliberate
    /// `docker compose stop` stays stopped.
    #[default]
    UnlessStopped,
    /// Restarts even after a deliberate stop.
    Always,
    /// Never restarts: a developer install that wants the failure in front of
    /// it rather than hidden behind a loop.
    No,
}

impl RestartPolicy {
    /// The value Docker understands.
    #[must_use]
    pub fn as_compose(self) -> &'static str {
        match self {
            RestartPolicy::UnlessStopped => "unless-stopped",
            RestartPolicy::Always => "always",
            RestartPolicy::No => "no",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "unless-stopped" => Some(Self::UnlessStopped),
            "always" => Some(Self::Always),
            "no" => Some(Self::No),
            _ => None,
        }
    }

    /// The sentence a wizard prints under the choice. The last clause is the
    /// part operators are surprised by, so it is not left implicit.
    #[must_use]
    pub fn consequence(self) -> &'static str {
        match self {
            RestartPolicy::UnlessStopped => {
                "Comes back after a crash and after the Docker daemon restarts. A deliberate stop stays stopped. It only starts at boot if Docker itself does."
            }
            RestartPolicy::Always => {
                "Comes back even after a deliberate stop, which surprises people who ran `compose stop`. It only starts at boot if Docker itself does."
            }
            RestartPolicy::No => {
                "Stays down when it exits. Right for a box you are working on, wrong for one that serves a team."
            }
        }
    }
}

impl Selection {
    #[must_use]
    pub fn new(profile: impl Into<String>, dir: impl Into<PathBuf>) -> Self {
        Self {
            profile: profile.into(),
            overrides: BTreeMap::new(),
            images: ImageSource::Build,
            image_base: None,
            image_digests: BTreeMap::new(),
            restart: RestartPolicy::UnlessStopped,
            probe_volumes: false,
            allow_unfit: false,
            dir: dir.into(),
        }
    }

    /// Adds one `NAME=value` override. The shape is parsed here so an operator's
    /// typo is an argument error, not a silently ignored setting.
    pub fn set(&mut self, assignment: &str) -> Result<(), PlanError> {
        let (name, value) = assignment
            .split_once('=')
            .ok_or_else(|| PlanError::BadOverride(assignment.to_string()))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(PlanError::BadOverride(assignment.to_string()));
        }
        self.overrides
            .insert(name.to_string(), value.trim().to_string());
        Ok(())
    }
}

/// Why a selection cannot become a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    UnknownProfile {
        asked: String,
        known: Vec<String>,
    },
    UnknownLever {
        name: String,
    },
    BadOverride(String),
    /// The host does not meet the profile's floors.
    FloorsNotMet {
        profile: String,
        missing: Vec<String>,
    },
    /// A registry install with no digest for a service it would start. A tag is
    /// mutable; a digest is what the release signed.
    ImageDigestMissing {
        service: String,
    },
    /// A registry install with no registry to pull from.
    ImageBaseMissing,
    /// The detected target cannot do what the selection asks for.
    TargetUnavailable {
        target: &'static str,
        reason: String,
    },
    /// A volume from an earlier install already holds the owner role's password,
    /// and this plan generated a new one.
    ///
    /// Postgres sets `POSTGRES_PASSWORD` when it initialises a volume and never
    /// again, so the stack starts, the API's health check passes, and the migrate
    /// one-shot dies on authentication. Failing here says which volume and which
    /// way out; failing there said "pool timed out" for a day (W60-6).
    VolumeHoldsAnotherPassword {
        volume: String,
    },
    /// The descriptor itself is unusable, which should be impossible in a
    /// released binary but is checked rather than assumed.
    Descriptor(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownProfile { asked, known } => {
                write!(f, "unknown profile {asked:?} — known: {}", known.join(", "))
            }
            Self::UnknownLever { name } => {
                write!(f, "{name} is not a lever this build publishes")
            }
            Self::BadOverride(raw) => write!(f, "expected NAME=value, got {raw:?}"),
            Self::FloorsNotMet { profile, missing } => write!(
                f,
                "profile {profile} needs more machine than this host has: {}",
                missing.join("; ")
            ),
            Self::ImageDigestMissing { service } => write!(
                f,
                "a registry install needs a digest for {service}: a tag can be moved, and the \
                 release signs a digest. The release's release.json carries one per service"
            ),
            Self::ImageBaseMissing => {
                write!(
                    f,
                    "a registry install needs the registry prefix the release published to, \
                     as in ghcr.io/marulagh/marula. Type it under Application images in \
                     settings, or pass --image-base on a terminal. A release bundle's \
                     release.json already names it"
                )
            }
            Self::TargetUnavailable { target, reason } => {
                write!(f, "the {target} target cannot run here: {reason}")
            }
            Self::VolumeHoldsAnotherPassword { volume } => write!(
                f,
                "the docker volume {volume} already holds an owner password from an \
                 earlier install, and this plan would generate a new one. Postgres sets \
                 that password once, when it initialises a volume, so the stack would \
                 come up with the API healthy and the migrate step failing on \
                 authentication. Either remove the volume if its data is disposable \
                 (`docker volume rm {volume}`), or pass the password it holds with \
                 `--set POSTGRES_PASSWORD=<that value>`"
            ),
            Self::Descriptor(reason) => write!(f, "the deploy descriptor is unusable: {reason}"),
        }
    }
}

impl std::error::Error for PlanError {}

/// What the installer recommends, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    /// The profile this host can run. `None` means no shipped profile fits.
    pub profile: Option<String>,
    /// Why, one line per fact that decided it.
    pub why: Vec<String>,
    /// What the next profile up would need, in numbers. `None` when this host
    /// already fits the largest shipped shape.
    pub shortfall: Option<Shortfall>,
    /// Whether the embedding model the chosen profile presets fits this host.
    /// When it does not, the plan says so and names the cloud path (R14).
    pub embedding_fits: bool,
}

/// What one profile would need beyond this host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortfall {
    pub profile: String,
    pub needs: Vec<String>,
}

/// Scores a host against every profile the descriptor publishes.
///
/// The rule (ADR 0026 §3): a profile fits when cores, memory and free disk each
/// meet its floors. Among the fitting profiles, `split` wins because it is the
/// shipped topology; otherwise the one with the fewest services wins, which is
/// `all-in-one`. Below every floor the answer is `None`, and the caller offers
/// the cloud embedding path rather than a shape the box cannot carry.
#[must_use]
pub fn recommend(desc: &DeployDescriptor, facts: &HostFacts) -> Recommendation {
    let mut why = Vec::new();
    let mut fitting: Vec<&Profile> = Vec::new();
    let mut shortfalls: Vec<Shortfall> = Vec::new();
    for profile in &desc.profiles {
        let needs = missing_for(profile, facts);
        if needs.is_empty() {
            why.push(format!(
                "{} fits: {} cores, {} MB, {} GB free",
                profile.name, profile.floors.cores, profile.floors.ram_mb, profile.floors.disk_gb
            ));
            fitting.push(profile);
        } else {
            shortfalls.push(Shortfall {
                profile: profile.name.to_string(),
                needs,
            });
        }
    }
    let chosen = fitting
        .iter()
        .find(|profile| profile.name == "split")
        .or_else(|| fitting.iter().min_by_key(|profile| profile.services.len()))
        .copied();
    // The shortfall is the next shape up when this host fits one, and the
    // closest shape when it fits none: "what would it take" is the question an
    // operator with a small box is actually asking.
    let shortfall = match chosen {
        Some(chosen) => shortfalls
            .into_iter()
            .find(|candidate| candidate.profile != chosen.name),
        None => shortfalls
            .into_iter()
            .min_by_key(|candidate| candidate.needs.len()),
    };
    // False when nothing fits: the model question cannot be better than the
    // profile question, so "no" is the answer that does not overpromise.
    let embedding_fits = chosen.is_some_and(|profile| embedding_fits(profile, facts));
    if chosen.is_none() {
        why.push("no shipped profile fits this host".to_string());
    }
    Recommendation {
        profile: chosen.map(|profile| profile.name.to_string()),
        why,
        shortfall,
        embedding_fits,
    }
}

/// What a host lacks for one profile, in the operator's units.
#[must_use]
pub fn missing_for(profile: &Profile, facts: &HostFacts) -> Vec<String> {
    let mut needs = Vec::new();
    if facts.cores < profile.floors.cores {
        needs.push(format!(
            "{} cores (has {})",
            profile.floors.cores, facts.cores
        ));
    }
    match facts.ram_mb {
        Some(have) if have >= profile.floors.ram_mb => {}
        // A host that publishes no memory reading cannot prove it fits. Saying
        // so is the honest answer; treating unknown as enough is how a box runs
        // out of memory during the first re-index.
        Some(have) => needs.push(format!(
            "{} MB memory (has {})",
            profile.floors.ram_mb, have
        )),
        None => needs.push(format!(
            "{} MB memory (this platform publishes no reading)",
            profile.floors.ram_mb
        )),
    }
    match facts.free_disk_gb {
        Some(have) if have >= profile.floors.disk_gb => {}
        Some(have) => needs.push(format!(
            "{} GB free disk (has {})",
            profile.floors.disk_gb, have
        )),
        None => needs.push(format!(
            "{} GB free disk (unreadable)",
            profile.floors.disk_gb
        )),
    }
    needs
}

/// Whether the embedding model a profile presets fits the host's measured
/// capacity. A cloud model carries no local floor and always fits.
#[must_use]
pub fn embedding_fits(profile: &Profile, facts: &HostFacts) -> bool {
    use core_domain::embedding::{Catalogue, EmbeddingTier, MODEL_ENV, PROVIDER_ENV, TIER_ENV};

    let tier = profile
        .sets
        .get(TIER_ENV)
        .and_then(|value| EmbeddingTier::parse(value))
        .unwrap_or(EmbeddingTier::Standard);
    let provider = profile.sets.get(PROVIDER_ENV).copied().unwrap_or("ollama");
    let catalogue = Catalogue::builtin();
    let entry = match profile.sets.get(MODEL_ENV) {
        Some(id) => catalogue.find(id),
        None => catalogue.for_tier(tier, provider),
    };
    let Some(entry) = entry else { return true };
    if !entry.floor.is_local() {
        return true;
    }
    entry.floor.cores <= facts.cores && facts.ram_mb.is_none_or(|have| entry.floor.ram_mb <= have)
}

/// One step of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    /// Why this step exists, in one sentence.
    pub reason: String,
    /// The descriptor field that justifies it (`profiles[split].floors`,
    /// `params.DATABASE_URL`, …), so a reader can check the claim.
    pub source: String,
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    WriteFile { path: PathBuf, contents: String },
    Run { program: String, args: Vec<String> },
}

impl Step {
    /// The step in one line, for `--dry-run`.
    #[must_use]
    pub fn headline(&self) -> String {
        match &self.kind {
            StepKind::WriteFile { path, .. } => format!("write {}", path.display()),
            StepKind::Run { program, args } => format!("run {program} {}", args.join(" ")),
        }
    }
}

/// What a secret reads as in every rendering: the terminal's dry run, the
/// wizard's config panel, a copied plan. One mask, so an operator cannot mistake
/// the UI's mark for a value the CLI would print.
pub const MASK: &str = "****";

/// The secret values an env text contains, longest first.
///
/// Purpose: a credential can appear somewhere other than its own assignment —
/// `DATABASE_URL` carries the application password inside the URL — so masking
/// only the line named `MARULA_APP_PASSWORD` leaves the password on screen. Both
/// renderings (the plan's text and the wizard's table) mask by value.
///
/// Short values are skipped: a password of one or two characters would otherwise
/// replace every matching character in the document.
#[must_use]
pub fn secret_values(secret_names: &[String], contents: &str) -> Vec<String> {
    let mut values: Vec<String> = contents
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(name, value)| {
            !value.trim().is_empty() && secret_names.iter().any(|secret| secret == name.trim())
        })
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| value.len() >= 8)
        .collect();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values
}

/// Replaces every known secret value inside `text` with [`MASK`].
#[must_use]
pub fn mask_secrets(text: &str, secrets: &[String]) -> String {
    let mut masked = text.to_string();
    for secret in secrets {
        masked = masked.replace(secret, MASK);
    }
    masked
}

/// A complete plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub target: &'static str,
    pub profile: String,
    pub facts: HostFacts,
    pub steps: Vec<Step>,
    /// What stays manual, named rather than implied: an install that silently
    /// leaves the backup target unset is the install nobody finishes.
    pub manual: Vec<String>,
    /// Values the plan writes but must not print: a generated key in a
    /// scrollback or a CI log is a leaked key.
    pub secret_names: Vec<String>,
}

impl Plan {
    /// The plan as an operator reads it before applying it.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "install plan — profile {}, target {}\n\n",
            self.profile, self.target
        ));
        out.push_str("this host:\n");
        for line in self.facts.describe() {
            out.push_str(&format!("  {line}\n"));
        }
        out.push_str(&format!(
            "\nthis plan writes {} file(s) and runs {} command(s):\n",
            self.writes().len(),
            self.steps
                .iter()
                .filter(|step| matches!(step.kind, StepKind::Run { .. }))
                .count()
        ));
        out.push_str("\nsteps:\n");
        for (i, step) in self.steps.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", i + 1, step.headline()));
            out.push_str(&format!("     why: {}\n", step.reason));
            out.push_str(&format!("     field: {}\n", step.source));
            if let StepKind::WriteFile { contents, .. } = &step.kind {
                out.push_str("     contents:\n");
                for line in self.redact(contents).lines() {
                    out.push_str(&format!("       {line}\n"));
                }
            }
        }
        if !self.manual.is_empty() {
            out.push_str("\nstays manual (the installer will not invent these):\n");
            for item in &self.manual {
                out.push_str(&format!("  - {item}\n"));
            }
        }
        out
    }

    /// The file contents with every secret masked, for `--dry-run`.
    ///
    /// Masked by VALUE, not only by line: a generated password also appears
    /// inside `DATABASE_URL`, and a dry run that hides the line while printing
    /// the URL has not hidden anything.
    #[must_use]
    pub fn redact(&self, contents: &str) -> String {
        let secrets = secret_values(&self.secret_names, contents);
        contents
            .lines()
            .map(|line| {
                if let Some((name, _)) = line.split_once('=') {
                    if self.secret_names.iter().any(|secret| secret == name.trim()) {
                        return format!("{name}={MASK}");
                    }
                }
                mask_secrets(line, &secrets)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every file the plan writes, in order.
    #[must_use]
    pub fn writes(&self) -> Vec<(&Path, &str)> {
        self.steps
            .iter()
            .filter_map(|step| match &step.kind {
                StepKind::WriteFile { path, contents } => Some((path.as_path(), contents.as_str())),
                StepKind::Run { .. } => None,
            })
            .collect()
    }
}

/// The effective environment a profile produces: catalogue defaults, the
/// profile's presets over them, then the operator's overrides.
#[must_use]
pub fn effective_env(
    desc: &DeployDescriptor,
    profile: &Profile,
    selection: &Selection,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for param in &desc.params {
        if let Some(default) = param.default {
            env.insert(param.name.to_string(), default.to_string());
        }
    }
    for (name, value) in &profile.sets {
        env.insert((*name).to_string(), (*value).to_string());
    }
    for (name, value) in &selection.overrides {
        env.insert(name.clone(), value.clone());
    }
    env
}

/// The env file the compose target writes, with the descriptor's own words
/// above each line it sets.
///
/// Only what the profile presets, what the operator overrides and what the
/// process refuses to boot without are written. Everything else is the
/// catalogue default, and copying 170 defaults into a file would turn a
/// deployment into a transcript of one release.
#[must_use]
pub fn render_env(
    desc: &DeployDescriptor,
    profile: &Profile,
    selection: &Selection,
    resolved: &BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# marula {} — generated by `marula install --apply` for profile {}\n",
        desc.core_version, profile.name
    ));
    out.push_str("# Do not edit by hand without reading docs/deploy-descriptor.md:\n");
    out.push_str("# every unset value here is the descriptor's default.\n\n");

    let mut env = effective_env(desc, profile, selection);
    for (name, value) in resolved {
        env.insert(name.clone(), value.clone());
    }
    // One expansion pass, after every value is known: the compose stack reads
    // these values as data and does not expand them, so a `${NAME}` left in the
    // file would reach Postgres as a literal password.
    expand(&mut env);

    let mut written: Vec<&str> = Vec::new();
    for (name, value) in &profile.sets {
        let Some(param) = find_param(desc, name) else {
            continue;
        };
        out.push_str(&format!("# {}\n", param.consequence));
        let value = env.get(*name).map_or(*value, String::as_str);
        out.push_str(&format!("{name}={value}\n\n"));
        written.push(name);
    }
    for (name, value) in &selection.overrides {
        if written.contains(&name.as_str()) {
            continue;
        }
        if let Some(param) = find_param(desc, name) {
            out.push_str(&format!("# {}\n", param.consequence));
        } else {
            out.push_str("# set by the operator\n");
        }
        out.push_str(&format!("{name}={value}\n\n"));
        written.push(name);
    }
    for (name, reason) in infra_requirements() {
        if written.contains(name) {
            continue;
        }
        let Some(value) = env.get(*name) else {
            continue;
        };
        out.push_str(&format!("# {reason}\n"));
        out.push_str(&format!("{name}={value}\n\n"));
        written.push(name);
    }
    for param in &desc.params {
        if !param.required || written.contains(&param.name) {
            continue;
        }
        let value = env.get(param.name).map_or("", String::as_str);
        out.push_str(&format!("# REQUIRED. {}\n", param.consequence));
        out.push_str(&format!("{}={value}\n\n", param.name));
    }
    out
}

/// Values the compose file requires that the descriptor does not describe,
/// because they are infrastructure rather than application settings.
///
/// The list is the compose file's own contract (`deploy/docker-compose.yml`
/// header: eight required variables, four of them parameters). A name here that
/// the compose file stops requiring is harmless; a name it starts requiring is
/// not, which is why the file's `:?` guards are the authority and this list is
/// their projection.
#[must_use]
pub fn infra_requirements() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "POSTGRES_PASSWORD",
            "the owner role's password, applied by initdb on the first boot of the data volume. Generated once for a fresh volume; an existing volume keeps the password it was initialised with, so edit this value rather than rotating it alone.",
        ),
        (
            "MARULA_APP_PASSWORD",
            "the app role's password, applied to the role on every start. Generated once; rotate it here and the one-shot applies it on the next start.",
        ),
        (
            "GARAGE_RPC_SECRET",
            "the object store's inter-node secret. Generated once; changing it on a running cluster needs the nodes restarted together.",
        ),
        (
            "GLITCHTIP_SECRET_KEY",
            "the error tracker's signing key. Generated once; changing it signs out the tracker's own sessions, not Marula's.",
        ),
    ]
}

/// The compose project the shipped file declares.
///
/// `name:` in `deploy/docker-compose.yml` namespaces every container and volume
/// the stack creates, so the installer has to know it to recognise a volume an
/// earlier install left behind. A test reads that file and fails when the two
/// drift, because a copied constant is exactly the kind of thing that drifts.
pub const COMPOSE_PROJECT: &str = "marula";

/// The volume Postgres keeps its data in, under the project's namespace.
#[must_use]
pub fn postgres_volume() -> String {
    format!("{COMPOSE_PROJECT}_pgdata")
}

/// Whether a plan should refuse because it would generate an owner password the
/// existing volume cannot know.
///
/// Two conditions, and both matter: a volume with no generated password is an
/// ordinary re-apply, which is how an update and a repair work.
#[must_use]
pub fn volume_conflicts(generates_password: bool, volume_present: bool) -> bool {
    generates_password && volume_present
}

/// Whether docker reports this volume.
///
/// A probe that cannot answer — no docker on the path, a daemon that is down —
/// is not evidence of a volume, so it answers `false` and the install proceeds.
/// The alternative is refusing every install on a host whose docker socket is
/// slow to answer, which is a worse failure than the one this check exists for.
#[must_use]
pub fn volume_present(name: &str) -> bool {
    std::process::Command::new("docker")
        .args(["volume", "inspect", name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Every required value, with a generated default for any the caller did not
/// supply.
///
/// A value already in `existing` is kept: re-running `--apply` must not rotate a
/// signing key (which signs everyone out) or a database password (which the
/// running volume no longer matches).
#[must_use]
pub fn resolve_required(
    desc: &DeployDescriptor,
    selection: &Selection,
    existing: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut resolved = BTreeMap::new();
    let names = desc
        .params
        .iter()
        .filter(|param| param.required)
        .map(|param| param.name)
        .chain(infra_requirements().iter().map(|(name, _)| *name));
    for name in names {
        if let Some(from_operator) = selection.overrides.get(name) {
            resolved.insert(name.to_string(), from_operator.clone());
            continue;
        }
        if let Some(kept) = existing.get(name).filter(|value| !value.trim().is_empty()) {
            resolved.insert(name.to_string(), kept.clone());
            continue;
        }
        resolved.insert(name.to_string(), generate_secret());
    }
    resolved
}

/// A 32-byte secret, hex. Hex on purpose: every one of these values is
/// interpolated into a URL or a DSN somewhere, and hex needs no escaping.
#[must_use]
pub fn generate_secret() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

/// Resolves `${NAME}` references against the map, once, so the written file
/// carries values rather than templates.
pub fn expand(env: &mut BTreeMap<String, String>) {
    let snapshot = env.clone();
    for value in env.values_mut() {
        let mut out = String::with_capacity(value.len());
        let mut rest = value.as_str();
        while let Some(start) = rest.find("${") {
            let (before, after) = rest.split_at(start);
            out.push_str(before);
            let Some(end) = after.find('}') else {
                out.push_str(after);
                rest = "";
                break;
            };
            let name = &after[2..end];
            match snapshot.get(name) {
                Some(replacement) => out.push_str(replacement),
                // An unknown reference stays as written: better a visible
                // template than a silently empty value in a URL.
                None => out.push_str(&after[..=end]),
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        *value = out;
    }
}

fn find_param<'a>(desc: &'a DeployDescriptor, name: &str) -> Option<&'a Param> {
    desc.params.iter().find(|param| param.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy;

    fn facts(cores: u16, ram_mb: Option<u32>, disk_gb: Option<u32>) -> HostFacts {
        HostFacts {
            cores,
            ram_mb,
            free_disk_gb: disk_gb,
            docker: Some("27.0.0".to_string()),
            gpu: None,
        }
    }

    fn profile(name: &str) -> Profile {
        deploy::descriptor()
            .profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .expect("a shipped profile")
    }

    #[test]
    fn the_env_file_carries_the_profiles_presets_and_the_required_blanks() {
        let desc = deploy::descriptor();
        let selection = Selection::new("all-in-one", "/tmp");
        let text = render_env(
            &desc,
            &profile("all-in-one"),
            &selection,
            &resolve_required(&desc, &selection, &BTreeMap::new()),
        );

        assert!(text.contains("MARULA_ALL_IN_ONE=1"));
        // Read the sentence from the descriptor instead of repeating it here: a
        // copy edit should not turn into a test failure, and a test that pins
        // prose it also ships cannot tell the two apart.
        let consequence = desc
            .params
            .iter()
            .find(|param| param.name == "MARULA_ALL_IN_ONE")
            .map(|param| param.consequence)
            .expect("the all-in-one lever publishes a consequence");
        assert!(
            text.contains(consequence),
            "the profile's preset carries the descriptor's consequence: {text}"
        );
        for required in [
            "JWT_SECRET=",
            "MARULA_SECRET_KEY=",
            "MARULA_INTERNAL_TOKEN=",
        ] {
            assert!(
                text.contains(required),
                "{required} is missing from the env file"
            );
        }
        assert!(
            text.contains("# REQUIRED."),
            "a required value is marked as such, not silently blank"
        );
    }

    #[test]
    fn an_override_wins_over_the_profile_preset() {
        let desc = deploy::descriptor();
        let mut selection = Selection::new("all-in-one", "/tmp");
        selection.set("QUEUE_IDLE_MS=250").expect("parsed");
        let env = effective_env(&desc, &profile("all-in-one"), &selection);
        assert_eq!(env.get("QUEUE_IDLE_MS").map(String::as_str), Some("250"));
        // A preset the operator did not touch keeps the profile's value.
        assert_eq!(env.get("MARULA_ALL_IN_ONE").map(String::as_str), Some("1"));
        // A lever nobody sets keeps the catalogue default.
        assert_eq!(
            env.get("MARULA_DB_POOL_MAX").map(String::as_str),
            Some("20")
        );
    }

    #[test]
    fn a_host_that_publishes_no_memory_cannot_prove_it_fits() {
        let needs = missing_for(&profile("all-in-one"), &facts(8, None, Some(100)));
        assert!(
            needs.iter().any(|need| need.contains("no reading")),
            "unknown memory is a refusal, not a pass: {needs:?}"
        );
    }

    #[test]
    fn a_profile_below_its_floors_is_never_recommended() {
        let rec = recommend(&deploy::descriptor(), &facts(1, Some(256), Some(2)));
        assert_eq!(rec.profile, None);
        assert!(
            !rec.embedding_fits,
            "a one-core box does not host the local model"
        );
        let shortfall = rec.shortfall.expect("the next shape's shortfall");
        assert!(shortfall.needs.len() >= 2, "{shortfall:?}");
    }

    #[test]
    fn required_values_are_generated_once_and_a_replan_keeps_them() {
        let desc = deploy::descriptor();
        let selection = Selection::new("all-in-one", "/tmp");

        let first = resolve_required(&desc, &selection, &BTreeMap::new());
        assert_eq!(
            first.len(),
            8,
            "four parameters and four infrastructure values"
        );
        for (name, value) in &first {
            assert_eq!(value.len(), 64, "{name} is not 32 bytes of hex");
        }

        // A second plan against the same directory must reuse them: rotating a
        // signing key signs everyone out, and rotating a database password
        // leaves the running volume unable to authenticate.
        let second = resolve_required(&desc, &selection, &first);
        assert_eq!(first, second, "a re-plan must not rotate any value");

        // An override still wins over both.
        let mut overridden = Selection::new("all-in-one", "/tmp");
        overridden
            .set("JWT_SECRET=operator-chosen")
            .expect("parsed");
        let third = resolve_required(&desc, &overridden, &first);
        assert_eq!(
            third.get("JWT_SECRET").map(String::as_str),
            Some("operator-chosen")
        );
    }

    #[test]
    fn a_template_in_a_preset_is_expanded_before_it_reaches_the_file() {
        let desc = deploy::descriptor();
        let selection = Selection::new("split", "/tmp");
        let resolved = resolve_required(&desc, &selection, &BTreeMap::new());
        let text = render_env(&desc, &profile("split"), &selection, &resolved);

        assert!(
            !text.contains("${"),
            "the compose file does not expand env-file values, so no template may survive: {text}"
        );
        let app_password = resolved.get("MARULA_APP_PASSWORD").expect("generated");
        assert!(
            text.contains(&format!(
                "postgres://marula_app:{app_password}@pgbouncer:6432/marula"
            )),
            "the profile's URL carries the generated password"
        );
        for (name, reason) in infra_requirements() {
            assert!(text.contains(name), "{name} is missing");
            assert!(text.contains(reason), "the reason for {name} is missing");
        }
    }

    #[test]
    fn a_dry_run_masks_every_secret_it_would_write() {
        let desc = deploy::descriptor();
        let selection = Selection::new("all-in-one", "/tmp");
        let resolved = resolve_required(&desc, &selection, &BTreeMap::new());
        let plan = Plan {
            target: "compose",
            profile: "all-in-one".to_string(),
            facts: facts(8, Some(16384), Some(100)),
            steps: vec![Step {
                kind: StepKind::WriteFile {
                    path: "/tmp/marula.env".into(),
                    contents: render_env(&desc, &profile("all-in-one"), &selection, &resolved),
                },
                reason: "the environment".to_string(),
                source: "profiles.all-in-one.sets".to_string(),
            }],
            manual: Vec::new(),
            secret_names: resolved.keys().cloned().collect(),
        };
        let rendered = plan.render();
        for value in resolved.values() {
            assert!(
                !rendered.contains(value),
                "a dry run printed a generated secret"
            );
        }
        assert!(rendered.contains("JWT_SECRET=****"), "{rendered}");
        assert!(rendered.contains("POSTGRES_PASSWORD=****"), "{rendered}");
    }

    #[test]
    fn a_small_box_is_not_offered_a_profile_its_stack_cannot_carry() {
        // The floors describe the stack, not the model: a 2-core box is below
        // both shipped shapes even with cloud embeddings, and the honest answer
        // is to say so (the shortfall names what it needs). The cloud path
        // belongs to the model, and the caller names it separately.
        let rec = recommend(&deploy::descriptor(), &facts(2, Some(4096), Some(30)));
        assert_eq!(rec.profile, None);
        assert!(rec.shortfall.is_some());
    }

    /// The volume is namespaced by the compose project, so the constant and the
    /// shipped compose file have to agree. A copy that drifts would make the
    /// check look at a volume no stack uses, and pass while the install fails.
    #[test]
    fn the_project_name_matches_the_shipped_compose_file() {
        let compose = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/docker-compose.yml"
        ))
        .expect("the base compose file is part of the release");
        let declared = compose
            .lines()
            .find_map(|line| line.strip_prefix("name:"))
            .map(str::trim)
            .expect("the base compose file declares a project name");
        assert_eq!(declared, COMPOSE_PROJECT);
        assert_eq!(postgres_volume(), "marula_pgdata");
    }

    /// Only a plan that would *write* a new password conflicts. Re-applying over
    /// an existing env, or an update, keeps the value the volume already has.
    #[test]
    fn only_a_generated_password_conflicts_with_an_existing_volume() {
        assert!(volume_conflicts(true, true));
        assert!(!volume_conflicts(true, false));
        assert!(!volume_conflicts(false, true));
        assert!(!volume_conflicts(false, false));
    }
}
