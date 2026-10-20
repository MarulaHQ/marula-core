//! Tenant suspension notice storage (the private build tracker Wave 47 lane A7,
//! `p47-org-suspension-notice`, migration 0136).
//!
//! Three narrow seams over the three `orgs` columns:
//!
//!   * [`get`] reads the flag, its reason, and the notice interval.
//!   * [`set`] flags the org. It is IDEMPOTENT by construction: a second call
//!     on an already-flagged org keeps the original `suspended_at` (so the
//!     notice countdown does not restart on a retry) and only refreshes the
//!     reason and interval.
//!   * [`clear`] removes the flag, the reason, and the interval together.
//!
//! THE POSTURE — NOTICE, NEVER A BLOCK. Nothing here reads a flag to decide
//! whether an operation is allowed, and no caller in the core does either.
//! A flagged org stays fully functional; the flag exists so a surface can
//! tell the customer what is happening and offer the way to take their data
//! out. Any future lane that gates behaviour on [`OrgSuspension::is_flagged`]
//! is changing the product decision this lane was built on.
//!
//! Everything runs on runtime `sqlx::query` runners rather than the `query!`
//! macros: the columns this migration adds are not in the offline macro cache
//! yet, and a compile-time cache would need a `sqlx prepare` pass for a shape
//! it cannot know. The callers bind the RLS scope (`app.org_id`) before every
//! statement, so the `org_self` policy on `orgs` does the scoping — these
//! functions never filter by org themselves beyond the primary key they are
//! handed.
//!
//! The interval crosses the wire as whole seconds in both directions
//! (`EXTRACT(EPOCH …)` out, `make_interval(secs => …)` in). Seconds are what
//! a countdown needs, and `make_interval` cannot be handed a bare NULL, so
//! the write wraps it in a CASE.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::Postgres;
use uuid::Uuid;

/// One org's suspension notice state. The reason and the flag move together
/// (migration 0136's `orgs_suspension_flag_coherent` constraint); the
/// interval is independent, and `None` means the notice is due immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgSuspension {
    pub org_id: Uuid,
    pub suspended_at: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    /// Whole seconds after `suspended_at` before the notice starts showing.
    pub notify_after_secs: Option<i64>,
}

impl OrgSuspension {
    /// Whether the org is flagged. INFORMATIONAL — see the module note: no
    /// authorization decision, route guard, or feature gate may branch on
    /// this value.
    pub fn is_flagged(&self) -> bool {
        self.suspended_at.is_some()
    }
}

/// The columns every statement below selects, so the three rows (read, set,
/// clear) decode through one shape.
const COLUMNS: &str = "id, suspended_at, suspension_reason, \
                       EXTRACT(EPOCH FROM suspension_notify_after)::bigint AS notify_after_secs";

fn row_to_suspension(row: &sqlx::postgres::PgRow) -> DbResult<OrgSuspension> {
    use sqlx::Row;
    Ok(OrgSuspension {
        org_id: row.try_get("id")?,
        suspended_at: row.try_get("suspended_at")?,
        reason: row.try_get("suspension_reason")?,
        notify_after_secs: row.try_get("notify_after_secs")?,
    })
}

/// The org's current flag, or `None` when the org is not visible under the
/// caller's RLS scope (an unbound or cross-org session sees nothing).
pub async fn get<'e, E>(executor: E, org_id: Uuid) -> DbResult<Option<OrgSuspension>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM orgs WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .fetch_optional(executor)
        .await?;
    match row {
        Some(ref r) => Ok(Some(row_to_suspension(r)?)),
        None => Ok(None),
    }
}

/// Flags the org (or refreshes the reason and interval on an already-flagged
/// one) and returns the resulting state.
///
/// `suspended_at` is `COALESCE`d against the stored value, so the call is
/// idempotent: a retry, a double click, or a replayed request cannot restart
/// the notice countdown. `reason` must be one of the closed set the schema
/// accepts; the API validates it before it gets here.
pub async fn set(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    reason: &str,
    notify_after_secs: Option<i64>,
) -> DbResult<OrgSuspension> {
    let sql = format!(
        "UPDATE orgs \
            SET suspended_at = COALESCE(suspended_at, now()), \
                suspension_reason = $2, \
                suspension_notify_after = CASE \
                  WHEN $3::bigint IS NULL THEN NULL \
                  ELSE make_interval(secs => $3::bigint) \
                END \
          WHERE id = $1 \
        RETURNING {COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(reason)
        .bind(notify_after_secs)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_suspension(&row)
}

/// Clears the flag, the reason, and the interval in one statement, and
/// returns the cleared state. `DbError::NotFound` when the org is not visible
/// under the caller's scope.
pub async fn clear(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<OrgSuspension> {
    let sql = format!(
        "UPDATE orgs \
            SET suspended_at = NULL, \
                suspension_reason = NULL, \
                suspension_notify_after = NULL \
          WHERE id = $1 \
        RETURNING {COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_suspension(&row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(notify_after_secs: Option<i64>) -> OrgSuspension {
        OrgSuspension {
            org_id: Uuid::nil(),
            suspended_at: Some(DateTime::<Utc>::UNIX_EPOCH),
            reason: Some("billing_lapsed".into()),
            notify_after_secs,
        }
    }

    #[test]
    fn is_flagged_tracks_only_the_timestamp() {
        let clear = OrgSuspension {
            org_id: Uuid::nil(),
            suspended_at: None,
            reason: None,
            notify_after_secs: None,
        };
        assert!(!clear.is_flagged());
        assert!(flagged(None).is_flagged());
        // The interval is an independent knob: no interval still means
        // flagged, and the notice is simply due immediately.
        assert!(flagged(Some(0)).is_flagged());
    }

    /// The columns fragment is interpolated into three statements, so a
    /// typo'd alias would surface as a decode failure at runtime rather than
    /// at compile time (these paths deliberately skip the `query!` macros).
    /// Pin the alias the decoder asks for.
    #[test]
    fn columns_fragment_carries_the_decoded_alias() {
        assert!(COLUMNS.contains("AS notify_after_secs"));
        assert!(COLUMNS.contains("suspension_reason"));
        assert!(COLUMNS.contains("suspended_at"));
    }
}
