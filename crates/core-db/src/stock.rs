//! Stock + consumables persistence (Wave 32 lane 1, `p32-stock-consumables`;
//! migration 0102).
//!
//! The append-only `stock_movements` ledger is the ground truth: every
//! mutating call appends ONE ledger row and applies its deterministic
//! consequence to `stock_levels` + `stock_serials` in the SAME
//! transaction. Levels that would go negative refuse here (typed
//! conflict — the API layer maps to 409). All quantity math is i64
//! `*_milli` (no float ever touches a quantity) and money stays minor
//! units. The ADR 0002 inventory-receipt FX lock is computed by the API
//! layer (routes/fx.rs `lock_snapshot`) and arrives stamped on the
//! movement — this layer never re-derives money.
//!
//! Every read and write carries an explicit `org_id` predicate on top of
//! the migration's RLS policies (same posture as the rest of the crate).

use crate::{DbError, DbResult};
use core_domain::entities::{StockItem, StockLevel, StockLocation, StockMovement, StockSerial};
use uuid::Uuid;

// Row shapes -----------------------------------------------------------------

struct ItemRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    sku: Option<String>,
    catalog_item_id: Option<Uuid>,
    part_number: Option<String>,
    kind: String,
    unit: String,
    currency: String,
    last_unit_cost_minor: Option<i64>,
    active: bool,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn item_row(r: ItemRow) -> StockItem {
    StockItem {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        sku: r.sku,
        catalog_item_id: r.catalog_item_id,
        part_number: r.part_number,
        kind: r.kind,
        unit: r.unit,
        currency: r.currency,
        last_unit_cost_minor: r.last_unit_cost_minor,
        active: r.active,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

struct LocationRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    kind: String,
    code: Option<String>,
    parent_id: Option<Uuid>,
    assigned_user_id: Option<Uuid>,
    active: bool,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn location_row(r: LocationRow) -> StockLocation {
    StockLocation {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        kind: r.kind,
        code: r.code,
        parent_id: r.parent_id,
        assigned_user_id: r.assigned_user_id,
        active: r.active,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

struct LevelRow {
    id: Uuid,
    org_id: Uuid,
    stock_item_id: Uuid,
    stock_location_id: Uuid,
    qty_milli: i64,
    min_qty_milli: Option<i64>,
    max_qty_milli: Option<i64>,
    reorder_point_milli: Option<i64>,
    low_stock_notified_at: Option<chrono::DateTime<chrono::Utc>>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn level_row(r: LevelRow) -> StockLevel {
    StockLevel {
        id: r.id,
        org_id: r.org_id,
        stock_item_id: r.stock_item_id,
        stock_location_id: r.stock_location_id,
        qty_milli: r.qty_milli,
        min_qty_milli: r.min_qty_milli,
        max_qty_milli: r.max_qty_milli,
        reorder_point_milli: r.reorder_point_milli,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// Items ----------------------------------------------------------------------

/// INSERT a stock item; SKU uniqueness is DB-enforced (partial unique
/// index on lower(sku)) — the API layer maps the unique violation to 409.
pub async fn insert_stock_item<'e, E>(executor: E, item: &StockItem) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO stock_items (id, org_id, name, sku, catalog_item_id, part_number,
                                    kind, unit, currency, last_unit_cost_minor, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
        item.id,
        item.org_id,
        item.name.trim(),
        item.sku.as_deref().map(str::trim),
        item.catalog_item_id,
        item.part_number.as_deref(),
        item.kind,
        item.unit.trim(),
        item.currency,
        item.last_unit_cost_minor,
        item.active,
        item.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_stock_item<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<StockItem>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        ItemRow,
        r#"SELECT id, org_id, name, sku, catalog_item_id, part_number, kind, unit, currency,
                  last_unit_cost_minor, active, created_by, created_at, updated_at
           FROM stock_items WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(item_row(row))
}

/// Substring search over name/sku/part_number; `only_active = false`
/// returns archived rows too (archival is a flag, never a delete).
pub async fn list_stock_items(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    search: Option<&str>,
    only_active: bool,
    limit: i64,
) -> DbResult<Vec<StockItem>> {
    let grep = search.map(|s| format!("%{}%", s.trim().to_lowercase()));
    let rows = sqlx::query_as!(
        ItemRow,
        r#"SELECT id, org_id, name, sku, catalog_item_id, part_number, kind, unit, currency,
                  last_unit_cost_minor, active, created_by, created_at, updated_at
           FROM stock_items
           WHERE org_id = $1
             AND ($2::text IS NULL
                  OR lower(name) LIKE $2
                  OR lower(coalesce(sku, '')) LIKE $2
                  OR lower(coalesce(part_number, '')) LIKE $2)
             AND (NOT $3::boolean OR active)
           ORDER BY created_at DESC
           LIMIT $4"#,
        org_id,
        grep,
        only_active,
        limit,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(item_row).collect())
}

/// PATCH — the route loads the current row first and passes every field;
/// only fields the request carries differ from the stored values.
pub async fn update_stock_item<'e, E>(executor: E, item: &StockItem) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let n = sqlx::query!(
        r#"UPDATE stock_items
             SET name = $3, sku = $4, catalog_item_id = $5, part_number = $6,
                 kind = $7, unit = $8, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        item.org_id,
        item.id,
        item.name.trim(),
        item.sku.as_deref().map(str::trim),
        item.catalog_item_id,
        item.part_number.as_deref(),
        item.kind,
        item.unit.trim(),
    )
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Archival is a data flag (`active = false`); the ledger history stays.
pub async fn archive_stock_item<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let n = sqlx::query!(
        r#"UPDATE stock_items SET active = false, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND active"#,
        org_id,
        id
    )
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// Locations ------------------------------------------------------------------

pub async fn insert_stock_location<'e, E>(executor: E, loc: &StockLocation) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO stock_locations (id, org_id, name, kind, code, parent_id,
                                        assigned_user_id, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#,
        loc.id,
        loc.org_id,
        loc.name.trim(),
        loc.kind,
        loc.code.as_deref().map(str::trim),
        loc.parent_id,
        loc.assigned_user_id,
        loc.active,
        loc.created_by,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_stock_location<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<StockLocation>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        LocationRow,
        r#"SELECT id, org_id, name, kind, code, parent_id, assigned_user_id, active,
                  created_by, created_at, updated_at
           FROM stock_locations WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(location_row(row))
}

pub async fn list_stock_locations(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    kind: Option<&str>,
    only_active: bool,
    limit: i64,
) -> DbResult<Vec<StockLocation>> {
    let rows = sqlx::query_as!(
        LocationRow,
        r#"SELECT id, org_id, name, kind, code, parent_id, assigned_user_id, active,
                  created_by, created_at, updated_at
           FROM stock_locations
           WHERE org_id = $1 AND ($2::text IS NULL OR kind = $2)
             AND (NOT $3::boolean OR active)
           ORDER BY created_at DESC
           LIMIT $4"#,
        org_id,
        kind,
        only_active,
        limit,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(location_row).collect())
}

pub async fn update_stock_location<'e, E>(executor: E, loc: &StockLocation) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let n = sqlx::query!(
        r#"UPDATE stock_locations
             SET name = $3, kind = $4, code = $5, assigned_user_id = $6,
                 active = $7, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        loc.org_id,
        loc.id,
        loc.name.trim(),
        loc.kind,
        loc.code.as_deref().map(str::trim),
        loc.assigned_user_id,
        loc.active,
    )
    .execute(executor)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// Levels ---------------------------------------------------------------------

pub async fn get_level<'e, E>(
    executor: E,
    org_id: Uuid,
    item_id: Uuid,
    location_id: Uuid,
) -> DbResult<StockLevel>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        LevelRow,
        r#"SELECT id, org_id, stock_item_id, stock_location_id, qty_milli,
                  min_qty_milli, max_qty_milli, reorder_point_milli,
                  low_stock_notified_at, created_at, updated_at
           FROM stock_levels
           WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3"#,
        org_id,
        item_id,
        location_id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(level_row(row))
}

/// The stock-on-hand base query: every level row (optionally scoped to
/// one item and/or one location). The named read models are exposed by
/// the public query API (routes).
pub async fn list_levels(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Option<Uuid>,
    location_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<StockLevel>> {
    let rows = sqlx::query_as!(
        LevelRow,
        r#"SELECT id, org_id, stock_item_id, stock_location_id, qty_milli,
                  min_qty_milli, max_qty_milli, reorder_point_milli,
                  low_stock_notified_at, created_at, updated_at
           FROM stock_levels
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR stock_item_id = $2)
             AND ($3::uuid IS NULL OR stock_location_id = $3)
           ORDER BY updated_at DESC
           LIMIT $4"#,
        org_id,
        item_id,
        location_id,
        limit,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(level_row).collect())
}

/// Set min/max/reorder thresholds on a (item, location) level row,
/// creating the row when absent. NULL clears the threshold.
pub async fn set_thresholds<'e, E>(
    executor: E,
    org_id: Uuid,
    item_id: Uuid,
    location_id: Uuid,
    min_qty_milli: Option<i64>,
    max_qty_milli: Option<i64>,
    reorder_point_milli: Option<i64>,
) -> DbResult<StockLevel>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row = sqlx::query_as!(
        LevelRow,
        r#"INSERT INTO stock_levels (id, org_id, stock_item_id, stock_location_id,
                                     min_qty_milli, max_qty_milli, reorder_point_milli)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (org_id, stock_item_id, stock_location_id) DO UPDATE
             SET min_qty_milli = $5, max_qty_milli = $6,
                 reorder_point_milli = $7, updated_at = now()
           RETURNING id, org_id, stock_item_id, stock_location_id, qty_milli,
                     min_qty_milli, max_qty_milli, reorder_point_milli,
                     low_stock_notified_at, created_at, updated_at"#,
        Uuid::now_v7(),
        org_id,
        item_id,
        location_id,
        min_qty_milli,
        max_qty_milli,
        reorder_point_milli,
    )
    .fetch_one(executor)
    .await?;
    Ok(level_row(row))
}

