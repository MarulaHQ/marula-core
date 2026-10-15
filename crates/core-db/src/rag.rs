//! RAG and multimodal vector namespace persistence (`rag_embeddings` and
//! `rag_deletion_receipts`, migration 0134; ADR 0020; Wave 45 lane 1).
//!
//! Purpose:
//! - [`upsert_chunks`] — idempotent batch upsert of passage and media chunks
//!   keyed on `(org_id, namespace, source_kind, source_id, chunk_index, modality)`.
//!   When `(model, content_hash)` is unchanged, the row is untouched (`unchanged`
//!   count increments, `embedded_at` stays stable); when either `model` or
//!   `content_hash` changes, the vector, provenance, text, and metadata update
//!   in place.
//! - [`query_hybrid`] — hybrid retrieval combining the lexical BM25 / FTS leg
//!   (`ts_rank_cd(tsv, websearch_to_tsquery('english', $q))`) and the exact-KNN
//!   cosine distance leg (`embedding <=> $vec`) via Reciprocal Rank Fusion
//!   (`core_domain::rag::fuse_rag_candidates`), scoped strictly to `(org_id,
//!   namespace)` with optional `modalities`, `source_kinds`, and `source_ids`
//!   filters.
//! - [`delete_source`] — removes all chunks for a `(org_id, namespace,
//!   source_kind, source_id)` tuple (or across every namespace when `namespace`
//!   is `None`) and mints one provable [`RagDeletionReceipt`] row per affected
//!   namespace in the same transaction.
//! - [`get_deletion_receipt`] / [`list_deletion_receipts`] — reads verifiable
//!   deletion receipts under RLS.

use crate::search::embedding_to_pg;
use crate::DbResult;
use chrono::{DateTime, Utc};
use core_domain::rag::{
    fuse_rag_candidates, validate_embedding_vector, PreparedRagChunk, RagChunkKey,
    RagDeletionReceipt, RagLexicalCandidate, RagModality, RagQueryHit, RagVectorCandidate,
};
use sqlx::{PgConnection, Postgres, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// Outcome of a batch chunk upsert.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RagUpsertOutcome {
    /// Total chunks processed in the batch.
    pub total: usize,
    /// Chunks newly inserted.
    pub inserted: usize,
    /// Existing chunks updated because `model` or `content_hash` changed.
    pub updated: usize,
    /// Existing chunks skipped because `(model, content_hash)` already matched.
    pub unchanged: usize,
}

/// Upserts a batch of validated, 768-dim chunks into `rag_embeddings` inside
/// the caller's RLS-scoped transaction.
///
/// Provenance idempotency rule (ADR 0020 §1, §4):
/// - If no row exists for `(org_id, namespace, source_kind, source_id,
///   chunk_index, modality)`, inserts the row (`inserted += 1`).
/// - If a row exists and `(model, content_hash)` differs, updates the vector,
///   provenance, text, metadata, and `embedded_at = now()` (`updated += 1`).
/// - If a row exists with the exact same `(model, content_hash)`, leaves the
///   row untouched (`unchanged += 1`).
pub async fn upsert_chunks(
    conn: &mut PgConnection,
    org_id: Uuid,
    chunks: &[PreparedRagChunk],
) -> DbResult<RagUpsertOutcome> {
    if chunks.is_empty() {
        return Ok(RagUpsertOutcome::default());
    }

    for chunk in chunks {
        validate_embedding_vector("embedding", &chunk.embedding).map_err(|e| {
            crate::DbError::Sql(sqlx::Error::Protocol(format!("invalid rag vector: {e}")))
        })?;
    }

    let mut outcome = RagUpsertOutcome {
        total: chunks.len(),
        ..Default::default()
    };

    for chunk in chunks {
        let vec_literal = embedding_to_pg(&chunk.embedding);
        let row = sqlx::query(
            r#"INSERT INTO rag_embeddings (
                 org_id, namespace, source_kind, source_id, chunk_index, modality,
                 content_text, metadata, model, content_hash, embedding, embedded_at
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::text::vector, now())
               ON CONFLICT (org_id, namespace, source_kind, source_id, chunk_index, modality)
               DO UPDATE SET
                 content_text = EXCLUDED.content_text,
                 metadata     = EXCLUDED.metadata,
                 model        = EXCLUDED.model,
                 content_hash = EXCLUDED.content_hash,
                 embedding    = EXCLUDED.embedding,
                 embedded_at  = now()
               WHERE rag_embeddings.model <> EXCLUDED.model
                  OR rag_embeddings.content_hash <> EXCLUDED.content_hash
               RETURNING (xmax = 0) AS was_inserted"#,
        )
        .bind(org_id)
        .bind(&chunk.namespace)
        .bind(&chunk.source_kind)
        .bind(&chunk.source_id)
        .bind(chunk.chunk_index)
        .bind(chunk.modality.as_str())
        .bind(&chunk.content_text)
        .bind(&chunk.metadata)
        .bind(&chunk.model)
        .bind(&chunk.content_hash)
        .bind(&vec_literal)
        .fetch_optional(&mut *conn)
        .await?;

        match row {
            Some(r) => {
                let was_inserted: bool = r.try_get("was_inserted")?;
                if was_inserted {
                    outcome.inserted += 1;
                } else {
                    outcome.updated += 1;
                }
            }
            None => {
                outcome.unchanged += 1;
            }
        }
    }

    Ok(outcome)
}

