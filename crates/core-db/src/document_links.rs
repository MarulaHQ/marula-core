//! Cross-entity relationship links (docs-platform v1, migration 0053):
//! any document or password can link to a document, password, client,
//! ticket, or KB article. Links to vanished targets are surfaced as
//! broken (typed read model) rather than silently dropped.

use crate::{DbError, DbResult};
use core_domain::entities::{DocumentLink, LinkKind, ResolvedLink};
use sqlx::{Postgres, Row};
use uuid::Uuid;

pub async fn list_links<'e, E>(
    executor: E,
    org_id: Uuid,
    from_kind: LinkKind,
    from_id: Uuid,
) -> DbResult<Vec<ResolvedLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT l.id, l.org_id, l.from_kind, l.from_id, l.to_kind, l.to_id,
                  l.created_by, l.created_at,
                  CASE l.to_kind
                    WHEN 'document' THEN (SELECT title FROM documents d WHERE d.org_id = l.org_id AND d.id = l.to_id)
                    WHEN 'password' THEN (SELECT title FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.to_id)
                    WHEN 'client'   THEN (SELECT name FROM clients c WHERE c.org_id = l.org_id AND c.id = l.to_id)
                    WHEN 'ticket'   THEN (SELECT t.number || ' ' || t.title FROM tickets t WHERE t.org_id = l.org_id AND t.id = l.to_id)
                    WHEN 'kb_article' THEN (SELECT title FROM kb_articles k WHERE k.org_id = l.org_id AND k.id = l.to_id)
                    WHEN 'asset' THEN (SELECT a.tag || ' ' || a.name FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.to_id)
                    WHEN 'secret' THEN (SELECT sr.title FROM secret_records sr
                        WHERE sr.id = l.to_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_label,
                  CASE l.to_kind
                    WHEN 'document' THEN (SELECT client_id FROM documents d WHERE d.org_id = l.org_id AND d.id = l.to_id)
                    WHEN 'password' THEN (SELECT client_id FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.to_id)
                    WHEN 'ticket'   THEN (SELECT client_id FROM tickets t WHERE t.org_id = l.org_id AND t.id = l.to_id)
                    WHEN 'client'   THEN l.to_id
                    WHEN 'asset'    THEN (SELECT a.client_id FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.to_id)
                    WHEN 'secret'   THEN (SELECT sr.client_id FROM secret_records sr
                        WHERE sr.id = l.to_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                    ELSE NULL
                  END AS target_client_id
           FROM document_links l
           WHERE l.org_id = $1 AND l.from_kind = $2 AND l.from_id = $3
           ORDER BY l.created_at DESC"#,
    )
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let to_kind = LinkKind::parse(r.get::<String, _>("to_kind").as_str());
            ResolvedLink {
                link: DocumentLink {
                    id: r.get("id"),
                    org_id: r.get("org_id"),
                    from_kind,
                    from_id: r.get("from_id"),
                    to_kind: to_kind.unwrap_or(LinkKind::Document),
                    to_id: r.get("to_id"),
                    created_by: r.get("created_by"),
                    created_at: r.get("created_at"),
                },
                target_label: r.get("target_label"),
                target_client_id: r.get("target_client_id"),
                target_deleted: r.get::<Option<String>, _>("target_label").is_none(),
            }
        })
        .collect())
}

/// Reverse-direction link list: links that POINT AT the given target
/// (e.g. the asset detail page's "vault entries" panel reads
/// password→asset links here). Same resolved-label read model.
pub async fn list_links_to<'e, E>(
    executor: E,
    org_id: Uuid,
    to_kind: LinkKind,
    to_id: Uuid,
) -> DbResult<Vec<ResolvedLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT l.id, l.org_id, l.from_kind, l.from_id, l.to_kind, l.to_id,
                  l.created_by, l.created_at,
                  CASE l.from_kind
                    WHEN 'document' THEN (SELECT title FROM documents d WHERE d.org_id = l.org_id AND d.id = l.from_id)
                    WHEN 'password' THEN (SELECT title FROM passwords p WHERE p.org_id = l.org_id AND p.id = l.from_id)
                    WHEN 'asset'    THEN (SELECT a.tag || ' ' || a.name FROM device_assets a WHERE a.org_id = l.org_id AND a.id = l.from_id)
                    WHEN 'secret'   THEN (SELECT sr.title FROM secret_records sr
                        WHERE sr.id = l.from_id
                          AND (sr.org_id = l.org_id
                               OR (sr.org_id IS NULL AND sr.client_id IN (
                                   SELECT c.id FROM clients c
                                   WHERE c.org_id = l.org_id))))
                  END AS target_label,
                  NULL::uuid AS target_client_id
           FROM document_links l
           WHERE l.org_id = $1 AND l.to_kind = $2 AND l.to_id = $3
           ORDER BY l.created_at DESC"#,
    )
    .bind(org_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let from_kind = LinkKind::parse(r.get::<String, _>("from_kind").as_str());
            ResolvedLink {
                link: DocumentLink {
                    id: r.get("id"),
                    org_id: r.get("org_id"),
                    from_kind: from_kind.unwrap_or(LinkKind::Document),
                    from_id: r.get("from_id"),
                    to_kind,
                    to_id: r.get("to_id"),
                    created_by: r.get("created_by"),
                    created_at: r.get("created_at"),
                },
                target_label: r.get("target_label"),
                target_client_id: r.get("target_client_id"),
                target_deleted: r.get::<Option<String>, _>("target_label").is_none(),
            }
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_link<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    from_kind: LinkKind,
    from_id: Uuid,
    to_kind: LinkKind,
    to_id: Uuid,
    created_by: Uuid,
) -> DbResult<DocumentLink>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query(
        r#"INSERT INTO document_links (id, org_id, from_kind, from_id, to_kind, to_id, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, from_kind, from_id, to_kind, to_id, created_by, created_at"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(from_kind.as_str())
    .bind(from_id)
    .bind(to_kind.as_str())
    .bind(to_id)
    .bind(created_by)
    .fetch_one(executor)
    .await?;
    Ok(DocumentLink {
        id: r.get("id"),
        org_id: r.get("org_id"),
        from_kind,
        from_id: r.get("from_id"),
        to_kind,
        to_id: r.get("to_id"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
    })
}

/// Removes one link, or both directions when `bidirectional` (the UI's
/// unlink acts on the link row it sees; the API also accepts the reverse
/// direction to make symmetric unlinking a single call).
pub async fn delete_link<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let r = sqlx::query("DELETE FROM document_links WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(executor)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