// The ledger engine -----------------------------------------------------------

/// Everything the ledger engine needs beyond the validated movement.
pub struct LedgerInput {
    /// The movement (the caller ran `validate` on it).
    pub movement: StockMovement,
    /// Item name/kind for the low-stock alert copy + serialized gating.
    pub item_name: String,
    pub item_kind: String,
    /// For a serialized RECEIVE: the serial number to track (upserted;
    /// the resulting serial id stamps the movement). Consumables carry
    /// none — serialized gating is enforced inside the engine.
    pub serial_number: Option<String>,
}

/// The deterministic consequence the read side + event consumers observe.
pub struct LedgerOutcome {
    pub movement: StockMovement,
    /// Set when a decrement landed the level at/below its reorder point
    /// AND this is the transition INTO low (the notified flag gates
    /// repeats; it clears again when the level rises above the point).
    pub low: Option<LowInfo>,
}

#[derive(Clone)]
pub struct LowInfo {
    pub stock_item_id: Uuid,
    pub stock_location_id: Uuid,
    pub item_name: String,
    pub qty_milli: i64,
    pub reorder_point_milli: i64,
}

/// Appends ONE ledger row and applies its deterministic consequence on
/// `stock_levels` + `stock_serials` in the caller's transaction. Signed
/// milli math only; insufficient stock is a typed conflict. Everything
/// runs inside the same commit as the ledger append (instant-first: one
/// write, no secondary reconciliation pass).
pub async fn record_movement(
    executor: &mut sqlx::PgConnection,
    input: &LedgerInput,
) -> DbResult<LedgerOutcome> {
    if input.item_kind == "serialized" {
        if input.movement.kind == "adjust" {
            return Err(DbError::Conflict(
                "adjust is not a serialized movement; move the unit".into(),
            ));
        }
        let mut m = input.movement.clone();
        if input.movement.kind == "receive" {
            let number = input
                .serial_number
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| {
                    DbError::Conflict("a serialized receive must carry a serial".into())
                })?;
            let serial = upsert_serial(
                executor,
                &StockSerial {
                    id: Uuid::now_v7(),
                    org_id: m.org_id,
                    stock_item_id: m.stock_item_id,
                    serial_number: number.to_string(),
                    location_id: m.to_location_id,
                    status: "in_stock".into(),
                    ticket_id: None,
                    client_id: None,
                    created_by: m.created_by,
                    created_at: m.created_at,
                    updated_at: m.created_at,
                },
            )
            .await?;
            m.serial_id = Some(serial.id);
            m.qty_milli = 1000;
        } else if m.serial_id.is_none() {
            return Err(DbError::Conflict(
                "a serialized OUT movement must carry a serial_id".into(),
            ));
        }
        return record_serialized(executor, input, m).await;
    }
    if input.movement.serial_id.is_some() {
        return Err(DbError::Conflict(
            "a consumable movement must not carry a serial_id".into(),
        ));
    }

    run_ledger_step(executor, input, input.movement.clone()).await
}

