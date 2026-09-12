//! Checklist templates (per board), ticket-instance instantiation, and the
//! derived progress view for pre-close enforcement.
//!
//! Purpose: persistence for TASKLIST 2.8's checklist half — template CRUD,
//! lazy idempotent instantiation of template items onto tickets, and the
//! read-time progress counts that close-time gating consumes. Instance rows
//! live in `checklist_items` (tickets.rs); this module only manages their
//! template linkage (`template_item_id`).
//!
//! Non-purpose: no enforcement decision (core-domain `notes`), no HTTP
//! shapes, and no ad-hoc item CRUD (that stays on tickets.rs).

use crate::{DbError, DbResult};
use core_domain::entities::{
    ChecklistItem, ChecklistProgress, ChecklistTemplate, ChecklistTemplateItem,
};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

fn parse_template_row(row: sqlx::postgres::PgRow) -> DbResult<ChecklistTemplate> {
    use sqlx::Row;
    Ok(ChecklistTemplate {
        id: row.get("id"),
        org_id: row.get("org_id"),
        board_id: row.get("board_id"),
        name: row.get("name"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const TEMPLATE_COLS: &str = "id, org_id, board_id, name, created_at, updated_at";

/// The board's checklist template, if any. `None` ⇒ closing is never
/// checklist-blocked for this board (defaults-first doctrine).
pub async fn get_template_for_board<'e, E>(
    executor: E,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<Option<ChecklistTemplate>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {TEMPLATE_COLS} FROM board_checklist_templates \
         WHERE org_id = $1 AND board_id = $2"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(board_id)
        .fetch_optional(executor)
        .await?
        .map(parse_template_row)
        .transpose()
}

/// The template's items, sort-order stable.
pub async fn list_template_items<'e, E>(
    executor: E,
    org_id: Uuid,
    template_id: Uuid,
) -> DbResult<Vec<ChecklistTemplateItem>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, template_id, text, sort_order
           FROM checklist_template_items
           WHERE org_id = $1 AND template_id = $2 ORDER BY sort_order"#,
        org_id,
        template_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ChecklistTemplateItem {
            id: r.id,
            org_id: r.org_id,
            template_id: r.template_id,
            text: r.text,
            sort_order: r.sort_order,
        })
        .collect())
}

/// Creates or replaces the board's template wholesale (one-per-board
/// upsert). Item sync is IDENTITY-PRESERVING: incoming texts matching an
/// existing item keep that item's row id, so ticket instances retain their
/// provenance and checked state across template edits; only genuinely new
/// texts become new rows and removed texts disappear (their instances
/// demote to ad-hoc via ON DELETE SET NULL). Returns the stored template
/// with its fresh items.
pub async fn upsert_template(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    template_id: Uuid,
    name: &str,
    items: &[String],
) -> DbResult<(ChecklistTemplate, Vec<ChecklistTemplateItem>)> {
    let row = sqlx::query!(
        r#"INSERT INTO board_checklist_templates (id, org_id, board_id, name)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (board_id) DO UPDATE SET name = $4, updated_at = now()
           RETURNING id, org_id, board_id, name, created_at, updated_at"#,
        template_id,
        org_id,
        board_id,
        name,
    )
    .fetch_one(&mut *conn)
    .await?;
    let stored = ChecklistTemplate {
        id: row.id,
        org_id: row.org_id,
        board_id: row.board_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    // Diff-sync items by text (see doc comment): keep matching ids so
    // instance linkage survives edits; drop vanished texts; append new ones.
    let existing = list_template_items(&mut *conn, org_id, stored.id).await?;
    let mut kept: std::collections::HashMap<String, Uuid> =
        existing.into_iter().map(|i| (i.text, i.id)).collect();
    for (pos, text) in items.iter().enumerate() {
        match kept.remove(text.as_str()) {
            Some(id) => {
                sqlx::query!(
                    "UPDATE checklist_template_items SET sort_order = $2 WHERE id = $1",
                    id,
                    pos as i32,
                )
                .execute(&mut *conn)
                .await?;
            }
            None => {
                sqlx::query!(
                    r#"INSERT INTO checklist_template_items (id, org_id, template_id, text, sort_order)
                       VALUES ($1, $2, $3, $4, $5)"#,
                    Uuid::now_v7(),
                    org_id,
                    stored.id,
                    text,
                    pos as i32,
                )
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    // Whatever remains in `kept` was removed from the template.
    for (_, id) in kept {
        sqlx::query!("DELETE FROM checklist_template_items WHERE id = $1", id)
            .execute(&mut *conn)
            .await?;
    }

    let stored_items = list_template_items(&mut *conn, org_id, stored.id).await?;
    Ok((stored, stored_items))
}

/// Removes the board's template. Template-derived instances demote to
/// ad-hoc items via ON DELETE SET NULL — nothing on a ticket is destroyed.
pub async fn delete_template<'e, E>(executor: E, org_id: Uuid, board_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        "DELETE FROM board_checklist_templates WHERE org_id = $1 AND board_id = $2",
        org_id,
        board_id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The per-board enforcement toggle (defaults OFF — see migration 0016).
pub async fn set_require_checklist_close<'e, E>(
    executor: E,
    org_id: Uuid,
    board_id: Uuid,
    required: bool,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE boards SET require_checklist_close = $3
           WHERE org_id = $1 AND id = $2
           RETURNING require_checklist_close"#,
        org_id,
        board_id,
        required
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row.require_checklist_close)
}

/// Instantiates any template items the ticket does not yet carry as linked
/// instances. Idempotent by construction (NOT EXISTS on the link), so it is
/// safe to call on every close attempt / checklist read: template edits
/// flow onto tickets lazily without ever duplicating rows. Returns the
/// number of instances created (0 on steady state).
pub async fn instantiate_missing(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<usize> {
    let result = sqlx::query!(
        r#"INSERT INTO checklist_items (id, org_id, ticket_id, text, sort_order, template_item_id)
           SELECT gen_random_uuid(), $2, $3, t.text, t.sort_order, t.id
           FROM checklist_template_items t
           JOIN board_checklist_templates bct ON bct.id = t.template_id
           WHERE bct.org_id = $2 AND bct.board_id = $1
             AND NOT EXISTS (
               SELECT 1 FROM checklist_items ci
               WHERE ci.ticket_id = $3 AND ci.template_item_id = t.id
             )"#,
        board_id,
        org_id,
        ticket_id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() as usize)
}

/// Derived progress for one ticket's checklist (read-time COUNTs — see the
/// 0016 header for why this is not materialized).
pub async fn progress<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<ChecklistProgress>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS total,
                   COUNT(*) FILTER (WHERE checked) AS checked,
                   COUNT(*) FILTER (WHERE template_item_id IS NOT NULL AND NOT checked
                                      AND NOT na) AS required_open
            FROM checklist_items
            WHERE org_id = $1 AND ticket_id = $2"#,
        org_id,
        ticket_id,
    )
    .fetch_one(executor)
    .await?;
    Ok(ChecklistProgress {
        total: row.total.unwrap_or(0),
        checked: row.checked.unwrap_or(0),
        required_open: row.required_open.unwrap_or(0),
    })
}

/// Convenience read used by close gating and UI: full instance list plus
/// derived progress in one round-trip.
pub async fn view(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
) -> DbResult<(Vec<ChecklistItem>, ChecklistProgress)> {
    let p = progress(&mut *conn, org_id, ticket_id).await?;
    let items = crate::tickets::list_checklist(&mut *conn, org_id, ticket_id).await?;
    Ok((items, p))
}
