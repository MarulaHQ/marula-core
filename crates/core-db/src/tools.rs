//! Tools + toolboxes persistence (Wave 32 lane 3, `p32-tools-loans`;
//! migration 0104).
//!
//! Tools are lane-1 stock items flagged `is_tool` (serialized, one physical
//! unit per `stock_serials` row). A toolbox is a kit of member tools,
//! checked out AS ONE transaction while each physical unit still rides its
//! own leg on the append-only `stock_movements` ledger (`checkout` out,
//! `checkin` back). The single-holder invariant — one tool, one person at
//! a time — is the partial unique index on open checkout lines, mirrored
//! by the serial status gate (`in_stock` → `checked_out` → `in_stock`).
//!
//! Overdue is DERIVED at read time (`returned_at IS NULL AND due_at <
//! now()`); nothing stores a stale flag. Every read/write carries an
//! explicit `org_id` predicate on top of the migration's RLS policies.

use crate::stock::{LedgerInput, MovementLeg};
use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::{
    is_valid_return_condition, StockMovement, ToolCheckout, ToolCheckoutLine, ToolKit,
    ToolKitMember,
};
use uuid::Uuid;

// Row shapes -----------------------------------------------------------------

struct KitRow {
    id: Uuid,
    org_id: Uuid,
    name: String,
    code: Option<String>,
    note: Option<String>,
    active: bool,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn kit_row(r: KitRow) -> ToolKit {
    ToolKit {
        id: r.id,
        org_id: r.org_id,
        name: r.name,
        code: r.code,
        note: r.note,
        active: r.active,
        created_by: r.created_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

struct CheckoutRow {
    id: Uuid,
    org_id: Uuid,
    holder_user_id: Uuid,
    kit_id: Option<Uuid>,
    from_location_id: Uuid,
    due_at: DateTime<Utc>,
    notes: Option<String>,
    status: String,
    returned_at: Option<DateTime<Utc>>,
    return_notes: Option<String>,
    checked_out_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn checkout_row(r: CheckoutRow) -> ToolCheckout {
    ToolCheckout {
        id: r.id,
        org_id: r.org_id,
        holder_user_id: r.holder_user_id,
        kit_id: r.kit_id,
        from_location_id: r.from_location_id,
        due_at: r.due_at,
        notes: r.notes,
        status: r.status,
        returned_at: r.returned_at,
        return_notes: r.return_notes,
        checked_out_by: r.checked_out_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

struct LineRow {
    id: Uuid,
    org_id: Uuid,
    checkout_id: Uuid,
    stock_item_id: Uuid,
    serial_id: Uuid,
    movement_out_id: Uuid,
    movement_in_id: Option<Uuid>,
    returned_at: Option<DateTime<Utc>>,
    return_condition: Option<String>,
    return_notes: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn line_row(r: LineRow) -> ToolCheckoutLine {
    ToolCheckoutLine {
        id: r.id,
        org_id: r.org_id,
        checkout_id: r.checkout_id,
        stock_item_id: r.stock_item_id,
        serial_id: r.serial_id,
        movement_out_id: r.movement_out_id,
        movement_in_id: r.movement_in_id,
        returned_at: r.returned_at,
        return_condition: r.return_condition,
        return_notes: r.return_notes,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// Kits -------------------------------------------------------------------------

pub async fn insert_kit(conn: &mut sqlx::PgConnection, kit: &ToolKit) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO tool_kits (id, org_id, name, code, note, active, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        kit.id,
        kit.org_id,
        kit.name.trim(),
        kit.code.as_deref(),
        kit.note.as_deref(),
        kit.active,
        kit.created_by,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn get_kit(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<ToolKit> {
    let row = sqlx::query_as!(
        KitRow,
        r#"SELECT id, org_id, name, code, note, active, created_by, created_at, updated_at
           FROM tool_kits WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(kit_row(row))
}

pub async fn list_kits(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    only_active: bool,
    limit: i64,
) -> DbResult<Vec<ToolKit>> {
    let rows = sqlx::query_as!(
        KitRow,
        r#"SELECT id, org_id, name, code, note, active, created_by, created_at, updated_at
           FROM tool_kits
           WHERE org_id = $1 AND (NOT $2::boolean OR active)
           ORDER BY name
           LIMIT $3"#,
        org_id,
        only_active,
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(kit_row).collect())
}

pub async fn update_kit(conn: &mut sqlx::PgConnection, kit: &ToolKit) -> DbResult<()> {
    let n = sqlx::query!(
        r#"UPDATE tool_kits
             SET name = $3, code = $4, note = $5, active = $6, updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        kit.org_id,
        kit.id,
        kit.name.trim(),
        kit.code.as_deref(),
        kit.note.as_deref(),
        kit.active,
    )
    .execute(&mut *conn)
    .await?;
    if n.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn add_kit_member(conn: &mut sqlx::PgConnection, member: &ToolKitMember) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO tool_kit_members (id, org_id, kit_id, stock_item_id, qty_milli)
           VALUES ($1, $2, $3, $4, $5)"#,
        member.id,
        member.org_id,
        member.kit_id,
        member.stock_item_id,
        member.qty_milli,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Removes a member from a kit. The returned row's item id lets the caller
/// audit what left the kit.
pub async fn remove_kit_member(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    kit_id: Uuid,
    member_id: Uuid,
) -> DbResult<Uuid> {
    let row = sqlx::query!(
        r#"DELETE FROM tool_kit_members
           WHERE org_id = $1 AND kit_id = $2 AND id = $3
           RETURNING stock_item_id"#,
        org_id,
        kit_id,
        member_id,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row.stock_item_id)
}

/// Kit members with their item identity, for the kit detail read model.
pub struct KitMemberView {
    pub member: ToolKitMember,
    pub item_name: String,
    pub item_sku: Option<String>,
    pub item_is_tool: bool,
    pub item_is_serialized: bool,
}

pub async fn list_kit_members(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    kit_id: Uuid,
) -> DbResult<Vec<KitMemberView>> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.org_id, m.kit_id, m.stock_item_id, m.qty_milli, m.created_at,
                  si.name AS item_name, si.sku AS item_sku, si.is_tool, si.kind
           FROM tool_kit_members m
           JOIN stock_items si ON si.id = m.stock_item_id AND si.org_id = m.org_id
           WHERE m.org_id = $1 AND m.kit_id = $2
           ORDER BY si.name"#,
        org_id,
        kit_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| KitMemberView {
            member: ToolKitMember {
                id: r.id,
                org_id: r.org_id,
                kit_id: r.kit_id,
                stock_item_id: r.stock_item_id,
                qty_milli: r.qty_milli,
                created_at: r.created_at,
            },
            item_name: r.item_name,
            item_sku: r.item_sku,
            item_is_tool: r.is_tool,
            item_is_serialized: r.kind == "serialized",
        })
        .collect())
}

/// Per-member availability of a kit at a location (`None` = org-wide):
/// required whole units vs in-stock serials. A kit is ready for checkout
/// when every member has enough available units.
pub struct KitAvailabilityRow {
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub required_units: i64,
    pub available_units: i64,
}

impl KitAvailabilityRow {
    pub fn is_ready(&self) -> bool {
        self.available_units >= self.required_units
    }
}

pub async fn kit_availability(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    kit_id: Uuid,
    location_id: Option<Uuid>,
) -> DbResult<Vec<KitAvailabilityRow>> {
    let rows = sqlx::query!(
        r#"SELECT m.stock_item_id, si.name AS item_name, m.qty_milli,
                  count(ss.id) FILTER (
                    WHERE ss.status = 'in_stock'
                      AND ($3::uuid IS NULL OR ss.location_id = $3)
                  ) AS "available_units!"
           FROM tool_kit_members m
           JOIN stock_items si ON si.id = m.stock_item_id AND si.org_id = m.org_id
           LEFT JOIN stock_serials ss
             ON ss.stock_item_id = m.stock_item_id AND ss.org_id = m.org_id
           WHERE m.org_id = $1 AND m.kit_id = $2
           GROUP BY m.stock_item_id, si.name, m.qty_milli
           ORDER BY si.name"#,
        org_id,
        kit_id,
        location_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| KitAvailabilityRow {
            stock_item_id: r.stock_item_id,
            item_name: r.item_name,
            required_units: r.qty_milli / 1000,
            available_units: r.available_units,
        })
        .collect())
}

// Checkout engine ---------------------------------------------------------------

/// One requested physical unit (explicit serial, or `None` = auto-pick the
/// oldest in-stock unit at the checkout location).
pub struct CheckoutTarget {
    pub stock_item_id: Uuid,
    pub serial_id: Option<Uuid>,
}

pub struct CheckoutRequest {
    pub holder_user_id: Uuid,
    /// Set when the checkout is a whole kit (toolbox checked out as one);
    /// `items` must then be empty.
    pub kit_id: Option<Uuid>,
    pub from_location_id: Uuid,
    pub due_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub items: Vec<CheckoutTarget>,
}

pub struct CheckoutOutcome {
    pub checkout: ToolCheckout,
    pub lines: Vec<ToolCheckoutLine>,
    /// The ledger legs appended, in order (one per physical unit).
    pub legs: Vec<MovementLeg>,
}

fn require_tool(item: &core_domain::entities::StockItem) -> DbResult<()> {
    if !item.is_tool {
        return Err(DbError::Conflict(format!(
            "stock item {} is not a tool; only tool-flagged serialized items check out",
            item.name
        )));
    }
    if item.kind != "serialized" {
        return Err(DbError::Conflict(format!(
            "tool {} is not serialized; tools are tracked per physical unit",
            item.name
        )));
    }
    if !item.active {
        return Err(DbError::Conflict(format!("tool {} is archived", item.name)));
    }
    Ok(())
}

/// Picks up to `need` in-stock serials of an item at a location (FIFO by
/// creation so repeated checkouts are deterministic).
async fn available_serials_at(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    location_id: Uuid,
    need: i64,
) -> DbResult<Vec<(Uuid, String)>> {
    let rows = sqlx::query!(
        r#"SELECT id, serial_number FROM stock_serials
           WHERE org_id = $1 AND stock_item_id = $2
             AND status = 'in_stock' AND location_id = $3
           ORDER BY created_at, id
           LIMIT $4"#,
        org_id,
        item_id,
        location_id,
        need,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.serial_number)).collect())
}

/// One-checkout-one-holder transaction: resolves the physical units,
/// appends one `checkout` ledger leg per unit (flipping each serial to
/// `checked_out`), and records the lines that cite those legs. The
/// single-holder invariant is enforced by the pre-checks here AND by the
/// partial unique index on open lines (the mechanical guarantee).
pub async fn create_checkout(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    req: &CheckoutRequest,
) -> DbResult<CheckoutOutcome> {
    let now = Utc::now();
    crate::stock::get_stock_location(&mut *conn, org_id, req.from_location_id).await?;

    // 1. Resolve the plan (item + serial per physical unit) before any write.
    struct Unit {
        item_id: Uuid,
        item_name: String,
        serial_id: Uuid,
        serial_number: String,
    }
    let mut plan: Vec<Unit> = Vec::new();

    if let Some(kit_id) = req.kit_id {
        if !req.items.is_empty() {
            return Err(DbError::Conflict(
                "a kit checkout takes no separate items".into(),
            ));
        }
        let kit = get_kit(&mut *conn, org_id, kit_id).await?;
        if !kit.active {
            return Err(DbError::Conflict(format!("kit {} is inactive", kit.name)));
        }
        let members = list_kit_members(&mut *conn, org_id, kit_id).await?;
        if members.is_empty() {
            return Err(DbError::Conflict(format!(
                "kit {} has no member tools to check out",
                kit.name
            )));
        }
        for m in members {
            let item =
                crate::stock::get_stock_item(&mut *conn, org_id, m.member.stock_item_id).await?;
            require_tool(&item)?;
            let need = m.member.qty_milli / 1000;
            let serials = available_serials_at(
                &mut *conn,
                org_id,
                m.member.stock_item_id,
                req.from_location_id,
                need,
            )
            .await?;
            if (serials.len() as i64) < need {
                return Err(DbError::Conflict(format!(
                    "kit member {} has {}/{} units available at this location",
                    item.name,
                    serials.len(),
                    need
                )));
            }
            for (serial_id, serial_number) in serials {
                plan.push(Unit {
                    item_id: item.id,
                    item_name: item.name.clone(),
                    serial_id,
                    serial_number,
                });
            }
        }
    } else {
        if req.items.is_empty() {
            return Err(DbError::Conflict(
                "a checkout needs a kit or at least one item".into(),
            ));
        }
        for target in &req.items {
            let item =
                crate::stock::get_stock_item(&mut *conn, org_id, target.stock_item_id).await?;
            require_tool(&item)?;
            let (serial_id, serial_number) = match target.serial_id {
                Some(serial_id) => {
                    let serial = crate::stock::get_serial(&mut *conn, org_id, serial_id).await?;
                    if serial.stock_item_id != item.id {
                        return Err(DbError::Conflict(format!(
                            "serial {} does not belong to {}",
                            serial.serial_number, item.name
                        )));
                    }
                    if serial.status != "in_stock" {
                        return Err(DbError::Conflict(format!(
                            "serial {} is not in stock (status {})",
                            serial.serial_number, serial.status
                        )));
                    }
                    if serial.location_id != Some(req.from_location_id) {
                        return Err(DbError::Conflict(format!(
                            "serial {} is not at the checkout location",
                            serial.serial_number
                        )));
                    }
                    (serial.id, serial.serial_number)
                }
                None => {
                    let picked =
                        available_serials_at(&mut *conn, org_id, item.id, req.from_location_id, 1)
                            .await?;
                    picked.into_iter().next().ok_or_else(|| {
                        DbError::Conflict(format!(
                            "no in-stock unit of {} at this location",
                            item.name
                        ))
                    })?
                }
            };
            if plan.iter().any(|u| u.serial_id == serial_id) {
                return Err(DbError::Conflict(format!(
                    "unit {} appears twice in the same checkout",
                    serial_number
                )));
            }
            plan.push(Unit {
                item_id: item.id,
                item_name: item.name.clone(),
                serial_id,
                serial_number,
            });
        }
    }

    // 2. Header row (lines FK to it).
    let checkout = ToolCheckout {
        id: Uuid::now_v7(),
        org_id,
        holder_user_id: req.holder_user_id,
        kit_id: req.kit_id,
        from_location_id: req.from_location_id,
        due_at: req.due_at,
        notes: req.notes.clone(),
        status: "open".into(),
        returned_at: None,
        return_notes: None,
        checked_out_by: Some(user_id),
        created_at: now,
        updated_at: now,
    };
    checkout
        .validate()
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    insert_checkout(conn, &checkout).await?;

    // 3. One ledger leg per physical unit, then the line citing it.
    let mut lines = Vec::with_capacity(plan.len());
    let mut legs = Vec::with_capacity(plan.len());
    for unit in plan {
        let movement = StockMovement {
            id: Uuid::now_v7(),
            org_id,
            stock_item_id: unit.item_id,
            kind: "checkout".into(),
            from_location_id: Some(req.from_location_id),
            to_location_id: None,
            qty_milli: -1000,
            cost_minor: None,
            currency: None,
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_cost_minor: None,
            serial_id: Some(unit.serial_id),
            client_id: None,
            ticket_id: None,
            carrier: None,
            tracking_ref: None,
            note: None,
            created_by: Some(user_id),
            created_at: now,
        };
        let outcome = crate::stock::record_movement(
            conn,
            &LedgerInput {
                movement,
                item_name: unit.item_name.clone(),
                item_kind: "serialized".into(),
                serial_number: None,
            },
        )
        .await?;
        // The ledger tick flipped the serial to `out`; the checkout is the
        // richer WHERE-IT-IS-NOW state.
        let n = sqlx::query!(
            r#"UPDATE stock_serials
                 SET status = 'checked_out', updated_at = now()
               WHERE org_id = $1 AND id = $2 AND status = 'out'"#,
            org_id,
            unit.serial_id,
        )
        .execute(&mut *conn)
        .await?;
        if n.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "unit {} could not be checked out (already out)",
                unit.serial_number
            )));
        }
        let line = insert_line(
            conn,
            org_id,
            checkout.id,
            unit.item_id,
            unit.serial_id,
            outcome.movement.id,
            now,
        )
        .await
        .map_err(|e| match e {
            // The mechanical single-holder guarantee surfaces as a unique
            // violation on the open-line index under concurrency.
            DbError::Sql(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                DbError::Conflict(format!(
                    "unit {} is already out under another open checkout",
                    unit.serial_number
                ))
            }
            other => other,
        })?;
        legs.push(MovementLeg {
            movement: outcome.movement,
            low: outcome.low,
        });
        lines.push(line);
    }

    Ok(CheckoutOutcome {
        checkout,
        lines,
        legs,
    })
}

async fn insert_checkout(conn: &mut sqlx::PgConnection, c: &ToolCheckout) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO tool_checkouts
             (id, org_id, holder_user_id, kit_id, from_location_id, due_at, notes, status,
              returned_at, return_notes, checked_out_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        c.id,
        c.org_id,
        c.holder_user_id,
        c.kit_id,
        c.from_location_id,
        c.due_at,
        c.notes.as_deref(),
        c.status,
        c.returned_at,
        c.return_notes.as_deref(),
        c.checked_out_by,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn insert_line(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    checkout_id: Uuid,
    item_id: Uuid,
    serial_id: Uuid,
    movement_out_id: Uuid,
    now: DateTime<Utc>,
) -> DbResult<ToolCheckoutLine> {
    let row = sqlx::query_as!(
        LineRow,
        r#"INSERT INTO tool_checkout_lines
             (id, org_id, checkout_id, stock_item_id, serial_id, movement_out_id)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING id, org_id, checkout_id, stock_item_id, serial_id, movement_out_id,
                     movement_in_id, returned_at, return_condition, return_notes,
                     created_at, updated_at"#,
        Uuid::now_v7(),
        org_id,
        checkout_id,
        item_id,
        serial_id,
        movement_out_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    let mut line = line_row(row);
    line.created_at = now;
    Ok(line)
}

// Check-in engine ---------------------------------------------------------------

/// One line to return; absent fields mean "no condition recorded".
#[derive(Clone)]
pub struct CheckinTarget {
    pub line_id: Uuid,
    pub return_condition: Option<String>,
    pub return_notes: Option<String>,
}

pub struct CheckinOutcome {
    pub checkout: ToolCheckout,
    pub lines: Vec<ToolCheckoutLine>,
    /// The ledger legs appended, in order (one per returned unit).
    pub legs: Vec<MovementLeg>,
}

pub async fn get_checkout(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<(ToolCheckout, Vec<ToolCheckoutLine>)> {
    let row = sqlx::query_as!(
        CheckoutRow,
        r#"SELECT id, org_id, holder_user_id, kit_id, from_location_id, due_at, notes, status,
                  returned_at, return_notes, checked_out_by, created_at, updated_at
           FROM tool_checkouts WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    let lines = list_lines(conn, org_id, id).await?;
    Ok((checkout_row(row), lines))
}

pub async fn list_lines(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    checkout_id: Uuid,
) -> DbResult<Vec<ToolCheckoutLine>> {
    let rows = sqlx::query_as!(
        LineRow,
        r#"SELECT id, org_id, checkout_id, stock_item_id, serial_id, movement_out_id,
                  movement_in_id, returned_at, return_condition, return_notes,
                  created_at, updated_at
           FROM tool_checkout_lines
           WHERE org_id = $1 AND checkout_id = $2
           ORDER BY created_at, id"#,
        org_id,
        checkout_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(line_row).collect())
}

/// Checks tools back in. `targets` empty = return every outstanding unit;
/// otherwise the named lines (partial returns keep the checkout open). Each
/// returned unit rides a `checkin` ledger leg and the serial re-arms to
/// `in_stock` at the destination.
#[allow(clippy::too_many_arguments)]
pub async fn checkin(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    checkout_id: Uuid,
    to_location_id: Uuid,
    targets: &[CheckinTarget],
    notes: Option<&str>,
) -> DbResult<CheckinOutcome> {
    let now = Utc::now();
    crate::stock::get_stock_location(&mut *conn, org_id, to_location_id).await?;
    let (checkout, lines) = get_checkout(conn, org_id, checkout_id).await?;
    if checkout.status == "returned" {
        return Err(DbError::Conflict(
            "this checkout is already fully returned".into(),
        ));
    }

    let outstanding: Vec<&ToolCheckoutLine> = lines.iter().filter(|l| !l.is_returned()).collect();
    let selected: Vec<CheckinTarget> = if targets.is_empty() {
        outstanding
            .iter()
            .map(|l| CheckinTarget {
                line_id: l.id,
                return_condition: None,
                return_notes: None,
            })
            .collect()
    } else {
        let mut seen = Vec::new();
        for t in targets {
            if seen.contains(&t.line_id) {
                return Err(DbError::Conflict(
                    "the same line appears twice in one check-in".into(),
                ));
            }
            let line = lines.iter().find(|l| l.id == t.line_id).ok_or_else(|| {
                DbError::Conflict(format!("line {} is not on this checkout", t.line_id))
            })?;
            if line.is_returned() {
                return Err(DbError::Conflict(format!(
                    "line {} was already returned",
                    t.line_id
                )));
            }
            if let Some(condition) = &t.return_condition {
                if !is_valid_return_condition(condition) {
                    return Err(DbError::Conflict(
                        "return condition must be one of: good, fair, damaged".into(),
                    ));
                }
            }
            seen.push(t.line_id);
        }
        targets.to_vec()
    };

    let mut legs = Vec::with_capacity(selected.len());
    for t in &selected {
        let line = outstanding
            .iter()
            .find(|l| l.id == t.line_id)
            .expect("validated above");
        let item = crate::stock::get_stock_item(&mut *conn, org_id, line.stock_item_id).await?;
        let movement = StockMovement {
            id: Uuid::now_v7(),
            org_id,
            stock_item_id: line.stock_item_id,
            kind: "checkin".into(),
            from_location_id: None,
            to_location_id: Some(to_location_id),
            qty_milli: 1000,
            cost_minor: None,
            currency: None,
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_cost_minor: None,
            serial_id: Some(line.serial_id),
            client_id: None,
            ticket_id: None,
            carrier: None,
            tracking_ref: None,
            note: None,
            created_by: Some(user_id),
            created_at: now,
        };
        let outcome = crate::stock::record_movement(
            conn,
            &LedgerInput {
                movement,
                item_name: item.name.clone(),
                item_kind: "serialized".into(),
                serial_number: None,
            },
        )
        .await?;
        // The lane-1 serial tick only arms serials that are `in_stock`;
        // a `checked_out` unit re-arms here, in the same commit.
        let n = sqlx::query!(
            r#"UPDATE stock_serials
                 SET status = 'in_stock', location_id = $3, movement_in_id = $4, updated_at = now()
               WHERE org_id = $1 AND id = $2 AND status = 'checked_out'"#,
            org_id,
            line.serial_id,
            to_location_id,
            outcome.movement.id,
        )
        .execute(&mut *conn)
        .await?;
        if n.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "unit on line {} is not checked out",
                t.line_id
            )));
        }
        let n = sqlx::query!(
            r#"UPDATE tool_checkout_lines
                 SET returned_at = $4, return_condition = $5, return_notes = $6,
                     movement_in_id = $7, updated_at = now()
               WHERE org_id = $1 AND id = $2 AND checkout_id = $3 AND returned_at IS NULL"#,
            org_id,
            t.line_id,
            checkout_id,
            now,
            t.return_condition.as_deref(),
            t.return_notes.as_deref(),
            outcome.movement.id,
        )
        .execute(&mut *conn)
        .await?;
        if n.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "line {} was already returned",
                t.line_id
            )));
        }
        legs.push(MovementLeg {
            movement: outcome.movement,
            low: outcome.low,
        });
    }

    // 4. Header consequence: fully returned closes it; partial keeps it open.
    let remaining: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM tool_checkout_lines
           WHERE org_id = $1 AND checkout_id = $2 AND returned_at IS NULL"#,
        org_id,
        checkout_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    if remaining == 0 {
        sqlx::query!(
            r#"UPDATE tool_checkouts
                 SET status = 'returned', returned_at = $3,
                     return_notes = COALESCE($4, return_notes), updated_at = now()
               WHERE org_id = $1 AND id = $2"#,
            org_id,
            checkout_id,
            now,
            notes,
        )
        .execute(&mut *conn)
        .await?;
    } else if notes.is_some() {
        sqlx::query!(
            r#"UPDATE tool_checkouts
                 SET return_notes = $3, updated_at = now()
               WHERE org_id = $1 AND id = $2"#,
            org_id,
            checkout_id,
            notes,
        )
        .execute(&mut *conn)
        .await?;
    }

    let (checkout, lines) = get_checkout(conn, org_id, checkout_id).await?;
    Ok(CheckinOutcome {
        checkout,
        lines,
        legs,
    })
}

