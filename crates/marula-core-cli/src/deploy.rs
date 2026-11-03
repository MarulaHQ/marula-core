//! The deploy descriptor (Wave 49 lane R7) — the interface a commercial
//! control plane consumes.
//!
//! One image runs every role, and a deployment is parameterized by runtime env
//! vars. This module is the machine-readable description of that contract: the
//! image and its roles, the deployment profiles, and the full catalogue of
//! runtime parameters with their kinds, defaults and grouping.
//!
//! Rust is the single source of truth. `marula deploy describe --json` emits
//! the descriptor for that consumer to pin; `marula deploy profiles --write`
//! regenerates `deploy/profiles/*.yaml`, and a test asserts the checked-in
//! files match this code so the human-readable copy cannot drift from the
//! contract.
//!
//! No control-plane logic lives here. marula-core states the interface; the
//! consumer decides what to render from it.

use std::path::{Path, PathBuf};

/// Schema version of the emitted descriptor. Bump only on a breaking shape
/// change, so a consumer can refuse a version it does not understand.
///
/// `2` (Wave 60 lane W60-1) added `consequence`, `default_rationale`,
/// `requires` and `conflicts` to every parameter, and `floors` to every
/// profile. A consumer that rendered `1` renders `2` without loss, but one that
/// validates strictly would refuse the extra keys, so the bump is honest.
pub const SCHEMA_VERSION: u32 = 2;

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

/// What a machine must offer before a profile is an honest choice.
///
/// A lower bound, not a budget. The embedding model the profile presets has to
/// fit inside it — the descriptor check refuses a profile that presets a local
/// model its own floors cannot host — and the wizard measures a host against
/// these numbers rather than guessing from a product name.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ResourceFloors {
    /// Logical cores.
    pub cores: u16,
    /// RAM in mebibytes.
    pub ram_mb: u32,
    /// Free disk in gibibytes.
    pub disk_gb: u32,
}

/// A named deployment shape.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Profile {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Parameters this profile presets, name to value. Everything not listed
    /// keeps its catalogue default.
    pub sets: std::collections::BTreeMap<&'static str, &'static str>,
    /// What the host has to offer for this shape. The wizard scores a host
    /// against this; the descriptor check refuses a preset the floors forbid.
    pub floors: ResourceFloors,
    pub services: Vec<Service>,
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
    /// What the lever is, in one line. Terse on purpose: it labels a field.
    pub description: &'static str,
    /// What changes for the operator when the lever moves: who notices, what it
    /// costs, what breaks. Empty means nobody wrote one, and the descriptor
    /// check refuses that — this is the sentence a wizard renders instead of
    /// inventing a second explanation.
    pub consequence: &'static str,
    /// Why the default is this value. `None` only when the lever has no
    /// default; a default with no stated reason is a check failure.
    pub default_rationale: Option<&'static str>,
    /// Levers that must be set alongside this one when it is in use. Empty for
    /// a lever that stands alone.
    pub requires: Vec<&'static str>,
    /// Levers that must not be set at the same time, because the process reads
    /// one of them and refuses or ignores the other.
    pub conflicts: Vec<&'static str>,
    /// The values this lever accepts, when the process accepts only a closed
    /// set. Empty means free text. The check refuses a default or a profile
    /// preset outside the set, so a wizard can render a select without deciding
    /// for itself what the process parses.
    pub values: &'static [&'static str],
}

impl Param {
    /// The operator-facing consequence. Required: the check fails on a lever
    /// that has none, or one too short to say anything.
    #[must_use]
    fn consequence(mut self, consequence: &'static str) -> Self {
        self.consequence = consequence;
        self
    }

    /// Why this default, rather than another value. Required whenever
    /// [`Param::default`] is `Some`.
    #[must_use]
    fn rationale(mut self, rationale: &'static str) -> Self {
        self.default_rationale = Some(rationale);
        self
    }

    /// Levers this one needs to be in use.
    #[must_use]
    fn needs(mut self, levers: &[&'static str]) -> Self {
        self.requires = levers.to_vec();
        self
    }

    /// Levers this one cannot be set beside.
    #[must_use]
    fn conflicts(mut self, levers: &[&'static str]) -> Self {
        self.conflicts = levers.to_vec();
        self
    }
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
    /// The retry matrix, published in the same vocabulary as the env group a
    /// consumer renders from it. The numbers are `core_domain::retry`'s — this
    /// is a projection, not a second copy.
    pub retries: RetrySpec,
    /// The embedding catalogue, the selection this environment resolves to, and
    /// the re-index pace (Wave 52 lane R14). Projected from
    /// `core_domain::embedding`, so a descriptor reader and the process that
    /// boots see the same table, the same floors and the same refusal wording.
    pub embedding: EmbeddingSpec,
    /// The retrieval mode (Wave 52 lane R15): the lever, the three modes with
    /// their measured consequence, the requirement each one carries, whether
    /// this deployment can run the selected one, and the per-tenant embedding
    /// distribution. The policy is `core_domain::retrieval`'s; this is a
    /// projection plus a live probe, never a second copy of the rules.
    pub retrieval: RetrievalSpec,
}

/// Every retry preset and the budgets it sets, one entry per subsystem.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrySpec {
    /// The env that selects the preset (`MARULA_ROBUSTNESS`).
    pub preset_env: &'static str,
    /// The subsystem→lever env group, so a consumer can render an override
    /// beside each budget without hard-coding the pattern.
    pub lever_env_template: &'static str,
    pub presets: Vec<RetryPreset>,
}

/// One preset and what it does.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPreset {
    pub name: &'static str,
    pub policies: Vec<RetryPolicySpec>,
}

/// One subsystem's resolved budget.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetryPolicySpec {
    pub subsystem: &'static str,
    pub base_ms: u64,
    pub factor: u32,
    pub cap_ms: u64,
    /// Total tries including the first. `0` means the budget is unlimited.
    pub attempts: u32,
    pub jitter: &'static str,
    /// What a `409` means here.
    pub on_conflict: &'static str,
}

/// The retry section of the descriptor, built from `core_domain::retry`.
#[must_use]
pub fn retry_spec() -> RetrySpec {
    use core_domain::retry::{RobustnessPreset, Subsystem};

    let presets = RobustnessPreset::ALL
        .into_iter()
        .map(|preset| RetryPreset {
            name: preset.as_str(),
            policies: Subsystem::ALL
                .into_iter()
                .map(|subsystem| {
                    let policy = preset.policy(subsystem);
                    RetryPolicySpec {
                        subsystem: subsystem.as_str(),
                        base_ms: u64::try_from(policy.base.as_millis()).unwrap_or(u64::MAX),
                        factor: policy.factor,
                        cap_ms: u64::try_from(policy.cap.as_millis()).unwrap_or(u64::MAX),
                        attempts: policy.budget.limit().unwrap_or(0),
                        jitter: policy.jitter.as_str(),
                        on_conflict: policy.on_conflict.as_str(),
                    }
                })
                .collect(),
        })
        .collect();
    RetrySpec {
        preset_env: core_domain::retry::ROBUSTNESS_ENV,
        lever_env_template: "MARULA_RETRY_<SUBSYSTEM>_<LEVER>",
        presets,
    }
}

/// The embedding section: what can be selected, what this environment selected,
/// and how fast the re-index may run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddingSpec {
    pub tier_env: &'static str,
    pub model_env: &'static str,
    pub provider_env: &'static str,
    pub api_key_env: &'static str,
    pub custom_models_env: &'static str,
    pub host_memory_env: &'static str,
    /// Every selectable model, shipped and custom, in catalogue order.
    pub models: Vec<EmbeddingModelSpec>,
    /// What this environment resolves to, and whether this host can host it.
    pub resolved: EmbeddingResolved,
    /// The re-index pace and window.
    pub reindex: EmbeddingReindexSpec,
}

/// One catalogue entry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddingModelSpec {
    pub id: String,
    pub display_name: String,
    pub dimensions: u16,
    /// `text`, or the modalities a multimodal build accepts.
    pub modality: String,
    pub license: String,
    pub provider: String,
    /// Build version recorded in the space tag.
    pub version: String,
    pub tier: &'static str,
    /// Local capacity required. Zero cores and zero megabytes means the model
    /// runs elsewhere and has no floor here.
    pub cores: u16,
    pub ram_mb: u32,
    /// `model@version/dimensions`: the tag every vector from this entry carries.
    pub space: String,
    pub quality_note: String,
    pub platform_note: String,
}

/// The selection this environment makes, or the reason it makes none.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmbeddingResolved {
    pub tier: Option<&'static str>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub space: Option<String>,
    /// `None` when this host can host the selection or the host publishes no
    /// memory reading. Otherwise the exact refusal the boot path prints.
    pub refusal: Option<String>,
    pub host_cores: u16,
    pub host_ram_mb: Option<u32>,
}

/// The re-index sweep, as it would run.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct EmbeddingReindexSpec {
    pub batch: i64,
    pub rows_per_min: u32,
    /// How often an idle sweep looks again; a tick with work is paced instead.
    pub idle_secs: u64,
    /// Inclusive start and exclusive end hour, UTC. `None` runs any hour.
    pub window: Option<(u8, u8)>,
}

/// The embedding section of the descriptor, resolved against this environment.
///
/// Resolution runs the same functions the processes run, which is the point: a
/// descriptor that restated the table by hand would drift from the selection a
/// boot makes, and the drift would be invisible until a search returned nothing.
#[must_use]
pub fn embedding_spec() -> EmbeddingSpec {
    use core_domain::embedding::{Catalogue, ReindexPolicy};

    let lookup = |key: &str| std::env::var(key).ok();
    let catalogue = Catalogue::from_lookup(&lookup);
    let models = catalogue
        .entries()
        .iter()
        .map(|entry| EmbeddingModelSpec {
            id: entry.id.to_string(),
            display_name: entry.display_name.to_string(),
            dimensions: entry.dimensions,
            modality: entry.modality.to_string(),
            license: entry.license.to_string(),
            provider: entry.provider.to_string(),
            version: entry.version.to_string(),
            tier: entry.tier.as_str(),
            cores: entry.floor.cores,
            ram_mb: entry.floor.ram_mb,
            space: entry.space().id(),
            quality_note: entry.quality_note.to_string(),
            platform_note: entry.platform_note.to_string(),
        })
        .collect();
    let resolved = match core_domain::embedding::resolve_instance(&lookup) {
        Ok((selection, host)) => EmbeddingResolved {
            tier: Some(selection.entry.tier.as_str()),
            provider: Some(selection.provider.clone()),
            model: Some(selection.entry.id.to_string()),
            space: Some(selection.space_id()),
            refusal: None,
            host_cores: host.cores,
            host_ram_mb: host.ram_mb,
        },
        Err(error) => {
            // A refused selection still reports what it resolved to, so the
            // reader can see the model whose floor is not met.
            let host = core_domain::embedding::detect_host_capacity(&lookup);
            let (tier, provider, model, space) = match core_domain::embedding::select_instance(
                &catalogue,
                lookup(core_domain::embedding::TIER_ENV).as_deref(),
                lookup(core_domain::embedding::MODEL_ENV).as_deref(),
                lookup(core_domain::embedding::PROVIDER_ENV).as_deref(),
            ) {
                Ok(selection) => (
                    Some(selection.entry.tier.as_str()),
                    Some(selection.provider.clone()),
                    Some(selection.entry.id.to_string()),
                    Some(selection.space_id()),
                ),
                Err(_) => (None, None, None, None),
            };
            EmbeddingResolved {
                tier,
                provider,
                model,
                space,
                refusal: Some(error.to_string()),
                host_cores: host.cores,
                host_ram_mb: host.ram_mb,
            }
        }
    };
    let policy = ReindexPolicy::from_lookup(&lookup);
    EmbeddingSpec {
        tier_env: core_domain::embedding::TIER_ENV,
        model_env: core_domain::embedding::MODEL_ENV,
        provider_env: core_domain::embedding::PROVIDER_ENV,
        api_key_env: core_domain::embedding::API_KEY_ENV,
        custom_models_env: core_domain::embedding::CUSTOM_MODELS_ENV,
        host_memory_env: core_domain::embedding::HOST_MEMORY_ENV,
        models,
        resolved,
        reindex: EmbeddingReindexSpec {
            batch: policy.batch,
            rows_per_min: policy.rows_per_min,
            idle_secs: policy.interval_secs,
            window: policy.window,
        },
    }
}

/// The retrieval-mode section of the descriptor.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalSpec {
    /// The lever that selects the mode.
    pub mode_env: &'static str,
    /// Embeddings in one tenant at which the exact scan stops fitting the
    /// vector leg's database share. The distribution marks tenants past it.
    pub threshold_embeddings: u64,
    /// Which table the mode governs. Named explicitly so nobody reads the
    /// descriptor as claiming the connector RAG vectors changed too.
    pub applies_to: Vec<&'static str>,
    /// Every mode with its measured consequence.
    pub modes: Vec<RetrievalModeSpec>,
    /// What this environment resolves to and whether it can run.
    pub resolved: RetrievalResolved,
    /// Per-tenant embedding counts, when a database was reachable.
    pub distribution: Option<RetrievalDistribution>,
}

/// One retrieval mode, in the operator's words.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalModeSpec {
    pub name: &'static str,
    pub default: bool,
    pub summary: &'static str,
    pub latency: &'static str,
    pub index_memory: &'static str,
    pub recall: &'static str,
    /// Model time plus database time against the 50 ms vector leg.
    pub leg_note: &'static str,
    pub recommend_when: &'static str,
    /// What the database must provide. Empty for `exact`.
    pub requires: Vec<&'static str>,
}

/// The mode this environment selected, or the reason it selected none.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalResolved {
    pub mode: Option<&'static str>,
    /// `null` when the mode can run. The same sentence the boot would print.
    pub refusal: Option<String>,
    /// `live` when a database answered, `unreachable` when one was configured
    /// but did not, `not_run` when there was none to ask.
    pub probe: &'static str,
    /// The connection fault, when `probe` is `unreachable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgvector_version: Option<String>,
    pub vchord_installed: bool,
    pub vchord_preloaded: bool,
    pub hnsw_index: bool,
    pub vchord_index: bool,
}

/// One tenant's embedding count.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalTenantSpec {
    pub org_id: String,
    pub embeddings: i64,
    pub index_recommended: bool,
}

/// The distribution report. It is deliberately not a mean: the operator needs
/// to see WHICH tenant crossed the threshold.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RetrievalDistribution {
    pub threshold_embeddings: u64,
    pub tenants: Vec<RetrievalTenantSpec>,
    pub tenants_over_threshold: usize,
    pub total_embeddings: i64,
    /// What the numbers say to do. The mode still changes by hand.
    pub recommendation: String,
}