/// The serialized-path body: validated movement now serial-stamped.
async fn record_serialized(
    executor: &mut sqlx::PgConnection,
    input: &LedgerInput,
    movement: StockMovement,
) -> DbResult<LedgerOutcome> {
    run_ledger_step(executor, input, movement).await
}

async fn run_ledger_step(
    executor: &mut sqlx::PgConnection,
    input: &LedgerInput,
    movement: StockMovement,
) -> DbResult<LedgerOutcome> {
    let m = &movement;
    m.validate().map_err(|e| DbError::Conflict(e.to_string()))?;

    // 1. THE append. No UPDATE, no DELETE (trigger-guarded at the DB);
    // the traceability record is written exactly once.
    sqlx::query!(
        r#"INSERT INTO stock_movements
             (id, org_id, stock_item_id, kind, from_location_id, to_location_id, qty_milli,
              cost_minor, currency, fx_rate_used, fx_rate_date, fx_source, base_cost_minor,
              serial_id, client_id, ticket_id, carrier, tracking_ref, note, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                   $15, $16, $17, $18, $19, $20)"#,
        m.id,
        m.org_id,
        m.stock_item_id,
        m.kind,
        m.from_location_id,
        m.to_location_id,
        m.qty_milli,
        m.cost_minor,
        m.currency,
        m.fx_rate_used,
        m.fx_rate_date,
        m.fx_source,
        m.base_cost_minor,
        m.serial_id,
        m.client_id,
        m.ticket_id,
        m.carrier,
        m.tracking_ref,
        m.note,
        m.created_by,
    )
    .execute(&mut *executor)
    .await?;

    // 2. INTO side: insert-or-increment the destination level row.
    if let Some(to_loc) = m.to_location_id {
        sqlx::query!(
            r#"INSERT INTO stock_levels (id, org_id, stock_item_id, stock_location_id, qty_milli)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (org_id, stock_item_id, stock_location_id) DO UPDATE
                 SET qty_milli = stock_levels.qty_milli + $5, updated_at = now()"#,
            Uuid::now_v7(),
            m.org_id,
            m.stock_item_id,
            to_loc,
            m.qty_milli.max(0),
        )
        .execute(&mut *executor)
        .await?;
    }

    // 3. OUT side: decrement from an existing level; never negative.
    let mut low: Option<LowInfo> = None;
    if let Some(from_loc) = m.from_location_id {
        let out = m.qty_milli.min(0);
        let row = sqlx::query_as!(
            LevelRow,
            r#"UPDATE stock_levels
                 SET qty_milli = qty_milli + $4, updated_at = now()
               WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3
                 AND qty_milli + $4 >= 0
               RETURNING id, org_id, stock_item_id, stock_location_id, qty_milli,
                         min_qty_milli, max_qty_milli, reorder_point_milli,
                         low_stock_notified_at, created_at, updated_at"#,
            m.org_id,
            m.stock_item_id,
            from_loc,
            out,
        )
        .fetch_optional(&mut *executor)
        .await?
        .ok_or_else(|| DbError::Conflict(format!("insufficient stock for movement {}", m.id)))?;

        if let Some(point) = row.reorder_point_milli {
            // Low detection: the OUT leg dipped the level to/below the
            // point. Notify-flag gates repeats while the level stays low.
            if row.qty_milli <= point && row.low_stock_notified_at.is_none() {
                low = Some(LowInfo {
                    stock_item_id: row.stock_item_id,
                    stock_location_id: row.stock_location_id,
                    item_name: input.item_name.clone(),
                    qty_milli: row.qty_milli,
                    reorder_point_milli: point,
                });
                sqlx::query!(
                    r#"UPDATE stock_levels SET low_stock_notified_at = now()
                        WHERE id = $1"#,
                    row.id,
                )
                .execute(&mut *executor)
                .await?;
            }
        }
    }

    // 4. Recovery clear: an INTO leg above the point re-arms the bell, so
    // a later dip notifies again instead of never re-floating.
    if let Some(to_loc) = m.to_location_id {
        sqlx::query!(
            r#"UPDATE stock_levels SET low_stock_notified_at = NULL
                WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3
                  AND qty_milli > reorder_point_milli"#,
            m.org_id,
            m.stock_item_id,
            to_loc,
        )
        .execute(&mut *executor)
        .await?;
    }

    // 5. Receipt cost roll-up: refresh the item's last locked unit cost.
    if m.kind == "receive" && m.cost_minor.is_some() {
        let cost = m.cost_minor.unwrap_or(0);
        sqlx::query!(
            r#"UPDATE stock_items SET last_unit_cost_minor = $3, updated_at = now()
                WHERE org_id = $1 AND id = $2"#,
            m.org_id,
            m.stock_item_id,
            cost,
        )
        .execute(&mut *executor)
        .await?;
    }

    // 6. Serialized: the serial row is the WHERE-IT-IS-NOW consequence.
    if input.item_kind == "serialized" {
        apply_serial_tick(executor, m).await?;
    }

    Ok(LedgerOutcome {
        movement: m.clone(),
        low,
    })
}

