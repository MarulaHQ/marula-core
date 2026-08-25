//! Boards, board statuses (semantic-category-bound), and favorites.

use crate::{DbError, DbResult};
use core_domain::entities::{Board, BoardFavorite, BoardStatus};
use core_domain::status::TicketStatusCategory;
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn add_status(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    board_id: Uuid,
    name: &str,
    category: TicketStatusCategory,
    sort_order: i32,
    is_entry: bool,
) -> DbResult<BoardStatus> {
    sqlx::query!(
        r#"INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        id,
        board_id,
        org_id,
        name,
        category.as_str(),
        sort_order,
        is_entry
    )
    .execute(&mut *conn)
    .await?;
    get_status(&mut *conn, id).await
}

pub async fn create_board<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: Option<&str>,
    is_default: bool,
) -> DbResult<Board>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO boards (id, org_id, name, description, is_default)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, org_id, name, description, is_default, created_at"#,
        id,
        org_id,
        name,
        description,
        is_default
    )
    .fetch_one(executor)
    .await?;
    Ok(Board {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        description: row.description,
        is_default: row.is_default,
        created_at: row.created_at,
    })
}

pub async fn list_boards<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<Board>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, description, is_default, created_at
           FROM boards WHERE org_id = $1 ORDER BY is_default DESC, name"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Board {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            description: row.description,
            is_default: row.is_default,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn get_board<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Board>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, description, is_default, created_at
           FROM boards WHERE org_id = $1 AND id = $2"#,
        org_id,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Board {
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        description: row.description,
        is_default: row.is_default,
        created_at: row.created_at,
    })
}

fn parse_category(s: &str) -> DbResult<TicketStatusCategory> {
    s.parse().map_err(|e: core_domain::DomainError| {
        DbError::Sql(sqlx::Error::Configuration(e.to_string().into()))
    })
}

fn status_from_row(row: sqlx::postgres::PgRow) -> DbResult<BoardStatus> {
    use sqlx::Row;
    Ok(BoardStatus {
        id: row.get("id"),
        board_id: row.get("board_id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        category: parse_category(&row.get::<String, _>("category"))?,
        sort_order: row.get("sort_order"),
        is_entry: row.get("is_entry"),
        color: row.get("color"),
    })
}

const STATUS_COLS: &str = "id, board_id, org_id, name, category, sort_order, is_entry, color";

pub async fn list_statuses<'e, E>(executor: E, board_id: Uuid) -> DbResult<Vec<BoardStatus>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql =
        format!("SELECT {STATUS_COLS} FROM board_statuses WHERE board_id = $1 ORDER BY sort_order");
    let rows = sqlx::query(&sql).bind(board_id).fetch_all(executor).await?;
    rows.into_iter().map(status_from_row).collect()
}

pub async fn get_status<'e, E>(executor: E, status_id: Uuid) -> DbResult<BoardStatus>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {STATUS_COLS} FROM board_statuses WHERE id = $1");
    let row = sqlx::query(&sql)
        .bind(status_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    status_from_row(row)
}

pub async fn default_status_for_board<'e, E>(executor: E, board_id: Uuid) -> DbResult<BoardStatus>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {STATUS_COLS} FROM board_statuses WHERE board_id = $1 AND is_entry ORDER BY sort_order LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(board_id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    status_from_row(row)
}

pub async fn add_favorite<'e, E>(
    executor: E,
    user_id: Uuid,
    board_id: Uuid,
    sort_order: i32,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"INSERT INTO board_favorites (user_id, board_id, sort_order) VALUES ($1, $2, $3)
           ON CONFLICT (user_id, board_id) DO UPDATE SET sort_order = $3"#,
        user_id,
        board_id,
        sort_order
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn remove_favorite<'e, E>(executor: E, user_id: Uuid, board_id: Uuid) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query!(
        r#"DELETE FROM board_favorites WHERE user_id = $1 AND board_id = $2"#,
        user_id,
        board_id
    )
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn favorites_for_user<'e, E>(executor: E, user_id: Uuid) -> DbResult<Vec<BoardFavorite>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT user_id, board_id, sort_order FROM board_favorites
           WHERE user_id = $1 ORDER BY sort_order"#,
        user_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| BoardFavorite {
            user_id: row.user_id,
            board_id: row.board_id,
            sort_order: row.sort_order,
        })
        .collect())
}
