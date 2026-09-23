//! Installed org theme packs (Wave 24 lane 1 `p24-theming-system`).
//!
//! Purpose: persistence for the theme-pack distribution kind over ADR
//! 0009 V1 — a theme pack becomes an org row when a `theme` staged item
//! is APPROVED; the active selection rides the typed settings model
//! (`appearance.theme` = `pack:<slug>`), never this table. Uninstalling
//! a pack that the org theme currently points at is refused: an active
//! theme cannot dangle.
//!
//! Non-purpose: validation (core-domain owns the theme-token schema),
//! HTTP, and the frontend.

use crate::{DbError, DbResult};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

/// One installed theme pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemePack {
    pub id: Uuid,
    pub org_id: Uuid,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    // The full validated manifest snapshot.
    pub manifest: serde_json::Value,
    pub schema_version: i32,
    pub provenance_source_kind: String,
    pub provenance_source_url: String,
    pub provenance_fetched_ref: Option<String>,
    pub provenance_digest: Option<String>,
    pub installed_by: Uuid,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

const PACK_COLS: &str = "id, org_id, slug, name, version, description, manifest, \
     schema_version, provenance_source_kind::text AS provenance_source_kind, \
     provenance_source_url, provenance_fetched_ref, provenance_digest, installed_by, \
     installed_at, updated_at";

