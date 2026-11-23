//! The accounting export ledger (claims-gaps G1–G3; migration 0187).
//!
//! Purpose: the storage half of "send this Marula document to that accounting
//! connection". Three facts live here and they fail independently:
//!
//! * a **run** — which document, which connection, and where it stands
//!   (`in_flight`, `succeeded`, `failed`, `held`) with a typed reason. A run is
//!   unique per `(connection, document)`, so a retry advances the same row
//!   instead of adding a second one that could push the same invoice twice;
//! * a **receipt** — the vendor id the export produced. A credit note links to
//!   the invoice's receipt, and a payment attaches to it; nothing here ever
//!   derives a vendor id from a document number, because neither vendor keeps
//!   those unique in a way you can trust;
//! * a **tax mapping** — Marula tax code to vendor tax code, per connection.
//!   Per connection because two clients' books name the same tax differently.
//!
//! The ordering that makes idempotency work is in [`claim_run`] and
//! [`finish_succeeded`], and it is a deliberate two-phase write: the claim is
//! committed BEFORE the vendor call (a claim that rolls back with the request
//! is no claim), and the receipt is written in the SAME transaction as the
//! terminal state (so a receipt never exists without its run saying so, and a
//! run never says success without an id). A process that dies in between
//! leaves `in_flight` and no receipt — the only state a retry can recognise and
//! resolve by asking the vendor, see `claim_run`'s `reclaim_after`.
//!
//! `held` is deliberately not `failed`. A run is held when no vendor call was
//! made and a person has a decision to make (an unmapped tax code). It carries
//! the same `reason` column so the report renders both from one shape, and
//! `state` keeps them apart, because telling an operator to retry work that is
//! waiting on them is worse than saying nothing.
//!
//! Non-purpose: no vendor HTTP, no permission decisions, no credential
//! decryption (the caller unseals, this module only names the sealed row).
//! RLS is org-level, and client scoping stays in the query layer, as
//! migration 0185 states.
//!
//! Query style: runtime `sqlx::query` with explicit row mapping, matching
//! `connector_connections` rather than the `query!` macros — these statements
//! carry `ON CONFLICT` inference against constraint names and partial indexes,
//! and a compiled cache entry would need re-preparing whenever a predicate
//! moved.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::{DbError, DbResult};

// --- vocabulary ---------------------------------------------------------------

/// Where a run stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportState {
    /// An attempt owns the row. Either it is in the vendor call right now, or
    /// a process died in it — which is why a retry age-tests `claimed_at`.
    InFlight,
    Succeeded,
    /// The vendor refused, the network failed, or the credential is wrong.
    /// Retrying is a person's decision and may legitimately help.
    Failed,
    /// Waiting on a human: nothing was sent and nothing went wrong. The reason
    /// names the decision (an unmapped tax code), and the note is not a fault.
    Held,
}

impl ExportState {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExportState::InFlight => "in_flight",
            ExportState::Succeeded => "succeeded",
            ExportState::Failed => "failed",
            ExportState::Held => "held",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "in_flight" => Some(Self::InFlight),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "held" => Some(Self::Held),
            _ => None,
        }
    }
}

/// What kind of Marula document a run or receipt is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Invoice,
    CreditNote,
    /// Receipts only: this lane does not export payments (nothing triggers it
    /// yet), but the table accepts them so the payment path can land without a
    /// migration.
    Payment,
}

impl RecordKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            RecordKind::Invoice => "invoice",
            RecordKind::CreditNote => "credit_note",
            RecordKind::Payment => "payment",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "invoice" => Some(Self::Invoice),
            "credit_note" => Some(Self::CreditNote),
            "payment" => Some(Self::Payment),
            _ => None,
        }
    }
}

/// The accounting vendor a connection speaks. Stored on the run because a
/// manifest can be upgraded or uninstalled, and the history of what a run
/// talked to must not change underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportVendor {
    Qbo,
    Xero,
}

impl ExportVendor {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExportVendor::Qbo => "qbo",
            ExportVendor::Xero => "xero",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "qbo" => Some(Self::Qbo),
            "xero" => Some(Self::Xero),
            _ => None,
        }
    }
}

// --- rows ---------------------------------------------------------------------

