//! Client loans + rentals persistence (Wave 32 lane 3, `p32-tools-loans`;
//! migration 0104).
//!
//! A loan issues devices to a CLIENT under an agreement. Stock-sourced
//! devices ride the lane-1 append-only ledger on both legs (`loan` out,
//! `loan_return` back) and, on issue, instantiate a `DeviceAsset` in the
//! client's fleet carrying identity keys ({serial}) — the ADR 0003 dedupe
//! path converges a later M365/RMM feed describing the same phone onto
//! that same row (a serial match is decisive at 0.95, above the shipped
//! auto-adopt threshold; never a twin). Asset-sourced lines place an
//! existing fleet asset under the agreement.
//!
//! When the loan IS a lease, the 0061 rental fields on the asset are
//! stamped from the lease terms (`ownership = leased`, `rental_start` =
//! out date, `rental_end` = expected return, plus cost/currency/cycle) —
//! the exact vocabulary the asset lifecycle API already uses, so no
//! translation layer exists anywhere.
//!
//! Overdue is DERIVED (`returned_at IS NULL AND expected_return_at <
//! current_date`), never a stored flag. Every read/write carries an
//! explicit `org_id` predicate on top of the migration's RLS policies.
//!
//! Query style in the rentals block (migration 0237): fixed SQL uses the
//! checked macros. A runtime `sqlx::query` appears only where a macro cannot
//! express the statement, and the reason is named at the statement:
//!
//!   * a column list assembled from a closed set of constants (the charge and
//!     vendor row shapes), which `query_as!` cannot take as a parameter;
//!   * a statement the branch's one `scripts/sqlx-prepare.sh` pass converts,
//!     because adding its cache entry mid-lane would break every peer's offline
//!     build.
//!
//! Every statement binds `org_id` explicitly and is covered by the table's RLS
//! policy.

use crate::stock::{LedgerInput, MovementLeg};
use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{
    loan_issue_origin_is_valid, ClientLoan, ClientLoanLine, CustodyClass, StockMovement,
    LOAN_OPEN_CLASS,
};
use uuid::Uuid;

// Row shapes -----------------------------------------------------------------