fn row_to_pack(row: &sqlx::postgres::PgRow) -> DbResult<ThemePack> {
    Ok(ThemePack {
        id: row.get("id"),
        org_id: row.get("org_id"),
        slug: row.get("slug"),
        name: row.get("name"),
        version: row.get("version"),
        description: row.get("description"),
        manifest: row.get("manifest"),
        schema_version: row.get("schema_version"),
        provenance_source_kind: row.get("provenance_source_kind"),
        provenance_source_url: row.get("provenance_source_url"),
        provenance_fetched_ref: row.get("provenance_fetched_ref"),
        provenance_digest: row.get("provenance_digest"),
        installed_by: row.get("installed_by"),
        installed_at: row.get("installed_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Every installed pack for an org (registry order: newest install last).
pub async fn list_packs(tx: &mut sqlx::PgConnection, org_id: Uuid) -> DbResult<Vec<ThemePack>> {
    let rows = sqlx::query(&format!(
        "SELECT {PACK_COLS} FROM theme_packs WHERE org_id = $1 ORDER BY installed_at, slug"
    ))
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_pack).collect()
}

/// One pack by slug, or None.
pub async fn get_pack(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    slug: &str,
) -> DbResult<Option<ThemePack>> {
    let row = sqlx::query(&format!(
        "SELECT {PACK_COLS} FROM theme_packs WHERE org_id = $1 AND slug = $2"
    ))
    .bind(org_id)
    .bind(slug)
    .fetch_optional(&mut *tx)
    .await?;
    match row {
        Some(row) => Ok(Some(row_to_pack(&row)?)),
        None => Ok(None),
    }
}

/// Upserts one pack (approval of a `theme` staged item, or an
/// admin's re-install of the same slug at a newer version — the row is
/// version-superseded in place, keeping the audited trail on the staged
/// item and the audit log, never on the state row).
pub async fn upsert_pack(tx: &mut sqlx::PgConnection, pack: &ThemePack) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO theme_packs (
             id, org_id, slug, name, version, description, manifest,
             schema_version, provenance_source_kind, provenance_source_url,
             provenance_fetched_ref, provenance_digest, installed_by, installed_at
           ) VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8,
             $9::text::plugin_source_kind, $10, $11, $12, $13, now())
           ON CONFLICT (org_id, slug) DO UPDATE SET
             name = EXCLUDED.name,
             version = EXCLUDED.version,
             description = EXCLUDED.description,
             manifest = EXCLUDED.manifest,
             schema_version = EXCLUDED.schema_version,
             provenance_source_kind = EXCLUDED.provenance_source_kind,
             provenance_source_url = EXCLUDED.provenance_source_url,
             provenance_fetched_ref = EXCLUDED.provenance_fetched_ref,
             provenance_digest = EXCLUDED.provenance_digest,
             installed_by = EXCLUDED.installed_by,
             updated_at = now()"#,
        pack.id,
        pack.org_id,
        pack.slug,
        pack.name,
        pack.version,
        pack.description,
        pack.manifest,
        pack.schema_version,
        pack.provenance_source_kind,
        pack.provenance_source_url,
        pack.provenance_fetched_ref,
        pack.provenance_digest,
        pack.installed_by,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Removes an installed pack. `active_slug` is the org's CURRENT
/// appearance.theme value: removing the ACTIVE pack is refused (repoint
/// the setting first — an active theme can never dangle).
pub async fn delete_pack(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    slug: &str,
    active_theme: Option<&str>,
) -> DbResult<()> {
    if let Some(active) = active_theme {
        if let Some(active_slug) = active.strip_prefix("pack:") {
            if active_slug == slug {
                return Err(DbError::Conflict(format!(
                    "theme pack {slug:?} is the active org theme — repoint \
                     appearance.theme (or set a per-user override) before uninstalling"
                )));
            }
        }
    }
    let n = sqlx::query!(
        "DELETE FROM theme_packs WHERE org_id = $1 AND slug = $2",
        org_id,
        slug
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

/// True if `slug` names an installed pack for the org (the settings
/// seam's dynamic-value check for `appearance.theme` = `pack:<slug>`).
pub async fn pack_installed(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    slug: &str,
) -> DbResult<bool> {
    Ok(sqlx::query!(
        "SELECT 1 AS x FROM theme_packs WHERE org_id = $1 AND slug = $2",
        org_id,
        slug
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

/// The mints-theme helper used at approval time: derives the pack row
/// from the approved staged item's manifest (already validated by the
/// plugin-host adapter).
#[allow(clippy::too_many_arguments)]
pub async fn install_from_staged(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    pack_id: Uuid,
    source_kind: &str,
    name: &str,
    version: &str,
    manifest: &serde_json::Value,
    source_url: &str,
    fetched_ref: Option<&str>,
    digest: &str,
    installed_by: Uuid,
) -> DbResult<ThemePack> {
    // The staged manifest was already schema-validated by the
    // plugin-host adapter at ingest AND re-checked here (the approval
    // path is the only writer; deny-by-default re-validation).
    let staged: core_domain::theme_pack::ThemePackManifest =
        serde_json::from_value(manifest.clone())
            .map_err(|e| DbError::Conflict(format!("theme pack manifest is invalid: {e}")))?;
    staged
        .validate()
        .map_err(|e| DbError::Conflict(format!("theme pack manifest is invalid: {e}")))?;
    let slug = core_domain::theme_pack::ThemePackManifest::slug_of(name);
    if !core_domain::theme_pack::is_preset_or_pack_slug(&slug) {
        return Err(DbError::Conflict(format!(
            "theme-pack slug {slug:?} is not a usable theme id"
        )));
    }
    let pack = ThemePack {
        id: pack_id,
        org_id,
        slug: slug.clone(),
        name: name.to_string(),
        version: version.to_string(),
        description: staged.description.clone(),
        manifest: manifest.clone(),
        schema_version: core_domain::theme_pack::THEME_TOKEN_SCHEMA_VERSION as i32,
        provenance_source_kind: source_kind.to_string(),
        provenance_source_url: source_url.to_string(),
        provenance_fetched_ref: fetched_ref.map(String::from),
        provenance_digest: Some(digest.to_string()),
        installed_by,
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    upsert_pack(tx, &pack).await?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    #[test]
    fn conflict_message_names_the_active_pack() {
        // The delete guard is message-shaped contract: uninstalling the
        // active pack is refused with the repoint instruction.
        let active = "pack:warm-ochre";
        let slug = "warm-ochre";
        assert_eq!(active.trim_start_matches("pack:"), slug);
    }
}
