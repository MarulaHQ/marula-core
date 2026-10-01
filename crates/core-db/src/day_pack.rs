//! Day-pack reads and writes (Wave 32 lane 8, `p32-today-day-pack`).
//!
//! Purpose: the persistence behind the tech's `/today` panel — manual
//! per-ticket picks, org dispatch rules (recommended items, default kits),
//! tick-off plan notes, and the day-context read (scheduled stops with the
//! ticket's board/type/category, checklist reminders, and the stock lookups
//! the panel's "from where" and availability answers need).
//!
//! Non-purpose: any movement write. Nothing in this module touches
//! `stock_movements`; collection stays with the checkout transactions and
//! the consume/transfer legs.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{DbError, DbResult};

fn unique_conflict(error: sqlx::Error, what: &str) -> DbError {
    if let sqlx::Error::Database(ref db) = error {
        if db.code().as_deref() == Some("23505") {
            return DbError::Conflict(what.to_string());
        }
    }
    DbError::Sql(error)
}

// ---------------------------------------------------------------------------
// Manual per-ticket picks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TicketPickRow {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub stock_item_id: Uuid,
    pub qty_milli: i64,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_ticket_picks(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_ids: &[Uuid],
) -> DbResult<Vec<TicketPickRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, ticket_id, stock_item_id, qty_milli, note, created_at
           FROM ticket_stock_picks
           WHERE org_id = $1 AND ticket_id = ANY($2)
           ORDER BY created_at"#,
        org_id,
        ticket_ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TicketPickRow {
            id: r.id,
            ticket_id: r.ticket_id,
            stock_item_id: r.stock_item_id,
            qty_milli: r.qty_milli,
            note: r.note,
            created_at: r.created_at,
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_ticket_pick(
    conn: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    ticket_id: Uuid,
    stock_item_id: Uuid,
    qty_milli: i64,
    note: Option<&str>,
    created_by: Option<Uuid>,
) -> DbResult<Uuid> {
    sqlx::query!(
        r#"INSERT INTO ticket_stock_picks
             (id, org_id, ticket_id, stock_item_id, qty_milli, note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        id,
        org_id,
        ticket_id,
        stock_item_id,
        qty_milli,
        note,
        created_by,
    )
    .execute(&mut *conn)
    .await
    .map_err(|e| unique_conflict(e, "this item is already pinned to the ticket"))?;
    Ok(id)
}

pub async fn delete_ticket_pick(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let result = sqlx::query!(
        r#"DELETE FROM ticket_stock_picks WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// One pick by id (the delete route's `before` snapshot).
pub async fn get_ticket_pick(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<TicketPickRow> {
    let row = sqlx::query!(
        r#"SELECT id, ticket_id, stock_item_id, qty_milli, note, created_at
           FROM ticket_stock_picks
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(TicketPickRow {
        id: row.id,
        ticket_id: row.ticket_id,
        stock_item_id: row.stock_item_id,
        qty_milli: row.qty_milli,
        note: row.note,
        created_at: row.created_at,
    })
}

// ---------------------------------------------------------------------------
// Org dispatch rules
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PackRuleRow {
    pub id: Uuid,
    pub scope_kind: String,
    pub scope_id: Uuid,
    pub target_kind: String,
    pub stock_item_id: Option<Uuid>,
    pub kit_id: Option<Uuid>,
    pub qty_milli: i64,
    pub note: Option<String>,
    pub sort_order: i32,
    pub active: bool,
}

/// Every rule of the org, active or not: the config surface edits all of
/// them, the panel filters `active`.
pub async fn list_pack_rules(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<PackRuleRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, scope_kind, scope_id, target_kind, stock_item_id, kit_id,
                  qty_milli, note, sort_order, active
           FROM dispatch_pack_rules
           WHERE org_id = $1
           ORDER BY scope_kind, sort_order, created_at"#,
        org_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PackRuleRow {
            id: r.id,
            scope_kind: r.scope_kind,
            scope_id: r.scope_id,
            target_kind: r.target_kind,
            stock_item_id: r.stock_item_id,
            kit_id: r.kit_id,
            qty_milli: r.qty_milli,
            note: r.note,
            sort_order: r.sort_order,
            active: r.active,
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_pack_rule(
    conn: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    scope_kind: &str,
    scope_id: Uuid,
    target_kind: &str,
    stock_item_id: Option<Uuid>,
    kit_id: Option<Uuid>,
    qty_milli: i64,
    note: Option<&str>,
    created_by: Option<Uuid>,
) -> DbResult<Uuid> {
    sqlx::query!(
        r#"INSERT INTO dispatch_pack_rules
             (id, org_id, scope_kind, scope_id, target_kind, stock_item_id, kit_id,
              qty_milli, note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        id,
        org_id,
        scope_kind,
        scope_id,
        target_kind,
        stock_item_id,
        kit_id,
        qty_milli,
        note,
        created_by,
    )
    .execute(&mut *conn)
    .await
    .map_err(|e| unique_conflict(e, "this scope already maps to that target"))?;
    Ok(id)
}

pub async fn set_pack_rule_active(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    active: bool,
) -> DbResult<bool> {
    let result = sqlx::query!(
        r#"UPDATE dispatch_pack_rules SET active = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        active,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_pack_rule(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let result = sqlx::query!(
        r#"DELETE FROM dispatch_pack_rules WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Tick-off plan notes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DayPackTickRow {
    pub line_key: String,
    pub ticket_id: Option<Uuid>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_ticks(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
) -> DbResult<Vec<DayPackTickRow>> {
    let rows = sqlx::query!(
        r#"SELECT line_key, ticket_id, note, created_at
           FROM day_pack_ticks
           WHERE org_id = $1 AND user_id = $2 AND day = $3
           ORDER BY created_at"#,
        org_id,
        user_id,
        day,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DayPackTickRow {
            line_key: r.line_key,
            ticket_id: r.ticket_id,
            note: r.note,
            created_at: r.created_at,
        })
        .collect())
}

/// The row's id, timestamp and note by natural key (the untick path).
pub async fn find_tick(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
    line_key: &str,
) -> DbResult<Option<(Uuid, DateTime<Utc>, Option<String>)>> {
    let row = sqlx::query!(
        r#"SELECT id, created_at, note
           FROM day_pack_ticks
           WHERE org_id = $1 AND user_id = $2 AND day = $3 AND line_key = $4"#,
        org_id,
        user_id,
        day,
        line_key,
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| (r.id, r.created_at, r.note)))
}

/// Writes the plan note for one line. Ticking twice keeps one row (and the
/// first timestamp); the tick is a plan note, never a movement. Returns the
/// row id and tick timestamp so the envelope can key the write.
pub async fn upsert_tick(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
    line_key: &str,
    ticket_id: Option<Uuid>,
    note: Option<&str>,
) -> DbResult<(Uuid, DateTime<Utc>)> {
    let row = sqlx::query!(
        r#"INSERT INTO day_pack_ticks
             (id, org_id, user_id, day, line_key, ticket_id, note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $3)
           ON CONFLICT (org_id, user_id, day, line_key)
           DO UPDATE SET note = EXCLUDED.note, ticket_id = EXCLUDED.ticket_id
           RETURNING id, created_at"#,
        Uuid::now_v7(),
        org_id,
        user_id,
        day,
        line_key,
        ticket_id,
        note,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok((row.id, row.created_at))
}

pub async fn delete_tick(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
    line_key: &str,
) -> DbResult<bool> {
    let result = sqlx::query!(
        r#"DELETE FROM day_pack_ticks
           WHERE org_id = $1 AND user_id = $2 AND day = $3 AND line_key = $4"#,
        org_id,
        user_id,
        day,
        line_key,
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Day context: stops, checklist reminders, stock lookups
// ---------------------------------------------------------------------------

/// One scheduled stop with everything the panel groups by: the appointment,
/// the ticket's board/type/category (rule scopes), and the site/client
/// labels.
#[derive(Debug, Clone)]
pub struct ScheduledStopRow {
    pub appointment_id: Uuid,
    pub ticket_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub title: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub kind: String,
    pub status: String,
    pub travel_minutes: i32,
    pub travel_from: Option<serde_json::Value>,
    pub travel_provider: Option<String>,
    pub travel_quality: Option<String>,
    pub ticket_number: Option<String>,
    pub board_id: Option<Uuid>,
    pub type_id: Option<Uuid>,
    pub category_id: Option<Uuid>,
    pub site_label: Option<String>,
    pub client_name: Option<String>,
}

pub async fn scheduled_stops_between(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> DbResult<Vec<ScheduledStopRow>> {
    let rows = sqlx::query!(
        r#"SELECT a.id, a.ticket_id, a.client_id, a.site_id, a.title,
                  a.starts_at, a.ends_at, a.kind::text AS "kind!", a.status::text AS "status!",
                  a.travel_minutes, a.travel_from, a.travel_provider, a.travel_quality,
                  t.number AS "ticket_number?", t.board_id AS "board_id?",
                  t.type_id AS "type_id?", t.category_id AS "category_id?",
                  s.name AS "site_label?", c.name AS "client_name?"
           FROM appointments a
           LEFT JOIN tickets t ON t.id = a.ticket_id
           LEFT JOIN sites s ON s.id = a.site_id
           LEFT JOIN clients c ON c.id = a.client_id
           WHERE a.org_id = $1 AND a.user_id = $2
             AND a.status <> 'cancelled'
             AND a.starts_at < $3 AND a.ends_at > $4
           ORDER BY a.starts_at"#,
        org_id,
        user_id,
        to,
        from,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ScheduledStopRow {
            appointment_id: r.id,
            ticket_id: r.ticket_id,
            client_id: r.client_id,
            site_id: r.site_id,
            title: r.title,
            starts_at: r.starts_at,
            ends_at: r.ends_at,
            kind: r.kind,
            status: r.status,
            travel_minutes: r.travel_minutes,
            travel_from: r.travel_from,
            travel_provider: r.travel_provider,
            travel_quality: r.travel_quality,
            ticket_number: r.ticket_number,
            board_id: r.board_id,
            type_id: r.type_id,
            category_id: r.category_id,
            site_label: r.site_label,
            client_name: r.client_name,
        })
        .collect())
}

#[derive(Debug, Clone)]
pub struct ChecklistReminderRow {
    pub id: Uuid,
    pub ticket_id: Uuid,
    pub text: String,
}

/// Open (unchecked, not marked N/A) checklist tasks for the given tickets:
/// the panel's plain reminders and the raw material for conservative item
/// mentions.
pub async fn open_checklist_items(
    conn: &mut PgConnection,
    org_id: Uuid,
    ticket_ids: &[Uuid],
) -> DbResult<Vec<ChecklistReminderRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, ticket_id, text
           FROM checklist_items
           WHERE org_id = $1 AND ticket_id = ANY($2)
             AND NOT checked AND NOT na
           ORDER BY ticket_id, sort_order"#,
        org_id,
        ticket_ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ChecklistReminderRow {
            id: r.id,
            ticket_id: r.ticket_id,
            text: r.text,
        })
        .collect())
}

/// The stock items the panel needs, with the fields a grab line shows.
#[derive(Debug, Clone)]
pub struct PackItemRow {
    pub id: Uuid,
    pub name: String,
    pub sku: Option<String>,
    pub part_number: Option<String>,
    pub unit: String,
    pub kind: String,
    pub is_tool: bool,
    pub active: bool,
}

pub async fn items_by_ids(
    conn: &mut PgConnection,
    org_id: Uuid,
    ids: &[Uuid],
) -> DbResult<Vec<PackItemRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, sku, part_number, unit, kind, is_tool, active
           FROM stock_items
           WHERE org_id = $1 AND id = ANY($2)"#,
        org_id,
        ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PackItemRow {
            id: r.id,
            name: r.name,
            sku: r.sku,
            part_number: r.part_number,
            unit: r.unit,
            kind: r.kind,
            is_tool: r.is_tool,
            active: r.active,
        })
        .collect())
}

/// Positive consumable levels with their location context: "where is there
/// stock to grab".
#[derive(Debug, Clone)]
pub struct ConsumableLevelRow {
    pub stock_item_id: Uuid,
    pub location_id: Uuid,
    pub location_name: String,
    pub location_kind: String,
    pub assigned_user_id: Option<Uuid>,
    pub qty_milli: i64,
}

pub async fn consumable_levels(
    conn: &mut PgConnection,
    org_id: Uuid,
    item_ids: &[Uuid],
) -> DbResult<Vec<ConsumableLevelRow>> {
    let rows = sqlx::query!(
        r#"SELECT l.stock_item_id, l.stock_location_id, loc.name AS location_name,
                  loc.kind AS location_kind, loc.assigned_user_id, l.qty_milli
           FROM stock_levels l
           JOIN stock_locations loc ON loc.id = l.stock_location_id
           WHERE l.org_id = $1 AND l.stock_item_id = ANY($2)
             AND loc.active AND l.qty_milli > 0
           ORDER BY loc.name"#,
        org_id,
        item_ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ConsumableLevelRow {
            stock_item_id: r.stock_item_id,
            location_id: r.stock_location_id,
            location_name: r.location_name,
            location_kind: r.location_kind,
            assigned_user_id: r.assigned_user_id,
            qty_milli: r.qty_milli,
        })
        .collect())
}

/// One serialized unit of a tool, wherever it is and whoever has it.
#[derive(Debug, Clone)]
pub struct ToolUnitRow {
    pub stock_item_id: Uuid,
    pub serial_id: Uuid,
    pub status: String,
    pub location_id: Option<Uuid>,
    pub location_name: Option<String>,
    pub location_kind: Option<String>,
    pub assigned_user_id: Option<Uuid>,
    pub holder_user_id: Option<Uuid>,
    pub holder_name: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub loan_client_name: Option<String>,
    pub expected_return_at: Option<NaiveDate>,
}

pub async fn tool_units(
    conn: &mut PgConnection,
    org_id: Uuid,
    item_ids: &[Uuid],
) -> DbResult<Vec<ToolUnitRow>> {
    let rows = sqlx::query!(
        r#"SELECT ss.stock_item_id, ss.id AS serial_id, ss.status::text AS "status!",
                  ss.location_id AS "location_id?", sl.name AS "location_name?",
                  sl.kind AS "location_kind?", sl.assigned_user_id AS "assigned_user_id?",
                  oc.holder_user_id AS "holder_user_id?", u.display_name AS "holder_name?",
                  oc.due_at AS "due_at?",
                  c.name AS "loan_client_name?", cl.expected_return_at AS "expected_return_at?"
           FROM stock_serials ss
           LEFT JOIN stock_locations sl ON sl.id = ss.location_id
           LEFT JOIN tool_checkout_lines ocl
             ON ocl.serial_id = ss.id AND ocl.org_id = ss.org_id AND ocl.returned_at IS NULL
           LEFT JOIN tool_checkouts oc ON oc.id = ocl.checkout_id
           LEFT JOIN users u ON u.id = oc.holder_user_id
           LEFT JOIN client_loan_lines ll
             ON ll.serial_id = ss.id AND ll.org_id = ss.org_id AND ll.returned_at IS NULL
           LEFT JOIN client_loans cl ON cl.id = ll.loan_id
           LEFT JOIN clients c ON c.id = cl.client_id
           WHERE ss.org_id = $1 AND ss.stock_item_id = ANY($2)
           ORDER BY ss.stock_item_id, ss.serial_number"#,
        org_id,
        item_ids,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ToolUnitRow {
            stock_item_id: r.stock_item_id,
            serial_id: r.serial_id,
            status: r.status,
            location_id: r.location_id,
            location_name: r.location_name,
            location_kind: r.location_kind,
            assigned_user_id: r.assigned_user_id,
            holder_user_id: r.holder_user_id,
            holder_name: r.holder_name,
            due_at: r.due_at,
            loan_client_name: r.loan_client_name,
            expected_return_at: r.expected_return_at,
        })
        .collect())
}

/// The tech's own van (a `van`/`truck` location assigned to them): the first
/// place the panel looks for gear that never has to leave the vehicle.
pub async fn tech_vehicles(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<(Uuid, String)>> {
    let rows = sqlx::query!(
        r#"SELECT id, name FROM stock_locations
           WHERE org_id = $1 AND assigned_user_id = $2
             AND kind IN ('van', 'truck') AND active
           ORDER BY name"#,
        org_id,
        user_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.name)).collect())
}

// ---------------------------------------------------------------------------
// Rule scope resolution (polymorphic by kind; validated app-side)
// ---------------------------------------------------------------------------

/// Whether the rule scope exists in this org. `scope_id` is a FK-less
/// polymorphic reference, so the API validates it before any write and the
/// config surface shows a typed "missing scope" when a row goes stale.
pub async fn scope_exists(
    conn: &mut PgConnection,
    org_id: Uuid,
    scope_kind: &str,
    scope_id: Uuid,
) -> DbResult<bool> {
    let sql = match scope_kind {
        "board" => "SELECT EXISTS(SELECT 1 FROM boards WHERE org_id = $1 AND id = $2)",
        "ticket_type" => "SELECT EXISTS(SELECT 1 FROM ticket_types WHERE org_id = $1 AND id = $2)",
        "ticket_category" => {
            "SELECT EXISTS(SELECT 1 FROM ticket_categories WHERE org_id = $1 AND id = $2)"
        }
        _ => return Ok(false),
    };
    let exists = sqlx::query_scalar::<_, bool>(sql)
        .bind(org_id)
        .bind(scope_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(exists)
}

/// The scope's display name, `None` when the referenced row is gone.
pub async fn scope_name(
    conn: &mut PgConnection,
    org_id: Uuid,
    scope_kind: &str,
    scope_id: Uuid,
) -> DbResult<Option<String>> {
    let sql = match scope_kind {
        "board" => "SELECT name FROM boards WHERE org_id = $1 AND id = $2",
        "ticket_type" => "SELECT name FROM ticket_types WHERE org_id = $1 AND id = $2",
        "ticket_category" => "SELECT name FROM ticket_categories WHERE org_id = $1 AND id = $2",
        _ => return Ok(None),
    };
    let name = sqlx::query_scalar::<_, String>(sql)
        .bind(org_id)
        .bind(scope_id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(name)
}
