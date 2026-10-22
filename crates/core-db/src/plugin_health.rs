//! Plugin install health: consecutive worker failures and the quarantine they
//! trigger (wave 53 lane 3, migration 0154).
//!
//! The plugin pool (`plugin_host::runtime`) owns the RULE — how many failures
//! in a row quarantine an install, and what the operator's clear means. This
//! module owns the SQL and the row shape, so the two do not drift: the threshold
//! and the reason string are arguments, never constants repeated here.
//!
//! Query style follows `ai_policy` and `byok`: plain `sqlx::query` and
//! `query_as` with an explicit [`FromRow`] rather than the compile-time
//! `query!` macro, so the module builds without the offline query cache and a
//! schema change cannot silently pass because a stale `.sqlx` blob says so.
//!
//! Every function takes the caller's connection, which must already carry
//! `app.org_id` (see [`crate::set_org`]). The row's `org_id` is derived from
//! `plugin_installs` inside the insert rather than bound from the caller, so a
//! frame whose (org, install) pair disagrees cannot create a row that belongs
//! to one tenant and names another.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use crate::DbResult;

/// One install's health row.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct InstallHealth {
    /// Worker failures in a row. A successful invocation resets it to zero.
    pub consecutive_failures: i32,
    /// `None` while the install is not quarantined.
    pub quarantined_at: Option<DateTime<Utc>>,
    /// The static refusal string that quarantined it (`plugin_worker_failed`).
    pub quarantine_reason: Option<String>,
    /// When an operator last cleared a quarantine, if ever.
    pub cleared_at: Option<DateTime<Utc>>,
}

impl InstallHealth {
    /// Whether the install is quarantined right now.
    pub fn is_quarantined(&self) -> bool {
        self.quarantined_at.is_some()
    }
}

/// Reads one install's health, or `None` when no failure has ever been
/// recorded for it (the common case, and the reason the read is one indexed
/// primary-key lookup).
pub async fn load(conn: &mut PgConnection, install_id: Uuid) -> DbResult<Option<InstallHealth>> {
    let row = sqlx::query_as::<_, InstallHealth>(
        r#"SELECT consecutive_failures, quarantined_at, quarantine_reason, cleared_at
             FROM plugin_install_health
            WHERE install_id = $1"#,
    )
    .bind(install_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// Records one worker failure and returns the install's health afterwards.
///
/// A row that is already quarantined is left alone: the counter stops moving
/// while an operator has the install stopped, so the number in the admin screen
/// is the number that caused the quarantine. `threshold` comes from the pool;
/// `reason` is the static refusal string the audit row carries.
pub async fn record_failure(
    conn: &mut PgConnection,
    install_id: Uuid,
    reason: &str,
    threshold: i32,
) -> DbResult<Option<InstallHealth>> {
    let row = sqlx::query_as::<_, InstallHealth>(
        r#"INSERT INTO plugin_install_health (install_id, org_id, consecutive_failures, updated_at)
           SELECT i.id, i.org_id, 1, now()
             FROM plugin_installs i
            WHERE i.id = $1
           ON CONFLICT (install_id) DO UPDATE
             SET consecutive_failures = plugin_install_health.consecutive_failures + 1,
                 updated_at = now()
             WHERE plugin_install_health.quarantined_at IS NULL
           RETURNING consecutive_failures, quarantined_at, quarantine_reason, cleared_at"#,
    )
    .bind(install_id)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else {
        // The upsert returns no row in two different situations, and they must
        // not be confused: the install is already quarantined (the `WHERE` on
        // the conflict arm stopped the increment), or the install is not
        // visible under this tenant's RLS scope (the `SELECT` inserted
        // nothing). One read separates them, and it only runs on the second
        // failure onward.
        return load(conn, install_id).await;
    };
    if row.is_quarantined() || row.consecutive_failures < threshold {
        return Ok(Some(row));
    }

    let quarantined = sqlx::query_as::<_, InstallHealth>(
        r#"UPDATE plugin_install_health
              SET quarantined_at = now(), quarantine_reason = $2, updated_at = now()
            WHERE install_id = $1 AND quarantined_at IS NULL
          RETURNING consecutive_failures, quarantined_at, quarantine_reason, cleared_at"#,
    )
    .bind(install_id)
    .bind(reason)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(Some(quarantined.unwrap_or(row)))
}

/// Resets the consecutive-failure counter after a successful invocation. The
/// guard keeps it off the happy path: an install with a zero count issues no
/// write at all.
pub async fn record_success(conn: &mut PgConnection, install_id: Uuid) -> DbResult<()> {
    sqlx::query(
        r#"UPDATE plugin_install_health
              SET consecutive_failures = 0, updated_at = now()
            WHERE install_id = $1 AND consecutive_failures > 0"#,
    )
    .bind(install_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Clears a quarantine: the counter goes to zero, the quarantine goes away and
/// the clear is attributed. Returns whether a quarantine was actually lifted,
/// so an admin route can answer "already clear" without a second read.
pub async fn clear_quarantine(
    conn: &mut PgConnection,
    install_id: Uuid,
    cleared_by: Option<Uuid>,
) -> DbResult<bool> {
    let result = sqlx::query(
        r#"UPDATE plugin_install_health
              SET consecutive_failures = 0,
                  quarantined_at = NULL,
                  quarantine_reason = NULL,
                  cleared_at = now(),
                  cleared_by = $2,
                  updated_at = now()
            WHERE install_id = $1 AND quarantined_at IS NOT NULL"#,
    )
    .bind(install_id)
    .bind(cleared_by)
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}
