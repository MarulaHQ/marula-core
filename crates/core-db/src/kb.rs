//! Knowledge-base queries (native KB v1, the private build tracker Phase 3): categories and
//! articles with client-facing flags.
//!
//! Purpose: typed CRUD + filtered lists over `kb_categories` /
//! `kb_articles` (0031). Every call is org-scoped and runs under the
//! caller's RLS transaction via the standard executor seam.
//!
//! Non-purpose: no status/visibility rules (core-kb owns those), no
//! search (the unified index in [`crate::search`] carries articles as
//! `kb_article` rows written by the indexer).

use crate::{DbError, DbResult};
use core_domain::entities::{KbArticle, KbArticleStatus, KbCategory};
use sqlx::Postgres;
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
fn row_category(
    id: Uuid,
    org_id: Uuid,
    name: String,
    slug: String,
    description: String,
    position: i32,
    client_visible: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> KbCategory {
    KbCategory {
        id,
        org_id,
        name,
        slug,
        description,
        position,
        client_visible,
        created_at,
        updated_at,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_category<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    slug: &str,
    description: &str,
    position: i32,
    client_visible: bool,
) -> DbResult<KbCategory>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO kb_categories (id, org_id, name, slug, description, position, client_visible)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, name, slug, description, position, client_visible, created_at, updated_at"#,
        id,
        org_id,
        name,
        slug,
        description,
        position,
        client_visible
    )
    .fetch_one(executor)
    .await?;
    Ok(row_category(
        row.id,
        row.org_id,
        row.name,
        row.slug,
        row.description,
        row.position,
        row.client_visible,
        row.created_at,
        row.updated_at,
    ))
}

pub async fn list_categories<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<KbCategory>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, slug, description, position, client_visible, created_at, updated_at
           FROM kb_categories WHERE org_id = $1
           ORDER BY position, name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            row_category(
                r.id,
                r.org_id,
                r.name,
                r.slug,
                r.description,
                r.position,
                r.client_visible,
                r.created_at,
                r.updated_at,
            )
        })
        .collect())
}

pub async fn get_category<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<KbCategory>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, slug, description, position, client_visible, created_at, updated_at
           FROM kb_categories WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_category(
        row.id,
        row.org_id,
        row.name,
        row.slug,
        row.description,
        row.position,
        row.client_visible,
        row.created_at,
        row.updated_at,
    ))
}

pub async fn update_category<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: &str,
    position: i32,
    client_visible: bool,
) -> DbResult<KbCategory>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE kb_categories
           SET name = $3, description = $4, position = $5, client_visible = $6, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, name, slug, description, position, client_visible, created_at, updated_at"#,
        org_id,
        id,
        name,
        description,
        position,
        client_visible
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_category(
        row.id,
        row.org_id,
        row.name,
        row.slug,
        row.description,
        row.position,
        row.client_visible,
        row.created_at,
        row.updated_at,
    ))
}

pub async fn delete_category<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM kb_categories WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Optional fields for an article update; `None` leaves the column as-is.
#[derive(Debug, Default, Clone)]
pub struct ArticlePatch {
    pub title: Option<String>,
    pub body: Option<String>,
    /// Canonical rich content (ADR 0007): the validated envelope; `None`
    /// leaves existing content untouched.
    pub content_jsonb: Option<serde_json::Value>,
    pub category_id: Option<Option<Uuid>>,
    pub status: Option<KbArticleStatus>,
    pub client_visible: Option<bool>,
    pub updated_by: Option<Uuid>,
}

