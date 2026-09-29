//! Billing verification run (Wave 39 lane 2, migration 0111): the
//! read/write seam behind the per-client/contract verification
//! workspace — report V1 (surface), V2 (adjustments), V8 (close
//! sign-off), V9 (audit rows).
//!
//! Three row families:
//!
//! * `time_entry_adjustments` — the additive minute-adjustment ledger.
//!   The original entry row is NEVER rewritten for billing: the engine
//!   inputs subtract approved deltas, so a later run bills
//!   `(minutes + Σ delta)`. The lock test rides `invoice_time_entries`
//!   — a linked (invoiced) entry refuses any new write here.
//! * `time_entry_dispositions` — one CURRENT disposition per entry
//!   (partial unique index), superseded rows kept as history.
//! * `invoice_run_signoffs` — the close checklist one reviewer attests
//!   per run; the create-run seam refuses an unsigned run.
//!
//! Non-purpose: no approval-status transitions here (approvals.rs owns
//! the queue state machine); no rate/pricing writes (contracts.rs).

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{DispositionStatus, TimeEntryAdjustment};
use sqlx::Postgres;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Lock seam (V2: pre-run only — invoiced entries stay locked)
// ---------------------------------------------------------------------------

/// True when the entry has been consumed by any invoice. Every
/// verification write checks this FIRST; the check rides the same
/// transaction as the write.
pub async fn entry_is_invoiced<'e, E>(executor: E, org_id: Uuid, entry_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let hit: Option<Uuid> = sqlx::query_scalar(
        "SELECT time_entry_id FROM invoice_time_entries
          WHERE time_entry_id = $2
            AND EXISTS (SELECT 1 FROM invoices i
                        WHERE i.id = invoice_time_entries.invoice_id AND i.org_id = $1)",
    )
    .bind(org_id)
    .bind(entry_id)
    .fetch_optional(executor)
    .await?;
    Ok(hit.is_some())
}

// ---------------------------------------------------------------------------
// Adjustments (V2 + V9 evidence)
// ---------------------------------------------------------------------------

/// The inputs of one adjustment write.
#[derive(Debug, Clone)]
pub struct NewAdjustment<'a> {
    pub org_id: Uuid,
    pub time_entry_id: Uuid,
    /// The run this adjustment rides, when it is made inside a run's
    /// close flow; `None` from the standalone workspace.
    pub run_id: Option<Uuid>,
    /// Minutes BEFORE this adjustment (denormalized evidence).
    pub original_minutes: i32,
    /// Signed: negative reduces, zero zeroes the entry out of billing.
    pub delta_minutes: i32,
    /// The human explanation. The column is NOT NULL, so an adjustment
    /// without one cannot be stored.
    pub reason: &'a str,
    pub actor_id: Uuid,
}

/// Inserts one adjustment row. The API layer has already: validated the
/// signature (reduction only, effective ≥ 0), required a non-empty
/// reason, verified the entry is invoiced-free inside THIS transaction,
/// and written the audit row.
pub async fn insert_adjustment(
    tx: &mut sqlx::PgConnection,
    new: NewAdjustment<'_>,
) -> DbResult<TimeEntryAdjustment> {
    let NewAdjustment {
        org_id,
        time_entry_id: entry_id,
        run_id,
        original_minutes,
        delta_minutes,
        reason,
        actor_id,
    } = new;
    if entry_is_invoiced(&mut *tx, org_id, entry_id).await? {
        return Err(DbError::Conflict(
            "entry is invoiced and locked; verification actions are pre-run only".into(),
        ));
    }
    TimeEntryAdjustment::validate(original_minutes, delta_minutes).map_err(|e| {
        DbError::Conflict(format!("invalid adjustment: {e}"))
    })?;
    let row = sqlx::query!(
        r#"INSERT INTO time_entry_adjustments
             (id, org_id, time_entry_id, run_id, original_minutes, delta_minutes, reason, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, org_id, time_entry_id, run_id, original_minutes,
                     delta_minutes, reason, created_by, created_at"#,
        Uuid::now_v7(),
        org_id,
        entry_id,
        run_id,
        original_minutes,
        delta_minutes,
        reason,
        actor_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TimeEntryAdjustment {
        id: row.id,
        org_id: row.org_id,
        time_entry_id: row.time_entry_id,
        run_id: row.run_id,
        original_minutes: row.original_minutes,
        delta_minutes: row.delta_minutes,
        reason: row.reason,
        created_by: row.created_by,
        created_at: row.created_at,
    })
}

/// Sum of signed deltas for one entry (negative or zero). The engine
/// inputs subtract this from the raw minutes before pricing.
pub async fn total_adjustment_minutes<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_id: Uuid,
) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(delta_minutes), 0) FROM time_entry_adjustments
          WHERE org_id = $1 AND time_entry_id = $2",
    )
    .bind(org_id)
    .bind(entry_id)
    .fetch_one(executor)
    .await?;
    Ok(v.unwrap_or(0))
}