/// Parameters for a hybrid or cross-modal query within a single namespace.
#[derive(Debug, Clone)]
pub struct RagQueryParams<'a> {
    pub org_id: Uuid,
    pub namespace: &'a str,
    /// Optional modality filter (`text | image | video | audio`). Empty slice
    /// means all modalities in the namespace participate.
    pub modalities: &'a [RagModality],
    /// Optional `source_kind` filter. Empty slice means all source kinds.
    pub source_kinds: &'a [String],
    /// Optional `source_id` filter. Empty slice means all source ids.
    pub source_ids: &'a [String],
    /// Optional lexical query string for the BM25 / FTS leg.
    pub query_text: Option<&'a str>,
    /// Optional 768-dim vector for the exact-KNN cosine leg (either embedded
    /// from `query_text` by the org's provider or supplied directly by the
    /// caller for media->text / media->media / pre-embedded queries).
    pub query_vector: Option<&'a [f32]>,
    pub limit: i64,
}

fn candidate_depth(limit: i64) -> i64 {
    (limit.saturating_mul(4)).clamp(50, 400)
}

/// The three optional retrieval filters shared by both hybrid legs. An empty
/// slice means "no filter on this dimension".
#[derive(Debug, Clone, Copy)]
pub struct RagFilter<'a> {
    /// `modality` values admitted (already stringified).
    pub modalities: &'a [String],
    /// `source_kind` values admitted.
    pub source_kinds: &'a [String],
    /// `source_id` values admitted.
    pub source_ids: &'a [String],
}

/// Runs the lexical (BM25 / `ts_rank_cd`) candidate leg within `(org_id, namespace)`.
pub async fn lexical_candidates<'e, E>(
    executor: E,
    org_id: Uuid,
    namespace: &str,
    filter: RagFilter<'_>,
    query_text: &str,
    depth: i64,
) -> DbResult<Vec<RagLexicalCandidate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let trimmed = query_text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        r#"SELECT source_kind,
                  source_id,
                  chunk_index,
                  modality,
                  ts_rank_cd(tsv, websearch_to_tsquery('english', $6))::float8 AS lexical_score
             FROM rag_embeddings
            WHERE org_id = $1
              AND namespace = $2
              AND (cardinality($3::text[]) = 0 OR modality = ANY($3))
              AND (cardinality($4::text[]) = 0 OR source_kind = ANY($4))
              AND (cardinality($5::text[]) = 0 OR source_id = ANY($5))
              AND tsv @@ websearch_to_tsquery('english', $6)
            ORDER BY lexical_score DESC,
                     embedded_at DESC,
                     source_kind ASC,
                     source_id ASC,
                     chunk_index ASC,
                     modality ASC
            LIMIT $7"#,
    )
    .bind(org_id)
    .bind(namespace)
    .bind(filter.modalities)
    .bind(filter.source_kinds)
    .bind(filter.source_ids)
    .bind(trimmed)
    .bind(depth)
    .fetch_all(executor)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let modality_str: String = row.try_get("modality")?;
        let Some(modality) = RagModality::parse(&modality_str) else {
            continue;
        };
        let key = RagChunkKey {
            source_kind: row.try_get("source_kind")?,
            source_id: row.try_get("source_id")?,
            chunk_index: row.try_get("chunk_index")?,
            modality,
        };
        let lexical_score: f64 = row.try_get("lexical_score")?;
        out.push(RagLexicalCandidate { key, lexical_score });
    }
    Ok(out)
}

