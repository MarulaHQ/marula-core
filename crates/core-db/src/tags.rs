//! Tags (Wave 11 lane 9, migration 0056): org tag taxonomy + per-ticket
//! application + the ticket-list filter seam.
//!
//! Purpose: persistence for the tag construct — org-wide tag CRUD, the
//! `ticket_tags` apply/remove join, tag enrichment for ticket rows, and
//! the filter clause the ticket list query integrates.
//!
//! Non-purpose: no permission decisions (routes gate), no event emission
//! (routes own the outbox), no tag suggestions/autocomplete ranking.

use crate::{DbError, DbResult};
use core_domain::entities::Tag;
use sqlx::Postgres;
use uuid::Uuid;

const TAG_COLS: &str = "id, org_id, name, color, created_by, created_at, updated_at";

fn tag_from(row: sqlx::postgres::PgRow) -> DbResult<Tag> {
    use sqlx::Row;
    Ok(Tag {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        color: row.get("color"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// The org's full tag taxonomy, name-ordered.
pub async fn list_tags<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<Tag>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TAG_COLS} FROM tags WHERE org_id = $1 ORDER BY name");
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.into_iter().map(tag_from).collect()
}

/// How many tickets currently carry each tag (org-wide). Tags with zero
/// applications are omitted; the route left-joins this over the taxonomy.
pub async fn tag_application_counts<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<(Uuid, i64)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT tag_id, count(*)::bigint AS n
           FROM ticket_tags WHERE org_id = $1 GROUP BY tag_id"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.tag_id, r.n.unwrap_or(0)))
        .collect())
}

pub async fn get_tag<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Tag>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TAG_COLS} FROM tags WHERE org_id = $1 AND id = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .map(tag_from)
        .transpose()?
        .ok_or(DbError::NotFound)
}

/// The org's tag by name (apply path resolves user-facing names through
/// this; uniqueness is `UNIQUE (org_id, name)`).
pub async fn get_tag_by_name<'e, E>(executor: E, org_id: Uuid, name: &str) -> DbResult<Tag>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TAG_COLS} FROM tags WHERE org_id = $1 AND name = $2");
    sqlx::query(&sql)
        .bind(org_id)
        .bind(name)
        .fetch_optional(executor)
        .await?
        .map(tag_from)
        .transpose()?
        .ok_or(DbError::NotFound)
}

pub struct NewTag<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub color: Option<&'a str>,
    pub created_by: Option<Uuid>,
}

pub async fn insert_tag<'e, E>(executor: E, t: &NewTag<'_>) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO tags (id, org_id, name, color, created_by)
           VALUES ($1, $2, $3, $4, $5)"#,
        t.id,
        t.org_id,
        t.name,
        t.color,
        t.created_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Rename/recolor. Returns NotFound when the tag is not in scope.
pub async fn update_tag<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    color: Option<Option<&str>>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE tags SET
             name = COALESCE($3, name),
             color = CASE WHEN $4::boolean THEN $5 ELSE color END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        name,
        color.is_some(),
        color.flatten()
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Deletes the tag; `ticket_tags` rows cascade. Returns the application
/// count the deleted tag carried (the event payload cites it).
pub async fn delete_tag(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<i64> {
    let applied = sqlx::query!(
        r#"SELECT count(*)::bigint AS n FROM ticket_tags WHERE org_id = $1 AND tag_id = $2"#,
        org_id,
        id
    )
    .fetch_one(&mut *executor)
    .await?
    .n;
    let res = sqlx::query!("DELETE FROM tags WHERE org_id = $1 AND id = $2", org_id, id)
        .execute(&mut *executor)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(applied.unwrap_or(0))
}

/// Applies one tag to one ticket (idempotent via the join's PK).
pub async fn apply_tag<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    tag_id: Uuid,
    applied_by: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO ticket_tags (org_id, ticket_id, tag_id, applied_by)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (ticket_id, tag_id) DO NOTHING"#,
        org_id,
        ticket_id,
        tag_id,
        applied_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Removes one tag from one ticket. Returns whether a row was removed.
pub async fn remove_tag<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    tag_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"DELETE FROM ticket_tags WHERE org_id = $1 AND ticket_id = $2 AND tag_id = $3"#,
        org_id,
        ticket_id,
        tag_id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// A ticket's applied tags, applied-order stable.
pub async fn list_ticket_tags<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<Tag>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = "SELECT t.id, t.org_id, t.name, t.color, t.created_by, t.created_at, t.updated_at \
         FROM tags t \
         JOIN ticket_tags tt ON t.org_id = tt.org_id AND t.id = tt.tag_id \
         WHERE tt.org_id = $1 AND tt.ticket_id = $2 ORDER BY tt.applied_at"
        .to_string();
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(tag_from).collect()
}

/// Batch enrichment: tag ids per ticket id (list rendering). Absent
/// tickets simply carry empty vectors.
pub async fn tags_for_tickets(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket_ids: &[Uuid],
) -> DbResult<std::collections::HashMap<Uuid, Vec<Tag>>> {
    use sqlx::Row;
    let sql = format!(
        "SELECT tt.ticket_id AS ticket_id, t.{TAG_COLS} \
         FROM ticket_tags tt JOIN tags t ON t.org_id = tt.org_id AND t.id = tt.tag_id \
         WHERE tt.org_id = $1 AND tt.ticket_id = ANY($2) \
         ORDER BY tt.applied_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_ids)
        .fetch_all(executor)
        .await?;
    let mut out: std::collections::HashMap<Uuid, Vec<Tag>> = std::collections::HashMap::new();
    for row in rows {
        let tag = tag_from_offset(&row)?;
        let ticket_id: Uuid = row.get("ticket_id");
        out.entry(ticket_id).or_default().push(tag);
    }
    Ok(out)
}

/// Same mapper as [`tag_from`], offset by the leading ticket_id column.
fn tag_from_offset(row: &sqlx::postgres::PgRow) -> DbResult<Tag> {
    use sqlx::Row;
    Ok(Tag {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        color: row.get("color"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// The `EXISTS` fragment the ticket-list query reuses for tag filtering.
/// Match semantics: a ticket qualifies when it carries EVERY requested
/// tag (AND across ids) — callers pass the full requested set.
pub fn tag_filter_sql(tag_param: i32) -> String {
    format!(
        "AND (SELECT count(DISTINCT tt_filter.tag_id) FROM ticket_tags tt_filter \
         WHERE tt_filter.ticket_id = tickets.id \
           AND tt_filter.tag_id = ANY(${tag_param})) = cardinality(${tag_param})"
    )
}

/// Bulk apply from the merge fold: re-applies one source ticket's tags
/// onto the survivor (dedup-safe via the join's PK).
pub async fn fold_tags<'e, E>(
    executor: E,
    org_id: Uuid,
    source_ticket_id: Uuid,
    survivor_ticket_id: Uuid,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"INSERT INTO ticket_tags (org_id, ticket_id, tag_id, applied_by, applied_at)
           SELECT org_id, $3, tag_id, applied_by, applied_at
           FROM ticket_tags WHERE org_id = $1 AND ticket_id = $2
           ON CONFLICT (ticket_id, tag_id) DO NOTHING"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}