// ---------------------------------------------------------------------------
// Dispositions (V1)
// ---------------------------------------------------------------------------

/// Replaces the entry's current disposition: the old current row flips
/// `current = false` (history kept), the new row lands current. The
/// invoiced lock applies with one carve-out — `needs_review` ON a
/// locked entry would contradict what the invoice already billed, so
/// locked entries refuse ALL disposition writes; a correction after
/// invoicing is a credit-note concern, not a verification one.
pub async fn upsert_disposition(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    entry_id: Uuid,
    status: DispositionStatus,
    unbillable_reason: Option<&str>,
    note: Option<&str>,
    actor_id: Uuid,
) -> DbResult<Uuid> {
    if status.requires_unbillable_reason() && unbillable_reason.is_none() {
        return Err(DbError::Conflict(format!(
            "disposition '{}' requires a typed unbillable_reason",
            status.as_str()
        )));
    }
    if entry_is_invoiced(&mut *tx, org_id, entry_id).await? {
        return Err(DbError::Conflict(
            "entry is invoiced and locked; verification actions are pre-run only".into(),
        ));
    }
    sqlx::query(
        "UPDATE time_entry_dispositions SET current = false
          WHERE org_id = $1 AND time_entry_id = $2 AND current",
    )
    .bind(org_id)
    .bind(entry_id)
    .execute(&mut *tx)
    .await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO time_entry_dispositions
           (id, org_id, time_entry_id, status, unbillable_reason, note, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(entry_id)
    .bind(status.as_str())
    .bind(unbillable_reason)
    .bind(note)
    .bind(actor_id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(id)
}

/// The current disposition per entry, if any.
pub async fn list_current_dispositions<'e, E>(
    executor: E,
    org_id: Uuid,
    entry_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, DispositionStatus, Option<String>)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if entry_ids.is_empty() {
        return Ok(vec![]);
    }
    let rows = sqlx::query!(
        r#"SELECT time_entry_id, status, unbillable_reason
             FROM time_entry_dispositions
            WHERE org_id = $1 AND current
              AND time_entry_id = ANY($2)"#,
        org_id,
        entry_ids,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.time_entry_id,
                DispositionStatus::parse(&r.status).unwrap_or(DispositionStatus::NeedsReview),
                r.unbillable_reason,
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Run sign-off (V8)
// ---------------------------------------------------------------------------

/// Records (or re-records) the reviewer's close checklist for one
/// contract window. Idempotent per window: a re-sign replaces the
/// identity/note and refreshes the timestamp, and an unsigned-then-
/// signed window keeps the same row. The API layer has already
/// re-verified each checklist boolean against the live workspace, so
/// this is the write of an attested fact, not the check itself.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_run_signoff<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
    period_from: NaiveDate,
    period_to: NaiveDate,
    entries_dispositioned: bool,
    zero_unclassified: bool,
    preview_seen: bool,
    reviewer_id: Uuid,
    reviewer_note: Option<&str>,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO invoice_run_signoffs
           (id, org_id, contract_id, period_from, period_to,
            entries_dispositioned, zero_unclassified, preview_seen,
            reviewer_id, reviewer_note)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (org_id, contract_id, period_from, period_to) DO UPDATE SET
           entries_dispositioned = EXCLUDED.entries_dispositioned,
           zero_unclassified = EXCLUDED.zero_unclassified,
           preview_seen = EXCLUDED.preview_seen,
           reviewer_id = EXCLUDED.reviewer_id,
           reviewer_note = EXCLUDED.reviewer_note,
           signed_at = now()",
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(contract_id)
    .bind(period_from)
    .bind(period_to)
    .bind(entries_dispositioned)
    .bind(zero_unclassified)
    .bind(preview_seen)
    .bind(reviewer_id)
    .bind(reviewer_note)
    .execute(executor)
    .await?;
    Ok(())
}

/// Stamps the sign-off rows a run just claimed: every window this run
/// invoiced that carried a sign-off. Runs that invoice without one still
/// work (the ritual is a warning surface, not a hard gate — see the
/// overlap warning) but the evidence is linked when it exists.
pub async fn attach_signoffs_to_run<'e, E>(
    executor: E,
    org_id: Uuid,
    run_id: Uuid,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE invoice_run_signoffs s
            SET run_id = $2
           FROM invoices i
          WHERE i.run_id = $2
            AND i.org_id = $1
            AND s.org_id = $1
            AND s.contract_id = i.contract_id
            AND s.run_id IS NULL",
    )
    .bind(org_id)
    .bind(run_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// The sign-off for one contract window, if any.
pub async fn get_run_signoff<'e, E>(
    executor: E,
    org_id: Uuid,
    contract_id: Uuid,
    period_from: NaiveDate,
    period_to: NaiveDate,
) -> DbResult<Option<RunSignoffRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, contract_id, period_from, period_to, run_id,
                  entries_dispositioned, zero_unclassified, preview_seen,
                  reviewer_id, reviewer_note, signed_at
             FROM invoice_run_signoffs
            WHERE org_id = $1 AND contract_id = $2
              AND period_from = $3 AND period_to = $4"#,
        org_id,
        contract_id,
        period_from,
        period_to,
    )
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| RunSignoffRow {
        id: r.id,
        contract_id: r.contract_id,
        period_from: r.period_from,
        period_to: r.period_to,
        run_id: r.run_id,
        entries_dispositioned: r.entries_dispositioned,
        zero_unclassified: r.zero_unclassified,
        preview_seen: r.preview_seen,
        reviewer_id: r.reviewer_id,
        reviewer_note: r.reviewer_note,
        signed_at: r.signed_at,
    }))
}