/// Runs the exact-KNN cosine distance (`embedding <=> $vec`) candidate leg
/// within `(org_id, namespace)`.
pub async fn vector_candidates<'e, E>(
    executor: E,
    org_id: Uuid,
    namespace: &str,
    filter: RagFilter<'_>,
    query_vector: &[f32],
    depth: i64,
) -> DbResult<Vec<RagVectorCandidate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    validate_embedding_vector("query_vector", query_vector).map_err(|e| {
        crate::DbError::Sql(sqlx::Error::Protocol(format!("invalid query_vector: {e}")))
    })?;
    let vec_literal = embedding_to_pg(query_vector);
    let rows = sqlx::query(
        r#"SELECT source_kind,
                  source_id,
                  chunk_index,
                  modality,
                  (embedding <=> $6::text::vector)::float8 AS vector_distance
             FROM rag_embeddings
            WHERE org_id = $1
              AND namespace = $2
              AND (cardinality($3::text[]) = 0 OR modality = ANY($3))
              AND (cardinality($4::text[]) = 0 OR source_kind = ANY($4))
              AND (cardinality($5::text[]) = 0 OR source_id = ANY($5))
            ORDER BY embedding <=> $6::text::vector ASC,
                     source_kind ASC,
                     source_id ASC,
                     chunk_index ASC,
                     modality ASC
            LIMIT $7"#,
    )
    .bind(org_id)
    .bind(namespace)
    .bind(filter.modalities)
    .bind(filter.source_kinds)
    .bind(filter.source_ids)
    .bind(&vec_literal)
    .bind(depth)
    .fetch_all(executor)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let modality_str: String = row.try_get("modality")?;
        let Some(modality) = RagModality::parse(&modality_str) else {
            continue;
        };
        let key = RagChunkKey {
            source_kind: row.try_get("source_kind")?,
            source_id: row.try_get("source_id")?,
            chunk_index: row.try_get("chunk_index")?,
            modality,
        };
        let vector_distance: f64 = row.try_get("vector_distance")?;
        out.push(RagVectorCandidate {
            key,
            vector_distance,
        });
    }
    Ok(out)
}

