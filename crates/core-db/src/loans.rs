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

use crate::stock::{LedgerInput, MovementLeg};
use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{ClientLoan, ClientLoanLine, StockMovement};
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
        ..Default::default()
    }
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
    if let Some(contract_id) = req.contract_id {
        crate::contracts::get_contract(&mut *conn, org_id, contract_id).await?;
    }
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
                    let row = sqlx::query!(
                        r#"SELECT id FROM stock_serials
                           WHERE org_id = $1 AND stock_item_id = $2 AND status = 'in_stock'
                           ORDER BY created_at, id
                           LIMIT 1"#,
                        org_id,
                        stock_item.id,
                    )
                    .fetch_optional(&mut *conn)
                    .await?;
                    match row {
                        Some(row) => crate::stock::get_serial(&mut *conn, org_id, row.id).await?,
                        None => {
                            return Err(DbError::Conflict(format!(
                                "no in-stock unit of {} to issue",
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
            let from_location_id = serial.location_id.ok_or_else(|| {
                DbError::Conflict(format!(
                    "serial {} has no location; it is not in stock",
                    serial.serial_number
                ))
            })?;

            // 1. The ledger leg (client destination, not a location).
            let movement = StockMovement {
                id: Uuid::now_v7(),
                org_id,
                stock_item_id: stock_item.id,
                kind: "loan".into(),
                from_location_id: Some(from_location_id),
                to_location_id: None,
                qty_milli: -1000,
                cost_minor: None,
                currency: None,
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_cost_minor: None,
                serial_id: Some(serial.id),
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
                    item_name: stock_item.name.clone(),
                    item_kind: "serialized".into(),
                    serial_number: None,
                },
            )
            .await?;
            // 2. WHERE-IT-IS-NOW: with the client (the tick flipped it to
            // `out`; the loan state is the richer consequence).
            let n = sqlx::query!(
                r#"UPDATE stock_serials
                     SET status = 'loaned', client_id = $3, updated_at = now()
                   WHERE org_id = $1 AND id = $2 AND status = 'out'"#,
                org_id,
                serial.id,
                loan.client_id,
            )
            .execute(&mut *conn)
            .await?;
            if n.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "serial {} could not be issued (already out)",
                    serial.serial_number
                )));
            }

            // 3. The device's row in the client's fleet, identity-keyed so
            // a later M365/RMM feed converges instead of twinning.
            let asset_id = Uuid::now_v7();
            let tag = format!("LOAN-{}", asset_id.simple().to_string()[..8].to_uppercase());
            let fields = serde_json::json!({ "serial_number": serial.serial_number });
            crate::assets::create_asset(
                &mut *conn,
                org_id,
                asset_id,
                Some(loan.client_id),
                "client",
                None,
                &tag,
                &stock_item.name,
                "device",
                "active",
                None,
                &fields,
            )
            .await?;
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
                Some(outcome.movement.id),
                now,
            )
            .await
            .map_err(|e| match e {
                DbError::Sql(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                    DbError::Conflict(format!(
                        "serial {} is already on an open loan",
                        serial.serial_number
                    ))
                }
                other => other,
            })?;
            legs.push(MovementLeg {
                movement: outcome.movement,
                low: outcome.low,
            });
            lines.push(line);
        } else if let Some(device_asset_id) = item.device_asset_id {
            let asset = crate::assets::get_asset(&mut *conn, org_id, device_asset_id).await?;
            if loan.is_lease {
                crate::assets::update_asset_lifecycle(
                    &mut *conn,
                    org_id,
                    device_asset_id,
                    &lease_patch(req, out_date),
                )
                .await?;
            }
            let line =
                insert_line(conn, org_id, loan.id, None, None, Some(asset.id), None, now).await?;
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
        if let (Some(serial_id), Some(stock_item_id)) = (line.serial_id, line.stock_item_id) {
            let location_id = to_location_id.expect("required for stock-sourced lines");
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
            // The lane-1 tick only arms serials that are `in_stock`; a
            // `loaned` unit re-arms here, in the same commit.
            let n = sqlx::query!(
                r#"UPDATE stock_serials
                     SET status = 'in_stock', location_id = $3, client_id = NULL,
                         movement_in_id = $4, updated_at = now()
                   WHERE org_id = $1 AND id = $2 AND status = 'loaned'"#,
                org_id,
                serial_id,
                location_id,
                outcome.movement.id,
            )
            .execute(&mut *conn)
            .await?;
            if n.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "unit on line {} is not on loan",
                    t.line_id
                )));
            }
            movement_in_id = Some(outcome.movement.id);
            legs.push(MovementLeg {
                movement: outcome.movement,
                low: outcome.low,
            });
        }
        if loan.is_lease {
            // Lease closed at the ACTUAL return date (expected return was
            // the plan; the asset carries what happened). Applies to both
            // sources: a stock-sourced device's instantiated asset closes
            // exactly like an existing fleet asset.
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
