//! The retrieval mode: a deployment decision about how the vector leg answers
//! (Wave 52 lane R15, `p52-retrieval-mode-config`).
//!
//! The search vector leg has one exact answer and two indexed shortcuts. Which
//! one a deployment should run depends on its shape, and the shapes pull in
//! opposite directions:
//!
//! - **Many small tenants on one machine** want the exact scan. P10 measured
//!   0.48 ms at recall 1.000 for a tenant holding 0.72% of the table, and the
//!   scan costs no index memory. Every indexed arm measured at that shape is
//!   9 to 62 times slower.
//! - **One big tenant on its own deployment** wants an index. The exact scan
//!   crosses the vector leg's database share at roughly 20,000 embeddings in a
//!   single tenant, and VectorChord's prefilter returns every row at recall
//!   1.000 in 1.35 ms there.
//!
//! So the mode is a lever, not a code default that quietly suits one of them.
//! `exact` is the default because it is the only mode whose result set is the
//! true nearest neighbours and the only one that is always satisfiable.
//!
//! Two rules this module exists to enforce:
//!
//! 1. **No mode silently differs from its name.** `exact` cannot use an ANN
//!    index ([`crate::retrieval::RetrievalMode::Exact`] is applied to the
//!    statement in `core-db`, which neutralises the ordering operator).
//! 2. **A mode that cannot run refuses, with the requirement named.** There is
//!    no fallback to the exact scan. That fallback is invisible, and it reads
//!    to an operator as search working badly rather than as the deployment
//!    being misconfigured.
//!
//! Satisfaction is a pure check over [`DatabaseCapabilities`], which `core-db`
//! reads from the connected database. Boot and `marula deploy describe` call
//! the same `check`, so the sentence an operator reads in the CLI is the one
//! the process would print at startup.

use std::fmt;

/// The environment lever that selects the mode.
pub const MODE_ENV: &str = "MARULA_RETRIEVAL_MODE";

/// Embeddings in one tenant at which the exact scan stops fitting the vector
/// leg's database share on the reference profiles (P10: 2.3 to 2.7
/// microseconds per embedding, 50 ms leg share). The distribution output marks
/// tenants at or past this number.
pub const INDEX_THRESHOLD_EMBEDDINGS: u64 = 20_000;

/// The minimum pgvector release with iterative index scans
/// (`hnsw.iterative_scan`). Managed instances on 0.8 can run this mode even
/// though they cannot install VectorChord.
pub const MIN_PGVECTOR_ITERATIVE: &str = "0.8";

/// The extension `index` mode needs, and the setting it must find preloaded.
pub const VCHORD_EXTENSION: &str = "vchord";

/// The index names [`crate::retrieval`]'s migrations create. The names are part
/// of the contract because the capability probe looks for them: a mode whose
/// index is missing is unsatisfiable, not an exact scan with a log line.
pub const HNSW_INDEX: &str = "search_embeddings_hnsw";
/// The VectorChord index, created only on a database that carries the
/// extension.
pub const VCHORD_INDEX: &str = "search_embeddings_vchord";

/// How the vector leg answers a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    /// Sequential scan, ordered by cosine distance. The true nearest
    /// neighbours, and the only mode with no requirement.
    #[default]
    Exact,
    /// pgvector HNSW with an iterative scan: the graph walk continues until the
    /// tenant and space filters have produced enough rows. Approximate recall,
    /// available on a managed pgvector 0.8 instance.
    Iterative,
    /// VectorChord's `vchordrq` with prefiltering: the filter runs inside the
    /// index, so a small tenant gets its rows back at full recall.
    Index,
}

impl RetrievalMode {
    /// Every mode, in the order the descriptor lists them.
    pub const ALL: [Self; 3] = [Self::Exact, Self::Iterative, Self::Index];

