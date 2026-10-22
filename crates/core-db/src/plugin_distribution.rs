//! Plugin & skill DISTRIBUTION persistence (Wave 16 lane 7, ADR 0009):
//! staged member imports, the append-only provenance chain, per-install
//! settings rendered from manifest JSON-Schemas, the update
//! re-consent pending state, and agent skills + version history.
//!
//! Non-purpose: validation (the domain model + the plugin-host source
//! adapters own it), enrollment/consent decisions and event emission
//! (routes), capability meaning (core-domain crate).

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::agent_skills::{AgentSkillVersion, SkillManifest};
use core_domain::entities::plugins::{
    PluginArtifactKind, PluginProvenanceEntry, PluginPublisherVerification, PluginRating,
    PluginSourceKind, PluginStagedItem, PluginStagedStatus,
};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

// --- row mapping helpers -------------------------------------------------------

fn parse_source_kind(value: String) -> DbResult<PluginSourceKind> {
    PluginSourceKind::parse(&value)
        .ok_or_else(|| DbError::Conflict(format!("unknown plugin source kind {value:?}")))
}

fn parse_artifact_kind(value: String) -> DbResult<PluginArtifactKind> {
    PluginArtifactKind::parse(&value)
        .ok_or_else(|| DbError::Conflict(format!("unknown plugin artifact kind {value:?}")))
}

fn parse_staged_status(value: String) -> DbResult<PluginStagedStatus> {
    PluginStagedStatus::parse(&value)
        .ok_or_else(|| DbError::Conflict(format!("unknown plugin staged status {value:?}")))
}

