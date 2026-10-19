//! Search index queries (TASKLIST 2.16): the single per-entity FTS table
//! backing GET /api/search, plus the pgvector leg (migration 0062) that the
//! two legs' reciprocal rank fusion folds together (§4.5).
//!
//! Purpose: exactly-once upsert/delete of indexed entities (tickets,
//! comments, clients, sites, contacts) and the ranked websearch+trigram
//! query; the embedding side table's idempotent upsert + backfill sweep;
//! and the hybrid page/count that fuses both legs. Writes are performed
//! ONLY by core-api's internal effect surface on behalf of the worker's
//! event-driven indexer (the backfill included); reads always filter by
//! org plus an explicit kind allow-list derived from permissions.
//!
//! Non-purpose: no embedding generation (core-api resolves the org's
//! `CompletionProvider` through the BYOK seam and feeds the vectors back),
//! no permission decisions, and no domain rules — those live in
//! core-domain/core-api.

use crate::DbResult;
use core_domain::search::{reciprocal_rank_fusion, SearchKey};
use sqlx::{PgConnection, Postgres};
use std::collections::HashMap;
use uuid::Uuid;

/// The indexed entity kinds; mirrors the `search_index.entity_kind` CHECK
/// (vocabulary widened once in 0031 for the Wave-8b kinds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    Ticket,
    Comment,
    Client,
    Site,
    Contact,
    /// Native KB article (TASKLIST Phase 3 native KB v1).
    KbArticle,
    /// Document, indexed by its CURRENT version (TASKLIST D4).
    Document,
}

impl SearchKind {
    pub const ALL: [SearchKind; 7] = [
        SearchKind::Ticket,
        SearchKind::Comment,
        SearchKind::Client,
        SearchKind::Site,
        SearchKind::Contact,
        SearchKind::KbArticle,
        SearchKind::Document,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SearchKind::Ticket => "ticket",
            SearchKind::Comment => "comment",
            SearchKind::Client => "client",
            SearchKind::Site => "site",
            SearchKind::Contact => "contact",
            SearchKind::KbArticle => "kb_article",
            SearchKind::Document => "document",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ticket" => SearchKind::Ticket,
            "comment" => SearchKind::Comment,
            "client" => SearchKind::Client,
            "site" => SearchKind::Site,
            "contact" => SearchKind::Contact,
            "kb_article" => SearchKind::KbArticle,
            "document" => SearchKind::Document,
            _ => return None,
        })
    }
}

/// Upserts one index row. Replaying the same (org, kind, entity) is
/// idempotent — the row count never grows on retries. `client_scope`
/// carries the document's client scoping (0089): set ONLY for the
/// `document` kind, mirroring `documents.client_id` (NULL = org-global);
/// ON DELETE SET NULL propagates client deletion into scope reassignment.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_row<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
    parent_id: Option<Uuid>,
    title: &str,
    body: &str,
    client_scope: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body, client_scope)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             parent_id = EXCLUDED.parent_id,
             title = EXCLUDED.title,
             body = EXCLUDED.body,
             client_scope = EXCLUDED.client_scope,
             indexed_at = now()"#,
        Uuid::now_v7(),
        org_id,
        kind.as_str(),
        entity_id,
        parent_id,
        title,
        body,
        client_scope
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Removes one index row; returns whether a row was actually deleted.
pub async fn delete_row<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM search_index WHERE org_id = $1 AND entity_kind = $2 AND entity_id = $3",
        org_id,
        kind.as_str(),
        entity_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// One stored index row, by identity — the pin-upsert resolution reads the
/// live title + parent + scope snapshot here (0089): a pin is a stored
/// result, so pinning REQUIRES the row to exist in the index.
#[derive(Debug, Clone)]
pub struct IndexedRow {
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub client_scope: Option<Uuid>,
}