    /// The token an operator sets in [`MODE_ENV`].
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Iterative => "iterative",
            Self::Index => "index",
        }
    }

    /// Whether this is the default a deployment gets when it sets nothing.
    #[must_use]
    pub fn is_default(self) -> bool {
        self == Self::Exact
    }

    /// Parse a lever value. An unknown token is a refusal that lists the valid
    /// tokens: accepting it as `exact` would make a typo look like a working
    /// deployment.
    ///
    /// # Errors
    ///
    /// Returns [`ModeError::Unknown`] for any token outside [`Self::ALL`].
    pub fn parse(value: &str) -> Result<Self, ModeError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "exact" => Ok(Self::Exact),
            "iterative" => Ok(Self::Iterative),
            "index" => Ok(Self::Index),
            other => Err(ModeError::Unknown {
                value: other.to_string(),
            }),
        }
    }

    /// Resolve the mode from an environment lookup. Unset means [`Self::Exact`].
    ///
    /// # Errors
    ///
    /// Returns [`ModeError::Unknown`] when the lever names no mode.
    pub fn from_lookup(lookup: &impl Fn(&str) -> Option<String>) -> Result<Self, ModeError> {
        match lookup(MODE_ENV) {
            Some(value) => Self::parse(&value),
            None => Ok(Self::Exact),
        }
    }

    /// What this mode needs from the database, in operator words. Empty for
    /// `exact`.
    #[must_use]
    pub fn requires(self) -> &'static [&'static str] {
        match self {
            Self::Exact => &[],
            Self::Iterative => &[
                "pgvector 0.8 or newer (iterative index scans)",
                "the HNSW index `search_embeddings_hnsw`",
            ],
            Self::Index => &[
                "the `vchord` extension (VectorChord 0.5)",
                "`vchord` in `shared_preload_libraries`",
                "the VectorChord index `search_embeddings_vchord`",
            ],
        }
    }

    /// The measured consequence, in the words the descriptor prints. The
    /// numbers are this lane's run of `scripts/vector-ann-bench.sh` on the
    /// product's own embedding model (18,247 documents, 40 iterations, p95 at
    /// 4 CPUs), except where a number is labelled otherwise; P10's numbers are
    /// used only where this lane did not re-measure. [`ModeConsequence::leg_note`]
    /// carries the arithmetic that keeps them honest.
    #[must_use]
    pub fn consequence(self) -> ModeConsequence {
        match self {
            Self::Exact => ModeConsequence {
                summary: "True nearest neighbours from a sequential scan. The \
                          default, and the only mode with no requirement.",
                latency: "0.5 ms p95 at a 132-embedding tenant, 16.9 ms at 6,660 in an \
                          18k table, 224 ms at 100,000 in one tenant.",
                index_memory: "None. No index is built or kept.",
                recall: "1.000 by construction.",
                leg_note: "At 4 cores the default embedding model spends about 47 ms \
                           of the 50 ms vector leg, so the scan has roughly 3 ms before \
                           it is over budget. That happens near 1,500 embeddings in one \
                           tenant, well before the 20,000 at which the scan alone \
                           crosses 50 ms.",
                recommend_when: "Many small tenants on one machine, or any tenant below \
                                 about 20,000 embeddings.",
            },
            Self::Iterative => ModeConsequence {
                summary: "pgvector HNSW with an iterative scan, so the graph keeps \
                          walking until the tenant and space filters have enough rows. \
                          Approximate, and available on a managed pgvector 0.8 instance.",
                latency: "9.8 ms p95 at a 132-embedding tenant (0.96 recall, against \
                          exact's 0.5 ms), 1.3 ms at 6,660 (1.000 recall), about 11 ms \
                          at 100,000.",
                index_memory: "56.4 MB at 18,247 vectors, built in 5.9 s; 1,179.7 MB at \
                               100,000.",
                recall: "0.96 at a small tenant, where a plain HNSW walk at the same \
                         ef_search returns 2.6 of 10 rows; 1.000 once the filter stops \
                         discarding rows. Strict order keeps the ranking honest.",
                leg_note: "The index walk replaces the scan, so the database share shrinks \
                           below 20,000 embeddings. At 4 cores the model still spends \
                           about 47 ms of the 50 ms leg, which is the number to watch.",
                recommend_when: "A tenant past ~20,000 embeddings on a managed Postgres \
                                 that offers pgvector 0.8 but no VectorChord.",
            },
            Self::Index => ModeConsequence {
                summary: "VectorChord's prefiltered index: the tenant and space filters \
                          run inside the index, so a small tenant gets every row it asked \
                          for.",
                latency: "4.4 ms p95 at a 132-embedding tenant (still slower than exact), \
                          0.9 ms at 6,660, both at 1.000 recall. 100,000 on product \
                          vectors is not measured; see the leg note.",
                index_memory: "76.3 MB at 18,247 vectors, built in 0.9 s; 1,261.5 MB at \
                               100,000, built in 16.7 s. 4-bit quantised by extension \
                               default.",
                recall: "1.000 at every tenant size measured on product vectors, which is \
                         why this ships instead of plain HNSW.",
                leg_note: "The prefilter removes the scan from the leg entirely. The \
                           remaining budget is the model's, about 47 ms of 50 ms at 4 \
                           cores with the default embedding model. The 100,000-embedding \
                           leg is random vectors, where this index measured 419 ms p95 — \
                           a number about the generator, not about a deployment, so the \
                           scale claim stays unproven until a product-sized corpus runs.",
                recommend_when: "A per-org deployment whose tenant crossed ~20,000 \
                                 embeddings, and Postgres you control (the extension needs \
                                 shared_preload_libraries).",
            },
        }
    }
}