/// The retrieval section with no database probe. Kept separate so
/// [`descriptor`] stays cheap and sync: a descriptor read must work on a host
/// that cannot reach the database, and say that it did not ask.
#[must_use]
pub fn retrieval_spec() -> RetrievalSpec {
    retrieval_spec_from(None, None, None)
}

fn retrieval_spec_from(
    probe: Option<&core_db::retrieval::Probe>,
    distribution: Option<Vec<core_db::retrieval::TenantDistribution>>,
    database_error: Option<String>,
) -> RetrievalSpec {
    use core_domain::retrieval::{RetrievalMode, INDEX_THRESHOLD_EMBEDDINGS, MODE_ENV};

    let modes = RetrievalMode::ALL
        .into_iter()
        .map(|mode| {
            let consequence = mode.consequence();
            RetrievalModeSpec {
                name: mode.as_str(),
                default: mode.is_default(),
                summary: consequence.summary,
                latency: consequence.latency,
                index_memory: consequence.index_memory,
                recall: consequence.recall,
                leg_note: consequence.leg_note,
                recommend_when: consequence.recommend_when,
                requires: mode.requires().to_vec(),
            }
        })
        .collect();
    let (mode, mode_error) = match RetrievalMode::from_lookup(&|key| std::env::var(key).ok()) {
        Ok(mode) => (Some(mode), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let (probe_state, refusal) = if let Some(error) = database_error.as_ref() {
        (
            "unreachable",
            Some(format!(
                "the retrieval mode was not checked: the database could not be reached ({error})"
            )),
        )
    } else if let Some(probe) = probe {
        match &probe.verdict {
            Ok(()) => ("live", mode_error),
            Err(refusal) => ("live", Some(refusal.to_string())),
        }
    } else {
        ("not_run", mode_error)
    };
    let resolved = RetrievalResolved {
        mode: mode.map(RetrievalMode::as_str),
        refusal,
        probe: probe_state,
        database_error,
        pgvector_version: probe.and_then(|p| p.capabilities.pgvector_version.clone()),
        vchord_installed: probe.is_some_and(|p| p.capabilities.vchord_installed),
        vchord_preloaded: probe.is_some_and(|p| p.capabilities.preloads("vchord")),
        hnsw_index: probe
            .is_some_and(|p| p.capabilities.has_index(core_domain::retrieval::HNSW_INDEX)),
        vchord_index: probe.is_some_and(|p| {
            p.capabilities
                .has_index(core_domain::retrieval::VCHORD_INDEX)
        }),
    };
    let distribution = distribution.map(|tenants| {
        let over = tenants.iter().filter(|t| t.index_recommended).count();
        let total = tenants.iter().map(|t| t.embeddings).sum();
        let recommendation = if tenants.is_empty() {
            "No vectors stored yet. The mode has nothing to accelerate.".to_string()
        } else if over == 0 {
            format!(
                "Every tenant is below {INDEX_THRESHOLD_EMBEDDINGS} embeddings. Keep \
                 `exact`: the scan is the true nearest neighbours and costs no index memory."
            )
        } else {
            let biggest = tenants.first().map_or(0, |t| t.embeddings);
            format!(
                "{over} of {} tenants are at or past {INDEX_THRESHOLD_EMBEDDINGS} \
                 embeddings (largest {biggest}). That is the shape `index` is for: a \
                 per-org deployment on Postgres you control. `iterative` is the managed \
                 alternative. Change the mode by hand.",
                tenants.len()
            )
        };
        RetrievalDistribution {
            threshold_embeddings: INDEX_THRESHOLD_EMBEDDINGS,
            tenants: tenants
                .into_iter()
                .map(|t| RetrievalTenantSpec {
                    org_id: t.org_id.to_string(),
                    embeddings: t.embeddings,
                    index_recommended: t.index_recommended,
                })
                .collect(),
            tenants_over_threshold: over,
            total_embeddings: total,
            recommendation,
        }
    });
    RetrievalSpec {
        mode_env: MODE_ENV,
        threshold_embeddings: INDEX_THRESHOLD_EMBEDDINGS,
        applies_to: vec!["search_embeddings"],
        modes,
        resolved,
        distribution,
    }
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
        // Every call site fills these in with the builder below. An empty
        // consequence is the state the check refuses, so forgetting one is a
        // failing check rather than a blank sentence in a wizard.
        consequence: "",
        default_rationale: None,
        requires: Vec::new(),
        conflicts: Vec::new(),
        values: &[],
    }
}

/// The closed vocabularies this descriptor publishes.
///
/// Purpose: a lever whose parser accepts a fixed set is rendered as a select,
/// and the set is the one the process parses — `minimal | standard | advanced`
/// from `core_domain::embedding::EmbeddingTier`, `exact | iterative | index`
/// from `core_domain::retrieval::RetrievalMode`, `local | s3` from
/// `core_store::StoreChoice`, `none | smtp | postmark | ses` from
/// `core_mail::TransportKind`, and `ollama | openai` from the embedding
/// provider. Everything not listed here is a string on purpose: free text with
/// an example is more honest than a select that omits a legal value.
///
/// Non-purpose: it is not a second parser. The default and every profile preset
/// are checked against these lists, so a value the process would reject cannot
/// be the one the installer writes.
fn choices(name: &str) -> &'static [&'static str] {
    match name {
        "MARULA_EMBEDDING_PROVIDER" => &["ollama", "openai"],
        "MARULA_EMBEDDING_TIER" => &["minimal", "standard", "advanced"],
        "MARULA_RETRIEVAL_MODE" => &["exact", "iterative", "index"],
        "OBJECT_STORE" => &["local", "s3"],
        "EMAIL_TRANSPORT" => &["none", "smtp", "postmark", "ses"],
        _ => &[],
    }
}

/// Attaches each lever's closed vocabulary after the catalogue is built.
fn with_choices(mut params: Vec<Param>) -> Vec<Param> {
    for param in &mut params {
        param.values = choices(param.name);
    }
    params
}

/// A parameter the process refuses to boot without. Today that is the four
/// seam and signing secrets, checked in `auth`/`secrets`; every other secret is
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
        params: with_choices(params()),
        retries: retry_spec(),
        embedding: embedding_spec(),
        retrieval: retrieval_spec(),
    }
}

