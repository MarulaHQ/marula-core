//! The embedding model catalogue, the instance-level selection, the space tag,
//! the tier floors, and the re-index policy (Wave 52 lane R14).
//!
//! Purpose: make the embedding model a named, chosen-once-per-instance thing.
//! `completion.rs` resolves one ORG's provider and model; this module resolves
//! the INSTANCE's model, proves the host can host it, and stamps the vector
//! space its output lives in, so vectors from two models are never compared.
//!
//! Four pieces:
//! - [`Catalogue`] — the shipped models with dimensions, modality, license,
//!   provider, tier, floor, quality note and platform note, plus custom entries
//!   parsed from data so adding a model is not a code change;
//! - [`select_instance`] — tier, model lever and provider lever in, one entry
//!   out, with a typed refusal for a tier or provider combination that has no
//!   model;
//! - [`check_floor`] — the resource floor for that entry against a measured
//!   [`HostCapacity`], refused with a value that names cores, megabytes, tier
//!   and model;
//! - [`EmbeddingSpace`] — `model@version/dimensions`, the tag every stored
//!   vector carries and every retrieval filters on.
//!
//! I/O exception: [`detect_host_capacity`] reads `/sys/fs/cgroup/memory.max`
//! and `/proc/meminfo`, and nothing else in this crate reads the machine. The
//! floor has to be checked identically in core-api and the worker before either
//! spawns a loop, and duplicating the probe in both is how the two processes
//! end up disagreeing about the same host. Tests inject a lookup and a
//! `HostCapacity` instead of touching either file.
//!
//! Non-purpose: no HTTP, no model call, no database. Whether a model is
//! reachable, and what its vectors are, belongs to core-api's adapter.

use std::borrow::Cow;
use std::fmt;

/// Environment lever naming the tier. Read by the CLI, core-api and the worker.
pub const TIER_ENV: &str = "MARULA_EMBEDDING_TIER";
/// Environment lever naming one model explicitly. Wins over the tier's default.
pub const MODEL_ENV: &str = "MARULA_EMBEDDING_MODEL";
/// Environment lever naming the provider path: `ollama` (local, the default)
/// or a cloud provider id. The catalogue entry names one too; this wins.
pub const PROVIDER_ENV: &str = "MARULA_EMBEDDING_PROVIDER";
/// Instance-level credential for a cloud override. Per-org BYOK is consulted
/// second, and only for the provider this instance selected.
pub const API_KEY_ENV: &str = "MARULA_EMBEDDING_API_KEY";
/// Data path for extra catalogue entries. See [`Catalogue::with_custom`].
pub const CUSTOM_MODELS_ENV: &str = "MARULA_EMBEDDING_CUSTOM_MODELS";
/// Declared host memory in megabytes. Beats cgroup and `/proc` detection, which
/// is what a container needs when the limit is not visible from inside.
pub const HOST_MEMORY_ENV: &str = "MARULA_HOST_MEMORY_MB";

/// Version component used for a model this build has no entry for. A build
/// number nobody recorded is not a number, and pretending `1` would make two
/// different builds of one id look interchangeable.
pub const UNVERSIONED: &str = "unversioned";
/// Version component used for a model an operator added through
/// [`CUSTOM_MODELS_ENV`].
pub const CUSTOM_VERSION: &str = "custom";

// ---------------------------------------------------------------------------
// Tiers and floors
// ---------------------------------------------------------------------------

/// The three model classes. A tier names a capacity class, not a feature
/// level: `advanced` buys multimodal chunks and costs resident memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingTier {
    /// The smallest profile: a bge-base-class local model, or a cloud one.
    Minimal,
    /// The shipped default: EmbeddingGemma 2, text-only.
    Standard,
    /// The multimodal class.
    Advanced,
}

impl EmbeddingTier {
    /// Every tier, in ascending order.
    pub const ALL: [Self; 3] = [Self::Minimal, Self::Standard, Self::Advanced];

    /// Stable id, the value the levers and the descriptor use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::Advanced => "advanced",
        }
    }

    /// Parses a lever value case-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|tier| tier.as_str().eq_ignore_ascii_case(value))
    }
}

impl fmt::Display for EmbeddingTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a tier needs from the machine before it may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EmbeddingFloor {
    /// Logical cores. Zero means "no local floor" (a cloud model).
    pub cores: u16,
    /// Resident megabytes the model needs with one batch in flight. Zero means
    /// "no local floor".
    pub ram_mb: u32,
}

impl EmbeddingFloor {
    /// No local requirement: the model runs elsewhere.
    pub const CLOUD: Self = Self {
        cores: 0,
        ram_mb: 0,
    };

    /// Whether this entry has a local floor at all.
    #[must_use]
    pub fn is_local(self) -> bool {
        self.cores > 0 || self.ram_mb > 0
    }
}

// ---------------------------------------------------------------------------
// Catalogue
// ---------------------------------------------------------------------------

/// One selectable model.
///
/// `quality_note` and `platform_note` are part of the entry on purpose: the
/// bf16 build of EmbeddingGemma 2 is three times the q8_0 rate on arm64 at
/// identical ranking, and it is still not the default because x86 is unmeasured
/// on the hardware these numbers come from. That belongs beside the id, where
/// the operator choosing a model reads it, not in a commit message.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EmbeddingModelEntry {
    /// Provider model id, as the adapter sends it.
    pub id: Cow<'static, str>,
    pub display_name: Cow<'static, str>,
    /// Native width this entry is asked for. The store is fixed at
    /// [`crate::SEARCH_EMBEDDING_DIM`], so an entry whose native width is wider
    /// is truncated by the provider (OpenAI's `dimensions`).
    pub dimensions: u16,
    /// `text`, or the modalities a multimodal build accepts.
    pub modality: Cow<'static, str>,
    pub license: Cow<'static, str>,
    /// Provider path that serves it: `ollama` or a cloud id.
    pub provider: Cow<'static, str>,
    /// Build version, recorded in the space tag so two builds of one id are
    /// distinguishable. [`UNVERSIONED`] for a pre-tag population,
    /// [`CUSTOM_VERSION`] for an operator-supplied entry.
    pub version: Cow<'static, str>,
    pub tier: EmbeddingTier,
    pub floor: EmbeddingFloor,
    /// What it costs and what it returns, with the measured numbers.
    pub quality_note: Cow<'static, str>,
    /// Where it is known to be the right build.
    pub platform_note: Cow<'static, str>,
}

