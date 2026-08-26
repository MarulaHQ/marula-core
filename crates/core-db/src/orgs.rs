//! Org lifecycle: creation bootstraps default roles, default board with
//! semantic statuses, and the default SLA policy in one transaction
//! (defaults-first doctrine — a new org is immediately operational).

use crate::{DbError, DbResult};
use core_domain::entities::Org;
use core_domain::policy::BuiltinRole;
use core_domain::status::TicketStatusCategory;
use sqlx::{PgConnection, Postgres};
use std::collections::HashMap;
use uuid::Uuid;

pub async fn insert_org<'e, E>(
    executor: E,
    id: Uuid,
    name: &str,
    slug: &str,
    ticket_prefix: &str,
) -> DbResult<Org>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO orgs (id, name, slug, ticket_prefix)
           VALUES ($1, $2, $3, $4)
           RETURNING id, name, slug, ticket_prefix, ticket_sequence, created_at"#,
        id,
        name,
        slug,
        ticket_prefix,
    )
    .fetch_one(executor)
    .await?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        created_at: row.created_at,
    })
}

pub async fn get_org<'e, E>(executor: E, id: Uuid) -> DbResult<Org>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, name, slug, ticket_prefix, ticket_sequence, created_at
           FROM orgs WHERE id = $1"#,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        created_at: row.created_at,
    })
}

/// Atomically allocates the next ticket number for an org (row-locked
/// increment; safe under concurrency).
pub async fn next_ticket_number<'e, E>(executor: E, org_id: Uuid) -> DbResult<(String, i64)>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE orgs SET ticket_sequence = ticket_sequence + 1
           WHERE id = $1
           RETURNING ticket_prefix, ticket_sequence"#,
        org_id
    )
    .fetch_one(executor)
    .await?;
    Ok((row.ticket_prefix, row.ticket_sequence))
}

/// Updates the editable profile fields (General settings / first-run
/// wizard). `None` leaves a field untouched; the slug is immutable.
pub async fn update_profile(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    name: Option<&str>,
    ticket_prefix: Option<&str>,
) -> DbResult<Org> {
    let row = sqlx::query!(
        r#"UPDATE orgs SET
             name = COALESCE($2, name),
             ticket_prefix = COALESCE($3, ticket_prefix)
           WHERE id = $1
           RETURNING id, name, slug, ticket_prefix, ticket_sequence, created_at"#,
        org_id,
        name,
        ticket_prefix,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        created_at: row.created_at,
    })
}

pub fn permissions_for_builtin(role: BuiltinRole) -> serde_json::Value {
    serde_json::to_value(role.permissions()).unwrap_or_else(|_| serde_json::json!([]))
}

/// The default role set shipped with every org: (name, builtin, grants, system).
pub fn default_roles() -> Vec<(String, Option<BuiltinRole>, serde_json::Value, bool)> {
    [
        BuiltinRole::Owner,
        BuiltinRole::Admin,
        BuiltinRole::Dispatcher,
        BuiltinRole::Technician,
        BuiltinRole::ReadOnly,
    ]
    .into_iter()
    .map(|b| {
        (
            match b {
                BuiltinRole::Owner => "Owner",
                BuiltinRole::Admin => "Admin",
                BuiltinRole::Dispatcher => "Dispatcher",
                BuiltinRole::Technician => "Technician",
                BuiltinRole::ReadOnly => "Read Only",
            }
            .to_string(),
            Some(b),
            permissions_for_builtin(b),
            true,
        )
    })
    .collect()
}

/// Seeds default roles for an org; returns role id keyed by builtin name.
/// Multi-statement: takes a connection (callers inside transactions pass
/// `&mut *tx`, which deref-coerces).
pub async fn seed_default_roles(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<HashMap<String, Uuid>> {
    let mut map = HashMap::new();
    for (name, builtin, permissions, is_system) in default_roles() {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
            id,
            org_id,
            name,
            builtin.map(|b| b.as_str().to_string()),
            permissions,
            is_system,
        )
        .execute(&mut *conn)
        .await?;
        if let Some(b) = builtin {
            map.insert(b.as_str().to_string(), id);
        }
    }
    Ok(map)
}

/// Seeds the default board with the four semantic statuses.
pub async fn seed_default_board(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Uuid> {
    let board_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO boards (id, org_id, name, description, is_default)
           VALUES ($1, $2, 'Service Desk', 'Default service board', true)"#,
        board_id,
        org_id
    )
    .execute(&mut *conn)
    .await?;
    let defaults: [(&str, TicketStatusCategory, i32, bool); 4] = [
        ("New", TicketStatusCategory::Open, 0, true),
        ("In Progress", TicketStatusCategory::InProgress, 10, false),
        ("Waiting", TicketStatusCategory::Waiting, 20, false),
        ("Resolved", TicketStatusCategory::Closed, 30, false),
    ];
    for (name, category, sort, is_entry) in defaults {
        sqlx::query!(
            r#"INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            Uuid::now_v7(),
            board_id,
            org_id,
            name,
            category.as_str(),
            sort,
            is_entry,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(board_id)
}

/// Seeds the default SLA policy (industry-standard PSA targets, 9-5 M-F UTC
/// calendar) — an org can run SLAs with zero configuration.
pub async fn seed_default_sla_policy<'e, E>(executor: E, org_id: Uuid) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let id = Uuid::now_v7();
    let targets = serde_json::json!({
        "p1": { "first_response_mins": 15, "resolution_mins": 480 },
        "p2": { "first_response_mins": 60, "resolution_mins": 1440 },
        "p3": { "first_response_mins": 240, "resolution_mins": 4320 },
        "p4": { "first_response_mins": 480, "resolution_mins": 10080 }
    });
    let calendar = serde_json::json!({
        "name": "9to5 M-F",
        "timezone": "UTC",
        "windows": [
            ["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],
            ["09:00","17:00"],["09:00","17:00"], null, null
        ],
        "holidays": []
    });
    sqlx::query!(
        r#"INSERT INTO sla_policies (id, org_id, name, is_default, targets, calendar)
           VALUES ($1, $2, 'Standard', true, $3, $4)"#,
        id,
        org_id,
        targets,
        calendar,
    )
    .execute(executor)
    .await?;
    Ok(id)
}
