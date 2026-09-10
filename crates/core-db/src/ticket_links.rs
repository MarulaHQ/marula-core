//! Ticket links (Wave 11 lane 9, migration 0056): typed ticket-to-ticket
//! edges (parent-child, major-incident) + the merge fold.
//!
//! Purpose: persistence for the linking construct — link CRUD, rollup
//! reads, and the one-transaction merge fold (children repointed,
//! comments/time/costs/expenses moved to the survivor, source marked
//! `merged_into_ticket_id`).
//!
//! Non-purpose: no conflict policy (routes decide typed conflicts before
//! writing), no event emission, no cascade beyond what FKs already give.

use crate::{DbError, DbResult};
use core_domain::entities::{TicketLink, TicketLinkType, TicketRollup};
use sqlx::Postgres;
use uuid::Uuid;

const LINK_COLS: &str =
    "id, org_id, from_ticket_id, to_ticket_id, link_type, created_by, created_at";

fn link_from(row: sqlx::postgres::PgRow) -> DbResult<TicketLink> {
    use sqlx::Row;
    let link_type: String = row.get("link_type");
    Ok(TicketLink {
        id: row.get("id"),
        org_id: row.get("org_id"),
        from_ticket_id: row.get("from_ticket_id"),
        to_ticket_id: row.get("to_ticket_id"),
        link_type: TicketLinkType::parse(&link_type).ok_or(DbError::NotFound)?,
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

pub async fn list_links_from<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<TicketLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {LINK_COLS} FROM ticket_links WHERE org_id = $1 AND from_ticket_id = $2 \
         ORDER BY created_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(link_from).collect()
}

pub async fn list_links_to<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<Vec<TicketLink>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {LINK_COLS} FROM ticket_links WHERE org_id = $1 AND to_ticket_id = $2 \
         ORDER BY created_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(ticket_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(link_from).collect()
}

pub async fn insert_link<'e, E>(executor: E, link: &TicketLink) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO ticket_links (id, org_id, from_ticket_id, to_ticket_id, link_type, created_by)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
        link.id,
        link.org_id,
        link.from_ticket_id,
        link.to_ticket_id,
        link.link_type.as_str(),
        link.created_by
    )
    .execute(executor)
    .await?;
    Ok(())
}

/// Removes one edge. True when a row went away.
pub async fn remove_link<'e, E>(executor: E, org_id: Uuid, link_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM ticket_links WHERE org_id = $1 AND id = $2",
        org_id,
        link_id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected() > 0)
}

// --- rollups ------------------------------------------------------------------------

/// The rollup read: child/member counts plus summed folded time and
/// billable money. Children/members that are closed or merged still
/// count toward totals (history stays queryable); only the OPEN split
/// excludes them.
pub async fn rollup_for_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<TicketRollup>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"WITH child_links AS (
             SELECT from_ticket_id AS id FROM ticket_links
             WHERE org_id = $1 AND to_ticket_id = $2 AND link_type = 'parent_child'
           ), mi_links AS (
             SELECT from_ticket_id AS id FROM ticket_links
             WHERE org_id = $1 AND to_ticket_id = $2 AND link_type = 'major_incident'
           ), fold_ids AS (
             SELECT id FROM child_links UNION SELECT id FROM mi_links
           )
           SELECT
             (SELECT count(*)::bigint FROM child_links) AS child_count,
             (SELECT count(*)::bigint FROM child_links c
                JOIN tickets t ON t.org_id = $1 AND t.id = c.id
                WHERE t.category <> 'closed' AND t.merged_into_ticket_id IS NULL
             ) AS child_open_count,
             (SELECT count(*)::bigint FROM mi_links) AS mi_count,
             (
               (SELECT COALESCE(sum(minutes), 0)::bigint FROM time_entries
                WHERE org_id = $1 AND ticket_id = $2)
               + (SELECT COALESCE(sum(te.minutes), 0)::bigint FROM time_entries te
                  JOIN fold_ids f ON te.org_id = $1 AND te.ticket_id = f.id)
             ) AS time_minutes,
             (
               (SELECT COALESCE(sum(quantity * unit_cost_minor), 0)::bigint
                FROM ticket_cost_lines WHERE org_id = $1 AND ticket_id = $2 AND billable)
               + (SELECT COALESCE(sum(amount_minor), 0)::bigint FROM ticket_expenses
                  WHERE org_id = $1 AND ticket_id = $2 AND billable)
               + (SELECT COALESCE(sum(tcl.quantity * tcl.unit_cost_minor), 0)::bigint
                  FROM ticket_cost_lines tcl
                  JOIN fold_ids f ON tcl.org_id = $1 AND tcl.ticket_id = f.id AND tcl.billable)
               + (SELECT COALESCE(sum(ex.amount_minor), 0)::bigint FROM ticket_expenses ex
                  JOIN fold_ids f ON ex.org_id = $1 AND ex.ticket_id = f.id AND ex.billable)
             ) AS billable_cost_minor"#,
        org_id,
        ticket_id
    )
    .fetch_one(executor)
    .await?;
    Ok(TicketRollup {
        ticket_id,
        child_count: row.child_count.unwrap_or(0),
        child_open_count: row.child_open_count.unwrap_or(0),
        major_incident_member_count: row.mi_count.unwrap_or(0),
        time_minutes: row.time_minutes.unwrap_or(0),
        billable_cost_minor: row.billable_cost_minor.unwrap_or(0),
    })
}

