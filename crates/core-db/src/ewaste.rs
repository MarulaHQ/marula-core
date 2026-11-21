//! E-waste custody persistence (ADR 0035 decision 5, migration 0238).
//!
//! Client e-waste is held, moved and disposed of; it is never stock. This
//! module writes four tables and reads their balance:
//!
//!   * `ewaste_intakes` — the receipt of custody, open until every gram has a
//!     pathway.
//!   * `ewaste_intake_lines` — a serialized unit or a lot, with its weight in
//!     integer grams and its destruction state.
//!   * `ewaste_dispositions` — the way out, one pathway per disposition, with
//!     the partner, date, weight, certificate and attached document.
//!   * `ewaste_events` — the append-only custody ledger.
//!
//! Every guard lives in migration 0238 as well, because a concurrent writer
//! can bypass any check made here. This module keeps the typed refusal: a
//! `RAISE EXCEPTION` from a guard arrives as SQLSTATE `P0001`, which
//! [`guard_error`] turns into [`DbError::Conflict`] with the guard's message,
//! so the API answers 409 instead of 500. The type-level defences the guards
//! carry are the point — a balance checked in two round trips would still race
//! a second disposition.
//!
//! Nothing in this module touches `stock_levels`, `stock_movements`, or a
//! valuation. The only unit tables written are the class-movement ledger and
//! the projections its triggers already own (a client asset retiring into
//! custody, a disposed unit leaving it), and those are written by migration
//! 0238's triggers rather than by a query here.
//!
//! Queries are runtime `sqlx::query`/`query_as` with explicit row mapping. This
//! is a deliberate deviation from the `sqlx::query!` macro the `.coderabbit`
//! path instruction asks for, and it follows the modules that are shaped the
//! same way (`assets`, `document_links`, `document_layouts`): the list query's
//! optional filter arms make its SQL text dynamic, and there is no compile-time
//! cache entry to prepare for four tables that this lane's migration creates.
//! The migration is the authority on every guard; a macro would check the
//! column names and prove nothing about the triggers.

use crate::tickets::ClientVisibility;
use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, Row};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The entity family the intake's write sequence lives under. Routes claim and
/// record with the same name, and the read paths below use it to return the
/// version a client needs for a conditional write.
pub const INTAKE_ENTITY: &str = "ewaste_intake";

/// Intake statuses. `open` is the only state that accepts a line, event or
/// disposition; `closed` is the balanced end of the receipt.
pub const INTAKE_STATUSES: [&str; 2] = ["open", "closed"];

/// A line is one serialized unit or one lot.
pub const LINE_KINDS: [&str; 2] = ["serial", "lot"];

/// Destruction state on a data-bearing line. `not_required` on a data-bearing
/// line means the client did not ask for destruction; `pending` blocks every
/// disposition.
pub const DESTRUCTION_STATES: [&str; 3] = ["not_required", "pending", "completed"];

/// Every way a gram can leave custody.
pub const PATHWAYS: [&str; 5] = [
    "recycled",
    "destroyed",
    "resold",
    "returned_to_client",
    "transferred_to_partner",
];

/// Pathways that name a downstream custodian and therefore need a certificate.
pub const CERTIFICATED_PATHWAYS: [&str; 3] = ["recycled", "destroyed", "transferred_to_partner"];

/// The custody ledger's vocabulary. `intake` and `disposition` are written by
/// the record that produces them; a caller records only the middle three.
pub const EVENT_KINDS: [&str; 5] = ["intake", "store", "move", "handoff", "disposition"];

/// The event kinds a caller may write directly.
pub const CALLER_EVENT_KINDS: [&str; 3] = ["store", "move", "handoff"];

/// Where e-waste sits while it is with the organisation.
pub const STORAGE_CLASSES: [&str; 6] = [
    "quarantine",
    "secure_cage",
    "warehouse",
    "van",
    "client_site",
    "partner_site",
];

/// Hazard flags the receiving bench can tick (migration 0238 CHECK).
pub const HAZARD_FLAGS: [&str; 7] = [
    "battery",
    "cracked",
    "liquid_damage",
    "biohazard",
    "loose_media",
    "pressurised",
    "unlabeled",
];

pub fn is_valid_intake_status(value: &str) -> bool {
    INTAKE_STATUSES.contains(&value.trim())
}

pub fn is_valid_line_kind(value: &str) -> bool {
    LINE_KINDS.contains(&value.trim())
}

pub fn is_valid_destruction_state(value: &str) -> bool {
    DESTRUCTION_STATES.contains(&value.trim())
}

pub fn is_valid_pathway(value: &str) -> bool {
    PATHWAYS.contains(&value.trim())
}

pub fn is_valid_caller_event_kind(value: &str) -> bool {
    CALLER_EVENT_KINDS.contains(&value.trim())
}

pub fn is_valid_storage_class(value: &str) -> bool {
    STORAGE_CLASSES.contains(&value.trim())
}

