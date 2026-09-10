//! Ticket type/category/subcategory taxonomy (Wave 11 lane 3).
//!
//! Purpose: the query layer behind org-owned classification trees — types
//! (flat) and categories (self-referencing, exactly two levels: category →
//! subcategory). Full org CRUD: create, rename, reparent, retire.
//! Retirement NEVER deletes: rows stay so existing tickets keep their
//! reference (FKs are ON DELETE RESTRICT at the schema level too).
//!
//! Classification-only doctrine (§5): nothing in behavior binds to these
//! trees — SLA, workflow, automations, billing, and reports key on the
//! SEMANTIC status categories. This module therefore has no coupling to
//! sla_engine/workflow, and renames/reparents/retires are behavior-neutral
//! by construction.
//!
//! Shape guarantees (two-level depth, sibling name uniqueness) are
//! enforced by the schema itself (0048: partial unique indexes + the
//! `trg_ticket_categories_depth` trigger), so a racing writer cannot grow
//! a third level even if a future caller skips pre-validation. Domain
//! pre-validation (does the parent exist, is it a root) lives at the API
//! layer where the typed 4xx mapping is.
//!
//! Non-purpose: no ticket write paths here (the tickets module owns the
//! ticket row; classification columns ride its patch path), no API shapes
//! (core-api `routes/taxonomy.rs` owns those).

use crate::{DbError, DbResult};
use uuid::Uuid;

/// The standard accepted type set seeded for every org (client-agnostic
/// IT/MSP baseline). Mirrored in migration 0048's SQL backfill.
pub const STANDARD_TYPES: &[(&str, i32)] = &[
    ("Incident", 0),
    ("Service Request", 1),
    ("Problem", 2),
    ("Change", 3),
];

/// Standard root categories, client-agnostic IT/MSP taxonomy.
pub const STANDARD_CATEGORY_ROOTS: &[(&str, i32)] = &[
    ("End-User Computing", 0),
    ("Network", 10),
    ("Servers & Infrastructure", 20),
    ("Cloud Services", 30),
    ("Security", 40),
    ("Telephony & Communications", 50),
    ("Line-of-Business Applications", 60),
    ("Facilities & Assets", 70),
];

/// Standard subcategories as `(root category name, name, sort_order)` —
/// two-level tree, exactly. Mirrored in migration 0048's SQL backfill.
pub const STANDARD_SUBCATEGORIES: &[(&str, &str, i32)] = &[
    ("End-User Computing", "Hardware", 0),
    ("End-User Computing", "Software & Applications", 1),
    ("End-User Computing", "Email & Collaboration", 2),
    ("End-User Computing", "Printing & Scanning", 3),
    ("Network", "Connectivity & Outage", 0),
    ("Network", "VPN & Remote Access", 1),
    ("Network", "DNS & DHCP", 2),
    ("Network", "Firewall & Perimeter", 3),
    ("Servers & Infrastructure", "Windows Server", 0),
    ("Servers & Infrastructure", "Linux & Unix", 1),
    ("Servers & Infrastructure", "Virtualization", 2),
    ("Servers & Infrastructure", "Storage & Backup", 3),
    ("Cloud Services", "Microsoft 365", 0),
    ("Cloud Services", "Identity & Access", 1),
    ("Cloud Services", "Azure & AWS", 2),
    ("Cloud Services", "Backup & Recovery", 3),
    ("Security", "Malware & Phishing", 0),
    ("Security", "Vulnerability & Patching", 1),
    ("Security", "Data Loss & Compliance", 2),
    ("Telephony & Communications", "VoIP & PBX", 0),
    ("Telephony & Communications", "Mobile Devices", 1),
    ("Telephony & Communications", "Video Conferencing", 2),
    ("Line-of-Business Applications", "Application Error", 0),
    ("Line-of-Business Applications", "Access Request", 1),
    ("Line-of-Business Applications", "Data & Reporting", 2),
    ("Line-of-Business Applications", "Integration & API", 3),
    ("Facilities & Assets", "Physical Access", 0),
    ("Facilities & Assets", "Peripherals", 1),
    ("Facilities & Assets", "New Equipment", 2),
];

/// One org taxonomy node, common to types and categories. `parent_id` is
/// `None` for types (flat) and for root categories; subcategories carry
/// their parent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaxonomyNode {
    pub id: Uuid,
    pub org_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub is_retired: bool,
    pub sort_order: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Which taxonomy tree an operation targets. Exhaustive so new trees must
/// thread through every match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxonomyTable {
    Type,
    Category,
}

impl TaxonomyTable {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Type => "ticket_types",
            Self::Category => "ticket_categories",
        }
    }
}

impl std::fmt::Display for TaxonomyTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `parent_id` exists only on ticket_categories; the shared node shape
/// reads NULL for types (flat list) via a constant.
const NODE_COLUMNS_TYPE: &str =
    "id, org_id, NULL::uuid AS parent_id, name, description, is_retired, sort_order, created_at, updated_at";
const NODE_COLUMNS_CATEGORY: &str =
    "id, org_id, parent_id, name, description, is_retired, sort_order, created_at, updated_at";