impl EmbeddingModelEntry {
    /// The space tag this entry's vectors live in.
    #[must_use]
    pub fn space(&self) -> EmbeddingSpace {
        EmbeddingSpace {
            model: self.id.to_string(),
            version: self.version.to_string(),
            dimensions: self.dimensions,
        }
    }

    // One call per catalogue row, so the table reads as a table. A struct
    // literal per entry would be three times the lines for the same data.
    #[allow(clippy::too_many_arguments)]
    fn borrowed(
        id: &'static str,
        display_name: &'static str,
        dimensions: u16,
        modality: &'static str,
        license: &'static str,
        provider: &'static str,
        version: &'static str,
        tier: EmbeddingTier,
        floor: EmbeddingFloor,
        quality_note: &'static str,
        platform_note: &'static str,
    ) -> Self {
        Self {
            id: Cow::Borrowed(id),
            display_name: Cow::Borrowed(display_name),
            dimensions,
            modality: Cow::Borrowed(modality),
            license: Cow::Borrowed(license),
            provider: Cow::Borrowed(provider),
            version: Cow::Borrowed(version),
            tier,
            floor,
            quality_note: Cow::Borrowed(quality_note),
            platform_note: Cow::Borrowed(platform_note),
        }
    }
}

/// The shipped models. Floors are R13's measurements plus headroom; see each
/// entry's `quality_note`.
#[must_use]
pub fn builtin_entries() -> Vec<EmbeddingModelEntry> {
    vec![
        EmbeddingModelEntry::borrowed(
            "bge-base-en-v1.5:109m-q8_0",
            "BGE Base EN v1.5 (109M, q8_0)",
            768,
            "text",
            "MIT",
            "ollama",
            "1",
            EmbeddingTier::Minimal,
            EmbeddingFloor {
                cores: 2,
                ram_mb: 512,
            },
            "The smallest local text embedder here, roughly 340 MB resident. Estimated, not \
             measured on our profiles: the 512 MB floor is the same headroom rule the measured \
             entries use.",
            "Any CPU. This is the local model for a 2-core box, where the standard tier refuses.",
        ),
        EmbeddingModelEntry::borrowed(
            "embeddinggemma-2:270m-q8_0-text",
            "EmbeddingGemma 2 text-only (271M, q8_0)",
            768,
            "text",
            "Apache-2.0",
            "ollama",
            "1",
            EmbeddingTier::Standard,
            EmbeddingFloor {
                cores: 4,
                ram_mb: 768,
            },
            "The shipped default. 16.0 chunks/s and 508 MB resident at 4 cores, 47 ms per \
             search query, 1.6 s to reload after an idle unload. Recall@10 0.863 on R13's \
             ticket-shaped corpus.",
            "Any CPU, and the default because x86 is unmeasured for the bf16 build.",
        ),
        EmbeddingModelEntry::borrowed(
            "embeddinggemma-2:270m-bf16-text",
            "EmbeddingGemma 2 text-only (271M, bf16)",
            768,
            "text",
            "Apache-2.0",
            "ollama",
            "1",
            EmbeddingTier::Standard,
            EmbeddingFloor {
                cores: 4,
                ram_mb: 1024,
            },
            "49.0 chunks/s against 16.0 for the q8_0 build, query p50/p95 19.6/25.3 ms. \
             Identical top-10 on 23 of 23 queries and the same recall@10.",
            "arm64 only so far: that is where R13 measured it. An x86 host picking this build \
             is choosing a number nobody has taken.",
        ),
        EmbeddingModelEntry::borrowed(
            "embeddinggemma-2:740m-q8_0",
            "EmbeddingGemma 2 multimodal (740M, q8_0)",
            768,
            "text, image, audio",
            "Apache-2.0",
            "ollama",
            "1",
            EmbeddingTier::Advanced,
            EmbeddingFloor {
                cores: 4,
                ram_mb: 2048,
            },
            "Text tower plus a 473M projector. 16.2 chunks/s at 1,461 MB resident, with the \
             identical top-10 on 23 of 23 text queries: media support is what the memory buys.",
            "Any CPU with the memory. The 2 GB floor is 1,461 MB resident plus one batch.",
        ),
        EmbeddingModelEntry::borrowed(
            "text-embedding-3-small",
            "OpenAI text-embedding-3-small",
            768,
            "text",
            "Proprietary",
            "openai",
            "1",
            EmbeddingTier::Minimal,
            EmbeddingFloor::CLOUD,
            "Cloud, so no resident set. The adapter asks for 768 and OpenAI truncates from 1536.",
            "No local capacity. Needs an instance key or an org's BYOK key for openai.",
        ),
        EmbeddingModelEntry::borrowed(
            "text-embedding-3-large",
            "OpenAI text-embedding-3-large",
            768,
            "text",
            "Proprietary",
            "openai",
            "1",
            EmbeddingTier::Standard,
            EmbeddingFloor::CLOUD,
            "Cloud. 3072 native, truncated to 768 by the same `dimensions` parameter.",
            "No local capacity. Needs an instance key or an org's BYOK key for openai.",
        ),
    ]
}

