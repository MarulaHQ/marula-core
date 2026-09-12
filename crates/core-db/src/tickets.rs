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

/// Shared with the portal read path (`crate::portal`) so both surfaces
/// project identical Ticket shapes from raw rows.
pub(crate) fn row_to_ticket(row: sqlx::postgres::PgRow) -> Result<Ticket, sqlx::Error> {
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
        merged_into_ticket_id: row.try_get("merged_into_ticket_id").unwrap_or(None),
        // Classification read-view (Wave 12 lane 6): try_get keeps callers
        // that project a narrower column list (portal surfaces) working —
        // they simply report unclassified.
        type_id: row.try_get("type_id").unwrap_or(None),
        category_id: row.try_get("category_id").unwrap_or(None),
    })
}

const TICKET_COLS: &str = "id, org_id, number, board_id, status_id, title, body, priority, \
     client_id, site_id, contact_id, assignee_id, requester_id, category, due_at, \
     resolved_at, created_at, updated_at, merged_into_ticket_id, type_id, category_id";

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
    /// Tag filter (Wave 11 lane 9): tickets carrying EVERY tag in the
    /// set (AND semantics across ids); None/empty disables the clause.
    pub tag_ids: Option<Vec<Uuid>>,
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
    // Tag clause (lane 9): semi-join against ticket_tags, $9. Built as a
    // literal fragment + bound array — no value interpolation, so the
    // injection surface stays exactly the bind parameter.
    let tag_clause = if f.tag_ids.as_ref().is_some_and(|v| !v.is_empty()) {
        crate::tags::tag_filter_sql(9)
    } else {
        String::new()
    };
    let sql = format!(
        r#"SELECT {TICKET_COLS} FROM tickets
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR board_id = $2)
             AND ($3::uuid IS NULL OR assignee_id = $3)
             AND ($4::uuid IS NULL OR client_id = $4)
             AND ($5::text IS NULL OR category = $5)
             AND ($6::text IS NULL OR priority = $6)
             {tag_clause}
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
        .bind(f.tag_ids.clone().unwrap_or_default())
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

/// One cross-board my-work row (TASKLIST 2.11): the ticket plus the
/// annotations the Today queue renders — client name (squared tag), source
/// board/status labels, and the effective due instant.
#[derive(Debug, Clone)]
pub struct MyWorkRow {
    pub ticket: Ticket,
    pub client_name: Option<String>,
    pub board_name: String,
    pub status_name: String,
    /// SLA resolution deadline when a state row exists; `None` means the
    /// seam is open for that ticket (no policy/state yet) and only
    /// `ticket.due_at` carries a deadline.
    pub sla_resolution_due: Option<chrono::DateTime<chrono::Utc>>,
}

fn my_work_row_from(row: sqlx::postgres::PgRow) -> DbResult<MyWorkRow> {
    use sqlx::Row;
    let client_name: Option<String> = row.try_get("client_name").ok();
    let board_name: String = row.try_get("board_name").map_err(|_| {
        DbError::Sql(sqlx::Error::ColumnDecode {
            index: "board_name".into(),
            source: "missing board name join".into(),
        })
    })?;
    let status_name: String = row.try_get("status_name").map_err(|_| {
        DbError::Sql(sqlx::Error::ColumnDecode {
            index: "status_name".into(),
            source: "missing status name join".into(),
        })
    })?;
    let sla_resolution_due: Option<chrono::DateTime<chrono::Utc>> =
        row.try_get("sla_resolution_due").ok();
    Ok(MyWorkRow {
        ticket: row_to_ticket(row)?,
        client_name,
        board_name,
        status_name,
        sla_resolution_due,
    })
}

/// Cross-board my-work queue: assigned to me, not closed, across ALL org
/// boards. Ordering contract: priority rank first, then the earliest of
/// SLA `resolution_due` / manual `due_at` (NULLS LAST), then creation.
/// The SLA join is a LEFT JOIN — tickets without state rows still appear,
/// ordered by priority + due_at alone (the seam stays honest if the
/// sla-engine lane reshapes `ticket_sla_state`).
pub async fn my_work_annotated<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    limit: i64,
) -> DbResult<Vec<MyWorkRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let cols = TICKET_COLS.replace(", ", ", t.");
    let sql = format!(
        r#"SELECT t.{cols},
                  c.name  AS client_name,
                  b.name  AS board_name,
                  st.name AS status_name,
                  s.resolution_due AS sla_resolution_due
           FROM tickets t
           LEFT JOIN clients c ON c.id = t.client_id
           LEFT JOIN boards b ON b.id = t.board_id
           LEFT JOIN board_statuses st ON st.id = t.status_id
           LEFT JOIN ticket_sla_state s ON s.ticket_id = t.id
           WHERE t.org_id = $1 AND t.assignee_id = $2 AND t.category <> 'closed'
           ORDER BY
             CASE t.priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END,
             COALESCE(s.resolution_due, t.due_at) NULLS LAST,
             t.created_at
           LIMIT $3"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(my_work_row_from).collect()
}