struct LoanRow {
    id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    contract_id: Option<Uuid>,
    agreement_ref: Option<String>,
    out_at: DateTime<Utc>,
    expected_return_at: NaiveDate,
    returned_at: Option<DateTime<Utc>>,
    status: String,
    is_lease: bool,
    lease_cycle: Option<String>,
    lease_cost_minor: Option<i64>,
    lease_currency: Option<String>,
    notes: Option<String>,
    return_notes: Option<String>,
    issued_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn loan_row(r: LoanRow) -> ClientLoan {
    ClientLoan {
        id: r.id,
        org_id: r.org_id,
        client_id: r.client_id,
        contract_id: r.contract_id,
        agreement_ref: r.agreement_ref,
        out_at: r.out_at,
        expected_return_at: r.expected_return_at,
        returned_at: r.returned_at,
        status: r.status,
        is_lease: r.is_lease,
        lease_cycle: r.lease_cycle,
        lease_cost_minor: r.lease_cost_minor,
        lease_currency: r.lease_currency,
        notes: r.notes,
        return_notes: r.return_notes,
        issued_by: r.issued_by,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

struct LineRow {
    id: Uuid,
    org_id: Uuid,
    loan_id: Uuid,
    stock_item_id: Option<Uuid>,
    serial_id: Option<Uuid>,
    device_asset_id: Option<Uuid>,
    movement_out_id: Option<Uuid>,
    movement_in_id: Option<Uuid>,
    returned_at: Option<DateTime<Utc>>,
    return_condition: Option<String>,
    return_notes: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn line_row(r: LineRow) -> ClientLoanLine {
    ClientLoanLine {
        id: r.id,
        org_id: r.org_id,
        loan_id: r.loan_id,
        stock_item_id: r.stock_item_id,
        serial_id: r.serial_id,
        device_asset_id: r.device_asset_id,
        movement_out_id: r.movement_out_id,
        movement_in_id: r.movement_in_id,
        returned_at: r.returned_at,
        return_condition: r.return_condition,
        return_notes: r.return_notes,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

// Issue -------------------------------------------------------------------------

/// One device to issue: from org stock (item + optional serial) or an
/// existing fleet asset.
/// The classes the automatic issue picker may choose from. The same set
/// `core_domain::entities::loan_issue_origin_is_valid` accepts; the explicit
/// list is what lets the picker filter in SQL, so an ineligible unit is never
/// selected only to be refused afterwards.
const ISSUE_ELIGIBLE_CLASSES: [&str; 4] = ["sale_stock", "tools", "internal_fleet", "rental_out"];

#[derive(Clone)]
pub struct LoanIssueItem {
    pub stock_item_id: Option<Uuid>,
    pub serial_id: Option<Uuid>,
    pub device_asset_id: Option<Uuid>,
}

pub struct LoanIssueRequest {
    pub client_id: Uuid,
    pub contract_id: Option<Uuid>,
    pub agreement_ref: Option<String>,
    /// Defaults to now.
    pub out_at: Option<DateTime<Utc>>,
    pub expected_return_at: NaiveDate,
    pub is_lease: bool,
    pub lease_cycle: Option<String>,
    pub lease_cost_minor: Option<i64>,
    pub lease_currency: Option<String>,
    pub notes: Option<String>,
    pub items: Vec<LoanIssueItem>,
}

pub struct LoanOutcome {
    pub loan: ClientLoan,
    pub lines: Vec<ClientLoanLine>,
    /// The ledger legs appended, in order (stock-sourced devices only).
    pub legs: Vec<MovementLeg>,
}

fn lease_patch(req: &LoanIssueRequest, out_date: NaiveDate) -> crate::assets::AssetLifecyclePatch {
    crate::assets::AssetLifecyclePatch {
        ownership: Some("leased".into()),
        rental_start: Some(out_date),
        rental_end: Some(req.expected_return_at),
        rental_cost_minor: req.lease_cost_minor,
        rental_currency: req.lease_currency.clone(),
        rental_cycle: req.lease_cycle.clone(),
        // A lease changes what the unit is for, so the patch says it may move
        // the class. On this path it writes nothing: `issue_unit_class` already
        // moved the unit to `rental_out` with the loan as its source, and the
        // class `leased` implies for a client-scoped row is the class the unit
        // already carries. The flag is what keeps a caller that patches a lease
        // onto a unit nobody moved from getting a silent class edit.
        record_class_movement: true,
        ..Default::default()
    }
}

/// Moves a unit into the class a client agreement gives it (`rental_out`) and
/// returns the class it came from plus the asset it is already paired with.
///
/// The origin is what a return goes back to when the caller does not route the
/// unit somewhere else, so it is returned rather than re-derived from a guess.
async fn issue_unit_class(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    unit: PoolUnit,
    label: &str,
    loan_id: Uuid,
) -> DbResult<(CustodyClass, PoolUnitState)> {
    let state = pool_unit_classes(conn, org_id, unit).await?;
    let current = state.class.clone();
    let parsed = CustodyClass::parse(&current).map_err(|e| DbError::Conflict(e.to_string()))?;
    if !loan_issue_origin_is_valid(parsed) {
        return Err(DbError::Conflict(format!(
            "{label} is {current} and cannot be issued on a client agreement"
        )));
    }
    // The repair hold: a unit returned for refurbishment is held out of every
    // issue until repair is released. The serial carries `in_repair`; an
    // asset-only unit carries it on the asset row, which is what a PATCH to the
    // asset status clears when the repair completes.
    if state.serial_status.as_deref() == Some(REPAIR_STATUS)
        || state.asset_status.as_deref() == Some(REPAIR_STATUS)
    {
        return Err(DbError::Conflict(format!(
            "{label} is held for repair and cannot be issued until the repair is released"
        )));
    }
    // The class trigger refuses this when it writes a movement, but an issue
    // that needs no movement (the unit is already `rental_out`) never reaches
    // it. A unit consumed on a ticket is on a job, not on a shelf to lend.
    if let Some(serial_id) = state.serial_id {
        let on_ticket: Option<Uuid> =
            sqlx::query_scalar("SELECT ticket_id FROM stock_serials WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(serial_id)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        if on_ticket.is_some() {
            return Err(DbError::Conflict(format!(
                "{label} is consumed on a ticket and cannot be issued"
            )));
        }
    }
    if parsed != LOAN_OPEN_CLASS {
        record_class_move(
            conn,
            org_id,
            user_id,
            unit,
            &current,
            LOAN_OPEN_CLASS.as_str(),
            "issued on a client agreement",
            "client_loan",
            Some(loan_id),
            None,
            None,
        )
        .await?;
    }
    Ok((parsed, state))
}

/// The repair hold's status value, on the serial and on the asset row.
const REPAIR_STATUS: &str = "in_repair";

/// The ledger leg a loan owes for a serialized unit, and the serial's move to
/// the client. One place for both the serial-sourced and the asset-sourced path,
/// because they are the same physical act.
///
/// The shelf leg is owed only when the unit actually leaves saleable stock from
/// a shelf. A `rental_out` unit left saleable quantity when it was commissioned,
/// and a unit with no shelf never was in it; both are the identity case and write
/// no ledger row. The serial still becomes `loaned`.
#[allow(clippy::too_many_arguments)]
async fn loan_stock_leg(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    client_id: Uuid,
    stock_item_id: Uuid,
    serial_id: Uuid,
    label: &str,
    location_id: Option<Uuid>,
    origin: CustodyClass,
    now: DateTime<Utc>,
) -> DbResult<Option<MovementLeg>> {
    let leg = match (origin, location_id) {
        (CustodyClass::RentalOut, _) => None,
        (_, None) => {
            return Err(DbError::Conflict(format!(
                "{label} has no location; it is not in stock"
            )))
        }
        (_, Some(location_id)) => {
            let item = crate::stock::get_stock_item(&mut *conn, org_id, stock_item_id).await?;
            let movement = StockMovement {
                id: Uuid::now_v7(),
                org_id,
                stock_item_id,
                kind: "loan".into(),
                from_location_id: Some(location_id),
                to_location_id: None,
                qty_milli: -1000,
                cost_minor: None,
                currency: None,
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_cost_minor: None,
                serial_id: Some(serial_id),
                client_id: Some(client_id),
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
            Some(MovementLeg {
                movement: outcome.movement,
                low: outcome.low,
            })
        }
    };
    // WHERE-IT-IS-NOW: with the client. With a leg the tick left the serial
    // `out`; without one it never left `in_stock`.
    let expected = if leg.is_some() { "out" } else { "in_stock" };
    let n = sqlx::query!(
        r#"UPDATE stock_serials
             SET status = 'loaned', client_id = $3, updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status = $4"#,
        org_id,
        serial_id,
        client_id,
        expected,
    )
    .execute(&mut *conn)
    .await?;
    if n.rows_affected() != 1 {
        return Err(DbError::Conflict(format!(
            "{label} could not be issued (it is not on the shelf)"
        )));
    }
    Ok(leg)
}

/// Issues the loan: header first, then one line per device. Stock-sourced
/// devices append their `loan` ledger leg, flip their serial to `loaned`
/// (carrying the client), and instantiate a `DeviceAsset` in the client's
/// fleet with identity keys; asset-sourced devices get their rental fields
/// stamped when the loan is a lease.
pub async fn issue_loan(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    req: &LoanIssueRequest,
) -> DbResult<LoanOutcome> {
    let now = Utc::now();
    crate::clients::get_client(&mut *conn, org_id, req.client_id).await?;
    let contract = match req.contract_id {
        Some(contract_id) => {
            Some(crate::contracts::get_contract(&mut *conn, org_id, contract_id).await?)
        }
        None => None,
    };
    if req.items.is_empty() {
        return Err(DbError::Conflict("a loan needs at least one device".into()));
    }
    for item in &req.items {
        if item.stock_item_id.is_none() && item.device_asset_id.is_none() {
            return Err(DbError::Conflict(
                "each loan item needs a stock item or a device asset".into(),
            ));
        }
        if item.stock_item_id.is_none() && item.serial_id.is_some() {
            return Err(DbError::Conflict(
                "a serial only makes sense with a stock item".into(),
            ));
        }
    }

    let loan = ClientLoan {
        id: Uuid::now_v7(),
        org_id,
        client_id: req.client_id,
        contract_id: req.contract_id,
        agreement_ref: req.agreement_ref.clone(),
        out_at: req.out_at.unwrap_or(now),
        expected_return_at: req.expected_return_at,
        returned_at: None,
        status: "open".into(),
        is_lease: req.is_lease,
        lease_cycle: req.lease_cycle.clone(),
        lease_cost_minor: req.lease_cost_minor,
        lease_currency: req.lease_currency.clone(),
        notes: req.notes.clone(),
        return_notes: None,
        issued_by: Some(user_id),
        created_at: now,
        updated_at: now,
    };
    loan.validate()
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    // The run bills a lease under its contract, and it is single-currency. A
    // lease in another currency would reach the run as a charge it must refuse,
    // aborting billing for every other contract in that sweep. Refuse it here,
    // where the agreement is written, rather than there.
    if let (true, Some(contract)) = (loan.is_lease, contract.as_ref()) {
        if contract.client_id != loan.client_id {
            return Err(DbError::Conflict(format!(
                "contract {} belongs to another client; a lease bills under it for its own client",
                contract.number
            )));
        }
    }
    if let (true, Some(contract), Some(currency)) = (
        loan.is_lease,
        contract.as_ref(),
        loan.lease_currency.as_deref(),
    ) {
        if !currency.eq_ignore_ascii_case(&contract.currency) {
            return Err(DbError::Conflict(format!(
                "lease bills in {currency} but its contract {} bills in {}",
                contract.number, contract.currency
            )));
        }
    }
    insert_loan(conn, &loan).await?;

    let out_date = loan.out_at.date_naive();
    let mut lines = Vec::with_capacity(req.items.len());
    let mut legs = Vec::with_capacity(req.items.len());

    for item in &req.items {
        if let Some(stock_item_id) = item.stock_item_id {
            let stock_item =
                crate::stock::get_stock_item(&mut *conn, org_id, stock_item_id).await?;
            if stock_item.kind != "serialized" {
                return Err(DbError::Conflict(format!(
                    "loaned device {} is not serialized; loans issue physical units",
                    stock_item.name
                )));
            }
            if !stock_item.active {
                return Err(DbError::Conflict(format!(
                    "loaned device {} is archived",
                    stock_item.name
                )));
            }
            // Explicit serial or the oldest in-stock unit (deterministic).
            let serial = match item.serial_id {
                Some(serial_id) => {
                    let serial = crate::stock::get_serial(&mut *conn, org_id, serial_id).await?;
                    if serial.stock_item_id != stock_item.id {
                        return Err(DbError::Conflict(format!(
                            "serial {} does not belong to {}",
                            serial.serial_number, stock_item.name
                        )));
                    }
                    serial
                }
                None => {
                    // The picker only offers a unit the issue path will accept.
                    // Status is the physical fact (is it on the shelf); the class
                    // is what the unit is for, and a pool or client-held unit is
                    // not an automatic loan candidate. Filtering here means the
                    // oldest eligible unit is chosen, instead of the oldest unit
                    // being chosen and refused after the pick.
                    let row = sqlx::query!(
                        r#"SELECT s.id AS "id!" FROM stock_serials s
                            WHERE s.org_id = $1 AND s.stock_item_id = $2
                              AND s.status = 'in_stock'
                              AND s.custody_class = ANY($3)
                              AND NOT EXISTS (
                                  SELECT 1 FROM device_assets a
                                   WHERE a.org_id = $1 AND a.serial_id = s.id
                                     AND a.status = 'in_repair'
                              )
                            ORDER BY s.created_at, s.id
                            LIMIT 1"#,
                        org_id,
                        stock_item.id,
                        &ISSUE_ELIGIBLE_CLASSES
                            .iter()
                            .map(|class| class.to_string())
                            .collect::<Vec<String>>(),
                    )
                    .fetch_optional(&mut *conn)
                    .await?;
                    match row {
                        Some(row) => crate::stock::get_serial(&mut *conn, org_id, row.id).await?,
                        None => {
                            return Err(DbError::Conflict(format!(
                                "no in-stock unit of {} that can be issued to a client",
                                stock_item.name
                            )))
                        }
                    }
                }
            };
            if serial.status != "in_stock" {
                return Err(DbError::Conflict(format!(
                    "serial {} is not in stock (status {})",
                    serial.serial_number, serial.status
                )));
            }
            // 0. Custody first: the unit leaves sale stock (or the fleet) and is
            //    `rental_out` while the client holds it. Written before the loan
            //    line exists, because the ledger refuses a move while an open
            //    agreement holds the unit — and the line is that agreement.
            let (issued_from, state) = issue_unit_class(
                conn,
                org_id,
                user_id,
                PoolUnit {
                    side: UnitSide::Serial,
                    id: serial.id,
                },
                &format!("serial {}", serial.serial_number),
                loan.id,
            )
            .await?;
            let paired_asset = state.asset_id;
            // 1. The shelf leg and the serial's move to the client, in one
            //    place. The location is the one read under the custody lock, not
            //    the earlier `get_serial` read.
            let leg = loan_stock_leg(
                conn,
                org_id,
                user_id,
                loan.client_id,
                stock_item.id,
                serial.id,
                &format!("serial {}", serial.serial_number),
                state.location_id,
                issued_from,
                now,
            )
            .await?;

            // 3. The device's row in the client's fleet. A serial that is
            //    already paired keeps that row — one unit, one asset — and the
            //    agreement places it with the client. An unpaired one gets a row
            //    born in `rental_out`, so the pair agrees the moment it is
            //    linked (migration 0233 refuses a mismatched pair).
            let asset_id = match paired_asset {
                Some(existing) => {
                    sqlx::query(
                        r#"UPDATE device_assets
                             SET client_id = $3, scope = 'client', site_id = NULL,
                                 updated_at = now()
                           WHERE org_id = $1 AND id = $2"#,
                    )
                    .bind(org_id)
                    .bind(existing)
                    .bind(loan.client_id)
                    .execute(&mut *conn)
                    .await?;
                    existing
                }
                None => {
                    let asset_id = Uuid::now_v7();
                    let tag = format!("LOAN-{}", asset_id.simple().to_string()[..8].to_uppercase());
                    let fields = serde_json::json!({ "serial_number": serial.serial_number });
                    sqlx::query(
                        r#"INSERT INTO device_assets
                             (id, org_id, client_id, scope, tag, name, kind, status,
                              fields, custody_class)
                           VALUES ($1, $2, $3, 'client', $4, $5, 'device', 'active',
                                   $6, 'rental_out')"#,
                    )
                    .bind(asset_id)
                    .bind(org_id)
                    .bind(loan.client_id)
                    .bind(&tag)
                    .bind(&stock_item.name)
                    .bind(&fields)
                    .execute(&mut *conn)
                    .await?;
                    sqlx::query(
                        r#"UPDATE stock_serials SET device_asset_id = $3, updated_at = now()
                           WHERE org_id = $1 AND id = $2 AND device_asset_id IS NULL"#,
                    )
                    .bind(org_id)
                    .bind(serial.id)
                    .bind(asset_id)
                    .execute(&mut *conn)
                    .await?;
                    asset_id
                }
            };
            let identity = serde_json::json!({
                "serial": core_domain::entities::config_identity::normalize_serial(&serial.serial_number),
            });
            crate::config_identity::refresh_identity_keys(&mut *conn, org_id, asset_id, &identity)
                .await?;
            if loan.is_lease {
                crate::assets::update_asset_lifecycle(
                    &mut *conn,
                    org_id,
                    asset_id,
                    &lease_patch(req, out_date),
                )
                .await?;
            }

            // 4. The line cites the asset + the ledger leg.
            let line = insert_line(
                conn,
                org_id,
                loan.id,
                Some(stock_item.id),
                Some(serial.id),
                Some(asset_id),
                leg.as_ref().map(|leg| leg.movement.id),
                now,
            )
            .await
            .map_err(|e| match e {
                DbError::Sql(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                    let message = match db.constraint() {
                        Some("uq_client_loan_open_unit") => {
                            format!("serial {} is already on an open loan", serial.serial_number)
                        }
                        Some("uq_client_loan_open_asset") => {
                            "that unit is already on an open loan".to_string()
                        }
                        _ => "that unit is already on an open loan".to_string(),
                    };
                    DbError::Conflict(message)
                }
                other => other,
            })?;
            if let Some(leg) = leg {
                legs.push(leg);
            }
            lines.push(line);
        } else if let Some(device_asset_id) = item.device_asset_id {
            let asset = crate::assets::get_asset(&mut *conn, org_id, device_asset_id).await?;
            // The unit is placed with the client: a fleet or pool asset is
            // `rental_out` for as long as the agreement holds it.
            let (issued_from, state) = issue_unit_class(
                conn,
                org_id,
                user_id,
                PoolUnit {
                    side: UnitSide::Asset,
                    id: device_asset_id,
                },
                &format!("asset {}", asset.tag),
                loan.id,
            )
            .await?;
            // An asset that is one projection of a serialized unit gets the same
            // treatment as a serial-sourced issue: the shelf leg and the serial's
            // move to the client. Issuing by asset id is not a way around the
            // stock ledger.
            let leg = match (state.serial_id, state.stock_item_id) {
                (Some(serial_id), Some(stock_item_id)) => {
                    loan_stock_leg(
                        conn,
                        org_id,
                        user_id,
                        loan.client_id,
                        stock_item_id,
                        serial_id,
                        &format!("asset {}", asset.tag),
                        state.location_id,
                        issued_from,
                        now,
                    )
                    .await?
                }
                _ => None,
            };
            // The agreement places the asset with the client, exactly as the
            // serial path does for a paired row. `update_asset_lifecycle`
            // derives the class from `scope`, so this must land before the
            // lease patch or a `leased` ownership derives `rental_in` for a
            // unit the client is holding.
            sqlx::query(
                r#"UPDATE device_assets
                     SET client_id = $3, scope = 'client', site_id = NULL,
                         updated_at = now()
                   WHERE org_id = $1 AND id = $2"#,
            )
            .bind(org_id)
            .bind(device_asset_id)
            .bind(loan.client_id)
            .execute(&mut *conn)
            .await?;
            if loan.is_lease {
                crate::assets::update_asset_lifecycle(
                    &mut *conn,
                    org_id,
                    device_asset_id,
                    &lease_patch(req, out_date),
                )
                .await?;
            }
            // The line carries the pair when the asset is one projection of a
            // serialized unit. The return path reads the line to decide whether
            // a receipt leg and a serial re-arm are owed; leaving the pair empty
            // here would let the return close without restoring the shelf.
            let line = insert_line(
                conn,
                org_id,
                loan.id,
                state.stock_item_id,
                state.serial_id,
                Some(asset.id),
                leg.as_ref().map(|leg| leg.movement.id),
                now,
            )
            .await?;
            if let Some(leg) = leg {
                legs.push(leg);
            }
            lines.push(line);
        }
    }

    Ok(LoanOutcome { loan, lines, legs })
}

async fn insert_loan(conn: &mut sqlx::PgConnection, l: &ClientLoan) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO client_loans
             (id, org_id, client_id, contract_id, agreement_ref, out_at, expected_return_at,
              returned_at, status, is_lease, lease_cycle, lease_cost_minor, lease_currency,
              notes, return_notes, issued_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"#,
        l.id,
        l.org_id,
        l.client_id,
        l.contract_id,
        l.agreement_ref.as_deref(),
        l.out_at,
        l.expected_return_at,
        l.returned_at,
        l.status,
        l.is_lease,
        l.lease_cycle.as_deref(),
        l.lease_cost_minor,
        l.lease_currency.as_deref(),
        l.notes.as_deref(),
        l.return_notes.as_deref(),
        l.issued_by,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_line(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    loan_id: Uuid,
    stock_item_id: Option<Uuid>,
    serial_id: Option<Uuid>,
    device_asset_id: Option<Uuid>,
    movement_out_id: Option<Uuid>,
    _now: DateTime<Utc>,
) -> DbResult<ClientLoanLine> {
    let row = sqlx::query_as!(
        LineRow,
        r#"INSERT INTO client_loan_lines
             (id, org_id, loan_id, stock_item_id, serial_id, device_asset_id, movement_out_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, loan_id, stock_item_id, serial_id, device_asset_id,
                     movement_out_id, movement_in_id, returned_at, return_condition,
                     return_notes, created_at, updated_at"#,
        Uuid::now_v7(),
        org_id,
        loan_id,
        stock_item_id,
        serial_id,
        device_asset_id,
        movement_out_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(line_row(row))
}

// Return ------------------------------------------------------------------------

#[derive(Clone)]
pub struct LoanReturnTarget {
    pub line_id: Uuid,
    pub return_condition: Option<String>,
    pub return_notes: Option<String>,
    /// Where the unit goes now that it is back: `pool | sale_stock |
    /// refurbishment | ewaste`. Absent = back to the class it was issued from,
    /// which is what a plain end-of-lease return wants.
    pub disposition: Option<String>,
    /// A damage charge raised with this return. The route builds it (it owns
    /// the FX snapshot and the agreement's currency); the lease's contract
    /// bills it in the next ordinary run like any other charge.
    pub damage_charge: Option<NewRentalCharge>,
}

/// The custody class a return routes the unit to. `refurbishment` is the
/// workshop holding: the unit is ours, it is not sellable or rentable until it
/// is fixed, and `internal_fleet` is the class that says exactly that. E-waste
/// is custody, not owned stock. The pool is where a rental unit already is, so
/// it routes to the class it carries.
pub fn return_disposition_class(disposition: &str) -> DbResult<CustodyClass> {
    Ok(match disposition.trim() {
        "pool" => CustodyClass::RentalOut,
        "sale_stock" => CustodyClass::SaleStock,
        "refurbishment" => CustodyClass::InternalFleet,
        "ewaste" => CustodyClass::EwasteCustody,
        other => {
            return Err(DbError::Conflict(format!(
                "return disposition must be one of: pool, sale_stock, refurbishment, ewaste (got {other})"
            )))
        }
    })
}

/// The class a unit was issued from, read from the ledger: the `from_class` of
/// its newest issue movement for this agreement. A unit that was already
/// `rental_out` when it went out has no such movement and returns to the pool.
async fn loan_issue_origin(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    loan_id: Uuid,
    serial_id: Option<Uuid>,
    device_asset_id: Option<Uuid>,
    current: CustodyClass,
) -> DbResult<CustodyClass> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"SELECT from_class FROM inventory_class_movements
            WHERE org_id = $1 AND source_kind = 'client_loan' AND source_id = $2
              AND ((unit_kind = 'serial' AND serial_id = $3)
                OR (unit_kind = 'asset' AND device_asset_id = $4))
            ORDER BY created_at DESC, id DESC
            LIMIT 1"#,
    )
    .bind(org_id)
    .bind(loan_id)
    .bind(serial_id)
    .bind(device_asset_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((class,)) = row {
        return CustodyClass::parse(&class).map_err(|e| DbError::Conflict(e.to_string()));
    }
    // No movement means the loan predates the class ledger. The unit's current
    // class is what it actually is, so a legacy sale-stock loan returns to sale
    // stock instead of being assumed to be pool gear. `rental_in` is the one
    // class a client return cannot keep: it is the vendor's, and a unit leased
    // to a client is `rental_out` (ADR 0035 §4.1), so that legacy row routes to
    // the pool.
    Ok(match current {
        CustodyClass::RentalIn => CustodyClass::RentalOut,
        other => other,
    })
}

/// Returns devices. Stock-sourced units ride a `loan_return` ledger leg and
/// re-arm to `in_stock` at the destination; a lease's asset gets its
/// `rental_end` closed at the actual return date. Partial returns keep the
/// loan open.
#[allow(clippy::too_many_arguments)]
pub async fn return_loan(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    loan_id: Uuid,
    to_location_id: Option<Uuid>,
    targets: &[LoanReturnTarget],
    notes: Option<&str>,
) -> DbResult<LoanOutcome> {
    let now = Utc::now();
    // Take the header lock before reading the lines. Two partial returns of the
    // same loan would otherwise both count the remaining lines and both decide
    // the loan is now complete; the lock makes the second one read the first
    // one's committed result. `FOR NO KEY UPDATE` is enough: the row's id and
    // foreign keys do not change, only its status.
    let locked = sqlx::query!(
        r#"SELECT id AS "id!" FROM client_loans
            WHERE org_id = $1 AND id = $2 FOR NO KEY UPDATE"#,
        org_id,
        loan_id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    if locked.is_none() {
        return Err(DbError::NotFound);
    }
    let (loan, lines) = get_loan(conn, org_id, loan_id).await?;
    if loan.status == "returned" {
        return Err(DbError::Conflict(
            "this loan is already fully returned".into(),
        ));
    }
    let outstanding: Vec<&ClientLoanLine> = lines.iter().filter(|l| !l.is_returned()).collect();
    let selected: Vec<LoanReturnTarget> = if targets.is_empty() {
        outstanding
            .iter()
            .map(|l| LoanReturnTarget {
                line_id: l.id,
                return_condition: None,
                return_notes: None,
                disposition: None,
                damage_charge: None,
            })
            .collect()
    } else {
        let mut seen = Vec::new();
        for t in targets {
            if seen.contains(&t.line_id) {
                return Err(DbError::Conflict(
                    "the same line appears twice in one return".into(),
                ));
            }
            let line = lines.iter().find(|l| l.id == t.line_id).ok_or_else(|| {
                DbError::Conflict(format!("line {} is not on this loan", t.line_id))
            })?;
            if line.is_returned() {
                return Err(DbError::Conflict(format!(
                    "line {} was already returned",
                    t.line_id
                )));
            }
            if let Some(condition) = &t.return_condition {
                if !core_domain::entities::is_valid_return_condition(condition) {
                    return Err(DbError::Conflict(
                        "return condition must be one of: good, fair, damaged".into(),
                    ));
                }
            }
            if let Some(disposition) = &t.disposition {
                return_disposition_class(disposition)?;
                if t.return_condition.is_none() {
                    return Err(DbError::Conflict(
                        "routing a returned unit out of the pool grades it: good, fair or damaged"
                            .into(),
                    ));
                }
            }
            if let Some(charge) = &t.damage_charge {
                let same_loan = charge.loan_id == loan.id
                    && charge.kind == "damage"
                    && charge.source_line_id == Some(t.line_id);
                if !same_loan {
                    return Err(DbError::Conflict(
                        "a damage charge must name this loan and this line".into(),
                    ));
                }
            }
            seen.push(t.line_id);
        }
        targets.to_vec()
    };

    let returning_stock = outstanding
        .iter()
        .filter(|l| l.from_stock() && selected.iter().any(|t| t.line_id == l.id))
        .count();
    let to_location_id = match (returning_stock, to_location_id) {
        (0, _) => to_location_id,
        (_, Some(location_id)) => {
            crate::stock::get_stock_location(&mut *conn, org_id, location_id).await?;
            Some(location_id)
        }
        (_, None) => {
            return Err(DbError::Conflict(
                "returning stock-sourced devices needs a destination location".into(),
            ))
        }
    };

    let mut legs = Vec::with_capacity(selected.len());
    for t in &selected {
        let line = outstanding
            .iter()
            .find(|l| l.id == t.line_id)
            .expect("validated above");
        let mut movement_in_id: Option<Uuid> = None;

        // Resolve the unit and where it goes before the ledger leg: the leg is
        // only owed when the destination is counted in saleable stock, and a
        // unit that leaves custody must not increment it. The read takes the
        // canonical locks (asset, then serial), so the class it returns is the
        // class the routing below will move from.
        let unit = match (line.serial_id, line.device_asset_id) {
            (Some(serial_id), _) => PoolUnit {
                side: UnitSide::Serial,
                id: serial_id,
            },
            (None, Some(device_asset_id)) => PoolUnit {
                side: UnitSide::Asset,
                id: device_asset_id,
            },
            (None, None) => {
                return Err(DbError::Conflict(format!(
                    "line {} names no unit to route",
                    t.line_id
                )))
            }
        };
        let state = pool_unit_classes(conn, org_id, unit).await?;
        let current_class =
            CustodyClass::parse(&state.class).map_err(|e| DbError::Conflict(e.to_string()))?;
        let destination = match t.disposition.as_deref() {
            Some(disposition) => return_disposition_class(disposition)?,
            None => {
                loan_issue_origin(
                    conn,
                    org_id,
                    loan_id,
                    line.serial_id,
                    line.device_asset_id,
                    current_class,
                )
                .await?
            }
        };
        // Saleable stock is the shelf. A pool unit and an e-waste unit are not
        // on it; every other destination keeps the unit in the ledger's
        // quantity, as it was before this return.
        let counted_in_stock = !matches!(
            destination,
            CustodyClass::RentalOut | CustodyClass::EwasteCustody
        );

        // The lease ends with the return. Close it at the ACTUAL date (the
        // expected return was the plan; the asset carries what happened). The
        // ownership is NOT reset here: an asset leased before the custody work
        // can still be classed `rental_in`, and `rental_in` plus `owned`
        // violates `device_assets_rented_in_shape`. It is reset after routing,
        // when the class can hold it.
        //
        // This runs before the ledger leg so the asset row is locked before the
        // serial. The class-movement trigger takes the same order, so a return
        // and an issue on one unit cannot deadlock by taking them in opposite
        // order.
        if loan.is_lease {
            if let Some(asset_id) = line.device_asset_id {
                crate::assets::update_asset_lifecycle(
                    &mut *conn,
                    org_id,
                    asset_id,
                    &crate::assets::AssetLifecyclePatch {
                        rental_end: Some(now.date_naive()),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }

        // A line issued by asset id on a paired unit carries the pair; a line
        // written before that change does not, so the locked state fills the
        // gap. Either way the return restores the shelf and the serial.
        let leg_serial = line.serial_id.or(state.serial_id);
        let leg_item = line.stock_item_id.or(state.stock_item_id);
        if let (Some(serial_id), Some(stock_item_id)) = (leg_serial, leg_item) {
            if counted_in_stock && to_location_id.is_none() {
                return Err(DbError::Conflict(format!(
                    "unit on line {} returns to stock and needs a destination location",
                    t.line_id
                )));
            }
            let Some(location_id) = to_location_id else {
                return Err(DbError::Conflict(format!(
                    "unit on line {} needs a destination location to record its return",
                    t.line_id
                )));
            };
            if counted_in_stock {
                let item = crate::stock::get_stock_item(&mut *conn, org_id, stock_item_id).await?;
                let movement = StockMovement {
                    id: Uuid::now_v7(),
                    org_id,
                    stock_item_id,
                    kind: "loan_return".into(),
                    from_location_id: None,
                    to_location_id: Some(location_id),
                    qty_milli: 1000,
                    cost_minor: None,
                    currency: None,
                    fx_rate_used: None,
                    fx_rate_date: None,
                    fx_source: None,
                    base_cost_minor: None,
                    serial_id: Some(serial_id),
                    client_id: Some(loan.client_id),
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
                movement_in_id = Some(outcome.movement.id);
                legs.push(MovementLeg {
                    movement: outcome.movement,
                    low: outcome.low,
                });
            }
            // The lane-1 tick only arms serials that are `in_stock`; a
            // `loaned` unit re-arms here, in the same commit. Active even when
            // no ledger leg was owed: the serial still physically comes back.
            let n = sqlx::query!(
                r#"UPDATE stock_serials
                     SET status = 'in_stock', location_id = $3, client_id = NULL,
                         movement_in_id = COALESCE($4, movement_in_id), updated_at = now()
                   WHERE org_id = $1 AND id = $2 AND status = 'loaned'"#,
                org_id,
                serial_id,
                location_id,
                movement_in_id,
            )
            .execute(&mut *conn)
            .await?;
            if n.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "unit on line {} is not on loan",
                    t.line_id
                )));
            }
        }
        let n = sqlx::query!(
            r#"UPDATE client_loan_lines
                 SET returned_at = $4, return_condition = $5, return_notes = $6,
                     movement_in_id = COALESCE($7, movement_in_id), updated_at = now()
               WHERE org_id = $1 AND id = $2 AND loan_id = $3 AND returned_at IS NULL"#,
            org_id,
            t.line_id,
            loan_id,
            now,
            t.return_condition.as_deref(),
            t.return_notes.as_deref(),
            movement_in_id,
        )
        .execute(&mut *conn)
        .await?;
        if n.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "line {} was already returned",
                t.line_id
            )));
        }

        // Custody: the agreement is closed (above), so the unit can move. It
        // goes back to the class resolved at the top unless the caller routed it
        // somewhere else, and every routing that changes the class is one
        // movement whose source is this return line. Routing back to the pool is
        // the identity case: the unit is already `rental_out`, and a movement
        // that does not change the class would be a lie.
        if current_class != destination {
            let reason = format!(
                "returned from client agreement → {}: {}",
                destination.as_str(),
                t.return_condition.as_deref().unwrap_or("ungraded")
            );
            record_class_move(
                conn,
                org_id,
                user_id,
                unit,
                &state.class,
                destination.as_str(),
                &reason,
                "client_loan",
                Some(t.line_id),
                t.return_condition.as_deref(),
                None,
            )
            .await?;
        }

        // The client no longer holds the unit, so its asset row leaves the
        // client's fleet and returns to the internal projection the issue put
        // aside. Scope and client move together (the shape constraint ties
        // them), and the site goes with them: whatever site the client had it
        // at is not where it is now.
        if let Some(asset_id) = line.device_asset_id {
            // A unit returned for refurbishment is not ready to go out again.
            // The asset's status carries the hold (`in_repair`), which the issue
            // path refuses and the automatic picker excludes; clearing it is the
            // ordinary asset status edit a technician makes when the repair is
            // done. The class stays `internal_fleet`: the unit is ours, in the
            // workshop, not sellable and not rentable.
            // Only an explicit refurbishment routing holds the unit. A default
            // return of an InternalFleet unit resolves to InternalFleet too, and
            // a fleet unit that came back in working order must stay issuable.
            let hold_for_repair = t.disposition.as_deref() == Some("refurbishment");
            sqlx::query!(
                r#"UPDATE device_assets
                      SET scope = 'internal_org', client_id = NULL, site_id = NULL,
                          status = CASE WHEN $3 THEN 'in_repair' ELSE status END,
                          updated_at = now()
                    WHERE org_id = $1 AND id = $2"#,
                org_id,
                asset_id,
                hold_for_repair,
            )
            .execute(&mut *conn)
            .await?;
        }

        // The unit is ours again once the lease ends. This is the second half of
        // the split above: the class has been routed, so `owned` is a shape the
        // row can satisfy in every destination, including a legacy `rental_in`
        // unit whose movement just took it to `rental_out`. One class movement
        // remains the only class write; this patch only stops the row claiming
        // a lease that has ended.
        if loan.is_lease {
            if let Some(asset_id) = line.device_asset_id {
                crate::assets::update_asset_lifecycle(
                    &mut *conn,
                    org_id,
                    asset_id,
                    &crate::assets::AssetLifecyclePatch {
                        ownership: Some("owned".into()),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }

        if let Some(charge) = &t.damage_charge {
            insert_rental_charge(conn, org_id, user_id, charge).await?;
        }
    }

    let remaining: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM client_loan_lines
           WHERE org_id = $1 AND loan_id = $2 AND returned_at IS NULL"#,
        org_id,
        loan_id,
    )
    .fetch_one(&mut *conn)
    .await?;
    if remaining == 0 {
        sqlx::query!(
            r#"UPDATE client_loans
                 SET status = 'returned', returned_at = $3,
                     return_notes = COALESCE($4, return_notes), updated_at = now()
               WHERE org_id = $1 AND id = $2"#,
            org_id,
            loan_id,
            now,
            notes,
        )
        .execute(&mut *conn)
        .await?;
    } else if notes.is_some() {
        sqlx::query!(
            r#"UPDATE client_loans SET return_notes = $3, updated_at = now()
               WHERE org_id = $1 AND id = $2"#,
            org_id,
            loan_id,
            notes,
        )
        .execute(&mut *conn)
        .await?;
    }

    let (loan, lines) = get_loan(conn, org_id, loan_id).await?;
    Ok(LoanOutcome { loan, lines, legs })
}

// Reads --------------------------------------------------------------------------

pub async fn get_loan(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<(ClientLoan, Vec<ClientLoanLine>)> {
    let row = sqlx::query_as!(
        LoanRow,
        r#"SELECT id, org_id, client_id, contract_id, agreement_ref, out_at,
                  expected_return_at, returned_at, status, is_lease, lease_cycle,
                  lease_cost_minor, lease_currency, notes, return_notes, issued_by,
                  created_at, updated_at
           FROM client_loans WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    let lines = list_lines(conn, org_id, id).await?;
    Ok((loan_row(row), lines))
}

pub async fn list_lines(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    loan_id: Uuid,
) -> DbResult<Vec<ClientLoanLine>> {
    let rows = sqlx::query_as!(
        LineRow,
        r#"SELECT id, org_id, loan_id, stock_item_id, serial_id, device_asset_id,
                  movement_out_id, movement_in_id, returned_at, return_condition,
                  return_notes, created_at, updated_at
           FROM client_loan_lines
           WHERE org_id = $1 AND loan_id = $2
           ORDER BY created_at, id"#,
        org_id,
        loan_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(line_row).collect())
}

pub struct LoanFilter<'a> {
    /// `open` | `returned` (None = both).
    pub status: Option<&'a str>,
    pub client_id: Option<Uuid>,
    pub overdue_only: bool,
    pub limit: i64,
}

pub async fn list_loans(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    f: &LoanFilter<'_>,
) -> DbResult<Vec<ClientLoan>> {
    let rows = sqlx::query_as!(
        LoanRow,
        r#"SELECT id, org_id, client_id, contract_id, agreement_ref, out_at,
                  expected_return_at, returned_at, status, is_lease, lease_cycle,
                  lease_cost_minor, lease_currency, notes, return_notes, issued_by,
                  created_at, updated_at
           FROM client_loans
           WHERE org_id = $1
             AND ($2::text IS NULL OR status = $2)
             AND ($3::uuid IS NULL OR client_id = $3)
             AND (NOT $4::boolean
                  OR (returned_at IS NULL AND expected_return_at < current_date))
           ORDER BY created_at DESC
           LIMIT $5"#,
        org_id,
        f.status,
        f.client_id,
        f.overdue_only,
        f.limit,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(loan_row).collect())
}
// --- the rental pool ----------------------------------------------------------
//
// Owned rental gear is its own custody class (ADR 0035 §4.1): `rental_out`.
// Commission moves sale stock into it, decommission moves it back, and both are
// class movements through the migration-0232 ledger — the trigger applies the
// new class to the unit and its paired projection, refuses a stale `from_class`,
// and refuses a move while the unit is on an open agreement.
//
// Both calls carry the two facts a pool move has to name: the condition grade
// the unit is in, and, for a unit that stores data, the evidence of the wipe.
// They ride the movement row because that is the record of the act.

/// Which projection of the unit a pool move names. A caller names one, never
/// both; the ledger's apply trigger keeps the pair in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitSide {
    Serial,
    Asset,
}

impl UnitSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitSide::Serial => "serial",
            UnitSide::Asset => "asset",
        }
    }
}

/// The unit a pool move acts on: exactly one projection and its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolUnit {
    pub side: UnitSide,
    pub id: Uuid,
}

pub struct PoolMoveRequest {
    pub unit: PoolUnit,
    /// `good | fair | damaged` — the grade the unit is in on the way in or out.
    pub condition_grade: String,
    /// The document that proves the device was wiped. Required by the route
    /// when the unit stores data; recorded here either way.
    pub data_wipe_ref: Option<String>,
    pub note: Option<String>,
}

/// The movement a pool leg wrote, flattened for the API.
#[derive(Debug, Clone)]
pub struct PoolMove {
    pub id: Uuid,
    pub unit_kind: String,
    pub serial_id: Option<Uuid>,
    pub device_asset_id: Option<Uuid>,
    pub from_class: String,
    pub to_class: String,
    pub condition_grade: String,
    pub data_wipe_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

const POOL_CLASS: &str = "rental_out";
const SALE_CLASS: &str = "sale_stock";

fn valid_grade(grade: &str) -> bool {
    core_domain::entities::is_valid_return_condition(grade)
}

/// Reads the unit's current class and its paired projection, scoped to the org.
/// What a custody action needs to know about a unit, read under the locks the
/// movement trigger takes. Everything here is committed state at the moment the
/// caller may write: the class it moves from, the pair it must not change, the
/// shelf the quantity belongs to, and the status that may hold it out of issue.
struct PoolUnitState {
    class: String,
    serial_id: Option<Uuid>,
    asset_id: Option<Uuid>,
    /// The serial's shelf. `None` for a unit with no shelf (an asset-only pool
    /// unit, or a pool unit whose shelf was cleared) — that is the identity case
    /// for a loan leg, not an error.
    location_id: Option<Uuid>,
    stock_item_id: Option<Uuid>,
    /// `in_stock`, `loaned`, `in_repair` … for the serial, when it has one.
    serial_status: Option<String>,
    /// The asset's own status; `in_repair` is the repair hold for an asset-only
    /// unit, which has no serial status to carry it.
    asset_status: Option<String>,
}

async fn pool_unit_classes(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    unit: PoolUnit,
) -> DbResult<PoolUnitState> {
    // The pair is read under the same lock order the class-movement trigger
    // takes (asset first, then serial), so a concurrent conversion cannot move
    // the partner between this read and the movement the caller writes. The
    // pair is re-read after both locks: if it changed while we waited, the unit
    // this caller looked at is not the unit it is about to move, and the answer
    // is a retry, not a stale `from_class`.
    let first_pass: (String, Option<Uuid>, Option<Uuid>) = match unit.side {
        UnitSide::Serial => {
            let row = sqlx::query!(
                r#"SELECT custody_class AS "custody_class!", device_asset_id
                     FROM stock_serials WHERE org_id = $1 AND id = $2"#,
                org_id,
                unit.id,
            )
            .fetch_optional(&mut *conn)
            .await?;
            let row = row.ok_or(DbError::NotFound)?;
            (row.custody_class, Some(unit.id), row.device_asset_id)
        }
        UnitSide::Asset => {
            let row = sqlx::query!(
                r#"SELECT custody_class AS "custody_class!", serial_id
                     FROM device_assets WHERE org_id = $1 AND id = $2"#,
                org_id,
                unit.id,
            )
            .fetch_optional(&mut *conn)
            .await?;
            let row = row.ok_or(DbError::NotFound)?;
            (row.custody_class, row.serial_id, Some(unit.id))
        }
    };

    // Asset before serial, the order the trigger uses.
    if let Some(asset_id) = first_pass.2 {
        sqlx::query!(
            "SELECT id FROM device_assets WHERE org_id = $1 AND id = $2 FOR UPDATE",
            org_id,
            asset_id,
        )
        .fetch_optional(&mut *conn)
        .await?;
    }
    if let Some(serial_id) = first_pass.1 {
        sqlx::query!(
            "SELECT id FROM stock_serials WHERE org_id = $1 AND id = $2 FOR UPDATE",
            org_id,
            serial_id,
        )
        .fetch_optional(&mut *conn)
        .await?;
    }

    let locked: (String, Option<Uuid>, Option<Uuid>) = match unit.side {
        UnitSide::Serial => {
            let row = sqlx::query!(
                r#"SELECT custody_class AS "custody_class!", device_asset_id
                     FROM stock_serials WHERE org_id = $1 AND id = $2"#,
                org_id,
                unit.id,
            )
            .fetch_optional(&mut *conn)
            .await?;
            let row = row.ok_or(DbError::NotFound)?;
            (row.custody_class, Some(unit.id), row.device_asset_id)
        }
        UnitSide::Asset => {
            let row = sqlx::query!(
                r#"SELECT custody_class AS "custody_class!", serial_id
                     FROM device_assets WHERE org_id = $1 AND id = $2"#,
                org_id,
                unit.id,
            )
            .fetch_optional(&mut *conn)
            .await?;
            let row = row.ok_or(DbError::NotFound)?;
            (row.custody_class, row.serial_id, Some(unit.id))
        }
    };
    if locked.1 != first_pass.1 || locked.2 != first_pass.2 {
        return Err(DbError::Conflict(
            "the unit was paired or unpaired while the move was prepared; retry".into(),
        ));
    }

    // The shelf and the status are read now, under the serial lock: a loan leg
    // that used a location read before the lock could decrement a shelf the unit
    // had already left.
    let (location_id, stock_item_id, serial_status) = match locked.1 {
        Some(serial_id) => {
            let row = sqlx::query!(
                r#"SELECT location_id, stock_item_id AS "stock_item_id!", status AS "status!"
                     FROM stock_serials WHERE org_id = $1 AND id = $2"#,
                org_id,
                serial_id,
            )
            .fetch_optional(&mut *conn)
            .await?;
            match row {
                Some(row) => (row.location_id, Some(row.stock_item_id), Some(row.status)),
                None => (None, None, None),
            }
        }
        None => (None, None, None),
    };
    let asset_status: Option<String> = match locked.2 {
        Some(asset_id) => {
            sqlx::query_scalar!(
                r#"SELECT status FROM device_assets WHERE org_id = $1 AND id = $2"#,
                org_id,
                asset_id,
            )
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };

    Ok(PoolUnitState {
        class: locked.0,
        serial_id: locked.1,
        asset_id: locked.2,
        location_id,
        stock_item_id,
        serial_status,
        asset_status,
    })
}

/// The class-movement trigger raises `P0001` with a sentence naming the unit and
/// why it cannot move: an open client loan, an open vendor rental, a tool
/// checkout, a ticket. That sentence is about the caller's own unit and says
/// nothing it does not already know, so it becomes a typed 409 instead of a
/// generic database failure. The asset route maps the same signal; a pool leg
/// or a loan leg that hits the lock must answer the same way.
fn map_class_guard(e: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("P0001") {
            return DbError::Conflict(db.message().to_string());
        }
    }
    DbError::Sql(e)
}

/// Writes one class movement for a pool leg or a loan leg. `manual` is the
/// honest source for a pool leg: it is an operator act with no document table of
/// its own. A loan leg names the agreement instead, so the ledger can answer
/// "which agreement moved this unit" without a second join.
#[allow(clippy::too_many_arguments)]
async fn record_class_move(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    unit: PoolUnit,
    from_class: &str,
    to_class: &str,
    reason: &str,
    source_kind: &str,
    source_id: Option<Uuid>,
    condition_grade: Option<&str>,
    data_wipe_ref: Option<&str>,
) -> DbResult<PoolMove> {
    let (serial_id, asset_id) = match unit.side {
        UnitSide::Serial => (Some(unit.id), None),
        UnitSide::Asset => (None, Some(unit.id)),
    };
    let id = Uuid::now_v7();
    let row: (DateTime<Utc>,) = sqlx::query_as(
        r#"INSERT INTO inventory_class_movements
             (id, org_id, unit_kind, serial_id, device_asset_id, from_class, to_class,
              reason, source_kind, source_id, created_by, condition_grade, data_wipe_ref,
              data_wipe_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                   CASE WHEN $13::text IS NULL THEN NULL ELSE now() END)
           RETURNING created_at"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(unit.side.as_str())
    .bind(serial_id)
    .bind(asset_id)
    .bind(from_class)
    .bind(to_class)
    .bind(reason)
    .bind(source_kind)
    .bind(source_id)
    .bind(user_id)
    .bind(condition_grade)
    .bind(data_wipe_ref)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_class_guard)?;
    Ok(PoolMove {
        id,
        unit_kind: unit.side.as_str().to_string(),
        serial_id,
        device_asset_id: asset_id,
        from_class: from_class.to_string(),
        to_class: to_class.to_string(),
        condition_grade: condition_grade.unwrap_or_default().to_string(),
        data_wipe_ref: data_wipe_ref.map(str::to_string),
        created_at: row.0,
    })
}

#[allow(clippy::too_many_arguments)]
async fn record_pool_move(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    unit: PoolUnit,
    from_class: &str,
    to_class: &str,
    reason: &str,
    condition_grade: &str,
    data_wipe_ref: Option<&str>,
) -> DbResult<PoolMove> {
    record_class_move(
        conn,
        org_id,
        user_id,
        unit,
        from_class,
        to_class,
        reason,
        "manual",
        None,
        Some(condition_grade),
        data_wipe_ref,
    )
    .await
}

fn pool_reason(note: Option<&str>, what: &str, grade: &str) -> String {
    let mut reason = format!("{what}: condition {grade}");
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        reason.push_str(" — ");
        reason.push_str(note);
    }
    reason.chars().take(500).collect()
}

/// Moves a serial-backed unit in or out of saleable `stock_levels` at the shelf
/// it sits on. A pool unit is not saleable, so the level must stop counting it
/// when it is commissioned and start again when it is decommissioned; the class
/// movement is the audit record of why the quantity moved. A unit with no level
/// row (an asset-only purchase that never went through receive) has nothing to
/// adjust, which is not an error.
async fn adjust_pool_quantity(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    serial_id: Option<Uuid>,
    delta_milli: i64,
) -> DbResult<()> {
    let Some(serial_id) = serial_id else {
        return Ok(());
    };
    let row = sqlx::query!(
        r#"SELECT stock_item_id AS "stock_item_id!", location_id
             FROM stock_serials WHERE org_id = $1 AND id = $2"#,
        org_id,
        serial_id,
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let Some(location_id) = row.location_id else {
        return Ok(());
    };
    let item_id = row.stock_item_id;
    if delta_milli < 0 {
        // Lock the level row: the balance checked here must be the row the
        // update below changes, or a concurrent writer can slide under it.
        let held = sqlx::query_scalar!(
            r#"SELECT qty_milli FROM stock_levels
                WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3
                FOR UPDATE"#,
            org_id,
            item_id,
            location_id,
        )
        .fetch_optional(&mut *conn)
        .await?;
        match held {
            None => Ok(()),
            Some(qty) if qty + delta_milli >= 0 => {
                sqlx::query!(
                    r#"UPDATE stock_levels SET qty_milli = qty_milli + $4, updated_at = now()
                        WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3"#,
                    org_id,
                    item_id,
                    location_id,
                    delta_milli,
                )
                .execute(&mut *conn)
                .await?;
                Ok(())
            }
            Some(_) => Err(DbError::Conflict(
                "the shelf does not hold this unit in saleable quantity".into(),
            )),
        }
    } else {
        sqlx::query!(
            r#"INSERT INTO stock_levels
                 (id, org_id, stock_item_id, stock_location_id, qty_milli)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (org_id, stock_item_id, stock_location_id) DO UPDATE
                 SET qty_milli = stock_levels.qty_milli + $5, updated_at = now()"#,
            Uuid::now_v7(),
            org_id,
            item_id,
            location_id,
            delta_milli,
        )
        .execute(&mut *conn)
        .await?;
        // An incoming ledger movement re-arms the low-stock bell when it lifts
        // a shelf above its reorder point; a pool credit is the same act, so
        // it clears the marker the same way. A shelf still at or below the
        // point keeps it.
        sqlx::query!(
            r#"UPDATE stock_levels SET low_stock_notified_at = NULL
                WHERE org_id = $1 AND stock_item_id = $2 AND stock_location_id = $3
                  AND qty_milli > reorder_point_milli"#,
            org_id,
            item_id,
            location_id,
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}

/// Commission: sale stock → the rental pool. The unit leaves saleable quantity
/// (its class is the only truth about that) and becomes available for leases.
pub async fn commission_to_pool(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    req: &PoolMoveRequest,
) -> DbResult<PoolMove> {
    let grade = req.condition_grade.trim();
    if !valid_grade(grade) {
        return Err(DbError::Conflict(
            "condition grade must be one of: good, fair, damaged".into(),
        ));
    }
    let state = pool_unit_classes(conn, org_id, req.unit).await?;
    let current = state.class.clone();
    if current != SALE_CLASS {
        return Err(DbError::Conflict(format!(
            "only sale stock can be commissioned into the rental pool; this unit is {current}"
        )));
    }
    let reason = pool_reason(
        req.note.as_deref(),
        "commissioned into the rental pool",
        grade,
    );
    let movement = record_pool_move(
        conn,
        org_id,
        user_id,
        req.unit,
        SALE_CLASS,
        POOL_CLASS,
        &reason,
        grade,
        req.data_wipe_ref.as_deref(),
    )
    .await?;
    adjust_pool_quantity(conn, org_id, state.serial_id, -1000).await?;
    Ok(movement)
}

/// Decommission: the rental pool → sale stock. Refused while the unit is on an
/// open lease, because the ledger refuses it: gear that is out cannot be moved
/// out from under the agreement.
pub async fn decommission_from_pool(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    req: &PoolMoveRequest,
) -> DbResult<PoolMove> {
    let grade = req.condition_grade.trim();
    if !valid_grade(grade) {
        return Err(DbError::Conflict(
            "condition grade must be one of: good, fair, damaged".into(),
        ));
    }
    let state = pool_unit_classes(conn, org_id, req.unit).await?;
    let current = state.class.clone();
    if current != POOL_CLASS {
        return Err(DbError::Conflict(format!(
            "only rental-pool gear can be decommissioned; this unit is {current}"
        )));
    }
    let reason = pool_reason(
        req.note.as_deref(),
        "decommissioned from the rental pool",
        grade,
    );
    let movement = record_pool_move(
        conn,
        org_id,
        user_id,
        req.unit,
        POOL_CLASS,
        SALE_CLASS,
        &reason,
        grade,
        req.data_wipe_ref.as_deref(),
    )
    .await?;
    // The unit is saleable again: it re-enters the shelf's quantity, and the
    // credit re-arms the low-stock bell when it lifts the shelf above reorder.
    adjust_pool_quantity(conn, org_id, state.serial_id, 1000).await?;
    Ok(movement)
}

// --- lease agreements and their charges ---------------------------------------
//
// A lease is a `client_loans` row with `is_lease` (ADR 0035 §4: the loan
// vocabulary is the agreement). The charge is the fact, the invoice line is the
// document: the run writes one charge per billable period, cites it from the
// line through `source_kind = 'rental_charge'`, and the charge cites the
// agreement. A billed charge is immutable — corrections are credit notes.

/// One lease the invoice run can charge: only agreements that name a contract,
/// because that is what the run bills under, and only those with complete lease
/// terms.
#[derive(Debug, Clone)]
pub struct BillableLease {
    pub loan_id: Uuid,
    pub contract_id: Uuid,
    pub client_id: Uuid,
    pub currency: String,
    pub cycle: String,
    pub cycle_amount_minor: i64,
    pub label: String,
    pub active_from: NaiveDate,
    pub active_until: NaiveDate,
}

/// Every lease agreement that bills under one of `contract_ids`, in one query
/// (the run charges a whole sweep, so this is never called per agreement).
///
/// `active_until` is the return day when the unit came back and the expected
/// return otherwise: a returned lease stops billing on the day it ended, and an
/// open one bills to the day it is due. Extending a lease is a new expected
/// return on the agreement, not a guess here.
pub async fn list_billable_leases(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_ids: &[Uuid],
) -> DbResult<Vec<BillableLease>> {
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }
    type LeaseChargeTuple = (
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
        i64,
        String,
        NaiveDate,
        NaiveDate,
    );
    let rows: Vec<LeaseChargeTuple> = sqlx::query_as(
        r#"SELECT l.id, l.contract_id, l.client_id, l.lease_currency, l.lease_cycle,
                      l.lease_cost_minor,
                      COALESCE(NULLIF(btrim(l.agreement_ref), ''), 'lease for ' || c.name) AS label,
                      l.out_at::date AS active_from,
                      COALESCE(l.returned_at::date, l.expected_return_at) AS active_until
                 FROM client_loans l
                 JOIN clients c ON c.org_id = l.org_id AND c.id = l.client_id
                WHERE l.org_id = $1
                  AND l.is_lease
                  AND l.contract_id = ANY($2)
                  AND EXISTS (
                      SELECT 1 FROM contracts k
                       WHERE k.org_id = l.org_id AND k.id = l.contract_id
                         AND k.status <> 'cancelled'
                  )
                  AND l.lease_cycle IS NOT NULL
                  AND l.lease_cost_minor IS NOT NULL
                  AND l.lease_currency IS NOT NULL
                ORDER BY l.out_at, l.id"#,
    )
    .bind(org_id)
    .bind(contract_ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(
                loan_id,
                contract_id,
                client_id,
                currency,
                cycle,
                cycle_amount_minor,
                label,
                active_from,
                active_until,
            )| BillableLease {
                loan_id,
                contract_id,
                client_id,
                currency,
                cycle,
                cycle_amount_minor,
                label,
                active_from,
                active_until,
            },
        )
        .collect())
}

/// A charge about to be written. The id is decided by the caller because the
/// invoice line cites it before the charge row is inserted: the line is the
/// document, and a document cannot name a record that does not exist yet.
#[derive(Debug, Clone)]
pub struct NewRentalCharge {
    pub id: Uuid,
    pub loan_id: Uuid,
    pub contract_id: Uuid,
    pub client_id: Uuid,
    /// `rent | damage | loss`.
    pub kind: String,
    /// The loan line a one-off charge names; rent covers the agreement.
    pub source_line_id: Option<Uuid>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub days_billed: i64,
    pub period_days: i64,
    pub unit_amount_minor: i64,
    pub amount_minor: i64,
    pub currency: String,
    pub description: String,
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_amount_minor: Option<i64>,
}

/// A stored charge, as the reads return it.
#[derive(Debug, Clone)]
pub struct RentalCharge {
    pub id: Uuid,
    pub loan_id: Uuid,
    pub contract_id: Uuid,
    pub client_id: Uuid,
    pub kind: String,
    pub source_line_id: Option<Uuid>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub days_billed: i64,
    pub period_days: i64,
    pub unit_amount_minor: i64,
    pub amount_minor: i64,
    pub currency: String,
    pub description: String,
    pub status: String,
    pub invoice_id: Option<Uuid>,
    pub invoice_line_id: Option<Uuid>,
    pub billed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ChargeRow {
    id: Uuid,
    loan_id: Uuid,
    contract_id: Uuid,
    client_id: Uuid,
    kind: String,
    source_line_id: Option<Uuid>,
    period_start: NaiveDate,
    period_end: NaiveDate,
    days_billed: i32,
    period_days: i32,
    unit_amount_minor: i64,
    amount_minor: i64,
    currency: String,
    description: String,
    status: String,
    invoice_id: Option<Uuid>,
    invoice_line_id: Option<Uuid>,
    billed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

fn charge_from_row(r: ChargeRow) -> RentalCharge {
    RentalCharge {
        id: r.id,
        loan_id: r.loan_id,
        contract_id: r.contract_id,
        client_id: r.client_id,
        kind: r.kind,
        source_line_id: r.source_line_id,
        period_start: r.period_start,
        period_end: r.period_end,
        days_billed: r.days_billed as i64,
        period_days: r.period_days as i64,
        unit_amount_minor: r.unit_amount_minor,
        amount_minor: r.amount_minor,
        currency: r.currency,
        description: r.description,
        status: r.status,
        invoice_id: r.invoice_id,
        invoice_line_id: r.invoice_line_id,
        billed_at: r.billed_at,
        created_at: r.created_at,
    }
}

const CHARGE_COLS: &str = "id, loan_id, contract_id, client_id, kind, source_line_id,
     period_start, period_end, days_billed, period_days, unit_amount_minor, amount_minor,
     currency, description, status, invoice_id, invoice_line_id, billed_at, created_at";

/// Writes one charge as `pending`. The run flips it to `billed` with the line it
/// became; the append-only guard refuses anything else about it changing.
pub async fn insert_rental_charge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    c: &NewRentalCharge,
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO rental_charges
             (id, org_id, loan_id, contract_id, client_id, kind, source_line_id,
              period_start, period_end, days_billed, period_days, unit_amount_minor,
              amount_minor, currency, fx_rate_used, fx_rate_date, fx_source,
              base_amount_minor, description, status, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                   $15, $16, $17, $18, $19, 'pending', $20)"#,
    )
    .bind(c.id)
    .bind(org_id)
    .bind(c.loan_id)
    .bind(c.contract_id)
    .bind(c.client_id)
    .bind(&c.kind)
    .bind(c.source_line_id)
    .bind(c.period_start)
    .bind(c.period_end)
    .bind(c.days_billed as i32)
    .bind(c.period_days as i32)
    .bind(c.unit_amount_minor)
    .bind(c.amount_minor)
    .bind(&c.currency)
    .bind(c.fx_rate_used.as_deref())
    .bind(c.fx_rate_date)
    .bind(c.fx_source.as_deref())
    .bind(c.base_amount_minor)
    .bind(&c.description)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Marks the charges one run billed with the invoice line each became, in one
/// statement. The role of this call is the only UPDATE the guard admits.
pub async fn mark_rental_charges_billed(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    billed: &[(Uuid, Uuid, Uuid)],
) -> DbResult<()> {
    if billed.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = billed.iter().map(|b| b.0).collect();
    let invoices: Vec<Uuid> = billed.iter().map(|b| b.1).collect();
    let lines: Vec<Uuid> = billed.iter().map(|b| b.2).collect();
    let n = sqlx::query(
        r#"UPDATE rental_charges c
              SET status = 'billed', invoice_id = v.invoice_id,
                  invoice_line_id = v.line_id, billed_at = now()
             FROM (SELECT unnest($2::uuid[]) AS id,
                          unnest($3::uuid[]) AS invoice_id,
                          unnest($4::uuid[]) AS line_id) v
            WHERE c.org_id = $1 AND c.id = v.id AND c.status = 'pending'"#,
    )
    .bind(org_id)
    .bind(&ids)
    .bind(&invoices)
    .bind(&lines)
    .execute(&mut *conn)
    .await?;
    if n.rows_affected() as usize != ids.len() {
        return Err(DbError::Conflict(
            "a rental charge was already billed by another run".into(),
        ));
    }
    Ok(())
}

/// Every charge already written for these agreements, as `(loan_id, kind,
/// period_start, period_end, source_line_id)`. The run uses it to skip periods
/// and one-off charges it has already billed; the unique indexes stay the
/// concurrency backstop.
pub async fn existing_rental_charge_keys(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    loan_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String, NaiveDate, NaiveDate, Option<Uuid>)>> {
    if loan_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String, NaiveDate, NaiveDate, Option<Uuid>)> = sqlx::query_as(
        r#"SELECT loan_id, kind, period_start, period_end, source_line_id
             FROM rental_charges WHERE org_id = $1 AND loan_id = ANY($2)"#,
    )
    .bind(org_id)
    .bind(loan_ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Charges the run still owes a line for: every pending charge under these
/// contracts, one query. Damage and loss charges land here between the return
/// that raises them and the run that bills them.
pub async fn list_pending_rental_charges(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    contract_ids: &[Uuid],
) -> DbResult<Vec<RentalCharge>> {
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<ChargeRow> = sqlx::query_as(&format!(
        r#"SELECT {CHARGE_COLS} FROM rental_charges
            WHERE org_id = $1 AND contract_id = ANY($2) AND status = 'pending'
            ORDER BY period_start, id"#
    ))
    .bind(org_id)
    .bind(contract_ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(charge_from_row).collect())
}

/// Contract ids with a lease that can bill inside the window. A contract whose
/// term starts after the window is not in the run's ordinary contract list, yet
/// a lease that started earlier can have a period whose in-term days fall in
/// this window; the run needs its contract to bill those days.
pub async fn list_lease_contract_ids(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> DbResult<Vec<Uuid>> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT l.contract_id AS "contract_id!"
             FROM client_loans l
            WHERE l.org_id = $1
              AND l.is_lease
              AND EXISTS (
                  SELECT 1 FROM contracts k
                   WHERE k.org_id = l.org_id AND k.id = l.contract_id
                     AND k.status <> 'cancelled'
              )
              AND l.contract_id IS NOT NULL
              AND l.lease_cycle IS NOT NULL
              AND l.lease_cost_minor > 0
              AND l.lease_currency IS NOT NULL
              AND l.out_at::date <= $3
              AND COALESCE(l.returned_at::date, l.expected_return_at) >= $2"#,
        org_id,
        to,
        from,
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(|row| row.contract_id).collect())
}

/// Every pending charge in the org, whatever contract it bills under. The run
/// uses this so a charge raised under a contract its window does not select
/// still bills: a receivable does not expire with the window.
pub async fn list_all_pending_rental_charges(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<RentalCharge>> {
    let rows: Vec<ChargeRow> = sqlx::query_as(&format!(
        r#"SELECT {CHARGE_COLS} FROM rental_charges
            WHERE org_id = $1 AND status = 'pending'
            ORDER BY period_start, id"#
    ))
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(charge_from_row).collect())
}

/// The charges one agreement has produced, newest period first.
pub async fn list_rental_charges(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    loan_id: Uuid,
) -> DbResult<Vec<RentalCharge>> {
    let rows: Vec<ChargeRow> = sqlx::query_as(&format!(
        r#"SELECT {CHARGE_COLS} FROM rental_charges
            WHERE org_id = $1 AND loan_id = $2
            ORDER BY period_start DESC, id"#
    ))
    .bind(org_id)
    .bind(loan_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(charge_from_row).collect())
}

// --- vendor rentals (inbound) -------------------------------------------------
//
// The unit is the vendor's: a `device_assets` row with `ownership = rented` and
// class `rental_in`, so it never depreciates and never enters owned valuation
// (migration 0237's constraint, and `CustodyClass::counts_as_owned`). This table
// is its agreement — the terms, the expense, the deposit, and the return.

#[derive(Debug, Clone)]
pub struct NewRentalAsset {
    pub tag: String,
    pub name: String,
    pub kind: String,
}

pub struct VendorRentalRequest {
    /// An asset the procurement receipt already created, or a new one.
    pub device_asset_id: Option<Uuid>,
    pub new_asset: Option<NewRentalAsset>,
    pub vendor_id: Option<Uuid>,
    pub vendor_name: Option<String>,
    pub agreement_ref: Option<String>,
    pub start_on: NaiveDate,
    pub end_on: Option<NaiveDate>,
    pub cost_minor: i64,
    pub currency: String,
    pub cycle: String,
    pub deposit_minor: Option<i64>,
    pub deposit_currency: Option<String>,
    /// FX snapshot of the cycle cost when it is not in the org base currency.
    pub fx_rate_used: Option<String>,
    pub fx_rate_date: Option<NaiveDate>,
    pub fx_source: Option<String>,
    pub base_cost_minor: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VendorRental {
    pub id: Uuid,
    pub device_asset_id: Uuid,
    /// The rented unit's tag and name, read from the asset so a register row
    /// names the thing rather than an opaque id.
    pub asset_tag: String,
    pub asset_name: String,
    pub vendor_id: Option<Uuid>,
    pub vendor_name: Option<String>,
    pub agreement_ref: Option<String>,
    pub start_on: NaiveDate,
    pub end_on: Option<NaiveDate>,
    pub cost_minor: i64,
    pub currency: String,
    pub cycle: String,
    pub deposit_minor: Option<i64>,
    pub deposit_currency: Option<String>,
    pub base_cost_minor: Option<i64>,
    pub status: String,
    pub returned_at: Option<DateTime<Utc>>,
    pub return_condition: Option<String>,
    pub damage_minor: Option<i64>,
    pub lost: bool,
    pub loss_minor: Option<i64>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct VendorRow {
    id: Uuid,
    device_asset_id: Uuid,
    vendor_id: Option<Uuid>,
    vendor_name: Option<String>,
    agreement_ref: Option<String>,
    start_on: NaiveDate,
    end_on: Option<NaiveDate>,
    cost_minor: i64,
    currency: String,
    cycle: String,
    deposit_minor: Option<i64>,
    deposit_currency: Option<String>,
    base_cost_minor: Option<i64>,
    status: String,
    returned_at: Option<DateTime<Utc>>,
    return_condition: Option<String>,
    damage_minor: Option<i64>,
    lost: bool,
    loss_minor: Option<i64>,
    notes: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    asset_tag: String,
    asset_name: String,
}

fn vendor_from_row(r: VendorRow) -> VendorRental {
    VendorRental {
        id: r.id,
        device_asset_id: r.device_asset_id,
        vendor_id: r.vendor_id,
        vendor_name: r.vendor_name,
        agreement_ref: r.agreement_ref,
        start_on: r.start_on,
        end_on: r.end_on,
        cost_minor: r.cost_minor,
        currency: r.currency,
        cycle: r.cycle,
        deposit_minor: r.deposit_minor,
        deposit_currency: r.deposit_currency,
        base_cost_minor: r.base_cost_minor,
        status: r.status,
        returned_at: r.returned_at,
        return_condition: r.return_condition,
        damage_minor: r.damage_minor,
        lost: r.lost,
        loss_minor: r.loss_minor,
        notes: r.notes,
        created_at: r.created_at,
        updated_at: r.updated_at,
        asset_tag: r.asset_tag,
        asset_name: r.asset_name,
    }
}

const VENDOR_COLS: &str = "v.id, v.device_asset_id, v.vendor_id, v.vendor_name, v.agreement_ref,
     v.start_on, v.end_on, v.cost_minor, v.currency, v.cycle, v.deposit_minor,
     v.deposit_currency, v.base_cost_minor, v.status, v.returned_at,
     v.return_condition, v.damage_minor, v.lost, v.loss_minor, v.notes,
     v.created_at, v.updated_at, da.tag AS asset_tag, da.name AS asset_name";

/// Creates the agreement, and the asset when the caller has none: a unit rented
/// from a vendor is born in `rental_in` with `ownership = rented`, exactly the
/// way the receipt path lands one. It is never created `owned` and converted,
/// because a conversion is a class movement and a birth is not.
pub async fn create_vendor_rental(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    req: &VendorRentalRequest,
) -> DbResult<VendorRental> {
    if req.vendor_id.is_none()
        && req
            .vendor_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .is_none()
    {
        return Err(DbError::Conflict(
            "a vendor rental names the vendor or the vendor's own name for it".into(),
        ));
    }
    if !["monthly", "quarterly", "annual"].contains(&req.cycle.trim()) {
        return Err(DbError::Conflict(
            "rental cycle must be one of: monthly, quarterly, annual".into(),
        ));
    }
    if req.cost_minor < 0 {
        return Err(DbError::Conflict(
            "a rental expense cannot be negative".into(),
        ));
    }
    if let Some(end) = req.end_on {
        if end < req.start_on {
            return Err(DbError::Conflict(
                "the rental end must not precede its start".into(),
            ));
        }
    }
    if (req.deposit_minor.is_some()) != (req.deposit_currency.is_some()) {
        return Err(DbError::Conflict(
            "a deposit carries both its amount and its currency".into(),
        ));
    }
    if req.vendor_id.is_some() {
        let exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM vendors WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(req.vendor_id)
                .fetch_optional(&mut *conn)
                .await?;
        if exists.is_none() {
            return Err(DbError::NotFound);
        }
    }

    let asset_id = match (req.device_asset_id, &req.new_asset) {
        (Some(id), _) => {
            // Lock before reading the class: a return can commit between a
            // plain read and the agreement insert, and the agreement would then
            // land on a unit that is no longer rented in. The lock makes the
            // class below the committed one.
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT custody_class, ownership FROM device_assets \
                  WHERE org_id = $1 AND id = $2 FOR UPDATE",
            )
            .bind(org_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            let (class, ownership) = row.ok_or(DbError::NotFound)?;
            if class != "rental_in" || !matches!(ownership.as_str(), "leased" | "rented") {
                return Err(DbError::Conflict(format!(
                    "asset {id} is {class} ({ownership}); a vendor rental is rental_in gear owned by the vendor"
                )));
            }
            // A unit a closed agreement recorded as lost never came back. The
            // class stays rental_in because no terminal class names "lost", so
            // the record is the only thing that can stop a second agreement on
            // a unit nobody has.
            let lost: Option<(Uuid,)> = sqlx::query_as(
                r#"SELECT id FROM vendor_rentals
                    WHERE org_id = $1 AND device_asset_id = $2 AND lost
                    ORDER BY created_at DESC
                    LIMIT 1"#,
            )
            .bind(org_id)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            if lost.is_some() {
                return Err(DbError::Conflict(format!(
                    "asset {id} was recorded lost on a closed vendor rental; it cannot be rented again"
                )));
            }
            id
        }
        (None, Some(asset)) => {
            let id = Uuid::now_v7();
            sqlx::query(
                r#"INSERT INTO device_assets
                     (id, org_id, client_id, scope, tag, name, kind, status,
                      ownership, custody_class, depreciation_method)
                   VALUES ($1, $2, NULL, 'internal_org', $3, $4, $5, 'active',
                           'rented', 'rental_in', 'none')"#,
            )
            .bind(id)
            .bind(org_id)
            .bind(&asset.tag)
            .bind(&asset.name)
            .bind(&asset.kind)
            .execute(&mut *conn)
            .await?;
            id
        }
        (None, None) => {
            return Err(DbError::Conflict(
                "a vendor rental names the rented asset or describes one to create".into(),
            ))
        }
    };

    let id = Uuid::now_v7();
    let row: VendorRow = sqlx::query_as(&format!(
        r#"INSERT INTO vendor_rentals
             (id, org_id, device_asset_id, vendor_id, vendor_name, agreement_ref,
              start_on, end_on, cost_minor, currency, cycle, deposit_minor,
              deposit_currency, fx_rate_used, fx_rate_date, fx_source, base_cost_minor,
              notes, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                   $15, $16, $17, $18, $19)
           RETURNING {VENDOR_COLS}"#
    ))
    .bind(id)
    .bind(org_id)
    .bind(asset_id)
    .bind(req.vendor_id)
    .bind(req.vendor_name.as_deref())
    .bind(req.agreement_ref.as_deref())
    .bind(req.start_on)
    .bind(req.end_on)
    .bind(req.cost_minor)
    .bind(&req.currency)
    .bind(&req.cycle)
    .bind(req.deposit_minor)
    .bind(req.deposit_currency.as_deref())
    .bind(req.fx_rate_used.as_deref())
    .bind(req.fx_rate_date)
    .bind(req.fx_source.as_deref())
    .bind(req.base_cost_minor)
    .bind(req.notes.as_deref())
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(vendor_from_row(row))
}

pub async fn get_vendor_rental(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<VendorRental> {
    let row: Option<VendorRow> = sqlx::query_as(&format!(
        r#"SELECT {VENDOR_COLS} FROM vendor_rentals v
             JOIN device_assets da ON da.org_id = v.org_id AND da.id = v.device_asset_id
            WHERE v.org_id = $1 AND v.id = $2"#
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(vendor_from_row).ok_or(DbError::NotFound)
}

pub async fn list_vendor_rentals(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    status: Option<&str>,
    limit: i64,
) -> DbResult<Vec<VendorRental>> {
    let rows: Vec<VendorRow> = sqlx::query_as(&format!(
        r#"SELECT {VENDOR_COLS} FROM vendor_rentals v
             JOIN device_assets da ON da.org_id = v.org_id AND da.id = v.device_asset_id
            WHERE v.org_id = $1 AND ($2::text IS NULL OR v.status = $2)
            ORDER BY v.start_on DESC, v.id
            LIMIT $3"#
    ))
    .bind(org_id)
    .bind(status)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(vendor_from_row).collect())
}

pub struct VendorRentalReturn {
    /// `good | fair | damaged`; required unless the unit was lost.
    pub return_condition: Option<String>,
    pub lost: bool,
    pub damage_minor: Option<i64>,
    pub loss_minor: Option<i64>,
    pub notes: Option<String>,
}

/// Closes the agreement with what happened to the unit: its condition (or that
/// it never came back), the damage and the loss. A returned unit leaves custody
/// through a class movement to the terminal `returned` disposition; a lost one
/// keeps `rental_in`, because no terminal class in ADR 0035 names "lost" and
/// inventing one is not this lane's to do.
pub async fn return_vendor_rental(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    id: Uuid,
    req: &VendorRentalReturn,
) -> DbResult<VendorRental> {
    let rental = get_vendor_rental(conn, org_id, id).await?;
    if rental.status != "open" {
        return Err(DbError::Conflict(
            "this vendor rental is already closed".into(),
        ));
    }
    // The asset before the agreement row, the order every other path takes:
    // a create waiting on this asset blocks here, then reads the class the
    // return is about to change, instead of racing the agreement update.
    sqlx::query("SELECT 1 FROM device_assets WHERE org_id = $1 AND id = $2 FOR UPDATE")
        .bind(org_id)
        .bind(rental.device_asset_id)
        .execute(&mut *conn)
        .await?;
    let condition = req.return_condition.as_deref().map(str::trim);
    if req.lost {
        if condition.is_some() {
            return Err(DbError::Conflict(
                "a lost unit cannot also be graded; it did not come back".into(),
            ));
        }
    } else {
        let Some(condition) = condition else {
            return Err(DbError::Conflict(
                "a return grades the unit: good, fair or damaged".into(),
            ));
        };
        if !valid_grade(condition) {
            return Err(DbError::Conflict(
                "return condition must be one of: good, fair, damaged".into(),
            ));
        }
    }
    if req.loss_minor.is_some() && !req.lost {
        return Err(DbError::Conflict(
            "a loss charge needs the unit to be lost".into(),
        ));
    }
    if let Some(amount) = req.damage_minor {
        if amount < 0 {
            return Err(DbError::Conflict("damage cannot be negative".into()));
        }
    }
    if let Some(amount) = req.loss_minor {
        if amount < 0 {
            return Err(DbError::Conflict("a loss cannot be negative".into()));
        }
    }

    let row: Option<VendorRow> = sqlx::query_as(&format!(
        r#"UPDATE vendor_rentals
              SET status = 'returned', returned_at = now(), return_condition = $3,
                  damage_minor = $4, lost = $5, loss_minor = $6,
                  notes = COALESCE($7, notes), updated_at = now()
            WHERE org_id = $1 AND id = $2 AND status = 'open'
            RETURNING {VENDOR_COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .bind(condition)
    .bind(req.damage_minor)
    .bind(req.lost)
    .bind(req.loss_minor)
    .bind(req.notes.as_deref())
    .fetch_optional(&mut *conn)
    .await?;
    let rental = row
        .map(vendor_from_row)
        .ok_or_else(|| DbError::Conflict("this vendor rental is already closed".into()))?;

    if !req.lost {
        // The unit goes back to the vendor: terminal `returned`, with the
        // agreement as the reason it left. Written after the agreement closes,
        // because the ledger refuses a move while an open agreement holds it.
        let reason = format!(
            "vendor rental {id} closed; unit returned to vendor ({})",
            condition.unwrap_or("good")
        );
        record_pool_move(
            conn,
            org_id,
            user_id,
            PoolUnit {
                side: UnitSide::Asset,
                id: rental.device_asset_id,
            },
            "rental_in",
            "returned",
            &reason.chars().take(500).collect::<String>(),
            condition.unwrap_or("good"),
            None,
        )
        .await?;
    }
    Ok(rental)
}

// --- reading the pool ---------------------------------------------------------

/// One unit in the owned rental pool. A unit is one row: a serial that has an
/// asset projection is listed by its serial, and an asset that has no serial is
/// listed by itself, so the pool never counts the same unit twice.
#[derive(Debug, Clone)]
pub struct PoolUnitRow {
    pub unit_kind: String,
    pub id: Uuid,
    pub label: String,
    pub stock_item_id: Option<Uuid>,
    pub device_asset_id: Option<Uuid>,
    /// False while an open client agreement, vendor rental, or ticket holds it.
    pub available: bool,
}

/// Every unit in the pool, one query, with availability derived from the open
/// agreements that hold it. Availability is never stored: an agreement closing
/// makes the unit available in the same instant.
pub async fn list_pool_units(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    available_only: bool,
    limit: i64,
) -> DbResult<Vec<PoolUnitRow>> {
    // Availability is derived from the open agreements that hold a unit, never
    // stored: the moment an agreement closes, the unit is available again.
    type PoolUnitTuple = (String, Uuid, String, Option<Uuid>, Option<Uuid>, bool);
    let rows: Vec<PoolUnitTuple> = sqlx::query_as(
        r#"SELECT unit_kind, id, label, stock_item_id, device_asset_id, available
             FROM (
               SELECT 'serial'::text AS unit_kind, s.id, s.serial_number AS label,
                      s.stock_item_id, s.device_asset_id,
                      NOT EXISTS (
                        SELECT 1 FROM client_loan_lines l
                          JOIN client_loans h ON h.id = l.loan_id
                         WHERE h.org_id = s.org_id AND l.returned_at IS NULL
                           AND (l.serial_id = s.id
                                OR l.device_asset_id = s.device_asset_id)
                      )
                      AND NOT EXISTS (
                        SELECT 1 FROM vendor_rentals v
                         WHERE v.org_id = s.org_id AND v.status = 'open'
                           AND v.device_asset_id = s.device_asset_id
                      )
                      AND s.ticket_id IS NULL AS available
                 FROM stock_serials s
                WHERE s.org_id = $1 AND s.custody_class = 'rental_out'
               UNION ALL
               SELECT 'asset'::text, a.id, a.tag, NULL, a.id,
                      NOT EXISTS (
                        SELECT 1 FROM client_loan_lines l
                          JOIN client_loans h ON h.id = l.loan_id
                         WHERE h.org_id = a.org_id AND l.returned_at IS NULL
                           AND l.device_asset_id = a.id
                      )
                      AND NOT EXISTS (
                        SELECT 1 FROM vendor_rentals v
                         WHERE v.org_id = a.org_id AND v.status = 'open'
                           AND v.device_asset_id = a.id
                      ) AS available
                 FROM device_assets a
                WHERE a.org_id = $1 AND a.custody_class = 'rental_out'
                  AND a.serial_id IS NULL
             ) pool
            WHERE (NOT $2::boolean OR available)
            ORDER BY label
            LIMIT $3"#,
    )
    .bind(org_id)
    .bind(available_only)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| PoolUnitRow {
            unit_kind: r.0,
            id: r.1,
            label: r.2,
            stock_item_id: r.3,
            device_asset_id: r.4,
            available: r.5,
        })
        .collect())
}