pub async fn lookup_row<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
) -> DbResult<Option<IndexedRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        "SELECT parent_id, title, client_scope FROM search_index \
         WHERE org_id = $1 AND entity_kind = $2 AND entity_id = $3",
        org_id,
        kind.as_str(),
        entity_id
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| IndexedRow {
        parent_id: r.parent_id,
        title: r.title,
        client_scope: r.client_scope,
    }))
}

/// One ranked hit. `body_excerpt` is raw text for client-side snippets;
/// `score` blends FTS rank (title/body weights) with trigram similarity.
///
/// `indexed_at` is not part of the client contract. It rides along because it
/// is the second member of the sort tuple and a keyset cursor is built from
/// the last row served; the route drops it when it maps to `SearchResult`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub body_excerpt: String,
    pub indexed_at: chrono::DateTime<chrono::Utc>,
    pub score: f64,
}

/// Keyset anchor for a ranked walk: the full sort tuple of the last row
/// served, `(score DESC, indexed_at DESC, entity_id ASC)`.
///
/// `entity_id` is the final tiebreaker because score and index time both
/// collide in real data: a body repeated on thirty rows gives thirty equal
/// ranks. Without it a cursor could resume inside a tie and skip or repeat
/// rows.
///
/// `indexed_at` and `score` are the only mutable members — re-indexing a row
/// sets `indexed_at = now()` and can change its score. A row re-indexed
/// mid-walk can therefore move across the anchor. The walk guarantees no
/// duplicates and page-1 cost, not a frozen snapshot; a client that needs an
/// exact set reconciles by id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchCursor {
    pub score: f64,
    pub indexed_at: chrono::DateTime<chrono::Utc>,
    pub entity_id: Uuid,
}

/// Ranking expression for the ranked page.
///
/// `$3` is the query text and `$4` enables the trigram fallback. It is
/// written once because a cursor compares against it: if the page and the
/// cursor disagreed by so much as a cast, the anchor row would not compare
/// equal and a walk would skip a row.
const SCORE_EXPR: &str = "(coalesce(ts_rank(tsv, websearch_to_tsquery('english', $3)), 0) * 8
    + CASE WHEN $4 THEN coalesce(similarity(title || ' ' || body, $3), 0) ELSE 0 END)";

/// Ranked page of hits. `kinds` must be non-empty (callers derive it from
/// the principal's permissions); `trigram` enables the partial-token ILIKE
/// leg used for prefix/partial tokens. `client_id` scopes the `document`
/// kind (0089): when set, indexed documents answer only if they are
/// org-global (`client_scope` IS NULL) or belong to the named client —
/// every other kind is untouched.
#[allow(clippy::too_many_arguments)]
pub async fn search_page<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<SearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    search_page_paged(
        executor, org_id, kinds, q, trigram, client_id, limit, offset, None,
    )
    .await
}

