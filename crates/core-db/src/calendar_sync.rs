//! Calendar-sync persistence (Wave 14 lane 2): sync connections and the
//! inbound external-event mirror.
//!
//! Purpose: CRUD for `calendar_sync_connections`, token bookkeeping, poll
//! state, and the transactional apply that mirrors external events onto
//! the 2.12 availability foundation. [`apply_sync`] is the duplicate +
//! conflict rule engine: highest external SEQUENCE wins per (connection,
//! UID); non-busy events feed nothing; vanished events release their
//! exclusion row; overlaps with live appointments are counted and
//! reported, never blocked.
//!
//! Non-purpose: provider transport/parsing (core-api's calendar_sync
//! route module owns the `CalendarSyncProvider` seam), conflict
//! *decisions* (core_domain::scheduling — busy time simply becomes
//! time-off, so the existing book-time double-check decides), outbox
//! emission (routes emit inside their transaction).

use crate::{DbError, DbResult};
use core_domain::entities::{
    CalendarSyncConnection, CalendarSyncDirection, CalendarSyncEvent, CalendarSyncStatus,
    ExternalCalendarEvent, SyncOutcome,
};
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

const CONNECTION_COLS: &str = "id, org_id, user_id, name, provider, direction::text AS direction, \
     status::text AS status, feed_url, feed_token_hash, sync_token, last_synced_at, \
     last_error, created_at, updated_at";