pub fn is_valid_hazard_flag(value: &str) -> bool {
    HAZARD_FLAGS.contains(&value.trim())
}

pub fn pathway_needs_certificate(value: &str) -> bool {
    CERTIFICATED_PATHWAYS.contains(&value.trim())
}

/// A refusal raised by a migration-0238 guard becomes a typed conflict so the
/// API answers 409 with the guard's own sentence. Every other database error
/// keeps its own shape.
fn guard_error(e: sqlx::Error) -> DbError {
    if let sqlx::Error::Database(db) = &e {
        if db.code().as_deref() == Some("P0001") {
            let message = db.message();
            let cleaned = message.strip_prefix("ewaste: ").unwrap_or(message);
            return DbError::Conflict(cleaned.to_string());
        }
    }
    DbError::Sql(e)
}

fn row_datetime(row: &sqlx::postgres::PgRow, column: &str) -> DbResult<DateTime<Utc>> {
    row.try_get::<DateTime<Utc>, _>(column)
        .map_err(DbError::Sql)
}

fn row_date(row: &sqlx::postgres::PgRow, column: &str) -> DbResult<NaiveDate> {
    row.try_get::<NaiveDate, _>(column).map_err(DbError::Sql)
}

// Row shapes -----------------------------------------------------------------

/// A receipt of custody. The client owns the goods; the organisation holds
/// them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EwasteIntake {
    pub id: Uuid,
    pub client_id: Uuid,
    pub site_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub reference: Option<String>,
    pub received_at: DateTime<Utc>,
    pub received_by_name: String,
    pub received_by_user_id: Option<Uuid>,
    pub signature_document_id: Option<Uuid>,
    pub document_reference: Option<String>,
    pub data_destruction_required: bool,
    pub hazard_flags: Vec<String>,
    pub notes: Option<String>,
    pub status: String,
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EwasteIntake {
    pub fn is_open(&self) -> bool {
        self.status == "open"
    }
}

/// One line: a serialized unit or a lot, with its weight in integer grams.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EwasteIntakeLine {
    pub id: Uuid,
    pub intake_id: Uuid,
    pub line_kind: String,
    pub serial_id: Option<Uuid>,
    pub device_asset_id: Option<Uuid>,
    pub lot_label: Option<String>,
    pub category: String,
    pub weight_g: i64,
    pub quantity: i32,
    pub data_bearing: bool,
    pub destruction_status: String,
    pub destruction_completed_at: Option<DateTime<Utc>>,
    pub destruction_completed_by: Option<Uuid>,
    pub destruction_document_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Sum of this line's dispositions. Derived on read, never stored.
    pub disposed_weight_g: i64,
}

impl EwasteIntakeLine {
    /// Weight still in custody on this line.
    pub fn remaining_weight_g(&self) -> i64 {
        self.weight_g.saturating_sub(self.disposed_weight_g)
    }

    pub fn destruction_pending(&self) -> bool {
        self.destruction_status == "pending"
    }
}

/// A recorded way out of custody.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EwasteDisposition {
    pub id: Uuid,
    pub intake_id: Uuid,
    pub line_id: Uuid,
    pub pathway: String,
    pub partner_name: Option<String>,
    pub disposed_at: NaiveDate,
    pub weight_g: i64,
    pub certificate_reference: Option<String>,
    pub document_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// One append-only custody event.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EwasteEvent {
    pub id: Uuid,
    pub intake_id: Uuid,
    pub line_id: Option<Uuid>,
    pub event_kind: String,
    pub occurred_at: DateTime<Utc>,
    pub storage_class: Option<String>,
    pub location_note: Option<String>,
    pub partner_name: Option<String>,
    pub disposition_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Weight moved by one pathway.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PathwayWeight {
    pub pathway: String,
    pub weight_g: i64,
}

/// The intake's weight accounting: everything received, everything that has a
/// pathway, and what is left. `remaining_g` must reach zero before the intake
/// closes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WeightBalance {
    pub total_in_g: i64,
    pub total_out_g: i64,
    pub remaining_g: i64,
    pub by_pathway: Vec<PathwayWeight>,
}

/// An intake as the list view sees it, with its weights aggregated in one
/// query rather than one query per row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IntakeSummary {
    pub intake: EwasteIntake,
    pub line_count: i64,
    pub total_in_g: i64,
    pub total_out_g: i64,
    pub remaining_g: i64,
    /// The intake's write-sequence version, for a conditional close.
    pub version: i64,
}

// Inputs --------------------------------------------------------------------

/// A new line on an intake.
#[derive(Debug, Clone)]
pub struct NewIntakeLine {
    pub line_kind: String,
    pub serial_id: Option<Uuid>,
    pub device_asset_id: Option<Uuid>,
    pub lot_label: Option<String>,
    pub category: String,
    pub weight_g: i64,
    pub quantity: i32,
    pub data_bearing: bool,
}