fn profiles() -> Vec<Profile> {
    use std::collections::BTreeMap;

    // The shipped shape is POOLED (Wave 52 lane P9). `DATABASE_URL` names the
    // transaction pooler, `MARULA_LISTEN_DATABASE_URL` names Postgres directly
    // because a LISTEN cannot ride a pooled connection, and the statement
    // cache is off because a prepared statement name must not outlive the
    // transaction that created it. `docs/reference/connection-pooling.md`
    // carries the measurements behind all three.
    let split = Profile {
        name: "split",
        title: "Split services behind a transaction pooler",
        description: "The default topology. core-api and worker run as separate processes, so each scales and restarts on its own, with PgBouncer in transaction mode in front of Postgres. The compose stack ships this shape. Pointing DATABASE_URL straight at Postgres works only once you raise MARULA_DB_STATEMENT_CACHE, and it brings back the connection ceiling the pooler was hiding. The floors come from the shipped embedding model, which needs four cores of its own, plus room for Postgres, the pooler and the object store.",
        sets: BTreeMap::from([
            (
                "DATABASE_URL",
                "postgres://marula_app:${MARULA_APP_PASSWORD}@pgbouncer:6432/marula",
            ),
            (
                "MARULA_LISTEN_DATABASE_URL",
                "postgres://marula_app:${MARULA_APP_PASSWORD}@postgres:5432/marula",
            ),
            ("MARULA_DB_STATEMENT_CACHE", "0"),
            ("MARULA_REPORTING_POOL_MAX", "8"),
        ]),
        floors: ResourceFloors {
            cores: 4,
            ram_mb: 8192,
            disk_gb: 40,
        },
        services: vec![
            Service {
                name: "core-api",
                role: "core-api",
                ports: vec![8080],
                needs: vec!["postgres", "redis", "pgbouncer"],
            },
            Service {
                name: "worker",
                role: "worker",
                ports: vec![8082],
                needs: vec!["postgres", "redis", "pgbouncer"],
            },
        ],
    };

    let all_in_one = Profile {
        name: "all-in-one",
        title: "All-in-one (demo / one-box)",
        description: "One core-api process hosts the worker loops and serves the SSE live-push stream, so a small box runs a single application container. See docs/all-in-one.md. It runs unpooled on purpose. One instance's 54 connections fit inside the default 100 a stock Postgres allows, so the demo box does not need a second moving part. The floors are the shipped embedding model's four cores, plus the database and cache it shares the box with. The disk floor is one tier smaller than split's because there is no second image copy.",
        floors: ResourceFloors {
            cores: 4,
            ram_mb: 8192,
            disk_gb: 30,
        },
        sets: BTreeMap::from([
            ("MARULA_ALL_IN_ONE", "1"),
            // One container, so the queue's retry window is short: there is no
            // second replica to absorb a long one, and the operator is the
            // only one watching.
            ("MARULA_ROBUSTNESS", "single_box"),
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
        services: vec![Service {
            name: "core-api",
            role: "core-api",
            ports: vec![8080, 8082],
            needs: vec!["postgres", "redis"],
        }],
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
        )
        .consequence("One container runs both roles. The background loops share core-api's memory and restart with it, so a fault the split shape contained now takes the API down too.")
        .rationale("Split is the default because a stalled worker restarts without interrupting the API a user is talking to.")
        .needs(&["MARULA_WORKER_DATABASE_URL"]),
        param(
            "BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8080"),
            false,
            "core-api HTTP listen address.",
        )
        .consequence("The address and port core-api binds. A loopback-only value is unreachable from a proxy in another container, and a taken port stops the API at boot.")
        .rationale("The shipped compose stack and chart both target 0.0.0.0:8080. A loopback address would break container networking."),
        param(
            "MARULA_WORKER_BIND_ADDR",
            "topology",
            String,
            Some("0.0.0.0:8082"),
            false,
            "Worker health-endpoint address in all-in-one mode.",
        )
        .consequence("Where the worker health endpoint answers when the loops run inside core-api. A split deployment never binds it, so the value changes nothing there.")
        .rationale("8082 is the port the compose health check and the chart probe both ask for."),
        param(
            "PUBLIC_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:8080"),
            false,
            "Public base URL of the API, used to build OIDC redirect URIs.",
        )
        .consequence("The base for every link Marula puts in mail and for OIDC redirect URIs. An address a browser cannot reach turns a successful login into an error page.")
        .rationale("localhost:8080 is the dev API. A deployment behind a domain has to set its public name, or invitations carry a link nobody can open."),
        param(
            "WEB_BASE_URL",
            "topology",
            Url,
            Some("http://localhost:3000"),
            false,
            "Web app base URL the SSO callback lands the browser on.",
        )
        .consequence("Where the SSO callback sends the browser once the identity provider answers. The wrong port authenticates the user and then drops them on a dead page.")
        .rationale("3000 is the dev web port, and the development identity provider redirects there."),
        param(
            "CORS_ORIGIN",
            "topology",
            String,
            None,
            false,
            "Allowed browser origin for the API. Unset disables CORS.",
        )
        .consequence("The one browser origin allowed to call the API. Unset disables CORS entirely, which a same-origin deployment can live with and a split web host cannot.")
        .rationale("Unset by default, because an allowlist that ships open is a hole nobody notices until someone finds it."),

        // database
        param(
            "DATABASE_URL",
            "database",
            Url,
            Some("postgres://marula_app:marula_app@localhost:15432/marula"),
            false,
            "RLS-scoped app-role Postgres URL. Set it in production.",
        )
        .consequence("The role every tenant-scoped query rides. A URL for the owner role instead of the app role removes row-level security from the API in one edit.")
        .rationale("The dev compose database, on the app role, at port 15432. A deployment replaces it rather than relying on it."),
        param(
            "MARULA_WORKER_DATABASE_URL",
            "database",
            Url,
            None,
            false,
            "Owner-role Postgres URL for the worker loops. Required when MARULA_ALL_IN_ONE=1.",
        )
        .consequence("The owner-role connection the background loops use for work no tenant scope can express. All-in-one refuses to start without it, and split hands it to the worker.")
        .rationale("No default on purpose. It carries DDL and cross-tenant work, and guessing an owner URL is how a worker ends up running with the wrong privileges."),
        param(
            "MARULA_DB_POOL_MAX",
            "database",
            Integer,
            Some("20"),
            false,
            "App-pool ceiling. Sizes both the API app pool and, in all-in-one, the worker pool.",
        )
        .consequence("The ceiling on the API's application connections. Each connection is a Postgres backend with its own memory, and a pool the database will not accept turns a load spike into refusals.")
        .rationale("20 is the per-process ceiling the capacity model assumes. Four pools add up to 54 connections per instance, which the pooler holds at 15 server side."),
        param(
            "MARULA_REPORTING_POOL_MAX",
            "database",
            Integer,
            Some("8"),
            false,
            "Reporting-pool ceiling (app role under RLS).",
        )
        .consequence("The ceiling on the reporting pool, which runs under the app role and RLS. Reports queue at the ceiling instead of competing with the hot path for connections.")
        .rationale("8 is what the hot path saturates at, measured in Wave 52 lane P9 and down from 25. The reports surface is cold, so the extra connections bought nothing."),
        param(
            "DELIVERY_POOL_MAX",
            "database",
            Integer,
            Some("5"),
            false,
            "Worker delivery reporting-pool ceiling.",
        )
        .consequence("The ceiling on the worker delivery pool. Too high and a slow delivery sweep starves the database. Too low and scheduled reports wait behind each other.")
        .rationale("5 matches the measured delivery concurrency: one sweep at a time with a little headroom."),
        param(
            "MARULA_DB_STATEMENT_CACHE",
            "database",
            Integer,
            Some("0"),
            false,
            "sqlx prepared-statement cache capacity per connection. 0 (the default) disables it. See docs/reference/connection-pooling.md.",
        )
        .consequence("How many prepared statements one connection keeps. Non-zero behind a transaction pooler hands a statement name to a connection that no longer has it, and the failure appears at query time. Raise it no higher than the pooler's max_prepared_statements, and only with the pooler's reset query off.")
        .rationale("0 because the shipped pooler runs DISCARD ALL after every transaction. A cached name cannot survive that, so a cache would pay for re-parsing twice."),
        param(
            "MARULA_LISTEN_DATABASE_URL",
            "database",
            Url,
            None,
            false,
            "Direct Postgres URL for the process's ONE LISTEN connection. Required when DATABASE_URL names a transaction pooler.",
        )
        .consequence("The direct connection the process holds open for LISTEN, which a subscription needs because it cannot ride a pooled connection. Left unset behind a transaction pooler, live updates stop arriving and nothing reports an error.")
        .rationale("No default on purpose. Unset means reuse DATABASE_URL, which is correct only when that URL is not pooled."),

        // cache
        param(
            "REDIS_URL",
            "cache",
            Url,
            Some("redis://localhost:16379/0"),
            false,
            "Redis/Valkey URL for the outbox relay and stream consumers.",
        )
        .consequence("Where the outbox relay publishes and the live-push consumers subscribe. A wrong URL stops job dispatch and live updates while the API keeps answering requests.")
        .rationale("The dev compose Valkey on port 16379. A deployment points at its own instance."),

        // secrets
        requires(
            "JWT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. Access/refresh token signing. Boot refuses without it outside dev.",
        )
        .consequence("Signs access and refresh tokens. Rotating it signs every session out at once. A missing one outside dev stops the API before it serves a request.")
        .rationale("No default at all, because a signing key that ships with the product would be a shared secret across every install."),
        requires(
            "MARULA_SECRET_KEY",
            "secrets",
            Secret,
            None,
            true,
            "64 hex chars. AES-256 at-rest key for sealed integration secrets.",
        )
        .consequence("Encrypts integration credentials at rest. Rotating it without re-sealing the stored values makes every third-party secret unreadable, and integrations start failing one by one.")
        .rationale("No default at all. The key that seals customer credentials cannot come from the image."),
        requires(
            "MARULA_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Bearer token for /api/internal/* (worker to API machine surface).",
        )
        .consequence("The bearer the worker presents on the /api/internal surface. It has to match on both sides. A mismatch turns every worker call into a 401 that reads as a broken automation.")
        .rationale("No default at all, because a published bearer token is a published machine surface."),
        requires(
            "EMAIL_INTERNAL_TOKEN",
            "secrets",
            Secret,
            None,
            true,
            "Shared token for the email claim/report loop.",
        )
        .consequence("The shared token for the email claim and report loop. A mismatch leaves queued mail claimed and never sent, with no error on the API side.")
        .rationale("No default at all. It guards a seam between two processes, so it has to be chosen once and injected on both."),
        param(
            "EMAIL_INBOUND_HMAC_SECRET",
            "secrets",
            Secret,
            None,
            false,
            "HMAC secret for the inbound email webhook. Unset disables it.",
        )
        .consequence("Verifies the inbound email webhook. Unset disables the inbound path entirely, which is the safe state for a deployment that does not accept mail.")
        .rationale("Unset by default, because an inbound webhook nobody configured should not be open."),
        param(
            "MARULA_PLUGIN_SIGNING_KEYS",
            "secrets",
            String,
            None,
            false,
            "Trusted plugin-signing public keys (key_id=hex, comma-separated).",
        )
        .consequence("The public keys whose plugin signatures are trusted, written as key_id=hex pairs. Adding a key trusts every plugin that key signed, so this is a trust decision rather than a setting.")
        .rationale("No default, because a trust root that ships with the product would trust anything signed with the shipped key."),

        // objects
        param(
            "OBJECT_STORE",
            "objects",
            String,
            Some("local"),
            false,
            "Attachment store: local or s3.",
        )
        .consequence("Where attachment bytes live. Switching from local to s3 changes where new uploads go, and objects already written stay where they were, so a switch without a copy strands them.")
        .rationale("local is the default because a one-box deployment has a disk and should not need an object service to attach a file to a ticket."),
        param(
            "OBJECT_STORE_LOCAL_DIR",
            "objects",
            String,
            Some("./data/objects"),
            false,
            "Local object-store directory.",
        )
        .consequence("The directory attachments are written to. It has to be a volume that outlives the container, or every restart drops bytes the database still references.")
        .rationale("./data/objects matches the compose volume mount, so the default and the shipped stack agree."),
        param(
            "S3_ENDPOINT",
            "objects",
            Url,
            None,
            false,
            "S3/MinIO endpoint when OBJECT_STORE=s3.",
        )
        .consequence("The S3 or MinIO endpoint the store talks to. Unset lets the SDK infer AWS, which works on AWS and fails against a private MinIO.")
        .rationale("Unset means the AWS endpoint derived from the region, which is right for S3 proper.")
        .needs(&["OBJECT_STORE"]),
        param(
            "S3_BUCKET",
            "objects",
            String,
            Some("marula"),
            false,
            "S3 bucket name.",
        )
        .consequence("The bucket attachments are written into. Marula does not create it, so a missing bucket fails the first upload rather than the boot.")
        .rationale("One bucket named marula keeps the whole install in one place an operator can find.")
        .needs(&["OBJECT_STORE"]),
        param(
            "S3_REGION",
            "objects",
            String,
            Some("us-east-1"),
            false,
            "S3 region.",
        )
        .consequence("The region the object store signs requests for. A wrong region is a signature error on every request, not a slower path.")
        .rationale("us-east-1 is the widest default. A deployment sets its own region.")
        .needs(&["OBJECT_STORE"]),
        param(
            "S3_ACCESS_KEY_ID",
            "objects",
            Secret,
            None,
            true,
            "S3 access key id.",
        )
        .consequence("The credential used for attachment writes. It needs read and write on the one bucket and nothing else.")
        .rationale("No default, because object credentials belong to the deployment's own bucket.")
        .needs(&["OBJECT_STORE"]),
        param(
            "S3_SECRET_ACCESS_KEY",
            "objects",
            Secret,
            None,
            true,
            "S3 secret access key.",
        )
        .consequence("The secret half of the object-store credential. It is read only when OBJECT_STORE is s3, and a wrong value fails every attachment write with an auth error.")
        .rationale("No default, because the bucket's owner issues this half of the pair.")
        .needs(&["OBJECT_STORE"]),

        // email
        param(
            "EMAIL_TRANSPORT",
            "email",
            String,
            Some("none"),
            false,
            "Outbound transport: none, smtp, postmark or ses.",
        )
        .consequence("Which provider sends outbound mail. none means invites, digests and notifications are written and never delivered, which is deliberate on a box with no mail path.")
        .rationale("none is the default so a fresh install cannot send mail as somebody else's domain before anyone configured a sender."),
        param(
            "EMAIL_FROM_ADDRESS",
            "email",
            String,
            Some("support@marula.local"),
            false,
            "From address on Marula-generated mail.",
        )
        .consequence("The address recipients see. A domain the transport has not verified is rejected or lands in spam, and that failure appears per message, not at boot.")
        .rationale("support@marula.local is a placeholder that cannot be mistaken for a real sender, so it is caught in testing rather than in a customer's inbox."),
        param(
            "EMAIL_FROM_NAME",
            "email",
            String,
            None,
            false,
            "From display name.",
        )
        .consequence("The display name beside the From address. It changes nothing a provider checks, only what the recipient reads.")
        .rationale("No default. The name is a deployment's own identity, and a shipped one would read as somebody else's brand.")
        .needs(&["EMAIL_FROM_ADDRESS"]),
        param(
            "POSTMARK_SERVER_TOKEN",
            "email",
            Secret,
            None,
            true,
            "Postmark server token.",
        )
        .consequence("The Postmark server credential. It only matters when EMAIL_TRANSPORT is postmark, and it is ignored otherwise.")
        .rationale("No default, because Postmark issues one per account.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "SMTP_HOST",
            "email",
            String,
            None,
            false,
            "SMTP host.",
        )
        .consequence("The SMTP server host. An empty value with EMAIL_TRANSPORT set to smtp means the first send fails, after the message is already queued.")
        .rationale("No default, because no host is right for more than one deployment.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "SMTP_PORT",
            "email",
            Integer,
            Some("25"),
            false,
            "SMTP port.",
        )
        .consequence("The SMTP port. 25 is plain and blocked outbound by most providers, which is why 587 with STARTTLS is the usual production answer.")
        .rationale("25 is the SMTP default and the right answer for a local relay. A public sender needs 587.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "SMTP_USERNAME",
            "email",
            String,
            None,
            false,
            "SMTP username.",
        )
        .consequence("The SMTP account name. A server that requires authentication refuses the send, and the message stays queued with an error in the delivery log.")
        .rationale("No default. An anonymous relay needs none, and an authenticated one cannot be guessed.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "SMTP_PASSWORD",
            "email",
            Secret,
            None,
            true,
            "SMTP password.",
        )
        .consequence("The SMTP account password. An authentication failure leaves each outbound message queued with an error in the delivery log rather than losing it.")
        .rationale("No default, because the password belongs to the account that authenticates.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "AWS_SES_ACCESS_KEY_ID",
            "email",
            Secret,
            None,
            true,
            "SES access key id.",
        )
        .consequence("The SES access key. It needs ses:SendRawEmail and nothing else.")
        .rationale("No default, because SES credentials belong to the deployment's own AWS account.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "AWS_SES_SECRET_ACCESS_KEY",
            "email",
            Secret,
            None,
            true,
            "SES secret access key.",
        )
        .consequence("The secret half of the SES credential. It is read only when EMAIL_TRANSPORT is ses, and a wrong value fails every send at the provider.")
        .rationale("No default, because AWS issues it per account.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "AWS_SES_REGION",
            "email",
            String,
            Some("eu-west-1"),
            false,
            "SES region. Part of the endpoint, not a preference.",
        )
        .consequence("The SES region. It is part of the endpoint, so a wrong value sends the request to a region where the sending identity is not verified.")
        .rationale("eu-west-1 is a common EU sending region. A deployment in another region changes it.")
        .needs(&["EMAIL_TRANSPORT"]),

        // auth
        param(
            "WEBAUTHN_RP_ID",
            "auth",
            String,
            Some("localhost"),
            false,
            "WebAuthn relying-party id (effective domain of the origin).",
        )
        .consequence("The domain passkeys are bound to. Changing it orphans every enrolled passkey, and users fall back to a password or a reset.")
        .rationale("localhost is the only domain a dev browser will register a passkey for without TLS."),
        param(
            "WEBAUTHN_RP_ORIGIN",
            "auth",
            Url,
            Some("http://localhost:3000"),
            false,
            "WebAuthn relying-party origin.",
        )
        .consequence("The exact origin a passkey assertion must come from. A mismatch is refused by the browser before Marula ever sees the assertion.")
        .rationale("localhost:3000 is the dev web origin, and the pair with the relying-party id is what makes passkeys work locally."),
        param(
            "ACCESS_TTL_SECS",
            "auth",
            Integer,
            Some("900"),
            false,
            "Access-token TTL.",
        )
        .consequence("How long an access token keeps working. Short values limit a stolen token's life and cost a refresh round trip. Long values save those round trips and leave a stolen token usable for longer.")
        .rationale("900 s is short enough that a leaked token ages out within a coffee break and long enough that a working session refreshes every 15 minutes."),
        param(
            "REFRESH_TTL_SECS",
            "auth",
            Integer,
            Some("2592000"),
            false,
            "Refresh-token TTL.",
        )
        .consequence("How long a session survives without signing in again. Lowering it signs people out mid-shift. Raising it extends what a stolen refresh token is worth.")
        .rationale("30 days is the shipped session length, chosen so a technician is not asked for a password between calls."),
        param(
            "MARULA_AUTH_HASH_MAX_CONCURRENCY",
            "auth",
            Integer,
            Some("0"),
            false,
            "Concurrent argon2id hashes this process will run. 0 resolves at boot to two slots per available core.",
        )
        .consequence("How many password hashes run at once. Each in-flight slot holds about nineteen megabytes, so a number past the core count widens the tail instead of adding throughput.")
        .rationale("0 means two slots per core, which is the measured bound. Every extra slot is memory that unauthenticated login traffic can claim."),
        param(
            "MARULA_AUTH_HASH_QUEUE_WAIT_MS",
            "auth",
            Integer,
            Some("250"),
            false,
            "How long a password request waits for a hash slot before the 503 with Retry-After.",
        )
        .consequence("How long a login waits for a hash slot before it is refused with a retry hint. Waiting longer than one hash only delays the answer a caller needs.")
        .rationale("250 ms is roughly one argon2 verify, so a caller that has already waited that long is better served by retrying than by queueing further."),

        // cadence
        param(
            "EMAIL_POLL_SECS",
            "cadence",
            Integer,
            Some("2"),
            false,
            "Outbound email idle poll interval. See docs/worker-cadence.md.",
        )
        .consequence("How often the outbound email loop looks for work. At two seconds a queued message leaves almost immediately. Each tick is a query, so a fleet adds up.")
        .rationale("2 s is the shipped idle cadence, and the reason a ticket notification arrives while the user is still looking at the screen."),
        param(
            "QUEUE_IDLE_MS",
            "cadence",
            Integer,
            Some("500"),
            false,
            "General job-queue idle sleep.",
        )
        .consequence("How long the job queue sleeps when it finds nothing. A lower value starts a job sooner and asks the database more often while the system is idle.")
        .rationale("500 ms is short enough that the demo feels live and long enough that an idle box is not querying twice a second per loop."),
        param(
            "EVAL_INTERVAL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Shared interval for the SLA, project billing, rotation, dedupe sweeps.",
        )
        .consequence("The interval all four sweeps share: SLA, billing, rotation and dedupe. Raising it delays all four together, because there is no per-sweep override.")
        .rationale("60 s is one minute of staleness across four sweeps, which is below what any of them promises a user."),
        param(
            "TIME_AUDIT_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Missing-time nudge sweep interval.",
        )
        .consequence("How often missing-time nudges are computed. A long interval means a reminder about yesterday's gap arrives in the afternoon.")
        .rationale("60 s keeps the nudge close to the gap it is about without a per-user query more often than that."),
        param(
            "DELIVERY_POLL_SECS",
            "cadence",
            Integer,
            Some("60"),
            false,
            "Scheduled report delivery sweep interval.",
        )
        .consequence("How often scheduled reports are checked for due time. It decides how late a nine o'clock report can be.")
        .rationale("60 s puts a report within a minute of its slot. A recipient expects no less."),
        param(
            "USAGE_REFRESH_SECS",
            "cadence",
            Integer,
            Some("300"),
            false,
            "Org usage refresh sweep interval.",
        )
        .consequence("How often org usage counters are recomputed. Longer windows show a stale count on the billing screen and cost fewer aggregate queries.")
        .rationale("300 s because usage is read by humans checking a number, not by a gate that has to be exact at the second."),
        param(
            "DIGEST_POLL_SECS",
            "cadence",
            Integer,
            Some("900"),
            false,
            "Daily digest sweep interval.",
        )
        .consequence("How often the daily digest sweep runs. It decides when the digest leaves, not whether it is sent.")
        .rationale("3600 s: a daily digest does not need minute precision, and an hourly look costs one query."),
        param(
            "RETENTION_SWEEP_SECS",
            "cadence",
            Integer,
            Some("21600"),
            false,
            "Workflow run-retention sweep interval.",
        )
        .consequence("How often workflow run retention is enforced. The window decides what is kept. This interval only decides how long a row past the window survives.")
        .rationale("21600 s is six hours, which is the shipped tolerance between a row ageing out and being deleted."),

        // lifecycle (retention matrix + object integrity; docs/reference/data-lifecycle.md)
        param(
            "MARULA_RETENTION_PROFILE",
            "lifecycle",
            String,
            Some("standard"),
            false,
            "Retention matrix profile: compact, standard or extended. Moves all nine windows at once.",
        )
        .consequence("Which retention matrix applies. One value moves all nine windows, so a quiet switch to compact deletes data an audit expected to keep.")
        .rationale("standard is the shipped matrix: it keeps the windows the product promises in its own documentation."),
        param(
            "MARULA_RETENTION_SWEEP_SECS",
            "lifecycle",
            Integer,
            Some("3600"),
            false,
            "Retention pass interval: partition upkeep plus the batched deletes.",
        )
        .consequence("How often the retention pass runs: partition upkeep plus the batched deletes. A short interval keeps each burst of deletes small.")
        .rationale("3600 s is hourly: frequent enough that deletes stay small, rare enough not to churn partitions."),
        param(
            "MARULA_RETENTION_BATCH",
            "lifecycle",
            Integer,
            Some("1000"),
            false,
            "Rows per retention delete statement.",
        )
        .consequence("Rows per delete statement. A large batch finishes sooner and holds locks longer, which shows up as a stall in whatever else is writing.")
        .rationale("1000 rows is a statement that completes in well under a second at the measured table sizes."),
        param(
            "MARULA_OBJECT_GC",
            "lifecycle",
            String,
            Some("off"),
            false,
            "Object-integrity pass: off, dry_run or on. `on` deletes orphaned bytes.",
        )
        .consequence("Whether orphaned object bytes are removed. on deletes them, dry_run only reports, and off lets the storage grow unreported.")
        .rationale("off is the default because a deletion pass over somebody's attachments should be switched on deliberately."),
        param(
            "MARULA_OBJECT_GC_GRACE_HOURS",
            "lifecycle",
            Integer,
            Some("24"),
            false,
            "How old an unreferenced object must be before the pass may delete it.",
        )
        .consequence("How long an unreferenced object is left alone before the pass may delete it. Too short deletes an upload whose row has not committed yet.")
        .rationale("24 h is far longer than any upload transaction and short enough that a real orphan is reclaimed the next day.")
        .needs(&["MARULA_OBJECT_GC"]),
        param(
            "MARULA_OBJECT_GC_SECS",
            "lifecycle",
            Integer,
            Some("3600"),
            false,
            "Object-integrity pass interval.",
        )
        .consequence("How often the object-integrity pass runs. Each pass lists storage and compares it with the reference tables, so it is not free.")
        .rationale("3600 s: an orphan costs disk, not correctness, so an hourly pass is plenty.")
        .needs(&["MARULA_OBJECT_GC"]),

        // reconciliation (index backfill, dead-entry replay, parked-run reap,
        // consumer gaps; docs/worker-cadence.md)
        param(
            "MARULA_RECONCILE_SECS",
            "lifecycle",
            Integer,
            Some("900"),
            false,
            "Reconciliation pass interval: the four repairs nothing re-attempts on its own.",
        )
        .consequence("How often the four repairs run: index backfill, dead-entry replay, parked-run reap and consumer gaps. Raising it leaves each fault in place longer.")
        .rationale("900 s is the shipped repair cadence: fast enough that a stuck run is noticed within a quarter hour."),
        param(
            "MARULA_RECONCILE_BATCH",
            "lifecycle",
            Integer,
            Some("200"),
            false,
            "Rows each reconciliation pass may repair.",
        )
        .consequence("Rows one reconciliation pass may repair. It bounds the work a pass does, so a large backlog clears over several passes instead of one long transaction.")
        .rationale("200 rows keeps a pass short and lets the next repair wait behind it rather than behind a huge batch."),
        param(
            "MARULA_RECONCILE_MAX_REPLAYS",
            "lifecycle",
            Integer,
            Some("3"),
            false,
            "Deaths allowed per (group, event) before a dead entry stops being replayed.",
        )
        .consequence("How many deaths one event group may have before replay stops. Zero parks a flapping event immediately. Three lets a transient fault recover.")
        .rationale("3 attempts covers a restart or a network blip and still stops a poison message from looping forever."),
        param(
            "MARULA_RECONCILE_WAKE_GRACE_SECS",
            "lifecycle",
            Integer,
            None,
            false,
            "How far past its deadline a parked run's wake job must be dead before the run is abandoned. Unset derives it from the queue retry policy.",
        )
        .consequence("How far past its deadline a parked run wake job must be dead before the run is abandoned. Unset derives it from the queue retry policy.")
        .rationale("Unset on purpose: the grace should follow the retry policy, and a fixed number here would stop tracking it."),

        // observability
        param(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "observability",
            Url,
            None,
            false,
            "OTLP endpoint. Unset runs the none mode (no export).",
        )
        .consequence("Where traces and metrics are exported. Unset runs the none mode: no export, no error, and no traces to look at when something goes wrong.")
        .rationale("Unset by default so an install with no collector does not spend its first weeks retrying an endpoint that does not exist."),
        param(
            "GLITCHTIP_DSN_CORE_API",
            "observability",
            Secret,
            None,
            false,
            "GlitchTip DSN presence for the self-host pane.",
        )
        .consequence("The API's error-reporting DSN. Setting it makes error reporting read as configured in the self-host pane. Left unset, errors stay in the log only.")
        .rationale("No default, because an error sink belongs to the deployment that runs it."),

        // --- topology (added by Wave 52 lane R1: the descriptor omitted them) ---
        param(
            "RUN_MIGRATIONS",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Run startup migrations as this process. Dev only: the compose migrate one-shot owns them.",
        )
        .consequence("Whether this process applies migrations at boot. The shipped stack gives that job to the migrate one-shot, so turning it on gives two processes the same task.")
        .rationale("false because the compose migrate service already owns the migration, and two owners is how a failed migration gets applied twice."),
        param(
            "MARULA_DEV_TOOLS",
            "topology",
            Boolean,
            Some("false"),
            false,
            "Enable the dev-only fallbacks for JWT_SECRET and MARULA_SECRET_KEY. Never set in production.",
        )
        .consequence("Enables dev fallbacks for the two signing secrets. On a reachable deployment it turns a boot refusal into a running server with a guessable key.")
        .rationale("false, and never true outside a laptop. The fallback exists so a demo needs no setup, not so a server can skip its keys."),
        param(
            "TRUSTED_PROXIES",
            "topology",
            String,
            None,
            false,
            "Comma-separated proxy CIDRs. Replaces the loopback plus RFC1918 default set.",
        )
        .consequence("The proxy CIDRs whose X-Forwarded-For header is believed. A set wider than the real fleet lets a client write its own address and step around per-IP limits.")
        .rationale("Unset keeps the built-in loopback-plus-private set. A proxy on the same host or network produces exactly those addresses."),
        param(
            "CF_CONNECTING_IP",
            "topology",
            String,
            Some("auto"),
            false,
            "Whether the client-IP resolver believes the CF-Connecting-IP header: auto, true or false.",
        )
        .consequence("Whether the client-IP resolver believes CF-Connecting-IP. true behind Cloudflare gets the real client, and true without it lets any caller claim one.")
        .rationale("auto trusts the header only when the peer address sits inside the Cloudflare ranges, so the safe answer needs no knowledge of the deployment's topology."),
        param(
            "CF_IP_RANGES",
            "topology",
            String,
            None,
            false,
            "Extra Cloudflare CIDRs. Extends the versioned baseline, never replaces it.",
        )
        .consequence("Extra Cloudflare ranges accepted as proxies. It only adds to the versioned baseline, so it cannot open the resolver to the internet.")
        .rationale("Unset means the shipped baseline, which is refreshed with the release."),
        param(
            "WORKER_ROLES",
            "topology",
            String,
            Some("all"),
            false,
            "Loop groups this worker runs: all, realtime, housekeeping, or both comma-separated.",
        )
        .consequence("Which loop groups this worker runs. Splitting them lets one process take the live path while another grinds through housekeeping.")
        .rationale("all means one worker runs everything, which is the shipped compose shape and the simplest thing to watch."),
        param(
            "CORE_API_URL",
            "topology",
            Url,
            Some("http://localhost:8080"),
            false,
            "core-api base URL for the email and missing-time loops.",
        )
        .consequence("The API base the email and missing-time loops call. A wrong value fails those calls while the rest of the worker keeps running.")
        .rationale("localhost:8080 is the dev default. Inside compose the worker reaches the API by service name, and the profile sets that."),
        param(
            "INTERNAL_API_URL",
            "topology",
            Url,
            Some("http://127.0.0.1:8080"),
            false,
            "core-api base URL for the machine effect calls.",
        )
        .consequence("The API base the machine effect calls use. It has to be reachable from the worker's network namespace, which localhost is not in compose.")
        .rationale("127.0.0.1:8080 is the dev default because a worker on the same host is the common laptop case."),
        param(
            "WORKER_INTERNAL_URL",
            "topology",
            Url,
            None,
            false,
            "Worker health URL the self-host pane probes. Unset shows the pane as not configured.",
        )
        .consequence("The worker health URL the self-host pane probes. Unset shows the pane as not configured rather than failing a process it never ran.")
        .rationale("Unset by default so a deployment without a worker pane does not show a red light for a process it never ran."),

        // --- database and cache ---
        param(
            "MARULA_REF_CACHE_TTL_SECS",
            "database",
            Integer,
            Some("30"),
            false,
            "Backstop TTL for the reference cache. An invalidation event normally drops the entry first.",
        )
        .consequence("The backstop lifetime of a cached reference row. Normal invalidation drops the entry first, so this only bounds how stale a missed invalidation can stay.")
        .rationale("30 s is short enough that a missed invalidation is a blink and long enough that the cache still saves the reads."),
        param(
            "MARULA_REF_CACHE_MAX_ENTRIES",
            "database",
            Integer,
            Some("4096"),
            false,
            "Entry ceiling per cache namespace, least-recently-used eviction.",
        )
        .consequence("The ceiling per cache namespace before least-recently-used eviction. Too small and the cache thrashes. Too large and it holds memory no report needs.")
        .rationale("4096 entries is a few megabytes for the tables that use it, well inside a container's memory budget."),
        param(
            "MARULA_MAX_UPLOAD_MB",
            "database",
            Integer,
            Some("25"),
            false,
            "Largest accepted attachment, in mebibytes.",
        )
        .consequence("The largest attachment accepted. A lower value refuses a file a user expects to attach. A higher one lets a single request hold that much memory and disk.")
        .rationale("25 MB covers a screenshot, a log bundle or a PDF export without letting a request become a bulk transfer."),

        // cache
        param(
            "MARULA_EVENT_STREAM_MAXLEN",
            "cache",
            Integer,
            Some("50000"),
            false,
            "Entries kept in the event stream before the oldest are trimmed.",
        )
        .consequence("Entries kept in the event stream. Trimming earlier loses replay for a client that reconnects late. Keeping more costs Redis memory.")
        .rationale("50000 entries is several hours of a busy instance's events, which is longer than any tab stays suspended."),
        param(
            "MARULA_SSE_MAX_CONNECTIONS",
            "cache",
            Integer,
            Some("8"),
            false,
            "Live-push connections allowed per user.",
        )
        .consequence("Live-push connections one user may hold. Each connection is a task and a slot, so the ceiling is what stops one browser with many tabs from consuming a worker.")
        .rationale("8 covers the tabs a person actually leaves open and bounds what one account can pin."),
        param(
            "MARULA_SSE_REPLAY_BUFFER",
            "cache",
            Integer,
            Some("256"),
            false,
            "Live-push events buffered for replay to a reconnecting client.",
        )
        .consequence("Events buffered for a reconnecting client. A client away longer than this gets a gap, and the UI answers it with a refresh.")
        .rationale("256 events covers a reconnect after a sleep or a tunnel, which is the case replay exists for."),
        param(
            "MARULA_PRESENCE_TTL_SECS",
            "cache",
            Integer,
            None,
            false,
            "Presence lease length. Unset keeps the built-in default.",
        )
        .consequence("How long a presence lease lasts. Too short flickers a colleague offline. Too long shows them present after the tab is gone.")
        .rationale("Unset keeps the built-in lease, which the presence loop renews while the tab is alive."),
        param(
            "OUTBOX_RETENTION_DAYS",
            "cache",
            Integer,
            Some("7"),
            false,
            "Days a published outbox row is kept before the sweep removes it.",
        )
        .consequence("Days a published outbox row is kept. The row is the record of what was emitted, so deleting it sooner loses that history.")
        .rationale("7 days is long enough to investigate a missed webhook from last week and short enough that the table stays small."),
        param(
            "OUTBOX_SWEEP_SECS",
            "cache",
            Integer,
            Some("21600"),
            false,
            "How often the outbox retention window is enforced.",
        )
        .consequence("How often the outbox retention window is enforced. Each pass deletes what has aged out, and the interval only decides how often that happens.")
        .rationale("21600 s is every six hours, which keeps the delete off the hot path entirely."),

        // --- secrets and keys ---
        param(
            "MARULA_PLUGIN_SIGNING_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Single trusted plugin-signing key (key_id=hex). Superseded by MARULA_PLUGIN_SIGNING_KEYS.",
        )
        .consequence("One trusted plugin key in the older singular form. The plural lever supersedes it, so set it only to keep an existing install working.")
        .rationale("No default, because a trust root cannot ship with the product."),
        param(
            "MARULA_JWT_EDDSA_PRIVATE_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Inline Ed25519 private key PEM for token signing. Set this or the _FILE form, not both.",
        )
        .consequence("The Ed25519 key that signs tokens instead of the HMAC secret. Tokens signed by it cannot be verified by a peer that only has the secret, so every verifier needs the public half.")
        .rationale("No default, because the signing key belongs to the deployment. The inline form exists for a platform that cannot mount a file.")
        .conflicts(&["MARULA_JWT_EDDSA_PRIVATE_KEY_FILE"]),
        param(
            "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
            "secrets",
            String,
            None,
            false,
            "Path to the Ed25519 private key PEM. Set this or the inline form, not both.",
        )
        .consequence("The path to the Ed25519 private key PEM. The file form keeps the key out of the process environment and out of a crash dump.")
        .rationale("No default, because only the deployment knows where it put the file.")
        .conflicts(&["MARULA_JWT_EDDSA_PRIVATE_KEY"]),
        param(
            "MARULA_JWT_EDDSA_PUBLIC_KEY",
            "secrets",
            String,
            None,
            false,
            "Inline Ed25519 public key PEM. Required whenever a private key is configured.",
        )
        .consequence("The public half verifiers use. A key pair that disagrees signs tokens nothing accepts, and the refusal appears at the first request rather than at boot.")
        .rationale("No default, because it has to match the private key generated for this deployment.")
        .conflicts(&["MARULA_JWT_EDDSA_PUBLIC_KEY_FILE"]),
        param(
            "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
            "secrets",
            String,
            None,
            false,
            "Path to the Ed25519 public key PEM.",
        )
        .consequence("The path to the Ed25519 public key PEM, for deployments that mount keys as files.")
        .rationale("No default, because only the deployment knows the path.")
        .conflicts(&["MARULA_JWT_EDDSA_PUBLIC_KEY"]),
        param(
            "MARULA_JWT_KID",
            "secrets",
            String,
            None,
            false,
            "Key id published in the token header so verifiers pick the right key.",
        )
        .consequence("The key id published in the token header so a verifier picks the right key. Two deployments sharing one id make a rotation ambiguous.")
        .rationale("m1 is a single-key deployment's identifier. A rotation gives the new key a new id."),
        param(
            "MARULA_STRIPE_SECRET_KEY",
            "secrets",
            Secret,
            None,
            true,
            "Stripe API key for the billing provider adapter.",
        )
        .consequence("The Stripe API key. With it unset the billing adapter stays in its none mode and no charge is attempted, which is the honest state of an install that does not bill.")
        .rationale("No default, because a live payment credential cannot ship in an image."),
        param(
            "MARULA_STRIPE_WEBHOOK_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Stripe webhook signing secret.",
        )
        .consequence("Verifies Stripe webhooks. Unset means settlement events are refused rather than trusted unverified.")
        .rationale("No default, because Stripe issues it per endpoint.")
        .needs(&["MARULA_STRIPE_SECRET_KEY"]),
        param(
            "GOOGLE_CALENDAR_CLIENT_ID",
            "secrets",
            String,
            None,
            false,
            "Delegated Google Calendar OAuth client id.",
        )
        .consequence("The delegated Google Calendar OAuth client id. Without it and its secret the provider cannot be connected, and the calendar sync stays unavailable.")
        .rationale("No default: an OAuth client belongs to the deployment's own Google project."),
        param(
            "GOOGLE_CALENDAR_CLIENT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Delegated Google Calendar OAuth client secret.",
        )
        .consequence("The Google Calendar OAuth client secret. It is exchanged for a refresh token per connected technician.")
        .rationale("No default, because it is issued per OAuth client.")
        .needs(&["GOOGLE_CALENDAR_CLIENT_ID"]),
        param(
            "MICROSOFT_GRAPH_CLIENT_ID",
            "secrets",
            String,
            None,
            false,
            "Delegated Microsoft Graph OAuth client id.",
        )
        .consequence("The delegated Microsoft Graph OAuth client id. Without both halves the Graph calendar and mail providers stay unavailable.")
        .rationale("No default, because the app registration belongs to the deployment."),
        param(
            "MICROSOFT_GRAPH_CLIENT_SECRET",
            "secrets",
            Secret,
            None,
            true,
            "Delegated Microsoft Graph OAuth client secret.",
        )
        .consequence("The Microsoft Graph OAuth client secret. Without it the Graph calendar and mail providers cannot be connected, so the sync stays unavailable.")
        .rationale("No default, because it is issued per app registration.")
        .needs(&["MICROSOFT_GRAPH_CLIENT_ID"]),

        // --- email (remaining levers) ---
        param(
            "AWS_SES_ENDPOINT",
            "email",
            Url,
            None,
            false,
            "SES endpoint override. Unset derives it from the region.",
        )
        .consequence("Overrides the SES endpoint. It is useful against a local mail sink. In production it has to be unset so the region decides.")
        .rationale("Unset means the regional endpoint, and production wants nothing else.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "POSTMARK_BASE_URL",
            "email",
            Url,
            Some("https://api.postmarkapp.com"),
            false,
            "Postmark API base URL.",
        )
        .consequence("The Postmark API base. Unset uses the public API, and a proxy or a regional endpoint goes here.")
        .rationale("api.postmarkapp.com is Postmark's own endpoint and needs no configuration.")
        .needs(&["EMAIL_TRANSPORT"]),
        param(
            "EMAIL_SCAN",
            "email",
            String,
            Some("none"),
            false,
            "Inbound attachment scanning: none or clamd.",
        )
        .consequence("Whether inbound attachments are scanned before storage. none accepts bytes without inspection, which is a decision a deployment should make deliberately.")
        .rationale("none is the default because a scanner is a separate service, and a scanning default with no clamd would refuse every inbound attachment."),
        param(
            "CLAMD_ADDR",
            "email",
            String,
            Some("127.0.0.1:3310"),
            false,
            "clamd address when EMAIL_SCAN=clamd.",
        )
        .consequence("Where clamd answers. Unreachable with scanning on means inbound attachments are refused rather than accepted unscanned.")
        .rationale("127.0.0.1:3310 is clamd's own default port, and the compose profile runs it there.")
        .needs(&["EMAIL_SCAN"]),
        param(
            "EMAIL_CLAIM_BATCH",
            "email",
            Integer,
            Some("25"),
            false,
            "Outbound email rows claimed per cycle.",
        )
        .consequence("How many outbound rows one claim takes. Larger batches raise throughput and widen the window a crashed claimant can hold before the lease expires.")
        .rationale("25 is one page of mail: enough to keep a sender busy, small enough that a crash strands few rows."),
        param(
            "EMAIL_LEASE_SECS",
            "email",
            Integer,
            Some("300"),
            false,
            "Lease seconds asked of the claim endpoint.",
        )
        .consequence("How long a claim is held before another worker may take the row. Too short and two workers send the same mail.")
        .rationale("300 s is longer than a send plus a provider timeout, so a live claimant is never overtaken."),
        param(
            "DELIVERY_CLAIM_BATCH",
            "email",
            Integer,
            Some("20"),
            false,
            "Scheduled-delivery rows claimed per sweep.",
        )
        .consequence("Scheduled-report rows claimed per sweep. The same trade as the email batch: throughput against the blast radius of one crashed claimant.")
        .rationale("20 keeps a delivery sweep short. Reports are heavier than mail, so the batch is smaller."),

        // --- auth (remaining levers) ---
        param(
            "MFA_PENDING_TTL_SECS",
            "auth",
            Integer,
            Some("300"),
            false,
            "How long an MFA challenge stays valid.",
        )
        .consequence("How long a half-finished MFA challenge stays open. Longer lets an abandoned login be completed later from another device.")
        .rationale("300 s covers typing a six-digit code with a mistyped first attempt and nothing more."),
        param(
            "MARULA_DECISION_BOT_USER_ID",
            "auth",
            String,
            None,
            false,
            "Real member id decision runs are attributed to. Unset disables the decision subsystem.",
        )
        .consequence("The real member id that decision runs are attributed to. Unset disables the decision subsystem, so every rule that asks it stops producing a suggestion.")
        .rationale("No default: attribution is to a person in the operator's own directory, and no shipped id could be right."),

        // --- cadence (remaining levers) ---
        param(
            "QUEUE_BATCH",
            "cadence",
            Integer,
            Some("10"),
            false,
            "Jobs claimed per queue tick.",
        )
        .consequence("Jobs claimed per queue tick. Larger batches keep workers busier and widen what one crashed claimant strands until its lease expires.")
        .rationale("10 jobs is a short transaction: enough to keep a worker busy, small enough that a crash strands little."),
        param(
            "QUEUE_STALE_SECS",
            "cadence",
            Integer,
            Some("300"),
            false,
            "Lease age after which a crashed claimant's job is re-armed.",
        )
        .consequence("Lease age after which a crashed claimant's job is re-armed. Too short steals work from a live worker. Too long leaves a job stuck for that whole window.")
        .rationale("300 s is longer than the slowest measured job and shorter than a person's patience with a stuck run."),
        param(
            "RELAY_BATCH",
            "cadence",
            Integer,
            Some("100"),
            false,
            "Outbox rows relayed per batch.",
        )
        .consequence("Outbox rows relayed per batch. A larger batch drains a backlog faster and holds a longer publish loop against Redis.")
        .rationale("100 rows is a single pipeline to Redis, which is cheap and bounded."),
        param(
            "RELAY_POLL_MS",
            "cadence",
            Integer,
            Some("5000"),
            false,
            "Backstop poll interval for the outbox relay.",
        )
        .consequence("The backstop poll interval for the outbox relay. With wakeups working it rarely delivers anything, but it is what covers a missed wakeup.")
        .rationale("5000 ms bounds how long an event waits if the notification channel is down, at one query every five seconds."),
        param(
            "MARULA_RELAY_WAKEUP",
            "cadence",
            Boolean,
            Some("true"),
            false,
            "Listen on the outbox notification channel. Off restores pure polling.",
        )
        .consequence("Whether the relay listens on the outbox notification channel. Off restores pure polling, so every event can wait up to the poll interval.")
        .rationale("true because the wakeup is what makes live push feel instant. The poll stays as the backstop either way."),

        // --- observability (remaining levers) ---
        param(
            "GLITCHTIP_DSN_WORKER",
            "observability",
            Secret,
            None,
            true,
            "GlitchTip DSN for the worker. Presence is shown in the self-host pane.",
        )
        .consequence("The worker's error-reporting DSN. It is separate from the API's so the two process types can report to different projects.")
        .rationale("No default, because the worker reports where its operator pointed it or nowhere."),
        param(
            "MARULA_GLITCHTIP_URL",
            "observability",
            Url,
            None,
            false,
            "GlitchTip console URL for the operator links.",
        )
        .consequence("The GlitchTip console link the self-host pane offers. It is a link an operator follows, not something Marula calls.")
        .rationale("Unset hides the link rather than pointing at a console that does not exist."),
        param(
            "MARULA_GRAFANA_URL",
            "observability",
            Url,
            None,
            false,
            "Grafana URL for the operator links.",
        )
        .consequence("The Grafana link the self-host pane offers. Nothing depends on it. It saves an operator from remembering the address.")
        .rationale("Unset leaves the link out, because there is no address to open until an operator runs Grafana."),
        param(
            "MARULA_LOKI_URL",
            "observability",
            Url,
            None,
            false,
            "Loki URL for the operator links.",
        )
        .consequence("The Loki link the self-host pane offers for log queries.")
        .rationale("Unset leaves the link out, because there is no address to open until an operator runs Loki."),
        param(
            "MARULA_OTEL_METRICS_URL",
            "observability",
            Url,
            None,
            false,
            "Metrics backend URL for the operator links.",
        )
        .consequence("The metrics-backend link the self-host pane offers. It is unrelated to the OTLP endpoint, which is where the data is actually sent.")
        .rationale("Unset leaves the link out, because there is no address to open until a metrics backend exists."),
        param(
            "MARULA_TEMPO_URL",
            "observability",
            Url,
            None,
            false,
            "Tempo URL for the operator links.",
        )
        .consequence("The Tempo link the self-host pane offers for trace lookup.")
        .rationale("Unset leaves the link out, because there is no address to open until an operator runs Tempo."),
        param(
            "MARULA_UPTIME_KUMA_URL",
            "observability",
            Url,
            None,
            false,
            "Uptime Kuma URL for the operator links.",
        )
        .consequence("The Uptime Kuma link the self-host pane offers. It points at whatever monitor the operator runs, and Marula does not read it.")
        .rationale("Unset leaves the link out, because there is no address to open until an operator runs Uptime Kuma."),

        // --- routing ---
        param(
            "ROUTING_PROVIDER",
            "routing",
            String,
            Some("none"),
            false,
            "Distance provider: none, osrm, ors, contract or mapbox.",
        )
        .consequence("Which distance provider answers travel time. none means the dispatch board shows no estimate instead of an invented one.")
        .rationale("none is the default because a fabricated drive time is worse than a blank one on a dispatch board."),
        param(
            "ROUTING_OSRM_URL",
            "routing",
            Url,
            None,
            false,
            "OSRM base URL.",
        )
        .consequence("The OSRM base URL used when the provider is osrm. An unreachable server degrades travel time, not dispatch itself.")
        .rationale("No default: OSRM is self-hosted, so only the deployment knows where it answers.")
        .needs(&["ROUTING_PROVIDER"]),
        param(
            "ROUTING_ORS_URL",
            "routing",
            Url,
            None,
            false,
            "OpenRouteService base URL.",
        )
        .consequence("The OpenRouteService base URL. Unset uses the public service, which is the endpoint the key was issued for.")
        .rationale("No default, because the public endpoint and a proxy are both deployment choices.")
        .needs(&["ROUTING_PROVIDER"]),
        param(
            "ROUTING_ORS_API_KEY",
            "routing",
            Secret,
            None,
            true,
            "OpenRouteService API key.",
        )
        .consequence("The OpenRouteService key. Without it the provider refuses every request, and travel times disappear rather than approximating.")
        .rationale("No default, because OpenRouteService issues one per account.")
        .needs(&["ROUTING_ORS_URL"]),
        param(
            "ROUTING_CONTRACT_URL",
            "routing",
            Url,
            None,
            false,
            "Contract routing service base URL.",
        )
        .consequence("The contract routing service base URL, for a deployment that has its own travel-time agreement.")
        .rationale("No default, because the service belongs to the customer.")
        .needs(&["ROUTING_PROVIDER"]),
        param(
            "ROUTING_CONTRACT_ID",
            "routing",
            String,
            None,
            false,
            "Contract routing service account id.",
        )
        .consequence("The account id the contract routing service expects. It is sent with every lookup and rejected if it does not match.")
        .rationale("No default, because the routing service issues it.")
        .needs(&["ROUTING_CONTRACT_URL"]),
        param(
            "ROUTING_CONTRACT_TOKEN",
            "routing",
            Secret,
            None,
            true,
            "Contract routing service token.",
        )
        .consequence("The token for the contract routing service. A wrong one fails lookups while the rest of the board keeps working.")
        .rationale("No default, because the routing service issues it per account.")
        .needs(&["ROUTING_CONTRACT_URL"]),
        param(
            "MAPBOX_ACCESS_TOKEN",
            "routing",
            Secret,
            None,
            true,
            "Mapbox access token.",
        )
        .consequence("The Mapbox token used when the provider is mapbox. It is billed per request, so a leaked token costs money rather than data.")
        .rationale("No default, because Mapbox issues one per account and bills it per request.")
        .needs(&["ROUTING_PROVIDER"]),
        param(
            "MAPBOX_PROFILE",
            "routing",
            String,
            None,
            false,
            "Mapbox routing profile (driving, driving-traffic, walking).",
        )
        .consequence("The Mapbox routing profile: driving, driving-traffic or walking. Traffic-aware routing costs more per request and answers a different question.")
        .rationale("No default: the profile is a scheduling policy, not a technical default.")
        .needs(&["MAPBOX_ACCESS_TOKEN"]),

        // --- rate limits and budgets ---
        param(
            "MARULA_MIGRATION_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Migration requests allowed per principal per minute.",
        )
        .consequence("Requests a migration key may make per minute. A lower value throttles a bulk push into a crawl. A higher one lets a single key crowd the API.")
        .rationale("600 is ten requests a second, which a real import sustains and a runaway client does not exceed unnoticed."),
        param(
            "MARULA_DISPATCH_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Dispatch requests allowed per principal per minute.",
        )
        .consequence("Dispatch requests allowed per key per minute. It bounds what one field-service connector can do to the API.")
        .rationale("600 matches the migration ceiling because both are machine clients doing batched work."),
        param(
            "MARULA_SCHEDULING_LINKS_RATE_LIMIT_PER_MIN",
            "limits",
            Integer,
            Some("60"),
            false,
            "Scheduling-link requests allowed per principal per minute.",
        )
        .consequence("Requests the public scheduling links accept per principal per minute. A lower value protects a page a stranger can reach. A higher one admits a burst of bookings.")
        .rationale("60 is one request a second, which no human booking flow needs and a scraper exceeds at once."),
        param(
            "MARULA_VAULT_WRITE_BUDGET_PER_MIN",
            "limits",
            Integer,
            Some("600"),
            false,
            "Vault writes allowed per org per minute.",
        )
        .consequence("Vault writes allowed per org per minute. It stops a runaway automation from filling the encrypted store.")
        .rationale("600 writes a minute is far above what a person or a workflow does and low enough that a loop is caught within a minute."),
        param(
            "MARULA_VAULT_BULK_ROWS_PER_SEC",
            "limits",
            Integer,
            Some("50"),
            false,
            "Vault bulk-import rows per second.",
        )
        .consequence("Rows per second a vault bulk import may write. Raising it imports faster and holds a longer write burst against the same table.")
        .rationale("50 rows a second finishes a large import in minutes without locking the vault for readers."),

        // --- ai (model endpoints) ---
        param(
            "OPENAI_BASE_URL",
            "ai",
            Url,
            None,
            false,
            "OpenAI-compatible base URL. Unset keeps the built-in default.",
        )
        .consequence("The OpenAI-compatible endpoint for cloud embeddings and completions. Unset keeps the built-in URL, and an internal gateway goes here.")
        .rationale("Unset means the public OpenAI URL, which is the only endpoint that needs no configuration."),
        param(
            "OLLAMA_BASE_URL",
            "ai",
            Url,
            None,
            false,
            "Ollama base URL for the local model option.",
        )
        .consequence("Where the local model runtime answers. Unset uses the default local port, which is wrong the moment Ollama runs in another container.")
        .rationale("Unset keeps the loopback default that a co-located Ollama uses. Compose sets the service name instead."),
        param(
            "MARULA_EMBEDDING_TIER",
            "ai",
            String,
            Some("standard"),
            false,
            "Embedding tier this instance ships: minimal, standard or advanced. Sets the default model and its capacity floor.",
        )
        .consequence("Which model class this instance ships and the capacity floor that comes with it. Raising the tier changes the default model and can refuse boot on a box that ran yesterday.")
        .rationale("standard is the tier R13 measured: EmbeddingGemma 2 text-only at 16 chunks a second and recall 0.863."),
        param(
            "MARULA_EMBEDDING_MODEL",
            "ai",
            String,
            Some("embeddinggemma-2:270m-q8_0-text"),
            false,
            "Embedding model this instance runs, set per instance rather than per org. The tier picks the default.",
        )
        .consequence("The one model every vector in this instance is written with. Changing it writes nothing by itself, and the vectors already stored belong to the old space until a re-index finishes.")
        .rationale("embeddinggemma-2:270m-q8_0-text is the shipped default because it is the measured one on the reference profiles and runs on any CPU."),
        param(
            "MARULA_EMBEDDING_PROVIDER",
            "ai",
            String,
            Some("ollama"),
            false,
            "Where embeddings are computed: ollama for a local model, or openai for an OpenAI-compatible endpoint.",
        )
        .consequence("Which runtime computes embeddings. The local one keeps search text on the box, and a cloud endpoint sends it off, which is a data-handling decision before it is a performance one.")
        .rationale("ollama is the default because the shipped model is local and search works on a box with no internet."),
        param(
            "MARULA_EMBEDDING_API_KEY",
            "ai",
            Secret,
            None,
            true,
            "Instance-wide key for a cloud embedding provider. Checked before an org's own credential.",
        )
        .consequence("The instance key for a cloud provider, used before any org's own credential. Unset with a cloud provider means every org brings its own key or embedding fails.")
        .rationale("No default, because it is a billing credential for one cloud account.")
        .needs(&["MARULA_EMBEDDING_PROVIDER"]),
        param(
            "MARULA_EMBEDDING_CUSTOM_MODELS",
            "ai",
            String,
            None,
            false,
            "Extra catalogue entries as `id|display|dimensions|modality|license|provider|version|tier|cores|ram_mb`, semicolon separated. A shipped id is replaced, not duplicated.",
        )
        .consequence("Extra catalogue entries as data. A record that replaces a shipped id changes the floor and the space tag for everyone, and a malformed record warns instead of refusing boot.")
        .rationale("No default: extra models are an operator's own catalogue, not something the product should invent."),
        param(
            "MARULA_HOST_MEMORY_MB",
            "ai",
            Integer,
            None,
            false,
            "Host memory the capacity floor is checked against. Unset reads the host, and a host that publishes no reading is not refused.",
        )
        .consequence("The memory figure the capacity floor is checked against. Unset reads the host. Declaring less than the box has is how a container gets refused before the kernel kills it.")
        .rationale("Unset by default because the host usually publishes a number, and a guessed value is worse than a measured one."),
        param(
            "MARULA_EMBEDDING_KEEP_ALIVE",
            "ai",
            String,
            Some("5m"),
            false,
            "How long the local embedding model stays resident after its last request (Ollama duration syntax).",
        )
        .consequence("How long the local model stays resident after its last request. A long value keeps searches fast and holds memory. A short one frees the memory and pays a reload on the next query, measured at 1.6 s.")
        .rationale("5m is the shipped balance: long enough to answer a burst of searches, short enough to give the memory back afterwards."),
        param(
            "MARULA_EMBEDDING_REINDEX_BATCH",
            "ai",
            Integer,
            Some("128"),
            false,
            "Rows embedded per pass while re-indexing into a new space.",
        )
        .consequence("Rows embedded per pass during a re-index. A larger batch fills the new space sooner and holds more text in memory while it does.")
        .rationale("128 rows is one embedding request batch at the measured throughput, so a pass is a request rather than a job."),
        param(
            "MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN",
            "ai",
            Integer,
            Some("600"),
            false,
            "Ceiling on rows embedded per minute during a re-index. The sweep pauses between batches to hold it.",
        )
        .consequence("The ceiling on re-index throughput. It is the knob that keeps a re-index from starving ordinary search and writes.")
        .rationale("600 rows a minute re-indexes a 100k-vector tenant in under three hours while leaving the database to its other work."),
        param(
            "MARULA_EMBEDDING_REINDEX_SECS",
            "ai",
            Integer,
            Some("60"),
            false,
            "How often an idle re-index sweep looks for work, in seconds. A tick with work is paced by the ceiling instead.",
        )
        .consequence("How often an idle re-index sweep looks for work. A tick that finds work is paced by the ceiling instead, so this only sets the idle look.")
        .rationale("60 s means a re-index that is allowed to run starts within a minute of being queued."),
        param(
            "MARULA_EMBEDDING_REINDEX_WINDOW",
            "ai",
            String,
            None,
            false,
            "UTC hours a re-index may run, as `HH-HH` such as 22-06 (end exclusive, a start after the end wraps midnight). Unset runs any hour.",
        )
        .consequence("The UTC hours a re-index may run. Outside the window the sweep does nothing, so a window that never includes the present stalls the re-index indefinitely.")
        .rationale("Unset runs any hour, which is right for an instance that does not have a quiet period to protect."),
        param(
            "MARULA_RETRIEVAL_MODE",
            "ai",
            String,
            Some("exact"),
            false,
            "How the search vector leg answers: exact, iterative or index.",
        )
        .consequence("The strategy the search vector leg uses. exact runs a sequential scan and returns the true nearest neighbours. iterative uses pgvector HNSW with hnsw.iterative_scan, which needs pgvector 0.8 and the HNSW index. index uses VectorChord with vchordrq.prefilter, which needs the vchord extension preloaded and its index. A mode this database cannot run refuses at boot instead of falling back to exact.")
        .rationale("exact is the default because it returns the true nearest neighbours at 0.48 ms for a small tenant. The two index modes need database capabilities a managed instance may not allow."),
        param(
            "MARULA_RETRIEVAL_WATCH_SECS",
            "ai",
            Integer,
            Some("300"),
            false,
            "How often the worker reports each tenant's embedding count as `marula.embedding.count`, in seconds. The report marks tenants past the 20,000 threshold where an index starts to pay.",
        )
        .consequence("How often each tenant's embedding count is reported. It decides how quickly an operator sees a tenant cross the threshold where an index starts to pay.")
        .rationale("300 s is five minutes, fast enough to notice growth and slow enough that the count query is not a load of its own."),

        // --- retries (Wave 52 lane R1) ---
        //
        // One preset sets every budget; the per-subsystem group is the
        // operator's escape hatch and always wins over the preset. Every
        // default below is the `balanced` preset's value, so a descriptor
        // reader can see the shipped numbers without reading the code.
        param(
            "MARULA_ROBUSTNESS",
            "retries",
            String,
            Some("balanced"),
            false,
            "Retry preset for every subsystem: balanced, durable, best_effort or single_box.",
        )
        .consequence("The preset every subsystem's retry budget starts from. Changing it moves seven subsystems at once, so a per-lever override is the surgical answer.")
        .rationale("balanced is the shipped preset because it is the one measured on the reference profiles. The other three exist for a one-box deployment, a long outage and a demo."),
        param(
            "MARULA_RETRY_QUEUE_BASE_MS",
            "retries",
            Integer,
            Some("5000"),
            false,
            "Delay before the second try for the job queue, in milliseconds.",
        )
        .consequence("The wait before the job queue retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("Five seconds is long enough for a restarting Postgres to come back and short enough that a ticket notification is not late."),
        param(
            "MARULA_RETRY_QUEUE_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for the job queue, in milliseconds.",
        )
        .consequence("The most the job queue backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("An hour is the ceiling because a queued job is work a person is waiting on. Past that, the operator should see the failure."),
        param(
            "MARULA_RETRY_QUEUE_ATTEMPTS",
            "retries",
            Integer,
            Some("10"),
            false,
            "Total tries for the job queue, including the first. 0 never gives up.",
        )
        .consequence("Total tries for the job queue, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("Ten tries cover the shipped backoff from five seconds to the one-hour cap."),
        param(
            "MARULA_RETRY_QUEUE_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for the job queue: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far job queue delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Full spread because every worker claims from the same table and would otherwise synchronise after an outage."),
        param(
            "MARULA_RETRY_QUEUE_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for the job queue. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for the job queue. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A queue job is not provably free of side effects when it is replayed, so a conflict is treated like any other 4xx instead of being posted again."),
        param(
            "MARULA_RETRY_EMAIL_BASE_MS",
            "retries",
            Integer,
            Some("30000"),
            false,
            "Delay before the second try for outbound email, in milliseconds.",
        )
        .consequence("The wait before outbound email retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("Thirty seconds is longer than a provider rate-limit window and shorter than the delay a recipient notices."),
        param(
            "MARULA_RETRY_EMAIL_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for outbound email, in milliseconds.",
        )
        .consequence("The most the outbound email backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("An hour lets a provider outage pass without dropping the message. Mail that is a day late has lost its point."),
        param(
            "MARULA_RETRY_EMAIL_ATTEMPTS",
            "retries",
            Integer,
            Some("6"),
            false,
            "Total tries for outbound email, including the first. 0 never gives up.",
        )
        .consequence("Total tries for outbound email, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("Six tries reach the one-hour cap with equal spread. A provider outage typically needs that much."),
        param(
            "MARULA_RETRY_EMAIL_JITTER",
            "retries",
            String,
            Some("equal"),
            false,
            "Delay spread for outbound email: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far outbound email delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Equal spread is enough for a single sender and keeps the retry order predictable in the delivery log."),
        param(
            "MARULA_RETRY_EMAIL_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for outbound email. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for outbound email. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A 409 from the mail provider is a settled outcome, so retrying it would only duplicate the refusal."),
        param(
            "MARULA_RETRY_WEBHOOK_BASE_MS",
            "retries",
            Integer,
            Some("30000"),
            false,
            "Delay before the second try for signed webhook delivery, in milliseconds.",
        )
        .consequence("The wait before signed webhook delivery retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("Thirty seconds lets a receiver that restarted come back before the second attempt."),
        param(
            "MARULA_RETRY_WEBHOOK_CAP_MS",
            "retries",
            Integer,
            Some("3600000"),
            false,
            "Ceiling on the computed delay for signed webhook delivery, in milliseconds.",
        )
        .consequence("The most the signed webhook delivery backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("An hour is the longest a receiver's outage is worth riding out before the delivery is marked failed."),
        param(
            "MARULA_RETRY_WEBHOOK_ATTEMPTS",
            "retries",
            Integer,
            Some("6"),
            false,
            "Total tries for signed webhook delivery, including the first. 0 never gives up.",
        )
        .consequence("Total tries for signed webhook delivery, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("Six tries reach the cap, the same shape the email path uses because receivers fail the same way."),
        param(
            "MARULA_RETRY_WEBHOOK_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for signed webhook delivery: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far signed webhook delivery delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Full spread because a fleet of receivers often fails together and should not retry together."),
        param(
            "MARULA_RETRY_WEBHOOK_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for signed webhook delivery. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for signed webhook delivery. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A replay that can double-post to a receiver is worse than a missed delivery the receiver can query for."),
        param(
            "MARULA_RETRY_EFFECTS_BASE_MS",
            "retries",
            Integer,
            Some("100"),
            false,
            "Delay before the second try for the worker to core-api effect seam, in milliseconds.",
        )
        .consequence("The wait before the worker to core-api effect seam retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("A hundred milliseconds is one local round trip: the seam is inside the deployment, not across the internet."),
        param(
            "MARULA_RETRY_EFFECTS_CAP_MS",
            "retries",
            Integer,
            Some("100"),
            false,
            "Ceiling on the computed delay for the worker to core-api effect seam, in milliseconds.",
        )
        .consequence("The most the worker to core-api effect seam backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("A hundred milliseconds is also the cap, because this seam is local and a slow call means core-api is unhealthy."),
        param(
            "MARULA_RETRY_EFFECTS_ATTEMPTS",
            "retries",
            Integer,
            Some("1"),
            false,
            "Total tries for the worker to core-api effect seam, including the first. 0 never gives up.",
        )
        .consequence("Total tries for the worker to core-api effect seam, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("One try: the seam is the API's own surface, and a refusal there is a real answer rather than a blip."),
        param(
            "MARULA_RETRY_EFFECTS_JITTER",
            "retries",
            String,
            Some("none"),
            false,
            "Delay spread for the worker to core-api effect seam: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far worker to core-api effect seam delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("No spread, because there is no fleet of clients to desynchronise."),
        param(
            "MARULA_RETRY_EFFECTS_CONFLICT",
            "retries",
            String,
            Some("retry"),
            false,
            "Whether a 409 is retried for the worker to core-api effect seam. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for the worker to core-api effect seam. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("Every endpoint on the effects seam is replay-inert, so a replayed conflict cannot double-apply the effect."),
        param(
            "MARULA_RETRY_CONSUMER_BASE_MS",
            "retries",
            Integer,
            Some("250"),
            false,
            "Delay before the second try for stream consumer reconnect, in milliseconds.",
        )
        .consequence("The wait before stream consumer reconnect retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("A quarter second reconnects a consumer before the stream falls behind the relay."),
        param(
            "MARULA_RETRY_CONSUMER_CAP_MS",
            "retries",
            Integer,
            Some("10000"),
            false,
            "Ceiling on the computed delay for stream consumer reconnect, in milliseconds.",
        )
        .consequence("The most the stream consumer reconnect backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("Ten seconds is the longest pause before a consumer looks stuck to the metrics."),
        param(
            "MARULA_RETRY_CONSUMER_ATTEMPTS",
            "retries",
            Integer,
            Some("0"),
            false,
            "Total tries for stream consumer reconnect, including the first. 0 never gives up.",
        )
        .consequence("Total tries for stream consumer reconnect, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("No ceiling, because a consumer that stops reconnecting stops live updates until the process restarts."),
        param(
            "MARULA_RETRY_CONSUMER_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for stream consumer reconnect: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far stream consumer reconnect delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Full spread keeps a fleet of consumer tasks from reconnecting in lockstep."),
        param(
            "MARULA_RETRY_CONSUMER_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for stream consumer reconnect. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for stream consumer reconnect. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A reconnect that collides is not provably replay-inert, so the consumer stops instead of replaying an entry it may already have applied."),
        param(
            "MARULA_RETRY_RELAY_BASE_MS",
            "retries",
            Integer,
            Some("250"),
            false,
            "Delay before the second try for outbox relay and realtime listener reconnect, in milliseconds.",
        )
        .consequence("The wait before outbox relay and realtime listener reconnect retries. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("A quarter second is below the interval at which a human notices a delayed update."),
        param(
            "MARULA_RETRY_RELAY_CAP_MS",
            "retries",
            Integer,
            Some("10000"),
            false,
            "Ceiling on the computed delay for outbox relay and realtime listener reconnect, in milliseconds.",
        )
        .consequence("The most the outbox relay and realtime listener reconnect backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("Ten seconds is the ceiling: a relay that waits longer looks down, and the backstop poll covers it."),
        param(
            "MARULA_RETRY_RELAY_ATTEMPTS",
            "retries",
            Integer,
            Some("0"),
            false,
            "Total tries for outbox relay and realtime listener reconnect, including the first. 0 never gives up.",
        )
        .consequence("Total tries for outbox relay and realtime listener reconnect, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("No ceiling, because the relay is what turns a committed row into a delivered event."),
        param(
            "MARULA_RETRY_RELAY_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for outbox relay and realtime listener reconnect: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far outbox relay and realtime listener reconnect delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Full spread because every instance runs a relay and they all reconnect to the same stream."),
        param(
            "MARULA_RETRY_RELAY_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for outbox relay and realtime listener reconnect. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for outbox relay and realtime listener reconnect. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A conflict on the relay means the row was already published under that key. Re-publishing would duplicate the event."),
        param(
            "MARULA_RETRY_DECISION_BASE_MS",
            "retries",
            Integer,
            Some("200"),
            false,
            "Delay before the second try for the decision provider's upstream calls, in milliseconds.",
        )
        .consequence("The wait before the decision provider's upstream calls retry. It sets how fast a transient fault is ridden out and how hard a failing dependency is hit.")
        .rationale("Two hundred milliseconds leaves room for a model call to start without hammering a rate limit."),
        param(
            "MARULA_RETRY_DECISION_CAP_MS",
            "retries",
            Integer,
            Some("2000"),
            false,
            "Ceiling on the computed delay for the decision provider's upstream calls, in milliseconds.",
        )
        .consequence("The most the decision provider's upstream calls backoff may grow to. A low cap exhausts the tries during a long outage. A high cap keeps work waiting after it could have failed.")
        .rationale("Two seconds is the cap: a decision that takes longer has failed a user who is waiting for an answer."),
        param(
            "MARULA_RETRY_DECISION_ATTEMPTS",
            "retries",
            Integer,
            Some("3"),
            false,
            "Total tries for the decision provider's upstream calls, including the first. 0 never gives up.",
        )
        .consequence("Total tries for the decision provider's upstream calls, including the first. Zero removes the ceiling, so a broken dependency is retried forever.")
        .rationale("Three tries cover a cold model or a transient 429 without a user staring at a spinner."),
        param(
            "MARULA_RETRY_DECISION_JITTER",
            "retries",
            String,
            Some("full"),
            false,
            "Delay spread for the decision provider's upstream calls: none, full (0 to delay) or equal (half to delay).",
        )
        .consequence("How far the decision provider's upstream call delays vary between attempts. With no variation, everything that failed in one outage retries in lockstep and reproduces the outage.")
        .rationale("Full spread because a batch of decisions often arrives together and should not retry in step."),
        param(
            "MARULA_RETRY_DECISION_CONFLICT",
            "retries",
            String,
            Some("permanent"),
            false,
            "Whether a 409 is retried for the decision provider's upstream calls. Retry only where a replay cannot double-apply.",
        )
        .consequence("Whether a 409 is retried for the decision provider's upstream calls. Retrying where a replay can double-apply posts the same thing twice. Refusing where it cannot gives up too early.")
        .rationale("A conflict from the provider is a settled refusal, and a replay would spend quota for the same answer."),
    ]
}

/// JSON rendering of the descriptor, optionally narrowed to one profile.
///
/// No database is consulted: the retrieval section says `probe: not_run`. Use
/// [`describe_json_probed`] for the satisfiability check the row asks for.
pub fn describe_json(profile: Option<&str>) -> anyhow::Result<String> {
    describe_json_of(descriptor(), profile)
}

/// JSON rendering with the retrieval mode probed against a database.
///
/// A connection fault is reported inside the descriptor rather than as a
/// command failure: an operator asking what a deployment looks like should
/// still get the answer when Postgres is down, with the unsatisfied check
/// labelled `unreachable` instead of a claim it did not verify.
pub async fn describe_json_probed(
    profile: Option<&str>,
    database_url: Option<&str>,
) -> anyhow::Result<String> {
    if database_url.is_none_or(|url| url.trim().is_empty()) {
        return describe_json(profile);
    }
    let mut desc = descriptor();
    desc.retrieval = probe_retrieval(database_url).await;
    describe_json_of(desc, profile)
}

/// Probe the retrieval mode and distribution, with a short timeout.
pub async fn probe_retrieval(database_url: Option<&str>) -> RetrievalSpec {
    use std::time::Duration;

    let Some(url) = database_url.filter(|u| !u.trim().is_empty()) else {
        return retrieval_spec();
    };
    let connect = tokio::time::timeout(
        Duration::from_secs(3),
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(3))
            .connect(url),
    )
    .await;
    let pool = match connect {
        Ok(Ok(pool)) => pool,
        Ok(Err(error)) => {
            return retrieval_spec_from(None, None, Some(error.to_string()));
        }
        Err(_) => {
            return retrieval_spec_from(
                None,
                None,
                Some("connection timed out after 3 s".to_string()),
            );
        }
    };
    let mode = core_domain::retrieval::RetrievalMode::from_lookup(&|key| std::env::var(key).ok())
        .unwrap_or(core_domain::retrieval::RetrievalMode::Exact);
    let probe = match core_db::retrieval::probe(&pool, mode).await {
        Ok(probe) => probe,
        Err(error) => return retrieval_spec_from(None, None, Some(error.to_string())),
    };
    let distribution = core_db::retrieval::embedding_distribution(&pool).await.ok();
    pool.close().await;
    retrieval_spec_from(Some(&probe), distribution, None)
}

/// Refuse to publish a descriptor that contradicts itself.
///
/// A consumer pins this JSON and renders from it, so a contradiction here
/// becomes a contradiction in somebody else's install wizard. The check is
/// cheap and static; running it on every publish is the point.
fn refuse_contradictions(desc: &DeployDescriptor) -> anyhow::Result<()> {
    let violations = crate::deploy_check::check(desc);
    if violations.is_empty() {
        return Ok(());
    }
    let listing = violations
        .iter()
        .map(|v| format!("  - {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::bail!(
        "the deploy descriptor contradicts itself ({} violation(s)):\n{listing}",
        violations.len()
    )
}

fn describe_json_of(mut desc: DeployDescriptor, profile: Option<&str>) -> anyhow::Result<String> {
    refuse_contradictions(&desc)?;
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
    describe_summary_of(&descriptor())
}

/// The same summary, with the retrieval mode probed against a database.
pub async fn describe_summary_probed(database_url: Option<&str>) -> String {
    if database_url.is_none_or(|url| url.trim().is_empty()) {
        return describe_summary();
    }
    let mut desc = descriptor();
    desc.retrieval = probe_retrieval(database_url).await;
    describe_summary_of(&desc)
}

fn describe_summary_of(d: &DeployDescriptor) -> String {
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
        "\nretrieval: mode via {} (default {}); this environment: {} (probe {})\n",
        d.retrieval.mode_env,
        d.retrieval
            .modes
            .iter()
            .find(|m| m.default)
            .map_or("exact", |m| m.name),
        d.retrieval.resolved.mode.unwrap_or("(refused)"),
        d.retrieval.resolved.probe,
    ));
    if let Some(refusal) = &d.retrieval.resolved.refusal {
        out.push_str(&format!("  refusal: {refusal}\n"));
    }
    if let Some(distribution) = &d.retrieval.distribution {
        out.push_str(&format!(
            "  embeddings per tenant (threshold {}): {} tenant(s), {} over\n",
            distribution.threshold_embeddings,
            distribution.tenants.len(),
            distribution.tenants_over_threshold,
        ));
        for tenant in distribution.tenants.iter().take(10) {
            out.push_str(&format!(
                "    {} {}{}\n",
                tenant.org_id,
                tenant.embeddings,
                if tenant.index_recommended {
                    "  (at/over threshold: index mode recommended)"
                } else {
                    ""
                }
            ));
        }
        out.push_str(&format!("  {}\n", distribution.recommendation));
    }
    out.push_str(&format!(
        "\nretries: preset via {} ({} presets, {} subsystems, {} levers each)\n",
        d.retries.preset_env,
        d.retries.presets.len(),
        d.retries.presets.first().map_or(0, |p| p.policies.len()),
        core_domain::retry::Lever::ALL.len(),
    ));
    out.push_str(&format!(
        "\n{} runtime parameters; emit JSON with `marula deploy describe --json`\n",
        d.params.len()
    ));
    let violations = crate::deploy_check::check(d);
    if !violations.is_empty() {
        out.push_str(&format!(
            "\ndescriptor self-check: {} violation(s) — `deploy describe --json` refuses this build\n",
            violations.len()
        ));
        for violation in &violations {
            out.push_str(&format!("  - {violation}\n"));
        }
    }
    out
}

/// The repo root, from this crate's manifest dir.
#[cfg(test)]
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Env names read by a source tree, discovered by scanning the code.
///
/// The scan is deliberately dumb: find a call that looks like an env read, then
/// take the first all-caps string literal in the same statement (up to the next
/// `;` or `{`, so a literal on a later line cannot be misread as this call's
/// argument). Test modules are skipped — a fixture's `"AKIAEXAMPLE"` is not a
/// lever.
///
/// Its failure mode is a false positive, which surfaces as "add a descriptor row
/// or explain in `NON_LEVER_ENV`". That is the right side to err on: a missing
/// row is the bug this test exists for.
#[cfg(test)]
fn env_names_in(root: &Path) -> std::collections::BTreeSet<String> {
    const SIGNALS: [&str; 9] = [
        "std::env::var(",
        "env::var(",
        "env_text(",
        "env_configured(",
        "lookup(",
        "parse_pool_max(",
        "parse_bounded_u64(",
        "dsn_state(",
        "env(",
    ];
    let mut names = std::collections::BTreeSet::new();
    for path in rust_files(root) {
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        // A test's own values are not levers.
        let source = match source.find("#[cfg(test)]") {
            Some(cut) => &source[..cut],
            None => source.as_str(),
        };
        for signal in SIGNALS {
            let mut from = 0;
            while let Some(offset) = source[from..].find(signal) {
                let after = from + offset + signal.len();
                let end = source.len().min(after + 160);
                let window = &source[after..end];
                let stop = window.find([';', '{']).unwrap_or(window.len());
                if let Some(name) = first_caps_literal(&window[..stop]) {
                    if !NON_LEVER_ENV.contains(&name.as_str()) {
                        names.insert(name);
                    }
                }
                from = after;
            }
        }
    }
    names
}

#[cfg(test)]
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The first `"SCREAMING_SNAKE"` literal in a fragment, if any.
#[cfg(test)]
fn first_caps_literal(fragment: &str) -> Option<String> {
    let mut rest = fragment;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let close = after.find('"')?;
        let candidate = &after[..close];
        let looks_like_env = candidate.len() >= 3
            && candidate.starts_with(|c: char| c.is_ascii_uppercase())
            && candidate
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
        if looks_like_env {
            return Some(candidate.to_string());
        }
        rest = &after[close + 1..];
    }
    None
}

/// Values read through a computed variable, so the scan cannot see them. Every
/// entry still has to be a descriptor parameter — that is the assertion.
#[cfg(test)]
const DYNAMIC_ENV_READS: [&str; 9] = [
    // `read_key(inline_var, file_var)` in core-api's EdDSA resolution.
    "MARULA_JWT_EDDSA_PRIVATE_KEY",
    "MARULA_JWT_EDDSA_PRIVATE_KEY_FILE",
    "MARULA_JWT_EDDSA_PUBLIC_KEY",
    "MARULA_JWT_EDDSA_PUBLIC_KEY_FILE",
    // `oauth_client_config(provider)` picks the pair by provider name.
    "GOOGLE_CALENDAR_CLIENT_ID",
    "GOOGLE_CALENDAR_CLIENT_SECRET",
    "MICROSOFT_GRAPH_CLIENT_ID",
    "MICROSOFT_GRAPH_CLIENT_SECRET",
    // `RetrievalMode::from_lookup` reads it once at boot (Wave 52 lane R15), so
    // the source scan cannot see the name.
    "MARULA_RETRIEVAL_MODE",
];

/// Values the processes read that are not operator levers, each for a reason.
/// Keep this list short: it is the only place a lever can hide.
#[cfg(test)]
const NON_LEVER_ENV: [&str; 1] = [
    // Provided by the container runtime for consumer naming; telling an
    // operator to set it would be noise.
    "HOSTNAME",
];

/// Regenerates `deploy/profiles/*.yaml` from this module. Returns the paths
/// written.
pub fn write_profiles(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    refuse_contradictions(&descriptor())?;
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
    s.push_str("floors:\n");
    s.push_str(&format!("  cores: {}\n", p.floors.cores));
    s.push_str(&format!("  ram_mb: {}\n", p.floors.ram_mb));
    s.push_str(&format!("  disk_gb: {}\n", p.floors.disk_gb));
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
    use std::collections::BTreeSet;

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
            "routing",
            "limits",
            "ai",
            "retries",
        ] {
            assert!(groups.contains(known), "group {known} missing");
        }
    }

    /// Every env value core-api or worker reads must appear in the descriptor.
    ///
    /// The gap this closes: `marula deploy describe` listed 49 parameters
    /// while the two processes read roughly a hundred, so an operator
    /// deploying from the descriptor could not discover the knobs that decide
    /// whether the queue retries, whether the relay listens, or whether the
    /// process boots at all.
    ///
    /// A name reaches the descriptor one of two ways: the scan finds it in
    /// source, or it is listed in [`DYNAMIC_ENV_READS`] because it is read
    /// through a computed variable. Both sets are checked, so "make the test
    /// pass" is not a way to hide a lever.
    #[test]
    fn every_env_value_the_processes_read_has_a_descriptor_entry() {
        let mut discovered = BTreeSet::new();
        for root in ["crates/core-api/src", "crates/worker/src"] {
            discovered.extend(env_names_in(&repo_root().join(root)));
        }
        assert!(
            discovered.len() > 80,
            "the scan found only {} env names — it is broken, not the descriptor",
            discovered.len()
        );

        for name in DYNAMIC_ENV_READS {
            assert!(
                params().iter().any(|p| p.name == name),
                "dynamic read {name} is not in the descriptor"
            );
        }

        let mut missing: Vec<&str> = discovered
            .iter()
            .map(String::as_str)
            .filter(|name| !NON_LEVER_ENV.contains(name))
            .filter(|name| !params().iter().any(|p| p.name == *name))
            .collect();
        missing.sort_unstable();
        assert!(
            missing.is_empty(),
            "{} env values are read by core-api or worker but absent from \
             `marula deploy describe`: {missing:?}\n\
             Add a descriptor entry (group it with its siblings) or, if it is \
             not an operator lever, say why in NON_LEVER_ENV.",
            missing.len()
        );
    }

    /// The retry matrix is the product decision; the descriptor has to publish
    /// it. Every lever the resolver accepts is a descriptor parameter and
    /// every descriptor parameter in the `retries` group is a lever the
    /// resolver reads — otherwise the escape hatch and its documentation can
    /// drift.
    #[test]
    fn every_retry_lever_is_in_the_descriptor_and_vice_versa() {
        let levers: BTreeSet<&str> = core_domain::retry::env_names().into_iter().collect();
        let described: BTreeSet<&str> = params()
            .iter()
            .filter(|p| p.group == "retries")
            .map(|p| p.name)
            .collect();
        assert_eq!(
            levers, described,
            "the retry levers and the descriptor's `retries` group disagree"
        );
        assert!(levers.contains(core_domain::retry::ROBUSTNESS_ENV));
    }

    /// The retry section of the JSON speaks in the same vocabulary as the env:
    /// a consumer renders the preset name and the lever names, not Rust type
    /// names.
    #[test]
    fn the_json_publishes_the_retry_presets_and_the_resolved_defaults() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let retries = &v["retries"];
        assert_eq!(retries["preset_env"], core_domain::retry::ROBUSTNESS_ENV);
        let presets = retries["presets"].as_array().expect("presets array");
        assert_eq!(presets.len(), 4);
        let names: Vec<&str> = presets
            .iter()
            .map(|p| p["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            names,
            vec!["balanced", "durable", "best_effort", "single_box"]
        );

        let balanced = presets
            .iter()
            .find(|p| p["name"] == "balanced")
            .expect("balanced preset");
        let policies = balanced["policies"].as_array().expect("policies");
        assert_eq!(policies.len(), 7);
        let queue = policies
            .iter()
            .find(|p| p["subsystem"] == "queue")
            .expect("queue policy");
        // The shipped queue budget, readable without the source.
        assert_eq!(queue["attempts"], 10);
        assert_eq!(queue["cap_ms"], 3_600_000);
        assert_eq!(queue["jitter"], "full");
    }

    /// The retrieval section is the deployment decision, in the descriptor:
    /// three modes with the requirement each one adds, exact as the default,
    /// and `probe: not_run` when no database was asked. An operator must be
    /// able to read the whole decision without the source (Wave 52 lane R15).
    #[test]
    fn the_json_publishes_the_retrieval_modes_and_their_requirements() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let retrieval = &v["retrieval"];
        assert_eq!(retrieval["mode_env"], "MARULA_RETRIEVAL_MODE");
        assert_eq!(retrieval["threshold_embeddings"], 20_000);
        assert_eq!(retrieval["applies_to"][0], "search_embeddings");
        assert_eq!(retrieval["resolved"]["probe"], "not_run");
        assert!(
            retrieval["distribution"].is_null(),
            "a descriptor read with no database must not invent a distribution"
        );

        let modes = retrieval["modes"].as_array().expect("modes array");
        assert_eq!(modes.len(), 3);
        let names: Vec<&str> = modes
            .iter()
            .map(|m| m["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["exact", "iterative", "index"]);

        let exact = &modes[0];
        assert_eq!(exact["default"], true);
        assert_eq!(exact["requires"].as_array().map(Vec::len), Some(0));

        let iterative = &modes[1];
        let iterative_requires = iterative["requires"].as_array().expect("requires");
        assert!(
            iterative_requires
                .iter()
                .any(|r| r.as_str().unwrap_or_default().contains("0.8")),
            "iterative must name its pgvector floor: {iterative_requires:?}"
        );
        assert!(
            iterative_requires.iter().any(|r| r
                .as_str()
                .unwrap_or_default()
                .contains("search_embeddings_hnsw")),
            "iterative must name the index it needs: {iterative_requires:?}"
        );

        let index = &modes[2];
        let index_requires = index["requires"].as_array().expect("requires");
        assert!(
            index_requires
                .iter()
                .any(|r| r.as_str().unwrap_or_default().contains("vchord")),
            "index must name the extension: {index_requires:?}"
        );
        assert!(
            index_requires.iter().any(|r| r
                .as_str()
                .unwrap_or_default()
                .contains("shared_preload_libraries")),
            "index must name the preload, which is the requirement a managed \
             instance cannot satisfy: {index_requires:?}"
        );

        // Every mode states the shared-leg arithmetic, so the numbers cannot be
        // read as "the database owns 50 ms".
        for mode in modes {
            let note = mode["leg_note"].as_str().unwrap_or_default();
            assert!(
                note.contains("47 ms"),
                "{} lacks the leg note",
                mode["name"]
            );
            assert!(!mode["latency"].as_str().unwrap_or_default().is_empty());
        }
    }

    #[test]
    fn only_the_boot_critical_secrets_are_required() {
        // `required` means the process refuses to boot without it. Most secrets
        // are conditional on the feature they configure (an S3 key, a Postmark
        // token), so they are not required here. The four that are: the two
        // signing/sealing keys and the two seam tokens, which a profile that
        // serves the seam refuses to start without (634bd65c made the seam
        // tokens requirements and left this assertion behind).
        let required: Vec<&str> = params()
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        assert_eq!(
            required,
            vec![
                "JWT_SECRET",
                "MARULA_SECRET_KEY",
                "MARULA_INTERNAL_TOKEN",
                "EMAIL_INTERNAL_TOKEN"
            ]
        );
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
        // Wave 52 lane P9: the shipped shape is pooled, and the three presets
        // that make it safe are part of the profile rather than prose.
        assert_eq!(
            p.sets.get("DATABASE_URL"),
            Some(&"postgres://marula_app:${MARULA_APP_PASSWORD}@pgbouncer:6432/marula"),
            "split dials the pooler, and carries no credential of its own"
        );
        assert_eq!(
            p.sets.get("MARULA_LISTEN_DATABASE_URL"),
            Some(&"postgres://marula_app:${MARULA_APP_PASSWORD}@postgres:5432/marula"),
            "the LISTEN connection bypasses the pooler"
        );
        assert_eq!(p.sets.get("MARULA_DB_STATEMENT_CACHE"), Some(&"0"));
    }

    /// The bound the lane promised to make mechanical: the statement-cache
    /// capacity the app is told to use must fit inside the pooler's own
    /// `max_prepared_statements`, and the shipped reset query decides whether
    /// a cache is legitimate at all.
    ///
    /// `deploy/pooler/pgbouncer.ini` is the config the compose stack runs, so
    /// reading it here ties the two halves of the setting together: editing
    /// either one alone fails this test.
    #[test]
    fn the_statement_cache_fits_the_shipped_pooler() {
        let ini = std::fs::read_to_string(repo_root().join("deploy/pooler/pgbouncer.ini"))
            .expect("deploy/pooler/pgbouncer.ini");
        let value = |key: &str| -> String {
            ini.lines()
                .map(str::trim)
                .filter(|line| !line.starts_with(';'))
                .find_map(|line| {
                    let (k, v) = line.split_once('=')?;
                    (k.trim() == key).then(|| v.trim().to_string())
                })
                .unwrap_or_else(|| panic!("deploy/pooler/pgbouncer.ini is missing {key}"))
        };

        assert_eq!(
            value("pool_mode"),
            "transaction",
            "the shipped pooler is a transaction pooler; the app's statement cache \
             capacity was chosen for it"
        );
        assert_eq!(value("server_reset_query"), "DISCARD ALL");
        let reset_always = value("server_reset_query_always");
        assert_eq!(
            reset_always, "1",
            "the reset query is what stops one tenant's session state reaching the next \
             client — see docs/reference/connection-pooling.md"
        );

        let cache_default: u32 = params()
            .iter()
            .find(|p| p.name == "MARULA_DB_STATEMENT_CACHE")
            .and_then(|p| p.default)
            .expect("the cache capacity has a default")
            .parse()
            .expect("the default is an integer");
        let max_prepared: u32 = value("max_prepared_statements")
            .parse()
            .expect("max_prepared_statements is an integer");

        assert!(
            cache_default <= max_prepared,
            "MARULA_DB_STATEMENT_CACHE default {cache_default} exceeds the pooler's \
             max_prepared_statements {max_prepared}"
        );
        // With the reset query running after every transaction the server
        // deallocates the statements anyway, so a non-zero cache would pay for
        // re-parsing AND gain nothing. Keep the pair honest.
        assert_eq!(
            (reset_always.as_str(), cache_default),
            ("1", 0),
            "the shipped pooler resets after every transaction, so the app's cache \
             must stay disabled: a cached name cannot survive the reset"
        );
    }

    #[test]
    fn describe_json_is_valid_and_pins_the_schema() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["profiles"].as_array().map(Vec::len), Some(2));
        assert!(!v["core_version"].as_str().unwrap_or("").is_empty());
    }

    /// Schema 2 is the shape a consumer pins: every lever states its
    /// consequence, why its default is what it is, and which levers it needs or
    /// cannot sit beside. A field that quietly disappears breaks a wizard that
    /// renders from it, so the shape is asserted rather than assumed.
    #[test]
    fn every_lever_publishes_its_consequence_and_edges() {
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["schema_version"], 2);
        let params = v["params"].as_array().expect("params array");
        assert_eq!(params.len(), 180);
        for param in params {
            let name = param["name"].as_str().unwrap_or_default();
            let consequence = param["consequence"].as_str().unwrap_or_default();
            assert!(
                consequence.split_whitespace().count() >= 8,
                "{name} publishes a consequence too short to render: {consequence:?}"
            );
            if !param["default"].is_null() {
                assert!(
                    param["default_rationale"]
                        .as_str()
                        .is_some_and(|r| r.split_whitespace().count() >= 8),
                    "{name} has a default and no rationale in the JSON"
                );
            }
            assert!(param["requires"].is_array(), "{name} has no requires list");
            assert!(
                param["conflicts"].is_array(),
                "{name} has no conflicts list"
            );
        }
        let all_in_one = params
            .iter()
            .find(|p| p["name"] == "MARULA_ALL_IN_ONE")
            .expect("the all-in-one lever");
        assert_eq!(
            all_in_one["requires"][0], "MARULA_WORKER_DATABASE_URL",
            "all-in-one must name the owner URL it needs"
        );
    }

    /// Every profile states the machine it needs. The wizard scores a host from
    /// these numbers and the check refuses a profile its own floors forbid, so
    /// an absent or zero floor is a broken contract rather than a missing field.
    #[test]
    fn every_profile_declares_its_floors() {
        for profile in profiles() {
            assert!(
                profile.floors.cores > 0,
                "profile {} declares {} cores",
                profile.name,
                profile.floors.cores
            );
            assert!(
                profile.floors.ram_mb >= 1024,
                "profile {} declares {} MB, which cannot run the database it needs",
                profile.name,
                profile.floors.ram_mb
            );
            assert!(
                profile.floors.disk_gb > 0,
                "profile {} declares no disk",
                profile.name
            );
        }
        let json = describe_json(None).expect("json");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(v["profiles"][0]["floors"]["cores"], 4);
    }

    /// The publish path refuses a descriptor that argues with itself, so a
    /// consumer never pins a contract whose own explanations contradict it.
    #[test]
    fn a_contradicting_descriptor_is_refused_at_publish() {
        let mut desc = descriptor();
        desc.params[0].consequence = "";
        let error = describe_json_of(desc, None).expect_err("a blank consequence must refuse");
        let message = error.to_string();
        assert!(
            message.contains("contradicts itself") && message.contains("no consequence"),
            "unexpected refusal: {message}"
        );

        let mut desc = descriptor();
        desc.params[2].requires = vec!["MARULA_NO_SUCH_LEVER"];
        let error = describe_json_of(desc, None).expect_err("a phantom lever must refuse");
        assert!(
            error.to_string().contains("MARULA_NO_SUCH_LEVER"),
            "the refusal must name the lever: {error}"
        );
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