// Reads --------------------------------------------------------------------------

pub struct CheckoutFilter<'a> {
    /// `open` | `returned` (None = both).
    pub status: Option<&'a str>,
    pub holder_user_id: Option<Uuid>,
    pub kit_id: Option<Uuid>,
    pub overdue_only: bool,
    pub limit: i64,
}

pub async fn list_checkouts(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    f: &CheckoutFilter<'_>,
) -> DbResult<Vec<ToolCheckout>> {
    let rows = sqlx::query_as!(
        CheckoutRow,
        r#"SELECT id, org_id, holder_user_id, kit_id, from_location_id, due_at, notes, status,
                  returned_at, return_notes, checked_out_by, created_at, updated_at
           FROM tool_checkouts
           WHERE org_id = $1
             AND ($2::text IS NULL OR status = $2)
             AND ($3::uuid IS NULL OR holder_user_id = $3)
             AND ($4::uuid IS NULL OR kit_id = $4)
             AND (NOT $5::boolean OR (returned_at IS NULL AND due_at < now()))
           ORDER BY created_at DESC
           LIMIT $6"#,
        org_id,
        f.status,
        f.holder_user_id,
        f.kit_id,
        f.overdue_only,
        f.limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(checkout_row).collect())
}

/// Where is it now: one row per serialized unit with its live holder or
/// client. Covers tools AND loaned devices (non-tool serialized items), so
/// a loaned phone is visible in the same read model.
pub struct ToolSerialWhereabouts {
    pub id: Uuid,
    pub org_id: Uuid,
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub is_tool: bool,
    pub serial_number: String,
    pub location_id: Option<Uuid>,
    pub location_name: Option<String>,
    pub status: String,
    pub serial_client_id: Option<Uuid>,
    pub checkout_id: Option<Uuid>,
    pub holder_user_id: Option<Uuid>,
    pub due_at: Option<DateTime<Utc>>,
    pub loan_id: Option<Uuid>,
    pub loan_client_id: Option<Uuid>,
    pub expected_return_at: Option<chrono::NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct WhereaboutsFilter<'a> {
    pub stock_item_id: Option<Uuid>,
    pub location_id: Option<Uuid>,
    /// `in_stock` | `out` | `checked_out` | `loaned`.
    pub status: Option<&'a str>,
    pub holder_user_id: Option<Uuid>,
    /// `Some(true)` = tools only; `Some(false)` = devices only.
    pub is_tool: Option<bool>,
    pub overdue_only: bool,
    pub limit: i64,
}

