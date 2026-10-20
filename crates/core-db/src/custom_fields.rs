//! Custom-field definitions and JSONB values (the private build tracker 2.17).
//!
//! Purpose: persistence for dynamic forms on the migration-0009 schema —
//! definition CRUD (org- or board-scoped, ticket entity), value upsert/clear
//! keyed by (field_id, entity_id), and batched value fetches for ticket
//! detail/list payloads. Validation of kinds/options/values is core-domain
//! `custom_fields` work; this module only stores what that layer approved.
//!
//! Non-purpose: no business rules (required-field enforcement decisions are
//! composed by core-api from domain fns), no HTTP shapes.

use crate::{DbError, DbResult};
use serde_json::Value as Json;
use sqlx::{Executor, PgConnection};
use uuid::Uuid;

/// One active (non-archived) field definition row.
#[derive(Debug, Clone)]
pub struct FieldDefRow {
    pub id: Uuid,
    pub org_id: Uuid,
    /// `None` = visible on every board in the org.
    pub board_id: Option<Uuid>,
    pub entity: String,
    pub name: String,
    pub kind: String,
    pub config: Json,
    pub required: bool,
    pub sort_order: i32,
}

/// One stored value joined with its definition.
#[derive(Debug, Clone)]
pub struct FieldValueRow {
    pub field_id: Uuid,
    pub entity_id: Uuid,
    pub name: String,
    pub kind: String,
    pub config: Json,
    pub required: bool,
    pub sort_order: i32,
    pub value: Json,
}

pub async fn insert_def(conn: &mut PgConnection, def: &FieldDefRow) -> DbResult<FieldDefRow> {
    let row = sqlx::query_as!(
        FieldDefRow,
        r#"INSERT INTO custom_field_defs
           (id, org_id, board_id, entity, name, kind, config, required, sort_order)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id, org_id, board_id, entity, name, kind, config, required, sort_order"#,
        def.id,
        def.org_id,
        def.board_id,
        def.entity,
        def.name,
        def.kind,
        def.config,
        def.required,
        def.sort_order
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Active ticket definitions governing one board: org-wide defs plus the
/// board's own, sort-order stable (name as tiebreak).
pub async fn list_defs_for_board<'e, E>(
    executor: E,
    org_id: Uuid,
    board_id: Uuid,
) -> DbResult<Vec<FieldDefRow>>
where
    E: Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        FieldDefRow,
        r#"SELECT id, org_id, board_id, entity, name, kind, config, required, sort_order
           FROM custom_field_defs
           WHERE org_id = $1 AND entity = 'ticket' AND archived_at IS NULL
             AND (board_id IS NULL OR board_id = $2)
           ORDER BY sort_order, name"#,
        org_id,
        board_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

pub async fn get_def(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<FieldDefRow> {
    sqlx::query_as!(
        FieldDefRow,
        r#"SELECT id, org_id, board_id, entity, name, kind, config, required, sort_order
           FROM custom_field_defs
           WHERE org_id = $1 AND id = $2 AND archived_at IS NULL"#,
        org_id,
        id
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)
}

/// Partial update of an editable definition. `None` leaves a column alone.
pub struct DefPatch<'a> {
    pub name: Option<&'a str>,
    pub config: Option<&'a Json>,
    pub required: Option<bool>,
    pub sort_order: Option<i32>,
}

pub async fn update_def(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    patch: &DefPatch<'_>,
) -> DbResult<FieldDefRow> {
    sqlx::query_as!(
        FieldDefRow,
        r#"UPDATE custom_field_defs SET
             name = COALESCE($3, name),
             config = COALESCE($4, config),
             required = COALESCE($5, required),
             sort_order = COALESCE($6, sort_order)
           WHERE org_id = $1 AND id = $2 AND archived_at IS NULL
           RETURNING id, org_id, board_id, entity, name, kind, config, required, sort_order"#,
        org_id,
        id,
        patch.name,
        patch.config,
        patch.required,
        patch.sort_order
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)
}

/// Hard delete (values cascade via FK). Archiving keeps history; v1 admins
/// who want soft removal can set `archived_at` through future tooling.
pub async fn delete_def(conn: &mut PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM custom_field_defs WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// All stored values for one entity (ticket), joined with their definitions
/// so consumers get label/kind/required context alongside each raw JSONB.
pub async fn list_values<'e, E>(
    executor: E,
    org_id: Uuid,
    entity_id: Uuid,
) -> DbResult<Vec<FieldValueRow>>
where
    E: Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as!(
        FieldValueRow,
        r#"SELECT v.field_id, v.entity_id, d.name, d.kind, d.config, d.required, d.sort_order, v.value
           FROM custom_field_values v
           JOIN custom_field_defs d ON d.id = v.field_id
           WHERE v.org_id = $1 AND v.entity_id = $2
           ORDER BY d.sort_order, d.name"#,
        org_id,
        entity_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Values for many entities in ONE round trip (ticket list payloads),
/// grouped by entity id.
pub async fn list_values_many<'e, E>(
    executor: E,
    org_id: Uuid,
    entity_ids: &[Uuid],
) -> DbResult<std::collections::HashMap<Uuid, Vec<FieldValueRow>>>
where
    E: Executor<'e, Database = sqlx::Postgres>,
{
    if entity_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let rows = sqlx::query_as!(
        FieldValueRow,
        r#"SELECT v.field_id, v.entity_id, d.name, d.kind, d.config, d.required, d.sort_order, v.value
           FROM custom_field_values v
           JOIN custom_field_defs d ON d.id = v.field_id
           WHERE v.org_id = $1 AND v.entity_id = ANY($2)
           ORDER BY d.sort_order, d.name"#,
        org_id,
        entity_ids
    )
    .fetch_all(executor)
    .await?;
    let mut grouped: std::collections::HashMap<Uuid, Vec<FieldValueRow>> =
        std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        grouped.entry(row.entity_id).or_default().push(row);
    }
    Ok(grouped)
}

/// Idempotent write of one value. The caller has validated `value` against
/// the definition already.
pub async fn upsert_value(
    conn: &mut PgConnection,
    org_id: Uuid,
    field_id: Uuid,
    entity_id: Uuid,
    value: &Json,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO custom_field_values (field_id, entity_id, org_id, value)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (field_id, entity_id) DO UPDATE SET value = EXCLUDED.value"#,
        field_id,
        entity_id,
        org_id,
        value
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn clear_value(
    conn: &mut PgConnection,
    org_id: Uuid,
    field_id: Uuid,
    entity_id: Uuid,
) -> DbResult<bool> {
    let result = sqlx::query!(
        "DELETE FROM custom_field_values \
         WHERE org_id = $1 AND field_id = $2 AND entity_id = $3",
        org_id,
        field_id,
        entity_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(result.rows_affected() > 0)
}