/// One catalogue and the custom records it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogue {
    entries: Vec<EmbeddingModelEntry>,
    warnings: Vec<String>,
}

impl Catalogue {
    /// The shipped entries.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            entries: builtin_entries(),
            warnings: Vec::new(),
        }
    }

    /// The shipped entries plus custom ones parsed from `spec`.
    ///
    /// Record grammar, `;`-separated:
    /// `id|display|dimensions|modality|license|provider|version|tier|cores|ram_mb`.
    /// A malformed record is dropped with a warning naming it, never a refused
    /// boot: a typo in an optional lever must not stop an install that would
    /// run fine on the shipped table. An entry whose id already exists replaces
    /// the shipped one, which is how an operator pins a floor for a build they
    /// measured themselves.
    #[must_use]
    pub fn with_custom(spec: &str) -> Self {
        let mut catalogue = Self::builtin();
        for record in spec.split(';').map(str::trim).filter(|r| !r.is_empty()) {
            match parse_custom_record(record) {
                Ok(entry) => {
                    catalogue.entries.retain(|existing| existing.id != entry.id);
                    catalogue.entries.push(entry);
                }
                Err(reason) => catalogue.warnings.push(format!(
                    "{CUSTOM_MODELS_ENV} record {record:?} ignored: {reason}"
                )),
            }
        }
        catalogue
    }

    /// The catalogue as the process is configured.
    #[must_use]
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Self {
        match lookup(CUSTOM_MODELS_ENV) {
            Some(spec) if !spec.trim().is_empty() => Self::with_custom(&spec),
            _ => Self::builtin(),
        }
    }

    /// Every entry, shipped first.
    #[must_use]
    pub fn entries(&self) -> &[EmbeddingModelEntry] {
        &self.entries
    }

    /// Records dropped while parsing, for a boot warning.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// One entry by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&EmbeddingModelEntry> {
        let id = id.trim();
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// The entry a tier ships for a provider: first match wins, so the table's
    /// order is the preference order.
    #[must_use]
    pub fn for_tier(&self, tier: EmbeddingTier, provider: &str) -> Option<&EmbeddingModelEntry> {
        self.entries
            .iter()
            .find(|entry| entry.tier == tier && entry.provider == provider)
    }
}

fn parse_custom_record(record: &str) -> Result<EmbeddingModelEntry, String> {
    let fields: Vec<&str> = record.split('|').map(str::trim).collect();
    if fields.len() != 10 {
        return Err(format!(
            "expected 10 fields separated by `|`, found {}",
            fields.len()
        ));
    }
    let dimensions: u16 = fields[2]
        .parse()
        .map_err(|_| format!("dimensions {:?} is not a number", fields[2]))?;
    if dimensions != crate::SEARCH_EMBEDDING_DIM as u16 {
        return Err(format!(
            "dimensions {dimensions} cannot be stored: the vector column is {} wide",
            crate::SEARCH_EMBEDDING_DIM
        ));
    }
    let tier = EmbeddingTier::parse(fields[7]).ok_or_else(|| {
        format!(
            "tier {:?} is not one of minimal, standard, advanced",
            fields[7]
        )
    })?;
    let cores: u16 = fields[8]
        .parse()
        .map_err(|_| format!("cores {:?} is not a number", fields[8]))?;
    let ram_mb: u32 = fields[9]
        .parse()
        .map_err(|_| format!("ram_mb {:?} is not a number", fields[9]))?;
    for (name, value) in [
        ("id", fields[0]),
        ("display", fields[1]),
        ("modality", fields[3]),
        ("license", fields[4]),
        ("provider", fields[5]),
    ] {
        if value.is_empty() {
            return Err(format!("{name} is empty"));
        }
    }
    Ok(EmbeddingModelEntry {
        id: Cow::Owned(fields[0].to_string()),
        display_name: Cow::Owned(fields[1].to_string()),
        dimensions,
        modality: Cow::Owned(fields[3].to_string()),
        license: Cow::Owned(fields[4].to_string()),
        provider: Cow::Owned(fields[5].to_string()),
        version: Cow::Owned(if fields[6].is_empty() {
            CUSTOM_VERSION.to_string()
        } else {
            fields[6].to_string()
        }),
        tier,
        floor: EmbeddingFloor { cores, ram_mb },
        quality_note: Cow::Owned("Operator-supplied catalogue entry.".to_string()),
        platform_note: Cow::Owned(
            "Not measured by this build: the floor is the operator's.".to_string(),
        ),
    })
}

// ---------------------------------------------------------------------------
// Space tag
// ---------------------------------------------------------------------------

/// The vector space one model build writes into.
///
/// Vectors from two models are not comparable, and comparing them does not
/// fail loudly: it returns a plausible ranking made of nothing. So the tag is
/// stored on every vector and every retrieval filters on it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct EmbeddingSpace {
    pub model: String,
    pub version: String,
    pub dimensions: u16,
}

impl EmbeddingSpace {
    /// Builds a space from its parts.
    #[must_use]
    pub fn new(model: impl Into<String>, version: impl Into<String>, dimensions: u16) -> Self {
        Self {
            model: model.into(),
            version: version.into(),
            dimensions,
        }
    }

