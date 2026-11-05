//! Report-definition persistence (the private build tracker R3; ADR 0001).
//!
//! Purpose: CRUD for stored report specs — the `report_definitions` rows
//! (migration 0034). Rows carry the registry-backed `kind`/`viz`/`scope`
//! columns plus the spec BODY jsonb and, since migration 0180, the card
//! document; core-reports re-validates the merged spec and the card on every
//! write and load, so this module stays deliberately thin: no spec knowledge,
//! no SQL compilation, no authorization (core-api owns RBAC; RLS keeps
//! tenancy absolute).
//!
//! Visibility is a QUERY concern here because it is a WHERE clause: admins
//! see every row of the org, everyone else sees org-published rows plus
//! their own private ones (`scope = 'org' OR created_by = $viewer`).
//!
//! Non-purpose: no execution (compiled on demand from the spec), no
//! manifest assembly (core-db `manifest` reads these rows for U5), no
//! scheduled delivery (R4).

use crate::{DbError, DbResult};
use sqlx::Postgres;
use uuid::Uuid;

/// One stored report definition as persisted. `spec` is the raw body
/// document (`{filters, group_by, measures}`); reassembly into a typed
/// [`core_reports::ReportSpec`] happens one layer up via
/// `ReportSpec::from_parts(kind, &spec, viz)`. Field names mirror the
/// columns exactly (the offline-checked `query_as!` record).
#[derive(Debug, Clone, PartialEq)]
pub struct ReportDefinitionRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// Registry-backed view slug (core-reports `ViewId`).
    pub kind: String,
    /// Spec body document.
    pub spec: serde_json::Value,
    /// Card document (`{size, window_days, refresh}`), or NULL when the
    /// definition carries no card. Reassembly into a typed
    /// [`core_reports::CardSpec`] happens one layer up, like `spec`.
    pub card: Option<serde_json::Value>,
    /// Registry-backed visualization hint (core-reports `VizHint`).
    pub viz: String,
    /// `org` (published org-wide) or `self` (private to the creator).
    pub scope: String,
    pub created_by: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// All fields of an insert or a full-row update; updates overwrite every
/// member (the caller merges over the current row and re-validates first).
#[derive(Debug, Clone)]
pub struct DefinitionFields {
    pub name: String,
    pub description: Option<String>,
    pub kind: String,
    pub spec: serde_json::Value,
    pub card: Option<serde_json::Value>,
    pub viz: String,
    pub scope: String,
}

/// Inserts one definition. The `(org_id, name)` UNIQUE constraint backs
/// per-org name uniqueness; the mapped conflict surfaces to clients as a
/// typed 409 through the central error layer.
pub async fn insert(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Uuid,
    created_by: Uuid,
    fields: &DefinitionFields,
) -> DbResult<ReportDefinitionRow> {
    let row = sqlx::query_as!(
        ReportDefinitionRow,
        r#"INSERT INTO report_definitions (id, org_id, name, description, kind, spec, card, viz, scope, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, org_id, name, description, kind, spec, card, viz, scope, created_by, created_at, updated_at"#,
        id,
        org_id,
        fields.name,
        fields.description,
        fields.kind,
        fields.spec,
        fields.card,
        fields.viz,
        fields.scope,
        created_by,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Fetches one definition by id within the org. A miss under RLS (another
/// org's row, deleted, or never existed) is deliberately indistinguishable.
/// Read-only, so any executor works (pool or org-bound transaction).
pub async fn get(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<ReportDefinitionRow> {
    sqlx::query_as!(
        ReportDefinitionRow,
        r#"SELECT id, org_id, name, description, kind, spec, card, viz, scope, created_by, created_at, updated_at
           FROM report_definitions WHERE org_id = $1 AND id = $2"#,
        org_id,
        id,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)
}

/// Every org definition — the ADMIN viewer's listing arm.
pub async fn list_all(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
) -> DbResult<Vec<ReportDefinitionRow>> {
    let rows = sqlx::query_as!(
        ReportDefinitionRow,
        r#"SELECT id, org_id, name, description, kind, spec, card, viz, scope, created_by, created_at, updated_at
           FROM report_definitions WHERE org_id = $1
           ORDER BY name, id"#,
        org_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// The NON-admin listing arm: org-published definitions plus the viewer's
/// own private ones — "manager sees org, user sees own" applied to stored
/// definitions. Ordered deterministically `(name, id)`.
pub async fn list_visible(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    viewer_id: Uuid,
) -> DbResult<Vec<ReportDefinitionRow>> {
    let rows = sqlx::query_as!(
        ReportDefinitionRow,
        r#"SELECT id, org_id, name, description, kind, spec, card, viz, scope, created_by, created_at, updated_at
           FROM report_definitions
           WHERE org_id = $1 AND (scope = 'org' OR created_by = $2)
           ORDER BY name, id"#,
        org_id,
        viewer_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Full-row update; `updated_at` advances server-side. Returns NotFound
/// when the row left the org scope (mutation callers map that to the
/// typed `rls_sparse` envelope code).
pub async fn update(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    fields: &DefinitionFields,
) -> DbResult<ReportDefinitionRow> {
    sqlx::query_as!(
        ReportDefinitionRow,
        r#"UPDATE report_definitions
           SET name = $3, description = $4, kind = $5, spec = $6, card = $7, viz = $8,
               scope = $9, updated_at = now()
           WHERE org_id = $1 AND id = $2
           RETURNING id, org_id, name, description, kind, spec, card, viz, scope, created_by, created_at, updated_at"#,
        org_id,
        id,
        fields.name,
        fields.description,
        fields.kind,
        fields.spec,
        fields.card,
        fields.viz,
        fields.scope,
    )
    .fetch_optional(conn)
    .await?
    .ok_or(DbError::NotFound)
}

/// Deletes one definition. Returns NotFound when nothing in scope matched.
pub async fn delete(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM report_definitions WHERE org_id = $1 AND id = $2",
        org_id,
        id
    )
    .execute(conn)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