/// Applies the serialized WHERE-IT-IS-NOW tick for a movement: the serial
/// row records both movement ids (movement_in at receive/transfer-in,
/// movement_out at OUT legs) so a later checkout/loan row can cite a
/// single serial + its pair of ledger rows. Receive legs stamp on the
/// upserted serial inside `record_movement` itself.
async fn apply_serial_tick(executor: &mut sqlx::PgConnection, m: &StockMovement) -> DbResult<()> {
    let Some(serial_id) = m.serial_id else {
        return Err(DbError::Conflict(
            "a serialized item movement must carry a serial_id".into(),
        ));
    };
    if m.to_location_id.is_some() {
        let to_loc = m.to_location_id;
        sqlx::query!(
            r#"UPDATE stock_serials
                 SET location_id = $3, status = 'in_stock',
                     movement_in_id = $4, updated_at = now()
               WHERE org_id = $1 AND id = $2 AND status = 'in_stock'"#,
            m.org_id,
            serial_id,
            to_loc,
            m.id,
        )
        .execute(&mut *executor)
        .await?;
    } else {
        sqlx::query!(
            r#"UPDATE stock_serials
                 SET status = 'out', location_id = NULL,
                     movement_out_id = $3, updated_at = now()
               WHERE org_id = $1 AND id = $2 AND status = 'in_stock'"#,
            m.org_id,
            serial_id,
            m.id,
        )
        .execute(&mut *executor)
        .await?;
    }
    Ok(())
}