/// Ranked page strictly after `cursor` — the keyset walk (docs/perf.md F2).
///
/// `offset` has no meaning here: a cursor page resumes from an anchor row,
/// so it never scans what it is not returning. `None` means the first page,
/// which is the same as [`search_page`] with offset zero.
///
/// This is a separate function rather than another argument on
/// [`search_page`] so the legacy offset callers — and their regression
/// tests — keep the exact shape they had. The two mechanisms answer
/// different clients during the migration window.
#[allow(clippy::too_many_arguments)]
pub async fn search_page_after<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    limit: i64,
    cursor: Option<SearchCursor>,
) -> DbResult<Vec<SearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    search_page_paged(
        executor, org_id, kinds, q, trigram, client_id, limit, 0, cursor,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn search_page_paged<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    limit: i64,
    offset: i64,
    cursor: Option<SearchCursor>,
) -> DbResult<Vec<SearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // The score is a computed expression, so the keyset predicate cannot
    // name a column alias. The ranked rows are wrapped in a subquery and the
    // predicate compares the projected `score`; the expression itself is
    // still written once, in `SCORE_EXPR`.
    let keyset_clause = if cursor.is_some() {
        r#"WHERE (score < $8
                  OR (score = $8 AND indexed_at < $9)
                  OR (score = $8 AND indexed_at = $9 AND entity_id > $10))"#
    } else {
        ""
    };
    let sql = format!(
        r#"SELECT entity_kind, entity_id, parent_id, title, body_excerpt, indexed_at, score
           FROM (
             SELECT entity_kind, entity_id, parent_id, title,
                    left(body, 512) AS body_excerpt,
                    indexed_at,
                    ({SCORE_EXPR})::double precision AS score
               FROM search_index
              WHERE org_id = $1
                AND entity_kind = ANY($2)
                AND (
                  tsv @@ websearch_to_tsquery('english', $3)
                  OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
                )
                AND (
                  entity_kind <> 'document'
                  OR $5::uuid IS NULL
                  OR client_scope IS NULL
                  OR client_scope = $5
                )
           ) ranked
           {keyset_clause}
           ORDER BY score DESC, indexed_at DESC, entity_id
           LIMIT $6 OFFSET $7"#
    );
    let mut query = sqlx::query(&sql)
        .bind(org_id)
        .bind(kinds)
        .bind(q)
        .bind(trigram)
        .bind(client_id)
        .bind(limit)
        .bind(offset);
    if let Some(anchor) = cursor {
        query = query
            .bind(anchor.score)
            .bind(anchor.indexed_at)
            .bind(anchor.entity_id);
    }
    let rows = query.fetch_all(executor).await?;
    use sqlx::Row as _;
    Ok(rows
        .into_iter()
        .map(|row| SearchHit {
            entity_kind: row.get("entity_kind"),
            entity_id: row.get("entity_id"),
            parent_id: row.get("parent_id"),
            title: row.get("title"),
            body_excerpt: row.get("body_excerpt"),
            indexed_at: row.get("indexed_at"),
            score: row.get("score"),
        })
        .collect())
}

/// How far a count scans before it reports "at least this many".
///
/// The exact number is pagination metadata, and computing it means scanning
/// the whole predicate — the second half of the OFFSET debt this lane
/// retired. A cap turns that into "is the result set bigger than a page or
/// two", which is all the UI uses it for.
pub const SEARCH_COUNT_CAP: i64 = 1_000;

/// A possibly-capped result count. `capped` says the real number is at least
/// `total`; it is false only when the whole predicate was counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchCount {
    pub total: i64,
    pub capped: bool,
}

fn count_from(n: i64, cap: i64) -> SearchCount {
    SearchCount {
        total: n.min(cap),
        capped: n > cap,
    }
}

/// Total matching rows for the same predicate — powers result counts and
/// pagination metadata without fetching the page.
pub async fn search_count<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    cap: i64,
) -> DbResult<SearchCount>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // `cap + 1` is what distinguishes "exactly cap" from "more than cap"
    // without counting the rest of the predicate.
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS total
           FROM (
             SELECT 1
               FROM search_index
              WHERE org_id = $1
                AND entity_kind = ANY($2)
                AND (
                  tsv @@ websearch_to_tsquery('english', $3)
                  OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
                )
                AND (
                  entity_kind <> 'document'
                  OR $5::uuid IS NULL
                  OR client_scope IS NULL
                  OR client_scope = $5
                )
              LIMIT $6
           ) capped"#,
        org_id,
        kinds,
        q,
        trigram,
        client_id,
        cap.saturating_add(1)
    )
    .fetch_one(executor)
    .await?;
    Ok(count_from(row.total.unwrap_or(0), cap))
}

// --- vector leg (migration 0062) ---------------------------------------------
//
// Embeddings live in the `search_embeddings` side table keyed by the same
// (org, kind, entity) identity as `search_index`, with a model + content
// hash for provenance. The content hash is computed SQL-side as
// md5(title || E'\x1f' || body) so the "needs embedding" predicate and the
// stored hash can never drift — one definition, in one place.