pub async fn serial_whereabouts(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    f: &WhereaboutsFilter<'_>,
) -> DbResult<Vec<ToolSerialWhereabouts>> {
    let rows = sqlx::query_as!(
        ToolSerialWhereabouts,
        r#"SELECT ss.id, ss.org_id, ss.stock_item_id, si.name AS item_name, si.is_tool,
                  ss.serial_number,
                  ss.location_id AS "location_id?", sl.name::text AS "location_name?",
                  ss.status,
                  ss.client_id AS "serial_client_id?",
                  oc.id AS "checkout_id?", oc.holder_user_id AS "holder_user_id?",
                  oc.due_at AS "due_at?",
                  ll.loan_id AS "loan_id?", cl.client_id AS "loan_client_id?",
                  cl.expected_return_at AS "expected_return_at?",
                  ss.created_at, ss.updated_at
           FROM stock_serials ss
           JOIN stock_items si ON si.id = ss.stock_item_id AND si.org_id = ss.org_id
           LEFT JOIN stock_locations sl ON sl.id = ss.location_id
           LEFT JOIN tool_checkout_lines ocl
             ON ocl.serial_id = ss.id AND ocl.org_id = ss.org_id AND ocl.returned_at IS NULL
           LEFT JOIN tool_checkouts oc ON oc.id = ocl.checkout_id
           LEFT JOIN client_loan_lines ll
             ON ll.serial_id = ss.id AND ll.org_id = ss.org_id AND ll.returned_at IS NULL
           LEFT JOIN client_loans cl ON cl.id = ll.loan_id
           WHERE ss.org_id = $1
             AND ($2::uuid IS NULL OR ss.stock_item_id = $2)
             AND ($3::uuid IS NULL OR ss.location_id = $3)
             AND ($4::text IS NULL OR ss.status = $4)
             AND ($5::uuid IS NULL OR oc.holder_user_id = $5)
             AND ($6::boolean IS NULL OR si.is_tool = $6)
             AND (NOT $7::boolean
                  OR (oc.due_at IS NOT NULL AND oc.due_at < now())
                  OR (cl.expected_return_at IS NOT NULL AND cl.expected_return_at < current_date))
           ORDER BY si.name, ss.serial_number
           LIMIT $8"#,
        org_id,
        f.stock_item_id,
        f.location_id,
        f.status,
        f.holder_user_id,
        f.is_tool,
        f.overdue_only,
        f.limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Availability counts per (tool item, location): the named read model the
/// UI asks "can I check this out here?".
pub struct ToolAvailabilityRow {
    pub stock_item_id: Uuid,
    pub item_name: String,
    pub location_id: Option<Uuid>,
    pub location_name: Option<String>,
    pub available_units: i64,
    pub checked_out_units: i64,
    pub loaned_units: i64,
    pub out_units: i64,
}

pub async fn tool_availability(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    location_id: Option<Uuid>,
    stock_item_id: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<ToolAvailabilityRow>> {
    let rows = sqlx::query!(
        r#"SELECT si.id AS stock_item_id, si.name AS item_name,
                  ss.location_id, sl.name::text AS "location_name?",
                  count(ss.id) FILTER (WHERE ss.status = 'in_stock') AS "available_units!",
                  count(ss.id) FILTER (WHERE ss.status = 'checked_out') AS "checked_out_units!",
                  count(ss.id) FILTER (WHERE ss.status = 'loaned') AS "loaned_units!",
                  count(ss.id) FILTER (WHERE ss.status = 'out') AS "out_units!"
           FROM stock_serials ss
           JOIN stock_items si ON si.id = ss.stock_item_id AND si.org_id = ss.org_id
           LEFT JOIN stock_locations sl ON sl.id = ss.location_id
           WHERE ss.org_id = $1 AND si.is_tool
             AND ($2::uuid IS NULL OR ss.location_id = $2)
             AND ($3::uuid IS NULL OR ss.stock_item_id = $3)
           GROUP BY si.id, si.name, ss.location_id, sl.name
           ORDER BY si.name, sl.name NULLS LAST
           LIMIT $4"#,
        org_id,
        location_id,
        stock_item_id,
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ToolAvailabilityRow {
            stock_item_id: r.stock_item_id,
            item_name: r.item_name,
            location_id: r.location_id,
            location_name: r.location_name,
            available_units: r.available_units,
            checked_out_units: r.checked_out_units,
            loaned_units: r.loaned_units,
            out_units: r.out_units,
        })
        .collect())
}

/// Kits whose short code matches exactly (case-insensitive). Scan resolution
/// keeps the same no-guess rule as every other kind: one hit resolves,
/// several return candidates.
pub struct KitCodeRow {
    pub id: Uuid,
    pub name: String,
    pub code: Option<String>,
}

pub async fn find_kits_by_code(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    code: &str,
    limit: i64,
) -> DbResult<Vec<KitCodeRow>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, code
           FROM tool_kits
           WHERE org_id = $1 AND active AND code IS NOT NULL AND lower(code) = lower($2)
           ORDER BY name
           LIMIT $3"#,
        org_id,
        code,
        limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| KitCodeRow {
            id: r.id,
            name: r.name,
            code: r.code,
        })
        .collect())
}