/// A new receipt of custody with its lines.
#[derive(Debug, Clone)]
pub struct NewIntake {
    pub client_id: Uuid,
    pub site_id: Option<Uuid>,
    pub ticket_id: Option<Uuid>,
    pub reference: Option<String>,
    pub received_at: DateTime<Utc>,
    pub received_by_name: String,
    pub received_by_user_id: Option<Uuid>,
    pub signature_document_id: Option<Uuid>,
    pub document_reference: Option<String>,
    pub data_destruction_required: bool,
    pub hazard_flags: Vec<String>,
    pub notes: Option<String>,
    pub lines: Vec<NewIntakeLine>,
}

/// A custody event a caller records.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub line_id: Option<Uuid>,
    pub event_kind: String,
    pub occurred_at: Option<DateTime<Utc>>,
    pub storage_class: Option<String>,
    pub location_note: Option<String>,
    pub partner_name: Option<String>,
    pub actor_user_id: Option<Uuid>,
    pub notes: Option<String>,
}

/// A new way out for a line.
#[derive(Debug, Clone)]
pub struct NewDisposition {
    pub line_id: Uuid,
    pub pathway: String,
    pub partner_name: Option<String>,
    pub disposed_at: NaiveDate,
    pub weight_g: i64,
    pub certificate_reference: Option<String>,
    pub document_id: Option<Uuid>,
}

/// Filters for the intake list.
#[derive(Debug, Clone, Default)]
pub struct IntakeFilter {
    pub status: Option<String>,
    pub client_id: Option<Uuid>,
    pub limit: i64,
    /// Whose client restrictions apply. `None` is an internal caller with no
    /// visibility gate; every route passes the caller's.
    pub visibility: Option<ClientVisibility>,
}

/// Filters for the org-wide disposition list.
#[derive(Debug, Clone, Default)]
pub struct DispositionFilter {
    pub client_id: Option<Uuid>,
    pub pathway: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub limit: i64,
    /// Keyset cursor over the register's order: the last row's
    /// `(disposed_at, created_at, id)`. `None` starts at the newest.
    pub cursor: Option<(NaiveDate, DateTime<Utc>, Uuid)>,
    /// Whose client restrictions apply. `None` is an internal caller with no
    /// visibility gate; every route passes the caller's.
    pub visibility: Option<ClientVisibility>,
}

/// The restriction predicate over the intake's client row. A live client with
/// a non-empty `restricted_to` that omits the caller hides the row; the
/// owner/admin bypass sees everything, and a client row that no longer exists
/// has no restriction left to apply, so a retained receipt stays readable.
/// Bind slots are arguments because each statement numbers its own.
fn visible_to_caller(admin_bind: usize, user_bind: usize, client_alias: &str) -> String {
    format!(
        "AND (${admin_bind}::boolean OR NOT EXISTS ( \
              SELECT 1 FROM clients c \
               WHERE c.id = {client_alias}.client_id \
                 AND c.org_id = {client_alias}.org_id \
                 AND jsonb_array_length(c.restricted_to) > 0 \
                 AND NOT (c.restricted_to ? ${user_bind}::text)))"
    )
}

/// The two binds [`visible_to_caller`] needs. `None` is an internal caller with
/// no gate, expressed as the admin bypass so the statement text stays static.
fn visibility_binds(visibility: Option<ClientVisibility>) -> (bool, Uuid) {
    match visibility {
        Some(v) => (v.is_admin, v.user_id),
        None => (true, Uuid::nil()),
    }
}

// Mapping --------------------------------------------------------------------

fn intake_from_row(row: &sqlx::postgres::PgRow) -> DbResult<EwasteIntake> {
    Ok(EwasteIntake {
        id: row.try_get("id").map_err(DbError::Sql)?,
        client_id: row.try_get("client_id").map_err(DbError::Sql)?,
        site_id: row.try_get("site_id").map_err(DbError::Sql)?,
        ticket_id: row.try_get("ticket_id").map_err(DbError::Sql)?,
        reference: row.try_get("reference").map_err(DbError::Sql)?,
        received_at: row_datetime(row, "received_at")?,
        received_by_name: row.try_get("received_by_name").map_err(DbError::Sql)?,
        received_by_user_id: row.try_get("received_by_user_id").map_err(DbError::Sql)?,
        signature_document_id: row.try_get("signature_document_id").map_err(DbError::Sql)?,
        document_reference: row.try_get("document_reference").map_err(DbError::Sql)?,
        data_destruction_required: row
            .try_get("data_destruction_required")
            .map_err(DbError::Sql)?,
        hazard_flags: row.try_get("hazard_flags").map_err(DbError::Sql)?,
        notes: row.try_get("notes").map_err(DbError::Sql)?,
        status: row.try_get("status").map_err(DbError::Sql)?,
        closed_at: row.try_get("closed_at").map_err(DbError::Sql)?,
        closed_by: row.try_get("closed_by").map_err(DbError::Sql)?,
        created_by: row.try_get("created_by").map_err(DbError::Sql)?,
        created_at: row_datetime(row, "created_at")?,
        updated_at: row_datetime(row, "updated_at")?,
    })
}