/// How many rows a backfill/upsert sweep may carry per provider batch.
pub const EMBEDDING_BATCH: i64 = 128;

/// Renders a `Vec<f32>` as pgvector's text literal — the single seam
/// between the domain's vectors and Postgres's `vector` type (no vendor
/// crate crosses this seam; sqlx binds it as `$n::text::vector`).
#[must_use]
pub fn embedding_to_pg(embedding: &[f32]) -> String {
    let mut out = String::with_capacity(embedding.len() * 10 + 2);
    out.push('[');
    for (i, value) in embedding.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{value}"));
    }
    out.push(']');
    out
}

/// One index row whose embedding is missing or stale: the backfill unit.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingTarget {
    pub kind: SearchKind,
    pub entity_id: Uuid,
    pub title: String,
    pub body: String,
}

/// Rows whose vector is absent, was produced by a different model, or whose
/// searchable text has moved on (content hash mismatch). Deterministic
/// order (oldest first); running the backfill twice is a no-op the second
/// time — the predicate excludes freshly embedded rows by construction.
pub async fn rows_needing_embedding<'e, E>(
    executor: E,
    org_id: Uuid,
    model: &str,
    limit: i64,
) -> DbResult<Vec<EmbeddingTarget>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT s.entity_kind, s.entity_id, s.title, s.body
           FROM search_index s
           LEFT JOIN search_embeddings e
             ON e.org_id = s.org_id
            AND e.entity_kind = s.entity_kind
            AND e.entity_id = s.entity_id
           WHERE s.org_id = $1
             AND (
                   e.id IS NULL
                OR e.model <> $2
                OR e.content_hash <> md5(s.title || E'\x1f' || s.body)
              )
           ORDER BY s.indexed_at, s.entity_id
           LIMIT $3"#,
        org_id,
        model,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let kind = SearchKind::parse(&row.entity_kind)?;
            Some(EmbeddingTarget {
                kind,
                entity_id: row.entity_id,
                title: row.title,
                body: row.body,
            })
        })
        .collect())
}

/// Upserts one vector. The content hash is computed from the index row's
/// ACTUAL title/body via the same SQL-side definition the needs-embedding
/// sweep compares against — the caller can never store a hash that drifts
/// from the text. Replaying the same (org, kind, entity) re-upserts one
/// row; a row with no index entry (deleted mid-flight) stores nothing.
pub async fn upsert_embedding<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
    model: &str,
    embedding: &[f32],
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        r#"INSERT INTO search_embeddings
             (id, org_id, entity_kind, entity_id, model, content_hash, embedding)
           SELECT $1, $2, $3, $4, $5,
                  md5(s.title || E'\x1f' || s.body),
                  $6::text::vector
           FROM search_index s
           WHERE s.org_id = $2 AND s.entity_kind = $3 AND s.entity_id = $4
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             model = EXCLUDED.model,
             content_hash = EXCLUDED.content_hash,
             embedding = EXCLUDED.embedding,
             embedded_at = now()"#,
        Uuid::now_v7(),
        org_id,
        kind.as_str(),
        entity_id,
        model,
        embedding_to_pg(embedding)
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Removes one vector (the source row is gone — the delete leg).
pub async fn delete_embedding<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM search_embeddings WHERE org_id = $1 AND entity_kind = $2 AND entity_id = $3",
        org_id,
        kind.as_str(),
        entity_id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

// --- hybrid retrieval (§4.5: RRF fusion of the two legs) ---------------------

/// Candidate depth per leg: deep enough that a page deep in the result set
/// still fuses over both legs, shallow enough to stay cheap. RRF over the
/// top candidates only — the tail beyond this is deliberately unscored.
fn candidate_limit(limit: i64, offset: i64) -> i64 {
    ((limit + offset) * 4).clamp(50, 400)
}

