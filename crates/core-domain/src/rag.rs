//! RAG and multimodal vector namespace domain contracts (ADR 0020; Wave 45
//! lane 1, `p45-rag-vector-namespace`).
//!
//! Purpose: pure validation, provenance hashing, modality rules, and
//! Reciprocal Rank Fusion (RRF) for the second tenant vector namespace
//! (`rag_embeddings`) and its deletion receipts (`rag_deletion_receipts`).
//!
//! Key contracts (ADR 0020):
//! - Fixed width [`SEARCH_EMBEDDING_DIM`] (768) across both text and media
//!   modalities (`text | image | video | audio`). Any vector whose length is
//!   not 768 or that contains non-finite floats (`NaN`, `±∞`) is refused at
//!   the domain boundary and never truncated or stored.
//! - Media chunks (`image | video | audio`) MUST supply a precomputed 768-dim
//!   vector and a model name: Core never decodes images, samples video frames,
//!   resamples audio, runs transcription, or loads model weights.
//! - Text chunks (`text`) may either supply a precomputed 768-dim vector or
//!   rely on the org's configured `CompletionProvider::embed` over non-empty
//!   `content_text`.
//! - Hybrid retrieval combines lexical (BM25 / FTS over `content_text`) and
//!   exact-KNN cosine distance (`embedding <=> $vec`) legs via Reciprocal
//!   Rank Fusion (`RRF_K = 60.0`), degrading cleanly to vector-only when
//!   querying by `query_vector` alone or when chunks carry no text.

use crate::completion::SEARCH_EMBEDDING_DIM;
use crate::error::DomainError;
use crate::search::RRF_K;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Maximum number of chunks accepted in a single ingest batch.
pub const MAX_RAG_INGEST_BATCH: usize = 128;

/// Maximum `limit` accepted on a RAG query.
pub const MAX_RAG_QUERY_LIMIT: i64 = 100;

/// Default `limit` when omitted on a RAG query.
pub const DEFAULT_RAG_QUERY_LIMIT: i64 = 20;

/// Maximum length in characters for `namespace` and `source_kind` identifiers.
pub const MAX_RAG_IDENTIFIER_CHARS: usize = 128;

/// Maximum length in characters for `source_id`, `model`, and `content_hash`.
pub const MAX_RAG_LOCATOR_CHARS: usize = 256;

/// Maximum length in bytes for `content_text` on a single chunk.
pub const MAX_RAG_CONTENT_TEXT_BYTES: usize = 65_536;

/// Maximum length in characters for `query_text`.
pub const MAX_RAG_QUERY_TEXT_CHARS: usize = 2_048;

/// First-class modality of a stored chunk or query filter (ADR 0020 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RagModality {
    Text,
    Image,
    Video,
    Audio,
}

impl RagModality {
    pub const ALL: [RagModality; 4] = [
        RagModality::Text,
        RagModality::Image,
        RagModality::Video,
        RagModality::Audio,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RagModality::Text => "text",
            RagModality::Image => "image",
            RagModality::Video => "video",
            RagModality::Audio => "audio",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "text" => Some(RagModality::Text),
            "image" => Some(RagModality::Image),
            "video" => Some(RagModality::Video),
            "audio" => Some(RagModality::Audio),
            _ => None,
        }
    }

    /// True for `image | video | audio` — modalities where Core requires a
    /// caller-supplied precomputed 768-dim vector (ADR 0020 §5).
    #[must_use]
    pub const fn is_media(self) -> bool {
        matches!(
            self,
            RagModality::Image | RagModality::Video | RagModality::Audio
        )
    }
}

impl std::fmt::Display for RagModality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Composite chunk identity within an org + namespace:
/// `(source_kind, source_id, chunk_index, modality)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RagChunkKey {
    pub source_kind: String,
    pub source_id: String,
    pub chunk_index: i32,
    pub modality: RagModality,
}

impl RagChunkKey {
    #[must_use]
    pub fn new(
        source_kind: impl Into<String>,
        source_id: impl Into<String>,
        chunk_index: i32,
        modality: RagModality,
    ) -> Self {
        Self {
            source_kind: source_kind.into(),
            source_id: source_id.into(),
            chunk_index,
            modality,
        }
    }
}

