//! Notification center persistence: inbox rows, read state, and the
//! preferences chain (user override → org default → built-in).
//!
//! Purpose: compile-time-checked queries for listing (keyset/cursor),
//! unread counts, single/bulk/all mark-read, per-user channel+quiet-hours
//! preferences, and org-level defaults rows.
//!
//! Non-purpose: no fan-out decisions (core-domain owns prefs × clock
//! rules), no event emission (core-api emits through the outbox), no HTTP.

use crate::{DbError, DbResult};
use core_domain::entities::{Notification, NotificationPreference};
use sqlx::Postgres;
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn insert<'e, E>(
    executor: E,
    id: Uuid,
    org_id: Uuid,
    user_id: Uuid,
    kind: &str,
    ticket_id: Option<Uuid>,
    title: &str,
    body: Option<&str>,
) -> DbResult<Notification>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO notifications (id, org_id, user_id, kind, ticket_id, title, body)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id, org_id, user_id, kind, ticket_id, title, body, read_at, created_at"#,
        id,
        org_id,
        user_id,
        kind,
        ticket_id,
        title,
        body
    )
    .fetch_one(executor)
    .await?;
    Ok(Notification {
        id: row.id,
        org_id: row.org_id,
        user_id: row.user_id,
        kind: row.kind.parse().map_err(|e: core_domain::DomainError| {
            DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
        })?,
        ticket_id: row.ticket_id,
        title: row.title,
        body: row.body,
        read_at: row.read_at,
        created_at: row.created_at,
    })
}

/// Recipients for an inbound malware-quarantine notice (Wave 36 lane 3):
/// active org owners and admins, bounded so one noisy inbound loop cannot
/// fan out without limit. Runtime-checked SQL on purpose — this seam must
/// not require a sqlx offline-cache regeneration to land.
pub async fn scan_detection_recipients<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT u.id FROM users u
           WHERE u.org_id = $1 AND u.is_active
             AND EXISTS (
                SELECT 1 FROM user_roles ur
                JOIN roles r ON r.id = ur.role_id
                WHERE ur.user_id = u.id AND r.builtin IN ('owner','admin'))
           ORDER BY u.id
           LIMIT 50"#,
    )
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

fn notification_from_row(row: sqlx::postgres::PgRow) -> DbResult<Notification> {
    use sqlx::Row;
    Ok(Notification {
        id: row.get("id"),
        org_id: row.get("org_id"),
        user_id: row.get("user_id"),
        kind: row
            .get::<String, _>("kind")
            .parse()
            .map_err(|e: core_domain::DomainError| {
                DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
            })?,
        ticket_id: row.get("ticket_id"),
        title: row.get("title"),
        body: row.get("body"),
        read_at: row.get("read_at"),
        created_at: row.get("created_at"),
    })
}

pub async fn list_for_user<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    unread_only: bool,
    limit: i64,
) -> DbResult<Vec<Notification>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = "SELECT id, org_id, user_id, kind, ticket_id, title, body, read_at, created_at \
               FROM notifications \
               WHERE org_id = $1 AND user_id = $2 AND ($3 = false OR read_at IS NULL) \
               ORDER BY created_at DESC LIMIT $4";
    let rows = sqlx::query(sql)
        .bind(org_id)
        .bind(user_id)
        .bind(unread_only)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(notification_from_row).collect()
}

/// Opaque keyset cursor: `(created_at, id)` of the last page's last row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotificationCursor {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub id: Uuid,
}

