//! RAG and media vector namespace routes (ADR 0020; the private build tracker Wave 45 lane 1,
//! `p45-rag-vector-namespace`, migration 0134).
//!
//! Published OpenAPI extension points under `/api/v1/rag/...` (and legacy
//! `/api/rag/...`):
//! - `POST /api/v1/rag/chunks` — batch upsert of passage and multimodal
//!   (`text | image | video | audio`) chunks inside a tenant `namespace`.
//!   Accepts precomputed 768-dim vectors for any modality or embeds `text`
//!   chunks via the org's configured `CompletionProvider::embed`. Enforces
//!   `SEARCH_EMBEDDING_DIM = 768` strictly (any wrong-width vector is refused
//!   with a typed `422` validation error (`validation_failed`) before touching the database) and records
//!   provenance (`model` + `content_hash`) for idempotent re-ingest. Rides the
//!   IF1–IF3 instant-first envelope (`idempotency_key`, `version`, `mutation`
//!   metadata).
//! - `POST /api/v1/rag/query` — hybrid retrieval combining the lexical BM25 /
//!   FTS leg (`ts_rank_cd`) and the exact-KNN cosine distance leg
//!   (`embedding <=> $vec`) via Reciprocal Rank Fusion (`RRF_K = 60.0`),
//!   scoped to a single `namespace` with optional `modalities`, `source_kinds`,
//!   and `source_ids` filters. Supports cross-modal retrieval (text->media and
//!   media->text within the shared 768 space) and returns mandatory source
//!   attribution on every hit.
//! - `DELETE /api/v1/rag/sources/{source_kind}/{source_id}` — deletes all
//!   chunks for a source within a `namespace` and mints a provable deletion
//!   receipt (`RagDeletionReceipt`) in the same transaction.
//! - `POST /api/v1/rag/delete` — body-driven source deletion within a
//!   `namespace` (convenience twin of the DELETE route for callers whose
//!   `source_id` contains slashes or URI locators, plus optional IF1–IF3
//!   idempotency envelope).
//! - `GET /api/v1/rag/deletion-receipts` and
//!   `GET /api/v1/rag/deletion-receipts/{id}` — list and verify immutable
//!   deletion receipts under RLS.
//!
//! Authorization:
//! - Ingest and source deletion require `kb:edit` (granted to `rag.write` /
//!   `rag:write` plugins, connectors, service accounts, and staff roles).
//! - Query and receipt verification require `kb:view` (granted to `rag.read` /
//!   `rag:read` plugins, connectors, service accounts, and staff roles).

use crate::ai::{embed_batch, resolve_org_completion_provider, OrgCompletionProvider};
use crate::auth::middleware::{authed, require_permission};
use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use crate::instant::{
    self, ClaimSpec, InstantClaim, MutationMeta, MutationOutcome, MutationParams,
    ProvenanceContext, RecordSpec, SeqSpec,
};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::Utc;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::rag::{
    compute_chunk_content_hash, validate_embedding_vector, validate_rag_identifier,
    validate_rag_locator, PreparedRagChunk, RagChunkInput, RagDeletionReceipt, RagModality,
    RagQueryHit, DEFAULT_RAG_QUERY_LIMIT, MAX_RAG_INGEST_BATCH, MAX_RAG_QUERY_LIMIT,
    MAX_RAG_QUERY_TEXT_CHARS,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn rag_read_permission() -> Permission {
    Permission::Matrix {
        module: Module::Kb,
        action: Action::View,
    }
}

fn rag_write_permission() -> Permission {
    Permission::Matrix {
        module: Module::Kb,
        action: Action::Edit,
    }
}

/// Deterministic UUID v5-style namespace anchor for `entity_write_seqs` and
/// `entity_writes` tracking per `(org_id, namespace)`.
fn namespace_entity_id(org_id: Uuid, namespace: &str) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"rag_namespace:");
    hasher.update(org_id.as_bytes());
    hasher.update(b":");
    hasher.update(namespace.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

// --- (B) Ingest Surface ------------------------------------------------------

/// Batch upsert request for `/api/v1/rag/chunks`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RagIngestRequest {
    /// Logical vector namespace within the tenant (e.g. `"default"`,
    /// `"kb-passages"`, `"ticket-media"`). Queries never cross namespaces.
    pub namespace: String,
    /// Chunks to upsert (1..=128 per batch).
    pub chunks: Vec<RagChunkInput>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// Response from `POST /api/v1/rag/chunks`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RagIngestResponse {
    pub namespace: String,
    pub total: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub mutation: MutationMeta,
}

