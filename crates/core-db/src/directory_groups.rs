//! Directory groups: the groups a directory synchronizes, their members, and
//! the roles each group carries (Wave 52 lane R10, migration 0148).
//!
//! The tables are plain storage on purpose. What makes them authorization is
//! [`crate::users::effective_roles_for_user`]: it unions the roles granted here
//! with `user_roles`, and every principal the API builds comes from that read.
//!
//! Every write pins the org explicitly instead of trusting the transaction's
//! `app.org_id`. RLS is the enforcement; the pin is what makes a foreign id a
//! clean `NotFound` rather than a silent zero-row write on a misconfigured
//! scope.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

/// One directory group, with the counts a listing shows.
#[derive(Debug, Clone)]
pub struct DirectoryGroup {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// The identity provider's identifier for this group; `None` for a group
    /// created in Marula that no directory knows about.
    pub external_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub member_count: i64,
    pub role_count: i64,
}

/// One member of a group, with the user facts a listing needs.
#[derive(Debug, Clone)]
pub struct DirectoryGroupMember {
    pub user_id: Uuid,
    pub display_name: String,
    pub email: String,
    pub is_active: bool,
    pub added_at: DateTime<Utc>,
    pub added_by: Option<Uuid>,
}

/// One role a group carries.
#[derive(Debug, Clone)]
pub struct DirectoryGroupRole {
    pub role_id: Uuid,
    pub name: String,
    pub builtin: Option<String>,
}

/// Insert payload for [`insert_group`].
pub struct NewDirectoryGroup<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub external_id: Option<&'a str>,
}

const GROUP_COLS: &str = "g.id, g.org_id, g.name, g.description, g.external_id, \
     g.created_at, g.updated_at, \
     (SELECT count(*) FROM directory_group_members m WHERE m.group_id = g.id) AS member_count, \
     (SELECT count(*) FROM directory_group_roles r WHERE r.group_id = g.id) AS role_count";

fn group_from_row(row: sqlx::postgres::PgRow) -> DbResult<DirectoryGroup> {
    Ok(DirectoryGroup {
        id: row.get("id"),
        org_id: row.get("org_id"),
        name: row.get("name"),
        description: row.get("description"),
        external_id: row.get("external_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        member_count: row.get("member_count"),
        role_count: row.get("role_count"),
    })
}

/// One page of groups, keyset-ordered `(name, id)`.
///
/// The read used to return every group in the tenant, which is right for a
/// small directory and wrong for the one this surface exists for: a customer
/// with thousands of groups paid for all of them on every sync pass. The
/// cursor carries the previous page's last `(name, id)`, so a page boundary
/// cannot skip or repeat a row when a sync renames a group between two pages —
/// the position is a value in the ordering, not a row number.
///
/// `after = None` is the first page. The predicate is the same one in both
/// cases (name/"" and id/nil sort before every real row), which is the shape
/// `crate::report_schedules::list_page` already uses; one expression means the
/// resume path and the first page cannot drift.
///
/// The `UNIQUE (org_id, name)` index from migration 0148 serves both the scan
/// and the ordering, so pagination added no index.
pub async fn list_groups_page<'e, E>(
    executor: E,
    org_id: Uuid,
    after: Option<(&str, Uuid)>,
    limit: i64,
) -> DbResult<Vec<DirectoryGroup>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {GROUP_COLS} FROM directory_groups g \
          WHERE g.org_id = $1 AND (g.name, g.id) > ($2, $3) \
          ORDER BY g.name, g.id \
          LIMIT $4"
    );
    let (after_name, after_id) = after.unwrap_or(("", Uuid::nil()));
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(after_name)
        .bind(after_id)
        .bind(limit)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(group_from_row).collect()
}

