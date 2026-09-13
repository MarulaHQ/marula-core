//! KB rich-content queries (KB1/KB2, migration 0069): the oEmbed
//! allowlist registry, per-article embed instances, and linked external
//! documents.
//!
//! Purpose: typed CRUD over `kb_embed_providers` / `kb_embeds` /
//! `kb_external_documents` under the caller's RLS transaction via the
//! standard executor seam. Provider reads are deliberately wide: system
//! rows (org_id IS NULL) are visible to every org; writes on system rows
//! are impossible because every mutation query filters `org_id = $1`.
//!
//! Non-purpose: no allowlist/regex rules (core-kb::rich owns those), no
//! HTTP fetching (core-api), no search (embeds and external documents
//! stay out of the unified index on purpose — they are pointers, not
//! searchable content).

use crate::{DbError, DbResult};
use core_domain::entities::{
    ExternalDocUrlKind, KbEmbed, KbEmbedProvider, KbExternalDocument, LinkCheckStatus,
};
use sqlx::Postgres;
use uuid::Uuid;

// --- embed providers -----------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn row_provider(
    id: Uuid,
    org_id: Option<Uuid>,
    name: String,
    slug: String,
    host_patterns: Vec<String>,
    url_regex: String,
    oembed_endpoint: String,
    enabled: bool,
    is_system: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> KbEmbedProvider {
    KbEmbedProvider {
        id,
        org_id,
        name,
        slug,
        host_patterns,
        url_regex,
        oembed_endpoint,
        enabled,
        is_system,
        created_at,
        updated_at,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_provider<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    slug: &str,
    host_patterns: &[String],
    url_regex: &str,
    oembed_endpoint: &str,
    enabled: bool,
) -> DbResult<KbEmbedProvider>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO kb_embed_providers
             (id, org_id, name, slug, host_patterns, url_regex, oembed_endpoint, enabled, is_system)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, false)
           RETURNING id, org_id, name, slug, host_patterns, url_regex, oembed_endpoint,
                     enabled, is_system, created_at, updated_at"#,
        id,
        org_id,
        name,
        slug,
        host_patterns,
        url_regex,
        oembed_endpoint,
        enabled,
    )
    .fetch_one(executor)
    .await?;
    Ok(row_provider(
        row.id,
        row.org_id,
        row.name,
        row.slug,
        row.host_patterns,
        row.url_regex,
        row.oembed_endpoint,
        row.enabled,
        row.is_system,
        row.created_at,
        row.updated_at,
    ))
}

/// The effective allowlist: system rows (org_id IS NULL) plus the org's
/// own rows, system first. Disabled rows are included — the API decides
/// what to do with them.
pub async fn list_providers<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<KbEmbedProvider>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, slug, host_patterns, url_regex, oembed_endpoint,
                  enabled, is_system, created_at, updated_at
           FROM kb_embed_providers
           WHERE org_id IS NULL OR org_id = $1
           ORDER BY is_system DESC, name, id"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            row_provider(
                r.id,
                r.org_id,
                r.name,
                r.slug,
                r.host_patterns,
                r.url_regex,
                r.oembed_endpoint,
                r.enabled,
                r.is_system,
                r.created_at,
                r.updated_at,
            )
        })
        .collect())
}

pub async fn get_provider<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<KbEmbedProvider>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, slug, host_patterns, url_regex, oembed_endpoint,
                  enabled, is_system, created_at, updated_at
           FROM kb_embed_providers WHERE (org_id = $1 OR org_id IS NULL) AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_provider(
        row.id,
        row.org_id,
        row.name,
        row.slug,
        row.host_patterns,
        row.url_regex,
        row.oembed_endpoint,
        row.enabled,
        row.is_system,
        row.created_at,
        row.updated_at,
    ))
}

/// Patchable provider fields; `None` leaves the column as-is. Only org
/// rows are reachable (queries filter org_id = $1, so system rows 404).
#[derive(Debug, Default, Clone)]
pub struct ProviderPatch {
    pub name: Option<String>,
    pub host_patterns: Option<Vec<String>>,
    pub url_regex: Option<String>,
    pub oembed_endpoint: Option<String>,
    pub enabled: Option<bool>,
}