pub(crate) fn caps_from_json(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// --- staged items ---------------------------------------------------------------

const STAGED_COLS: &str =
    "id, org_id, artifact_kind::text AS artifact_kind, source_kind::text AS source_kind, \
     source_url, fetched_ref, artifact_digest, module_digest, manifest, declared_capabilities, \
     status::text AS status, uploaded_by, uploaded_at, review_note, reviewed_by, \
     reviewed_at, adopted_install_id, adopted_skill_id, adopted_theme_pack_id, \
     adopted_identity_bundle_id";

fn row_to_staged(row: &sqlx::postgres::PgRow) -> DbResult<PluginStagedItem> {
    Ok(PluginStagedItem {
        id: row.get("id"),
        org_id: row.get("org_id"),
        artifact_kind: parse_artifact_kind(row.get("artifact_kind"))?,
        source_kind: parse_source_kind(row.get("source_kind"))?,
        source_url: row.get("source_url"),
        fetched_ref: row.get("fetched_ref"),
        artifact_digest: row.get("artifact_digest"),
        module_digest: row.get("module_digest"),
        manifest: row.get("manifest"),
        declared_capabilities: caps_from_json(
            &row.get::<serde_json::Value, _>("declared_capabilities"),
        ),
        status: parse_staged_status(row.get::<String, _>("status"))?,
        uploaded_by: row.get("uploaded_by"),
        uploaded_at: row.get("uploaded_at"),
        review_note: row.get("review_note"),
        reviewed_by: row.get("reviewed_by"),
        reviewed_at: row.get("reviewed_at"),
        adopted_install_id: row.get("adopted_install_id"),
        adopted_skill_id: row.get("adopted_skill_id"),
        adopted_theme_pack_id: row.get("adopted_theme_pack_id"),
        adopted_identity_bundle_id: row.get("adopted_identity_bundle_id"),
    })
}

/// Appends a staged import (PENDING state machine entry). Caller runs
/// sandbox validation through the plugin-host adapters BEFORE this.
///
/// `sqlx::query()` rather than `query!`: the column list gained `module_digest`
/// in migration 0156 and a changed `query!` text needs a refreshed offline
/// cache, which this lane does not run (`cargo sqlx prepare` is a group-gate
/// action).
pub async fn create_staged_item(tx: &mut PgConnection, item: &PluginStagedItem) -> DbResult<()> {
    let manifest = item.manifest.clone();
    let declared = serde_json::to_value(&item.declared_capabilities)
        .map_err(|e| DbError::Conflict(format!("capabilities do not serialize: {e}")))?;
    sqlx::query(
        r#"INSERT INTO plugin_staged_items
             (id, org_id, artifact_kind, source_kind, source_url, fetched_ref,
              artifact_digest, module_digest, manifest, declared_capabilities, status, uploaded_by)
           VALUES ($1, $2, $3::text::plugin_artifact_kind, $4::text::plugin_source_kind,
                   $5, $6, $7, $8, $9::jsonb, $10::jsonb, 'pending', $11)"#,
    )
    .bind(item.id)
    .bind(item.org_id)
    .bind(item.artifact_kind.as_str())
    .bind(item.source_kind.as_str())
    .bind(&item.source_url)
    .bind(&item.fetched_ref)
    .bind(&item.artifact_digest)
    .bind(&item.module_digest)
    .bind(manifest)
    .bind(declared)
    .bind(item.uploaded_by)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Pending staged items for an org. Route layer enforces uploader
/// visibility for non-admins (admins see all).
pub async fn list_staged_items(
    tx: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<PluginStagedItem>> {
    let rows = sqlx::query(&format!(
        "SELECT {STAGED_COLS} FROM plugin_staged_items
         WHERE org_id = $1 ORDER BY uploaded_at DESC"
    ))
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_staged).collect()
}

pub async fn get_staged_item(
    tx: &mut PgConnection,
    org_id: Uuid,
    staged_id: Uuid,
) -> DbResult<PluginStagedItem> {
    let row = sqlx::query(&format!(
        "SELECT {STAGED_COLS} FROM plugin_staged_items WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(staged_id)
    .fetch_one(&mut *tx)
    .await?;
    row_to_staged(&row)
}

/// Transitions the staged state machine exactly once (a pending row
/// only ever moves `approved`/`refused`; terminal rows refuse again).
/// `adopted_*` links are stamped on approval for traceability.
#[allow(clippy::too_many_arguments)]
pub async fn update_staged_status(
    tx: &mut PgConnection,
    org_id: Uuid,
    staged_id: Uuid,
    status: PluginStagedStatus,
    review_note: Option<&str>,
    reviewer: Option<Uuid>,
    now: chrono::DateTime<chrono::Utc>,
    adopted_install_id: Option<Uuid>,
    adopted_skill_id: Option<Uuid>,
    adopted_theme_pack_id: Option<Uuid>,
) -> DbResult<()> {
    let mut current: Option<PluginStagedStatus> = None;
    let row = sqlx::query(&format!(
        "SELECT {STAGED_COLS} FROM plugin_staged_items WHERE org_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(org_id)
    .bind(staged_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = &row {
        current = Some(parse_staged_status(row.get::<String, _>("status"))?);
    }
    let current = current.ok_or(DbError::NotFound)?;
    if current != PluginStagedStatus::Pending {
        return Err(DbError::Conflict(format!(
            "staged import is terminal ({:?}); it cannot restart the review",
            current.as_str()
        )));
    }
    sqlx::query!(
        r#"UPDATE plugin_staged_items SET
             status = $3::text::plugin_staged_status,
             review_note = $4,
             reviewed_by = $5,
             reviewed_at = $6,
             adopted_install_id = COALESCE($7, adopted_install_id),
             adopted_skill_id = COALESCE($8, adopted_skill_id),
             adopted_theme_pack_id = COALESCE($9, adopted_theme_pack_id),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        staged_id,
        status.as_str(),
        review_note,
        reviewer,
        now,
        adopted_install_id,
        adopted_skill_id,
        adopted_theme_pack_id,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

// --- provenance chain (append-only; survives uninstall) ---------------------------

const PROVENANCE_COLS: &str = "id, org_id, install_id, source_kind::text AS source_kind, \
     source_url, fetched_ref, artifact_digest, uploader, action, detail, recorded_at";

fn row_to_provenance(row: &sqlx::postgres::PgRow) -> DbResult<PluginProvenanceEntry> {
    Ok(PluginProvenanceEntry {
        id: row.get("id"),
        org_id: row.get("org_id"),
        install_id: row.get("install_id"),
        source_kind: parse_source_kind(row.get::<String, _>("source_kind"))?,
        source_url: row.get("source_url"),
        fetched_ref: row.get("fetched_ref"),
        artifact_digest: row.get("artifact_digest"),
        uploader: row.get("uploader"),
        action: row.get("action"),
        detail: row.get("detail"),
        recorded_at: row.get("recorded_at"),
    })
}

/// Appends one provenance link to an install's chain. Insert-only —
/// this table has no UPDATE path by design (append-only provenance).
pub async fn insert_provenance(
    tx: &mut PgConnection,
    entry: &PluginProvenanceEntry,
) -> DbResult<()> {
    let detail = match &entry.detail {
        Some(d) => d.clone(),
        None => serde_json::Value::Null,
    };
    entry
        .validate()
        .map_err(|e| DbError::Conflict(format!("invalid provenance entry: {e}")))?;
    sqlx::query!(
        r#"INSERT INTO plugin_install_provenance
             (id, org_id, install_id, source_kind, source_url, fetched_ref,
              artifact_digest, uploader, action, detail, recorded_at)
           VALUES ($1, $2, $3, $4::text::plugin_source_kind, $5, $6, $7, $8,
                   $9, $10::jsonb, $11)"#,
        entry.id,
        entry.org_id,
        entry.install_id,
        entry.source_kind.as_str(),
        entry.source_url,
        entry.fetched_ref,
        entry.artifact_digest,
        entry.uploader,
        entry.action,
        detail,
        entry.recorded_at,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The chain for one install id, oldest first. Survives uninstall (the
/// table carries NO cascading delete to `plugin_installs` BY DESIGN).
pub async fn list_provenance(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<Vec<PluginProvenanceEntry>> {
    let rows = sqlx::query(&format!(
        "SELECT {PROVENANCE_COLS} FROM plugin_install_provenance
         WHERE org_id = $1 AND install_id = $2 ORDER BY recorded_at"
    ))
    .bind(org_id)
    .bind(install_id)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_provenance).collect()
}

// --- update re-consent pending state ----------------------------------------------

/// Pins an update proposal: the OLD version keeps running while the
/// pending columns carry the candidate awaiting the re-consent diff.
#[allow(clippy::too_many_arguments)]
pub async fn set_pending_version(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    pending_version: &str,
    pending_manifest: &serde_json::Value,
    pending_digest: &str,
    pending_capabilities: Option<&[String]>,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<()> {
    let manifest = pending_manifest.clone();
    let caps = pending_capabilities
        .map(|c| {
            serde_json::to_value(c)
                .map_err(|e| DbError::Conflict(format!("caps do not serialize: {e}")))
        })
        .transpose()?;
    sqlx::query!(
        r#"UPDATE plugin_installs SET
             pending_version = $3,
             pending_manifest = $4::jsonb,
             pending_digest = $5,
             pending_capabilities = $6::jsonb,
             pending_consent_at = $7,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        install_id,
        pending_version,
        manifest,
        pending_digest,
        caps,
        now,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Commits the consented swap: version, PIN and grants move to the pending
/// candidate, then the pending columns clear. Caller validates the consent diff
/// happened (routes own audit + events).
///
/// The pin moves with the version, and the outgoing (version, digest, grants)
/// becomes the rollback target, parked in `previous_*` in the same statement —
/// one write, so the pin and its rollback target cannot disagree.
///
/// A version with NO module cannot take over an install that has one: the
/// `WHERE` refuses that row, so the update answers `Conflict` instead of
/// silently clearing a pin. That is the difference between "the update failed"
/// and "the plugin quietly stopped being the thing that was verified".
pub async fn commit_pending_version(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<()> {
    let updated = sqlx::query(
        r#"UPDATE plugin_installs SET
             previous_version = CASE WHEN module_digest IS DISTINCT FROM COALESCE(pending_digest, '')
                                     THEN version ELSE previous_version END,
             previous_module_digest = CASE WHEN module_digest IS DISTINCT FROM COALESCE(pending_digest, '')
                                           THEN module_digest ELSE previous_module_digest END,
             previous_capabilities = CASE WHEN module_digest IS DISTINCT FROM COALESCE(pending_digest, '')
                                          THEN granted_capabilities ELSE previous_capabilities END,
             version = pending_version,
             module_digest = COALESCE(pending_digest, ''),
             granted_capabilities = COALESCE(pending_capabilities, granted_capabilities),
             pending_version = NULL,
             pending_manifest = NULL,
             pending_digest = NULL,
             pending_capabilities = NULL,
             pending_consent_at = NULL,
             updated_at = now()
           WHERE org_id = $1 AND id = $2
             AND pending_version IS NOT NULL
             AND status <> 'revoked'
             AND (COALESCE(pending_digest, '') <> '' OR module_digest = '')"#,
    )
    .bind(org_id)
    .bind(install_id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::Conflict(
            "the update cannot be committed: no pending version, the install is revoked, or the \
             candidate ships no module to pin"
                .into(),
        ));
    }
    Ok(())
}

/// Discards an update proposal (never re-consented).
pub async fn clear_pending_version(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE plugin_installs SET
             pending_version = NULL,
             pending_manifest = NULL,
             pending_digest = NULL,
             pending_capabilities = NULL,
             pending_consent_at = NULL,
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        install_id,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn get_pending_manifest(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<Option<(String, serde_json::Value, String, Vec<String>)>> {
    let row = sqlx::query(
        "SELECT pending_version, pending_manifest, pending_digest, pending_capabilities
         FROM plugin_installs WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Err(DbError::NotFound);
    };
    let Some(version) = row.get::<Option<String>, _>("pending_version") else {
        return Ok(None);
    };
    let manifest = row.get::<serde_json::Value, _>("pending_manifest");
    let digest = row.get::<String, _>("pending_digest");
    let caps = caps_from_json(&row.get::<serde_json::Value, _>("pending_capabilities"));
    Ok(Some((version, manifest, digest, caps)))
}

// --- per-install settings (rendered from manifest JSON-Schema) --------------------

pub async fn upsert_install_settings(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    config: &serde_json::Value,
    updated_by: Uuid,
) -> DbResult<()> {
    // The install row must exist AND be this org's (RLS posture).
    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM plugin_installs WHERE org_id = $1 AND id = $2) AS ok",
        org_id,
        install_id,
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(false);
    if !exists {
        return Err(DbError::NotFound);
    }
    sqlx::query!(
        r#"INSERT INTO plugin_install_settings (install_id, config, updated_by)
           VALUES ($1, $2::jsonb, $3)
           ON CONFLICT (install_id)
           DO UPDATE SET config = EXCLUDED.config,
                        updated_by = EXCLUDED.updated_by,
                        updated_at = now()"#,
        install_id,
        config,
        updated_by,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn get_install_settings(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<serde_json::Value> {
    let row = sqlx::query(
        "SELECT s.config AS config FROM plugin_install_settings s
             JOIN plugin_installs i ON i.id = s.install_id
             WHERE i.org_id = $1 AND s.install_id = $2",
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some(r) => Ok(r.get("config")),
        None => Ok(serde_json::json!({})),
    }
}

// --- agent skills (distribution columns on the migration-0040 seam) -------------

const SKILL_COLS: &str = "id, org_id, slug, version, manifest, description, \
     enabled, enabled_by, enabled_at, source_kind::text AS source_kind, \
     source_url, fetched_ref, artifact_digest, created_by, created_at";

fn row_to_skill_version(row: &sqlx::postgres::PgRow) -> DbResult<AgentSkillVersion> {
    let manifest_value: serde_json::Value = row.get("manifest");
    let manifest: SkillManifest =
        serde_json::from_value(manifest_value.clone()).unwrap_or(SkillManifest {
            name: row.get("slug"),
            version: String::new(),
            description: row.get("description"),
            capabilities: Vec::new(),
            prompts: Vec::new(),
            rules: Vec::new(),
        });
    Ok(AgentSkillVersion {
        id: row.get("id"),
        org_id: row.get("org_id"),
        slug: row.get("slug"),
        version: row.get::<i32, _>("version"),
        manifest,
        description: row.get("description"),
        source_kind: parse_source_kind(
            row.get::<Option<String>, _>("source_kind")
                .unwrap_or_else(|| "manual_upload".into()),
        )?,
        source_url: row.get("source_url"),
        fetched_ref: row.get("fetched_ref"),
        artifact_digest: row.get("artifact_digest"),
        enabled: row.get("enabled"),
        enabled_by: row.get("enabled_by"),
        enabled_at: row.get("enabled_at"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

/// Appends one skill VERSION carrying the SAME provenance chain shape
/// every other source records. Skill content is untrusted RAG-class
/// data (§4) — persisted verbatim, never executed.
pub async fn insert_skill_version(
    tx: &mut PgConnection,
    version: &AgentSkillVersion,
) -> DbResult<()> {
    let manifest = serde_json::to_value(&version.manifest)
        .map_err(|e| DbError::Conflict(format!("skill manifest does not serialize: {e}")))?;
    sqlx::query!(
        r#"INSERT INTO agent_skills
             (id, org_id, slug, version, manifest, description, source_kind,
              source_url, fetched_ref, artifact_digest, enabled, created_by)
           VALUES ($1, $2, $3, $4, $5::jsonb, $6,
                   $7::text::plugin_source_kind, $8, $9, $10, false, $11)"#,
        version.id,
        version.org_id,
        version.slug,
        version.version,
        manifest,
        version.description,
        version.source_kind.as_str(),
        version.source_url,
        version.fetched_ref,
        version.artifact_digest,
        version.created_by,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn get_skill_version(
    tx: &mut PgConnection,
    org_id: Uuid,
    skill_id: Uuid,
) -> DbResult<AgentSkillVersion> {
    let row = sqlx::query(&format!(
        "SELECT {SKILL_COLS} FROM agent_skills WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(skill_id)
    .fetch_one(&mut *tx)
    .await?;
    row_to_skill_version(&row)
}

/// The org's skills — the ACTIVE (latest) version of every slug.
pub async fn list_latest_skills(
    tx: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<AgentSkillVersion>> {
    let rows = sqlx::query(&format!(
        "SELECT {SKILL_COLS} FROM agent_skills
         WHERE org_id = $1 AND id IN (
           SELECT DISTINCT ON (slug) id FROM agent_skills
           WHERE org_id = $1 ORDER BY slug, version DESC)
         ORDER BY slug"
    ))
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_skill_version).collect()
}

/// Every version of one slug, newest first (the diff/rollback view).
pub async fn list_skill_versions(
    tx: &mut PgConnection,
    org_id: Uuid,
    slug: &str,
) -> DbResult<Vec<AgentSkillVersion>> {
    let rows = sqlx::query(&format!(
        "SELECT {SKILL_COLS} FROM agent_skills
         WHERE org_id = $1 AND slug = $2 ORDER BY version DESC"
    ))
    .bind(org_id)
    .bind(slug)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_skill_version).collect()
}

/// Enable/disable gate on the ACTIVE version row of a slug
/// (deny-by-default: enable is the explicit admin action; version rows
/// other than the active one are never enabled).
pub async fn set_skill_enabled(
    tx: &mut PgConnection,
    org_id: Uuid,
    slug: &str,
    enabled: bool,
    actor: Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<()> {
    let updated = if enabled {
        sqlx::query!(
            "UPDATE agent_skills SET enabled = true, enabled_by = $3,
                 enabled_at = $4
             WHERE org_id = $1 AND slug = $2
               AND version = (SELECT MAX(version) FROM agent_skills
                              WHERE org_id = $1 AND slug = $2)",
            org_id,
            slug,
            actor,
            now,
        )
        .execute(&mut *tx)
        .await?
    } else {
        sqlx::query!(
            "UPDATE agent_skills SET enabled = false
             WHERE org_id = $1 AND slug = $2
               AND version = (SELECT MAX(version) FROM agent_skills
                              WHERE org_id = $1 AND slug = $2)",
            org_id,
            slug,
        )
        .execute(&mut *tx)
        .await?
    };
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// Validates one skill manifest JSON at the write boundary (the domain
/// gate: name must exist).
pub fn parse_skill_manifest(value: serde_json::Value) -> Result<SkillManifest, DbError> {
    let manifest: SkillManifest = serde_json::from_value(value)
        .map_err(|e| DbError::Conflict(format!("skill manifest is unreadable: {e}")))?;
    manifest
        .validate()
        .map_err(|e| DbError::Conflict(format!("invalid skill manifest: {e}")))?;
    Ok(manifest)
}

// --- provenance chain tail --------------------------------------------------------

/// The LATEST provenance link of one install (the chain-context the
/// lifecycle appends reference).
pub async fn latest_provenance(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<PluginProvenanceEntry> {
    let row = sqlx::query(&format!(
        "SELECT {PROVENANCE_COLS} FROM plugin_install_provenance
         WHERE org_id = $1 AND install_id = $2
         ORDER BY recorded_at DESC LIMIT 1"
    ))
    .bind(org_id)
    .bind(install_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_provenance(&row)
}

// --- Marketplace v2 (Wave 22): verified publishers + ratings ---------------

const PUBLISHER_COLS: &str = "id, org_id, publisher, display_name, status, \
     verified_by, verified_at, revoked_at, revoked_by, note";

fn row_to_publisher(row: &sqlx::postgres::PgRow) -> DbResult<PluginPublisherVerification> {
    let model = PluginPublisherVerification {
        id: row.get("id"),
        org_id: row.get("org_id"),
        publisher: row.get("publisher"),
        display_name: row.get("display_name"),
        status: row.get("status"),
        verified_by: row.get("verified_by"),
        verified_at: row.get("verified_at"),
        revoked_at: row.get("revoked_at"),
        revoked_by: row.get("revoked_by"),
        note: row.get("note"),
    };
    model
        .validate()
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    Ok(model)
}

/// Inserts a new publisher verification row (status flips ride UPDATE).
pub async fn create_publisher_verification(
    tx: &mut PgConnection,
    model: &PluginPublisherVerification,
) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO plugin_publisher_verifications
           (id, org_id, publisher, display_name, status, verified_by, verified_at, revoked_at,
            revoked_by, note)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(model.id)
    .bind(model.org_id)
    .bind(model.publisher.trim())
    .bind(model.display_name.trim())
    .bind(&model.status)
    .bind(model.verified_by)
    .bind(model.verified_at)
    .bind(model.revoked_at)
    .bind(model.revoked_by)
    .bind(&model.note)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn list_publisher_verifications<'e, E>(
    executor: E,
    org_id: Uuid,
    active_only: bool,
) -> DbResult<Vec<PluginPublisherVerification>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {PUBLISHER_COLS} FROM plugin_publisher_verifications
         WHERE org_id = $1 AND ($2::boolean IS FALSE OR status = 'verified')
         ORDER BY verified_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(active_only)
        .fetch_all(executor)
        .await?;
    rows.iter().map(row_to_publisher).collect()
}

/// Finds the verification row for one normalized publisher identity.
pub async fn get_publisher_by_identity(
    executor: &mut PgConnection,
    org_id: Uuid,
    publisher: &str,
) -> DbResult<PluginPublisherVerification> {
    let sql = format!(
        "SELECT {PUBLISHER_COLS} FROM plugin_publisher_verifications
         WHERE org_id = $1 AND publisher = $2"
    );
    sqlx::query(&sql)
        .bind(org_id)
        .bind(publisher)
        .fetch_optional(&mut *executor)
        .await?
        .ok_or(DbError::NotFound)
        .and_then(|r| row_to_publisher(&r))
}

/// Flips the verification state of one row (`verified` | `revoked`).
/// Revocation stamps the revocation envelope; re-verification clears it.
pub async fn set_publisher_status(
    tx: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
    status: &str,
    actor: Option<Uuid>,
    now: DateTime<Utc>,
) -> DbResult<()> {
    let (revoked_at, revoked_by, display) = match status {
        "verified" => (None, None, "verified"),
        "revoked" => (Some(now), actor, "revoked"),
        other => return Err(DbError::Conflict(format!("unknown status {other:?}"))),
    };
    let updated = sqlx::query(
        "UPDATE plugin_publisher_verifications
         SET status = $3, revoked_at = $4, revoked_by = $5
         WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(id)
    .bind(display)
    .bind(revoked_at)
    .bind(revoked_by)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

const RATING_COLS: &str = "id, org_id, source_kind::text AS source_kind, source_url, \
     artifact_digest, rating, comment, rated_by, rated_at, updated_at";

fn row_to_rating(row: &sqlx::postgres::PgRow) -> DbResult<PluginRating> {
    let kind = parse_source_kind(row.get("source_kind"))?;
    Ok(PluginRating {
        id: row.get("id"),
        org_id: row.get("org_id"),
        source_kind: kind,
        source_url: row.get("source_url"),
        artifact_digest: row.get("artifact_digest"),
        rating: row.get("rating"),
        comment: row.get("comment"),
        rated_by: row.get("rated_by"),
        rated_at: row.get("rated_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Upserts the caller's rating on one artifact release (re-rate wins).
pub async fn upsert_rating(tx: &mut PgConnection, model: &PluginRating) -> DbResult<()> {
    sqlx::query(
        "INSERT INTO plugin_ratings
           (id, org_id, source_kind, source_url, artifact_digest, rating, comment, rated_by,
            rated_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
         ON CONFLICT (org_id, rated_by, source_url, artifact_digest) DO UPDATE
         SET rating = EXCLUDED.rating,
             comment = EXCLUDED.comment,
             updated_at = EXCLUDED.updated_at",
    )
    .bind(model.id)
    .bind(model.org_id)
    .bind(model.source_kind.as_str())
    .bind(&model.source_url)
    .bind(&model.artifact_digest)
    .bind(model.rating)
    .bind(&model.comment)
    .bind(model.rated_by)
    .bind(model.rated_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Lists the org's ratings, optionally narrowed to one artifact release.
pub async fn list_ratings<'e, E>(
    executor: E,
    org_id: Uuid,
    source_url: Option<&str>,
    artifact_digest: Option<&str>,
) -> DbResult<Vec<PluginRating>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let sql = format!(
        "SELECT {RATING_COLS} FROM plugin_ratings
         WHERE org_id = $1
           AND ($2::text IS NULL OR source_url = $2)
           AND ($3::text IS NULL OR artifact_digest = $3)
         ORDER BY updated_at DESC LIMIT 500"
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(source_url.map(str::trim).filter(|s| !s.is_empty()))
        .bind(artifact_digest.map(str::trim).filter(|s| !s.is_empty()))
        .fetch_all(executor)
        .await?;
    rows.iter().map(row_to_rating).collect()
}