    /// The stored tag: `model@version/dimensions`.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}@{}/{}", self.model, self.version, self.dimensions)
    }

    /// Reads a stored tag. `None` for anything this build did not write.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let tag = tag.trim();
        let (head, dimensions) = tag.rsplit_once('/')?;
        let (model, version) = head.rsplit_once('@')?;
        if model.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            model: model.to_string(),
            version: version.to_string(),
            dimensions: dimensions.parse().ok()?,
        })
    }

    /// Whether two tags name the same model (any build).
    #[must_use]
    pub fn same_model(&self, other: &Self) -> bool {
        self.model == other.model
    }

    /// Whether this tag's build was never recorded — the population that
    /// predates the tag.
    #[must_use]
    pub fn is_unversioned(&self) -> bool {
        self.version == UNVERSIONED
    }
}

/// The tag a stored vector carries when its model is known but its build is
/// not: every row written before migration 0152.
#[must_use]
pub fn unversioned_tag(model: &str, dimensions: u16) -> String {
    EmbeddingSpace::new(model, UNVERSIONED, dimensions).id()
}

/// The space a model id belongs to, from the catalogue's point of view.
///
/// A caller that supplies precomputed vectors (the RAG surface accepts them for
/// media chunks) names a model id and nothing else. It gets the catalogue's
/// version for that id, or [`CUSTOM_VERSION`] for an id the table does not know:
/// core records the build it can justify, not one the caller asserted.
#[must_use]
pub fn space_for_model(catalogue: &Catalogue, model: &str) -> EmbeddingSpace {
    match catalogue.find(model) {
        Some(entry) => entry.space(),
        None => EmbeddingSpace::new(
            model.trim(),
            CUSTOM_VERSION,
            crate::SEARCH_EMBEDDING_DIM as u16,
        ),
    }
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// One resolved instance selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceEmbedding {
    pub entry: EmbeddingModelEntry,
    pub provider: String,
    pub space: EmbeddingSpace,
    /// True when the model came from [`MODEL_ENV`] rather than the tier.
    pub model_from_lever: bool,
}

impl InstanceEmbedding {
    /// The space id, for callers that only want the tag.
    #[must_use]
    pub fn space_id(&self) -> String {
        self.space.id()
    }

    /// Whether the model runs on this host (and therefore has a floor).
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.entry.floor.is_local()
    }
}

/// A selection that cannot be resolved, named precisely enough to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// The tier lever is not one of the three names.
    UnknownTier { value: String },
    /// The provider has no entry in that tier.
    NoModelForTier {
        tier: EmbeddingTier,
        provider: String,
        available: Vec<String>,
    },
    /// The model lever names an id that no catalogue entry covers and the tier
    /// lever is unusable as a fallback.
    ModelWithoutTier { model: String },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTier { value } => write!(
                f,
                "{TIER_ENV}={value:?} is not a tier: use minimal, standard or advanced"
            ),
            Self::NoModelForTier {
                tier,
                provider,
                available,
            } => write!(
                f,
                "no catalogue model for tier `{tier}` on provider `{provider}`; \
                 provider `{provider}` offers tiers [{}] — pick one of those with {TIER_ENV}, \
                 or name a model with {MODEL_ENV}",
                available.join(", ")
            ),
            Self::ModelWithoutTier { model } => write!(
                f,
                "model `{model}` is not in the catalogue and {TIER_ENV} names no tier to \
                 borrow a floor from: add a {CUSTOM_MODELS_ENV} record for it"
            ),
        }
    }
}

impl std::error::Error for SelectionError {}

/// Resolves the instance's model from the levers.
///
/// Order of precedence: [`MODEL_ENV`] (an explicit model, R13's lever, now
/// validated against the catalogue), then the tier's entry for the provider,
/// then the tier's local entry. `provider` defaults to `ollama` because the
/// shipped path is local: it needs no per-org key, so every org reaches it.
pub fn select_instance(
    catalogue: &Catalogue,
    tier: Option<&str>,
    model: Option<&str>,
    provider: Option<&str>,
) -> Result<InstanceEmbedding, SelectionError> {
    let tier = match tier.map(str::trim).filter(|t| !t.is_empty()) {
        Some(value) => EmbeddingTier::parse(value).ok_or_else(|| SelectionError::UnknownTier {
            value: value.to_string(),
        })?,
        None => EmbeddingTier::Standard,
    };
    let requested_provider = provider
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.to_ascii_lowercase());

    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(match catalogue.find(model) {
            Some(entry) => InstanceEmbedding {
                provider: requested_provider
                    .clone()
                    .unwrap_or_else(|| entry.provider.to_string()),
                space: entry.space(),
                entry: entry.clone(),
                model_from_lever: true,
            },
            None => {
                // An operator pinned a model this build has no entry for. It
                // is still selectable — the tier supplies the floor and the
                // entry records that its build is the operator's, not ours.
                let entry = EmbeddingModelEntry {
                    id: Cow::Owned(model.to_string()),
                    display_name: Cow::Owned(model.to_string()),
                    dimensions: crate::SEARCH_EMBEDDING_DIM as u16,
                    modality: Cow::Borrowed("text"),
                    license: Cow::Borrowed("unknown"),
                    provider: Cow::Owned(
                        requested_provider
                            .clone()
                            .unwrap_or_else(|| "ollama".to_string()),
                    ),
                    version: Cow::Borrowed(CUSTOM_VERSION),
                    tier,
                    floor: tier_default_floor(tier),
                    quality_note: Cow::Borrowed(
                        "Not in this build's catalogue: the floor is the tier's, not a \
                         measurement of this model.",
                    ),
                    platform_note: Cow::Borrowed(
                        "An operator-pinned model. Nothing here was measured on it.",
                    ),
                };
                InstanceEmbedding {
                    provider: entry.provider.to_string(),
                    space: entry.space(),
                    entry,
                    model_from_lever: true,
                }
            }
        });
    }

    let provider = requested_provider.unwrap_or_else(|| "ollama".to_string());
    let Some(entry) = catalogue.for_tier(tier, &provider) else {
        let mut available: Vec<String> = catalogue
            .entries()
            .iter()
            .filter(|entry| entry.provider == provider)
            .map(|entry| entry.tier.as_str().to_string())
            .collect();
        available.sort_unstable();
        available.dedup();
        return Err(SelectionError::NoModelForTier {
            tier,
            provider,
            available,
        });
    };
    Ok(InstanceEmbedding {
        provider,
        space: entry.space(),
        entry: entry.clone(),
        model_from_lever: false,
    })
}

