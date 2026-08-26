//! Ticket queries: creation with number allocation, filtered lists, status
//! transitions, structured comments, and pre-close checklists.

use crate::{DbError, DbResult};
use core_domain::entities::{ChecklistItem, Ticket, TicketComment};
use core_domain::ids::TicketNumber;
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

fn parse_priority(s: &str) -> DbResult<Priority> {
    s.parse().map_err(|e: core_domain::DomainError| {
        DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
    })
}

fn row_to_ticket(row: sqlx::postgres::PgRow) -> Result<Ticket, sqlx::Error> {
    use sqlx::Row;
    Ok(Ticket {
        id: row.get("id"),
        org_id: row.get("org_id"),
        number: TicketNumber(row.get("number")),
        board_id: row.get("board_id"),
        status_id: row.get("status_id"),
        title: row.get("title"),
        body: row.get("body"),
        priority: row.get::<String, _>("priority").parse().map_err(
            |e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "priority".into(),
                source: Box::new(e),
            },
        )?,
        client_id: row.get("client_id"),
        site_id: row.get("site_id"),
        contact_id: row.get("contact_id"),
        assignee_id: row.get("assignee_id"),
        requester_id: row.get("requester_id"),
        category: row.get::<String, _>("category").parse().map_err(
            |e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "category".into(),
                source: Box::new(e),
            },
        )?,
        due_at: row.get("due_at"),
        resolved_at: row.get("resolved_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const TICKET_COLS: &str = "id, org_id, number, board_id, status_id, title, body, priority, \
     client_id, site_id, contact_id, assignee_id, requester_id, category, due_at, \
     resolved_at, created_at, updated_at";

pub struct NewTicket<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub number: &'a str,
    pub board_id: Uuid,
    pub status_id: Uuid,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub priority: Priority,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub requester_id: Option<Uuid>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Insert + re-fetch (category comes from the board status row): takes a
/// connection so it can run inside the caller's transaction.
pub async fn insert_ticket(conn: &mut PgConnection, t: NewTicket<'_>) -> DbResult<Ticket> {
    let row = sqlx::query!(
        r#"INSERT INTO tickets (id, org_id, number, board_id, status_id, title, body,
                                priority, client_id, site_id, contact_id, assignee_id,
                                requester_id, category, due_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                   (SELECT category FROM board_statuses WHERE id = $5), $14)
           RETURNING id"#,
        t.id,
        t.org_id,
        t.number,
        t.board_id,
        t.status_id,
        t.title,
        t.body,
        t.priority.as_str(),
        t.client_id,
        t.site_id,
        t.contact_id,
        t.assignee_id,
        t.requester_id,
        t.due_at,
    )
    .fetch_one(&mut *conn)
    .await?;
    get_ticket(&mut *conn, t.org_id, row.id).await
}

pub async fn get_ticket<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Ticket>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TICKET_COLS} FROM tickets WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_ticket(row)?)
}

pub async fn get_ticket_by_number<'e, E>(
    executor: E,
    org_id: Uuid,
    number: &str,
) -> DbResult<Ticket>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {TICKET_COLS} FROM tickets WHERE org_id = $1 AND number = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(number)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    Ok(row_to_ticket(row)?)
}

pub struct TicketFilter {
    pub board_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub category: Option<TicketStatusCategory>,
    pub priority: Option<Priority>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn list_tickets<'e, E>(
    executor: E,
    org_id: Uuid,
    f: TicketFilter,
) -> DbResult<Vec<Ticket>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {TICKET_COLS} FROM tickets
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR board_id = $2)
             AND ($3::uuid IS NULL OR assignee_id = $3)
             AND ($4::uuid IS NULL OR client_id = $4)
             AND ($5::text IS NULL OR category = $5)
             AND ($6::text IS NULL OR priority = $6)
           ORDER BY
             CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END,
             due_at NULLS LAST, created_at
           LIMIT $7 OFFSET $8"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(f.board_id)
        .bind(f.assignee_id)
        .bind(f.client_id)
        .bind(f.category.map(|c| c.as_str().to_string()))
        .bind(f.priority.map(|p| p.as_str().to_string()))
        .bind(f.limit)
        .bind(f.offset)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_ticket)
        .collect::<Result<_, _>>()?)
}

/// Tickets for a user's my-work view: assigned to them, not closed, across
/// all boards, priority-ordered.
pub async fn my_work<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    limit: i64,
) -> DbResult<Vec<Ticket>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {TICKET_COLS} FROM tickets
           WHERE org_id = $1 AND assignee_id = $2 AND category <> 'closed'
           ORDER BY
             CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END,
              due_at NULLS LAST, created_at
            LIMIT $3"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    Ok(rows
        .into_iter()
        .map(row_to_ticket)
        .collect::<Result<_, _>>()?)
}