// --- merge fold -----------------------------------------------------------------------

#[derive(Debug)]
pub struct MergeFoldResult {
    pub comments_folded: i64,
    pub time_entries_folded: i64,
    pub time_minutes_folded: i64,
    pub cost_lines_folded: i64,
    pub expenses_folded: i64,
    pub children_relinked: i64,
}

/// Folds one source ticket's work into the survivor, in the caller's
/// transaction: comments, time entries, cost lines, expenses, and child
/// links re-point; the source's own tags re-apply (dedup-safe); the
/// source marks `merged_into_ticket_id`. The source's status close (its
/// board's closed-category status) is applied by the caller via
/// `tickets::move_to_board`-shaped writes so the status event contract
/// stays in the route layer.
pub async fn merge_fold(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    source_ticket_id: Uuid,
    survivor_ticket_id: Uuid,
) -> DbResult<MergeFoldResult> {
    // Guard: a merged source never folds twice (the route also checks;
    // this keeps a stale concurrent caller honest).
    let already: Option<Uuid> = sqlx::query_scalar(
        "SELECT merged_into_ticket_id FROM tickets WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(source_ticket_id)
    .fetch_optional(&mut *executor)
    .await?
    .flatten();
    if already.is_some() {
        return Err(DbError::Conflict(
            "source ticket has already been merged".into(),
        ));
    }
    // Comments + time entries move wholesale (ticket_id re-point). (ticket_id re-point).
    let comments = sqlx::query!(
        r#"UPDATE ticket_comments SET ticket_id = $3
           WHERE org_id = $1 AND ticket_id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    let time = sqlx::query!(
        r#"UPDATE time_entries SET ticket_id = $3
           WHERE org_id = $1 AND ticket_id = $2
           RETURNING minutes"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .fetch_all(&mut *executor)
    .await?;
    let time_minutes: i64 = time.iter().map(|r| r.minutes as i64).sum();
    let cost_lines = sqlx::query!(
        r#"UPDATE ticket_cost_lines SET ticket_id = $3, updated_at = now()
           WHERE org_id = $1 AND ticket_id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    let expenses = sqlx::query!(
        r#"UPDATE ticket_expenses SET ticket_id = $3, updated_at = now()
           WHERE org_id = $1 AND ticket_id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    // Children + member links re-point onto the survivor. Postgres has
    // no UPDATE ... ON CONFLICT, so the fold deletes the edges that
    // would collide first (survivor already has the same target/parent
    // edge, or the edge itself points at the survivor), then moves the
    // rest — identical semantics, no duplicate rows possible.
    sqlx::query!(
        r#"DELETE FROM ticket_links t WHERE org_id = $1 AND from_ticket_id = $2
             AND (to_ticket_id = $3 OR EXISTS (
               SELECT 1 FROM ticket_links d
               WHERE d.org_id = $1 AND d.from_ticket_id = $3
                 AND d.to_ticket_id = t.to_ticket_id AND d.link_type = t.link_type))"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?;
    let relinked_from = sqlx::query!(
        r#"UPDATE ticket_links SET from_ticket_id = $3
           WHERE org_id = $1 AND from_ticket_id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    sqlx::query!(
        r#"DELETE FROM ticket_links t WHERE org_id = $1 AND to_ticket_id = $2
             AND (from_ticket_id = $3 OR EXISTS (
               SELECT 1 FROM ticket_links d
               WHERE d.org_id = $1 AND d.to_ticket_id = $3
                 AND d.from_ticket_id = t.from_ticket_id AND d.link_type = t.link_type))"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?;
    let relinked_to = sqlx::query!(
        r#"UPDATE ticket_links SET to_ticket_id = $3
           WHERE org_id = $1 AND to_ticket_id = $2 AND from_ticket_id <> $3"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    // Mark the source folded: its row stays (doctrine #9) but points at
    // the survivor; the route closes it through the board-move path.
    sqlx::query!(
        r#"UPDATE tickets SET merged_into_ticket_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?;
    // Source's applied tags re-apply onto the survivor (dedup-safe).
    crate::tags::fold_tags(&mut *executor, org_id, source_ticket_id, survivor_ticket_id).await?;
    // Remaining links touching the source as an endpoint collapse.
    sqlx::query!(
        r#"DELETE FROM ticket_links WHERE org_id = $1
             AND (from_ticket_id = $2 OR to_ticket_id = $2)"#,
        org_id,
        source_ticket_id
    )
    .execute(&mut *executor)
    .await?;
    // Mark the source merged.
    sqlx::query!(
        r#"UPDATE tickets SET merged_into_ticket_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        source_ticket_id,
        survivor_ticket_id
    )
    .execute(&mut *executor)
    .await?;
    Ok(MergeFoldResult {
        comments_folded: comments as i64,
        time_entries_folded: time.len() as i64,
        time_minutes_folded: time_minutes,
        cost_lines_folded: cost_lines as i64,
        expenses_folded: expenses as i64,
        children_relinked: (relinked_from + relinked_to) as i64,
    })
}
