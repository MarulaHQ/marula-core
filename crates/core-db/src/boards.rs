//! Boards, board statuses (semantic-category-bound), and favorites.
//! Workflow customization v1 lives here too: the explicit per-board
//! transition adjacency read/written through core-api only, plus board
//! groups/favorites annotations and per-board membership roles
//! (TASKLIST 2.11).

use crate::{DbError, DbResult};
use core_domain::entities::{Board, BoardFavorite, BoardGroup, BoardMember, BoardStatus};
use core_domain::policy::BoardRole;
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
           RETURNING id, org_id, name, description, is_default, require_checklist_close, created_at"#,
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
        require_checklist_close: row.require_checklist_close,
        created_at: row.created_at,
    })
}

/// Content-only board update (import framework, TASKLIST D2 remainder).
/// `is_default` is deliberately NOT writable here: switching the org's
/// default intake board is an explicit operator action on the boards
/// surface, never bulk content drift.
pub async fn update_board<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: Option<&str>,
) -> DbResult<Board>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE boards SET name = $3, description = $4
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, name, description, is_default, require_checklist_close, created_at"#,
        org_id,
        id,
        name,
        description,
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
        require_checklist_close: row.require_checklist_close,
        created_at: row.created_at,
    })
}

pub async fn list_boards<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<Board>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT id, org_id, name, description, is_default, require_checklist_close, created_at
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
            require_checklist_close: row.require_checklist_close,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn get_board<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Board>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, org_id, name, description, is_default, require_checklist_close, created_at
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
        require_checklist_close: row.require_checklist_close,
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

// --- board groups (TASKLIST 2.11) ----------------------------------------

/// Creates an org-scoped nav group; names are unique per org.
pub async fn create_group(
    executor: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    owner_id: Uuid,
    name: &str,
    position: i32,
) -> DbResult<BoardGroup> {
    let row = sqlx::query!(
        r#"INSERT INTO board_groups (id, org_id, owner_id, name, position)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, org_id, owner_id, name, position, created_at"#,
        id,
        org_id,
        owner_id,
        name,
        position
    )
    .fetch_one(&mut *executor)
    .await?;
    Ok(BoardGroup {
        id: row.id,
        org_id: row.org_id,
        owner_id: row.owner_id,
        name: row.name,
        position: row.position,
        created_at: row.created_at,
    })
}

fn group_from_row(row: sqlx::postgres::PgRow) -> DbResult<BoardGroup> {
    use sqlx::Row;
    Ok(BoardGroup {
        id: row.get("id"),
        org_id: row.get("org_id"),
        owner_id: row.get("owner_id"),
        name: row.get("name"),
        position: row.get("position"),
        created_at: row.get("created_at"),
    })
}

const GROUP_COLS: &str = "id, org_id, owner_id, name, position, created_at";

/// Fetches one group by id within the org.
pub async fn get_group<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<BoardGroup>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {GROUP_COLS} FROM board_groups WHERE org_id = $1 AND id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or(DbError::NotFound)?;
    group_from_row(row)
}

/// Lists a org's groups in nav order.
pub async fn list_groups<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<BoardGroup>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql =
        format!("SELECT {GROUP_COLS} FROM board_groups WHERE org_id = $1 ORDER BY position, name");
    let rows = sqlx::query(&sql).bind(org_id).fetch_all(executor).await?;
    rows.into_iter().map(group_from_row).collect()
}

/// Patchable group metadata; `None` fields are left unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct GroupPatch<'a> {
    pub name: Option<&'a str>,
    pub position: Option<i32>,
}