#[allow(clippy::too_many_arguments)]
fn row_article(
    id: Uuid,
    org_id: Uuid,
    category_id: Option<Uuid>,
    title: String,
    slug: String,
    body: String,
    content_jsonb: Option<serde_json::Value>,
    status: String,
    client_visible: bool,
    created_by: Option<Uuid>,
    updated_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<KbArticle> {
    Ok(KbArticle {
        id,
        org_id,
        category_id,
        title,
        slug,
        body,
        content_jsonb,
        status: KbArticleStatus::parse(&status).ok_or_else(|| {
            DbError::Sql(sqlx::Error::ColumnDecode {
                index: "status".into(),
                source: format!("unknown kb article status '{status}'").into(),
            })
        })?,
        client_visible,
        created_by,
        updated_by,
        created_at,
        updated_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_article<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    category_id: Option<Uuid>,
    title: &str,
    slug: &str,
    body: &str,
    content_jsonb: Option<serde_json::Value>,
    status: KbArticleStatus,
    client_visible: bool,
    created_by: Uuid,
) -> DbResult<KbArticle>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO kb_articles
             (id, org_id, category_id, title, slug, body, content_jsonb, status,
              client_visible, created_by, updated_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
           RETURNING id, org_id, category_id, title, slug, body, content_jsonb, status,
                     client_visible, created_by, updated_by, created_at, updated_at"#,
        id,
        org_id,
        category_id,
        title,
        slug,
        body,
        content_jsonb,
        status.as_str(),
        client_visible,
        created_by
    )
    .fetch_one(executor)
    .await?;
    row_article(
        row.id,
        row.org_id,
        row.category_id,
        row.title,
        row.slug,
        row.body,
        row.content_jsonb,
        row.status,
        row.client_visible,
        row.created_by,
        row.updated_by,
        row.created_at,
        row.updated_at,
    )
}

/// Filters for [`list_articles`]; every field is an AND-ed constraint and
/// `None` means unconstrained.
#[derive(Debug, Default, Clone)]
pub struct ArticleFilter {
    pub category_id: Option<Uuid>,
    pub status: Option<KbArticleStatus>,
    /// Only articles passing the effective client-facing gate (published,
    /// article flag, AND their category's flag).
    pub client_visible: bool,
    /// Case-insensitive substring match over title (list narrowing; real
    /// text search rides the unified search index).
    pub q: Option<String>,
}

pub async fn list_articles<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: &ArticleFilter,
) -> DbResult<Vec<KbArticle>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // The client-visible gate needs the category row, so the query joins;
    // the remaining filters compose over the same shape.
    let rows = sqlx::query!(
        r#"SELECT a.id, a.org_id, a.category_id, a.title, a.slug, a.body, a.content_jsonb, a.status,
                  a.client_visible, a.created_by, a.updated_by, a.created_at, a.updated_at
           FROM kb_articles a
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE a.org_id = $1
             AND ($2::uuid IS NULL OR a.category_id = $2)
             AND ($3::text IS NULL OR a.status = $3)
             AND (NOT $4 OR (a.status = 'published' AND a.client_visible
                             AND COALESCE(c.client_visible, false)))
             AND ($5::text IS NULL OR a.title ILIKE '%' || $5 || '%')
           ORDER BY a.updated_at DESC, a.id"#,
        org_id,
        filter.category_id,
        filter.status.map(|s| s.as_str()),
        filter.client_visible,
        filter.q.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            row_article(
                r.id,
                r.org_id,
                r.category_id,
                r.title,
                r.slug,
                r.body,
                r.content_jsonb,
                r.status,
                r.client_visible,
                r.created_by,
                r.updated_by,
                r.created_at,
                r.updated_at,
            )
        })
        .collect()
}

pub async fn get_article<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<KbArticle>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, category_id, title, slug, body, content_jsonb, status, client_visible,
                  created_by, updated_by, created_at, updated_at
           FROM kb_articles WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    row_article(
        row.id,
        row.org_id,
        row.category_id,
        row.title,
        row.slug,
        row.body,
        row.content_jsonb,
        row.status,
        row.client_visible,
        row.created_by,
        row.updated_by,
        row.created_at,
        row.updated_at,
    )
}

// --- portal v2 client-facing reads --------------------------------------------
//
// The SAME client-facing gate the staff filter implements
// (`core-kb::client_viewable`: published AND article flag AND category
// flag — uncategorized articles are invisible to clients), enforced in
// the query so portal handlers cannot drift from it. Hidden rows resolve
// to NotFound / absent — no existence leak.

/// Categories the portal may show: `client_visible` only, tree order.
pub async fn list_client_categories<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<KbCategory>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, slug, description, position, client_visible, created_at, updated_at
           FROM kb_categories
           WHERE org_id = $1 AND client_visible
           ORDER BY position, name"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(row_category(
                r.id,
                r.org_id,
                r.name,
                r.slug,
                r.description,
                r.position,
                r.client_visible,
                r.created_at,
                r.updated_at,
            ))
        })
        .collect()
}