impl fmt::Display for RetrievalMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The measured consequence of one mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ModeConsequence {
    /// One sentence for the descriptor.
    pub summary: &'static str,
    /// Query latency at tenant sizes P10 measured.
    pub latency: &'static str,
    /// What the index costs to keep.
    pub index_memory: &'static str,
    /// Recall, stated as the number it is.
    pub recall: &'static str,
    /// The shared-budget arithmetic: model time plus database time.
    pub leg_note: &'static str,
    /// The deployment shape this mode is for.
    pub recommend_when: &'static str,
}

/// Why a mode token was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeError {
    /// The lever named no mode.
    Unknown { value: String },
}

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { value } => write!(
                f,
                "retrieval mode `{value}` is not a mode: use exact, iterative or index \
                 ({MODE_ENV}); an unknown value is refused rather than treated as exact"
            ),
        }
    }
}

impl std::error::Error for ModeError {}

/// What the connected database offers. `core-db` fills this from `pg_extension`,
/// `pg_available_extensions`, `pg_settings` and `pg_indexes`; tests fill it by
/// hand.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct DatabaseCapabilities {
    /// `vector` extension version, when installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pgvector_version: Option<String>,
    /// `vchord` is installed in this database.
    pub vchord_installed: bool,
    /// `vchord` is installable from this image (`pg_available_extensions`).
    pub vchord_available: bool,
    /// Entries of `shared_preload_libraries`, trimmed.
    pub preload_libraries: Vec<String>,
    /// Index names present on `search_embeddings`.
    pub indexes: Vec<String>,
}

impl DatabaseCapabilities {
    /// Whether a library is preloaded.
    #[must_use]
    pub fn preloads(&self, library: &str) -> bool {
        self.preload_libraries.iter().any(|l| l == library)
    }

    /// Whether an index exists by name.
    #[must_use]
    pub fn has_index(&self, name: &str) -> bool {
        self.indexes.iter().any(|i| i == name)
    }
}

/// One missing requirement, carrying the fact that was found instead.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "requirement", rename_all = "snake_case")]
pub enum Missing {
    /// pgvector is too old, or absent altogether.
    Pgvector {
        required: &'static str,
        found: Option<String>,
    },
    /// The extension is not installed.
    Extension { name: &'static str, available: bool },
    /// The extension is installed but not preloaded.
    Preload { name: &'static str },
    /// The index the mode needs is absent.
    Index { name: &'static str },
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pgvector {
                required,
                found: Some(found),
            } => write!(
                f,
                "pgvector {found} is installed and iterative index scans need \
                 {required} or newer"
            ),
            Self::Pgvector {
                required,
                found: None,
            } => write!(
                f,
                "the pgvector extension is not installed and this mode needs {required} \
                 or newer"
            ),
            Self::Extension {
                name,
                available: true,
            } => write!(
                f,
                "the `{name}` extension is available but not installed in this database"
            ),
            Self::Extension {
                name,
                available: false,
            } => write!(
                f,
                "the `{name}` extension is not installed and this Postgres image does \
                 not offer it"
            ),
            Self::Preload { name } => write!(
                f,
                "`{name}` is not in `shared_preload_libraries`, so its index cannot be \
                 maintained"
            ),
            Self::Index { name } => write!(
                f,
                "the index `{name}` does not exist on `search_embeddings`"
            ),
        }
    }
}

/// The typed refusal a boot or a descriptor prints.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RetrievalRefusal {
    /// The mode that cannot run.
    pub mode: RetrievalMode,
    /// Every missing requirement, not only the first.
    pub missing: Vec<Missing>,
}

impl fmt::Display for RetrievalRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "retrieval mode `{}` cannot run on this database: ",
            self.mode
        )?;
        for (i, missing) in self.missing.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{missing}")?;
        }
        write!(
            f,
            ". The vector leg will not fall back to the exact scan: pick a mode this \
             database can run, satisfy the requirement, or set {MODE_ENV}=exact for the \
             true nearest neighbours"
        )
    }
}

impl std::error::Error for RetrievalRefusal {}