pub async fn update_provider<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: &ProviderPatch,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Dynamic patch via QueryBuilder (same injection-safe shape as
    // kb::update_article). System rows are unreachable: the WHERE filters
    // on the caller's org, and system rows carry org_id IS NULL.
    let mut q = sqlx::QueryBuilder::new("UPDATE kb_embed_providers SET updated_at = now()");
    if let Some(v) = &patch.name {
        q.push(", name = ").push_bind(v.clone());
    }
    if let Some(v) = &patch.host_patterns {
        q.push(", host_patterns = ").push_bind(v.clone());
    }
    if let Some(v) = &patch.url_regex {
        q.push(", url_regex = ").push_bind(v.clone());
    }
    if let Some(v) = &patch.oembed_endpoint {
        q.push(", oembed_endpoint = ").push_bind(v.clone());
    }
    if let Some(v) = patch.enabled {
        q.push(", enabled = ").push_bind(v);
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

pub async fn delete_provider<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM kb_embed_providers WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}
// --- embeds --------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn row_embed(
    id: Uuid,
    org_id: Uuid,
    article_id: Uuid,
    url: String,
    provider_id: Option<Uuid>,
    provider_slug: Option<String>,
    title: String,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> KbEmbed {
    KbEmbed {
        id,
        org_id,
        article_id,
        url,
        provider_id,
        provider_slug,
        title,
        created_by,
        created_at,
        updated_at,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_embed<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    article_id: Uuid,
    url: &str,
    provider_id: Option<Uuid>,
    provider_slug: Option<&str>,
    title: &str,
    created_by: Uuid,
) -> DbResult<KbEmbed>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO kb_embeds
             (id, org_id, article_id, url, provider_id, provider_slug, title, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, org_id, article_id, url, provider_id, provider_slug, title,
                     created_by, created_at, updated_at"#,
        id,
        org_id,
        article_id,
        url,
        provider_id,
        provider_slug,
        title,
        created_by,
    )
    .fetch_one(executor)
    .await?;
    Ok(row_embed(
        row.id,
        row.org_id,
        row.article_id,
        row.url,
        row.provider_id,
        row.provider_slug,
        row.title,
        row.created_by,
        row.created_at,
        row.updated_at,
    ))
}

pub async fn get_embed<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<KbEmbed>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, article_id, url, provider_id, provider_slug, title,
                  created_by, created_at, updated_at
           FROM kb_embeds WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row_embed(
        row.id,
        row.org_id,
        row.article_id,
        row.url,
        row.provider_id,
        row.provider_slug,
        row.title,
        row.created_by,
        row.created_at,
        row.updated_at,
    ))
}

/// Embeds attached to one article, newest first.
pub async fn list_embeds<'e, E>(
    executor: E,
    org_id: Uuid,
    article_id: Uuid,
) -> DbResult<Vec<KbEmbed>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, article_id, url, provider_id, provider_slug, title,
                  created_by, created_at, updated_at
           FROM kb_embeds WHERE org_id = $1 AND article_id = $2
           ORDER BY created_at, id"#,
        org_id,
        article_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            row_embed(
                r.id,
                r.org_id,
                r.article_id,
                r.url,
                r.provider_id,
                r.provider_slug,
                r.title,
                r.created_by,
                r.created_at,
                r.updated_at,
            )
        })
        .collect())
}

pub async fn delete_embed<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM kb_embeds WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

// --- external documents --------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn row_external_document(
    id: Uuid,
    org_id: Uuid,
    article_id: Option<Uuid>,
    title: String,
    url: String,
    url_kind: String,
    permission_note: String,
    last_verified_at: Option<chrono::DateTime<chrono::Utc>>,
    last_verified_status: Option<String>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<KbExternalDocument> {
    Ok(KbExternalDocument {
        id,
        org_id,
        article_id,
        title,
        url,
        url_kind: ExternalDocUrlKind::parse(&url_kind).ok_or_else(|| {
            DbError::Sql(sqlx::Error::ColumnDecode {
                index: "url_kind".into(),
                source: format!("unknown external document url kind '{url_kind}'").into(),
            })
        })?,
        permission_note,
        last_verified_at,
        last_verified_status: last_verified_status.and_then(|s| LinkCheckStatus::parse(&s)),
        created_by,
        created_at,
        updated_at,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn create_external_document<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    article_id: Option<Uuid>,
    title: &str,
    url: &str,
    url_kind: ExternalDocUrlKind,
    permission_note: &str,
    created_by: Uuid,
) -> DbResult<KbExternalDocument>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO kb_external_documents
             (id, org_id, article_id, title, url, url_kind, permission_note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, org_id, article_id, title, url, url_kind, permission_note,
                     last_verified_at, last_verified_status, created_by, created_at, updated_at"#,
        id,
        org_id,
        article_id,
        title,
        url,
        url_kind.as_str(),
        permission_note,
        created_by,
    )
    .fetch_one(executor)
    .await?;
    row_external_document(
        row.id,
        row.org_id,
        row.article_id,
        row.title,
        row.url,
        row.url_kind,
        row.permission_note,
        row.last_verified_at,
        row.last_verified_status,
        row.created_by,
        row.created_at,
        row.updated_at,
    )
}

/// Optional fields for an external-document update; `None` leaves the
/// column as-is.
#[derive(Debug, Default, Clone)]
pub struct ExternalDocumentPatch {
    pub title: Option<String>,
    pub url: Option<String>,
    pub url_kind: Option<ExternalDocUrlKind>,
    pub permission_note: Option<String>,
    /// `Some(None)` detaches the article; `None` leaves it untouched.
    pub article_id: Option<Option<Uuid>>,
}

pub async fn update_external_document<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    patch: &ExternalDocumentPatch,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Changing the URL invalidates the last link-check: both reset here.
    let mut q = sqlx::QueryBuilder::new("UPDATE kb_external_documents SET updated_at = now()");
    if let Some(v) = &patch.title {
        q.push(", title = ").push_bind(v.clone());
    }
    if let Some(v) = &patch.url {
        q.push(", url = ").push_bind(v.clone());
        q.push(", last_verified_at = ")
            .push_bind(Option::<chrono::DateTime<chrono::Utc>>::None);
        q.push(", last_verified_status = ")
            .push_bind(Option::<String>::None);
    }
    if let Some(v) = patch.url_kind {
        q.push(", url_kind = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = &patch.permission_note {
        q.push(", permission_note = ").push_bind(v.clone());
    }
    if let Some(v) = patch.article_id {
        q.push(", article_id = ").push_bind(v);
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

pub async fn get_external_document<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<KbExternalDocument>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, article_id, title, url, url_kind, permission_note,
                  last_verified_at, last_verified_status, created_by, created_at, updated_at
           FROM kb_external_documents WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    row_external_document(
        row.id,
        row.org_id,
        row.article_id,
        row.title,
        row.url,
        row.url_kind,
        row.permission_note,
        row.last_verified_at,
        row.last_verified_status,
        row.created_by,
        row.created_at,
        row.updated_at,
    )
}

/// Staff listing (optionally narrowed to one article). Portal reads go
/// through the client-gated query below, never this one.
pub async fn list_external_documents<'e, E>(
    executor: E,
    org_id: Uuid,
    article_id: Option<Uuid>,
) -> DbResult<Vec<KbExternalDocument>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, article_id, title, url, url_kind, permission_note,
                  last_verified_at, last_verified_status, created_by, created_at, updated_at
           FROM kb_external_documents
           WHERE org_id = $1 AND ($2::uuid IS NULL OR article_id = $2)
           ORDER BY updated_at DESC, id"#,
        org_id,
        article_id,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            row_external_document(
                row.id,
                row.org_id,
                row.article_id,
                row.title,
                row.url,
                row.url_kind,
                row.permission_note,
                row.last_verified_at,
                row.last_verified_status,
                row.created_by,
                row.created_at,
                row.updated_at,
            )
        })
        .collect()
}