/// Client-gated article list (optionally narrowed to one category).
pub async fn list_client_articles<'e, E>(
    executor: E,
    org_id: Uuid,
    category_id: Option<Uuid>,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<KbArticle>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT a.id, a.org_id, a.category_id, a.title, a.slug, a.body, a.content_jsonb, a.status,
                  a.client_visible, a.created_by, a.updated_by, a.created_at, a.updated_at
           FROM kb_articles a
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE a.org_id = $1
             AND a.status = 'published' AND a.client_visible
             AND COALESCE(c.client_visible, false)
             AND ($2::uuid IS NULL OR a.category_id = $2)
           ORDER BY a.updated_at DESC, a.id
           LIMIT $3 OFFSET $4"#,
        org_id,
        category_id,
        limit,
        offset,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            row_article(
                r.id,
                r.org_id,
                r.category_id,
                r.title,
                r.slug,
                r.body,
                r.content_jsonb,
                r.status,
                r.client_visible,
                r.created_by,
                r.updated_by,
                r.created_at,
                r.updated_at,
            )
        })
        .collect()
}

/// One client-gated article; `DbError::NotFound` when the gate fails —
/// indistinguishable from missing.
pub async fn get_client_article<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<KbArticle>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT a.id, a.org_id, a.category_id, a.title, a.slug, a.body, a.content_jsonb, a.status,
                  a.client_visible, a.created_by, a.updated_by, a.created_at, a.updated_at
           FROM kb_articles a
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE a.org_id = $1 AND a.id = $2
             AND a.status = 'published' AND a.client_visible
             AND COALESCE(c.client_visible, false)"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    row_article(
        row.id,
        row.org_id,
        row.category_id,
        row.title,
        row.slug,
        row.body,
        row.content_jsonb,
        row.status,
        row.client_visible,
        row.created_by,
        row.updated_by,
        row.created_at,
        row.updated_at,
    )
}

/// One ranked client-gated FTS hit: websearch query over title + body
/// with a trigram/ILIKE fallback for partial tokens, ranked the same way
/// the unified search ranks (`ts_rank` weighted, trigram similarity
/// blended). The gate applies to every row before ranking.
pub struct ClientArticleHit {
    pub article: KbArticle,
    pub score: f64,
}

pub async fn search_client_articles<'e, E>(
    executor: E,
    org_id: Uuid,
    q: &str,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<ClientArticleHit>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT a.id, a.org_id, a.category_id, a.title, a.slug, a.body, a.content_jsonb, a.status,
                  a.client_visible, a.created_by, a.updated_by, a.created_at, a.updated_at,
                  ts_rank(to_tsvector('english', a.title || ' ' || a.body),
                          websearch_to_tsquery('english', $2)) * 8
                    + similarity(a.title || ' ' || a.body, $2) AS score
           FROM kb_articles a
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE a.org_id = $1
             AND a.status = 'published' AND a.client_visible
             AND COALESCE(c.client_visible, false)
             AND (to_tsvector('english', a.title || ' ' || a.body)
                    @@ websearch_to_tsquery('english', $2)
                  OR a.title || ' ' || a.body ILIKE '%' || $2 || '%')
           ORDER BY score DESC, a.updated_at DESC, a.id
           LIMIT $3 OFFSET $4"#,
        org_id,
        q,
        limit,
        offset,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(ClientArticleHit {
                article: row_article(
                    r.id,
                    r.org_id,
                    r.category_id,
                    r.title,
                    r.slug,
                    r.body,
                    r.content_jsonb,
                    r.status,
                    r.client_visible,
                    r.created_by,
                    r.updated_by,
                    r.created_at,
                    r.updated_at,
                )?,
                score: r.score.unwrap_or(0.0),
            })
        })
        .collect()
}

pub async fn update_article<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: &ArticlePatch,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Dynamic patch via QueryBuilder (same injection-safe shape as
    // tickets::update_ticket). `Some(None)` clears the nullable category;
    // `None` leaves the column untouched.
    let mut q = sqlx::QueryBuilder::new("UPDATE kb_articles SET updated_at = now()");
    if let Some(v) = &patch.title {
        q.push(", title = ").push_bind(v.clone());
    }
    if let Some(v) = &patch.content_jsonb {
        q.push(", content_jsonb = ")
            .push_bind(sqlx::types::Json(v.clone()));
    }
    if let Some(v) = &patch.body {
        q.push(", body = ").push_bind(v.clone());
    }
    if let Some(v) = patch.category_id {
        q.push(", category_id = ").push_bind(v);
    }
    if let Some(v) = patch.status {
        q.push(", status = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = patch.client_visible {
        q.push(", client_visible = ").push_bind(v);
    }
    if let Some(v) = patch.updated_by {
        q.push(", updated_by = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id);
    let updated = q.build().execute(executor).await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Removes an article; returns whether a row was actually deleted. The
/// unified-search entry is dropped by the indexer off the domain event —
/// never here (2.16 pattern: the index is indexer-owned).
pub async fn delete_article<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM kb_articles WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}