fn tier_default_floor(tier: EmbeddingTier) -> EmbeddingFloor {
    match tier {
        // The smallest measured local floor, for a model nobody measured.
        EmbeddingTier::Minimal => EmbeddingFloor {
            cores: 2,
            ram_mb: 512,
        },
        EmbeddingTier::Standard => EmbeddingFloor {
            cores: 4,
            ram_mb: 768,
        },
        EmbeddingTier::Advanced => EmbeddingFloor {
            cores: 4,
            ram_mb: 2048,
        },
    }
}

// ---------------------------------------------------------------------------
// Host capacity and the floor refusal
// ---------------------------------------------------------------------------

/// What the machine offers, as measured or as declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct HostCapacity {
    pub cores: u16,
    /// `None` when the platform publishes neither a cgroup limit nor
    /// `/proc/meminfo`. Unknown is not zero: the floor check then reports what
    /// it could measure rather than refusing on a guess.
    pub ram_mb: Option<u32>,
}

impl HostCapacity {
    /// A capacity for tests and for a caller that measured its own host.
    #[must_use]
    pub fn new(cores: u16, ram_mb: Option<u32>) -> Self {
        Self { cores, ram_mb }
    }
}

/// Reads the host's capacity.
///
/// Cores come from [`std::thread::available_parallelism`]. Memory comes from
/// the declared lever first, then cgroup v2's `memory.max` (the container's own
/// limit), then `/proc/meminfo`. On a platform with none of those, memory stays
/// `None`.
#[must_use]
pub fn detect_host_capacity(lookup: &impl Fn(&str) -> Option<String>) -> HostCapacity {
    let cores = std::thread::available_parallelism()
        .map(|n| u16::try_from(n.get()).unwrap_or(u16::MAX))
        .unwrap_or(1);
    let declared = lookup(HOST_MEMORY_ENV)
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|mb| *mb > 0);
    let ram_mb = declared
        .or_else(read_cgroup_memory_mb)
        .or_else(read_proc_meminfo_mb)
        .or_else(read_platform_memory_mb);
    HostCapacity { cores, ram_mb }
}

fn read_cgroup_memory_mb() -> Option<u32> {
    let raw = std::fs::read_to_string("/sys/fs/cgroup/memory.max").ok()?;
    let trimmed = raw.trim();
    if trimmed == "max" {
        return None;
    }
    let bytes: u64 = trimmed.parse().ok()?;
    Some(u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX))
}

fn read_proc_meminfo_mb() -> Option<u32> {
    let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = raw.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(u32::try_from(kb / 1024).unwrap_or(u32::MAX))
}

/// Bytes to mebibytes, refusing a reading no machine could have.
///
/// The ceiling is 4 TiB: a parse that picked up a timestamp or a byte count from
/// the wrong column would otherwise become a host that passes every floor, which
/// is worse than a host the operator has to declare.
///
/// Only the macOS and Windows readers call it; on Linux the cgroup and
/// `/proc/meminfo` path never needs the conversion, so the lib build there has
/// no caller and `-D warnings` would refuse it. The unit test below still
/// exercises it on every platform.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn bytes_to_mb(bytes: u64) -> Option<u32> {
    const CEILING_MB: u64 = 4 * 1024 * 1024;
    let mb = bytes / (1024 * 1024);
    if mb == 0 || mb > CEILING_MB {
        return None;
    }
    u32::try_from(mb).ok()
}

/// The platform reading, for the two platforms that have no `/proc`.
///
/// Purpose: until Wave 60 the function below read cgroups and `/proc/meminfo`,
/// which exist on Linux only, so macOS and Windows reported no memory at all.
/// The installer then refused every profile on a floor it could not measure, and
/// the desktop app — the platform this product ships for — could not recommend
/// anything. Linux keeps its cheap reads; these two ask the OS once.
///
/// Non-purpose: no dependency. `sysctl` ships with macOS and PowerShell ships
/// with Windows, and a crate that reads the same value would be a heavier
/// replacement for one command.
#[cfg(target_os = "macos")]
fn read_platform_memory_mb() -> Option<u32> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    bytes_to_mb(text.trim().parse().ok()?)
}

/// See [`read_platform_memory_mb`] for macOS. Windows answers in bytes through
/// the CIM instance, which is present on every supported release; `wmic` is not
/// (it is removed from current Windows 11 images).
#[cfg(target_os = "windows")]
fn read_platform_memory_mb() -> Option<u32> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    bytes_to_mb(text.trim().parse().ok()?)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_platform_memory_mb() -> Option<u32> {
    None
}

/// The floor check's failure: every number in the message is a field, so the
/// CLI, the boot path and the tests all read the same values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorRefusal {
    pub model: String,
    pub tier: EmbeddingTier,
    pub required: EmbeddingFloor,
    pub available: HostCapacity,
}

impl fmt::Display for FloorRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let memory = match self.available.ram_mb {
            Some(mb) => format!("{mb} MB"),
            None => "an unknown amount of memory".to_string(),
        };
        write!(
            f,
            "embedding model `{}` needs at least {} cores and {} MB of RAM (tier `{}`); this \
             host offers {} cores and {memory} — give the host more capacity, select a smaller \
             tier with {TIER_ENV} (minimal, standard), or run the model on a cloud provider \
             with {PROVIDER_ENV}=openai",
            self.model, self.required.cores, self.required.ram_mb, self.tier, self.available.cores
        )
    }
}