fn line_from_row(row: &sqlx::postgres::PgRow) -> DbResult<EwasteIntakeLine> {
    Ok(EwasteIntakeLine {
        id: row.try_get("id").map_err(DbError::Sql)?,
        intake_id: row.try_get("intake_id").map_err(DbError::Sql)?,
        line_kind: row.try_get("line_kind").map_err(DbError::Sql)?,
        serial_id: row.try_get("serial_id").map_err(DbError::Sql)?,
        device_asset_id: row.try_get("device_asset_id").map_err(DbError::Sql)?,
        lot_label: row.try_get("lot_label").map_err(DbError::Sql)?,
        category: row.try_get("category").map_err(DbError::Sql)?,
        weight_g: row.try_get("weight_g").map_err(DbError::Sql)?,
        quantity: row.try_get("quantity").map_err(DbError::Sql)?,
        data_bearing: row.try_get("data_bearing").map_err(DbError::Sql)?,
        destruction_status: row.try_get("destruction_status").map_err(DbError::Sql)?,
        destruction_completed_at: row
            .try_get("destruction_completed_at")
            .map_err(DbError::Sql)?,
        destruction_completed_by: row
            .try_get("destruction_completed_by")
            .map_err(DbError::Sql)?,
        destruction_document_id: row
            .try_get("destruction_document_id")
            .map_err(DbError::Sql)?,
        created_by: row.try_get("created_by").map_err(DbError::Sql)?,
        created_at: row_datetime(row, "created_at")?,
        updated_at: row_datetime(row, "updated_at")?,
        disposed_weight_g: row.try_get("disposed_weight_g").map_err(DbError::Sql)?,
    })
}

fn disposition_from_row(row: &sqlx::postgres::PgRow) -> DbResult<EwasteDisposition> {
    Ok(EwasteDisposition {
        id: row.try_get("id").map_err(DbError::Sql)?,
        intake_id: row.try_get("intake_id").map_err(DbError::Sql)?,
        line_id: row.try_get("line_id").map_err(DbError::Sql)?,
        pathway: row.try_get("pathway").map_err(DbError::Sql)?,
        partner_name: row.try_get("partner_name").map_err(DbError::Sql)?,
        disposed_at: row_date(row, "disposed_at")?,
        weight_g: row.try_get("weight_g").map_err(DbError::Sql)?,
        certificate_reference: row.try_get("certificate_reference").map_err(DbError::Sql)?,
        document_id: row.try_get("document_id").map_err(DbError::Sql)?,
        created_by: row.try_get("created_by").map_err(DbError::Sql)?,
        created_at: row_datetime(row, "created_at")?,
    })
}

fn event_from_row(row: &sqlx::postgres::PgRow) -> DbResult<EwasteEvent> {
    Ok(EwasteEvent {
        id: row.try_get("id").map_err(DbError::Sql)?,
        intake_id: row.try_get("intake_id").map_err(DbError::Sql)?,
        line_id: row.try_get("line_id").map_err(DbError::Sql)?,
        event_kind: row.try_get("event_kind").map_err(DbError::Sql)?,
        occurred_at: row_datetime(row, "occurred_at")?,
        storage_class: row.try_get("storage_class").map_err(DbError::Sql)?,
        location_note: row.try_get("location_note").map_err(DbError::Sql)?,
        partner_name: row.try_get("partner_name").map_err(DbError::Sql)?,
        disposition_id: row.try_get("disposition_id").map_err(DbError::Sql)?,
        actor_user_id: row.try_get("actor_user_id").map_err(DbError::Sql)?,
        notes: row.try_get("notes").map_err(DbError::Sql)?,
        created_at: row_datetime(row, "created_at")?,
    })
}

const INTAKE_COLUMNS: &str = "id, client_id, site_id, ticket_id, reference, \
    received_at, received_by_name, received_by_user_id, signature_document_id, \
    document_reference, data_destruction_required, hazard_flags, notes, status, \
    closed_at, closed_by, created_by, created_at, updated_at";

// Writes ---------------------------------------------------------------------