pub async fn update_status<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    status_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE tickets SET
             status_id = $3,
             category = (SELECT category FROM board_statuses WHERE id = $3),
             resolved_at = CASE
               WHEN (SELECT category FROM board_statuses WHERE id = $3) = 'closed'
                 AND resolved_at IS NULL THEN now()
               ELSE resolved_at END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        ticket_id,
        status_id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub struct TicketPatch<'a> {
    pub title: Option<&'a str>,
    pub body: Option<&'a str>,
    pub priority: Option<Priority>,
    pub assignee_id: Option<Option<Uuid>>,
    pub client_id: Option<Option<Uuid>>,
    pub site_id: Option<Option<Uuid>>,
    pub contact_id: Option<Option<Uuid>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

/// Dynamic patch via QueryBuilder (injection-safe: every value is push_bind).
/// `Some(None)` clears a nullable column; `None` leaves it untouched.
pub async fn update_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    p: TicketPatch<'_>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let mut q = sqlx::QueryBuilder::new("UPDATE tickets SET updated_at = now()");
    if let Some(v) = p.title {
        q.push(", title = ").push_bind(v.to_string());
    }
    if let Some(v) = p.body {
        q.push(", body = ").push_bind(v.to_string());
    }
    if let Some(v) = p.priority {
        q.push(", priority = ").push_bind(v.as_str().to_string());
    }
    if let Some(v) = p.assignee_id {
        q.push(", assignee_id = ").push_bind(v);
    }
    if let Some(v) = p.client_id {
        q.push(", client_id = ").push_bind(v);
    }
    if let Some(v) = p.site_id {
        q.push(", site_id = ").push_bind(v);
    }
    if let Some(v) = p.contact_id {
        q.push(", contact_id = ").push_bind(v);
    }
    if let Some(v) = p.due_at {
        q.push(", due_at = ").push_bind(v);
    }
    q.push(" WHERE org_id = ")
        .push_bind(org_id)
        .push(" AND id = ")
        .push_bind(id);
    let res = q.build().execute(executor).await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn insert_comment<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    author_id: Option<Uuid>,
    kind: NoteKind,
    body: &str,
) -> DbResult<TicketComment>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, ticket_id, author_id, kind, body, created_at, updated_at"#,
        id,
        org_id,
        ticket_id,
        author_id,
        kind.as_str(),
        body
    )
    .fetch_one(executor)
    .await?;
    Ok(TicketComment {
        id: row.id,
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        author_id: row.author_id,
        kind: row.kind.parse().map_err(|e: core_domain::DomainError| {
            DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
        })?,
        body: row.body,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Fetches one comment by id — the search indexer's refetch path.
pub async fn get_comment<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<TicketComment>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, ticket_id, author_id, kind, body, created_at, updated_at
           FROM ticket_comments WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(TicketComment {
        id: row.id,
        org_id: row.org_id,
        ticket_id: row.ticket_id,
        author_id: row.author_id,
        kind: row.kind.parse().map_err(|e: core_domain::DomainError| {
            DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
        })?,
        body: row.body,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_comments<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    kinds: Option<&[NoteKind]>,
) -> DbResult<Vec<TicketComment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let kind_strs: Option<Vec<String>> =
        kinds.map(|ks| ks.iter().map(|k| k.as_str().to_string()).collect());
    let rows = sqlx::query!(
        r#"SELECT id, org_id, ticket_id, author_id, kind, body, created_at, updated_at
           FROM ticket_comments
           WHERE org_id = $1 AND ticket_id = $2
             AND ($3::text[] IS NULL OR kind = ANY($3))
           ORDER BY created_at"#,
        org_id,
        ticket_id,
        kind_strs.as_deref()
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(TicketComment {
                id: row.id,
                org_id: row.org_id,
                ticket_id: row.ticket_id,
                author_id: row.author_id,
                kind: row.kind.parse().map_err(|e: core_domain::DomainError| {
                    DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
                })?,
                body: row.body,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
        })
        .collect()
}

// --- checklist -------------------------------------------------------------

pub async fn add_checklist_item<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    text: &str,
    sort_order: i32,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO checklist_items (id, org_id, ticket_id, text, sort_order)
           VALUES ($1, $2, $3, $4, $5)"#,
        id,
        org_id,
        ticket_id,
        text,
        sort_order
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn set_checklist_item<'e, E>(
    executor: E,
    org_id: Uuid,
    item_id: Uuid,
    checked: bool,
    checked_by: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE checklist_items SET checked = $3, checked_by = $4,
             checked_at = CASE WHEN $3 THEN now() ELSE NULL END
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        item_id,
        checked,
        checked_by
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn list_checklist<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<ChecklistItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, ticket_id, text, checked, sort_order, checked_by, checked_at
           FROM checklist_items WHERE org_id = $1 AND ticket_id = $2 ORDER BY sort_order"#,
        org_id,
        ticket_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ChecklistItem {
            id: row.id,
            org_id: row.org_id,
            ticket_id: row.ticket_id,
            text: row.text,
            checked: row.checked,
            sort_order: row.sort_order,
            checked_by: row.checked_by,
            checked_at: row.checked_at,
        })
        .collect())
}

pub async fn parse_priority_or_err(s: &str) -> DbResult<Priority> {
    parse_priority(s)
}