/// CRUD unassigned tickets for the dispatch queue (TASKLIST 2.19): same
/// annotated shape and ordering as [`my_work_annotated`] — priority rank
/// first, then the earliest effective deadline — but the unassigned
/// filter replaces the per-user one, so a dispatcher sees exactly what
/// needs placing.
pub async fn unassigned_queue_annotated<'e, E>(
    executor: E,
    org_id: Uuid,
    limit: i64,
) -> DbResult<Vec<MyWorkRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let cols = TICKET_COLS.replace(", ", ", t.");
    let sql = format!(
        r#"SELECT t.{cols},
                  c.name  AS client_name,
                  b.name  AS board_name,
                  st.name AS status_name,
                  s.resolution_due AS sla_resolution_due
           FROM tickets t
           LEFT JOIN clients c ON c.id = t.client_id
           LEFT JOIN boards b ON b.id = t.board_id
           LEFT JOIN board_statuses st ON st.id = t.status_id
           LEFT JOIN ticket_sla_state s ON s.ticket_id = t.id
           WHERE t.org_id = $1 AND t.assignee_id IS NULL AND t.category <> 'closed'
           ORDER BY
             CASE t.priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END,
             COALESCE(s.resolution_due, t.due_at) NULLS LAST,
             t.created_at
           LIMIT $2"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(my_work_row_from).collect()
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

/// Move a ticket to a different board with an explicit target status
/// (Wave 11 lane 3). The category re-derives from the TARGET board's
/// status row — the semantic-category binding lives on board_statuses,
/// never on the taxonomy trees (classification-only doctrine, §5).
/// `resolved_at` mirrors `update_status`: crossing into a closed-category
/// status stamps it once; leaving one keeps the original stamp.
pub async fn move_to_board<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    new_board_id: Uuid,
    new_status_id: Uuid,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE tickets SET
             board_id = $3,
             status_id = $4,
             category = (SELECT category FROM board_statuses WHERE id = $4),
             resolved_at = CASE
               WHEN (SELECT category FROM board_statuses WHERE id = $4) = 'closed'
                 AND resolved_at IS NULL THEN now()
               ELSE resolved_at END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        ticket_id,
        new_board_id,
        new_status_id
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
    /// `Some(Some(text))` sets; `Some(None)` clears; `None` = no change.
    /// (IF7 reverts restore an absent body; JSON patches keep the legacy
    /// null-means-unchanged surface via `.map(Some)`.)
    pub body: Option<Option<&'a str>>,
    pub priority: Option<Priority>,
    pub assignee_id: Option<Option<Uuid>>,
    pub client_id: Option<Option<Uuid>>,
    pub site_id: Option<Option<Uuid>>,
    pub contact_id: Option<Option<Uuid>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Classification (Wave 11 lane 3): `Some(Some(id))` sets; `Some(None)`
    /// clears; `None` = no change. The caller validates org membership and
    /// retired-ness (classification-only doctrine: behavior-neutral).
    pub type_id: Option<Option<Uuid>>,
    pub category_id: Option<Option<Uuid>>,
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
        // Option<&str> encodes SQL NULL for None (clear), the text
        // otherwise.
        q.push(", body = ").push_bind(v);
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
    if let Some(v) = p.type_id {
        q.push(", type_id = ").push_bind(v);
    }
    if let Some(v) = p.category_id {
        q.push(", category_id = ").push_bind(v);
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

/// Client-facing comment read (portal v1 will consume this): the kind
/// whitelist comes from the pure domain policy (`notes::visible_kinds`),
/// so the query layer and the policy layer cannot drift apart. There is no
/// parameter that can widen this to staff-only kinds.
pub async fn list_client_visible_comments<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<TicketComment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    list_comments(
        executor,
        org_id,
        ticket_id,
        Some(core_domain::notes::visible_kinds(
            core_domain::notes::NoteAudience::Portal,
        )),
    )
    .await
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

/// Toggles one checklist item. `checked` semantics are UNCHANGED from
/// 0004; `na` is the shared not-applicable state (Wave 12 lane 6) —
/// mutually exclusive with `checked` (the API validates), and setting
/// either clears the other's stamps: an N-A item carries no checkmark
/// provenance, and unchecking clears it too.
pub async fn set_checklist_item<'e, E>(
    executor: E,
    org_id: Uuid,
    item_id: Uuid,
    checked: bool,
    na: bool,
    checked_by: Option<Uuid>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE checklist_items SET checked = $3, na = $4,
             checked_by = CASE WHEN $3 AND NOT $4 THEN $5::uuid ELSE NULL END,
             checked_at = CASE WHEN $3 AND NOT $4 THEN now() ELSE NULL END
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        item_id,
        checked,
        na,
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
        r#"SELECT id, org_id, ticket_id, text, checked, na, sort_order, checked_by, checked_at,
                  template_item_id
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
            na: row.na,
            sort_order: row.sort_order,
            checked_by: row.checked_by,
            checked_at: row.checked_at,
            template_item_id: row.template_item_id,
        })
        .collect())
}

pub async fn parse_priority_or_err(s: &str) -> DbResult<Priority> {
    parse_priority(s)
}