/// Records one receipt of custody and its lines. The intake's arrival event
/// and the retirement of any named client asset are written by migration
/// 0238's triggers inside this transaction.
pub async fn create_intake(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
    input: &NewIntake,
) -> DbResult<EwasteIntake> {
    sqlx::query(
        r#"INSERT INTO ewaste_intakes (
             id, org_id, client_id, site_id, ticket_id, reference, received_at,
             received_by_name, received_by_user_id, signature_document_id,
             document_reference, data_destruction_required, hazard_flags, notes,
             created_by
           ) VALUES (
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
           )"#,
    )
    .bind(intake_id)
    .bind(org_id)
    .bind(input.client_id)
    .bind(input.site_id)
    .bind(input.ticket_id)
    .bind(input.reference.as_deref())
    .bind(input.received_at)
    .bind(input.received_by_name.trim())
    .bind(input.received_by_user_id.or(Some(user_id)))
    .bind(input.signature_document_id)
    .bind(input.document_reference.as_deref())
    .bind(input.data_destruction_required)
    .bind(input.hazard_flags.as_slice())
    .bind(input.notes.as_deref())
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;

    lock_units(conn, org_id, &input.lines).await?;
    for line in &input.lines {
        insert_line(conn, org_id, user_id, intake_id, line).await?;
    }

    get_intake(conn, org_id, intake_id).await
}