/// One keyset-paginated page for the notification dock. `before = None`
/// starts at the newest row; pass the `next_cursor` of the previous page to
/// continue. Returns the page plus the cursor for the following page
/// (`None` = exhausted).
pub async fn list_page<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    unread_only: bool,
    before: Option<NotificationCursor>,
    limit: i64,
) -> DbResult<(Vec<Notification>, Option<NotificationCursor>)>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let fetch = limit + 1; // one extra row decides whether a next page exists
    let rows = sqlx::query(
        r#"SELECT id, org_id, user_id, kind, ticket_id, title, body, read_at, created_at
           FROM notifications
           WHERE org_id = $1 AND user_id = $2 AND ($3 = false OR read_at IS NULL)
             AND ($4::timestamptz IS NULL OR (created_at, id) < ($4, $5))
           ORDER BY created_at DESC, id DESC
           LIMIT $6"#,
    )
    .bind(org_id)
    .bind(user_id)
    .bind(unread_only)
    .bind(before.map(|c| c.created_at))
    .bind(before.map(|c| c.id))
    .bind(fetch)
    .fetch_all(executor)
    .await?;

    let mut page = rows;
    let has_more = page.len() as i64 > limit;
    if has_more {
        page.truncate(limit as usize);
    }
    let items: Vec<Notification> = page
        .into_iter()
        .map(notification_from_row)
        .collect::<DbResult<_>>()?;
    let next_cursor = if has_more {
        items.last().map(|n| NotificationCursor {
            created_at: n.created_at,
            id: n.id,
        })
    } else {
        None
    };
    Ok((items, next_cursor))
}

pub async fn mark_read<'e, E>(executor: E, org_id: Uuid, user_id: Uuid, id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE notifications SET read_at = now()
           WHERE org_id = $1 AND user_id = $2 AND id = $3 AND read_at IS NULL"#,
        org_id,
        user_id,
        id
    )
    .execute(executor)
    .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Marks the given ids read for one user; returns how many rows flipped
/// from unread to read (already-read and foreign ids are silently skipped
/// so bulk retries are idempotent).
pub async fn mark_read_many<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    ids: &[Uuid],
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if ids.is_empty() {
        return Ok(0);
    }
    let res = sqlx::query!(
        r#"UPDATE notifications SET read_at = now()
           WHERE org_id = $1 AND user_id = $2 AND read_at IS NULL AND id = ANY($3)"#,
        org_id,
        user_id,
        ids
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// Marks every unread notification of the user read; returns the flip
/// count (0 when there was nothing to do).
pub async fn mark_all_read<'e, E>(executor: E, org_id: Uuid, user_id: Uuid) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let res = sqlx::query!(
        r#"UPDATE notifications SET read_at = now()
           WHERE org_id = $1 AND user_id = $2 AND read_at IS NULL"#,
        org_id,
        user_id
    )
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

pub async fn unread_count<'e, E>(executor: E, org_id: Uuid, user_id: Uuid) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS c FROM notifications
           WHERE org_id = $1 AND user_id = $2 AND read_at IS NULL"#,
        org_id,
        user_id
    )
    .fetch_one(executor)
    .await?;
    Ok(row.c.unwrap_or(0))
}

pub async fn upsert_preference<'e, E>(executor: E, p: &NotificationPreference) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let kind = p.kind.as_str().to_string();
    sqlx::query!(
        r#"INSERT INTO notification_preferences
             (user_id, kind, in_app, email, quiet_from, quiet_until, timezone)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (user_id, kind) DO UPDATE SET
             in_app = $3, email = $4, quiet_from = $5, quiet_until = $6,
             timezone = $7"#,
        p.user_id,
        kind,
        p.in_app,
        p.email,
        p.quiet_from,
        p.quiet_until,
        p.timezone,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_preferences<'e, E>(
    executor: E,
    user_id: Uuid,
) -> DbResult<Vec<NotificationPreference>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT user_id, kind, in_app, email, quiet_from, quiet_until, timezone
           FROM notification_preferences WHERE user_id = $1 ORDER BY kind"#,
        user_id
    )
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(NotificationPreference {
                user_id: row.user_id,
                kind: row.kind.parse().map_err(|e: core_domain::DomainError| {
                    DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
                })?,
                in_app: row.in_app,
                email: row.email,
                quiet_from: row.quiet_from,
                quiet_until: row.quiet_until,
                timezone: row.timezone,
            })
        })
        .collect()
}