/// One export of one document to one connection.
#[derive(Debug, Clone)]
pub struct ExportRun {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    /// The operator's label for the connection, joined for the report so a row
    /// names "Xero — Contoso" instead of a uuid.
    pub connection_label: String,
    pub vendor: ExportVendor,
    pub record_kind: RecordKind,
    pub record_id: Uuid,
    pub state: ExportState,
    /// Typed reason code (`tax_code_unmapped`, `auth_failed`, …). `None` only
    /// while in flight or on success.
    pub reason: Option<String>,
    /// The vendor's own words, verbatim, when the vendor said anything.
    pub reason_detail: Option<String>,
    /// The Marula tax code the caller named, remembered across attempts.
    pub tax_code: Option<String>,
    /// The vendor code the mapping resolved to, when it resolved.
    pub vendor_tax_code: Option<String>,
    pub attempts: i32,
    pub requested_by: Option<Uuid>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn row_to_run(r: sqlx::postgres::PgRow) -> DbResult<ExportRun> {
    let vendor = r.try_get::<String, _>("vendor")?;
    let state = r.try_get::<String, _>("state")?;
    let kind = r.try_get::<String, _>("record_kind")?;
    Ok(ExportRun {
        id: r.try_get("id")?,
        org_id: r.try_get("org_id")?,
        connection_id: r.try_get("connection_id")?,
        connection_label: r.try_get("connection_label")?,
        vendor: ExportVendor::parse(&vendor).ok_or_else(|| {
            DbError::Corrupt(format!(
                "accounting_export_runs.vendor `{vendor}` is not in the vocabulary"
            ))
        })?,
        record_kind: RecordKind::parse(&kind).ok_or_else(|| {
            DbError::Corrupt(format!(
                "accounting_export_runs.record_kind `{kind}` is not in the vocabulary"
            ))
        })?,
        record_id: r.try_get("record_id")?,
        state: ExportState::parse(&state).ok_or_else(|| {
            DbError::Corrupt(format!(
                "accounting_export_runs.state `{state}` is not in the vocabulary"
            ))
        })?,
        reason: r.try_get("reason")?,
        reason_detail: r.try_get("reason_detail")?,
        tax_code: r.try_get("tax_code")?,
        vendor_tax_code: r.try_get("vendor_tax_code")?,
        attempts: r.try_get("attempts")?,
        requested_by: r.try_get("requested_by")?,
        claimed_at: r.try_get("claimed_at")?,
        completed_at: r.try_get("completed_at")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

const SELECT_RUN: &str = "SELECT r.id, r.org_id, r.connection_id, c.label AS connection_label,
        r.vendor, r.record_kind, r.record_id, r.state, r.reason, r.reason_detail,
        r.tax_code, r.vendor_tax_code, r.attempts, r.requested_by, r.claimed_at,
        r.completed_at, r.created_at, r.updated_at
   FROM accounting_export_runs r
   JOIN connector_connections c
     ON c.id = r.connection_id AND c.org_id = r.org_id";

/// The vendor id an export produced.
#[derive(Debug, Clone)]
pub struct ExportReceiptRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    pub record_kind: RecordKind,
    pub record_id: Uuid,
    /// The vendor's id. Never a document number.
    pub external_id: String,
    pub external_number: Option<String>,
    pub created_at: DateTime<Utc>,
}

const RECEIPT_COLUMNS: &str = "id, org_id, connection_id, record_kind, record_id,
     external_id, external_number, created_at";

fn row_to_receipt(r: sqlx::postgres::PgRow) -> DbResult<ExportReceiptRow> {
    let kind = r.try_get::<String, _>("record_kind")?;
    Ok(ExportReceiptRow {
        id: r.try_get("id")?,
        org_id: r.try_get("org_id")?,
        connection_id: r.try_get("connection_id")?,
        record_kind: RecordKind::parse(&kind).ok_or_else(|| {
            DbError::Corrupt(format!(
                "accounting_export_receipts.record_kind `{kind}` is not in the vocabulary"
            ))
        })?,
        record_id: r.try_get("record_id")?,
        external_id: r.try_get("external_id")?,
        external_number: r.try_get("external_number")?,
        created_at: r.try_get("created_at")?,
    })
}

/// One Marula tax code's vendor counterpart on one connection.
#[derive(Debug, Clone)]
pub struct TaxMapping {
    pub id: Uuid,
    pub org_id: Uuid,
    pub connection_id: Uuid,
    pub marula_tax_code: String,
    pub vendor_tax_code: String,
    pub vendor_tax_code_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const TAX_MAPPING_COLUMNS: &str = "id, org_id, connection_id, marula_tax_code, vendor_tax_code,
     vendor_tax_code_name, created_at, updated_at";

fn row_to_mapping(r: sqlx::postgres::PgRow) -> DbResult<TaxMapping> {
    Ok(TaxMapping {
        id: r.try_get("id")?,
        org_id: r.try_get("org_id")?,
        connection_id: r.try_get("connection_id")?,
        marula_tax_code: r.try_get("marula_tax_code")?,
        vendor_tax_code: r.try_get("vendor_tax_code")?,
        vendor_tax_code_name: r.try_get("vendor_tax_code_name")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

// --- claims -------------------------------------------------------------------

/// Everything a claim needs. The caller resolved the vendor from the installed
/// manifest and already checked that the document exists in its scope.
#[derive(Debug, Clone)]
pub struct ClaimSpec<'a> {
    pub connection_id: Uuid,
    pub vendor: ExportVendor,
    pub record_kind: RecordKind,
    pub record_id: Uuid,
    /// The Marula tax code this export is for, when the caller named one.
    pub tax_code: Option<&'a str>,
    pub requested_by: Uuid,
}

/// What a claim got.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// This caller owns the attempt. Commit before calling the vendor.
    Claimed(Box<ExportRun>),
    /// A receipt already exists, so there is nothing to send. The run is the
    /// current (succeeded) row.
    AlreadySucceeded(Box<ExportRun>, Box<ExportReceiptRow>),
    /// Another in-flight attempt claimed this run inside the reclaim window.
    /// The caller answers 409 rather than racing a sibling for the same
    /// invoice.
    InFlight(Box<ExportRun>),
}

/// Takes the run for a document, or reports why this caller cannot.
///
/// One statement, because claim-and-check must not be two decisions: the
/// `INSERT … ON CONFLICT … DO UPDATE … WHERE` either hands this caller the row
/// or writes nothing, and `RETURNING` is what says which. The `WHERE` is the
/// whole retry policy:
///
/// * a `succeeded` run is never re-claimed — the receipt is the answer;
/// * a `held` or `failed` run is always re-claimable — that is what "adding the
///   mapping exports it without a re-issue" means;
/// * an `in_flight` run is re-claimable only once `claimed_at` is older than
///   `reclaim_after_secs`. That window is the crash window: older means the
///   process that claimed it is gone, younger means it may still be inside the
///   vendor call. The vendor request's own timeout is shorter than the window,
///   so a live attempt cannot be raced by a retry.
pub async fn claim_run(
    conn: &mut PgConnection,
    org_id: Uuid,
    spec: &ClaimSpec<'_>,
    reclaim_after_secs: i64,
) -> DbResult<ClaimOutcome> {
    let claimed: Option<sqlx::postgres::PgRow> = sqlx::query(
        "INSERT INTO accounting_export_runs
             (id, org_id, connection_id, vendor, record_kind, record_id, state,
              tax_code, requested_by, attempts, claimed_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'in_flight', $7, $8, 1, now())
         ON CONFLICT (org_id, connection_id, record_kind, record_id) DO UPDATE
            SET state = 'in_flight',
                tax_code = COALESCE(EXCLUDED.tax_code, accounting_export_runs.tax_code),
                vendor_tax_code = NULL,
                reason = NULL,
                reason_detail = NULL,
                attempts = accounting_export_runs.attempts + 1,
                claimed_at = now(),
                completed_at = NULL,
                requested_by = EXCLUDED.requested_by,
                updated_at = now()
          WHERE accounting_export_runs.state <> 'succeeded'
            AND (accounting_export_runs.state <> 'in_flight'
                 OR accounting_export_runs.claimed_at IS NULL
                 OR accounting_export_runs.claimed_at
                    < now() - make_interval(secs => $9::double precision))
         RETURNING id, org_id, connection_id, vendor, record_kind, record_id, state,
                   reason, reason_detail, tax_code, vendor_tax_code, attempts,
                   requested_by, claimed_at, completed_at, created_at, updated_at",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(spec.connection_id)
    .bind(spec.vendor.as_str())
    .bind(spec.record_kind.as_str())
    .bind(spec.record_id)
    .bind(spec.tax_code)
    .bind(spec.requested_by)
    .bind(reclaim_after_secs as f64)
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(row) = claimed {
        // The join that supplies the label is not on the RETURNING shape, so
        // re-read through the labelled select.
        let id: Uuid = row.try_get("id")?;
        return Ok(ClaimOutcome::Claimed(Box::new(
            get_run(conn, org_id, id).await?,
        )));
    }

    let existing = get_run_for(conn, org_id, spec).await?.ok_or_else(|| {
        // The upsert returned nothing and the row is gone: only a concurrent
        // delete explains it, and a retry is the honest answer.
        DbError::Conflict("the export run disappeared between claim and read".into())
    })?;
    if existing.state == ExportState::Succeeded {
        let receipt = get_receipt(
            conn,
            org_id,
            existing.connection_id,
            existing.record_kind,
            existing.record_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::Corrupt(format!(
                "export run {} is succeeded with no receipt",
                existing.id
            ))
        })?;
        return Ok(ClaimOutcome::AlreadySucceeded(
            Box::new(existing),
            Box::new(receipt),
        ));
    }
    Ok(ClaimOutcome::InFlight(Box::new(existing)))
}

/// One run by document identity.
pub async fn get_run_for(
    conn: &mut PgConnection,
    org_id: Uuid,
    spec: &ClaimSpec<'_>,
) -> DbResult<Option<ExportRun>> {
    let row = sqlx::query(&format!(
        "{SELECT_RUN}
         WHERE r.org_id = $1 AND r.connection_id = $2
           AND r.record_kind = $3 AND r.record_id = $4"
    ))
    .bind(org_id)
    .bind(spec.connection_id)
    .bind(spec.record_kind.as_str())
    .bind(spec.record_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(row_to_run).transpose()
}

/// One run by id.
pub async fn get_run(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<ExportRun> {
    let row = sqlx::query(&format!("{SELECT_RUN} WHERE r.org_id = $1 AND r.id = $2"))
        .bind(org_id)
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_run(row)
}

// --- terminal states ----------------------------------------------------------

/// Records the vendor tax code a mapping resolved to, before the send.
///
/// Separate from the terminal states because the interesting report row is the
/// one where the mapping exists but the books do not carry the code: the run
/// must name the code it tried, or the operator repeats the same write.
pub async fn set_run_vendor_tax_code(
    conn: &mut PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    vendor_tax_code: &str,
) -> DbResult<ExportRun> {
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE accounting_export_runs
            SET vendor_tax_code = $3, updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING id",
    )
    .bind(org_id)
    .bind(run_id)
    .bind(vendor_tax_code)
    .fetch_optional(&mut *conn)
    .await?;
    if updated.is_none() {
        return Err(DbError::NotFound);
    }
    get_run(conn, org_id, run_id).await
}

/// Marks a run held: nothing was sent, a person has a decision to make.
///
/// `vendor_tax_code` is stored because the interesting case is not "the
/// mapping is missing" but "the mapping names a code the books do not have" —
/// the report must be able to say which code, or the operator repeats the same
/// write.
pub async fn mark_held(
    conn: &mut PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    reason: &str,
    detail: Option<&str>,
    vendor_tax_code: Option<&str>,
) -> DbResult<ExportRun> {
    let updated: Option<Uuid> = sqlx::query_scalar(
        "UPDATE accounting_export_runs
            SET state = 'held', reason = $3, reason_detail = $4,
                vendor_tax_code = COALESCE($5, vendor_tax_code),
                completed_at = now(), updated_at = now()
          WHERE org_id = $1 AND id = $2
          RETURNING id",
    )
    .bind(org_id)
    .bind(run_id)
    .bind(reason)
    .bind(detail)
    .bind(vendor_tax_code)
    .fetch_optional(&mut *conn)
    .await?;
    if updated.is_none() {
        return Err(DbError::NotFound);
    }
    get_run(conn, org_id, run_id).await
}

/// Ends a run in failure with a typed reason and the vendor's words.
pub async fn finish_failed(
    conn: &mut PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    reason: &str,
    detail: Option<&str>,
) -> DbResult<ExportRun> {
    let affected = sqlx::query(
        "UPDATE accounting_export_runs
            SET state = 'failed', reason = $3, reason_detail = $4,
                completed_at = now(), updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(run_id)
    .bind(reason)
    .bind(detail)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    get_run(conn, org_id, run_id).await
}

/// What a successful vendor call produced, as the store needs it.
#[derive(Debug, Clone)]
pub struct NewReceipt<'a> {
    pub connection_id: Uuid,
    pub record_kind: RecordKind,
    pub record_id: Uuid,
    pub external_id: &'a str,
    pub external_number: Option<&'a str>,
    pub created_by: Option<Uuid>,
}

/// Writes the receipt and moves the run to `succeeded`, in ONE call so a
/// caller cannot do half of it.
///
/// The receipt upsert keeps the FIRST receipt's vendor id: two receipts for one
/// Marula document would be two vendor documents, so a conflicting insert with
/// a DIFFERENT id is reported as a conflict rather than silently accepted.
/// (The reconcile path never hits that branch — it adopts the id it found
/// before writing.)
pub async fn finish_succeeded(
    conn: &mut PgConnection,
    org_id: Uuid,
    run_id: Uuid,
    receipt: &NewReceipt<'_>,
) -> DbResult<(ExportRun, ExportReceiptRow)> {
    let inserted = sqlx::query(&format!(
        "INSERT INTO accounting_export_receipts
             (id, org_id, connection_id, record_kind, record_id, external_id,
              external_number, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (org_id, connection_id, record_kind, record_id) DO NOTHING
         RETURNING {RECEIPT_COLUMNS}"
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(receipt.connection_id)
    .bind(receipt.record_kind.as_str())
    .bind(receipt.record_id)
    .bind(receipt.external_id)
    .bind(receipt.external_number)
    .bind(receipt.created_by)
    .fetch_optional(&mut *conn)
    .await?;

    let row = match inserted {
        Some(row) => row_to_receipt(row)?,
        None => {
            let existing = get_receipt(
                conn,
                org_id,
                receipt.connection_id,
                receipt.record_kind,
                receipt.record_id,
            )
            .await?
            .ok_or_else(|| {
                DbError::Conflict("the receipt insert conflicted but no receipt is readable".into())
            })?;
            if existing.external_id != receipt.external_id {
                return Err(DbError::Conflict(format!(
                    "this document already has vendor receipt {} on this connection; refusing to \
                     record {} as well (two vendor documents for one Marula document)",
                    existing.external_id, receipt.external_id
                )));
            }
            existing
        }
    };

    sqlx::query(
        "UPDATE accounting_export_runs
            SET state = 'succeeded', reason = NULL, reason_detail = NULL,
                completed_at = now(), updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(run_id)
    .execute(&mut *conn)
    .await?;

    let run = get_run(conn, org_id, run_id).await?;
    Ok((run, row))
}

// --- receipts -----------------------------------------------------------------

/// The receipt for one document on one connection, or `None` when it was never
/// exported there.
///
/// This is the question a credit note asks about its parent invoice, and the
/// answer being `None` is a refusal upstream — never a cue to create a second
/// standalone document.
pub async fn get_receipt(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    record_kind: RecordKind,
    record_id: Uuid,
) -> DbResult<Option<ExportReceiptRow>> {
    let row = sqlx::query(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM accounting_export_receipts
          WHERE org_id = $1 AND connection_id = $2
            AND record_kind = $3 AND record_id = $4"
    ))
    .bind(org_id)
    .bind(connection_id)
    .bind(record_kind.as_str())
    .bind(record_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(row_to_receipt).transpose()
}

// --- listing ------------------------------------------------------------------

/// The run report's filters. Every one is optional; the UI sends the ones the
/// operator set.
#[derive(Debug, Clone, Default)]
pub struct RunFilters {
    pub connection_id: Option<Uuid>,
    pub state: Option<ExportState>,
    pub record_kind: Option<RecordKind>,
    pub record_id: Option<Uuid>,
}

/// The newest runs, capped. A report that can return an unbounded set is a
/// report that eventually pages by accident.
pub async fn list_runs(
    conn: &mut PgConnection,
    org_id: Uuid,
    filters: &RunFilters,
    limit: i64,
) -> DbResult<Vec<ExportRun>> {
    let rows = sqlx::query(&format!(
        "{SELECT_RUN}
         WHERE r.org_id = $1
           AND ($2::uuid IS NULL OR r.connection_id = $2)
           AND ($3::text IS NULL OR r.state = $3)
           AND ($4::text IS NULL OR r.record_kind = $4)
           AND ($5::uuid IS NULL OR r.record_id = $5)
         ORDER BY r.created_at DESC, r.id DESC
         LIMIT $6"
    ))
    .bind(org_id)
    .bind(filters.connection_id)
    .bind(filters.state.map(ExportState::as_str))
    .bind(filters.record_kind.map(RecordKind::as_str))
    .bind(filters.record_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_run).collect()
}

// --- tax mappings -------------------------------------------------------------

/// Reads the mapping for one Marula code on one connection.
pub async fn find_tax_mapping(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    marula_tax_code: &str,
) -> DbResult<Option<TaxMapping>> {
    let row = sqlx::query(&format!(
        "SELECT {TAX_MAPPING_COLUMNS} FROM accounting_tax_mappings
          WHERE org_id = $1 AND connection_id = $2 AND marula_tax_code = $3"
    ))
    .bind(org_id)
    .bind(connection_id)
    .bind(marula_tax_code)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(row_to_mapping).transpose()
}

/// The connection's mappings, in Marula-code order.
pub async fn list_tax_mappings(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Option<Uuid>,
) -> DbResult<Vec<TaxMapping>> {
    let rows = sqlx::query(&format!(
        "SELECT {TAX_MAPPING_COLUMNS} FROM accounting_tax_mappings
          WHERE org_id = $1 AND ($2::uuid IS NULL OR connection_id = $2)
          ORDER BY marula_tax_code, connection_id"
    ))
    .bind(org_id)
    .bind(connection_id)
    .fetch_all(&mut *conn)
    .await?;
    rows.into_iter().map(row_to_mapping).collect()
}

/// Writes (or rewrites) the mapping for a Marula code. Rewriting is intended:
/// a vendor code changes when a rate changes, and refusing the write would send
/// an operator to delete-then-create for a one-field edit.
pub async fn upsert_tax_mapping(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    marula_tax_code: &str,
    vendor_tax_code: &str,
    vendor_tax_code_name: Option<&str>,
    actor: Option<Uuid>,
) -> DbResult<TaxMapping> {
    let row = sqlx::query(&format!(
        "INSERT INTO accounting_tax_mappings
             (id, org_id, connection_id, marula_tax_code, vendor_tax_code,
              vendor_tax_code_name, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (org_id, connection_id, marula_tax_code) DO UPDATE
            SET vendor_tax_code = EXCLUDED.vendor_tax_code,
                vendor_tax_code_name = EXCLUDED.vendor_tax_code_name,
                updated_at = now()
         RETURNING {TAX_MAPPING_COLUMNS}"
    ))
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(connection_id)
    .bind(marula_tax_code)
    .bind(vendor_tax_code)
    .bind(vendor_tax_code_name)
    .bind(actor)
    .fetch_one(&mut *conn)
    .await?;
    row_to_mapping(row)
}

/// Removes a mapping. `false` means it was not there (already gone, or another
/// org's), which the route turns into a 404 rather than a silent success.
pub async fn delete_tax_mapping(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<bool> {
    let affected = sqlx::query("DELETE FROM accounting_tax_mappings WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(affected == 1)
}

// --- connection credential + vendor resolution --------------------------------

/// The connection's sealed credential document, untouched.
///
/// Deliberately returns ciphertext: decryption belongs to the caller that holds
/// the key, and a module that can both read the sealed column and open it is a
/// module one mistake away from returning a token.
pub async fn get_connection_secret(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
) -> DbResult<Option<Vec<u8>>> {
    let row = sqlx::query(
        "SELECT secrets_encrypted FROM connector_connections WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(connection_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row.try_get("secrets_encrypted")?)
}

/// The names an installed connector declares itself by: the manifest's `name`
/// and the catalog slug.
///
/// The accounting vendor is read from here rather than stored on the
/// connection, because ADR 0032 makes the manifest the definition: a connector
/// is what its manifest says it is, and a second place to record "this is QBO"
/// is a second thing to get out of step.
pub async fn get_installation_identity(
    conn: &mut PgConnection,
    org_id: Uuid,
    installation_id: Uuid,
) -> DbResult<Option<(String, String)>> {
    let row = sqlx::query(
        "SELECT COALESCE(v.manifest->>'name', '') AS name,
                COALESCE(cat.slug, '') AS slug
           FROM plugin_installs i
           JOIN plugin_versions v
             ON v.catalog_id = i.catalog_id AND v.version = i.version
           LEFT JOIN plugin_catalog cat ON cat.id = i.catalog_id
          WHERE i.org_id = $1 AND i.id = $2",
    )
    .bind(org_id)
    .bind(installation_id)
    .fetch_optional(&mut *conn)
    .await?;
    match row {
        Some(r) => Ok(Some((r.try_get("name")?, r.try_get("slug")?))),
        None => Ok(None),
    }
}

// --- accountant package: class→account mapping -------------------------------

/// One reporting class and the account it posts to.
#[derive(Debug, Clone)]
pub struct ClassAccount {
    pub reporting_class: String,
    pub account_code: String,
    pub account_name: String,
}

/// The built-in defaults, so a package never needs a mapping meeting: every
/// class a money row can carry has an account even before the org chooses one.
/// `cogs` is deliberately absent — cost of goods sold has no universal account
/// number, so the export holds and names it instead of guessing.
pub const DEFAULT_CLASS_ACCOUNTS: &[(&str, &str, &str)] = &[
    ("inventory", "1200", "Inventory"),
    ("sales", "4000", "Sales"),
    ("rental_income", "4100", "Rental income"),
    ("freight_in", "5100", "Freight in"),
    ("duty", "5140", "Duty"),
    ("duty_in", "5110", "Duty in"),
    ("brokerage_in", "5120", "Brokerage in"),
    ("insurance_in", "5130", "Insurance in"),
    ("service_cost", "5200", "Service cost"),
    ("rental_expense", "5400", "Rental expense"),
    ("ewaste_cost", "5500", "E-waste cost"),
    ("overhead", "5600", "Overhead"),
    ("other", "5900", "Other cost"),
];

/// The org's mapping, with the defaults filled in for every class it has not
/// chosen. Ordered by class so a package is byte-stable between runs.
pub async fn class_accounts(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<ClassAccount>> {
    let rows = sqlx::query!(
        r#"SELECT reporting_class, account_code, account_name
             FROM accounting_class_accounts WHERE org_id = $1"#,
        org_id,
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut chosen: std::collections::HashMap<String, (String, String)> = rows
        .into_iter()
        .map(|r| (r.reporting_class, (r.account_code, r.account_name)))
        .collect();
    let mut out: Vec<ClassAccount> = DEFAULT_CLASS_ACCOUNTS
        .iter()
        .map(|(class, code, name)| {
            let (code, name) = chosen
                .remove(*class)
                .unwrap_or_else(|| ((*code).to_string(), (*name).to_string()));
            ClassAccount {
                reporting_class: (*class).to_string(),
                account_code: code,
                account_name: name,
            }
        })
        .collect();
    // A class outside the defaults still resolves to the org's own code.
    let mut extra: Vec<ClassAccount> = chosen
        .into_iter()
        .map(
            |(reporting_class, (account_code, account_name))| ClassAccount {
                reporting_class,
                account_code,
                account_name,
            },
        )
        .collect();
    extra.sort_by(|a, b| a.reporting_class.cmp(&b.reporting_class));
    out.extend(extra);
    Ok(out)
}

/// Sets one class's account for the org.
pub async fn upsert_class_account(
    conn: &mut PgConnection,
    org_id: Uuid,
    reporting_class: &str,
    account_code: &str,
    account_name: &str,
) -> DbResult<ClassAccount> {
    sqlx::query!(
        r#"INSERT INTO accounting_class_accounts
             (id, org_id, reporting_class, account_code, account_name)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (org_id, reporting_class) DO UPDATE
             SET account_code = EXCLUDED.account_code,
                 account_name = EXCLUDED.account_name,
                 updated_at = now()"#,
        Uuid::now_v7(),
        org_id,
        reporting_class,
        account_code,
        account_name,
    )
    .execute(&mut *conn)
    .await?;
    Ok(ClassAccount {
        reporting_class: reporting_class.to_string(),
        account_code: account_code.to_string(),
        account_name: account_name.to_string(),
    })
}

/// One line of the package: a money row with the record it belongs to.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PackageRecord {
    pub kind: String,
    pub stable_ref: String,
    pub document_ref: Option<String>,
    pub occurred_on: NaiveDate,
    pub client_id: Option<Uuid>,
    pub currency: String,
    pub amount_minor: i64,
    pub base_amount_minor: Option<i64>,
    pub reporting_class: Option<String>,
    pub memo: Option<String>,
}

/// Thousandths of an item's unit rendered the way a person reads it: `10`,
/// `2.5`, `0.125`.
fn units_text(qty_milli: i64) -> String {
    let whole = qty_milli / 1000;
    let frac = qty_milli % 1000;
    if frac == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{frac:03}")
            .trim_end_matches('0')
            .to_string()
    }
}

/// A quantity in thousandths times a per-unit minor amount, rounded half up to
/// whole minor units. Integer math throughout: a valuation never introduces a
/// float into the books.
fn snapshot_value_minor(qty_milli: i64, unit_cost_minor: i64) -> i64 {
    let product = i128::from(qty_milli) * i128::from(unit_cost_minor);
    i64::try_from((product + 500) / 1000).unwrap_or(i64::MAX)
}

/// Every exportable row in the period, each with the stable reference that
/// names it: `kind:id`. Invoices and credit notes carry their document number;
/// a vendor bill names the vendor's bill and the purchase order; a cost layer
/// and a rental charge carry their reference or description; a stock valuation
/// row names the stock item and the quantity on hand at period end; an e-waste
/// disposition names the certificate or the intake.
pub async fn package_records(
    conn: &mut PgConnection,
    org_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> DbResult<Vec<PackageRecord>> {
    let mut out: Vec<PackageRecord> = Vec::new();
    let invoices = sqlx::query!(
        r#"SELECT id, number, issued_at, client_id, currency, total_minor, base_total_minor
             FROM invoices
            WHERE org_id = $1 AND issued_at BETWEEN $2 AND $3 AND status <> 'void'"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(invoices.into_iter().map(|r| PackageRecord {
        kind: "invoice".into(),
        stable_ref: format!("invoice:{}", r.id),
        document_ref: Some(r.number),
        occurred_on: r.issued_at,
        client_id: Some(r.client_id),
        currency: r.currency,
        amount_minor: r.total_minor,
        base_amount_minor: r.base_total_minor,
        reporting_class: Some("sales".into()),
        memo: None,
    }));
    let credits = sqlx::query!(
        r#"SELECT id, number, created_at::date AS issued_on, client_id, currency,
                  total_minor
             FROM credit_notes
            WHERE org_id = $1 AND created_at::date BETWEEN $2 AND $3 AND status <> 'void'"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(credits.into_iter().map(|r| PackageRecord {
        kind: "credit_note".into(),
        stable_ref: format!("credit_note:{}", r.id),
        document_ref: Some(r.number),
        occurred_on: r.issued_on.unwrap_or(from),
        client_id: Some(r.client_id),
        currency: r.currency,
        amount_minor: -r.total_minor,
        // A credit note carries no base snapshot of its own; the package
        // names the document currency and leaves conversion to the books.
        base_amount_minor: None,
        reporting_class: Some("sales".into()),
        memo: None,
    }));
    // A posted vendor bill is money owed in the period. The class follows what
    // the order bought: goods that enter custody are inventory, a rented-in
    // unit is rental expense, an e-waste service is e-waste cost, and a
    // consumable bought for a project is that project's cost of goods sold.
    let bills = sqlx::query!(
        r#"SELECT b.id, b.bill_number, (b.posted_at AT TIME ZONE 'UTC')::date AS "posted_on!", po.number AS order_number,
                  po.client_id, b.currency, b.amount_minor, b.base_amount_minor,
                  CASE po.acquired_for
                    WHEN 'vendor_rental' THEN 'rental_expense'
                    WHEN 'ewaste_service' THEN 'ewaste_cost'
                    WHEN 'project_consumable' THEN 'cogs'
                    ELSE 'inventory'
                  END AS "reporting_class!"
             FROM purchase_bills b
             JOIN purchase_orders po ON po.id = b.purchase_order_id AND po.org_id = $1
            WHERE b.org_id = $1 AND b.posted_at IS NOT NULL
              -- Half-open UTC bounds, so the (org_id, posted_at) index is used
              -- and a bill posted at 23:59 on the last day is in the period.
              AND b.posted_at >= ($2::date::timestamp AT TIME ZONE 'UTC')
              AND b.posted_at <  (($3::date + 1)::timestamp AT TIME ZONE 'UTC')"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(bills.into_iter().map(|r| PackageRecord {
        kind: "vendor_bill".into(),
        stable_ref: format!("purchase_bill:{}", r.id),
        document_ref: r.bill_number,
        occurred_on: r.posted_on,
        client_id: r.client_id,
        currency: r.currency,
        amount_minor: r.amount_minor,
        base_amount_minor: r.base_amount_minor,
        reporting_class: Some(r.reporting_class),
        memo: Some(format!("purchase order {}", r.order_number)),
    }));
    let layers = sqlx::query!(
        r#"SELECT id, reference, document_date, client_id, currency, amount_minor,
                  base_amount_minor, reporting_class, kind
             FROM acquisition_costs
            WHERE org_id = $1 AND document_date BETWEEN $2 AND $3 AND capitalised"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(layers.into_iter().map(|r| PackageRecord {
        kind: "acquisition_cost".into(),
        stable_ref: format!("acquisition_cost:{}", r.id),
        document_ref: Some(r.reference),
        occurred_on: r.document_date,
        client_id: r.client_id,
        currency: r.currency,
        amount_minor: r.amount_minor,
        base_amount_minor: r.base_amount_minor,
        reporting_class: Some(r.reporting_class),
        memo: Some(r.kind),
    }));
    let charges = sqlx::query!(
        r#"SELECT id, description, period_start, client_id, currency, amount_minor,
                  base_amount_minor
             FROM rental_charges
            WHERE org_id = $1 AND period_start BETWEEN $2 AND $3 AND kind = 'rent'"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(charges.into_iter().map(|r| PackageRecord {
        kind: "rental_charge".into(),
        stable_ref: format!("rental_charge:{}", r.id),
        document_ref: None,
        occurred_on: r.period_start,
        client_id: Some(r.client_id),
        currency: r.currency,
        amount_minor: r.amount_minor,
        base_amount_minor: r.base_amount_minor,
        reporting_class: Some("rental_income".into()),
        memo: Some(r.description),
    }));
    // Stock valuation at period end: one row per item that has quantity on
    // hand, valued at the last receipt cost locked at or before the period end
    // (the item's current cost when the ledger holds no receipt up to that
    // date). Quantity is the
    // ledger's net movement up to that date, never the live level, so a package
    // for an earlier period values what was on the shelf then.
    let stock = sqlx::query!(
        r#"WITH on_hand AS (
              SELECT m.stock_item_id, SUM(m.qty_milli)::bigint AS qty_milli
                FROM stock_movements m
               WHERE m.org_id = $1
                 AND m.created_at < (($2::date + 1)::timestamp AT TIME ZONE 'UTC')
               GROUP BY m.stock_item_id
              HAVING SUM(m.qty_milli) > 0
           ), basis AS (
              SELECT DISTINCT ON (m.stock_item_id)
                     m.stock_item_id, m.cost_minor, m.currency, m.base_cost_minor
                FROM stock_movements m
               WHERE m.org_id = $1 AND m.kind = 'receive' AND m.cost_minor IS NOT NULL
                 AND m.created_at < (($2::date + 1)::timestamp AT TIME ZONE 'UTC')
               ORDER BY m.stock_item_id, m.created_at DESC, m.id DESC
           )
           SELECT i.id, i.name, i.sku, i.currency AS item_currency, i.last_unit_cost_minor,
                  h.qty_milli AS "qty_milli!", b.cost_minor AS "receipt_cost_minor?",
                  b.currency AS "receipt_currency?", b.base_cost_minor
             FROM on_hand h
             JOIN stock_items i ON i.id = h.stock_item_id AND i.org_id = $1
             LEFT JOIN basis b ON b.stock_item_id = i.id
            ORDER BY i.id"#,
        org_id,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    out.extend(stock.into_iter().map(|r| {
        let unit_cost = r.receipt_cost_minor.or(r.last_unit_cost_minor);
        let amount_minor = unit_cost.map_or(0, |c| snapshot_value_minor(r.qty_milli, c));
        let base_amount_minor = match (r.receipt_cost_minor, r.base_cost_minor) {
            (Some(_), Some(base)) => Some(snapshot_value_minor(r.qty_milli, base)),
            _ => None,
        };
        let mut memo = format!("{} {} on hand", units_text(r.qty_milli), r.name);
        if unit_cost.is_none() {
            memo.push_str(" (no cost on record)");
        }
        PackageRecord {
            kind: "stock_item".into(),
            stable_ref: format!("stock_item:{}", r.id),
            document_ref: r.sku,
            occurred_on: to,
            client_id: None,
            currency: r.receipt_currency.unwrap_or(r.item_currency),
            amount_minor,
            base_amount_minor,
            reporting_class: Some("inventory".into()),
            memo: Some(memo),
        }
    }));
    // E-waste dispositions carry no money: the row is the custody fact, the
    // weight and the certificate, with a zero amount so the totals still add
    // up. The client is the intake's, and the reference points at the paper an
    // auditor asks for.
    let dispositions = sqlx::query!(
        r#"SELECT d.id, d.pathway, d.partner_name, d.disposed_at, d.weight_g,
                  d.certificate_reference, i.client_id, i.reference AS intake_reference
             FROM ewaste_dispositions d
             JOIN ewaste_intakes i ON i.id = d.intake_id AND i.org_id = $1
            WHERE d.org_id = $1 AND d.disposed_at BETWEEN $2 AND $3"#,
        org_id,
        from,
        to,
    )
    .fetch_all(&mut *conn)
    .await?;
    if !dispositions.is_empty() {
        // The rows carry no currency of their own, so they are stated in base.
        let base_currency = crate::fx::base_currency(&mut *conn, org_id).await?;
        out.extend(dispositions.into_iter().map(|r| {
            let mut memo = vec![r.pathway.clone(), format!("{} g", r.weight_g)];
            if let Some(partner) = &r.partner_name {
                memo.push(partner.clone());
            }
            if let Some(certificate) = &r.certificate_reference {
                memo.push(format!("certificate {certificate}"));
            }
            PackageRecord {
                kind: "ewaste_disposition".into(),
                stable_ref: format!("ewaste_disposition:{}", r.id),
                document_ref: r.certificate_reference.or(r.intake_reference),
                occurred_on: r.disposed_at,
                client_id: Some(r.client_id),
                currency: base_currency.clone(),
                amount_minor: 0,
                base_amount_minor: None,
                reporting_class: Some("ewaste_cost".into()),
                memo: Some(memo.join(" · ")),
            }
        }));
    }
    out.sort_by(|a, b| {
        (a.occurred_on, a.kind.as_str(), a.stable_ref.as_str()).cmp(&(
            b.occurred_on,
            b.kind.as_str(),
            b.stable_ref.as_str(),
        ))
    });
    Ok(out)
}