// Ledger reads ----------------------------------------------------------------

/// Traceability query: the ledger rows for one item, newest first.
pub async fn list_movements(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    limit: i64,
) -> DbResult<Vec<StockMovement>> {
    if item_id == Uuid::nil() {
        movement_rows(executor, org_id, None, None, limit).await
    } else {
        movement_rows(executor, org_id, Some(item_id), None, limit).await
    }
}

pub async fn list_movements_for_ticket(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket_id: Uuid,
    limit: i64,
) -> DbResult<Vec<StockMovement>> {
    movement_rows(executor, org_id, None, Some(ticket_id), limit).await
}

struct MovementRow {
    id: Uuid,
    org_id: Uuid,
    stock_item_id: Uuid,
    kind: String,
    from_location_id: Option<Uuid>,
    to_location_id: Option<Uuid>,
    qty_milli: i64,
    cost_minor: Option<i64>,
    currency: Option<String>,
    fx_rate_used: Option<String>,
    fx_rate_date: Option<chrono::NaiveDate>,
    fx_source: Option<String>,
    base_cost_minor: Option<i64>,
    serial_id: Option<Uuid>,
    client_id: Option<Uuid>,
    ticket_id: Option<Uuid>,
    carrier: Option<String>,
    tracking_ref: Option<String>,
    note: Option<String>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

fn mv(r: MovementRow) -> StockMovement {
    StockMovement {
        id: r.id,
        org_id: r.org_id,
        stock_item_id: r.stock_item_id,
        kind: r.kind,
        from_location_id: r.from_location_id,
        to_location_id: r.to_location_id,
        qty_milli: r.qty_milli,
        cost_minor: r.cost_minor,
        currency: r.currency,
        fx_rate_used: r.fx_rate_used,
        fx_rate_date: r.fx_rate_date,
        fx_source: r.fx_source,
        base_cost_minor: r.base_cost_minor,
        serial_id: r.serial_id,
        client_id: r.client_id,
        ticket_id: r.ticket_id,
        carrier: r.carrier,
        tracking_ref: r.tracking_ref,
        note: r.note,
        created_by: r.created_by,
        created_at: r.created_at,
    }
}

pub async fn movement_rows(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Option<Uuid>,
    ticket_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<StockMovement>> {
    let rows = sqlx::query_as!(
        MovementRow,
        r#"SELECT id, org_id, stock_item_id, kind, from_location_id, to_location_id, qty_milli,
                  cost_minor, currency, fx_rate_used, fx_rate_date, fx_source, base_cost_minor,
                  serial_id, client_id, ticket_id, carrier, tracking_ref, note, created_by, created_at
           FROM stock_movements
           WHERE org_id = $1
             AND ($2::uuid IS NULL OR stock_item_id = $2)
             AND ($3::uuid IS NULL OR ticket_id = $3)
           ORDER BY created_at DESC
           LIMIT $4"#,
        org_id,
        item_id,
        ticket_id,
        limit,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(mv).collect())
}

// Consumption-velocity read model ----------------------------------------------

/// One row of the consumption-velocity read model: the ledger summed over
/// the requested lookback window per item.
#[derive(Debug, serde::Serialize)]
pub struct VelocityRow {
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub unit: String,
    /// Outbound milli over the window (consumed + shipped out).
    pub consumed_milli: i64,
    /// Average daily outbound rate in milli (window / full days).
    pub avg_daily_milli: i64,
}

/// The consumption-velocity read model (docs/instant-first.md: computed
/// by the core from its own ledger, exposed as a NAMED read model through
/// the public query API — never invented by a client-side aggregate).
/// `window_days >= 1`; outbound = consume + ship_out legs.
pub async fn consumption_velocity(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    window_days: i32,
    limit: i64,
) -> DbResult<Vec<VelocityRow>> {
    let rows = sqlx::query_as!(
        VelocityRow,
        r#"SELECT i.id AS "stock_item_id!", i.name AS "item_name!", i.unit AS "unit!",
                  COALESCE(out.qty_milli, 0) AS "consumed_milli!",
                  CASE WHEN $3::int <= 0 THEN 0
                       ELSE COALESCE(out.qty_milli, 0) / GREATEST($3::bigint, 1)::bigint END
                     AS "avg_daily_milli!: i64"
           FROM stock_items i
           LEFT JOIN (
             SELECT stock_item_id, SUM(-qty_milli)::bigint AS qty_milli
             FROM stock_movements
             WHERE org_id = $1 AND kind IN ('consume', 'ship_out')
               AND created_at >= now() - make_interval(days => GREATEST($3::int, 1))
             GROUP BY stock_item_id
           ) out ON out.stock_item_id = i.id
           WHERE i.org_id = $1
           ORDER BY COALESCE(out.qty_milli, 0) DESC, i.name
           LIMIT $2"#,
        org_id,
        limit,
        window_days,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows)
}
// Low-stock notification fan-out ----------------------------------------------

/// Recipients of a `stock_low` notification: the org's owner + admin +
/// bookkeeper (purchasing decision makers) plus the location's assigned
/// tech when the location carries one (van/truck). Empty never happens
/// (owner exists); the route inserts per-user rows in the same commit.
pub async fn low_stock_recipients(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    location_id: Uuid,
) -> DbResult<Vec<Uuid>> {
    let rows = sqlx::query!(
        r#"SELECT u.id FROM users u
           WHERE u.org_id = $1 AND u.is_active
             AND (u.id = $2 OR EXISTS (
                SELECT 1 FROM user_roles ur
                JOIN roles r ON r.id = ur.role_id
                WHERE ur.user_id = u.id AND r.builtin IN ('owner','admin','bookkeeper')))
           LIMIT 50"#,
        org_id,
        location_id,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

// Serials --------------------------------------------------------------------

struct SerialRow {
    id: Uuid,
    org_id: Uuid,
    stock_item_id: Uuid,
    serial_number: String,
    location_id: Option<Uuid>,
    status: String,
    ticket_id: Option<Uuid>,
    client_id: Option<Uuid>,
    created_by: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

fn serial_row(r: SerialRow) -> StockSerial {
    StockSerial {
        id: r.id,
        org_id: r.org_id,
        stock_item_id: r.stock_item_id,
        serial_number: r.serial_number,
        location_id: r.location_id,
        status: r.status,
        ticket_id: r.ticket_id,
        client_id: r.client_id,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

/// Upsert one serialized unit at receive (uniqueness is org+item scope,
/// case-insensitive); the serial re-arms to `in_stock` at its location.
pub async fn upsert_serial(
    executor: &mut sqlx::PgConnection,
    serial: &StockSerial,
) -> DbResult<StockSerial> {
    let row = sqlx::query_as!(
        SerialRow,
        r#"INSERT INTO stock_serials (id, org_id, stock_item_id, serial_number,
                                      location_id, status, created_by)
           VALUES ($1, $2, $3, $4, $5, 'in_stock', $6)
           ON CONFLICT (org_id, stock_item_id, lower(serial_number)) DO UPDATE
             SET location_id = $5, status = 'in_stock', updated_at = now()
           RETURNING id, org_id, stock_item_id, serial_number, location_id, status,
                     ticket_id, client_id, created_by, created_at, updated_at"#,
        serial.id,
        serial.org_id,
        serial.stock_item_id,
        serial.serial_number.trim(),
        serial.location_id,
        serial.created_by,
    )
    .fetch_one(&mut *executor)
    .await?;
    Ok(serial_row(row))
}

pub async fn get_serial(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<StockSerial> {
    let row = sqlx::query_as!(
        SerialRow,
        r#"SELECT id, org_id, stock_item_id, serial_number, location_id, status,
                  ticket_id, client_id, created_by, created_at, updated_at
           FROM stock_serials WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(serial_row(row))
}

/// List serials for one item (optionally location + status filtered).
pub async fn list_serials(
    executor: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    location_id: Option<Uuid>,
    status: Option<&str>,
    limit: i64,
) -> DbResult<Vec<StockSerial>> {
    let rows = sqlx::query_as!(
        SerialRow,
        r#"SELECT id, org_id, stock_item_id, serial_number, location_id, status,
                  ticket_id, client_id, created_by, created_at, updated_at
           FROM stock_serials
           WHERE org_id = $1 AND stock_item_id = $2
             AND ($3::uuid IS NULL OR location_id = $3)
             AND ($4::text IS NULL OR status = $4)
           ORDER BY created_at DESC
           LIMIT $5"#,
        org_id,
        item_id,
        location_id,
        status,
        limit,
    )
    .fetch_all(&mut *executor)
    .await?;
    Ok(rows.into_iter().map(serial_row).collect())
}
