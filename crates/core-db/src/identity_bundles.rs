//! Installed identity bundles (lane `p52-identity-bundle`).
//!
//! Persistence only: an identity bundle becomes an org row when an
//! `identity_bundle` staged item is APPROVED, and the row is the review
//! record the executable half registers from. Validation lives in
//! `core_domain::entities::identity_bundle`; the HTTP surface lives in
//! core-api.
//!
//! Two rules the queries here enforce rather than assume:
//!
//! * **Deny-by-default.** [`install_from_staged`] always writes
//!   `status = 'disabled'`; the only path to `enabled` is
//!   [`set_status`], which routes through the domain state machine and
//!   refuses to revive a revoked row.
//! * **The pin is not caller-supplied.** The digest column is taken from
//!   the approved staged item, validated as sha256, and never re-read
//!   from the request body.
//!
//! Queries are runtime (`sqlx::query`) rather than the compile-time
//! `query!` macro on purpose: the macro pins every statement into the
//! offline `.sqlx` cache, and this lane runs light checks only — a
//! schema the lane cannot prepare against must not require a database to
//! compile.

use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use core_domain::entities::identity_bundle::{
    IdentityBundleAction, IdentityBundleInstall, IdentityBundleManifest,
};
use core_domain::entities::plugins::PluginInstallStatus;
use sqlx::Row;
use uuid::Uuid;

const INSTALL_COLS: &str = "id, org_id, slug, name, version, interface_version, provider_kinds, \
     scopes, manifest, provenance_source_kind::text AS provenance_source_kind, \
     provenance_source_url, provenance_fetched_ref, provenance_digest, status::text AS status, \
     jit_enabled, revoked_reason, installed_by, installed_at, updated_at";