/// Lock every unit a receipt names, in one canonical order, before any line is
/// inserted. Two receipts that list the same units in opposite order then queue
/// on the same first lock instead of deadlocking. The per-line guard still
/// re-reads each pair under its own locks and refuses a pair that changed, so
/// resolving the pair here is an ordering aid, not the authority.
async fn lock_units(
    conn: &mut PgConnection,
    org_id: Uuid,
    lines: &[NewIntakeLine],
) -> DbResult<()> {
    let mut serials: BTreeSet<Uuid> = BTreeSet::new();
    let mut assets: BTreeSet<Uuid> = BTreeSet::new();
    for line in lines {
        if let Some(serial_id) = line.serial_id {
            serials.insert(serial_id);
        }
        if let Some(asset_id) = line.device_asset_id {
            assets.insert(asset_id);
        }
    }
    if serials.is_empty() && assets.is_empty() {
        return Ok(());
    }

    // Resolve both projections, so a serial line and an asset-only line for the
    // same unit join the same lock set.
    let serial_ids: Vec<Uuid> = serials.iter().copied().collect();
    let pairs = sqlx::query!(
        "SELECT device_asset_id FROM stock_serials WHERE org_id = $1 AND id = ANY($2)",
        org_id,
        &serial_ids
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    for pair in pairs {
        if let Some(asset_id) = pair.device_asset_id {
            assets.insert(asset_id);
        }
    }

    let asset_ids: Vec<Uuid> = assets.iter().copied().collect();
    let pairs = sqlx::query!(
        "SELECT serial_id FROM device_assets WHERE org_id = $1 AND id = ANY($2)",
        org_id,
        &asset_ids
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    for pair in pairs {
        if let Some(serial_id) = pair.serial_id {
            serials.insert(serial_id);
        }
    }

    // Assets first (0232's order), then serials, each class in id order.
    let asset_ids: Vec<Uuid> = assets.into_iter().collect();
    if !asset_ids.is_empty() {
        sqlx::query!(
            "SELECT id FROM device_assets WHERE org_id = $1 AND id = ANY($2) ORDER BY id FOR UPDATE",
            org_id,
            &asset_ids
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(guard_error)?;
    }
    let serial_ids: Vec<Uuid> = serials.into_iter().collect();
    if !serial_ids.is_empty() {
        sqlx::query!(
            "SELECT id FROM stock_serials WHERE org_id = $1 AND id = ANY($2) ORDER BY id FOR UPDATE",
            org_id,
            &serial_ids
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(guard_error)?;
    }
    Ok(())
}

async fn insert_line(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
    input: &NewIntakeLine,
) -> DbResult<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO ewaste_intake_lines (
             id, org_id, intake_id, line_kind, serial_id, device_asset_id,
             lot_label, category, weight_g, quantity, data_bearing, created_by
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(intake_id)
    .bind(input.line_kind.trim())
    .bind(input.serial_id)
    .bind(input.device_asset_id)
    .bind(input.lot_label.as_deref().map(str::trim))
    .bind(input.category.trim())
    .bind(input.weight_g)
    .bind(input.quantity)
    .bind(input.data_bearing)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;
    Ok(id)
}

/// Adds one line to an open intake.
pub async fn add_line(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
    input: &NewIntakeLine,
) -> DbResult<Uuid> {
    insert_line(conn, org_id, user_id, intake_id, input).await
}

/// Records a store, move or handoff event. Intake and disposition events are
/// refused here because the records that produce them write their own.
pub async fn record_event(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
    input: &NewEvent,
) -> DbResult<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO ewaste_events (
             id, org_id, intake_id, line_id, event_kind, occurred_at,
             storage_class, location_note, partner_name, actor_user_id, notes
           ) VALUES (
             $1, $2, $3, $4, $5, coalesce($6, now()), $7, $8, $9, $10, $11
           )"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(intake_id)
    .bind(input.line_id)
    .bind(input.event_kind.trim())
    .bind(input.occurred_at)
    .bind(input.storage_class.as_deref().map(str::trim))
    .bind(input.location_note.as_deref())
    .bind(input.partner_name.as_deref().map(str::trim))
    .bind(input.actor_user_id)
    .bind(input.notes.as_deref())
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;
    Ok(id)
}

/// Moves a data-bearing line's destruction state. `completed` records the
/// evidence and the moment; `pending` withdraws a completion that no
/// disposition has relied on yet.
pub async fn set_destruction_state(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
    line_id: Uuid,
    state: &str,
    document_id: Option<Uuid>,
) -> DbResult<()> {
    let data_bearing: Option<bool> = sqlx::query_scalar(
        r#"SELECT data_bearing FROM ewaste_intake_lines
            WHERE id = $1 AND org_id = $2 AND intake_id = $3"#,
    )
    .bind(line_id)
    .bind(org_id)
    .bind(intake_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(guard_error)?;

    let data_bearing = data_bearing.ok_or(DbError::NotFound)?;
    if !data_bearing && state != "not_required" {
        return Err(DbError::Conflict(
            "the line is not data-bearing; it has no destruction state to record".into(),
        ));
    }

    let affected = sqlx::query(
        r#"UPDATE ewaste_intake_lines
              SET destruction_status = $1,
                  destruction_completed_at = CASE WHEN $1 = 'completed' THEN now() ELSE NULL END,
                  destruction_completed_by = CASE WHEN $1 = 'completed' THEN $2 ELSE NULL END,
                  destruction_document_id = CASE WHEN $1 = 'completed' THEN $3 ELSE NULL END,
                  updated_at = now()
            WHERE id = $4 AND org_id = $5 AND intake_id = $6"#,
    )
    .bind(state.trim())
    .bind(user_id)
    .bind(document_id)
    .bind(line_id)
    .bind(org_id)
    .bind(intake_id)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?
    .rows_affected();

    if affected == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Records one way out for a line. The guard in migration 0238 refuses a
/// pending destruction, an overdraw, a split unit, and a closed intake; the
/// disposition's custody event and the unit's terminal class follow from the
/// row.
pub async fn record_disposition(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
    input: &NewDisposition,
) -> DbResult<EwasteDisposition> {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO ewaste_dispositions (
             id, org_id, intake_id, line_id, pathway, partner_name, disposed_at,
             weight_g, certificate_reference, document_id, created_by
           ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(intake_id)
    .bind(input.line_id)
    .bind(input.pathway.trim())
    .bind(input.partner_name.as_deref().map(str::trim))
    .bind(input.disposed_at)
    .bind(input.weight_g)
    .bind(input.certificate_reference.as_deref().map(str::trim))
    .bind(input.document_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?;

    get_disposition(conn, org_id, id).await
}

/// Closes an intake. The guard refuses a close that leaves weight, destruction
/// or lines unaccounted for.
pub async fn close_intake(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    intake_id: Uuid,
) -> DbResult<EwasteIntake> {
    let affected = sqlx::query(
        r#"UPDATE ewaste_intakes
              SET status = 'closed', closed_at = now(), closed_by = $1,
                  updated_at = now()
            WHERE id = $2 AND org_id = $3 AND status = 'open'"#,
    )
    .bind(user_id)
    .bind(intake_id)
    .bind(org_id)
    .execute(&mut *conn)
    .await
    .map_err(guard_error)?
    .rows_affected();

    if affected == 0 {
        let current = get_intake(conn, org_id, intake_id).await?;
        if current.status == "closed" {
            return Err(DbError::Conflict("the intake is already closed".into()));
        }
        return Err(DbError::NotFound);
    }
    get_intake(conn, org_id, intake_id).await
}

// Reads ----------------------------------------------------------------------

/// The intake's write-sequence version, or `0` before its first write. Read it
/// in the same transaction as the detail so a client can send a conditional
/// write against the state it saw.
pub async fn intake_version(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<i64> {
    let seq = sqlx::query_scalar!(
        "SELECT seq FROM entity_write_seqs
          WHERE org_id = $1 AND entity = $2 AND entity_id = $3",
        org_id,
        INTAKE_ENTITY,
        intake_id
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(guard_error)?;
    Ok(seq.unwrap_or(0))
}

/// One intake, or `DbError::NotFound` when it is not in the org.
pub async fn get_intake(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<EwasteIntake> {
    let row = sqlx::query(&format!(
        "SELECT {INTAKE_COLUMNS} FROM ewaste_intakes WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(intake_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(guard_error)?
    .ok_or(DbError::NotFound)?;
    intake_from_row(&row)
}

/// Intakes for the org, newest receipt first, with their weights aggregated in
/// the same query (no per-row round trip).
pub async fn list_intakes(
    conn: &mut PgConnection,
    org_id: Uuid,
    filter: &IntakeFilter,
) -> DbResult<Vec<IntakeSummary>> {
    let (admin, user_id) = visibility_binds(filter.visibility);
    let rows = sqlx::query(&format!(
        r#"SELECT {columns},
                  coalesce(l.line_count, 0)::bigint AS line_count,
                  coalesce(l.total_in_g, 0)::bigint  AS total_in_g,
                  coalesce(d.total_out_g, 0)::bigint AS total_out_g,
                  coalesce(w.seq, 0)::bigint        AS version
             FROM ewaste_intakes i
             LEFT JOIN entity_write_seqs w
               ON w.org_id = i.org_id AND w.entity = $7 AND w.entity_id = i.id
             LEFT JOIN (
               SELECT intake_id, count(*) AS line_count, sum(weight_g) AS total_in_g
                 FROM ewaste_intake_lines
                WHERE org_id = $1
                GROUP BY intake_id
             ) l ON l.intake_id = i.id
             LEFT JOIN (
               SELECT intake_id, sum(weight_g) AS total_out_g
                 FROM ewaste_dispositions
                WHERE org_id = $1
                GROUP BY intake_id
             ) d ON d.intake_id = i.id
            WHERE i.org_id = $1
              AND ($2::text IS NULL OR i.status = $2)
              AND ($3::uuid IS NULL OR i.client_id = $3)
              {visibility}
            ORDER BY i.received_at DESC, i.id DESC
            LIMIT $4"#,
        columns = INTAKE_COLUMNS
            .split(", ")
            .map(|c| format!("i.{c} AS {c}"))
            .collect::<Vec<_>>()
            .join(", "),
        visibility = visible_to_caller(5, 6, "i")
    ))
    .bind(org_id)
    .bind(filter.status.as_deref())
    .bind(filter.client_id)
    .bind(filter.limit)
    .bind(admin)
    .bind(user_id.to_string())
    .bind(INTAKE_ENTITY)
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;

    rows.iter()
        .map(|row| {
            let total_in_g: i64 = row.try_get("total_in_g").map_err(DbError::Sql)?;
            let total_out_g: i64 = row.try_get("total_out_g").map_err(DbError::Sql)?;
            Ok(IntakeSummary {
                intake: intake_from_row(row)?,
                line_count: row.try_get("line_count").map_err(DbError::Sql)?,
                total_in_g,
                total_out_g,
                remaining_g: total_in_g.saturating_sub(total_out_g),
                version: row.try_get("version").map_err(DbError::Sql)?,
            })
        })
        .collect()
}

/// The lines of one intake, each with its disposed sum joined in one query.
pub async fn list_lines(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<Vec<EwasteIntakeLine>> {
    let rows = sqlx::query(
        r#"SELECT l.id, l.intake_id, l.line_kind, l.serial_id, l.device_asset_id,
                  l.lot_label, l.category, l.weight_g, l.quantity, l.data_bearing,
                  l.destruction_status, l.destruction_completed_at,
                  l.destruction_completed_by, l.destruction_document_id,
                  l.created_by, l.created_at, l.updated_at,
                  coalesce(d.disposed_g, 0)::bigint AS disposed_weight_g
             FROM ewaste_intake_lines l
             LEFT JOIN (
               SELECT line_id, sum(weight_g) AS disposed_g
                 FROM ewaste_dispositions
                WHERE org_id = $1 AND intake_id = $2
                GROUP BY line_id
             ) d ON d.line_id = l.id
            WHERE l.org_id = $1 AND l.intake_id = $2
            ORDER BY l.created_at, l.id"#,
    )
    .bind(org_id)
    .bind(intake_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    rows.iter().map(line_from_row).collect()
}

/// The custody ledger of one intake, oldest first.
pub async fn list_events(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<Vec<EwasteEvent>> {
    let rows = sqlx::query(
        r#"SELECT id, intake_id, line_id, event_kind, occurred_at, storage_class,
                  location_note, partner_name, disposition_id, actor_user_id,
                  notes, created_at
             FROM ewaste_events
            WHERE org_id = $1 AND intake_id = $2
            ORDER BY occurred_at, created_at, id"#,
    )
    .bind(org_id)
    .bind(intake_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    rows.iter().map(event_from_row).collect()
}

/// The dispositions of one intake.
pub async fn list_dispositions(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<Vec<EwasteDisposition>> {
    let rows = sqlx::query(
        r#"SELECT id, intake_id, line_id, pathway, partner_name, disposed_at,
                  weight_g, certificate_reference, document_id, created_by,
                  created_at
             FROM ewaste_dispositions
            WHERE org_id = $1 AND intake_id = $2
            ORDER BY disposed_at, created_at, id"#,
    )
    .bind(org_id)
    .bind(intake_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    rows.iter().map(disposition_from_row).collect()
}

async fn get_disposition(
    conn: &mut PgConnection,
    org_id: Uuid,
    disposition_id: Uuid,
) -> DbResult<EwasteDisposition> {
    let row = sqlx::query(
        r#"SELECT id, intake_id, line_id, pathway, partner_name, disposed_at,
                  weight_g, certificate_reference, document_id, created_by,
                  created_at
             FROM ewaste_dispositions
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(disposition_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(guard_error)?
    .ok_or(DbError::NotFound)?;
    disposition_from_row(&row)
}

/// The org-wide disposition register, filterable for the accountant handoff.
pub async fn list_dispositions_for_org(
    conn: &mut PgConnection,
    org_id: Uuid,
    filter: &DispositionFilter,
) -> DbResult<Vec<EwasteDisposition>> {
    let (admin, user_id) = visibility_binds(filter.visibility);
    let (cursor_disposed_at, cursor_created_at, cursor_id) = match filter.cursor {
        Some((disposed_at, created_at, id)) => (Some(disposed_at), Some(created_at), Some(id)),
        None => (None, None, None),
    };
    let rows = sqlx::query(&format!(
        r#"SELECT d.id, d.intake_id, d.line_id, d.pathway, d.partner_name,
                  d.disposed_at, d.weight_g, d.certificate_reference, d.document_id,
                  d.created_by, d.created_at
             FROM ewaste_dispositions d
             JOIN ewaste_intakes i ON i.id = d.intake_id AND i.org_id = d.org_id
            WHERE d.org_id = $1
              AND ($2::uuid IS NULL OR i.client_id = $2)
              AND ($3::text IS NULL OR d.pathway = $3)
              AND ($4::date IS NULL OR d.disposed_at >= $4)
              AND ($5::date IS NULL OR d.disposed_at <= $5)
              AND ($9::date IS NULL
                   OR (d.disposed_at, d.created_at, d.id) < ($9, $10, $11))
              {visibility}
            ORDER BY d.disposed_at DESC, d.created_at DESC, d.id DESC
            LIMIT $6"#,
        visibility = visible_to_caller(7, 8, "i")
    ))
    .bind(org_id)
    .bind(filter.client_id)
    .bind(filter.pathway.as_deref())
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.limit)
    .bind(admin)
    .bind(user_id.to_string())
    .bind(cursor_disposed_at)
    .bind(cursor_created_at)
    .bind(cursor_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(guard_error)?;
    rows.iter().map(disposition_from_row).collect()
}

/// The intake's weight accounting.
///
/// One statement, one snapshot: the outbound total and the per-pathway sums
/// are aggregated together, so a disposition that commits during the read can
/// never make `total_out_g` disagree with the sum of `by_pathway`.
pub async fn weight_balance(
    conn: &mut PgConnection,
    org_id: Uuid,
    intake_id: Uuid,
) -> DbResult<WeightBalance> {
    let row = sqlx::query!(
        r#"SELECT
             coalesce((SELECT sum(weight_g) FROM ewaste_intake_lines
                        WHERE org_id = $1 AND intake_id = $2), 0)::bigint AS "total_in_g!",
             coalesce(sum(weight_g), 0)::bigint AS "total_out_g!",
             coalesce(json_agg(json_build_object('pathway', pathway, 'weight_g', weight_g)
                               ORDER BY pathway) FILTER (WHERE pathway IS NOT NULL),
                      '[]'::json) AS "by_pathway!"
           FROM ewaste_dispositions
          WHERE org_id = $1 AND intake_id = $2"#,
        org_id,
        intake_id
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(guard_error)?;
    let by_pathway: Vec<PathwayWeight> = serde_json::from_value(row.by_pathway).map_err(|error| {
        DbError::Corrupt(format!("weight balance pathway aggregate: {error}"))
    })?;
    let total_in_g = row.total_in_g;
    let total_out_g = row.total_out_g;

    Ok(WeightBalance {
        total_in_g,
        total_out_g,
        remaining_g: total_in_g.saturating_sub(total_out_g),
        by_pathway,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathway_vocabulary_matches_the_migration_check() {
        assert_eq!(
            PATHWAYS,
            [
                "recycled",
                "destroyed",
                "resold",
                "returned_to_client",
                "transferred_to_partner"
            ]
        );
        assert!(is_valid_pathway("recycled"));
        assert!(!is_valid_pathway("landfill"));
    }

    #[test]
    fn only_caller_written_event_kinds_are_accepted_from_a_request() {
        assert!(is_valid_caller_event_kind("store"));
        assert!(is_valid_caller_event_kind("handoff"));
        assert!(!is_valid_caller_event_kind("intake"));
        assert!(!is_valid_caller_event_kind("disposition"));
    }

    #[test]
    fn downstream_custodians_need_a_certificate() {
        for pathway in ["recycled", "destroyed", "transferred_to_partner"] {
            assert!(pathway_needs_certificate(pathway));
        }
        for pathway in ["resold", "returned_to_client"] {
            assert!(!pathway_needs_certificate(pathway));
        }
    }

    #[test]
    fn hazard_flags_and_storage_classes_are_closed_vocabularies() {
        assert!(is_valid_hazard_flag("battery"));
        assert!(!is_valid_hazard_flag("explosive"));
        assert!(is_valid_storage_class("secure_cage"));
        assert!(!is_valid_storage_class("dumpster"));
    }
}