/// One sign-off row, flattened for the workspace response.
pub struct RunSignoffRow {
    pub id: Uuid,
    pub contract_id: Uuid,
    pub period_from: NaiveDate,
    pub period_to: NaiveDate,
    pub run_id: Option<Uuid>,
    pub entries_dispositioned: bool,
    pub zero_unclassified: bool,
    pub preview_seen: bool,
    pub reviewer_id: Option<Uuid>,
    pub reviewer_note: Option<String>,
    pub signed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Workspace reads (V1/V7) + overlap warning (V8)
// ---------------------------------------------------------------------------

/// One workspace entry row: the identity + everything the badge, cost
/// cell, and counters need. `effective_minutes` carries the
/// adjustment-adjusted minutes; `adjustment_minutes` is the Σ delta;
/// `invoiced` is the lock flag; `disposition` is the current badge.
pub struct VerificationEntryRow {
    pub entry_id: Uuid,
    pub org_id: Uuid,
    pub ticket_id: Option<Uuid>,
    pub ticket_number: Option<String>,
    pub phase_id: Option<Uuid>,
    pub phase_name: Option<String>,
    pub project_name: Option<String>,
    pub user_id: Uuid,
    pub user_name: String,
    pub started_at: DateTime<Utc>,
    pub minutes: i64,
    pub effective_minutes: i64,
    pub adjustment_minutes: i64,
    pub billable: bool,
    pub work_type: Option<String>,
    pub notes: Option<String>,
    pub unbillable_reason: Option<String>,
    pub approval_status: String,
    pub disposition: Option<DispositionStatus>,
    pub invoiced: bool,
}

/// Every verification-scope entry for one client over `[from, to]`:
/// all statuses (the verification surface shows draft + rejected +
/// non-billable too — the whole story, not just what would bill), with
/// adjustment sums and current dispositions joined. Ordered by
/// (started_at, entry id) so per-ticket grouping is deterministic.
///
/// Scope is the CLIENT, so both entry parents count: ticket-scoped
/// entries through `tickets.client_id` (lane 4 gave entries a second
/// parent, so an inner join on tickets would silently drop every
/// project phase's time) and phase-scoped entries through
/// `projects.client_id`.
pub async fn list_client_entries(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    client_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> DbResult<Vec<VerificationEntryRow>> {
    let rows = sqlx::query!(
        r#"SELECT te.id, te.org_id, te.ticket_id, tk.number AS "ticket_number?",
                  te.phase_id, ph.name AS "phase_name?", pj.name AS "project_name?",
                  te.user_id, u.display_name,
                  te.started_at, te.minutes, te.billable, te.work_type,
                  te.notes, te.unbillable_reason,
                  COALESCE(te.approval_status, 'draft') AS "approval_status!",
                  COALESCE((SELECT SUM(a.delta_minutes) FROM time_entry_adjustments a
                             WHERE a.org_id = te.org_id AND a.time_entry_id = te.id), 0)
                     AS "adjustment_minutes!: i64",
                  d.status AS "disposition_status?",
                  EXISTS (SELECT 1 FROM invoice_time_entries l
                           JOIN invoices i ON i.id = l.invoice_id
                          WHERE l.time_entry_id = te.id AND i.org_id = te.org_id) AS "invoiced!: bool"
             FROM time_entries te
             JOIN users u ON u.id = te.user_id
        LEFT JOIN tickets tk ON tk.id = te.ticket_id
        LEFT JOIN project_phases ph ON ph.id = te.phase_id
        LEFT JOIN projects pj ON pj.id = ph.project_id
        LEFT JOIN time_entry_dispositions d
                ON d.time_entry_id = te.id AND d.current AND d.org_id = te.org_id
            WHERE te.org_id = $1
              AND (tk.client_id = $2 OR pj.client_id = $2)
              AND te.started_at::date >= $3 AND te.started_at::date <= $4
            ORDER BY te.started_at, te.id"#,
        org_id,
        client_id,
        from,
        to,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| VerificationEntryRow {
            entry_id: r.id,
            org_id: r.org_id,
            ticket_id: r.ticket_id,
            ticket_number: r.ticket_number,
            phase_id: r.phase_id,
            phase_name: r.phase_name,
            project_name: r.project_name,
            user_id: r.user_id,
            user_name: r.display_name,
            started_at: r.started_at,
            minutes: r.minutes as i64,
            // The ledger is SIGNED (verification only ever reduces), so
            // the effective minutes ADD the delta. This mirrors the
            // engine's own `minutes + SUM(delta)` in
            // `contracts::list_billable_entries`; the two must never
            // disagree, and a workspace that showed 210 minutes for a
            // 120-minute entry trimmed to 30 would be worse than no
            // number at all.
            effective_minutes: r.minutes as i64 + r.adjustment_minutes,
            adjustment_minutes: r.adjustment_minutes,
            billable: r.billable,
            work_type: r.work_type,
            notes: r.notes,
            unbillable_reason: r.unbillable_reason,
            approval_status: r.approval_status,
            disposition: r
                .disposition_status
                .as_deref()
                .and_then(DispositionStatus::parse),
            invoiced: r.invoiced,
        })
        .collect())
}

/// One issued-run window for the overlap warning (V8). `invoice_runs`
/// is the run-window ledger — issued runs only exist here.
pub async fn list_run_windows<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<Vec<(Uuid, String, NaiveDate, NaiveDate)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, number, period_from, period_to
             FROM invoice_runs
            WHERE org_id = $1
            ORDER BY created_at DESC
            LIMIT 200"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.id, r.number, r.period_from, r.period_to))
        .collect())
}