/// Validates a non-empty identifier (`namespace`, `source_kind`) against
/// character and length bounds.
pub fn validate_rag_identifier(field: &str, value: &str) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation(format!(
            "{field} must not be empty"
        )));
    }
    if trimmed.chars().count() > MAX_RAG_IDENTIFIER_CHARS {
        return Err(DomainError::Validation(format!(
            "{field} exceeds maximum length of {MAX_RAG_IDENTIFIER_CHARS} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DomainError::Validation(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(trimmed.to_string())
}

/// Validates a non-empty locator or provenance string (`source_id`, `model`,
/// `content_hash`).
pub fn validate_rag_locator(field: &str, value: &str) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Validation(format!(
            "{field} must not be empty"
        )));
    }
    if trimmed.chars().count() > MAX_RAG_LOCATOR_CHARS {
        return Err(DomainError::Validation(format!(
            "{field} exceeds maximum length of {MAX_RAG_LOCATOR_CHARS} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DomainError::Validation(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(trimmed.to_string())
}

/// Strictly validates a 768-dimension embedding vector at the domain/API
/// boundary (ADR 0020 §2). Any vector whose length is not
/// [`SEARCH_EMBEDDING_DIM`] (768) or that carries `NaN` / `±∞` is refused with
/// `DomainError::Validation` (mapped to `400 Bad Request`), never truncated or
/// padded.
pub fn validate_embedding_vector(field: &str, vector: &[f32]) -> Result<(), DomainError> {
    if vector.len() != SEARCH_EMBEDDING_DIM {
        return Err(DomainError::Validation(format!(
            "{field} has dimension {}, but RAG vector namespace requires exactly {SEARCH_EMBEDDING_DIM} (SEARCH_EMBEDDING_DIM)",
            vector.len()
        )));
    }
    if vector.iter().any(|v| !v.is_finite()) {
        return Err(DomainError::Validation(format!(
            "{field} contains non-finite float values (NaN or infinity)"
        )));
    }
    Ok(())
}

/// Computes a deterministic SHA-256 hex content hash when the caller omits
/// `content_hash` on a chunk. Covers modality, `content_text`, and when no
/// text is present, the canonical float bytes of the precomputed vector.
#[must_use]
pub fn compute_chunk_content_hash(
    modality: RagModality,
    content_text: Option<&str>,
    embedding: Option<&[f32]>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(modality.as_str().as_bytes());
    hasher.update([0x1f]);
    if let Some(text) = content_text {
        hasher.update(text.as_bytes());
    }
    hasher.update([0x1f]);
    if content_text.is_none() {
        if let Some(vec) = embedding {
            for val in vec {
                hasher.update(val.to_le_bytes());
            }
        }
    }
    hex::encode(hasher.finalize())
}

/// Input chunk in a batch ingest request before provider embedding resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RagChunkInput {
    /// Source entity family (e.g. `"ticket"`, `"kb_article"`, `"attachment"`,
    /// `"document"`, or a connector-defined kind).
    pub source_kind: String,
    /// Stable identifier of the source within `source_kind` (UUID or external
    /// locator).
    pub source_id: String,
    /// Zero-based passage or frame/segment index within the source.
    pub chunk_index: i32,
    /// Modality of this chunk (`text | image | video | audio`).
    pub modality: RagModality,
    /// Passage text (required when `modality == text` and `embedding` is
    /// omitted; optional caption/transcript snippet for media chunks to
    /// participate in the lexical BM25 leg).
    #[serde(default)]
    pub content_text: Option<String>,
    /// Attribution metadata (title, uri/locator, span/page/timecode,
    /// mime_type, etc.) returned on every query hit. Defaults to `{}`.
    #[serde(default = "default_metadata_object")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub metadata: serde_json::Value,
    /// Embedding model identifier. Required when `embedding` is supplied;
    /// defaults to the org's configured embedding model when Core embeds a
    /// `text` chunk.
    #[serde(default)]
    pub model: Option<String>,
    /// Content hash for idempotent re-ingest provenance. Computed
    /// deterministically as SHA-256 hex when omitted.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Precomputed 768-dim vector. Required for `image | video | audio`
    /// modalities; optional for `text` chunks when the org has an active
    /// embedding provider.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

fn default_metadata_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

impl RagChunkInput {
    /// Validates chunk fields, modality constraints, metadata shape, and
    /// precomputed vector width (768).
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_rag_identifier("source_kind", &self.source_kind)?;
        validate_rag_locator("source_id", &self.source_id)?;
        if self.chunk_index < 0 {
            return Err(DomainError::Validation(
                "chunk_index must be non-negative (>= 0)".into(),
            ));
        }
        if let Some(text) = &self.content_text {
            if text.len() > MAX_RAG_CONTENT_TEXT_BYTES {
                return Err(DomainError::Validation(format!(
                    "content_text exceeds maximum size of {MAX_RAG_CONTENT_TEXT_BYTES} bytes"
                )));
            }
        }
        if !self.metadata.is_object() {
            return Err(DomainError::Validation(
                "metadata must be a JSON object".into(),
            ));
        }
        if let Some(model) = &self.model {
            validate_rag_locator("model", model)?;
        }
        if let Some(hash) = &self.content_hash {
            validate_rag_locator("content_hash", hash)?;
        }
        match &self.embedding {
            Some(vec) => {
                validate_embedding_vector("embedding", vec)?;
                if self
                    .model
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
                {
                    return Err(DomainError::Validation(
                        "model is required when a precomputed embedding vector is supplied".into(),
                    ));
                }
            }
            None => {
                if self.modality.is_media() {
                    return Err(DomainError::Validation(format!(
                        "modality '{}' requires a precomputed {SEARCH_EMBEDDING_DIM}-dim embedding vector (Core does not decode or embed media)",
                        self.modality
                    )));
                }
                let text = self
                    .content_text
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default();
                if text.is_empty() {
                    return Err(DomainError::Validation(
                        "text chunk without a precomputed embedding vector requires non-empty content_text".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// A validated chunk with a resolved 768-dim vector, model, and content hash,
/// ready for database upsert.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedRagChunk {
    pub namespace: String,
    pub source_kind: String,
    pub source_id: String,
    pub chunk_index: i32,
    pub modality: RagModality,
    pub content_text: Option<String>,
    pub metadata: serde_json::Value,
    pub model: String,
    pub content_hash: String,
    pub embedding: Vec<f32>,
}

/// Ranked candidate from the lexical (BM25 / `ts_rank_cd`) leg.
#[derive(Debug, Clone, PartialEq)]
pub struct RagLexicalCandidate {
    pub key: RagChunkKey,
    pub lexical_score: f64,
}

/// Ranked candidate from the exact-KNN cosine distance (`embedding <=> $vec`) leg.
#[derive(Debug, Clone, PartialEq)]
pub struct RagVectorCandidate {
    pub key: RagChunkKey,
    pub vector_distance: f64,
}

/// Fused score breakdown for one chunk after Reciprocal Rank Fusion (RRF).
#[derive(Debug, Clone, PartialEq)]
pub struct RagFusedCandidate {
    pub key: RagChunkKey,
    pub score: f64,
    pub vector_distance: Option<f64>,
    pub lexical_score: Option<f64>,
}

/// Combines lexical (BM25 / `ts_rank_cd`) and vector (exact-KNN cosine
/// distance) candidate lists via Reciprocal Rank Fusion (`RRF_K = 60.0`).
///
/// - Each leg contributes `1.0 / (RRF_K + rank + 1.0)` for every distinct
///   [`RagChunkKey`] (0-indexed `rank` in the leg's best-first ordering).
/// - When both legs are present, a chunk appearing in both sums its RRF
///   contributions and retains both `vector_distance` and `lexical_score`.
/// - When only one leg is active (e.g. `query_vector`-only cross-modal query,
///   or lexical-only fallback when no query vector is available), RRF degrades
///   cleanly to that leg's ranking while preserving `vector_distance` /
///   `lexical_score`.
/// - Ties break deterministically by `RagChunkKey` ascending.
#[must_use]
pub fn fuse_rag_candidates(
    lexical: &[RagLexicalCandidate],
    vector: &[RagVectorCandidate],
) -> Vec<RagFusedCandidate> {
    let mut rrf_scores: HashMap<RagChunkKey, f64> = HashMap::new();
    let mut lex_scores: HashMap<RagChunkKey, f64> = HashMap::new();
    let mut vec_distances: HashMap<RagChunkKey, f64> = HashMap::new();

    let mut seen_lex: HashSet<&RagChunkKey> = HashSet::new();
    for (rank, cand) in lexical.iter().enumerate() {
        if !seen_lex.insert(&cand.key) {
            continue;
        }
        *rrf_scores.entry(cand.key.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
        lex_scores.insert(cand.key.clone(), cand.lexical_score);
    }

    let mut seen_vec: HashSet<&RagChunkKey> = HashSet::new();
    for (rank, cand) in vector.iter().enumerate() {
        if !seen_vec.insert(&cand.key) {
            continue;
        }
        *rrf_scores.entry(cand.key.clone()).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
        vec_distances.insert(cand.key.clone(), cand.vector_distance);
    }

    let mut fused: Vec<RagFusedCandidate> = rrf_scores
        .into_iter()
        .map(|(key, score)| {
            let vector_distance = vec_distances.remove(&key);
            let lexical_score = lex_scores.remove(&key);
            RagFusedCandidate {
                key,
                score,
                vector_distance,
                lexical_score,
            }
        })
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.key.cmp(&b.key))
    });
    fused
}

/// Ranked RAG hit with mandatory source attribution (ADR 0020 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RagQueryHit {
    pub namespace: String,
    pub source_kind: String,
    pub source_id: String,
    pub chunk_index: i32,
    pub modality: RagModality,
    pub model: String,
    pub content_hash: String,
    /// Reciprocal Rank Fusion score combining the active retrieval legs.
    pub score: f64,
    /// Cosine distance (`embedding <=> $query_vec`) when the vector leg ran
    /// and matched this chunk.
    pub vector_distance: Option<f64>,
    /// Lexical FTS rank (`ts_rank_cd`) when the BM25/FTS leg ran and matched
    /// this chunk.
    pub lexical_score: Option<f64>,
    pub content_text: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub metadata: serde_json::Value,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub embedded_at: DateTime<Utc>,
}

/// Provable deletion receipt recorded when a source's chunks are removed from
/// `rag_embeddings` (ADR 0020 §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RagDeletionReceipt {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub receipt_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    pub namespace: String,
    pub source_kind: String,
    pub source_id: String,
    pub deleted_chunks: i32,
    pub content_hashes: Vec<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub deleted_by: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub deleted_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_768(val: f32) -> Vec<f32> {
        vec![val; SEARCH_EMBEDDING_DIM]
    }

    #[test]
    fn modality_round_trips_and_flags_media() {
        for m in RagModality::ALL {
            assert_eq!(RagModality::parse(m.as_str()), Some(m));
        }
        assert_eq!(RagModality::parse("unknown"), None);
        assert!(!RagModality::Text.is_media());
        assert!(RagModality::Image.is_media());
        assert!(RagModality::Video.is_media());
        assert!(RagModality::Audio.is_media());
    }

    #[test]
    fn validate_embedding_vector_enforces_exact_768_and_finite_floats() {
        assert!(validate_embedding_vector("embedding", &vec_768(0.1)).is_ok());

        for bad_len in [0_usize, 1, 384, 512, 767, 769, 1024, 1536] {
            let err = validate_embedding_vector("embedding", &vec![0.1; bad_len])
                .expect_err("wrong width must fail");
            assert!(
                err.to_string().contains("768"),
                "error must name 768 requirement: {err}"
            );
        }

        let mut nan_vec = vec_768(0.1);
        nan_vec[42] = f32::NAN;
        assert!(validate_embedding_vector("embedding", &nan_vec).is_err());

        let mut inf_vec = vec_768(0.1);
        inf_vec[0] = f32::INFINITY;
        assert!(validate_embedding_vector("embedding", &inf_vec).is_err());
    }

    #[test]
    fn media_chunk_requires_precomputed_vector_and_model() {
        for modality in [RagModality::Image, RagModality::Video, RagModality::Audio] {
            let missing_vec = RagChunkInput {
                source_kind: "attachment".into(),
                source_id: "att-1".into(),
                chunk_index: 0,
                modality,
                content_text: Some("error screenshot".into()),
                metadata: serde_json::json!({"mime_type": "image/png"}),
                model: Some("embeddinggemma-2-768".into()),
                content_hash: None,
                embedding: None,
            };
            assert!(
                missing_vec.validate().is_err(),
                "{modality} without vector must be refused"
            );

            let with_vec = RagChunkInput {
                embedding: Some(vec_768(0.25)),
                ..missing_vec
            };
            assert!(with_vec.validate().is_ok());
        }
    }

    #[test]
    fn text_chunk_accepts_text_for_provider_or_precomputed_vector() {
        let provider_text = RagChunkInput {
            source_kind: "kb_article".into(),
            source_id: "kb-1".into(),
            chunk_index: 0,
            modality: RagModality::Text,
            content_text: Some("Reset the VPN tunnel on firewall A.".into()),
            metadata: serde_json::json!({"title": "VPN Reset"}),
            model: None,
            content_hash: None,
            embedding: None,
        };
        assert!(provider_text.validate().is_ok());

        let empty_text_no_vec = RagChunkInput {
            content_text: Some("   ".into()),
            ..provider_text.clone()
        };
        assert!(empty_text_no_vec.validate().is_err());

        let precomputed_text = RagChunkInput {
            embedding: Some(vec_768(0.5)),
            model: Some("bge-base-en".into()),
            ..provider_text
        };
        assert!(precomputed_text.validate().is_ok());
    }

    #[test]
    fn rrf_fusion_boosts_dual_leg_hits_and_preserves_cross_modal_vector_hits() {
        let k_both = RagChunkKey::new("kb_article", "a1", 0, RagModality::Text);
        let k_lex_only = RagChunkKey::new("ticket", "t1", 1, RagModality::Text);
        let k_media_vec = RagChunkKey::new("attachment", "img1", 0, RagModality::Image);

        let lexical = vec![
            RagLexicalCandidate {
                key: k_both.clone(),
                lexical_score: 0.9,
            },
            RagLexicalCandidate {
                key: k_lex_only.clone(),
                lexical_score: 0.7,
            },
        ];
        let vector = vec![
            RagVectorCandidate {
                key: k_media_vec.clone(),
                vector_distance: 0.08,
            },
            RagVectorCandidate {
                key: k_both.clone(),
                vector_distance: 0.12,
            },
        ];

        let fused = fuse_rag_candidates(&lexical, &vector);
        assert_eq!(fused.len(), 3);
        // `k_both` appears in both legs (rank 0 in lexical + rank 1 in vector)
        // so its RRF score (1/61 + 1/62) beats single-leg rank 0 (1/61).
        assert_eq!(fused[0].key, k_both);
        assert_eq!(fused[0].lexical_score, Some(0.9));
        assert_eq!(fused[0].vector_distance, Some(0.12));

        // Cross-modal image hit from the vector leg survives with its distance.
        let media_hit = fused
            .iter()
            .find(|c| c.key == k_media_vec)
            .expect("media hit present");
        assert_eq!(media_hit.vector_distance, Some(0.08));
        assert_eq!(media_hit.lexical_score, None);
    }

    #[test]
    fn content_hash_is_deterministic_and_sensitive_to_changes() {
        let h1 = compute_chunk_content_hash(RagModality::Text, Some("hello world"), None);
        let h2 = compute_chunk_content_hash(RagModality::Text, Some("hello world"), None);
        let h3 = compute_chunk_content_hash(RagModality::Text, Some("hello world!"), None);
        assert_eq!(h1.len(), 64);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);

        let m1 = compute_chunk_content_hash(RagModality::Image, None, Some(&vec_768(0.1)));
        let m2 = compute_chunk_content_hash(RagModality::Image, None, Some(&vec_768(0.2)));
        assert_ne!(m1, m2);
    }
}