fn row_to_connection(row: &sqlx::postgres::PgRow) -> DbResult<CalendarSyncConnection> {
    let direction: String = row.get("direction");
    let status: String = row.get("status");
    let feed_token_hash: Option<String> = row.get("feed_token_hash");
    Ok(CalendarSyncConnection {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        provider: row.get("provider"),
        direction: CalendarSyncDirection::parse(&direction).ok_or_else(|| {
            crate::scheduling::unknown_enum("calendar_sync_direction", &direction)
        })?,
        status: CalendarSyncStatus::parse(&status)
            .ok_or_else(|| crate::scheduling::unknown_enum("calendar_sync_status", &status))?,
        feed_url: row.get("feed_url"),
        // The raw token is never stored; only its path-with-hash shape is
        // rendered by the route layer.
        feed_path: feed_token_hash
            .as_ref()
            .map(|hash| format!("/api/calendar-sync/feed/{}", &hash[..hash.len().min(12)])),
        last_synced_at: row.get("last_synced_at"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

use sqlx::Row;

/// Inserts a fully-formed connection. Caller generates ids/timestamps.
pub async fn create_connection<'e, E>(executor: E, c: &CalendarSyncConnection) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO calendar_sync_connections (id, org_id, user_id, name, provider,
                                                   direction, status, feed_url)
           VALUES ($1, $2, $3, $4, $5, $6::text::calendar_sync_direction,
                   $7::text::calendar_sync_status, $8)"#,
        c.id,
        c.org_id,
        c.user_id,
        c.name,
        c.provider,
        c.direction.as_str(),
        c.status.as_str(),
        c.feed_url,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_connections(
    executor: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<CalendarSyncConnection>> {
    let sql = format!(
        "SELECT {CONNECTION_COLS} FROM calendar_sync_connections
         WHERE org_id = $1 ORDER BY created_at"
    );
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.iter().map(row_to_connection).collect()
}

pub async fn get_connection(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<CalendarSyncConnection> {
    let sql = format!(
        "SELECT {CONNECTION_COLS} FROM calendar_sync_connections
         WHERE org_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    row_to_connection(&row)
}

/// Patch the editable fields (name/direction/feed_url/status). `None`
/// leaves a field untouched; `status` always applies when given.
pub async fn update_connection(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    direction: Option<CalendarSyncDirection>,
    feed_url: Option<Option<&str>>,
    status: Option<CalendarSyncStatus>,
) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE calendar_sync_connections SET
             name = COALESCE($3, name),
             direction = COALESCE($4::text::calendar_sync_direction, direction),
             feed_url = CASE WHEN $5 THEN $6 ELSE feed_url END,
             status = COALESCE($7::text::calendar_sync_status, status),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        name,
        direction.map(|d| d.as_str()),
        feed_url.is_some(),
        feed_url.flatten(),
        status.map(|s| s.as_str()),
    )
    .execute(executor)
    .await?
    .rows_affected()
    .checked_sub(1)
    .ok_or(DbError::NotFound)
    .map(|_| ())
}

pub async fn delete_connection(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    // Mirror rows cascade (and through them the exclusion rows the sync
    // owns — external busy time frees the calendar when the connection
    // goes away).
    sqlx::query!(
        "DELETE FROM calendar_sync_connections WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(executor)
    .await?
    .rows_affected()
    .checked_sub(1)
    .ok_or(DbError::NotFound)
    .map(|_| ())
}

/// Sets (or rotates) the outbound feed token hash. The raw token is
/// returned exactly once by the route layer and never persisted.
pub async fn set_feed_token(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    token_hash: &str,
) -> DbResult<()> {
    sqlx::query!(
        "UPDATE calendar_sync_connections SET feed_token_hash = $3, updated_at = now()
         WHERE org_id = $1 AND id = $2",
        org_id,
        id,
        token_hash
    )
    .execute(executor)
    .await?
    .rows_affected()
    .checked_sub(1)
    .ok_or(DbError::NotFound)
    .map(|_| ())
}

/// Resolves an outbound feed by token hash (unauthenticated feed surface;
/// the caller scopes the read afterwards). Returns (org_id, connection_id).
pub async fn resolve_feed_target(
    executor: &mut PgConnection,
    token_hash: &str,
) -> DbResult<(Uuid, Uuid)> {
    let row = sqlx::query!(
        "SELECT org_id, id FROM calendar_sync_connections
         WHERE feed_token_hash = $1 AND status <> 'disabled'",
        token_hash
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok((row.org_id, row.id))
}

/// Records a poll outcome: token refresh, health status, error copy.
pub async fn record_sync_result(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    sync_token: Option<&str>,
    ok: bool,
    error: Option<&str>,
    synced_at: chrono::DateTime<chrono::Utc>,
) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE calendar_sync_connections SET
             sync_token = COALESCE($3, sync_token),
             status = CASE WHEN $4 THEN 'active'::calendar_sync_status
                           ELSE 'error'::calendar_sync_status END,
             last_error = $5,
             last_synced_at = CASE WHEN $4 THEN $6 ELSE last_synced_at END,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        sync_token,
        ok,
        error,
        synced_at,
    )
    .execute(executor)
    .await?;
    Ok(())
}

const EVENT_COLS: &str = "id, org_id, connection_id, user_id, external_uid, sequence, title, \
     starts_at, ends_at, exclusion_id, first_seen_at, last_seen_at";

pub async fn list_events(
    executor: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
) -> DbResult<Vec<CalendarSyncEvent>> {
    let sql = format!(
        "SELECT {EVENT_COLS} FROM calendar_sync_events
         WHERE org_id = $1 AND connection_id = $2 ORDER BY starts_at"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(connection_id)
        .fetch_all(executor)
        .await?;
    rows.iter().map(row_to_event).collect()
}

fn row_to_event(row: &sqlx::postgres::PgRow) -> DbResult<CalendarSyncEvent> {
    Ok(CalendarSyncEvent {
        id: row.get("id"),
        org_id: row.get("org_id"),
        connection_id: row.get("connection_id"),
        user_id: row.get("user_id"),
        external_uid: row.get("external_uid"),
        sequence: row.get("sequence"),
        title: row.get("title"),
        starts_at: row.get("starts_at"),
        ends_at: row.get("ends_at"),
        exclusion_id: row.get("exclusion_id"),
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
    })
}

/// The transactional sync apply. `events` is the provider's NORMALIZED
/// full snapshot of the external calendar. Rules (documented in
/// migration 0074):
/// - duplicate: a known UID with an equal-or-higher external SEQUENCE is
///   a duplicate re-announcement — counted, not re-applied;
/// - busy events map 1:1 onto `calendar_exclusions` rows (moved events
///   update their row in place, never duplicate it);
/// - non-busy/cancelled events release their exclusion row;
/// - vanished events (not in the snapshot) release theirs;
/// - overlaps with live appointments are COUNTED (reported via the
///   `calendar.sync_completed` event), never blocked.
pub async fn apply_sync(
    executor: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    user_id: Uuid,
    events: &[ExternalCalendarEvent],
) -> DbResult<SyncOutcome> {
    let mut outcome = SyncOutcome::default();
    let mut tx = executor.begin().await?;

    // Load the current mirror for this connection.
    let existing: std::collections::HashMap<String, CalendarSyncEvent> =
        list_events(&mut tx, org_id, connection_id)
            .await?
            .into_iter()
            .map(|e| (e.external_uid.clone(), e))
            .collect();

    let mut seen_uids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for event in events {
        seen_uids.insert(event.external_uid.clone());
        if !event.is_busy() {
            // Cancelled/transparent: release any mirror row that used to
            // be busy, keep the mirror as a non-busy row (no exclusion).
            if let Some(prev) = existing.get(&event.external_uid) {
                release_exclusion(&mut tx, org_id, prev).await?;
                if event.sequence >= prev.sequence {
                    sqlx::query!(
                        "UPDATE calendar_sync_events SET sequence = $3, title = $4,
                             starts_at = $5, ends_at = $6, exclusion_id = NULL,
                             last_seen_at = now()
                         WHERE org_id = $1 AND connection_id = $2 AND external_uid = $7",
                        org_id,
                        connection_id,
                        event.sequence,
                        event.title,
                        event.starts_at,
                        event.ends_at,
                        event.external_uid,
                    )
                    .execute(&mut *tx)
                    .await?;
                    outcome.updated += 1;
                }
            } else {
                sqlx::query!(
                    "INSERT INTO calendar_sync_events (id, org_id, connection_id, user_id,
                                                       external_uid, sequence, title,
                                                       starts_at, ends_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                    Uuid::now_v7(),
                    org_id,
                    connection_id,
                    user_id,
                    event.external_uid,
                    event.sequence,
                    event.title,
                    event.starts_at,
                    event.ends_at,
                )
                .execute(&mut *tx)
                .await?;
            }
            continue;
        }

        if let Some(prev) = existing.get(&event.external_uid) {
            if event.sequence < prev.sequence {
                outcome.duplicates += 1;
                continue;
            }
            if event.sequence == prev.sequence
                && prev.starts_at == event.starts_at
                && prev.ends_at == event.ends_at
                && prev.title == event.title
                && prev.exclusion_id.is_some()
            {
                // Unchanged re-announcement — refresh the seen stamp only.
                sqlx::query!(
                    "UPDATE calendar_sync_events SET last_seen_at = now()
                     WHERE org_id = $1 AND connection_id = $2 AND external_uid = $3",
                    org_id,
                    connection_id,
                    event.external_uid,
                )
                .execute(&mut *tx)
                .await?;
                outcome.duplicates += 1;
                continue;
            }
            // Changed: move the exclusion row in place (never duplicate).
            let exclusion_id = prev.exclusion_id.unwrap_or_else(Uuid::now_v7);
            if prev.exclusion_id.is_some() {
                sqlx::query!(
                    "UPDATE calendar_exclusions SET starts_at = $3, ends_at = $4, reason = $5
                     WHERE org_id = $1 AND id = $2",
                    org_id,
                    exclusion_id,
                    event.starts_at,
                    event.ends_at,
                    event
                        .title
                        .as_deref()
                        .map(|t| format!("Calendar sync: {t}")),
                )
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query!(
                    "INSERT INTO calendar_exclusions (id, org_id, user_id, starts_at, ends_at,
                                                      reason)
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    exclusion_id,
                    org_id,
                    user_id,
                    event.starts_at,
                    event.ends_at,
                    event
                        .title
                        .as_deref()
                        .map(|t| format!("Calendar sync: {t}")),
                )
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query!(
                "UPDATE calendar_sync_events SET sequence = $3, title = $4, starts_at = $5,
                     ends_at = $6, exclusion_id = $7, last_seen_at = now()
                 WHERE org_id = $1 AND connection_id = $2 AND external_uid = $8",
                org_id,
                connection_id,
                event.sequence,
                event.title,
                event.starts_at,
                event.ends_at,
                exclusion_id,
                event.external_uid,
            )
            .execute(&mut *tx)
            .await?;
            outcome.updated += 1;
            continue;
        }

        // New busy event: mirror row + exclusion row together.
        let exclusion_id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO calendar_exclusions (id, org_id, user_id, starts_at, ends_at, reason)
             VALUES ($1, $2, $3, $4, $5, $6)",
            exclusion_id,
            org_id,
            user_id,
            event.starts_at,
            event.ends_at,
            event
                .title
                .as_deref()
                .map(|t| format!("Calendar sync: {t}")),
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO calendar_sync_events (id, org_id, connection_id, user_id, external_uid,
                                               sequence, title, starts_at, ends_at, exclusion_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            Uuid::now_v7(),
            org_id,
            connection_id,
            user_id,
            event.external_uid,
            event.sequence,
            event.title,
            event.starts_at,
            event.ends_at,
            exclusion_id,
        )
        .execute(&mut *tx)
        .await?;
        outcome.imported += 1;
    }

    // Released: mirror rows not present in this snapshot (the external
    // event disappeared). Their exclusion rows cascade away.
    for uid in existing.keys() {
        if seen_uids.contains(uid) {
            continue;
        }
        sqlx::query!(
            "DELETE FROM calendar_sync_events WHERE org_id = $1 AND connection_id = $2
                 AND external_uid = $3",
            org_id,
            connection_id,
            uid,
        )
        .execute(&mut *tx)
        .await?;
        outcome.released += 1;
    }

    // Conflict report: imported/updated busy events overlapping a live
    // appointment on the same calendar. Counted, never blocked.
    outcome.overlaps = sqlx::query_scalar!(
        r#"SELECT count(*)::bigint AS "n!" FROM calendar_sync_events e
           JOIN appointments a ON a.org_id = e.org_id AND a.user_id = e.user_id
             AND a.status <> 'cancelled'
             AND a.starts_at < e.ends_at AND e.starts_at < a.ends_at
           WHERE e.org_id = $1 AND e.connection_id = $2 AND e.exclusion_id IS NOT NULL"#,
        org_id,
        connection_id,
    )
    .fetch_one(&mut *tx)
    .await? as u32;

    tx.commit().await?;
    Ok(outcome)
}

/// Drops the exclusion row a mirror event currently feeds (slot freed).
async fn release_exclusion(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: Uuid,
    prev: &CalendarSyncEvent,
) -> DbResult<()> {
    if let Some(exclusion_id) = prev.exclusion_id {
        sqlx::query!(
            "DELETE FROM calendar_exclusions WHERE org_id = $1 AND id = $2",
            org_id,
            exclusion_id
        )
        .execute(&mut **tx)
        .await?;
        sqlx::query!(
            "UPDATE calendar_sync_events SET exclusion_id = NULL
             WHERE org_id = $1 AND connection_id = $2 AND external_uid = $3",
            org_id,
            prev.connection_id,
            prev.external_uid,
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The user's CURRENTLY-FED busy spans (mirror rows with a live
/// exclusion): the piece of the 2.12 book-time double-check that makes
/// external busy time real. The scheduling-links preview and the
/// book-time gate extend their busy set with these spans, exactly like
/// on-call blocks.
pub async fn list_busy_spans_between<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query!(
        "SELECT starts_at, ends_at FROM calendar_sync_events
         WHERE org_id = $1 AND user_id = $2
           AND exclusion_id IS NOT NULL
           AND starts_at < $3 AND ends_at > $4",
        org_id,
        user_id,
        to,
        from,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(|r| (r.starts_at, r.ends_at)).collect())
}