fn row_to_install(row: &sqlx::postgres::PgRow) -> DbResult<IdentityBundleInstall> {
    let status_raw: String = row.get("status");
    let status = PluginInstallStatus::parse(&status_raw)
        .ok_or_else(|| DbError::Conflict(format!("unknown install status {status_raw:?}")))?;
    Ok(IdentityBundleInstall {
        id: row.get("id"),
        org_id: row.get("org_id"),
        slug: row.get("slug"),
        name: row.get("name"),
        version: row.get("version"),
        interface_version: row.get::<i32, _>("interface_version") as u32,
        provider_kinds: row.get("provider_kinds"),
        scopes: row.get("scopes"),
        manifest: row.get("manifest"),
        provenance_source_kind: row.get("provenance_source_kind"),
        provenance_source_url: row.get("provenance_source_url"),
        provenance_fetched_ref: row.get("provenance_fetched_ref"),
        provenance_digest: row.get("provenance_digest"),
        status,
        jit_enabled: row.get("jit_enabled"),
        revoked_reason: row.get("revoked_reason"),
        installed_by: row.get("installed_by"),
        installed_at: row.get("installed_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Every install for an org, newest last — the admin review list.
pub async fn list_installs(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<IdentityBundleInstall>> {
    let rows = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM identity_bundle_installs \
         WHERE org_id = $1 ORDER BY installed_at, id"
    ))
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    rows.iter().map(row_to_install).collect()
}

pub async fn get_install(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<IdentityBundleInstall> {
    let row = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM identity_bundle_installs WHERE org_id = $1 AND id = $2"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_install(&row)
}

pub async fn get_by_slug(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    slug: &str,
) -> DbResult<IdentityBundleInstall> {
    let row = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM identity_bundle_installs WHERE org_id = $1 AND slug = $2"
    ))
    .bind(org_id)
    .bind(slug)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_install(&row)
}

/// The mints-the-install helper used at approval time. The staged
/// manifest was already validated by the plugin-host adapter at ingest
/// and is re-validated here — the approval path is the only writer, so it
/// re-checks rather than trusting the row.
///
/// A re-install of the same bundle (same org, same slug) REPLACES the
/// row: version, manifest, provenance and pin move together, the status
/// returns to `disabled`, and any prior `revoked_reason` clears. Nothing
/// carries an "enabled" flag across an install — the new artifact has to
/// be switched on again, deliberately.
#[allow(clippy::too_many_arguments)]
pub async fn install_from_staged(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    staged_item_id: Option<Uuid>,
    source_kind: &str,
    source_url: &str,
    fetched_ref: Option<&str>,
    digest: &str,
    manifest_json: &serde_json::Value,
    installed_by: Uuid,
) -> DbResult<IdentityBundleInstall> {
    let manifest = IdentityBundleManifest::from_value(manifest_json)
        .map_err(|e| DbError::Conflict(format!("identity bundle manifest is invalid: {e}")))?;
    let row = IdentityBundleInstall::from_approved_staged(
        install_id,
        org_id,
        &manifest,
        manifest_json.clone(),
        source_kind,
        source_url,
        fetched_ref,
        digest,
        installed_by,
        Utc::now(),
    )
    .map_err(|e| DbError::Conflict(format!("identity bundle install is invalid: {e}")))?;

    let stored = sqlx::query(&format!(
        "INSERT INTO identity_bundle_installs (
             id, org_id, slug, name, version, interface_version, provider_kinds, scopes,
             manifest, provenance_source_kind, provenance_source_url, provenance_fetched_ref,
             provenance_digest, status, jit_enabled, revoked_reason, installed_by,
             installed_at, updated_at
         ) VALUES (
             $1, $2, $3, $4, $5, $6, $7, $8, $9, $10::text::plugin_source_kind, $11, $12,
             $13, $14::text::plugin_install_status, $15, NULL, $16, $17, $17
         )
         ON CONFLICT (org_id, slug) DO UPDATE SET
             -- `id` is deliberately NOT replaced: an earlier staged row's
             -- `adopted_identity_bundle_id` references this row, and the
             -- slug is the stable identity inside an org. Re-installing
             -- moves the artifact, not the handle.
             name = EXCLUDED.name,
             version = EXCLUDED.version,
             interface_version = EXCLUDED.interface_version,
             provider_kinds = EXCLUDED.provider_kinds,
             scopes = EXCLUDED.scopes,
             manifest = EXCLUDED.manifest,
             provenance_source_kind = EXCLUDED.provenance_source_kind,
             provenance_source_url = EXCLUDED.provenance_source_url,
             provenance_fetched_ref = EXCLUDED.provenance_fetched_ref,
             provenance_digest = EXCLUDED.provenance_digest,
             status = 'disabled'::plugin_install_status,
             jit_enabled = false,
             revoked_reason = NULL,
             installed_by = EXCLUDED.installed_by,
             installed_at = EXCLUDED.installed_at,
             updated_at = EXCLUDED.updated_at
         RETURNING {INSTALL_COLS}"
    ))
    .bind(row.id)
    .bind(org_id)
    .bind(&row.slug)
    .bind(&row.name)
    .bind(&row.version)
    .bind(row.interface_version as i32)
    .bind(&row.provider_kinds)
    .bind(&row.scopes)
    .bind(&row.manifest)
    .bind(&row.provenance_source_kind)
    .bind(&row.provenance_source_url)
    .bind(&row.provenance_fetched_ref)
    .bind(&row.provenance_digest)
    .bind(row.status.as_str())
    .bind(row.jit_enabled)
    .bind(row.installed_by)
    .bind(row.installed_at)
    .fetch_one(&mut *tx)
    .await?;
    let install = row_to_install(&stored)?;

    // Stamp the staged row's adoption handle. The approval path sets the
    // status afterwards; the coherence CHECK admits an adoption link on a
    // still-pending row, which is exactly this window.
    if let Some(staged_id) = staged_item_id {
        sqlx::query(
            "UPDATE plugin_staged_items SET adopted_identity_bundle_id = $3, updated_at = now() \
             WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(staged_id)
        .bind(install.id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(install)
}

/// Applies one lifecycle action, refusing anything the domain state
/// machine refuses. A revoked install is terminal here, not by UI
/// convention: the UPDATE cannot reach a `revoked` row because the
/// transition is checked under `FOR UPDATE` first.
pub async fn set_status(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    action: IdentityBundleAction,
    reason: Option<&str>,
    now: DateTime<Utc>,
) -> DbResult<IdentityBundleInstall> {
    let mut install = get_install_for_update(tx, org_id, id).await?;
    install
        .apply(action, reason.map(String::from), now)
        .map_err(|e| DbError::Conflict(e.to_string()))?;
    let row = sqlx::query(&format!(
        "UPDATE identity_bundle_installs SET
             status = $3::text::plugin_install_status,
             revoked_reason = $4,
             updated_at = $5
         WHERE org_id = $1 AND id = $2
         RETURNING {INSTALL_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(install.status.as_str())
    .bind(&install.revoked_reason)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    row_to_install(&row)
}

/// The JIT opt-in is separate from enable/disable on purpose: a bundle
/// can be enabled for existing linked identities while provisioning
/// stays off.
pub async fn set_jit_enabled(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
    enabled: bool,
    now: DateTime<Utc>,
) -> DbResult<IdentityBundleInstall> {
    let current = get_install_for_update(tx, org_id, id).await?;
    if current.status == PluginInstallStatus::Revoked {
        return Err(DbError::Conflict(
            "a revoked identity bundle accepts no configuration changes".into(),
        ));
    }
    let row = sqlx::query(&format!(
        "UPDATE identity_bundle_installs SET jit_enabled = $3, updated_at = $4
         WHERE org_id = $1 AND id = $2
         RETURNING {INSTALL_COLS}"
    ))
    .bind(org_id)
    .bind(id)
    .bind(enabled)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    row_to_install(&row)
}

/// Re-fetch verification: does the artifact a caller holds still match
/// the pin the reviewer approved? A drifted bundle refuses to register.
pub fn digest_matches(install: &IdentityBundleInstall, artifact_bytes: &[u8]) -> bool {
    install.pins(artifact_bytes)
}

/// What the approval path audits, assembled so the route cannot forget a
/// field. Deliberately carries NO manifest and no settings value: an
/// issuer URL or client id is configuration, not event data.
pub fn install_audit_detail(install: &IdentityBundleInstall) -> serde_json::Value {
    serde_json::json!({
        "install_id": install.id,
        "name": install.name,
        "version": install.version,
        "interface_version": install.interface_version,
        "provider_kinds": install.provider_kinds,
        "scopes": install.scopes,
        "artifact_digest": install.provenance_digest,
        "source_kind": install.provenance_source_kind,
        "source_url": install.provenance_source_url,
        "fetched_ref": install.provenance_fetched_ref,
        "status": install.status.as_str(),
    })
}

async fn get_install_for_update(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<IdentityBundleInstall> {
    let row = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM identity_bundle_installs \
         WHERE org_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(org_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    row_to_install(&row)
}

/// The lifecycle transitions a caller may ask for, as text — one
/// vocabulary shared by the route layer and the audit detail.
pub fn action_or_refuse(value: &str) -> Result<IdentityBundleAction, DbError> {
    IdentityBundleAction::parse(value)
        .ok_or_else(|| DbError::Conflict(format!("unknown identity bundle action {value:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_vocabulary_round_trips_through_the_column() {
        for status in [
            PluginInstallStatus::Disabled,
            PluginInstallStatus::Enabled,
            PluginInstallStatus::Revoked,
        ] {
            assert_eq!(PluginInstallStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(PluginInstallStatus::parse("live"), None);
    }

    #[test]
    fn actions_are_a_closed_vocabulary() {
        for action in [
            IdentityBundleAction::Enable,
            IdentityBundleAction::Disable,
            IdentityBundleAction::Revoke,
        ] {
            assert_eq!(
                action_or_refuse(action.as_str()).expect("round trip"),
                action
            );
        }
        assert!(action_or_refuse("uninstall").is_err());
    }

    #[test]
    fn the_digest_check_is_the_domain_pin() {
        let bytes = b"identity bundle archive bytes";
        let pin = core_domain::entities::identity_bundle::plugin_digest_of(bytes);
        assert_eq!(pin.len(), 64);
        let mut install = IdentityBundleInstall::from_approved_staged(
            Uuid::nil(),
            Uuid::nil(),
            &IdentityBundleManifest::from_value(&serde_json::json!({
                "name": "corp-okta",
                "version": "1.4.0",
                "interface_version": 1,
                "provider_kinds": ["oidc"],
                "settings_schema": {"type": "object", "properties": {}},
                "scopes": ["openid"]
            }))
            .expect("manifest"),
            serde_json::json!({}),
            "local_zip",
            "local://zip-import",
            None,
            &pin,
            Uuid::nil(),
            Utc::now(),
        )
        .expect("install");
        assert!(digest_matches(&install, bytes));
        assert!(!digest_matches(&install, b"tampered"));
        // The audit detail carries the pin and the transports, never a
        // settings VALUE (issuer URLs and client ids are config, not
        // event data).
        install.scopes = vec!["openid".into()];
        let detail = install_audit_detail(&install);
        assert_eq!(detail["artifact_digest"], serde_json::json!(pin));
        assert!(detail.get("manifest").is_none());
    }
}
