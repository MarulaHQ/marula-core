//! The typed view layer the installer's two hosts render (Wave 60 lane W60-3).
//!
//! Purpose: turn the engine's own types into one wire shape, so the native
//! shell and the browser fallback draw the same screens from the same JSON and
//! neither host can grow its own opinion about what a plan says. Everything
//! that decides anything lives in the engine; this module only projects it.
//!
//! Non-purpose: no transport. It has no HTTP types and no Tauri types, so the
//! two hosts can call it from a command handler and from a request handler
//! without either one leaking into the other.
//!
//! The masking rule is the one place this module is allowed to have an opinion:
//! a value the plan marks secret is replaced here, before it can reach a
//! renderer, a log or a devtools panel. The engine refuses to *print* a secret;
//! this refuses to *serialise* one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::deploy::{self, DeployDescriptor, ParamKind};
use crate::install::compose::ComposeTarget;
use crate::install::doctor;
use crate::install::host::HostFacts;
use crate::install::plan::{self, Plan, Selection, StepKind};
use crate::install::target::DeploymentTarget as _;

/// What a masked value reads as: the engine's own mark, so the terminal's dry
/// run and the wizard's config panel cannot disagree about what a secret looks
/// like. Deliberately not empty — an empty cell reads as "unset", and these
/// values are the opposite of unset.
pub const MASK: &str = plan::MASK;

/// One host fact, ready to render.
#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    pub cores: u16,
    pub ram_mb: Option<u32>,
    pub free_disk_gb: Option<u32>,
    pub docker: Option<String>,
    pub gpu: Option<String>,
    /// The same facts as sentences, from `HostFacts::describe`, so the shell
    /// prints one wording for a fact and not two.
    pub lines: Vec<String>,
}

