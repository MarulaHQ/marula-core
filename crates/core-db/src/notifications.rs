//! Notification center persistence.

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
             (user_id, kind, in_app, email, quiet_from, quiet_until)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (user_id, kind) DO UPDATE SET
             in_app = $3, email = $4, quiet_from = $5, quiet_until = $6"#,
        p.user_id,
        kind,
        p.in_app,
        p.email,
        p.quiet_from,
        p.quiet_until,
    )
    .execute(executor)
    .await?;
    Ok(())
}