/// Updates one group's presentation metadata in place.
pub async fn update_group(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    patch: GroupPatch<'_>,
) -> DbResult<BoardGroup> {
    let affected = sqlx::query!(
        r#"UPDATE board_groups SET
             name = COALESCE($3, name),
             position = COALESCE($4, position)
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
        patch.name,
        patch.position,
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    get_group(&mut *conn, org_id, id).await
}

/// Deletes a group; member boards become ungrouped (`ON DELETE SET NULL`),
/// never deleted.
pub async fn delete_group(executor: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let affected = sqlx::query!(
        "DELETE FROM board_groups WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Assigns a board to a group (or ungroups it with `None`). Caller has
/// already validated the group belongs to the same org.
pub async fn set_board_group(
    executor: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    group_id: Option<Uuid>,
) -> DbResult<()> {
    if let Some(gid) = group_id {
        // Guard the cross-org assignment the FK cannot see (group lookup is
        // org-scoped; a mismatched id must fail here, not silently bind).
        let belongs = sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM board_groups WHERE org_id = $1 AND id = $2)"#,
            org_id,
            gid
        )
        .fetch_one(&mut *executor)
        .await?;
        if belongs != Some(true) {
            return Err(DbError::NotFound);
        }
    }
    let affected = sqlx::query!(
        "UPDATE boards SET board_group_id = $3 WHERE org_id = $1 AND id = $2",
        org_id,
        board_id,
        group_id
    )
    .execute(&mut *executor)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- per-board membership roles (TASKLIST 2.11) ---------------------------

fn member_from_row(row: sqlx::postgres::PgRow) -> DbResult<BoardMember> {
    use sqlx::Row;
    let role_text: String = row.get("role");
    Ok(BoardMember {
        org_id: row.get("org_id"),
        board_id: row.get("board_id"),
        user_id: row.get("user_id"),
        role: BoardRole::parse(&role_text).ok_or_else(|| {
            DbError::Sql(sqlx::Error::Configuration(
                format!("unknown board_role '{role_text}'").into(),
            ))
        })?,
        created_at: row.get("created_at"),
    })
}

const MEMBER_COLS: &str = "org_id, board_id, user_id, role::text AS role, created_at";

/// Inserts or re-roles one membership row.
pub async fn upsert_member(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    user_id: Uuid,
    role: BoardRole,
) -> DbResult<BoardMember> {
    sqlx::query!(
        r#"INSERT INTO board_members (org_id, board_id, user_id, role)
           VALUES ($1, $2, $3, $4::text::board_role)
           ON CONFLICT (user_id, board_id)
           DO UPDATE SET role = EXCLUDED.role"#,
        org_id,
        board_id,
        user_id,
        role.as_str()
    )
    .execute(&mut *conn)
    .await?;
    let sql = format!(
        "SELECT {MEMBER_COLS} FROM board_members WHERE org_id = $1 AND board_id = $2 AND user_id = $3"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(board_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    member_from_row(row)
}

/// Removes one membership row; the user falls back to org-governed access.
pub async fn remove_member(
    conn: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    let affected = sqlx::query!(
        "DELETE FROM board_members WHERE org_id = $1 AND board_id = $2 AND user_id = $3",
        org_id,
        board_id,
        user_id
    )
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Lists a board's membership roster with display names for admin UIs.
pub async fn list_members(
    executor: &mut PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<Vec<(BoardMember, String)>> {
    let rows = sqlx::query(
        r#"SELECT m.org_id, m.board_id, m.user_id, m.role::text AS role, m.created_at,
                  u.display_name
           FROM board_members m JOIN users u ON u.id = m.user_id
           WHERE m.org_id = $1 AND m.board_id = $2
           ORDER BY u.display_name"#,
    )
    .bind(org_id)
    .bind(board_id)
    .fetch_all(&mut *executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            use sqlx::Row;
            let display_name: String = row.get("display_name");
            Ok((member_from_row(row)?, display_name))
        })
        .collect()
}

/// The caller's board role on this board, if any. Absence means
/// org-governed access (defaults-first passthrough).
pub async fn role_for_user<'e, E>(
    executor: E,
    org_id: Uuid,
    board_id: Uuid,
    user_id: Uuid,
) -> DbResult<Option<BoardRole>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT role::text FROM board_members WHERE org_id = $1 AND board_id = $2 AND user_id = $3",
    )
    .bind(org_id)
    .bind(board_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.and_then(|(r,)| BoardRole::parse(&r)))
}

/// One nav-tree entry (TASKLIST 2.11): the board plus the CALLER's
/// annotations — group assignment, favorite mark, and board role.
#[derive(Debug, Clone)]
pub struct BoardNavEntry {
    pub board: Board,
    pub group_id: Option<Uuid>,
    pub favorite: bool,
    pub my_role: Option<BoardRole>,
}

/// Lists all org boards annotated for the given user's nav rendering.
/// Boards the caller has no membership on carry `my_role = None`
/// (org-governed access).
pub async fn list_boards_annotated<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<BoardNavEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT b.id, b.org_id, b.name, b.description, b.is_default,
                  b.require_checklist_close, b.created_at, b.board_group_id,
                  (f.user_id IS NOT NULL) AS favorite,
                  m.role::text AS my_role
           FROM boards b
           LEFT JOIN board_favorites f ON f.board_id = b.id AND f.user_id = $2
           LEFT JOIN board_members m ON m.board_id = b.id AND m.user_id = $2
           WHERE b.org_id = $1
           ORDER BY b.is_default DESC, b.name"#,
    )
    .bind(org_id)
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    rows.into_iter()
        .map(|row| {
            use sqlx::Row;
            let role_text: Option<String> = row.get("my_role");
            Ok(BoardNavEntry {
                board: Board {
                    id: row.get("id"),
                    org_id: row.get("org_id"),
                    name: row.get("name"),
                    description: row.get("description"),
                    is_default: row.get("is_default"),
                    require_checklist_close: row.get("require_checklist_close"),
                    created_at: row.get("created_at"),
                },
                group_id: row.get("board_group_id"),
                favorite: row.get("favorite"),
                my_role: role_text.as_deref().and_then(BoardRole::parse),
            })
        })
        .collect()
}