/// Executes hybrid BM25 + exact-KNN retrieval within `(org_id, namespace)`,
/// fuses the active legs with Reciprocal Rank Fusion (RRF), and hydrates full
/// attribution metadata for the top `limit` hits.
pub async fn query_hybrid(
    conn: &mut PgConnection,
    params: RagQueryParams<'_>,
) -> DbResult<Vec<RagQueryHit>> {
    let limit = params.limit.clamp(1, core_domain::rag::MAX_RAG_QUERY_LIMIT);
    let depth = candidate_depth(limit);
    let modalities: Vec<String> = params
        .modalities
        .iter()
        .map(|m| m.as_str().to_string())
        .collect();
    let filter = RagFilter {
        modalities: &modalities,
        source_kinds: params.source_kinds,
        source_ids: params.source_ids,
    };

    let lexical = match params.query_text {
        Some(q) if !q.trim().is_empty() => {
            lexical_candidates(
                &mut *conn,
                params.org_id,
                params.namespace,
                filter,
                q,
                depth,
            )
            .await?
        }
        _ => Vec::new(),
    };

    let vector = match params.query_vector {
        Some(vec) => {
            vector_candidates(
                &mut *conn,
                params.org_id,
                params.namespace,
                filter,
                vec,
                depth,
            )
            .await?
        }
        None => Vec::new(),
    };

    let fused = fuse_rag_candidates(&lexical, &vector);
    let top: Vec<_> = fused
        .into_iter()
        .take(usize::try_from(limit).unwrap_or(20))
        .collect();
    if top.is_empty() {
        return Ok(Vec::new());
    }

    let kinds: Vec<String> = top.iter().map(|c| c.key.source_kind.clone()).collect();
    let ids: Vec<String> = top.iter().map(|c| c.key.source_id.clone()).collect();
    let indices: Vec<i32> = top.iter().map(|c| c.key.chunk_index).collect();
    let mods: Vec<String> = top
        .iter()
        .map(|c| c.key.modality.as_str().to_string())
        .collect();

    let rows = sqlx::query(
        r#"SELECT r.namespace,
                  r.source_kind,
                  r.source_id,
                  r.chunk_index,
                  r.modality,
                  r.model,
                  r.content_hash,
                  r.content_text,
                  r.metadata,
                  r.embedded_at
             FROM rag_embeddings r
             JOIN unnest($3::text[], $4::text[], $5::int4[], $6::text[])
               AS k(source_kind, source_id, chunk_index, modality)
               ON r.source_kind = k.source_kind
              AND r.source_id   = k.source_id
              AND r.chunk_index = k.chunk_index
              AND r.modality    = k.modality
            WHERE r.org_id = $1
              AND r.namespace = $2"#,
    )
    .bind(params.org_id)
    .bind(params.namespace)
    .bind(&kinds)
    .bind(&ids)
    .bind(&indices)
    .bind(&mods)
    .fetch_all(&mut *conn)
    .await?;

    struct HydratedRow {
        namespace: String,
        model: String,
        content_hash: String,
        content_text: Option<String>,
        metadata: serde_json::Value,
        embedded_at: DateTime<Utc>,
    }

    let mut by_key: HashMap<RagChunkKey, HydratedRow> = HashMap::with_capacity(rows.len());
    for row in rows {
        let modality_str: String = row.try_get("modality")?;
        let Some(modality) = RagModality::parse(&modality_str) else {
            continue;
        };
        let key = RagChunkKey {
            source_kind: row.try_get("source_kind")?,
            source_id: row.try_get("source_id")?,
            chunk_index: row.try_get("chunk_index")?,
            modality,
        };
        by_key.insert(
            key,
            HydratedRow {
                namespace: row.try_get("namespace")?,
                model: row.try_get("model")?,
                content_hash: row.try_get("content_hash")?,
                content_text: row.try_get("content_text")?,
                metadata: row.try_get("metadata")?,
                embedded_at: row.try_get("embedded_at")?,
            },
        );
    }

    let mut hits = Vec::with_capacity(top.len());
    for cand in top {
        let Some(hydrated) = by_key.remove(&cand.key) else {
            continue;
        };
        hits.push(RagQueryHit {
            namespace: hydrated.namespace,
            source_kind: cand.key.source_kind,
            source_id: cand.key.source_id,
            chunk_index: cand.key.chunk_index,
            modality: cand.key.modality,
            model: hydrated.model,
            content_hash: hydrated.content_hash,
            score: cand.score,
            vector_distance: cand.vector_distance,
            lexical_score: cand.lexical_score,
            content_text: hydrated.content_text,
            metadata: hydrated.metadata,
            embedded_at: hydrated.embedded_at,
        });
    }

    Ok(hits)
}

