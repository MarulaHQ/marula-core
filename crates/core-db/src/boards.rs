//! Boards, board statuses (semantic-category-bound), and favorites.
//! Workflow customization v1 lives here too: the explicit per-board
//! transition adjacency read/written through core-api only.

use crate::{DbError, DbResult};
use core_domain::entities::{Board, BoardFavorite, BoardStatus};
use core_domain::status::TicketStatusCategory;
use core_domain::workflow::TransitionEdge;
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

/// Patchable status metadata. `None` fields are left unchanged; the semantic
/// category may be re-bound (behavior follows it immediately).
#[derive(Debug, Default, Clone, Copy)]
pub struct StatusPatch<'a> {
    pub name: Option<&'a str>,
    pub category: Option<TicketStatusCategory>,
    pub sort_order: Option<i32>,
    pub is_entry: Option<bool>,
}

/// Updates one status's presentation/binding metadata in place.
pub async fn update_status(
    conn: &mut PgConnection,
    status_id: Uuid,
    patch: StatusPatch<'_>,
) -> DbResult<BoardStatus> {
    let affected = sqlx::query!(
        r#"UPDATE board_statuses SET
             name = COALESCE($2, name),
             category = COALESCE($3, category),
             sort_order = COALESCE($4, sort_order),
             is_entry = COALESCE($5, is_entry)
           WHERE id = $1"#,
        status_id,
        patch.name,
        patch.category.map(|c| c.as_str()),
        patch.sort_order,
        patch.is_entry,
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    get_status(&mut *conn, status_id).await
}

/// Removes an unused custom status. Referencing tickets raise a foreign-key
/// violation (`23503`) which callers surface as a conflict — deletion never
/// cascades into ticket history.
pub async fn delete_status(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    status_id: Uuid,
) -> DbResult<()> {
    let affected = sqlx::query!("DELETE FROM board_statuses WHERE id = $1", status_id)
        .execute(executor)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// The board's explicit transition adjacency (possibly empty ⇒ the
/// semantic-category floor governs; see core-domain `workflow`).
pub async fn list_transitions<'e, E>(executor: E, board_id: Uuid) -> DbResult<Vec<TransitionEdge>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT from_status_id, to_status_id
           FROM board_status_transitions WHERE board_id = $1"#,
        board_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| TransitionEdge::new(r.from_status_id, r.to_status_id))
        .collect())
}

/// Atomically replaces the board's whole explicit matrix. Empty `edges`
/// returns the board to floor-mode governance. Callers must have validated
/// edge endpoints against the board's statuses (FKs are the backstop).
pub async fn replace_transitions(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    edges: &[TransitionEdge],
) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM board_status_transitions WHERE board_id = $1",
        board_id
    )
    .execute(&mut *conn)
    .await?;
    for edge in edges {
        sqlx::query!(
            r#"INSERT INTO board_status_transitions
                 (org_id, board_id, from_status_id, to_status_id)
               VALUES ($1, $2, $3, $4)"#,
            org_id,
            board_id,
            edge.from_status_id,
            edge.to_status_id,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
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