/// One group, or `NotFound` when it is not visible in this org.
pub async fn get_group<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<DirectoryGroup>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql =
        format!("SELECT {GROUP_COLS} FROM directory_groups g WHERE g.org_id = $1 AND g.id = $2");
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_optional(executor)
        .await?;
    row.map(group_from_row).unwrap_or(Err(DbError::NotFound))
}

/// One group by the identity provider's identifier for it, or `None` when this
/// org has not adopted that external group yet.
///
/// This is the lookup an identity sync performs on every pass ("do I already
/// have this external group, or do I create it?"), and the reason
/// `idx_directory_groups_external` exists (migration 0148). It returns
/// `Option` rather than `NotFound` because "not yet" is the ordinary first
/// answer for a new group, not an error a caller has to translate.
///
/// The org is pinned in the SQL rather than left to RLS: the same external
/// group id in two orgs resolves to two different local rows, and an executor
/// with no `app.org_id` scope must still answer correctly.
pub async fn get_group_by_external_id<'e, E>(
    executor: E,
    org_id: Uuid,
    external_id: &str,
) -> DbResult<Option<DirectoryGroup>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {GROUP_COLS} FROM directory_groups g \
          WHERE g.org_id = $1 AND g.external_id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(external_id)
        .fetch_optional(executor)
        .await?;
    row.map(group_from_row).transpose()
}

/// The groups one user belongs to, by name.
///
/// The companion read to [`crate::users::effective_roles_for_user`]: that one
/// answers "what does this person hold", this one answers "which groups put it
/// there". An identity bundle expanding a directory user needs both — the
/// roles to write into the principal path it does not own, the groups to show
/// an administrator why the roles are what they are.
///
/// Inactive users are included: a sync pass has to see the membership of a
/// person it may be about to deactivate, and dropping them here would make a
/// revoked member look like a member of nothing.
pub async fn list_groups_for_user<'e, E>(
    executor: E,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<Vec<DirectoryGroup>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {GROUP_COLS} FROM directory_groups g \
           JOIN directory_group_members mem ON mem.group_id = g.id \
          WHERE g.org_id = $1 AND mem.user_id = $2 \
          ORDER BY g.name"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(user_id)
        .fetch_all(executor)
        .await?;
    rows.into_iter().map(group_from_row).collect()
}