/// Lexical leg: the SAME predicate and ordering as [`search_page`], reduced
/// to keys, so fusion can never reorder the FTS leg's own semantics.
pub async fn fts_candidates<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<SearchKey>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT entity_kind, entity_id
           FROM search_index
           WHERE org_id = $1
             AND entity_kind = ANY($2)
             AND (
               tsv @@ websearch_to_tsquery('english', $3)
               OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
             )
             AND (
               entity_kind <> 'document'
               OR $5::uuid IS NULL
               OR client_scope IS NULL
               OR client_scope = $5
             )
           ORDER BY ts_rank(tsv, websearch_to_tsquery('english', $3)) * 8
                      + CASE WHEN $4 THEN similarity(title || ' ' || body, $3)
                             ELSE 0 END DESC,
                    indexed_at DESC, entity_id
           LIMIT $6"#,
        org_id,
        kinds,
        q,
        trigram,
        client_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SearchKey::new(row.entity_kind, row.entity_id))
        .collect())
}

/// Vector leg: cosine-distance nearest neighbors within the org's rows
/// (RLS applies on top — reads are tenant-scoped by the policy, and the
/// explicit org filter keeps the exact-KNN scan bounded). Exact search, no
/// ANN index: see migration 0062 for why that is deliberate in v1.
pub async fn vector_candidates<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    embedding: &[f32],
    client_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<SearchKey>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Embeddings carry no scope of their own — the document scoping is
    // re-derived from the mirrored search_index row (an orphaned embedding
    // of a deleted document cannot answer; the EXISTS resolves the scope).
    let rows = sqlx::query!(
        r#"SELECT e.entity_kind, e.entity_id
           FROM search_embeddings e
           WHERE e.org_id = $1
             AND e.entity_kind = ANY($2)
             AND (
               e.entity_kind <> 'document'
               OR $4::uuid IS NULL
               OR EXISTS (
                 SELECT 1 FROM search_index s
                  WHERE s.org_id = e.org_id
                    AND s.entity_kind = e.entity_kind
                    AND s.entity_id = e.entity_id
                    AND (s.client_scope IS NULL OR s.client_scope = $4)
               )
             )
           ORDER BY e.embedding <=> $3::text::vector, e.entity_kind, e.entity_id
           LIMIT $5"#,
        org_id,
        kinds,
        embedding_to_pg(embedding),
        client_id,
        limit
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SearchKey::new(row.entity_kind, row.entity_id))
        .collect())
}

/// Row content for a page of fused keys, in the fused order (the order the
/// fusion produced is the contract; SQL returns by key membership only).
async fn hydrate_keys<'e, E>(
    executor: E,
    org_id: Uuid,
    keys: &[SearchKey],
    scores: &HashMap<(String, Uuid), f64>,
) -> DbResult<Vec<SearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let kinds: Vec<String> = keys.iter().map(|k| k.entity_kind.clone()).collect();
    let ids: Vec<Uuid> = keys.iter().map(|k| k.entity_id).collect();
    let rows = sqlx::query!(
        r#"SELECT s.entity_kind, s.entity_id, s.parent_id, s.title,
                  left(s.body, 512) AS body_excerpt, s.indexed_at
           FROM search_index s
           JOIN unnest($2::text[], $3::uuid[]) AS k(kind, id)
             ON s.entity_kind = k.kind AND s.entity_id = k.id
           WHERE s.org_id = $1"#,
        org_id,
        &kinds,
        &ids
    )
    .fetch_all(executor)
    .await?;
    let mut by_key: HashMap<(String, Uuid), _> = rows
        .into_iter()
        .map(|row| ((row.entity_kind.clone(), row.entity_id), row))
        .collect();
    let mut hits = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(row) = by_key.remove(&(key.entity_kind.clone(), key.entity_id)) else {
            continue; // source row vanished between legs and hydration
        };
        hits.push(SearchHit {
            entity_kind: row.entity_kind,
            entity_id: row.entity_id,
            parent_id: row.parent_id,
            title: row.title,
            body_excerpt: row.body_excerpt.unwrap_or_default(),
            indexed_at: row.indexed_at,
            score: scores
                .get(&(key.entity_kind.clone(), key.entity_id))
                .copied()
                .unwrap_or(0.0),
        });
    }
    Ok(hits)
}

