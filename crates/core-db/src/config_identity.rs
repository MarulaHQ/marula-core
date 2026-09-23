//! Configuration identity (ADR 0003, CI1–CI6, migration 0061): source
//! links — the only feed write path — CI interfaces, field tiers, field
//! precedence, and the merge queue. The canonical config item IS a
//! `device_assets` row; this module persists the CI facets around it.
//!
//! Non-purpose: no vendor transports (lane 8) and no business rules beyond
//! persistence-ordered feed upsert; curated stickiness, precedence
//! defaults, and deterministic matching resolve through
//! `core_domain::entities::config_identity`.

use crate::assets::{get_asset, ASSET_COLUMNS};
use crate::{DbError, DbResult};
use chrono::Utc;
use core_domain::entities::config_identity::{
    contradicts_curated, effective_fields, match_identity, shipped_priority, shipped_source_kind,
    CiFieldPrecedence, CiInterface, CiMerge, CiMergeCandidate, CiMergeStatus, ConfigSourceLink,
    IdentityKeys,
};
use core_domain::entities::DeviceAsset;
use serde_json::{json, Map, Value};
use sqlx::Row;
use uuid::Uuid;

const LINK_COLUMNS: &str = "id, org_id, config_item_id, source_system, external_id, \
     source_guid, payload_ref, last_seen_at, created_at, updated_at";