/// Creates the group. A duplicate name inside the org surfaces as the SQL
/// unique violation, which the API maps to a typed conflict.
pub async fn insert_group(conn: &mut PgConnection, group: &NewDirectoryGroup<'_>) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO directory_groups (id, org_id, name, description, external_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(group.id)
    .bind(group.org_id)
    .bind(group.name)
    .bind(group.description)
    .bind(group.external_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Replaces the mutable fields of one group in this org.
pub async fn update_group(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    name: &str,
    description: Option<&str>,
    external_id: Option<&str>,
) -> DbResult<()> {
    let done = sqlx::query(
        "UPDATE directory_groups \
            SET name = $3, description = $4, external_id = $5, updated_at = now() \
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(external_id)
    .execute(&mut *conn)
    .await?;
    if done.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Deletes the group; membership and role mappings cascade with it.
pub async fn delete_group(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let done = sqlx::query("DELETE FROM directory_groups WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if done.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Adds a user to a group. Idempotent; a user or group outside the org adds
/// nothing.
pub async fn add_member(
    conn: &mut PgConnection,
    org_id: Uuid,
    group_id: Uuid,
    user_id: Uuid,
    added_by: Option<Uuid>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO directory_group_members (group_id, user_id, added_by) \
         SELECT g.id, u.id, $4 \
           FROM directory_groups g \
           JOIN users u ON u.org_id = g.org_id \
          WHERE g.id = $1 AND g.org_id = $2 AND u.id = $3 \
         ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(org_id)
    .bind(user_id)
    .bind(added_by)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Removes a user from a group.
pub async fn remove_member(
    conn: &mut PgConnection,
    org_id: Uuid,
    group_id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM directory_group_members m \
               USING directory_groups g \
               WHERE m.group_id = g.id AND g.org_id = $2 \
                 AND m.group_id = $1 AND m.user_id = $3",
    )
    .bind(group_id)
    .bind(org_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether the user is already in the group. The API asks before inserting so
/// a repeated `PUT` reports the same membership without emitting a second
/// event for a non-change.
pub async fn is_member<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
    user_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let present: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS ( \
           SELECT 1 FROM directory_group_members m \
             JOIN directory_groups g ON g.id = m.group_id \
            WHERE m.group_id = $1 AND m.user_id = $2 AND g.org_id = $3)",
    )
    .bind(group_id)
    .bind(user_id)
    .bind(org_id)
    .fetch_optional(executor)
    .await?;
    Ok(present.unwrap_or(false))
}

/// Every member of one group in this org.
pub async fn list_members<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
) -> DbResult<Vec<DirectoryGroupMember>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        "SELECT m.user_id, u.display_name, u.email, u.is_active, m.added_at, m.added_by \
           FROM directory_group_members m \
           JOIN users u ON u.id = m.user_id \
           JOIN directory_groups g ON g.id = m.group_id \
          WHERE m.group_id = $1 AND g.org_id = $2 \
          ORDER BY u.display_name, u.email",
    )
    .bind(group_id)
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DirectoryGroupMember {
            user_id: row.get("user_id"),
            display_name: row.get("display_name"),
            email: row.get("email"),
            is_active: row.get("is_active"),
            added_at: row.get("added_at"),
            added_by: row.get("added_by"),
        })
        .collect())
}

/// Maps a role onto a group. Idempotent; a group or role outside the org maps
/// nothing.
pub async fn add_role(
    conn: &mut PgConnection,
    org_id: Uuid,
    group_id: Uuid,
    role_id: Uuid,
    granted_by: Option<Uuid>,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO directory_group_roles (group_id, role_id, granted_by) \
         SELECT g.id, r.id, $4 \
           FROM directory_groups g \
           JOIN roles r ON r.org_id = g.org_id \
          WHERE g.id = $1 AND g.org_id = $2 AND r.id = $3 \
         ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(org_id)
    .bind(role_id)
    .bind(granted_by)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Drops one role from a group. The members lose exactly what this mapping
/// granted.
pub async fn remove_role(
    conn: &mut PgConnection,
    org_id: Uuid,
    group_id: Uuid,
    role_id: Uuid,
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM directory_group_roles dgr \
               USING directory_groups g \
               WHERE dgr.group_id = g.id AND g.org_id = $2 \
                 AND dgr.group_id = $1 AND dgr.role_id = $3",
    )
    .bind(group_id)
    .bind(org_id)
    .bind(role_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether the group already carries this role.
pub async fn has_role<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
    role_id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let present: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS ( \
           SELECT 1 FROM directory_group_roles dgr \
             JOIN directory_groups g ON g.id = dgr.group_id \
            WHERE dgr.group_id = $1 AND dgr.role_id = $2 AND g.org_id = $3)",
    )
    .bind(group_id)
    .bind(role_id)
    .bind(org_id)
    .fetch_optional(executor)
    .await?;
    Ok(present.unwrap_or(false))
}

/// Every role one group carries.
pub async fn list_group_roles<'e, E>(
    executor: E,
    org_id: Uuid,
    group_id: Uuid,
) -> DbResult<Vec<DirectoryGroupRole>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query(
        "SELECT r.id AS role_id, r.name, r.builtin \
           FROM directory_group_roles dgr \
           JOIN roles r ON r.id = dgr.role_id \
           JOIN directory_groups g ON g.id = dgr.group_id \
          WHERE dgr.group_id = $1 AND g.org_id = $2 \
          ORDER BY r.name",
    )
    .bind(group_id)
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DirectoryGroupRole {
            role_id: row.get("role_id"),
            name: row.get("name"),
            builtin: row.get("builtin"),
        })
        .collect())
}