/// An org-level defaults row: the middle link of the preference chain.
/// Absent (org, kind) rows mean "use built-in defaults".
#[derive(Debug, Clone)]
pub struct OrgDefaultRow {
    pub org_id: Uuid,
    pub kind: String,
    pub in_app: bool,
    pub email: bool,
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    pub timezone: String,
}

pub async fn upsert_org_default<'e, E>(executor: E, d: &OrgDefaultRow) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO notification_org_defaults
             (org_id, kind, in_app, email, quiet_from, quiet_until, timezone)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (org_id, kind) DO UPDATE SET
             in_app = $3, email = $4, quiet_from = $5, quiet_until = $6,
             timezone = $7"#,
        d.org_id,
        d.kind,
        d.in_app,
        d.email,
        d.quiet_from,
        d.quiet_until,
        d.timezone,
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_org_defaults<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<OrgDefaultRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT org_id, kind, in_app, email, quiet_from, quiet_until, timezone
           FROM notification_org_defaults WHERE org_id = $1 ORDER BY kind"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| OrgDefaultRow {
            org_id: row.org_id,
            kind: row.kind,
            in_app: row.in_app,
            email: row.email,
            quiet_from: row.quiet_from,
            quiet_until: row.quiet_until,
            timezone: row.timezone,
        })
        .collect())
}

/// The fully resolved preference for one (org, user, kind): the user's row
/// wins whole-row; otherwise the org default; otherwise built-in values
/// (`in_app=true`, `email=false`, no quiet hours, UTC).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectivePreference {
    /// Which chain link supplied the values: `user` | `org_default` |
    /// `builtin`.
    pub source: &'static str,
    pub in_app: bool,
    pub email: bool,
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
    pub timezone: String,
}

pub async fn effective_preference<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
    kind: &str,
) -> DbResult<EffectivePreference>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT
             CASE WHEN up.user_id IS NOT NULL THEN 'user'
                  WHEN od.org_id  IS NOT NULL THEN 'org_default'
                  ELSE 'builtin' END AS source,
             CASE WHEN up.user_id IS NOT NULL THEN up.in_app
                  WHEN od.org_id  IS NOT NULL THEN od.in_app
                  ELSE TRUE END AS in_app,
             CASE WHEN up.user_id IS NOT NULL THEN up.email
                  WHEN od.org_id  IS NOT NULL THEN od.email
                  ELSE FALSE END AS email,
             CASE WHEN up.user_id IS NOT NULL THEN up.quiet_from
                  WHEN od.org_id  IS NOT NULL THEN od.quiet_from
                  ELSE NULL END AS quiet_from,
             CASE WHEN up.user_id IS NOT NULL THEN up.quiet_until
                  WHEN od.org_id  IS NOT NULL THEN od.quiet_until
                  ELSE NULL END AS quiet_until,
             CASE WHEN up.user_id IS NOT NULL THEN up.timezone
                  WHEN od.org_id  IS NOT NULL THEN od.timezone
                  ELSE 'UTC' END AS timezone
           FROM (SELECT 1) AS one
           LEFT JOIN notification_preferences up
                  ON up.user_id = $1 AND up.kind = $2
           LEFT JOIN notification_org_defaults od
                  ON od.org_id = $3 AND od.kind = $2"#,
        user_id,
        kind,
        org_id
    )
    .fetch_one(executor)
    .await?;
    // The CASE chain always yields a value (ELSE covers every branch), so
    // the inferred nullables are unreachable; fallbacks restate built-ins.
    Ok(EffectivePreference {
        source: match row.source.as_deref() {
            Some("user") => "user",
            Some("org_default") => "org_default",
            _ => "builtin",
        },
        in_app: row.in_app.unwrap_or(true),
        email: row.email.unwrap_or(false),
        quiet_from: row.quiet_from,
        quiet_until: row.quiet_until,
        timezone: row.timezone.unwrap_or_else(|| "UTC".to_string()),
    })
}