fn row_to_link(r: sqlx::postgres::PgRow) -> ConfigSourceLink {
    ConfigSourceLink {
        id: r.get("id"),
        org_id: r.get("org_id"),
        config_item_id: r.get("config_item_id"),
        source_system: r.get("source_system"),
        external_id: r.get("external_id"),
        source_guid: r.get("source_guid"),
        payload_ref: r.get("payload_ref"),
        last_seen_at: r.get("last_seen_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const INTERFACE_COLUMNS: &str = "id, org_id, config_item_id, name, mac, ips, vlans, \
     is_primary, source_link_id, created_at, updated_at";

fn row_to_interface(r: sqlx::postgres::PgRow) -> CiInterface {
    CiInterface {
        id: r.get("id"),
        org_id: r.get("org_id"),
        config_item_id: r.get("config_item_id"),
        name: r.get("name"),
        mac: r.get("mac"),
        ips: serde_json::from_value(r.get("ips")).unwrap_or_default(),
        vlans: serde_json::from_value(r.get("vlans")).unwrap_or_default(),
        is_primary: r.get("is_primary"),
        source_link_id: r.get("source_link_id"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

const CANDIDATE_COLUMNS: &str = "id, org_id, primary_config_item_id, candidate_config_item_id, \
     confidence::float8 AS confidence, reason, evidence, status, decided_by, decided_at, created_at, updated_at";

fn row_to_candidate(r: sqlx::postgres::PgRow) -> CiMergeCandidate {
    CiMergeCandidate {
        id: r.get("id"),
        org_id: r.get("org_id"),
        primary_config_item_id: r.get("primary_config_item_id"),
        candidate_config_item_id: r.get("candidate_config_item_id"),
        confidence: r.get::<Option<f64>, _>("confidence").unwrap_or(0.0),
        reason: r.get("reason"),
        evidence: r.get("evidence"),
        status: r
            .get::<String, _>("status")
            .parse::<CiMergeStatusMarker>()
            .map(|m| m.0)
            .unwrap_or(CiMergeStatus::Pending),
        decided_by: r.get("decided_by"),
        decided_at: r.get("decided_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

/// `FromStr` bridge so the row mapper can reuse the domain enum's parser.
struct CiMergeStatusMarker(CiMergeStatus);

impl std::str::FromStr for CiMergeStatusMarker {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        CiMergeStatus::parse(s).map(CiMergeStatusMarker).ok_or(())
    }
}

const MERGE_COLUMNS: &str = "id, org_id, primary_config_item_id, merged_config_item_id, \
     candidate_id, confidence::float8 AS confidence, reason, evidence, moved, status, merged_by, merged_at, \
     undone_by, undone_at";

fn row_to_merge(r: sqlx::postgres::PgRow) -> CiMerge {
    CiMerge {
        id: r.get("id"),
        org_id: r.get("org_id"),
        primary_config_item_id: r.get("primary_config_item_id"),
        merged_config_item_id: r.get("merged_config_item_id"),
        candidate_id: r.get("candidate_id"),
        confidence: r.get::<Option<f64>, _>("confidence"),
        reason: r.get("reason"),
        evidence: r.get("evidence"),
        moved: r.get("moved"),
        status: r.get("status"),
        merged_by: r.get("merged_by"),
        merged_at: r.get("merged_at"),
        undone_by: r.get("undone_by"),
        undone_at: r.get("undone_at"),
    }
}

const PRECEDENCE_COLUMNS: &str = "id, org_id, layout_id, semantic_class, field_key, \
     source_system, priority, created_at, updated_at";

fn row_to_precedence(r: sqlx::postgres::PgRow) -> CiFieldPrecedence {
    CiFieldPrecedence {
        id: r.get("id"),
        org_id: r.get("org_id"),
        layout_id: r.get("layout_id"),
        semantic_class: r
            .get::<Option<String>, _>("semantic_class")
            .and_then(|s| core_domain::entities::CiSemanticClass::parse(&s)),
        field_key: r.get("field_key"),
        source_system: r.get("source_system"),
        priority: r.get("priority"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

// ---- CI6: the item's semantic class (via its layout type) ------------------

/// The CI6 semantic class an item's type binds to (None = untyped or a
/// type without a class). Renaming a layout never changes this — behavior
/// reads the class.
pub async fn semantic_class_for_item(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item: &DeviceAsset,
) -> DbResult<Option<String>> {
    match item.layout_id {
        None => Ok(None),
        Some(layout_id) => Ok(sqlx::query_scalar(
            "SELECT semantic_class FROM document_layouts
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(layout_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten()),
    }
}

// ---- CI2: source links -----------------------------------------------------

pub async fn get_link_by_external(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    source_system: &str,
    external_id: &str,
) -> DbResult<Option<ConfigSourceLink>> {
    sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM config_source_links
         WHERE org_id = $1 AND source_system = $2 AND external_id = $3"
    ))
    .bind(org_id)
    .bind(source_system)
    .bind(external_id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_link)
    .map(Some)
    .map(Ok)
    .unwrap_or(Ok(None))
}

pub async fn list_links_for_item(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    config_item_id: Uuid,
) -> DbResult<Vec<ConfigSourceLink>> {
    let rows = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM config_source_links
         WHERE org_id = $1 AND config_item_id = $2 ORDER BY created_at"
    ))
    .bind(org_id)
    .bind(config_item_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_link).collect())
}

/// Creates a source link bound to one canonical item. The table's UNIQUE
/// (org, source_system, external_id) makes a second sync update its own
/// link instead of creating a twin (CI2).
#[allow(clippy::too_many_arguments)]
pub async fn create_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    config_item_id: Uuid,
    source_system: &str,
    external_id: &str,
    source_guid: Option<&str>,
    payload_ref: Option<&str>,
) -> DbResult<ConfigSourceLink> {
    let r = sqlx::query(&format!(
        "INSERT INTO config_source_links
           (id, org_id, config_item_id, source_system, external_id, source_guid, payload_ref)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {LINK_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(config_item_id)
    .bind(source_system)
    .bind(external_id)
    .bind(source_guid)
    .bind(payload_ref)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_link(r))
}

/// Second sync: updates THIS link's last-seen + payload reference.
pub async fn touch_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    source_guid: Option<&str>,
    payload_ref: Option<&str>,
) -> DbResult<ConfigSourceLink> {
    let r = sqlx::query(&format!(
        "UPDATE config_source_links SET
           last_seen_at = now(),
           source_guid = COALESCE($3, source_guid),
           payload_ref = COALESCE($4, payload_ref),
           updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {LINK_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(source_guid)
    .bind(payload_ref)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_link)
    .ok_or(DbError::NotFound)?;
    Ok(r)
}

/// Repoints a link at another config item (merge) or back (split).
pub async fn move_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    config_item_id: Uuid,
) -> DbResult<()> {
    let r = sqlx::query(
        "UPDATE config_source_links SET config_item_id = $3, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .bind(config_item_id)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// ---- CI interfaces ---------------------------------------------------------

pub async fn list_interfaces(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    config_item_id: Uuid,
) -> DbResult<Vec<CiInterface>> {
    let rows = sqlx::query(&format!(
        "SELECT {INTERFACE_COLUMNS} FROM ci_interfaces
         WHERE org_id = $1 AND config_item_id = $2 ORDER BY is_primary DESC, name, created_at"
    ))
    .bind(org_id)
    .bind(config_item_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_interface).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_interface(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    config_item_id: Uuid,
    name: Option<&str>,
    mac: Option<&str>,
    ips: &Value,
    vlans: &Value,
    is_primary: bool,
    source_link_id: Option<Uuid>,
) -> DbResult<CiInterface> {
    let r = sqlx::query(&format!(
        "INSERT INTO ci_interfaces
           (id, org_id, config_item_id, name, mac, ips, vlans, is_primary, source_link_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING {INTERFACE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(config_item_id)
    .bind(name)
    .bind(mac)
    .bind(ips)
    .bind(vlans)
    .bind(is_primary)
    .bind(source_link_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_interface(r))
}

pub async fn delete_interface(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<()> {
    let r = sqlx::query("DELETE FROM ci_interfaces WHERE org_id = $1 AND id = $2")
        .bind(org_id)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if r.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

pub async fn move_interfaces(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    from_item: Uuid,
    to_item: Uuid,
) -> DbResult<Vec<Uuid>> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE ci_interfaces SET config_item_id = $3, updated_at = now()
         WHERE org_id = $1 AND config_item_id = $2 RETURNING id",
    )
    .bind(org_id)
    .bind(from_item)
    .bind(to_item)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids)
}

// ---- CI5: field precedence -------------------------------------------------

pub async fn list_precedence(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<CiFieldPrecedence>> {
    let rows = sqlx::query(&format!(
        "SELECT {PRECEDENCE_COLUMNS} FROM ci_field_precedence
         WHERE org_id = $1 ORDER BY layout_id NULLS FIRST, field_key, priority DESC"
    ))
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_precedence).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_precedence(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    layout_id: Option<Uuid>,
    semantic_class: Option<&str>,
    field_key: &str,
    source_system: &str,
    priority: i16,
) -> DbResult<CiFieldPrecedence> {
    let r = sqlx::query(&format!(
        "INSERT INTO ci_field_precedence
           (id, org_id, layout_id, semantic_class, field_key, source_system, priority)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (org_id, COALESCE(layout_id, '00000000-0000-0000-0000-000000000000'::uuid),
                      field_key, source_system)
         DO UPDATE SET semantic_class = EXCLUDED.semantic_class,
                       priority = EXCLUDED.priority,
                       updated_at = now()
         RETURNING {PRECEDENCE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(layout_id)
    .bind(semantic_class)
    .bind(field_key)
    .bind(source_system)
    .bind(priority)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_precedence(r))
}

/// Resolved precedence for one (type, field, source): the org override row
/// when present, else the ADR-shipped default.
pub async fn resolve_field_priority(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    layout_id: Option<Uuid>,
    field_key: &str,
    source_system: &str,
) -> DbResult<i16> {
    let override_row: Option<i16> = sqlx::query_scalar(
        "SELECT priority FROM ci_field_precedence
         WHERE org_id = $1 AND field_key = $2 AND source_system = $3
           AND (layout_id = $4 OR layout_id IS NULL)
         ORDER BY layout_id NULLS LAST
         LIMIT 1",
    )
    .bind(org_id)
    .bind(field_key)
    .bind(source_system)
    .bind(layout_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(override_row
        .unwrap_or_else(|| shipped_priority(field_key, shipped_source_kind(source_system))))
}

// ---- CI4: merge queue + org threshold --------------------------------------

pub async fn list_candidates(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    status: Option<&str>,
) -> DbResult<Vec<CiMergeCandidate>> {
    let rows = sqlx::query(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM ci_merge_candidates
         WHERE org_id = $1 AND ($2::text IS NULL OR status = $2)
         ORDER BY confidence DESC, created_at DESC"
    ))
    .bind(org_id)
    .bind(status)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_candidate).collect())
}

pub async fn get_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<CiMergeCandidate> {
    sqlx::query(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM ci_merge_candidates WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    primary_config_item_id: Uuid,
    candidate_config_item_id: Uuid,
    confidence: f64,
    reason: &str,
    evidence: &Value,
    status: &str,
) -> DbResult<CiMergeCandidate> {
    let r = sqlx::query(&format!(
        "INSERT INTO ci_merge_candidates
           (id, org_id, primary_config_item_id, candidate_config_item_id,
            confidence, reason, evidence, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (org_id, primary_config_item_id, candidate_config_item_id)
         DO UPDATE SET confidence = EXCLUDED.confidence,
                       reason = EXCLUDED.reason,
                       evidence = EXCLUDED.evidence,
                       status = CASE WHEN ci_merge_candidates.status = 'pending'
                                     THEN EXCLUDED.status
                                     ELSE ci_merge_candidates.status END,
                       updated_at = now()
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(primary_config_item_id)
    .bind(candidate_config_item_id)
    .bind(confidence)
    .bind(reason)
    .bind(evidence)
    .bind(status)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_candidate(r))
}

/// Marks a candidate decided; only pending/auto_merged rows can flip.
pub async fn decide_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    status: &str,
    decided_by: Uuid,
) -> DbResult<CiMergeCandidate> {
    let r = sqlx::query(&format!(
        "UPDATE ci_merge_candidates
         SET status = $3, decided_by = $4, decided_at = now(), updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status IN ('pending', 'auto_merged')
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(status)
    .bind(decided_by)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::Conflict("merge candidate already decided".into()))?;
    Ok(r)
}

/// Re-opens a DEFERRED candidate (the workbench un-defer affordance).
/// Decided rows stay decided — undo for accepts rides the merge's split.
pub async fn restore_candidate(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<CiMergeCandidate> {
    let r = sqlx::query(&format!(
        "UPDATE ci_merge_candidates
         SET status = 'pending', decided_by = NULL, decided_at = NULL, updated_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'deferred'
         RETURNING {CANDIDATE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_candidate)
    .ok_or(DbError::Conflict("candidate is not deferred".into()))?;
    Ok(r)
}

// ---- CI4: merges (audit + undo substrate) ----------------------------------

#[allow(clippy::too_many_arguments)]
pub async fn record_merge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    primary_config_item_id: Uuid,
    merged_config_item_id: Uuid,
    candidate_id: Option<Uuid>,
    confidence: Option<f64>,
    reason: &str,
    evidence: &Value,
    moved: &Value,
    merged_by: Option<Uuid>,
) -> DbResult<CiMerge> {
    let r = sqlx::query(&format!(
        "INSERT INTO ci_merges
           (id, org_id, primary_config_item_id, merged_config_item_id, candidate_id,
            confidence, reason, evidence, moved, status, merged_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'applied', $10)
         RETURNING {MERGE_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(primary_config_item_id)
    .bind(merged_config_item_id)
    .bind(candidate_id)
    .bind(confidence)
    .bind(reason)
    .bind(evidence)
    .bind(moved)
    .bind(merged_by)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_merge(r))
}

pub async fn get_merge(conn: &mut sqlx::PgConnection, org_id: Uuid, id: Uuid) -> DbResult<CiMerge> {
    sqlx::query(&format!(
        "SELECT {MERGE_COLUMNS} FROM ci_merges WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_merge)
    .ok_or(DbError::NotFound)
}

pub async fn list_merges_for_item(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    config_item_id: Uuid,
) -> DbResult<Vec<CiMerge>> {
    let rows = sqlx::query(&format!(
        "SELECT {MERGE_COLUMNS} FROM ci_merges
         WHERE org_id = $1 AND (primary_config_item_id = $2 OR merged_config_item_id = $2)
         ORDER BY merged_at DESC"
    ))
    .bind(org_id)
    .bind(config_item_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(row_to_merge).collect())
}

// ---- CI4: org-configurable auto-merge threshold ----------------------------

pub async fn auto_merge_threshold(conn: &mut sqlx::PgConnection, org_id: Uuid) -> DbResult<f64> {
    let v: Option<f64> = sqlx::query_scalar(
        "SELECT auto_merge_threshold::float8 FROM ci_settings WHERE org_id = $1",
    )
    .bind(org_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(v.unwrap_or(0.85))
}

pub async fn upsert_settings(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    auto_merge_threshold: f64,
) -> DbResult<f64> {
    let v: f64 = sqlx::query_scalar(
        "INSERT INTO ci_settings (org_id, auto_merge_threshold)
         VALUES ($1, $2)
         ON CONFLICT (org_id) DO UPDATE
           SET auto_merge_threshold = EXCLUDED.auto_merge_threshold, updated_at = now()
         RETURNING auto_merge_threshold::float8",
    )
    .bind(org_id)
    .bind(auto_merge_threshold)
    .fetch_one(&mut *conn)
    .await?;
    Ok(v)
}

// ---- CI1: config-item reads / creates on device_assets ---------------------

pub async fn get_config_item(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<DeviceAsset> {
    get_asset(&mut *conn, org_id, id).await
}

/// Active config items: merged tombstones are excluded.
pub async fn list_config_items(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Option<Uuid>,
    layout_id: Option<Uuid>,
    semantic_class: Option<&str>,
    q: Option<&str>,
) -> DbResult<Vec<DeviceAsset>> {
    let rows = sqlx::query(&format!(
        r#"SELECT {ASSET_COLUMNS}
           FROM device_assets a
           WHERE a.org_id = $1
             AND a.merged_into_id IS NULL
             AND ($2::uuid IS NULL OR a.client_id = $2)
             AND ($3::uuid IS NULL OR a.layout_id = $3)
             AND ($4::text IS NULL OR EXISTS (
                   SELECT 1 FROM document_layouts dl
                   WHERE dl.id = a.layout_id AND dl.semantic_class = $4))
             AND ($5::text IS NULL OR
                  a.tag ILIKE '%' || $5 || '%' OR a.name ILIKE '%' || $5 || '%' OR
                  a.fields::text ILIKE '%' || $5 || '%' OR a.identity_keys::text ILIKE '%' || $5 || '%')
           ORDER BY a.tag, a.name"#
    ))
    .bind(org_id)
    .bind(client_id)
    .bind(layout_id)
    .bind(semantic_class)
    .bind(q.map(|s| s.trim()).filter(|s| !s.is_empty()))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(crate::assets::row_to_asset).collect())
}

#[allow(clippy::too_many_arguments)]
pub async fn create_config_item(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    client_id: Uuid,
    site_id: Option<Uuid>,
    tag: &str,
    name: &str,
    layout_id: Option<Uuid>,
    identity_keys: &Value,
) -> DbResult<DeviceAsset> {
    let row = sqlx::query(&format!(
        "INSERT INTO device_assets
           (id, org_id, client_id, site_id, tag, name, kind, status, layout_id, fields, identity_keys)
         VALUES ($1, $2, $3, $4, $5, $6, 'device', 'active', $7, '{{}}'::jsonb, $8)
         RETURNING {ASSET_COLUMNS}"
    ))
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(site_id)
    .bind(tag)
    .bind(name)
    .bind(layout_id)
    .bind(identity_keys)
    .fetch_one(&mut *conn)
    .await?;
    Ok(crate::assets::row_to_asset(row))
}

// ---- CI3: identity matching ------------------------------------------------

/// Org-scoped deterministic identity search (CI3). Tombstoned items never
/// match; signals below 0.6 are noise and are dropped.
pub async fn find_identity_matches(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    client_id: Uuid,
    keys: &IdentityKeys,
) -> DbResult<
    Vec<(
        DeviceAsset,
        core_domain::entities::config_identity::IdentityMatch,
    )>,
> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(&format!(
        r#"SELECT {ASSET_COLUMNS} FROM device_assets
           WHERE org_id = $1 AND client_id = $2 AND merged_into_id IS NULL
             AND identity_keys::text <> '{{}}'"#
    ))
    .bind(org_id)
    .bind(client_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let asset = crate::assets::row_to_asset(row);
        let stored = IdentityKeys::from_value(&asset.identity_keys);
        if let Some(m) = match_identity(keys, &stored) {
            if m.confidence >= 0.6 {
                out.push((asset, m));
            }
        }
    }
    Ok(out)
}

/// Refreshes an item's identity keys: supplied non-empty signals merge over
/// the stored ones (stored values the feed no longer sends are kept).
pub async fn refresh_identity_keys(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    incoming: &Value,
) -> DbResult<()> {
    let item = get_asset(&mut *conn, org_id, item_id).await?;
    let mut merged = item.identity_keys.as_object().cloned().unwrap_or_default();
    if let Some(incoming_obj) = incoming.as_object() {
        for (k, v) in incoming_obj {
            let empty = v.is_null()
                || v.as_str().map(str::is_empty).unwrap_or(false)
                || v.as_array().map(|a| a.is_empty()).unwrap_or(false);
            if !empty {
                merged.insert(k.clone(), v.clone());
            }
        }
    }
    sqlx::query(
        "UPDATE device_assets SET identity_keys = $3, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(item_id)
    .bind(Value::Object(merged))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---- CI5: curated vs observed ----------------------------------------------

/// One observed field arriving from a feeder.
#[derive(Debug, Clone)]
pub struct FeedField {
    pub key: String,
    pub value: Value,
}

/// Applies one observed field under CI5 rules: a curated value is sticky
/// (the observed value is still recorded + flagged as a contradiction);
/// among feeds, the higher-priority source wins. Returns the field keys it
/// actually wrote.
async fn apply_observed_field(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    layout_id: Option<Uuid>,
    link_id: Uuid,
    source_system: &str,
    field: &FeedField,
) -> DbResult<Option<String>> {
    let item = get_asset(&mut *conn, org_id, item_id).await?;
    let curated = item.curated_fields.as_object().cloned().unwrap_or_default();
    let contradiction =
        contradicts_curated(&Value::Object(curated.clone()), &field.key, &field.value);

    let existing_source = item
        .field_sources
        .get(&field.key)
        .and_then(|s| s.get("source_system"))
        .and_then(|s| s.as_str())
        .map(str::to_string);
    let existing_priority = match existing_source.as_deref() {
        Some(s) => resolve_field_priority(&mut *conn, org_id, layout_id, &field.key, s).await?,
        None => -1,
    };
    let incoming_priority =
        resolve_field_priority(&mut *conn, org_id, layout_id, &field.key, source_system).await?;

    if !contradiction && incoming_priority < existing_priority {
        return Ok(None); // a stronger source already owns this observed field
    }

    let mut observed = item
        .observed_fields
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    observed.insert(field.key.clone(), field.value.clone());

    let mut sources = item
        .field_sources
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    let mut entry = sources
        .get(&field.key)
        .cloned()
        .unwrap_or_else(|| json!({}))
        .as_object()
        .cloned()
        .unwrap_or_default();
    entry.insert("source_system".into(), json!(source_system));
    entry.insert("source_link_id".into(), json!(link_id));
    entry.insert("observed_at".into(), json!(Utc::now().to_rfc3339()));
    if contradiction {
        entry.insert("contradicts_curated".into(), json!(true));
    } else {
        entry.remove("contradicts_curated");
    }
    sources.insert(field.key.clone(), Value::Object(entry));

    if contradiction {
        // Never touch `fields`: the curated value stays effective.
        sqlx::query(
            "UPDATE device_assets SET observed_fields = $3, field_sources = $4, updated_at = now()
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(item_id)
        .bind(Value::Object(observed))
        .bind(Value::Object(sources))
        .execute(&mut *conn)
        .await?;
    } else {
        let effective = effective_fields(&Value::Object(observed.clone()), &item.curated_fields);
        sqlx::query(
            "UPDATE device_assets SET observed_fields = $3, field_sources = $4, fields = $5, updated_at = now()
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(item_id)
        .bind(Value::Object(observed))
        .bind(Value::Object(sources))
        .bind(effective)
        .execute(&mut *conn)
        .await?;
    }
    Ok(Some(field.key.clone()))
}

/// CI5: human curation — merges curated values in (sticky forever; a null
/// value un-curates the field) and projects the effective view.
pub async fn curate_fields(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    curated: &Value,
) -> DbResult<DeviceAsset> {
    let item = get_asset(&mut *conn, org_id, item_id).await?;
    let mut merged_curated = item.curated_fields.as_object().cloned().unwrap_or_default();
    if let Some(obj) = curated.as_object() {
        for (k, v) in obj {
            if v.is_null() {
                merged_curated.remove(k);
            } else {
                merged_curated.insert(k.clone(), v.clone());
            }
        }
    }
    let observed = item.observed_fields.clone();
    let effective = effective_fields(&observed, &Value::Object(merged_curated.clone()));
    let row = sqlx::query(&format!(
        "UPDATE device_assets SET curated_fields = $3, fields = $4, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {ASSET_COLUMNS}"
    ))
    .bind(org_id)
    .bind(item_id)
    .bind(Value::Object(merged_curated))
    .bind(effective)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(crate::assets::row_to_asset(row))
}

/// CI5 one-click "accept the new value": promotes the observed value into
/// the sticky curated tier and clears the contradiction flag.
pub async fn accept_observed_value(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    field_key: &str,
) -> DbResult<DeviceAsset> {
    let item = get_asset(&mut *conn, org_id, item_id).await?;
    let observed = item
        .observed_fields
        .as_object()
        .cloned()
        .unwrap_or_default();
    let incoming = observed
        .get(field_key)
        .cloned()
        .ok_or_else(|| DbError::Conflict(format!("no observed value for '{field_key}'")))?;
    let mut curated = item.curated_fields.as_object().cloned().unwrap_or_default();
    curated.insert(field_key.to_string(), incoming);
    let mut sources = item
        .field_sources
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    if let Some(entry) = sources.get_mut(field_key).and_then(|e| e.as_object_mut()) {
        entry.remove("contradicts_curated");
        entry.insert("accepted_at".into(), json!(Utc::now().to_rfc3339()));
    }
    let observed_value = item.observed_fields.clone();
    let effective = effective_fields(&observed_value, &Value::Object(curated.clone()));
    let row = sqlx::query(&format!(
        "UPDATE device_assets SET curated_fields = $3, field_sources = $4, fields = $5, updated_at = now()
         WHERE org_id = $1 AND id = $2
         RETURNING {ASSET_COLUMNS}"
    ))
    .bind(org_id)
    .bind(item_id)
    .bind(Value::Object(curated))
    .bind(Value::Object(sources))
    .bind(effective)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(crate::assets::row_to_asset(row))
}

// ---- CI1/CI2: the feed upsert (the lane-8 seam) ----------------------------

/// What one feed sync did — the visible-evidence packet for the response.
#[derive(Debug, Clone)]
pub struct FeedUpsertResult {
    pub item: DeviceAsset,
    pub link: ConfigSourceLink,
    /// The canonical item row is new (created by THIS sync, no twin risk).
    pub created_item: bool,
    /// An existing item was deterministically resolved (no new row).
    pub resolved_existing: bool,
    /// Auto-merged at/above the org threshold.
    pub auto_merged: bool,
    /// Reviewable candidates queued for a human.
    pub queued_candidates: Vec<CiMergeCandidate>,
    /// Observed values that contradict a curated value (never applied).
    pub contradictions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FeedUpsert {
    pub source_system: String,
    pub external_id: String,
    pub source_guid: Option<String>,
    pub payload_ref: Option<String>,
    pub client_id: Uuid,
    /// Display defaults for a freshly created item.
    pub tag: String,
    pub name: String,
    pub layout_id: Option<Uuid>,
    pub identity_keys: Value,
    pub observed_fields: Vec<FeedField>,
}

/// Follows merge tombstones to the canonical item.
async fn canonical(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    mut item: DeviceAsset,
) -> DbResult<DeviceAsset> {
    for _ in 0..4 {
        match item.merged_into_id {
            Some(target) => item = get_asset(&mut *conn, org_id, target).await?,
            None => return Ok(item),
        }
    }
    Err(DbError::Conflict("merge tombstone chain too deep".into()))
}

/// CI2 + CI3 + CI4: the single feed write path.
///
/// 1. An existing (org, source_system, external_id) link resolves to its
///    own canonical item — a second sync updates its link, never a twin.
/// 2. Otherwise deterministic identity keys resolve the common case; an
///    exact-enough match ADOPTS the existing item (no new row).
/// 3. Ambiguous matches: at/above the org threshold the best candidate is
///    adopted automatically (candidate row recorded as `auto_merged`);
///    below it the sync creates a fresh item and queues reviewable
///    candidates for a human.
/// 4. No signals: a fresh canonical item with its link (never blind — the
///    link is always written in the same transaction).
pub async fn feed_upsert(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    input: &FeedUpsert,
) -> DbResult<FeedUpsertResult> {
    // 1. own link first: upsert, never blind-insert.
    if let Some(link) =
        get_link_by_external(&mut *conn, org_id, &input.source_system, &input.external_id).await?
    {
        let bound_item = get_asset(&mut *conn, org_id, link.config_item_id).await?;
        let item = canonical(&mut *conn, org_id, bound_item).await?;
        let link = if item.id == link.config_item_id {
            touch_link(
                &mut *conn,
                org_id,
                link.id,
                input.source_guid.as_deref(),
                input.payload_ref.as_deref(),
            )
            .await?
        } else {
            // The link still points at a tombstone (pre-0061 merge): heal it.
            move_link(&mut *conn, org_id, link.id, item.id).await?;
            touch_link(
                &mut *conn,
                org_id,
                link.id,
                input.source_guid.as_deref(),
                input.payload_ref.as_deref(),
            )
            .await?
        };
        let contradictions = write_observed(
            &mut *conn,
            org_id,
            item.id,
            input.layout_id,
            link.id,
            &input.source_system,
            &input.observed_fields,
        )
        .await?;
        refresh_identity_keys(&mut *conn, org_id, item.id, &input.identity_keys).await?;
        let item = get_asset(&mut *conn, org_id, item.id).await?;
        return Ok(FeedUpsertResult {
            item,
            link,
            created_item: false,
            resolved_existing: false,
            auto_merged: false,
            queued_candidates: Vec::new(),
            contradictions,
        });
    }

    // 2. deterministic matching.
    let keys = IdentityKeys::from_fields(&Value::Null, Some(&input.identity_keys));
    let mut candidates = find_identity_matches(&mut *conn, org_id, input.client_id, &keys).await?;
    candidates.sort_by(|a, b| {
        b.1.confidence
            .partial_cmp(&a.1.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let threshold = auto_merge_threshold(&mut *conn, org_id).await?;
    let mut queued_candidates: Vec<CiMergeCandidate> = Vec::new();

    let (target, created_item, resolved_existing) = match candidates.first() {
        Some((asset, m)) if m.confidence >= 0.95 => (asset.id, false, true),
        Some((asset, m)) if m.confidence >= threshold => (asset.id, false, true),
        _ => {
            // Below threshold (ambiguous) or no signal: fresh canonical row.
            let id = Uuid::now_v7();
            let _item = create_config_item(
                &mut *conn,
                org_id,
                id,
                input.client_id,
                None,
                &input.tag,
                &input.name,
                input.layout_id,
                &input.identity_keys,
            )
            .await?;
            // Queue the reviewable candidates against the new row.
            for (asset, m) in &candidates {
                let candidate = create_candidate(
                    &mut *conn,
                    org_id,
                    Uuid::now_v7(),
                    asset.id,
                    id,
                    m.confidence,
                    &m.reason,
                    &m.evidence,
                    "pending",
                )
                .await?;
                queued_candidates.push(candidate);
            }
            (id, true, false)
        }
    };

    let link = create_link(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        target,
        &input.source_system,
        &input.external_id,
        input.source_guid.as_deref(),
        input.payload_ref.as_deref(),
    )
    .await?;
    let contradictions = write_observed(
        &mut *conn,
        org_id,
        target,
        input.layout_id,
        link.id,
        &input.source_system,
        &input.observed_fields,
    )
    .await?;
    refresh_identity_keys(&mut *conn, org_id, target, &input.identity_keys).await?;
    let item = get_asset(&mut *conn, org_id, target).await?;
    Ok(FeedUpsertResult {
        item,
        link,
        created_item,
        resolved_existing,
        auto_merged: resolved_existing && !created_item,
        queued_candidates,
        contradictions,
    })
}

/// Writes the observed batch (curated stickiness + feed precedence).
async fn write_observed(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    item_id: Uuid,
    layout_id: Option<Uuid>,
    link_id: Uuid,
    source_system: &str,
    fields: &[FeedField],
) -> DbResult<Vec<String>> {
    let mut contradictions = Vec::new();
    for field in fields {
        let item = get_asset(&mut *conn, org_id, item_id).await?;
        if contradicts_curated(&item.curated_fields, &field.key, &field.value) {
            contradictions.push(field.key.clone());
        }
        apply_observed_field(
            &mut *conn,
            org_id,
            item_id,
            layout_id,
            link_id,
            source_system,
            field,
        )
        .await?;
    }
    Ok(contradictions)
}

// ---- CI4: merge / split ----------------------------------------------------

/// Folds `merged_id` into `primary_id`: source links + interfaces move,
/// observed/curated fields fill gaps only (curated stays sticky), and the
/// absorbed row becomes a tombstone. Returns the audit row.
#[allow(clippy::too_many_arguments)]
pub async fn merge_items(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    primary_id: Uuid,
    merged_id: Uuid,
    candidate_id: Option<Uuid>,
    confidence: Option<f64>,
    reason: &str,
    evidence: &Value,
    merged_by: Option<Uuid>,
) -> DbResult<CiMerge> {
    if primary_id == merged_id {
        return Err(DbError::Conflict("cannot merge an item into itself".into()));
    }
    let primary = get_asset(&mut *conn, org_id, primary_id).await?;
    let absorbed = get_asset(&mut *conn, org_id, merged_id).await?;
    if absorbed.merged_into_id.is_some() {
        return Err(DbError::Conflict("item already merged".into()));
    }

    let moved_links: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE config_source_links SET config_item_id = $3, updated_at = now()
         WHERE org_id = $1 AND config_item_id = $2 RETURNING id",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(primary_id)
    .fetch_all(&mut *conn)
    .await?;
    let moved_interfaces = move_interfaces(&mut *conn, org_id, merged_id, primary_id).await?;

    let mut observed = primary
        .observed_fields
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    if let Some(incoming) = absorbed.observed_fields.as_object() {
        for (k, v) in incoming {
            observed.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let mut curated = primary
        .curated_fields
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    if let Some(incoming) = absorbed.curated_fields.as_object() {
        for (k, v) in incoming {
            curated.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let effective = effective_fields(
        &Value::Object(observed.clone()),
        &Value::Object(curated.clone()),
    );

    sqlx::query(
        "UPDATE device_assets SET observed_fields = $3, curated_fields = $4, fields = $5, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(primary_id)
    .bind(Value::Object(observed))
    .bind(Value::Object(curated))
    .bind(effective)
    .execute(&mut *conn)
    .await?;

    let moved = json!({
        "source_link_ids": moved_links,
        "interface_ids": moved_interfaces,
        "observed_before": absorbed.observed_fields,
        "curated_before": absorbed.curated_fields,
    });

    // Tombstone the absorbed row LAST (it is the split substrate).
    sqlx::query(
        "UPDATE device_assets SET merged_into_id = $3, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(primary_id)
    .execute(&mut *conn)
    .await?;

    record_merge(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        primary_id,
        merged_id,
        candidate_id,
        confidence,
        reason,
        evidence,
        &moved,
        merged_by,
    )
    .await
}

/// Split (undo): restores the absorbed item from the audit row's `moved`
/// packet — links and interfaces ride back, tiers are restored from the
/// snapshot, and the tombstone clears.
pub async fn split_merge(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    merge_id: Uuid,
    undone_by: Uuid,
) -> DbResult<CiMerge> {
    let merge = get_merge(&mut *conn, org_id, merge_id).await?;
    if merge.status != "applied" {
        return Err(DbError::Conflict("merge already undone".into()));
    }
    let merged_id = merge.merged_config_item_id;

    let parse_ids = |v: Option<&Value>| -> Vec<Uuid> {
        v.and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                    .collect()
            })
            .unwrap_or_default()
    };
    for link_id in parse_ids(merge.moved.get("source_link_ids")) {
        move_link(&mut *conn, org_id, link_id, merged_id).await?;
    }
    for iface_id in parse_ids(merge.moved.get("interface_ids")) {
        sqlx::query(
            "UPDATE ci_interfaces SET config_item_id = $3, updated_at = now()
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(iface_id)
        .bind(merged_id)
        .execute(&mut *conn)
        .await?;
    }

    let observed_before = merge
        .moved
        .get("observed_before")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let curated_before = merge
        .moved
        .get("curated_before")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let effective = effective_fields(&observed_before, &curated_before);
    sqlx::query(
        "UPDATE device_assets SET
           observed_fields = $3, curated_fields = $4, fields = $5,
           merged_into_id = NULL, updated_at = now()
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(merged_id)
    .bind(observed_before)
    .bind(curated_before)
    .bind(effective)
    .execute(&mut *conn)
    .await?;

    let r = sqlx::query(&format!(
        "UPDATE ci_merges
         SET status = 'undone', undone_by = $3, undone_at = now()
         WHERE org_id = $1 AND id = $2 AND status = 'applied'
         RETURNING {MERGE_COLUMNS}"
    ))
    .bind(org_id)
    .bind(merge_id)
    .bind(undone_by)
    .fetch_optional(&mut *conn)
    .await?
    .map(row_to_merge)
    .ok_or(DbError::Conflict("merge already undone".into()))?;
    Ok(r)
}