/// Anchor for a fused walk: the fused score and the key of the last row
/// served, in the order `reciprocal_rank_fusion` produces (score descending,
/// then `(entity_kind, entity_id)` ascending — the fusion's own tiebreak).
///
/// A fused walk is exact only over the CANDIDATE WINDOW, which is why
/// [`search_hybrid_page_after`] fuses with the page-1 window: every page of a
/// walk sees the same candidates, so the ranks that produce the scores do not
/// move between pages. The window is bounded (`candidate_limit`, at most 400),
/// so a walk ends when the window does — and `has_more` says so instead of the
/// old silent empty page.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedCursor {
    pub score: f64,
    pub entity_kind: String,
    pub entity_id: Uuid,
}

/// One fused page plus whether the window holds more rows after it.
#[derive(Debug, Clone)]
pub struct FusedPage {
    pub hits: Vec<SearchHit>,
    pub has_more: bool,
}

/// Is `(score, key)` strictly after `anchor` in the fused order?
fn fused_after(score: f64, key: &SearchKey, anchor: &FusedCursor) -> bool {
    if score < anchor.score {
        return true;
    }
    if score > anchor.score {
        return false;
    }
    (&key.entity_kind, key.entity_id) > (&anchor.entity_kind, anchor.entity_id)
}

/// The hybrid page: both legs ranked independently, fused by RRF in
/// core-domain, paginated, then hydrated. Score scale differs from the
/// legacy FTS path (RRF shares, higher is better) — same response shape.
/// `client_id` scopes the `document` kind in BOTH legs (0089), exactly as
/// the legacy FTS path documents it. Runs over one connection
/// (`&mut PgConnection`) because it issues several queries; callers pass
/// an RLS-scoped transaction exactly as elsewhere.
#[allow(clippy::too_many_arguments)]
pub async fn search_hybrid_page(
    tx: &mut PgConnection,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    embedding: &[f32],
    limit: i64,
    offset: i64,
) -> DbResult<Vec<SearchHit>> {
    Ok(hybrid_page_inner(
        tx, org_id, kinds, q, trigram, client_id, embedding, limit, offset, None,
    )
    .await?
    .hits)
}

/// Offset page plus `has_more`, for the legacy hybrid path.
///
/// `search_hybrid_page` is this with the flag dropped; the route needs the
/// flag to decide whether a page hands out a cursor, and an approximation from
/// `len == limit` would hand out one more cursor than the window holds.
#[allow(clippy::too_many_arguments)]
pub async fn search_hybrid_page_paged(
    tx: &mut PgConnection,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    embedding: &[f32],
    limit: i64,
    offset: i64,
) -> DbResult<FusedPage> {
    hybrid_page_inner(
        tx, org_id, kinds, q, trigram, client_id, embedding, limit, offset, None,
    )
    .await
}

