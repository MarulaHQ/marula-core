//! Vault scale-out admission (Wave 19 lane 1, migration 0094): the durable
//! cross-replica layer in front of the Wave 16 lane 9 in-memory budget.
//!
//! Layering:
//! * `routes::secrets::check_vault_budget` (in-memory fixed-window
//!   bucket) stays the single-replica FAST PATH — consulted first; a
//!   local breach is a guaranteed breach org-wide (the local window can
//!   be no larger than the real one), so it answers 429 without a DB
//!   round-trip.
//! * [`ensure_vault_budget`] then consumes a DURABLE seat
//!   (`vault_write_budget_windows`, one `FOR UPDATE`-locked row per org)
//!   whose verdict is authoritative when the local path passes — two
//!   replicas now share one budget instead of each granting N×limit.
//! * [`pace_bulk_rows`] is the durable token-bucket pacer for the A2
//!   bulk-ingestion worker path: pacing state lives in
//!   `vault_bulk_pacers`, so a restarted bulk driver resumes its pace
//!   (no restart rebound) and replicas pace one shared bucket.
//!
//! Every call uses an org-GUC-bound transaction on its acquired pool
//! connection (RLS posture identical to every tenant read) — no bare
//! unscoped pool reads.

use crate::error::{ApiError, ApiResult};
use crate::routes::secrets::check_vault_budget;
use crate::state::AppState;
use uuid::Uuid;

/// One admission decision for a vault write: in-memory fast path first,
/// then the durable seat. `limit == 0` disables everywhere (no DB touch).
pub async fn ensure_vault_budget(state: &AppState, org_id: Uuid) -> ApiResult<()> {
    // Fast path: the in-process bucket — a local breach is conclusive.
    check_vault_budget(state, org_id)?;
    let limit_per_min = state.config.vault_write_budget_per_min;
    if limit_per_min == 0 {
        return Ok(());
    }
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    match core_db::vault_scale::consume_write_budget_seat(&mut conn, org_id, limit_per_min).await {
        Ok(core_db::vault_scale::BudgetSeatVerdict::Admitted) => Ok(()),
        Ok(core_db::vault_scale::BudgetSeatVerdict::Exceeded { retry_after_secs }) => {
            Err(ApiError::RateLimited { retry_after_secs })
        }
        Err(e) => Err(ApiError::from(e)),
    }
}

/// Consumes one bulk-pacing token, sleeping the DB-clock hint until the
/// durable bucket grants it. Returns when a token has been consumed —
/// the caller commits its next row immediately. `rows_per_sec == 0`
/// (paced-off) short-circuits with no DB touch.
///
/// The wait loop releases its pool connection between sleeps (a paced
/// request must not hold a connection idle) and sleeps in steps capped
/// well under the hinted duration so a bucket refill signals promptly.
pub async fn pace_bulk_token(state: &AppState, org_id: Uuid) -> ApiResult<()> {
    let rows_per_sec = state.config.vault_bulk_rows_per_sec;
    if rows_per_sec == 0 {
        return Ok(());
    }
    loop {
        let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
        let wait = core_db::vault_scale::acquire_bulk_token(&mut conn, org_id, rows_per_sec)
            .await
            .map_err(ApiError::from)?;
        drop(conn);
        match wait {
            core_db::vault_scale::PacerWait::Emit => return Ok(()),
            core_db::vault_scale::PacerWait::SleepMs(hint_ms) => {
                let step = hint_ms.min(50);
                let sleep_ms = if step == 0 { 1 } else { step };
                tracing::debug!(org = %org_id, hint_ms, sleep_ms = sleep_ms,
                    "bulk pacer: waiting on the durable token bucket");
                tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
            }
        }
    }
}