impl std::error::Error for FloorRefusal {}

/// Refuses a selection this host cannot host.
///
/// Cores are always checked. Memory is checked only when it was measured: a
/// platform that publishes no cgroup limit and no `/proc/meminfo` yields
/// `None`, and refusing on an unknown would take a working install down.
pub fn check_floor(selection: &InstanceEmbedding, host: HostCapacity) -> Result<(), FloorRefusal> {
    let required = selection.entry.floor;
    let cores_short = required.cores > host.cores;
    let memory_short = match host.ram_mb {
        Some(mb) => required.ram_mb > mb,
        None => false,
    };
    if !cores_short && !memory_short {
        return Ok(());
    }
    Err(FloorRefusal {
        model: selection.entry.id.to_string(),
        tier: selection.entry.tier,
        required,
        available: host,
    })
}

/// Every way resolving the instance selection can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceError {
    Selection(SelectionError),
    Floor(FloorRefusal),
}

impl fmt::Display for InstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => error.fmt(f),
            Self::Floor(refusal) => refusal.fmt(f),
        }
    }
}

impl std::error::Error for InstanceError {}

/// Resolves the instance selection from the process environment and proves the
/// host can host it. This is the function `core-api` and the worker call at
/// boot; the CLI calls it to describe what would run.
pub fn resolve_instance(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<(InstanceEmbedding, HostCapacity), InstanceError> {
    let (_, selection, host) = resolve_instance_with_catalogue(lookup)?;
    Ok((selection, host))
}

/// [`resolve_instance`], keeping the catalogue so a caller can log the custom
/// records it refused. A dropped record is exactly the case where the boot log
/// has to say why the model the operator named is not in the table.
pub fn resolve_instance_with_catalogue(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<(Catalogue, InstanceEmbedding, HostCapacity), InstanceError> {
    let catalogue = Catalogue::from_lookup(lookup);
    let selection = select_instance(
        &catalogue,
        lookup(TIER_ENV).as_deref(),
        lookup(MODEL_ENV).as_deref(),
        lookup(PROVIDER_ENV).as_deref(),
    )
    .map_err(InstanceError::Selection)?;
    let host = detect_host_capacity(lookup);
    check_floor(&selection, host).map_err(InstanceError::Floor)?;
    Ok((catalogue, selection, host))
}

// ---------------------------------------------------------------------------
// Re-index policy
// ---------------------------------------------------------------------------

/// Rows per core-api call by default: one embedding batch. The request budget
/// and the batch size are the same number the backfill has always used.
pub const REINDEX_BATCH_DEFAULT: i64 = 128;
/// Rows per minute by default. R13 measured 16.0 chunks/s (960/min) for the
/// text-only default, so 600 leaves about a third of the capacity for live
/// search traffic.
pub const REINDEX_ROWS_PER_MIN_DEFAULT: u32 = 600;
/// How often an idle sweep looks again. A tick with work is paced by
/// [`ReindexPolicy::pace`] instead, so this is the caught-up cadence, not the
/// job's throughput.
pub const REINDEX_INTERVAL_SECS_DEFAULT: u64 = 60;

/// Levers and pacing for the re-index sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReindexPolicy {
    pub batch: i64,
    pub rows_per_min: u32,
    pub interval_secs: u64,
    /// Inclusive start hour and exclusive end hour, UTC. `None` means the job
    /// may run any hour.
    pub window: Option<(u8, u8)>,
}

impl Default for ReindexPolicy {
    fn default() -> Self {
        Self {
            batch: REINDEX_BATCH_DEFAULT,
            rows_per_min: REINDEX_ROWS_PER_MIN_DEFAULT,
            interval_secs: REINDEX_INTERVAL_SECS_DEFAULT,
            window: None,
        }
    }
}

impl ReindexPolicy {
    /// Reads the levers. A zero or unparsable value falls back to the default:
    /// the re-index is repair work, and a typo must not stop it.
    #[must_use]
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Self {
        let default = Self::default();
        Self {
            batch: lookup("MARULA_EMBEDDING_REINDEX_BATCH")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|b| *b > 0)
                .unwrap_or(default.batch),
            rows_per_min: lookup("MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|r| *r > 0)
                .unwrap_or(default.rows_per_min),
            interval_secs: lookup("MARULA_EMBEDDING_REINDEX_SECS")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(default.interval_secs),
            window: lookup("MARULA_EMBEDDING_REINDEX_WINDOW")
                .as_deref()
                .and_then(parse_window),
        }
    }

    /// How long one batch must be followed by a pause to stay under the
    /// per-minute ceiling.
    #[must_use]
    pub fn pace(&self) -> std::time::Duration {
        let rows = u64::try_from(self.batch).unwrap_or(1).max(1);
        let per_min = u64::from(self.rows_per_min.max(1));
        std::time::Duration::from_millis(rows.saturating_mul(60_000) / per_min)
    }

    /// Whether the sweep may work at this UTC hour.
    #[must_use]
    pub fn allows_hour(&self, hour: u8) -> bool {
        match self.window {
            None => true,
            Some((start, end)) if start == end => true,
            // A window that wraps midnight (22-06) is the common case for an
            // overnight rebuild, so wrapping is the rule rather than an error.
            Some((start, end)) if start < end => hour >= start && hour < end,
            Some((start, end)) => hour >= start || hour < end,
        }
    }
}

/// Parses `HH-HH`, both hours `0..=23`.
#[must_use]
pub fn parse_window(value: &str) -> Option<(u8, u8)> {
    let (start, end) = value.trim().split_once('-')?;
    let start: u8 = start.trim().parse().ok()?;
    let end: u8 = end.trim().parse().ok()?;
    if start > 23 || end > 23 {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_from(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    #[test]
    fn the_shipped_default_is_embeddinggemma_text_only_on_the_local_path() {
        let catalogue = Catalogue::builtin();
        let selection = select_instance(&catalogue, None, None, None).expect("default resolves");
        assert_eq!(selection.entry.id, "embeddinggemma-2:270m-q8_0-text");
        assert_eq!(selection.provider, "ollama");
        assert_eq!(
            selection.space_id(),
            "embeddinggemma-2:270m-q8_0-text@1/768"
        );
        assert!(selection.is_local());
        assert!(!selection.model_from_lever);
    }

    #[test]
    fn the_tier_moves_the_model_and_the_floor_with_it() {
        let catalogue = Catalogue::builtin();
        let minimal = select_instance(&catalogue, Some("minimal"), None, None).expect("minimal");
        assert_eq!(minimal.entry.tier, EmbeddingTier::Minimal);
        assert_eq!(minimal.entry.floor.cores, 2);

        let advanced = select_instance(&catalogue, Some("advanced"), None, None).expect("advanced");
        assert_eq!(advanced.entry.id, "embeddinggemma-2:740m-q8_0");
        assert_eq!(advanced.entry.floor.ram_mb, 2048);
        assert_eq!(advanced.entry.modality, "text, image, audio");
    }

    #[test]
    fn a_bad_tier_is_refused_by_name() {
        let catalogue = Catalogue::builtin();
        let error = select_instance(&catalogue, Some("huge"), None, None).expect_err("refused");
        assert_eq!(
            error,
            SelectionError::UnknownTier {
                value: "huge".to_string()
            }
        );
        assert!(error.to_string().contains("minimal, standard or advanced"));
    }

    #[test]
    fn the_model_lever_wins_over_the_tier_and_keeps_the_entrys_tier() {
        let catalogue = Catalogue::builtin();
        let selection = select_instance(
            &catalogue,
            Some("advanced"),
            Some("embeddinggemma-2:270m-bf16-text"),
            None,
        )
        .expect("resolves");
        assert_eq!(selection.entry.id, "embeddinggemma-2:270m-bf16-text");
        assert_eq!(
            selection.entry.tier,
            EmbeddingTier::Standard,
            "the entry's class wins over the requested tier"
        );
        assert!(selection.model_from_lever);
        assert_eq!(selection.space.version, "1");
    }

    #[test]
    fn an_unknown_model_is_selectable_and_says_its_build_is_the_operators() {
        let catalogue = Catalogue::builtin();
        let selection = select_instance(&catalogue, Some("minimal"), Some("my-embed:1b"), None)
            .expect("custom model resolves");
        assert_eq!(selection.entry.version, CUSTOM_VERSION);
        assert_eq!(selection.entry.floor.cores, 2, "tier floor is the fallback");
        assert_eq!(selection.space_id(), "my-embed:1b@custom/768");
        assert!(selection
            .entry
            .quality_note
            .contains("Not in this build's catalogue"));
    }

    #[test]
    fn a_cloud_provider_with_no_entry_in_the_tier_is_refused_with_the_tiers_it_has() {
        let catalogue = Catalogue::builtin();
        let error = select_instance(&catalogue, Some("advanced"), None, Some("openai"))
            .expect_err("openai has no advanced entry");
        match error {
            SelectionError::NoModelForTier {
                tier, available, ..
            } => {
                assert_eq!(tier, EmbeddingTier::Advanced);
                assert_eq!(
                    available,
                    vec!["minimal".to_string(), "standard".to_string()]
                );
            }
            other => panic!("wrong refusal: {other:?}"),
        }
    }

    #[test]
    fn custom_records_are_data_and_a_bad_record_warns_instead_of_refusing_boot() {
        let catalogue = Catalogue::with_custom(
            "acme-embed:1|Acme Embed|768|text|MIT|ollama|7|minimal|2|600;\
             broken-record|only-three|fields",
        );
        assert_eq!(catalogue.warnings().len(), 1, "one record dropped");
        assert!(catalogue.warnings()[0].contains("broken-record"));
        let entry = catalogue
            .find("acme-embed:1")
            .expect("custom entry present");
        assert_eq!(entry.version, "7");
        assert_eq!(entry.floor.ram_mb, 600);
        assert_eq!(entry.space().id(), "acme-embed:1@7/768");

        let wrong_width = Catalogue::with_custom("wide|x|1024|text|MIT|ollama|1|minimal|2|512");
        assert_eq!(wrong_width.warnings().len(), 1);
        assert!(wrong_width.warnings()[0].contains("1024"));
    }

    #[test]
    fn a_custom_record_can_replace_a_shipped_entry_floor() {
        let catalogue = Catalogue::with_custom(
            "embeddinggemma-2:270m-q8_0-text|EmbeddingGemma 2 (measured)|768|text|Apache-2.0|\
             ollama|1|standard|2|640",
        );
        let entry = catalogue
            .find("embeddinggemma-2:270m-q8_0-text")
            .expect("entry");
        assert_eq!(entry.floor.cores, 2);
        assert_eq!(
            catalogue
                .entries()
                .iter()
                .filter(|e| e.id == "embeddinggemma-2:270m-q8_0-text")
                .count(),
            1,
            "replacement, not a duplicate"
        );
    }

    #[test]
    fn a_byte_reading_becomes_mebibytes_and_a_nonsense_one_is_refused() {
        assert_eq!(bytes_to_mb(16 * 1024 * 1024 * 1024), Some(16 * 1024));
        assert_eq!(bytes_to_mb(8 * 1024 * 1024 * 1024), Some(8 * 1024));
        // Under a mebibyte, or past four tebibytes, is a misread, not a machine.
        assert_eq!(bytes_to_mb(512 * 1024), None);
        assert_eq!(bytes_to_mb(8 * 1024 * 1024 * 1024 * 1024), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_its_memory_rather_than_saying_nothing() {
        let mb = read_platform_memory_mb().expect("sysctl reports hw.memsize on every Mac");
        assert!(
            mb > 1024,
            "a Mac with less than a gibibyte cannot run this product: {mb}"
        );
    }

    #[test]
    fn the_floor_refuses_a_two_core_host_and_names_every_number() {
        let catalogue = Catalogue::builtin();
        let advanced = select_instance(&catalogue, Some("advanced"), None, None).expect("advanced");
        let host = HostCapacity::new(2, Some(1024));
        let refusal = check_floor(&advanced, host).expect_err("refused");
        assert_eq!(refusal.required.cores, 4);
        assert_eq!(refusal.required.ram_mb, 2048);
        let message = refusal.to_string();
        assert!(message.contains("embeddinggemma-2:740m-q8_0"), "{message}");
        assert!(
            message.contains("at least 4 cores and 2048 MB"),
            "{message}"
        );
        assert!(message.contains("2 cores and 1024 MB"), "{message}");
        assert!(message.contains("MARULA_EMBEDDING_TIER"), "{message}");
    }

    #[test]
    fn an_unknown_memory_reading_checks_cores_only() {
        let catalogue = Catalogue::builtin();
        let advanced = select_instance(&catalogue, Some("advanced"), None, None).expect("advanced");
        assert!(check_floor(&advanced, HostCapacity::new(8, None)).is_ok());
        assert!(
            check_floor(&advanced, HostCapacity::new(2, None)).is_err(),
            "cores are always measurable, so they are always checked"
        );
    }

    #[test]
    fn a_cloud_model_has_no_local_floor() {
        let catalogue = Catalogue::builtin();
        let cloud = select_instance(&catalogue, Some("standard"), None, Some("openai"))
            .expect("cloud standard");
        assert_eq!(cloud.entry.id, "text-embedding-3-large");
        assert!(!cloud.is_local());
        assert!(check_floor(&cloud, HostCapacity::new(1, Some(64))).is_ok());
    }

    #[test]
    fn the_declared_memory_lever_beats_detection() {
        let declared = lookup_from(&[(HOST_MEMORY_ENV, "1234")]);
        let host = detect_host_capacity(&declared);
        assert_eq!(host.ram_mb, Some(1234));
        assert!(host.cores >= 1);
    }

    #[test]
    fn resolve_instance_ties_selection_to_the_floor() {
        let levers = lookup_from(&[(TIER_ENV, "advanced"), (HOST_MEMORY_ENV, "512")]);
        let error = resolve_instance(&levers).expect_err("512 MB cannot host the multimodal build");
        match error {
            InstanceError::Floor(refusal) => {
                assert_eq!(refusal.tier, EmbeddingTier::Advanced);
                assert_eq!(refusal.required.ram_mb, 2048);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn a_space_tag_round_trips_and_refuses_garbage() {
        let space = EmbeddingSpace::new("model:x", "3", 768);
        assert_eq!(space.id(), "model:x@3/768");
        assert_eq!(EmbeddingSpace::parse("model:x@3/768"), Some(space.clone()));
        assert!(EmbeddingSpace::parse("model:x").is_none());
        assert!(EmbeddingSpace::parse("model:x@3").is_none());
        assert!(EmbeddingSpace::parse("@3/768").is_none());
        assert_eq!(
            unversioned_tag("nomic-embed-text", 768),
            "nomic-embed-text@unversioned/768"
        );
        assert!(EmbeddingSpace::parse("nomic-embed-text@unversioned/768")
            .expect("parses")
            .is_unversioned());
    }

    #[test]
    fn the_pace_keeps_one_batch_under_the_per_minute_ceiling() {
        let policy = ReindexPolicy {
            batch: 128,
            rows_per_min: 600,
            ..ReindexPolicy::default()
        };
        assert_eq!(policy.pace(), std::time::Duration::from_millis(12_800));
        let default = ReindexPolicy::default();
        assert_eq!(default.pace(), std::time::Duration::from_millis(12_800));
    }

    #[test]
    fn an_overnight_window_wraps_midnight() {
        let policy = ReindexPolicy {
            window: parse_window("22-06"),
            ..ReindexPolicy::default()
        };
        assert!(policy.allows_hour(23));
        assert!(policy.allows_hour(2));
        assert!(!policy.allows_hour(6));
        assert!(!policy.allows_hour(12));
        let day = ReindexPolicy {
            window: parse_window("09-17"),
            ..ReindexPolicy::default()
        };
        assert!(day.allows_hour(9));
        assert!(!day.allows_hour(17));
        assert!(ReindexPolicy::default().allows_hour(3));
        assert_eq!(parse_window("22-24"), None, "24 is not an hour");
    }

    #[test]
    fn reindex_levers_fall_back_on_nonsense() {
        let levers = lookup_from(&[
            ("MARULA_EMBEDDING_REINDEX_BATCH", "0"),
            ("MARULA_EMBEDDING_REINDEX_ROWS_PER_MIN", "abc"),
            ("MARULA_EMBEDDING_REINDEX_SECS", "0"),
            ("MARULA_EMBEDDING_REINDEX_WINDOW", "not-a-window"),
        ]);
        let policy = ReindexPolicy::from_lookup(&levers);
        assert_eq!(policy, ReindexPolicy::default());
    }
}