/// Check one mode against what the database offers.
///
/// # Errors
///
/// Returns [`RetrievalRefusal`] naming every requirement the database does not
/// meet. It never falls back to a satisfiable mode.
pub fn check(mode: RetrievalMode, caps: &DatabaseCapabilities) -> Result<(), RetrievalRefusal> {
    let mut missing = Vec::new();
    match mode {
        RetrievalMode::Exact => {}
        RetrievalMode::Iterative => {
            if !caps
                .pgvector_version
                .as_deref()
                .is_some_and(|found| pgvector_at_least(found, MIN_PGVECTOR_ITERATIVE))
            {
                missing.push(Missing::Pgvector {
                    required: MIN_PGVECTOR_ITERATIVE,
                    found: caps.pgvector_version.clone(),
                });
            }
            if !caps.has_index(HNSW_INDEX) {
                missing.push(Missing::Index { name: HNSW_INDEX });
            }
        }
        RetrievalMode::Index => {
            if !caps.vchord_installed {
                missing.push(Missing::Extension {
                    name: VCHORD_EXTENSION,
                    available: caps.vchord_available,
                });
            }
            if !caps.preloads(VCHORD_EXTENSION) {
                missing.push(Missing::Preload {
                    name: VCHORD_EXTENSION,
                });
            }
            if !caps.has_index(VCHORD_INDEX) {
                missing.push(Missing::Index { name: VCHORD_INDEX });
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(RetrievalRefusal { mode, missing })
    }
}

/// Whether an extension version string is at least `required`. Accepts the
/// forms `pg_extension.extversion` produces (`0.8.0`, `0.8.6`) and ignores a
/// prerelease suffix the way a floor comparison should.
#[must_use]
pub fn pgvector_at_least(found: &str, required: &str) -> bool {
    fn parts(value: &str) -> Option<Vec<u32>> {
        let core = value.trim().split(['-', '+']).next().unwrap_or_default();
        let parsed: Option<Vec<u32>> = core
            .split('.')
            .map(|part| part.parse::<u32>().ok())
            .collect();
        parsed.filter(|v| !v.is_empty())
    }
    let (Some(found), Some(required)) = (parts(found), parts(required)) else {
        return false;
    };
    for i in 0..found.len().max(required.len()) {
        let a = found.get(i).copied().unwrap_or(0);
        let b = required.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    true
}

/// Whether a tenant's embedding count is at or past the threshold at which the
/// exact scan stops fitting (see [`INDEX_THRESHOLD_EMBEDDINGS`]).
#[must_use]
pub fn index_recommended(embeddings: u64) -> bool {
    embeddings >= INDEX_THRESHOLD_EMBEDDINGS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(
        version: &str,
        preload: &[&str],
        indexes: &[&str],
        vchord: bool,
    ) -> DatabaseCapabilities {
        DatabaseCapabilities {
            pgvector_version: Some(version.to_string()),
            vchord_installed: vchord,
            vchord_available: vchord,
            preload_libraries: preload.iter().map(ToString::to_string).collect(),
            indexes: indexes.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn exact_is_the_default_and_needs_nothing() {
        assert_eq!(RetrievalMode::default(), RetrievalMode::Exact);
        assert_eq!(
            RetrievalMode::from_lookup(&|_| None).expect("unset is exact"),
            RetrievalMode::Exact
        );
        // A stock pgvector database with no index satisfies the default.
        let stock = caps("0.8.6", &[], &[], false);
        assert!(check(RetrievalMode::Exact, &stock).is_ok());
        // And so does a database with nothing at all.
        assert!(check(RetrievalMode::Exact, &DatabaseCapabilities::default()).is_ok());
        assert!(RetrievalMode::Exact.requires().is_empty());
    }

    #[test]
    fn an_unknown_mode_is_refused_rather_than_treated_as_exact() {
        let error = RetrievalMode::parse("ann").expect_err("must refuse");
        let message = error.to_string();
        assert!(message.contains("`ann` is not a mode"), "{message}");
        assert!(message.contains("exact, iterative or index"), "{message}");
        assert!(message.contains(MODE_ENV), "{message}");
        let from_env =
            RetrievalMode::from_lookup(&|key| (key == MODE_ENV).then(|| "ANN".to_string()))
                .expect_err("case-insensitive token still unknown");
        assert!(from_env.to_string().contains("`ann`"));
        assert_eq!(
            RetrievalMode::parse(" Iterative ").expect("trimmed"),
            RetrievalMode::Iterative
        );
    }

    #[test]
    fn iterative_refuses_on_old_pgvector_and_names_the_version() {
        let old = caps("0.7.4", &[], &[HNSW_INDEX], false);
        let refusal = check(RetrievalMode::Iterative, &old).expect_err("0.7 must refuse");
        assert_eq!(refusal.mode, RetrievalMode::Iterative);
        assert_eq!(
            refusal.missing,
            vec![Missing::Pgvector {
                required: "0.8",
                found: Some("0.7.4".to_string()),
            }]
        );
        let message = refusal.to_string();
        assert!(message.contains("pgvector 0.7.4"), "{message}");
        assert!(message.contains("0.8 or newer"), "{message}");
        assert!(
            message.contains("will not fall back to the exact scan"),
            "{message}"
        );

        // Absent entirely is also a refusal, and says so.
        let none = DatabaseCapabilities {
            pgvector_version: None,
            ..DatabaseCapabilities::default()
        };
        let refusal = check(RetrievalMode::Iterative, &none).expect_err("no vector");
        assert!(refusal.to_string().contains("not installed"));
    }

    #[test]
    fn iterative_needs_the_hnsw_index_as_well_as_the_version() {
        let version_only = caps("0.8.6", &[], &[], false);
        let refusal = check(RetrievalMode::Iterative, &version_only).expect_err("no index");
        assert_eq!(refusal.missing, vec![Missing::Index { name: HNSW_INDEX }]);
        let ready = caps("0.8.6", &[], &[HNSW_INDEX], false);
        assert!(check(RetrievalMode::Iterative, &ready).is_ok());
    }

    #[test]
    fn index_refuses_without_the_extension_the_preload_and_the_index() {
        let stock = caps("0.8.6", &[], &[], false);
        let refusal = check(RetrievalMode::Index, &stock).expect_err("stock refuses");
        assert!(matches!(
            refusal.missing.first(),
            Some(Missing::Extension {
                available: false,
                ..
            })
        ));
        let message = refusal.to_string();
        assert!(message.contains("`vchord` extension"), "{message}");
        assert!(message.contains("does not offer it"), "{message}");
        assert!(
            message.contains("will not fall back to the exact scan"),
            "{message}"
        );

        // Installed but not preloaded: the preload is the requirement a managed
        // instance cannot satisfy, and the message says exactly that.
        let installed = caps("0.8.6", &[], &[VCHORD_INDEX], true);
        let refusal = check(RetrievalMode::Index, &installed).expect_err("no preload");
        assert_eq!(
            refusal.missing,
            vec![Missing::Preload {
                name: VCHORD_EXTENSION
            }]
        );
        assert!(
            refusal.to_string().contains("shared_preload_libraries"),
            "{refusal}"
        );

        // Everything but the index: still a refusal, because an index mode that
        // quietly ran the exact scan is the failure this lane removes.
        let built = caps("0.8.6", &["pg_cron", "vchord"], &[], true);
        let refusal = check(RetrievalMode::Index, &built).expect_err("no index");
        assert_eq!(refusal.missing, vec![Missing::Index { name: VCHORD_INDEX }]);
        assert!(refusal.to_string().contains("does not exist"));

        let ready = caps("0.8.6", &["vchord"], &[VCHORD_INDEX], true);
        assert!(check(RetrievalMode::Index, &ready).is_ok());
    }

    #[test]
    fn a_version_floor_compares_numbers_not_strings() {
        assert!(pgvector_at_least("0.8", "0.8"));
        assert!(pgvector_at_least("0.8.0", "0.8"));
        assert!(pgvector_at_least("0.8.6", "0.8"));
        assert!(pgvector_at_least("0.10.0", "0.8"));
        assert!(pgvector_at_least("1.0.0-rc1", "0.8"));
        assert!(!pgvector_at_least("0.7.4", "0.8"));
        assert!(!pgvector_at_least("0.8.0-rc1", "0.8.1"));
        assert!(!pgvector_at_least("garbage", "0.8"));
        assert!(!pgvector_at_least("", "0.8"));
    }

    #[test]
    fn the_threshold_is_visible_and_the_recommendation_follows_it() {
        assert_eq!(INDEX_THRESHOLD_EMBEDDINGS, 20_000);
        assert!(!index_recommended(19_999));
        assert!(index_recommended(20_000));
        for mode in RetrievalMode::ALL {
            let consequence = mode.consequence();
            assert!(!consequence.latency.is_empty());
            assert!(!consequence.index_memory.is_empty());
            assert!(!consequence.recall.is_empty());
            assert!(
                consequence.leg_note.contains("47 ms"),
                "the shared-leg arithmetic must be in the consequence: {:?}",
                mode
            );
        }
    }
}
