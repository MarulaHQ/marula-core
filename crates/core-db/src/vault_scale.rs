//! Durable vault scale-out state (Wave 19 lane 1, migration 0094):
//! cross-replica write-budget seats + the token-bucket bulk-ingestion
//! pacer. Both share one property: admission state lives in Postgres
//! keyed per org, so a multi-replica deployment budgets/paces as ONE
//! org instead of N uncoordinated processes. The API layer keeps the
//! Wave 16 lane 9 in-memory bucket (`check_vault_budget`) as the
//! single-replica fast path in front of these calls — a local breach still
//! answers 429 without touching the DB (a local count over the limit can
//! never be a false negative: the local window can be no bigger than the
//! true org-wide window), while the durable seat/token check is the
//! authoritative verdict for writes that pass locally.
//!
//! All timings ride the DB clock (`now()`), matching the queue/lease
//! discipline in [`crate::queue`]: every timing decision is made in SQL,
//! immune to restarts and uncoordinated per-replica process state.

use crate::DbResult;
use sqlx::Acquire as _;
use sqlx::PgConnection;

/// Fixed window length for the per-org write budget, matching
/// `VAULT_BUDGET_WINDOW_SECS` over in core-api::routes::secrets.
pub const BUDGET_WINDOW_SECS: u32 = 60;

/// Outcome of consuming one durable write-budget seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetSeatVerdict {
    /// The seat fits under the limit for this window.
    Admitted,
    /// The window is full — the caller maps this to the typed 429 with
    /// this `Retry-After` hint (seconds until the window rolls, min 1).
    Exceeded { retry_after_secs: i64 },
}

/// Consumes one seat of the org's durable write budget. A short-circuit
/// (`Admitted`, no table touch) when `limit_per_min == 0` (the documented
/// escape hatch).
///
/// Convenience wrapper over [`consume_write_budget_seats`] for the
/// single-row mutation surfaces; the multi-row path (bulk vault ingest)
/// charges one seat per row it writes.
pub async fn consume_write_budget_seat(
    conn: &mut PgConnection,
    org_id: uuid::Uuid,
    limit_per_min: u32,
) -> DbResult<BudgetSeatVerdict> {
    consume_write_budget_seats(conn, org_id, limit_per_min, 1).await
}

/// Consumes `seats` of the org's durable write budget in ONE verdict: a
/// multi-row request either fits entirely or consumes nothing. A
/// short-circuit (`Admitted`, no table touch) when `limit_per_min == 0`
/// (the documented escape hatch) or `seats == 0`.
///
/// Semantics mirror the Wave 16 in-memory bucket exactly, now durable:
/// a tumbling 60-second window anchored where the window STARTED (a
/// window older than `BUDGET_WINDOW_SECS` by the DB clock rolls: seats
/// reset and the anchor moves to now). An increment that would exceed
/// the limit is NOT applied — a refused write consumes nothing, so the
/// window keeps yielding as many GOOD seats as the limit allows. A
/// request asking for more seats than an ENTIRE window holds is refused
/// against a freshly rolled window too, so it can never pass by waiting.
/// The `FOR UPDATE` row lock serializes concurrent replicas, so the count
/// is exact even when two replicas race the same window edge; no batch
/// of concurrent seats overshoots the limit.
pub async fn consume_write_budget_seats(
    conn: &mut PgConnection,
    org_id: uuid::Uuid,
    limit_per_min: u32,
    seats: u32,
) -> DbResult<BudgetSeatVerdict> {
    if limit_per_min == 0 || seats == 0 {
        return Ok(BudgetSeatVerdict::Admitted);
    }
    let mut tx = conn.begin().await?;
    // RLS: this admission row is tenant-scoped state, so the decision
    // rides the standard org GUC (same posture as every pool read).
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut *tx)
        .await?;
    // Ensure the window row exists (idempotent), then lock it and decide.
    sqlx::query(
        "INSERT INTO vault_write_budget_windows (org_id, window_started, seats, updated_at) \
         VALUES ($1, now(), 0, now()) \
         ON CONFLICT (org_id) DO NOTHING",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    let row: (chrono::DateTime<chrono::Utc>, i64, f64) = sqlx::query_as(
        "SELECT window_started, seats, \
                EXTRACT(EPOCH FROM (now() - window_started))::double precision \
         FROM vault_write_budget_windows WHERE org_id = $1 FOR UPDATE",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await?;
    let (window_started, seats_used, elapsed_secs) = row;
    let limit = i64::from(limit_per_min);
    let wanted = i64::from(seats);
    if elapsed_secs as i64 >= BUDGET_WINDOW_SECS as i64 || elapsed_secs < 0.0 {
        // Window rolled (the <0 arm guards the clock edge where a
        // just-inserted anchor reads negative under rounding). A request
        // bigger than a whole window can never be admitted, fresh or not.
        if wanted > limit {
            tracing::debug!(wanted, limit, org = %org_id,
                "vault budget: request exceeds a whole window");
            return Ok(BudgetSeatVerdict::Exceeded {
                retry_after_secs: i64::from(BUDGET_WINDOW_SECS),
            });
        }
        sqlx::query(
            "UPDATE vault_write_budget_windows SET window_started = now(), seats = $2, \
             updated_at = now() WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(wanted)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        tracing::debug!(?window_started, seats_used, elapsed_secs, wanted, org = %org_id,
            "vault budget: durable window rolled, {wanted} seat(s)");
        return Ok(BudgetSeatVerdict::Admitted);
    }
    if seats_used + wanted > limit {
        // Exceeded: the tx commits NOTHING (the refused write consumes
        // no seat), so the window keeps yielding good seats up to the
        // limit — identical to the in-memory bucket's refusal semantics.
        tracing::debug!(seats_used, wanted, elapsed_secs, org = %org_id,
            "vault budget: durable seat refused (window full)");
        return Ok(BudgetSeatVerdict::Exceeded {
            retry_after_secs: ((BUDGET_WINDOW_SECS as f64 - elapsed_secs).ceil() as i64).max(1),
        });
    }
    sqlx::query(
        "UPDATE vault_write_budget_windows SET seats = seats + $2, updated_at = now() \
         WHERE org_id = $1",
    )
    .bind(org_id)
    .bind(wanted)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::debug!(seats = seats_used + wanted, elapsed_secs, org = %org_id,
        "vault budget: durable seat admitted");
    Ok(BudgetSeatVerdict::Admitted)
}

/// Outcome of acquiring one bulk-ingestion pacing token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PacerWait {
    /// A token was consumed from the durable bucket; commit the next row.
    Emit,
    /// No token available yet — sleep this many milliseconds, then call
    /// [`acquire_bulk_token`] again. The hint is exact refill arithmetic
    /// on the DB clock; the bucket STATE survives restarts in the row,
    /// so pacing resumes where it left off instead of rebounding a
    /// fresh blocker.
    SleepMs(u64),
}