#[derive(Debug, Serialize)]
struct RagIngestPayload {
    namespace: String,
    total: usize,
    inserted: usize,
    updated: usize,
    unchanged: usize,
}

/// POST /api/v1/rag/chunks — batch upsert of RAG passage and media chunks.
#[utoipa::path(
    operation_id = "rag_chunks_ingest",
    post,
    path = "/api/rag/chunks",
    tag = "rag",
    security(("bearer" = [])),
    request_body = RagIngestRequest,
    responses(
        (status = 200, body = RagIngestResponse),
        (status = 422, description = "validation error or wrong-width vector (must be 768)"),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:edit / rag.write permission"),
        (status = 409, description = "idempotency key reused or version conflict")
    )
)]
pub async fn ingest_chunks(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RagIngestRequest>,
) -> ApiResult<Json<RagIngestResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, rag_write_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let namespace = validate_rag_identifier("namespace", &req.namespace).map_err(ApiError::from)?;
    if req.chunks.is_empty() {
        return Err(ApiError::Validation(
            "chunks must contain at least 1 item".into(),
        ));
    }
    if req.chunks.len() > MAX_RAG_INGEST_BATCH {
        return Err(ApiError::Validation(format!(
            "chunks batch size {} exceeds maximum of {MAX_RAG_INGEST_BATCH}",
            req.chunks.len()
        )));
    }

    // Validate every chunk at the boundary BEFORE any provider call or DB write
    // so a single malformed or wrong-width vector rejects the entire batch cleanly.
    for (idx, chunk) in req.chunks.iter().enumerate() {
        chunk
            .validate()
            .map_err(|e| ApiError::Validation(format!("chunks[{idx}]: {e}")))?;
    }

    let fingerprint = serde_json::to_value(&req)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize request: {e}")))?;
    let idem_key = req.instant.idempotency_key.clone();
    let version = req.instant.version;
    let ns_entity_id = namespace_entity_id(org_id, &namespace);

    // Resolve text chunks that omitted a precomputed vector via the org's
    // configured CompletionProvider::embed (outside the DB transaction so no
    // HTTP call holds a database connection).
    let indices_needing_embed: Vec<usize> = req
        .chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| if c.embedding.is_none() { Some(i) } else { None })
        .collect();

    let (provider_model, mut embedded_vectors) = if !indices_needing_embed.is_empty() {
        let provider = resolve_org_completion_provider(&state, org_id).await;
        let model_name = match &provider {
            OrgCompletionProvider::Embeddings(emb) => emb.model_name().to_string(),
            OrgCompletionProvider::None(_) => {
                return Err(ApiError::Validation(
                    "text chunks without precomputed embedding vectors require a configured org embedding provider (or supply 768-dim precomputed vectors)".into(),
                ));
            }
        };
        let texts: Vec<String> = indices_needing_embed
            .iter()
            .map(|&i| req.chunks[i].content_text.clone().unwrap_or_default())
            .collect();
        let vectors = embed_batch(&provider, texts).await.ok_or_else(|| {
            ApiError::Validation(
                "org embedding provider failed or returned non-768 vectors for text chunks".into(),
            )
        })?;
        (
            Some(model_name),
            indices_needing_embed
                .into_iter()
                .zip(vectors)
                .collect::<std::collections::HashMap<usize, Vec<f32>>>(),
        )
    } else {
        (None, std::collections::HashMap::new())
    };

    let mut prepared = Vec::with_capacity(req.chunks.len());
    for (idx, chunk) in req.chunks.into_iter().enumerate() {
        let source_kind =
            validate_rag_identifier("source_kind", &chunk.source_kind).map_err(ApiError::from)?;
        let source_id =
            validate_rag_locator("source_id", &chunk.source_id).map_err(ApiError::from)?;
        let (embedding, resolved_model) = match chunk.embedding {
            Some(vec) => {
                let m = chunk.model.as_deref().ok_or_else(|| {
                    ApiError::Validation(format!(
                        "chunks[{idx}]: model is required with precomputed embedding"
                    ))
                })?;
                let validated_model = validate_rag_locator("model", m).map_err(ApiError::from)?;
                (vec, validated_model)
            }
            None => {
                let vec = embedded_vectors.remove(&idx).ok_or_else(|| {
                    ApiError::Internal(anyhow::anyhow!("missing embedded vector for chunk {idx}"))
                })?;
                let m = match chunk.model.as_deref() {
                    Some(explicit) if !explicit.trim().is_empty() => {
                        validate_rag_locator("model", explicit).map_err(ApiError::from)?
                    }
                    _ => provider_model.clone().unwrap_or_else(|| "unknown".into()),
                };
                (vec, m)
            }
        };
        let content_hash = match chunk.content_hash.as_deref() {
            Some(h) if !h.trim().is_empty() => {
                validate_rag_locator("content_hash", h).map_err(ApiError::from)?
            }
            _ => compute_chunk_content_hash(
                chunk.modality,
                chunk.content_text.as_deref(),
                Some(&embedding),
            ),
        };
        prepared.push(PreparedRagChunk {
            namespace: namespace.clone(),
            source_kind,
            source_id,
            chunk_index: chunk.chunk_index,
            modality: chunk.modality,
            content_text: chunk.content_text,
            metadata: chunk.metadata,
            model: resolved_model,
            content_hash,
            embedding,
        });
    }

    let actor_kind = if authed.via_api_key {
        Some("connector")
    } else {
        None
    };
    let audit_actor = if authed.via_api_key {
        Actor::Agent(format!("api-key:{user_id}"))
    } else {
        Actor::Human(user_id)
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            ClaimSpec {
                entity: "rag_namespace",
                entity_id: Some(ns_entity_id),
                key: idem_key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let seq = instant::entity_seq(
            &mut ctx,
            SeqSpec {
                entity: "rag_namespace",
                entity_id: ns_entity_id,
                version,
            },
        )
        .await?;

        let db_outcome = core_db::rag::upsert_chunks(&mut ctx.tx, org_id, &prepared).await?;
        let now = Utc::now();
        let payload = RagIngestPayload {
            namespace: namespace.clone(),
            total: db_outcome.total,
            inserted: db_outcome.inserted,
            updated: db_outcome.updated,
            unchanged: db_outcome.unchanged,
        };
        let after = serde_json::to_value(&payload).unwrap_or_default();

        ctx.emit(
            types::RAG_CHUNKS_INGESTED,
            "rag_namespace",
            Some(namespace.as_str()),
            &after,
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: audit_actor,
                action: "rag.chunks_ingested".into(),
                entity_type: Some("rag_namespace".into()),
                entity_id: Some(namespace.clone()),
                detail: after.clone(),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        let meta = instant::record(
            &mut ctx,
            RecordSpec {
                entity: "rag_namespace",
                entity_id: ns_entity_id,
                op: "ingest_chunks",
                seq,
                before: None,
                after,
                updated_at: now,
                key: idem_key.as_deref(),
                payload: &payload,
                provenance: ProvenanceContext {
                    actor_kind,
                    ..Default::default()
                },
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(RagIngestResponse {
                namespace,
                total: db_outcome.total,
                inserted: db_outcome.inserted,
                updated: db_outcome.updated,
                unchanged: db_outcome.unchanged,
                mutation: meta,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- (C) Query Surface -------------------------------------------------------

fn default_query_limit() -> i64 {
    DEFAULT_RAG_QUERY_LIMIT
}

/// Hybrid BM25 + exact-KNN query request for `POST /api/v1/rag/query`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RagQueryRequest {
    /// Target namespace within the caller's org. Queries never cross
    /// namespaces.
    pub namespace: String,
    /// Optional lexical query string. When supplied without `query_vector`,
    /// Core also embeds `query_text` via the org's configured embedding
    /// provider so both the BM25 and vector legs run and fuse via RRF.
    #[serde(default)]
    pub query_text: Option<String>,
    /// Optional precomputed 768-dim query vector. Enables media->text,
    /// media->media, and caller-embedded text->media queries across the shared
    /// 768-dimension space.
    #[serde(default)]
    pub query_vector: Option<Vec<f32>>,
    /// Optional modality filter (`text | image | video | audio`). When empty
    /// or omitted, all modalities in the namespace participate (cross-modal
    /// retrieval).
    #[serde(default)]
    pub modalities: Vec<RagModality>,
    /// Optional `source_kind` filter.
    #[serde(default)]
    pub source_kinds: Vec<String>,
    /// Optional `source_id` filter.
    #[serde(default)]
    pub source_ids: Vec<String>,
    /// Maximum hits to return (1..=100, default 20).
    #[serde(default = "default_query_limit")]
    pub limit: i64,
}

/// Ranked response from `POST /api/v1/rag/query`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RagQueryResponse {
    pub namespace: String,
    pub limit: i64,
    /// Whether the lexical BM25/FTS leg participated in this query.
    pub lexical_leg_active: bool,
    /// Whether the 768-dim exact-KNN vector leg participated in this query.
    pub vector_leg_active: bool,
    pub hits: Vec<RagQueryHit>,
}

/// POST /api/v1/rag/query — hybrid BM25 + exact-KNN vector query with RRF
/// fusion and mandatory source attribution.
#[utoipa::path(
    operation_id = "rag_query",
    post,
    path = "/api/rag/query",
    tag = "rag",
    security(("bearer" = [])),
    request_body = RagQueryRequest,
    responses(
        (status = 200, body = RagQueryResponse),
        (status = 422, description = "invalid query parameters or wrong-width query_vector (must be 768)"),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:view / rag.read permission")
    )
)]
pub async fn query_namespace(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RagQueryRequest>,
) -> ApiResult<Json<RagQueryResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, rag_read_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let namespace = validate_rag_identifier("namespace", &req.namespace).map_err(ApiError::from)?;

    let trimmed_text = req
        .query_text
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(ref text) = trimmed_text {
        if text.chars().count() > MAX_RAG_QUERY_TEXT_CHARS {
            return Err(ApiError::Validation(format!(
                "query_text exceeds maximum length of {MAX_RAG_QUERY_TEXT_CHARS} characters"
            )));
        }
    }

    if let Some(ref vec) = req.query_vector {
        validate_embedding_vector("query_vector", vec).map_err(ApiError::from)?;
    }

    if trimmed_text.is_none() && req.query_vector.is_none() {
        return Err(ApiError::Validation(
            "at least one of query_text or query_vector must be provided".into(),
        ));
    }

    let mut source_kinds = Vec::with_capacity(req.source_kinds.len());
    for kind in &req.source_kinds {
        source_kinds.push(validate_rag_identifier("source_kinds[]", kind).map_err(ApiError::from)?);
    }
    let mut source_ids = Vec::with_capacity(req.source_ids.len());
    for sid in &req.source_ids {
        source_ids.push(validate_rag_locator("source_ids[]", sid).map_err(ApiError::from)?);
    }

    let limit = req.limit.clamp(1, MAX_RAG_QUERY_LIMIT);

    // Resolve query_vector: if caller supplied one, use it directly; otherwise
    // if query_text is present, attempt to embed it via the org's provider
    // (degrading cleanly to lexical-only if the org is in none mode).
    let resolved_vector = match req.query_vector {
        Some(vec) => Some(vec),
        None => match &trimmed_text {
            Some(text) => crate::ai::embed_search_query(&state, org_id, text).await,
            None => None,
        },
    };

    let lexical_leg_active = trimmed_text.is_some();
    let vector_leg_active = resolved_vector.is_some();

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let hits = core_db::rag::query_hybrid(
        &mut tx,
        core_db::rag::RagQueryParams {
            org_id,
            namespace: &namespace,
            modalities: &req.modalities,
            source_kinds: &source_kinds,
            source_ids: &source_ids,
            query_text: trimmed_text.as_deref(),
            query_vector: resolved_vector.as_deref(),
            limit,
        },
    )
    .await?;

    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(RagQueryResponse {
        namespace,
        limit,
        lexical_leg_active,
        vector_leg_active,
        hits,
    }))
}

// --- (D) Deletion Propagation & Receipt Surface ------------------------------

/// Query parameters for `DELETE /api/v1/rag/sources/{source_kind}/{source_id}`.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct DeleteRagSourceQuery {
    /// Target namespace from which to delete the source's chunks (defaults to
    /// `"default"` when omitted so parameterless probes and simple callers
    /// have a deterministic default).
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

fn default_namespace() -> String {
    "default".to_string()
}

/// Request body for `POST /api/v1/rag/delete`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RagDeleteSourceRequest {
    pub namespace: String,
    pub source_kind: String,
    pub source_id: String,
    #[serde(flatten)]
    pub instant: MutationParams,
}