/// Client-gated external documents for the portal: attached rows only,
/// gated by the SAME effective rule as the article itself (published +
/// article flag + category flag). Standalone rows never appear. Hidden
/// articles surface as empty — no existence leak.
pub async fn list_client_external_documents<'e, E>(
    executor: E,
    org_id: Uuid,
    article_id: Uuid,
) -> DbResult<Vec<KbExternalDocument>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT e.id, e.org_id, e.article_id, e.title, e.url, e.url_kind, e.permission_note,
                  e.last_verified_at, e.last_verified_status, e.created_by, e.created_at, e.updated_at
           FROM kb_external_documents e
           JOIN kb_articles a ON a.id = e.article_id AND a.org_id = e.org_id
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE e.org_id = $1 AND e.article_id = $2
             AND a.status = 'published' AND a.client_visible
             AND COALESCE(c.client_visible, false)
           ORDER BY e.created_at, e.id"#,
        org_id,
        article_id,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            row_external_document(
                r.id,
                r.org_id,
                r.article_id,
                r.title,
                r.url,
                r.url_kind,
                r.permission_note,
                r.last_verified_at,
                r.last_verified_status,
                r.created_by,
                r.created_at,
                r.updated_at,
            )
        })
        .collect()
}

/// Client-gated embeds for one article — the same effective gate as the
/// article. A system-provider embed keeps its sanction; a NULL provider
/// (row deleted after the fact) is filtered out here: it renders nothing.
pub async fn list_client_embeds<'e, E>(
    executor: E,
    org_id: Uuid,
    article_id: Uuid,
) -> DbResult<Vec<KbEmbed>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT b.id, b.org_id, b.article_id, b.url, b.provider_id, b.provider_slug, b.title,
                  b.created_by, b.created_at, b.updated_at
           FROM kb_embeds b
           JOIN kb_articles a ON a.id = b.article_id AND a.org_id = b.org_id
           LEFT JOIN kb_categories c ON c.id = a.category_id AND c.org_id = a.org_id
           WHERE b.org_id = $1 AND b.article_id = $2
             AND a.status = 'published' AND a.client_visible
             AND COALESCE(c.client_visible, false)
             AND b.provider_id IS NOT NULL
           ORDER BY b.created_at, b.id"#,
        org_id,
        article_id,
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(row_embed(
                r.id,
                r.org_id,
                r.article_id,
                r.url,
                r.provider_id,
                r.provider_slug,
                r.title,
                r.created_by,
                r.created_at,
                r.updated_at,
            ))
        })
        .collect()
}

pub async fn delete_external_document<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query!(
        "DELETE FROM kb_external_documents WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Records a link-check result: the verified-at stamp is the caller's
/// `now()` anchor so tests stay deterministic. Returns NotFound when the
/// row is gone (checked after deletion).
pub async fn record_link_check<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    verified_at: chrono::DateTime<chrono::Utc>,
    status: LinkCheckStatus,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let updated = sqlx::query!(
        r#"UPDATE kb_external_documents
           SET last_verified_at = $3, last_verified_status = $4, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        verified_at,
        status.as_str(),
    )
    .execute(executor)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
