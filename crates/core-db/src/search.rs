//! Search index queries (TASKLIST 2.16): the single per-entity FTS table
//! backing GET /api/search.
//!
//! Purpose: exactly-once upsert/delete of indexed entities (tickets,
//! comments, clients, sites, contacts) and the ranked websearch+trigram
//! query. Writes are performed ONLY by core-api's internal effect surface
//! on behalf of the worker's event-driven indexer; reads always filter by
//! org plus an explicit kind allow-list derived from permissions.
//!
//! Non-purpose: no vector/embedding ranking (Stage 3 grows the pgvector leg
//! behind this same seam and fuses with RRF), no permission decisions, and
//! no domain rules — those live in core-domain/core-api.

use crate::DbResult;
use sqlx::Postgres;
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
/// idempotent — the row count never grows on retries.
pub async fn upsert_row<'e, E>(
    executor: E,
    org_id: Uuid,
    kind: SearchKind,
    entity_id: Uuid,
    parent_id: Option<Uuid>,
    title: &str,
    body: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO search_index (id, org_id, entity_kind, entity_id, parent_id, title, body)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (org_id, entity_kind, entity_id) DO UPDATE SET
             parent_id = EXCLUDED.parent_id,
             title = EXCLUDED.title,
             body = EXCLUDED.body,
             indexed_at = now()"#,
        Uuid::now_v7(),
        org_id,
        kind.as_str(),
        entity_id,
        parent_id,
        title,
        body
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

/// One ranked hit. `body_excerpt` is raw text for client-side snippets;
/// `score` blends FTS rank (title/body weights) with trigram similarity.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub body_excerpt: String,
    pub score: f64,
}

/// Ranked page of hits. `kinds` must be non-empty (callers derive it from
/// the principal's permissions); `trigram` enables the partial-token ILIKE
/// leg used for prefix/partial tokens.
pub async fn search_page<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<SearchHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT entity_kind, entity_id, parent_id, title,
                  left(body, 512) AS body_excerpt,
                  ts_rank(tsv, websearch_to_tsquery('english', $3)) * 8
                    + CASE WHEN $4 THEN similarity(title || ' ' || body, $3)
                           ELSE 0 END AS score
           FROM search_index
           WHERE org_id = $1
             AND entity_kind = ANY($2)
             AND (
               tsv @@ websearch_to_tsquery('english', $3)
               OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
             )
           ORDER BY score DESC, indexed_at DESC, entity_id
           LIMIT $5 OFFSET $6"#,
        org_id,
        kinds,
        q,
        trigram,
        limit,
        offset
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SearchHit {
            entity_kind: row.entity_kind,
            entity_id: row.entity_id,
            parent_id: row.parent_id,
            title: row.title,
            body_excerpt: row.body_excerpt.unwrap_or_default(),
            score: row.score.unwrap_or(0.0),
        })
        .collect())
}

/// Total matching rows for the same predicate — powers result counts and
/// pagination metadata without fetching the page.
pub async fn search_count<'e, E>(
    executor: E,
    org_id: Uuid,
    kinds: &[String],
    q: &str,
    trigram: bool,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS total
           FROM search_index
           WHERE org_id = $1
             AND entity_kind = ANY($2)
             AND (
               tsv @@ websearch_to_tsquery('english', $3)
               OR ($4 AND title || ' ' || body ILIKE '%' || $3 || '%')
             )"#,
        org_id,
        kinds,
        q,
        trigram
    )
    .fetch_one(executor)
    .await?;
    Ok(row.total.unwrap_or(0))
}