/// Response from deleting a source's chunks in a namespace.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RagDeleteSourceResponse {
    pub receipt: RagDeletionReceipt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation: Option<MutationMeta>,
}

#[derive(Debug, Serialize)]
struct RagDeletePayload {
    receipt: RagDeletionReceipt,
}

/// DELETE /api/v1/rag/sources/{source_kind}/{source_id} — deletes all chunks
/// for `(namespace, source_kind, source_id)` and returns a verifiable deletion
/// receipt.
#[utoipa::path(
    operation_id = "rag_source_delete",
    delete,
    path = "/api/rag/sources/{source_kind}/{source_id}",
    tag = "rag",
    security(("bearer" = [])),
    params(
        ("source_kind" = String, Path, description = "Source entity family (e.g. ticket, attachment, kb_article)"),
        ("source_id" = String, Path, description = "Source identifier within source_kind"),
        DeleteRagSourceQuery
    ),
    responses(
        (status = 200, body = RagDeleteSourceResponse),
        (status = 422, description = "invalid namespace or source identifier"),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:edit / rag.write permission")
    )
)]
pub async fn delete_source_path(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((source_kind, source_id)): Path<(String, String)>,
    Query(q): Query<DeleteRagSourceQuery>,
) -> ApiResult<Json<RagDeleteSourceResponse>> {
    let req = RagDeleteSourceRequest {
        namespace: q.namespace,
        source_kind,
        source_id,
        instant: MutationParams::default(),
    };
    delete_source_inner(state, extensions, req).await
}