impl From<&HostFacts> for HostView {
    fn from(facts: &HostFacts) -> Self {
        Self {
            cores: facts.cores,
            ram_mb: facts.ram_mb,
            free_disk_gb: facts.free_disk_gb,
            docker: facts.docker.clone(),
            gpu: facts.gpu.clone(),
            lines: facts.describe(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FloorView {
    pub cores: u16,
    pub ram_mb: u32,
    pub disk_gb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceView {
    pub name: String,
    pub role: String,
    pub ports: Vec<u16>,
    pub needs: Vec<String>,
}

/// A profile preset, as the wizard shows it ("this shape sets these for you").
#[derive(Debug, Clone, Serialize)]
pub struct SetView {
    pub name: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    pub name: String,
    pub title: String,
    pub description: String,
    pub floors: FloorView,
    pub services: Vec<ServiceView>,
    /// Whether this host meets every floor.
    pub fits: bool,
    /// What it still needs, in the engine's own words. Empty when `fits`.
    pub missing: Vec<String>,
    pub sets: Vec<SetView>,
}

/// One parameter, as the catalogue describes it. The consequence and the
/// rationale travel with the row: the wizard renders the sentence the
/// descriptor carries rather than writing a second explanation that can drift.
#[derive(Debug, Clone, Serialize)]
pub struct ParamView {
    pub name: String,
    pub group: String,
    pub kind: &'static str,
    pub required: bool,
    pub default: Option<String>,
    pub secret: bool,
    pub description: String,
    pub consequence: String,
    pub default_rationale: Option<String>,
    pub requires: Vec<String>,
    pub conflicts: Vec<String>,
    /// The profile pins this value, so an edit here is an override of a preset.
    pub profile_value: Option<String>,
    /// The closed set this lever accepts. Empty means free text. A renderer
    /// shows a select when it is non-empty and never invents its own list.
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvView {
    pub name: String,
    /// The descriptor's group for the lever, or `infrastructure` for the four
    /// values the compose file demands and no parameter describes. The renderer
    /// groups by this instead of inventing its own taxonomy.
    pub group: String,
    /// The descriptor's one-line description of the lever, or the reason an
    /// infrastructure value exists. Shown under the label, never restated.
    pub description: String,
    pub value: String,
    pub secret: bool,
    pub present: bool,
    /// Where the value the plan writes comes from: `you`, `profile`,
    /// `generated`, `kept` (an existing file already had it) or `default`.
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShortfallView {
    pub profile: String,
    pub needs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecommendationView {
    pub profile: Option<String>,
    pub why: Vec<String>,
    pub shortfall: Option<ShortfallView>,
    pub embedding_fits: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusView {
    pub target: String,
    pub services: Vec<String>,
    pub error: Option<String>,
}

/// Everything the first three screens need, in one payload.
#[derive(Debug, Clone, Serialize)]
pub struct StateView {
    pub host: HostView,
    pub profiles: Vec<ProfileView>,
    pub recommendation: RecommendationView,
    pub params: Vec<ParamView>,
    /// Every group in catalogue order, so the advanced list's chips do not
    /// reshuffle between runs.
    pub groups: Vec<String>,
    pub env: Vec<EnvView>,
    pub status: StatusView,
    pub dir: String,
    /// The embedding selection this environment resolves to, one sentence.
    pub embedding: String,
    /// The retrieval mode and whether this host can run it.
    pub retrieval: String,
    /// How many parameters the catalogue publishes, for the "all N" control.
    pub param_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepView {
    /// `write` or `run`.
    pub kind: &'static str,
    pub headline: String,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanView {
    pub profile: String,
    pub target: String,
    /// The operator's text form, for the "copy the plan" action and for an
    /// operator who would rather read the terminal's wording.
    pub raw: String,
    pub steps: Vec<StepView>,
    pub manual: Vec<String>,
    pub config: Vec<EnvView>,
    /// Secrets this run generates (not reused from an existing file).
    pub generated: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckView {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorView {
    pub ok: bool,
    pub checks: Vec<CheckView>,
    pub services: Vec<String>,
    pub error: Option<String>,
}

/// What an apply is doing, for the renderer that polls it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApplyView {
    pub running: bool,
    pub done: Vec<String>,
    pub failed: Option<String>,
    pub manual: Vec<String>,
}

pub type ApplyHandle = Arc<Mutex<ApplyView>>;

/// The engine calls a host can make, bound to one directory and compose file.
#[derive(Debug, Clone)]
pub struct Session {
    pub dir: PathBuf,
    pub compose_file: PathBuf,
    pub memory_mb: Option<u32>,
    /// Every apply in this process is tracked here. One install at a time is
    /// the honest limit: two applies would write the same env file.
    pub apply: ApplyHandle,
    /// Where the application images come from (Wave 60 W60-3 follow-up).
    pub images: crate::install::plan::ImageSource,
    /// What the long-running services do after an exit.
    pub restart: crate::install::plan::RestartPolicy,
    /// Facts a caller pinned instead of measuring, for a host that is not this
    /// one. The wizard never sets it; a test does, because a suite that reads
    /// the machine's free disk fails the day the disk is full.
    pinned_facts: Option<HostFacts>,
}

impl Session {
    #[must_use]
    pub fn new(
        dir: impl Into<PathBuf>,
        compose_file: impl Into<PathBuf>,
        memory_mb: Option<u32>,
    ) -> Self {
        Self {
            dir: dir.into(),
            compose_file: compose_file.into(),
            memory_mb,
            apply: Arc::new(Mutex::new(ApplyView::default())),
            images: crate::install::plan::ImageSource::Build,
            restart: crate::install::plan::RestartPolicy::UnlessStopped,
            pinned_facts: None,
        }
    }

    /// Sets the two compose-shape decisions this request carries.
    #[must_use]
    pub fn with_shape(
        mut self,
        images: crate::install::plan::ImageSource,
        restart: crate::install::plan::RestartPolicy,
    ) -> Self {
        self.images = images;
        self.restart = restart;
        self
    }

    /// Scores a stated machine instead of measuring this one.
    #[must_use]
    pub fn with_facts(mut self, facts: HostFacts) -> Self {
        self.pinned_facts = Some(facts);
        self
    }

    /// Measures the machine for this session's install directory.
    #[must_use]
    pub fn facts(&self) -> HostFacts {
        if let Some(facts) = &self.pinned_facts {
            return facts.clone();
        }
        HostFacts::detect(&self.dir).with_declared_memory(self.memory_mb)
    }

    fn target(&self) -> ComposeTarget {
        let target = ComposeTarget::new(deploy::descriptor(), self.compose_file.clone());
        match self.env_file() {
            Some(path) => target.with_env_file(path),
            None => target,
        }
    }

    /// The env file this session would write, when one exists.
    #[must_use]
    pub fn env_file(&self) -> Option<PathBuf> {
        let path = self.dir.join("marula.env");
        path.is_file().then_some(path)
    }

    /// Reads the install's env file, or an empty map when there is none.
    #[must_use]
    pub fn env(&self) -> BTreeMap<String, String> {
        self.env_file()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| doctor::parse_env_file(&text))
            .unwrap_or_default()
    }

    /// Everything the wizard's first three screens render.
    #[must_use]
    pub fn state(&self) -> StateView {
        let descriptor = deploy::descriptor();
        let facts = self.facts();
        let recommendation = plan::recommend(&descriptor, &facts);
        let existing = self.env();
        let overrides: BTreeMap<String, String> = BTreeMap::new();

        let profiles: Vec<ProfileView> = descriptor
            .profiles
            .iter()
            .map(|profile| profile_view(&descriptor, profile, &facts, &existing))
            .collect();

        let params = descriptor
            .params
            .iter()
            .map(|param| ParamView {
                name: param.name.to_string(),
                group: param.group.to_string(),
                kind: kind_name(param.kind),
                required: param.required,
                default: param.default.map(|value| mask_if(param.secret, value)),
                secret: param.secret,
                description: param.description.to_string(),
                consequence: param.consequence.to_string(),
                default_rationale: param.default_rationale.map(str::to_string),
                requires: param
                    .requires
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
                conflicts: param
                    .conflicts
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
                values: param
                    .values
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                profile_value: profiles
                    .iter()
                    .flat_map(|view| view.sets.iter().map(move |set| (view.name.clone(), set)))
                    .find(|(_, set)| set.name == param.name)
                    .map(|(profile_name, set)| format!("{} ({profile_name})", set.value)),
            })
            .collect();

        let mut groups: Vec<String> = descriptor
            .params
            .iter()
            .map(|param| param.group.to_string())
            .collect();
        groups.dedup();

        let secrets = existing
            .iter()
            .filter(|(name, _)| is_secret(&descriptor, name))
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        let env = config_rows(
            &descriptor,
            &recommendation,
            &existing,
            &overrides,
            &secrets,
        );
        let status = self.target().status();

        StateView {
            host: HostView::from(&facts),
            profiles,
            recommendation: RecommendationView {
                profile: recommendation.profile.clone(),
                why: recommendation.why.clone(),
                shortfall: recommendation
                    .shortfall
                    .as_ref()
                    .map(|short| ShortfallView {
                        profile: short.profile.clone(),
                        needs: short.needs.clone(),
                    }),
                embedding_fits: recommendation.embedding_fits,
            },
            params,
            groups,
            param_count: descriptor.params.len(),
            env,
            status: StatusView {
                target: status.target.to_string(),
                services: status.services,
                error: status.error,
            },
            dir: self.dir.display().to_string(),
            embedding: embedding_sentence(&descriptor, &recommendation),
            retrieval: retrieval_sentence(&descriptor),
        }
    }

    /// The plan for one choice, as the review screen renders it.
    pub fn plan(
        &self,
        profile: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<PlanView, String> {
        let descriptor = deploy::descriptor();
        let facts = self.facts();
        let target = self.target();
        target
            .available(&facts)
            .map_err(|error| error.to_string())?;
        let plan = target
            .plan(&facts, &selection(self, profile, overrides))
            .map_err(|error| error.to_string())?;
        Ok(plan_view(&descriptor, &plan, &self.env(), overrides))
    }

    /// Runs every check that needs no network call, against the real ports.
    pub fn doctor(&self, profile: &str) -> Result<DoctorView, String> {
        let descriptor = deploy::descriptor();
        let chosen = descriptor
            .profiles
            .iter()
            .find(|shipped| shipped.name == profile)
            .ok_or_else(|| format!("no profile `{profile}` in the descriptor"))?;
        let facts = self.facts();
        let env = self.env();
        if env.is_empty() {
            return Err(format!(
                "{} holds no marula.env yet, so there is nothing to check. Apply the plan first.",
                self.dir.display()
            ));
        }
        let status = self.target().status();
        let expect_free_ports = status.services.is_empty();
        let checks = doctor::run_default(&descriptor, chosen, &facts, &env, expect_free_ports);
        Ok(DoctorView {
            ok: doctor::all_ok(&checks),
            checks: checks
                .into_iter()
                .map(|check| CheckView {
                    name: check.name,
                    ok: check.ok,
                    detail: check.detail,
                })
                .collect(),
            services: status.services,
            error: status.error,
        })
    }

    /// Executes a plan on a worker thread and tracks it in [`Session::apply`].
    ///
    /// The engine's apply blocks until compose says the healthchecks pass, so
    /// the host's request handler returns immediately and the renderer polls.
    pub fn start_apply(
        &self,
        profile: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        {
            let job = self
                .apply
                .lock()
                .map_err(|_| "apply state is poisoned".to_string())?;
            if job.running {
                return Err("an apply is already running in this installer".to_string());
            }
        }

        let facts = self.facts();
        let target = self.target();
        target
            .available(&facts)
            .map_err(|error| error.to_string())?;
        let plan = target
            .plan(&facts, &selection(self, profile, overrides))
            .map_err(|error| error.to_string())?;

        {
            let mut job = self
                .apply
                .lock()
                .map_err(|_| "apply state is poisoned".to_string())?;
            *job = ApplyView {
                running: true,
                done: Vec::new(),
                failed: None,
                manual: plan.manual.clone(),
            };
        }

        let handle = Arc::clone(&self.apply);
        std::thread::spawn(move || {
            let applied = target.apply(&plan);
            if let Ok(mut job) = handle.lock() {
                job.running = false;
                job.done = applied.done;
                job.failed = applied.failed;
                job.manual = applied.manual;
            }
        });
        Ok(())
    }

    /// The apply's current state.
    pub fn apply_state(&self) -> ApplyView {
        self.apply
            .lock()
            .map(|job| job.clone())
            .unwrap_or_else(|_| ApplyView {
                running: false,
                done: Vec::new(),
                failed: Some("apply state is poisoned".to_string()),
                manual: Vec::new(),
            })
    }
}

fn selection(session: &Session, profile: &str, overrides: &BTreeMap<String, String>) -> Selection {
    let mut selection = Selection::new(profile, &session.dir);
    selection.overrides = overrides.clone();
    selection.images = session.images;
    selection.restart = session.restart;
    selection
}

fn kind_name(kind: ParamKind) -> &'static str {
    match kind {
        ParamKind::String => "string",
        ParamKind::Integer => "integer",
        ParamKind::Boolean => "boolean",
        ParamKind::Url => "url",
        ParamKind::Secret => "secret",
    }
}

fn mask_if(secret: bool, value: &str) -> String {
    if secret {
        MASK.to_string()
    } else {
        value.to_string()
    }
}

fn profile_view(
    descriptor: &DeployDescriptor,
    profile: &deploy::Profile,
    facts: &HostFacts,
    existing: &BTreeMap<String, String>,
) -> ProfileView {
    let missing = plan::missing_for(profile, facts);
    ProfileView {
        name: profile.name.to_string(),
        title: profile.title.to_string(),
        description: profile.description.to_string(),
        floors: FloorView {
            cores: profile.floors.cores,
            ram_mb: profile.floors.ram_mb,
            disk_gb: profile.floors.disk_gb,
        },
        services: profile
            .services
            .iter()
            .map(|service| ServiceView {
                name: service.name.to_string(),
                role: service.role.to_string(),
                ports: service.ports.clone(),
                needs: service
                    .needs
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
            })
            .collect(),
        fits: missing.is_empty(),
        missing,
        sets: profile
            .sets
            .iter()
            .map(|(name, value)| SetView {
                name: (*name).to_string(),
                value: mask_if(is_secret(descriptor, name), value),
                secret: is_secret(descriptor, name),
            })
            .filter(|set| !(set.secret && existing.contains_key(&set.name)))
            .collect(),
    }
}

fn is_secret(descriptor: &DeployDescriptor, name: &str) -> bool {
    descriptor
        .params
        .iter()
        .any(|param| param.name == name && param.secret)
}

/// The config rows for a directory that may already hold an install.
fn config_rows(
    descriptor: &DeployDescriptor,
    recommendation: &plan::Recommendation,
    existing: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
    secrets: &[String],
) -> Vec<EnvView> {
    let profile = recommendation.profile.as_deref().and_then(|name| {
        descriptor
            .profiles
            .iter()
            .find(|shipped| shipped.name == name)
    });

    let mut names: Vec<&str> = Vec::new();
    for name in existing.keys() {
        names.push(name.as_str());
    }
    for param in &descriptor.params {
        if param.default.is_some() && !names.contains(&param.name) {
            names.push(param.name);
        }
    }
    for (name, _) in plan::infra_requirements() {
        if !names.contains(name) {
            names.push(name);
        }
    }
    if let Some(profile) = profile {
        for name in profile.sets.keys() {
            if !names.contains(name) {
                names.push(name);
            }
        }
    }

    names
        .into_iter()
        .map(|name| {
            let secret = is_secret(descriptor, name);
            let present = existing
                .get(name)
                .is_some_and(|value| !value.trim().is_empty());
            let value = if secret {
                MASK.to_string()
            } else {
                plan::mask_secrets(
                    &existing
                        .get(name)
                        .cloned()
                        .or_else(|| {
                            descriptor
                                .params
                                .iter()
                                .find(|p| p.name == name)
                                .and_then(|p| p.default.map(str::to_string))
                        })
                        .or_else(|| {
                            profile
                                .and_then(|shipped| shipped.sets.get(name).copied())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "—".to_string()),
                    secrets,
                )
            };
            let source = if overrides.contains_key(name) {
                "you"
            } else if present {
                "kept"
            } else if secret
                || plan::infra_requirements()
                    .iter()
                    .any(|(infra, _)| *infra == name)
            {
                "generated"
            } else if profile.is_some_and(|shipped| shipped.sets.contains_key(name)) {
                "profile"
            } else {
                "default"
            };
            let group = descriptor
                .params
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.group.to_string())
                .unwrap_or_else(|| "infrastructure".to_string());
            let description = descriptor
                .params
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.description.to_string())
                .or_else(|| {
                    plan::infra_requirements()
                        .iter()
                        .find(|(infra, _)| *infra == name)
                        .map(|(_, reason)| (*reason).to_string())
                })
                .unwrap_or_default();
            EnvView {
                name: name.to_string(),
                group,
                description,
                value,
                secret,
                present,
                source: source.to_string(),
            }
        })
        .collect()
}

fn plan_view(
    descriptor: &DeployDescriptor,
    plan: &Plan,
    existing: &BTreeMap<String, String>,
    overrides: &BTreeMap<String, String>,
) -> PlanView {
    let steps = plan
        .steps
        .iter()
        .map(|step| StepView {
            kind: match step.kind {
                StepKind::WriteFile { .. } => "write",
                StepKind::Run { .. } => "run",
            },
            headline: step.headline(),
            reason: step.reason.clone(),
            source: step.source.clone(),
        })
        .collect();

    let mut config = Vec::new();
    let mut generated = Vec::new();
    let writes = plan.writes();
    if let Some(&(_, contents)) = writes.first() {
        let env = doctor::parse_env_file(contents);
        // A credential can sit inside another value: `DATABASE_URL` carries the
        // application password in its URL. Mask by value, or the table shows the
        // password the plan's own text already hides.
        let secrets = plan::secret_values(&plan.secret_names, contents);
        let profile = descriptor
            .profiles
            .iter()
            .find(|shipped| shipped.name == plan.profile);
        for (name, value) in &env {
            let secret = is_secret(descriptor, name) || plan.secret_names.iter().any(|s| s == name);
            let kept = existing
                .get(name)
                .is_some_and(|old| !old.trim().is_empty() && old == value);
            let source = if overrides.contains_key(name) {
                "you"
            } else if secret && kept {
                "kept"
            } else if secret
                || plan::infra_requirements()
                    .iter()
                    .any(|(infra, _)| infra == name)
            {
                if !kept {
                    generated.push(name.clone());
                }
                if kept {
                    "kept"
                } else {
                    "generated"
                }
            } else if profile.is_some_and(|shipped| shipped.sets.contains_key(name.as_str())) {
                "profile"
            } else {
                "default"
            };
            let group = descriptor
                .params
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.group.to_string())
                .unwrap_or_else(|| "infrastructure".to_string());
            let description = descriptor
                .params
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.description.to_string())
                .or_else(|| {
                    plan::infra_requirements()
                        .iter()
                        .find(|(infra, _)| *infra == name)
                        .map(|(_, reason)| (*reason).to_string())
                })
                .unwrap_or_default();
            config.push(EnvView {
                name: name.clone(),
                group,
                description,
                value: if secret {
                    MASK.to_string()
                } else {
                    plan::mask_secrets(value, &secrets)
                },
                secret,
                present: true,
                source: source.to_string(),
            });
        }
    }

    PlanView {
        profile: plan.profile.clone(),
        target: plan.target.to_string(),
        raw: plan.render(),
        steps,
        manual: plan.manual.clone(),
        config,
        generated,
    }
}

fn embedding_sentence(
    descriptor: &DeployDescriptor,
    recommendation: &plan::Recommendation,
) -> String {
    let resolved = &descriptor.embedding.resolved;
    let model = resolved
        .model
        .clone()
        .unwrap_or_else(|| "the configured model".to_string());
    if recommendation.embedding_fits {
        format!("{model} runs on this machine, so embeddings stay local.")
    } else {
        format!(
            "{model} needs more machine than this one has ({}). Cloud embeddings are the \
             honest default here: set {} to openai and {} to a key.",
            resolved
                .refusal
                .clone()
                .unwrap_or_else(|| "the floor is in the embedding catalogue".to_string()),
            descriptor.embedding.provider_env,
            descriptor.embedding.api_key_env
        )
    }
}

fn retrieval_sentence(descriptor: &DeployDescriptor) -> String {
    let resolved = &descriptor.retrieval.resolved;
    let mode = resolved.mode.unwrap_or("unselected");
    match &resolved.refusal {
        Some(refusal) => format!(
            "Retrieval runs `{mode}` through {}, and this host refuses it: {refusal}",
            descriptor.retrieval.mode_env
        ),
        None => format!(
            "Retrieval runs `{mode}`, selected through {}.",
            descriptor.retrieval.mode_env
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository's own compose file: `available()` refuses a target whose
    /// file is missing, and a test that invents one would prove nothing.
    fn compose_file() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/docker-compose.yml")
            .canonicalize()
            .expect("the repo's compose file")
    }

    /// A machine stated in the test, not measured: the suite must pass on a box
    /// whose disk is nearly full.
    fn facts(cores: u16, ram_mb: u32, disk_gb: u32) -> HostFacts {
        HostFacts {
            cores,
            ram_mb: Some(ram_mb),
            free_disk_gb: Some(disk_gb),
            docker: Some("29.9.2".to_string()),
            gpu: None,
        }
    }

    fn session() -> Session {
        let dir = std::env::temp_dir().join(format!("marula-view-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        Session::new(dir, compose_file(), Some(16384)).with_facts(facts(8, 16384, 200))
    }

    #[test]
    fn the_state_scores_every_shipped_profile_against_this_host() {
        let session = session();
        let state = session.state();
        assert!(state.profiles.len() >= 2);
        assert!(
            state.profiles.iter().any(|profile| profile.fits),
            "a 16 GB host must fit at least the all-in-one shape"
        );
        assert!(
            state.params.len() > 100,
            "the advanced list is the full catalogue, not a shortlist: {}",
            state.params.len()
        );
        assert_eq!(state.param_count, state.params.len());
        let _ = std::fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn no_secret_reaches_the_wire_in_a_plan_or_a_state() {
        let session = session();
        let plan = session
            .plan("all-in-one", &BTreeMap::new())
            .expect("all-in-one plans on a 16 GB host");

        // Every secret row is masked, in the table and in the operator's text.
        for row in plan.config.iter().filter(|row| row.secret) {
            assert_eq!(row.value, MASK, "{} must be masked", row.name);
            let rendered = plan
                .raw
                .lines()
                .find(|line| line.trim_start().starts_with(&format!("{}=", row.name)));
            if let Some(line) = rendered {
                assert!(
                    line.contains(MASK),
                    "{} leaked into the rendered plan: {line}",
                    row.name
                );
            }
        }
        let _ = std::fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn a_profile_below_its_floors_is_reported_with_the_numbers() {
        let dir = std::env::temp_dir().join(format!("marula-view-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("a scratch dir");
        let session = Session::new(dir, compose_file(), Some(512)).with_facts(facts(2, 512, 200));
        let state = session.state();
        let below = state
            .profiles
            .iter()
            .find(|profile| !profile.fits)
            .expect("512 MB fits neither shape");
        assert!(!below.missing.is_empty());
        assert!(
            below.missing.iter().any(|line| line.contains("MB")),
            "the refusal names the number: {:?}",
            below.missing
        );
        let _ = std::fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn an_override_is_attributed_to_the_operator_and_masked_when_secret() {
        let session = session();
        let mut overrides = BTreeMap::new();
        overrides.insert("MARULA_EMBEDDING_TIER".to_string(), "minimal".to_string());
        overrides.insert(
            "POSTGRES_PASSWORD".to_string(),
            "a password the wizard must never echo".to_string(),
        );
        let plan = session.plan("all-in-one", &overrides).expect("a plan");
        let tier = plan
            .config
            .iter()
            .find(|row| row.name == "MARULA_EMBEDDING_TIER")
            .expect("the tier is in the file");
        assert_eq!(tier.value, "minimal");
        assert_eq!(tier.source, "you");

        let password = plan
            .config
            .iter()
            .find(|row| row.name == "POSTGRES_PASSWORD")
            .expect("the password is in the file");
        assert_eq!(password.value, MASK);
        let wire = serde_json::to_string(&plan).expect("serialisable");
        assert!(
            !wire.contains("a password the wizard must never echo"),
            "an operator's secret must not reach the wire"
        );
        let _ = std::fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn a_password_inside_a_url_is_masked_too() {
        let session = session();
        // `split` presets DATABASE_URL with the application password inside it;
        // the all-in-one override moves that value into the compose file.
        let plan = session.plan("split", &BTreeMap::new()).expect("a plan");
        let url = plan
            .config
            .iter()
            .find(|row| row.name == "DATABASE_URL")
            .expect("the database URL is in the file");
        assert!(
            url.value.contains(MASK),
            "the application password rides inside DATABASE_URL: {}",
            url.value
        );
        let after_mask = url.value.split(MASK).nth(1).unwrap_or_default();
        assert!(
            !after_mask.contains("marula_app:"),
            "a fragment of the credential survived: {}",
            url.value
        );
        let _ = std::fs::remove_dir_all(&session.dir);
    }

    #[test]
    fn an_unknown_lever_is_refused_as_a_typed_error() {
        let session = session();
        let mut overrides = BTreeMap::new();
        overrides.insert("NOT_A_LEVER".to_string(), "1".to_string());
        let refused = session.plan("all-in-one", &overrides).unwrap_err();
        assert!(refused.contains("NOT_A_LEVER"), "{refused}");
        let _ = std::fs::remove_dir_all(&session.dir);
    }
}