/// Fused page strictly after `cursor`, over the page-1 candidate window.
///
/// `has_more` is exact: the inner query asks for one row past the page, so a
/// full final page reports `false` rather than handing out a cursor that would
/// return an empty page.
#[allow(clippy::too_many_arguments)]
pub async fn search_hybrid_page_after(
    tx: &mut PgConnection,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    embedding: &[f32],
    limit: i64,
    cursor: Option<FusedCursor>,
) -> DbResult<FusedPage> {
    hybrid_page_inner(
        tx, org_id, kinds, q, trigram, client_id, embedding, limit, 0, cursor,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn hybrid_page_inner(
    tx: &mut PgConnection,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    embedding: &[f32],
    limit: i64,
    offset: i64,
    cursor: Option<FusedCursor>,
) -> DbResult<FusedPage> {
    // A cursor page must fuse the same candidates as the page it continues,
    // so it uses the page-1 window regardless of the offset the first request
    // carried. The legacy offset path keeps the historical widening window.
    let depth = candidate_limit(limit, if cursor.is_some() { 0 } else { offset });
    let lexical = fts_candidates(&mut *tx, org_id, kinds, q, trigram, client_id, depth).await?;
    let vector = vector_candidates(&mut *tx, org_id, kinds, embedding, client_id, depth).await?;
    let fused = reciprocal_rank_fusion(&[&lexical, &vector]);

    let want = usize::try_from(limit.max(0)).unwrap_or(0);
    let page: Vec<(SearchKey, f64)> = match &cursor {
        None => fused
            .iter()
            .skip(usize::try_from(offset.max(0)).unwrap_or(usize::MAX))
            .take(want + 1)
            .cloned()
            .collect(),
        Some(anchor) => fused
            .iter()
            .filter(|(k, s)| fused_after(*s, k, anchor))
            .take(want + 1)
            .cloned()
            .collect(),
    };
    let has_more = page.len() > want;
    let page: Vec<(SearchKey, f64)> = page.into_iter().take(want).collect();
    if page.is_empty() {
        return Ok(FusedPage {
            hits: Vec::new(),
            has_more: false,
        });
    }
    let keys: Vec<SearchKey> = page.iter().map(|(k, _)| k.clone()).collect();
    let scores: HashMap<(String, Uuid), f64> = page
        .iter()
        .map(|(k, s)| ((k.entity_kind.clone(), k.entity_id), *s))
        .collect();
    let hits = hydrate_keys(&mut *tx, org_id, &keys, &scores).await?;
    Ok(FusedPage { hits, has_more })
}

/// Total distinct matches across both legs — the union the fused page is
/// drawn from, powering result counts and pagination metadata. `client_id`
/// scopes the `document` kind on BOTH union legs (0089 — the embeddings
/// leg re-derives the scope from the mirrored index row, so an embedded
/// out-of-scope document never inflates the count).
pub async fn search_hybrid_count<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    client_id: Option<Uuid>,
    cap: i64,
) -> DbResult<SearchCount>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // The vector leg has no text predicate — every embedded row of the
    // permitted kinds is a vector candidate — so the union is exactly:
    // lexical matches ∪ embedded rows of those kinds. The cap bounds the
    // union scan the same way `search_count` bounds the lexical one.
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS total FROM (
             SELECT entity_kind, entity_id FROM search_index
              WHERE org_id = $1 AND entity_kind = ANY($2)
                AND (
                  tsv @@ websearch_to_tsquery('english', $3)
                  OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
                )
                AND (
                  entity_kind <> 'document'
                  OR $5::uuid IS NULL
                  OR client_scope IS NULL
                  OR client_scope = $5
                )
             UNION
             SELECT e.entity_kind, e.entity_id FROM search_embeddings e
              WHERE e.org_id = $1 AND e.entity_kind = ANY($2)
                AND (
                  e.entity_kind <> 'document'
                  OR $5::uuid IS NULL
                  OR EXISTS (
                    SELECT 1 FROM search_index s
                     WHERE s.org_id = e.org_id
                       AND s.entity_kind = e.entity_kind
                       AND s.entity_id = e.entity_id
                       AND (s.client_scope IS NULL OR s.client_scope = $5)
                  )
                )
             LIMIT $6
           ) u"#,
        org_id,
        kinds,
        q,
        trigram,
        client_id,
        cap.saturating_add(1)
    )
    .fetch_one(executor)
    .await?;
    Ok(count_from(row.total.unwrap_or(0), cap))
}