/// One taxonomy row as `query_as` sees it (the shared node shape).
type NodeRow = (
    Uuid,
    Uuid,
    Option<Uuid>,
    String,
    Option<String>,
    bool,
    i32,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
);

fn node_columns(table: TaxonomyTable) -> &'static str {
    match table {
        TaxonomyTable::Type => NODE_COLUMNS_TYPE,
        TaxonomyTable::Category => NODE_COLUMNS_CATEGORY,
    }
}

fn node(row: NodeRow) -> TaxonomyNode {
    let (id, org_id, parent_id, name, description, is_retired, sort_order, created_at, updated_at) =
        row;
    TaxonomyNode {
        id,
        org_id,
        parent_id,
        name,
        description,
        is_retired,
        sort_order,
        created_at,
        updated_at,
    }
}

// --- reads ---------------------------------------------------------------------

pub async fn get_node(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    id: Uuid,
) -> DbResult<TaxonomyNode> {
    let row: Option<NodeRow> = sqlx::query_as(&format!(
        "SELECT {} FROM {table} WHERE org_id = $1 AND id = $2",
        node_columns(table)
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(node).ok_or(DbError::NotFound)
}

/// All nodes of one tree, active-only unless the caller asks for the full
/// surface (admin editors + the U5 manifest want retired rows too).
/// Ordered by (sort_order, name, id) — the deterministic manifest order.
pub async fn list_nodes(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    include_retired: bool,
) -> DbResult<Vec<TaxonomyNode>> {
    let filter = if include_retired {
        ""
    } else {
        "AND is_retired = false"
    };
    let rows: Vec<NodeRow> = sqlx::query_as(&format!(
        "SELECT {} FROM {table} WHERE org_id = $1 {filter} ORDER BY sort_order, name, id",
        node_columns(table)
    ))
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(node).collect())
}

/// How many tickets still reference the node — purely informational for
/// the picker/UX (retirement does not require zero references).
pub async fn count_ticket_references(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    id: Uuid,
) -> DbResult<i64> {
    let column = match table {
        TaxonomyTable::Type => "type_id",
        TaxonomyTable::Category => "category_id",
    };
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM tickets WHERE org_id = $1 AND {column} = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(count)
}

// --- creates ---------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn insert_node(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    table: TaxonomyTable,
    parent_id: Option<Uuid>,
    name: &str,
    description: Option<&str>,
    sort_order: i32,
) -> DbResult<TaxonomyNode> {
    match table {
        TaxonomyTable::Type => {
            if parent_id.is_some() {
                return Err(DbError::NotFound);
            }
            sqlx::query(
                "INSERT INTO ticket_types (id, org_id, name, description, sort_order) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(id)
            .bind(org_id)
            .bind(name)
            .bind(description)
            .bind(sort_order)
            .execute(&mut *conn)
            .await?;
        }
        TaxonomyTable::Category => {
            sqlx::query(
                "INSERT INTO ticket_categories (id, org_id, parent_id, name, description, sort_order) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(id)
            .bind(org_id)
            .bind(parent_id)
            .bind(name)
            .bind(description)
            .bind(sort_order)
            .execute(&mut *conn)
            .await?;
        }
    }
    get_node(conn, org_id, table, id).await
}

// --- mutations ---------------------------------------------------------------------

/// Rename / edit description / re-sort. Sibling name uniqueness is the
/// DB's partial unique indexes (a duplicate surfaces as a conflict).
pub async fn update_node(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    id: Uuid,
    name: &str,
    description: Option<&str>,
    sort_order: i32,
) -> DbResult<()> {
    let updated = sqlx::query(&format!(
        "UPDATE {table} SET name = $3, description = $4, sort_order = $5, updated_at = now() \
         WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(sort_order)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Reparent a SUBCATEGORY under a different root category. The caller
/// (API) pre-validates that `id` is a subcategory and `new_parent_id` is a
/// root of the same org; the depth trigger is the concurrency guarantor.
/// The row is locked so two concurrent reparents serialize cleanly.
pub async fn reparent_category(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    new_parent_id: Uuid,
) -> DbResult<TaxonomyNode> {
    let locked: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT parent_id FROM ticket_categories WHERE org_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    match locked {
        None => Err(DbError::NotFound),
        Some(None) => Err(DbError::NotFound),
        Some(Some(_)) => {
            sqlx::query(
                "UPDATE ticket_categories SET parent_id = $3, updated_at = now() \
                 WHERE org_id = $1 AND id = $2",
            )
            .bind(org_id)
            .bind(id)
            .bind(new_parent_id)
            .execute(&mut *conn)
            .await?;
            get_node(conn, org_id, TaxonomyTable::Category, id).await
        }
    }
}

/// Soft-retire: pickers hide the node, existing tickets keep their
/// reference, the manifest still exports it. Never deletes. Idempotent
/// double-retire reports NotFound on the second call (the row is already
/// retired — the caller sees the current state via read).
pub async fn retire_node(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    table: TaxonomyTable,
    id: Uuid,
) -> DbResult<()> {
    let updated = sqlx::query(&format!(
        "UPDATE {table} SET is_retired = true, updated_at = now() \
         WHERE org_id = $1 AND id = $2 AND is_retired = false"
    ))
    .bind(org_id)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}