/// Deletes all chunks for `(org_id, namespace, source_kind, source_id)` and
/// records a provable [`RagDeletionReceipt`] in `rag_deletion_receipts` inside
/// the same transaction. Even if 0 chunks existed at the time of the call
/// (e.g. idempotent retry of a DSAR erasure), a receipt with
/// `deleted_chunks = 0` is minted so the caller has a verifiable timestamped
/// proof that the source has zero vectors in `(org_id, namespace)`.
pub async fn delete_source_in_namespace(
    conn: &mut PgConnection,
    org_id: Uuid,
    namespace: &str,
    source_kind: &str,
    source_id: &str,
    deleted_by: Option<Uuid>,
) -> DbResult<RagDeletionReceipt> {
    let receipt_id = Uuid::now_v7();
    let row = sqlx::query(
        r#"WITH removed AS (
             DELETE FROM rag_embeddings
              WHERE org_id = $1
                AND namespace = $2
                AND source_kind = $3
                AND source_id = $4
             RETURNING chunk_index, modality, content_hash
           ),
           agg AS (
             SELECT
               count(*)::integer AS deleted_chunks,
               COALESCE(
                 array_agg(content_hash ORDER BY chunk_index, modality),
                 '{}'::text[]
               ) AS content_hashes
             FROM removed
           )
           INSERT INTO rag_deletion_receipts (
             receipt_id,
             org_id,
             namespace,
             source_kind,
             source_id,
             deleted_chunks,
             content_hashes,
             deleted_by
           )
           SELECT $5, $1, $2, $3, $4, agg.deleted_chunks, agg.content_hashes, $6
             FROM agg
           RETURNING receipt_id, org_id, namespace, source_kind, source_id,
                     deleted_chunks, content_hashes, deleted_by, deleted_at"#,
    )
    .bind(org_id)
    .bind(namespace)
    .bind(source_kind)
    .bind(source_id)
    .bind(receipt_id)
    .bind(deleted_by)
    .fetch_one(&mut *conn)
    .await?;

    row_to_receipt(&row)
}

/// Fetches a single deletion receipt by `receipt_id` under RLS.
pub async fn get_deletion_receipt<'e, E>(
    executor: E,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<Option<RagDeletionReceipt>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT receipt_id, org_id, namespace, source_kind, source_id,
                  deleted_chunks, content_hashes, deleted_by, deleted_at
             FROM rag_deletion_receipts
            WHERE org_id = $1 AND receipt_id = $2"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_optional(executor)
    .await?;

    row.as_ref().map(row_to_receipt).transpose()
}

/// Lists deletion receipts for an org (optionally filtered by `namespace`,
/// `source_kind`, and `source_id`), newest first.
pub async fn list_deletion_receipts<'e, E>(
    executor: E,
    org_id: Uuid,
    namespace: Option<&str>,
    source_kind: Option<&str>,
    source_id: Option<&str>,
    limit: i64,
) -> DbResult<Vec<RagDeletionReceipt>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let clamped_limit = limit.clamp(1, 200);
    let rows = sqlx::query(
        r#"SELECT receipt_id, org_id, namespace, source_kind, source_id,
                  deleted_chunks, content_hashes, deleted_by, deleted_at
             FROM rag_deletion_receipts
            WHERE org_id = $1
              AND ($2::text IS NULL OR namespace = $2)
              AND ($3::text IS NULL OR source_kind = $3)
              AND ($4::text IS NULL OR source_id = $4)
            ORDER BY deleted_at DESC, receipt_id DESC
            LIMIT $5"#,
    )
    .bind(org_id)
    .bind(namespace)
    .bind(source_kind)
    .bind(source_id)
    .bind(clamped_limit)
    .fetch_all(executor)
    .await?;

    rows.iter().map(row_to_receipt).collect()
}

fn row_to_receipt(row: &sqlx::postgres::PgRow) -> DbResult<RagDeletionReceipt> {
    Ok(RagDeletionReceipt {
        receipt_id: row.try_get("receipt_id")?,
        org_id: row.try_get("org_id")?,
        namespace: row.try_get("namespace")?,
        source_kind: row.try_get("source_kind")?,
        source_id: row.try_get("source_id")?,
        deleted_chunks: row.try_get("deleted_chunks")?,
        content_hashes: row.try_get("content_hashes")?,
        deleted_by: row.try_get("deleted_by")?,
        deleted_at: row.try_get("deleted_at")?,
    })
}