/// POST /api/v1/rag/delete — deletes all chunks for `(namespace, source_kind,
/// source_id)` and returns a verifiable deletion receipt (supports arbitrary
/// URI/slash `source_id` values and the IF1–IF3 idempotency envelope).
#[utoipa::path(
    operation_id = "rag_source_delete_post",
    post,
    path = "/api/rag/delete",
    tag = "rag",
    security(("bearer" = [])),
    request_body = RagDeleteSourceRequest,
    responses(
        (status = 200, body = RagDeleteSourceResponse),
        (status = 422, description = "invalid namespace or source identifier"),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:edit / rag.write permission"),
        (status = 409, description = "idempotency key reused or version conflict")
    )
)]
pub async fn delete_source_post(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RagDeleteSourceRequest>,
) -> ApiResult<Json<RagDeleteSourceResponse>> {
    delete_source_inner(state, extensions, req).await
}

async fn delete_source_inner(
    state: AppState,
    extensions: Extensions,
    req: RagDeleteSourceRequest,
) -> ApiResult<Json<RagDeleteSourceResponse>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, rag_write_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let namespace = validate_rag_identifier("namespace", &req.namespace).map_err(ApiError::from)?;
    let source_kind =
        validate_rag_identifier("source_kind", &req.source_kind).map_err(ApiError::from)?;
    let source_id = validate_rag_locator("source_id", &req.source_id).map_err(ApiError::from)?;

    let fingerprint = serde_json::to_value(&req)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("serialize request: {e}")))?;
    let idem_key = req.instant.idempotency_key.clone();
    let version = req.instant.version;
    let ns_entity_id = namespace_entity_id(org_id, &namespace);

    let actor_kind = if authed.via_api_key {
        Some("connector")
    } else {
        None
    };
    let deleted_by_user = if authed.via_api_key {
        None
    } else {
        Some(user_id)
    };
    let audit_actor = if authed.via_api_key {
        Actor::Agent(format!("api-key:{user_id}"))
    } else {
        Actor::Human(user_id)
    };

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            ClaimSpec {
                entity: "rag_namespace",
                entity_id: Some(ns_entity_id),
                key: idem_key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let seq = instant::entity_seq(
            &mut ctx,
            SeqSpec {
                entity: "rag_namespace",
                entity_id: ns_entity_id,
                version,
            },
        )
        .await?;

        let receipt = core_db::rag::delete_source_in_namespace(
            &mut ctx.tx,
            org_id,
            &namespace,
            &source_kind,
            &source_id,
            deleted_by_user,
        )
        .await?;

        let payload = RagDeletePayload {
            receipt: receipt.clone(),
        };
        let after = serde_json::to_value(&payload).unwrap_or_default();

        ctx.emit(
            types::RAG_SOURCE_DELETED,
            "rag_deletion_receipt",
            Some(receipt.receipt_id.to_string().as_str()),
            &after,
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: audit_actor,
                action: "rag.source_deleted".into(),
                entity_type: Some("rag_deletion_receipt".into()),
                entity_id: Some(receipt.receipt_id.to_string()),
                detail: after.clone(),
                ip: None,
                occurred_at: receipt.deleted_at,
            },
        )
        .await?;

        let meta = instant::record(
            &mut ctx,
            RecordSpec {
                entity: "rag_namespace",
                entity_id: ns_entity_id,
                op: "delete_source",
                seq,
                before: None,
                after,
                updated_at: receipt.deleted_at,
                key: idem_key.as_deref(),
                payload: &payload,
                provenance: ProvenanceContext {
                    actor_kind,
                    ..Default::default()
                },
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(RagDeleteSourceResponse {
                receipt,
                mutation: Some(meta),
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// Query filter for `GET /api/v1/rag/deletion-receipts`.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListDeletionReceiptsQuery {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default = "default_receipts_limit")]
    pub limit: i64,
}

fn default_receipts_limit() -> i64 {
    50
}

/// Response from `GET /api/v1/rag/deletion-receipts`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RagDeletionReceiptList {
    pub receipts: Vec<RagDeletionReceipt>,
}

/// GET /api/v1/rag/deletion-receipts — lists verifiable deletion receipts for
/// the caller's org (including receipts minted automatically by core entity
/// deletion cascades).
#[utoipa::path(
    operation_id = "rag_deletion_receipts_list",
    get,
    path = "/api/rag/deletion-receipts",
    tag = "rag",
    security(("bearer" = [])),
    params(ListDeletionReceiptsQuery),
    responses(
        (status = 200, body = RagDeletionReceiptList),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:view / rag.read permission")
    )
)]
pub async fn list_deletion_receipts(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ListDeletionReceiptsQuery>,
) -> ApiResult<Json<RagDeletionReceiptList>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, rag_read_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let receipts = core_db::rag::list_deletion_receipts(
        &mut *tx,
        org_id,
        q.namespace.as_deref(),
        q.source_kind.as_deref(),
        q.source_id.as_deref(),
        q.limit,
    )
    .await?;

    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(RagDeletionReceiptList { receipts }))
}

/// GET /api/v1/rag/deletion-receipts/{id} — fetches a single verifiable
/// deletion receipt by `receipt_id` under RLS.
#[utoipa::path(
    operation_id = "rag_deletion_receipt_get",
    get,
    path = "/api/rag/deletion-receipts/{id}",
    tag = "rag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Deletion receipt ID")),
    responses(
        (status = 200, body = RagDeletionReceipt),
        (status = 401, description = "not authenticated"),
        (status = 403, description = "missing kb:view / rag.read permission"),
        (status = 404, description = "receipt not found in caller's org")
    )
)]
pub async fn get_deletion_receipt(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RagDeletionReceipt>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    require_permission(&authed.principal, rag_read_permission())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let receipt = core_db::rag::get_deletion_receipt(&mut *tx, org_id, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(receipt))
}