impl PacerWait {
    pub fn is_ready(&self) -> bool {
        matches!(self, PacerWait::Emit)
    }
}

/// Consumes one token from the org's durable token-bucket pacer, refilled
/// at `rows_per_sec` tokens/second up to a burst capacity of
/// `rows_per_sec` tokens (one full second's worth — a restarted driver
/// can never burst beyond the ceiling it left behind). A short-circuit
/// (`Emit`) when `rows_per_sec == 0` (documented unbounded mode, no
/// table touch).
///
/// The caller loops: `SleepMs(hint)` → sleep → call again. Tokens are
/// never allowed to go negative and the refill ledger is never clamped
/// below 0, so a lag behind the rate (e.g. after a pause) is repaid
/// faithfully by the source of truth (the DB clock), not the caller's
/// bookkeeping.
pub async fn acquire_bulk_token(
    conn: &mut PgConnection,
    org_id: uuid::Uuid,
    rows_per_sec: u32,
) -> DbResult<PacerWait> {
    if rows_per_sec == 0 {
        return Ok(PacerWait::Emit);
    }
    let burst = f64::from(rows_per_sec);
    let mut tx = conn.begin().await?;
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut *tx)
        .await?;
    // Seed the bucket empty; the first call pays the refill start price
    // (rate tokens accumulate per second from `last_refill` = now()).
    sqlx::query(
        "INSERT INTO vault_bulk_pacers (org_id, tokens, last_refill, updated_at) \
         VALUES ($1, 0, now(), now()) ON CONFLICT (org_id) DO NOTHING",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    let row: (f64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT tokens, last_refill FROM vault_bulk_pacers WHERE org_id = $1 FOR UPDATE",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await?;
    let (tokens, last_refill) = row;
    let refill_elapsed: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM (now() - last_refill))::double precision \
         FROM vault_bulk_pacers WHERE org_id = $1 FOR UPDATE",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await?;
    let _ = last_refill;
    let refilled = (tokens + refill_elapsed.max(0.0) * burst).min(burst);
    if refilled >= 1.0 {
        sqlx::query(
            "UPDATE vault_bulk_pacers SET tokens = $2 - 1, last_refill = now(), \
             updated_at = now() WHERE org_id = $1",
        )
        .bind(org_id)
        .bind(refilled)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(PacerWait::Emit);
    }
    // Not ready: advance the refill ledger to `refilled` (the consumed
    // pace is recorded even without a grant) and report the exact wait.
    let wait_secs = (1.0 - refilled) / burst;
    let wait_ms = (wait_secs * 1000.0).ceil() as u64;
    sqlx::query(
        "UPDATE vault_bulk_pacers SET tokens = $2, last_refill = now(), \
         updated_at = now() WHERE org_id = $1",
    )
    .bind(org_id)
    .bind(refilled)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(PacerWait::SleepMs(wait_ms.max(1)))
}
