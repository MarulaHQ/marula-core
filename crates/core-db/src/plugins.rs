//! Marketplace registry persistence (Wave 14 lane 5): the global
//! plugin catalog, per-version manifest snapshots, and org installs.
//!
//! Purpose: CRUD for `plugin_catalog` / `plugin_versions` /
//! `plugin_installs` plus the registry-revocation cascade that marks
//! every org install of a revoked entry revoked (deny-by-default — a
//! revoked plugin never runs). Install = adoption of a catalog entry at
//! a PINNED version; upgrades are explicit updates, never automatic.
//!
//! Non-purpose: capability meaning (core-domain owns the closed
//! vocabulary and its mapping onto the permission matrix), invocation
//! authorization (the runtime seam in `plugin-host` resolves grants
//! through `core_domain::policy`), outbox emission (routes emit inside
//! their transaction).

use crate::{DbError, DbResult};
use core_domain::entities::{
    PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginVersion,
};
use sqlx::{PgConnection, Row};
use uuid::Uuid;

const CATALOG_COLS: &str = "id, slug, name, description, publisher, origin_org_id, \
     latest_version, interface_version, revoked_at, revoked_reason, created_at, updated_at";

fn row_to_catalog_entry(row: &sqlx::postgres::PgRow) -> DbResult<PluginCatalogEntry> {
    Ok(PluginCatalogEntry {
        id: row.get("id"),
        slug: row.get("slug"),
        name: row.get("name"),
        description: row.get("description"),
        publisher: row.get("publisher"),
        origin_org_id: row.get("origin_org_id"),
        latest_version: row.get("latest_version"),
        interface_version: row.get::<i32, _>("interface_version") as u32,
        revoked_at: row.get("revoked_at"),
        revoked_reason: row.get("revoked_reason"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

const INSTALL_COLS: &str = "i.id, i.org_id, i.catalog_id, c.slug AS slug, i.version, \
     i.module_digest, i.status::text AS status, i.granted_capabilities, i.installed_by, \
     i.installed_at, i.updated_at, i.revoked_at, i.revoked_by, i.revoked_reason, \
     i.previous_version, i.previous_module_digest, i.previous_capabilities";

fn row_to_install(row: &sqlx::postgres::PgRow) -> DbResult<PluginInstall> {
    let status: String = row.get("status");
    let granted: serde_json::Value = row.get("granted_capabilities");
    let granted_capabilities = granted
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(PluginInstall {
        id: row.get("id"),
        org_id: row.get("org_id"),
        catalog_id: row.get("catalog_id"),
        slug: row.get("slug"),
        version: row.get("version"),
        module_digest: row.get("module_digest"),
        status: PluginInstallStatus::parse(&status).ok_or_else(|| {
            DbError::Conflict(format!("unknown plugin install status {status:?}"))
        })?,
        granted_capabilities,
        installed_by: row.get("installed_by"),
        installed_at: row.get("installed_at"),
        updated_at: row.get("updated_at"),
        revoked_at: row.get("revoked_at"),
        revoked_by: row.get("revoked_by"),
        revoked_reason: row.get("revoked_reason"),
        previous_version: row.get("previous_version"),
        previous_module_digest: row.get("previous_module_digest"),
        previous_capabilities: row
            .get::<Option<serde_json::Value>, _>("previous_capabilities")
            .map(|value| crate::plugin_distribution::caps_from_json(&value))
            .unwrap_or_default(),
    })
}

/// Wave 43 lane 4 (E): the EMIT seam's validation half.
///
/// A plugin runtime authorized to emit (`events.emit`, the capability
/// beside `automation.actions`) publishes an envelope of a type it
/// DECLARED — and the declaration is checked here, against lane 2's
/// event-type registry (`event_types`, migration 0124), never against a
/// list this lane maintains. A type is admissible when it is a PLATFORM
/// type (no grant needed) or the org holds an `event_type_grants` row;
/// anything else is refused rather than published, which is what stops a
/// plugin emitting `acme.crm.contact_created` that nobody admitted.
///
/// Returns the types that were NOT admitted. An empty list is the pass;
/// a non-empty one is a typed refusal the caller turns into prose.
///
/// This is deliberately NOT the default path for a connector: inbound
/// through lane 1's generic signed ingest needs no surface at all, and
/// the authoring guide says so first. Emit is for a runtime that wants to
/// publish without a round trip through the receiver.
pub async fn validate_declared_event_types(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    event_types: &[String],
) -> DbResult<Vec<String>> {
    crate::event_types::check_event_types(pool, org_id, event_types).await
}

// --- catalog -----------------------------------------------------------------

/// Publishes a catalog entry plus its first version snapshot.
/// Caller generates ids/timestamps; the entry's `latest_version` is
/// written from the version row.
///
/// The version's manifest lands UNAUTHENTICATED. This is the declarative
/// publish shape: the manifest arrives as a JSON body on an authenticated admin
/// call, so there are no artifact bytes and no publisher statement for anyone to
/// verify. Recording `true` here would claim a signature that does not exist
/// (migration 0172's header). A caller that DOES hold a verdict — the staged
/// approval path, which carries the adapter's answer on the staged row — uses
/// [`create_catalog_entry_with_manifest_auth`].
pub async fn create_catalog_entry(
    tx: &mut PgConnection,
    entry: &PluginCatalogEntry,
    version: &PluginVersion,
) -> DbResult<()> {
    create_catalog_entry_with_manifest_auth(tx, entry, version, false).await
}

/// [`create_catalog_entry`] for a caller that holds the adapter's
/// manifest-authentication verdict (`plugin_staged_items.manifest_authenticated`,
/// 0167) and needs it to travel onto the registry row that other orgs install
/// from. Without this hop the weaker claim died with the staged item.
pub async fn create_catalog_entry_with_manifest_auth(
    tx: &mut PgConnection,
    entry: &PluginCatalogEntry,
    version: &PluginVersion,
    manifest_authenticated: bool,
) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO plugin_catalog (id, slug, name, description, publisher,
                                       origin_org_id, latest_version, interface_version)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        entry.id,
        entry.slug,
        entry.name,
        entry.description,
        entry.publisher,
        entry.origin_org_id,
        version.version,
        entry.interface_version as i32,
    )
    .execute(&mut *tx)
    .await?;
    insert_version(tx, version, manifest_authenticated).await
}

pub async fn insert_version(
    tx: &mut PgConnection,
    version: &PluginVersion,
    manifest_authenticated: bool,
) -> DbResult<()> {
    let manifest = serde_json::to_value(&version.manifest)
        .map_err(|e| DbError::Conflict(format!("manifest does not serialize: {e}")))?;
    // `query()` because the column list gained `module_digest` in 0156 (and
    // `manifest_authenticated` in 0172) and a changed `query!` needs a refreshed
    // offline cache (a group-gate action).
    sqlx::query(
        r#"INSERT INTO plugin_versions (id, catalog_id, version, manifest, module_digest,
                                       manifest_authenticated)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(version.id)
    .bind(version.catalog_id)
    .bind(&version.version)
    .bind(manifest)
    .bind(&version.module_digest)
    .bind(manifest_authenticated)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Appends a new version to a LIVE entry and moves `latest_version`.
/// Unknown entries are NotFound; revoked entries accept no new
/// versions (the row lock makes the check race-free).
///
/// Unauthenticated, for the same reason [`create_catalog_entry`] is: the
/// declarative append carries no statement to verify. The staged path uses
/// [`append_version_with_manifest_auth`].
pub async fn append_version(
    tx: &mut PgConnection,
    catalog_id: Uuid,
    version: &PluginVersion,
) -> DbResult<()> {
    append_version_with_manifest_auth(tx, catalog_id, version, false).await
}

/// [`append_version`] carrying the adapter's manifest-authentication verdict, so
/// a version appended from a staged import keeps the claim the staged row made.
pub async fn append_version_with_manifest_auth(
    tx: &mut PgConnection,
    catalog_id: Uuid,
    version: &PluginVersion,
    manifest_authenticated: bool,
) -> DbResult<()> {
    let revoked_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT revoked_at FROM plugin_catalog WHERE id = $1 FOR UPDATE")
            .bind(catalog_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(DbError::NotFound)?;
    if revoked_at.is_some() {
        return Err(DbError::Conflict(
            "revoked catalog entries accept no new versions".into(),
        ));
    }
    insert_version(tx, version, manifest_authenticated).await?;
    sqlx::query!(
        "UPDATE plugin_catalog SET latest_version = $2, updated_at = now() WHERE id = $1",
        catalog_id,
        version.version,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn get_catalog_entry(
    executor: &mut PgConnection,
    slug: &str,
) -> DbResult<PluginCatalogEntry> {
    let row = sqlx::query(&format!(
        "SELECT {CATALOG_COLS} FROM plugin_catalog WHERE slug = $1"
    ))
    .bind(slug)
    .fetch_one(executor)
    .await?;
    row_to_catalog_entry(&row)
}

pub async fn list_catalog(executor: &mut PgConnection) -> DbResult<Vec<PluginCatalogEntry>> {
    let rows = sqlx::query(&format!(
        "SELECT {CATALOG_COLS} FROM plugin_catalog ORDER BY slug"
    ))
    .fetch_all(executor)
    .await?;
    rows.iter().map(row_to_catalog_entry).collect()
}

/// The registry-level kill switch: stamps the entry revoked and marks
/// every org install of it revoked in the same transaction (deny-by-
/// default — a revoked plugin never runs). Returns the (install_id,
/// org_id) pairs the cascade touched, for the route layer's event
/// emission. Unknown entries are NotFound; double revoke is a Conflict.
pub async fn revoke_catalog_entry(
    tx: &mut PgConnection,
    catalog_id: Uuid,
    reason: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<Vec<(Uuid, Uuid)>> {
    let updated = sqlx::query!(
        "UPDATE plugin_catalog SET revoked_at = $2, revoked_reason = $3, updated_at = now()
         WHERE id = $1 AND revoked_at IS NULL",
        catalog_id,
        now,
        reason,
    )
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        let exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM plugin_catalog WHERE id = $1")
                .bind(catalog_id)
                .fetch_optional(&mut *tx)
                .await?;
        return match exists {
            Some(_) => Err(DbError::Conflict("catalog entry is already revoked".into())),
            None => Err(DbError::NotFound),
        };
    }
    let installs = sqlx::query(
        "UPDATE plugin_installs SET status = 'revoked', revoked_at = $2,
                revoked_reason = $3, updated_at = now()
         WHERE catalog_id = $1 AND status <> 'revoked'
         RETURNING id, org_id",
    )
    .bind(catalog_id)
    .bind(now)
    .bind(reason)
    .fetch_all(&mut *tx)
    .await?;
    // Wave 43 lane 4 (A): a revoked install's outbound subscriptions must
    // not survive it. The FK's `ON DELETE CASCADE` covers UNINSTALL (the
    // row goes away) but NOT revoke — revocation is a status flip, so the
    // install row keeps existing and nothing cascades. Tear them down
    // here, in the same transaction as the status flip, so the two cannot
    // disagree.
    //
    // The org_id is bound explicitly from the RETURNING list rather than
    // inferred from the transaction's RLS scope. RLS narrows the UPDATE
    // above to the acting org in practice, but relying on that would make
    // this delete silently correct only while the policy holds; binding
    // the pair makes it exact either way.
    for install in &installs {
        let install_org: Uuid = install.get("org_id");
        let install_id: Uuid = install.get("id");
        crate::webhooks::remove_install_webhooks(&mut *tx, install_org, install_id).await?;
    }
    Ok(installs
        .iter()
        .map(|row| (row.get::<Uuid, _>("id"), row.get::<Uuid, _>("org_id")))
        .collect())
}

// --- versions ----------------------------------------------------------------

fn manifest_from_row(row: &sqlx::postgres::PgRow) -> DbResult<PluginVersion> {
    let manifest: serde_json::Value = row.get("manifest");
    Ok(PluginVersion {
        id: row.get("id"),
        catalog_id: row.get("catalog_id"),
        version: row.get("version"),
        manifest: serde_json::from_value(manifest)
            .map_err(|e| DbError::Conflict(format!("stored manifest does not parse: {e}")))?,
        module_digest: row.get("module_digest"),
        created_at: row.get("created_at"),
    })
}

const VERSION_COLS: &str = "id, catalog_id, version, manifest, module_digest, created_at";

pub async fn get_version(
    executor: &mut PgConnection,
    catalog_id: Uuid,
    version: &str,
) -> DbResult<PluginVersion> {
    let row = sqlx::query(&format!(
        "SELECT {VERSION_COLS} FROM plugin_versions WHERE catalog_id = $1 AND version = $2"
    ))
    .bind(catalog_id)
    .bind(version)
    .fetch_one(executor)
    .await?;
    manifest_from_row(&row)
}

/// Whether a signature the operator's trust set verified covers THIS version's
/// manifest (migration 0172), not just its artifact bytes.
///
/// Read on its own rather than carried on [`PluginVersion`] because the entity is
/// the declarative manifest and its pin, and the only caller that needs the
/// verdict is the cross-org adoption guard in core-api. `false` is the honest
/// default for every row the backfill could not prove: unknown is treated as
/// unauthenticated (fail closed), never the other way round.
pub async fn version_manifest_authenticated(
    executor: &mut PgConnection,
    catalog_id: Uuid,
    version: &str,
) -> DbResult<bool> {
    let authenticated: bool = sqlx::query_scalar(
        "SELECT manifest_authenticated FROM plugin_versions WHERE catalog_id = $1 AND version = $2",
    )
    .bind(catalog_id)
    .bind(version)
    .fetch_one(executor)
    .await?;
    Ok(authenticated)
}

pub async fn list_versions(
    executor: &mut PgConnection,
    catalog_id: Uuid,
) -> DbResult<Vec<PluginVersion>> {
    let rows = sqlx::query(&format!(
        "SELECT {VERSION_COLS} FROM plugin_versions WHERE catalog_id = $1 ORDER BY created_at, version"
    ))
    .bind(catalog_id)
    .fetch_all(executor)
    .await?;
    rows.iter().map(manifest_from_row).collect()
}

// --- installs ----------------------------------------------------------------

/// Inserts an org install. The route layer always creates DISABLED
/// (deny-by-default); the DB default backs it up.
///
/// Written with `sqlx::query()` rather than the `query!` macro: the column
/// list gained `module_digest` in migration 0156, and a `query!` whose text
/// changed needs a refreshed offline cache (`cargo sqlx prepare`). The crate
/// already carries both styles (`ai_policy.rs`, `plugin_health.rs`), and this
/// path is one INSERT on an admin action, so the macro buys nothing here.
pub async fn create_install(executor: &mut PgConnection, install: &PluginInstall) -> DbResult<()> {
    let granted = serde_json::to_value(&install.granted_capabilities)
        .map_err(|e| DbError::Conflict(format!("grants do not serialize: {e}")))?;
    sqlx::query(
        r#"INSERT INTO plugin_installs (id, org_id, catalog_id, version, module_digest, status,
                                        granted_capabilities, installed_by)
           VALUES ($1, $2, $3, $4, $5, $6::text::plugin_install_status, $7::jsonb, $8)"#,
    )
    .bind(install.id)
    .bind(install.org_id)
    .bind(install.catalog_id)
    .bind(&install.version)
    .bind(&install.module_digest)
    .bind(install.status.as_str())
    .bind(granted)
    .bind(install.installed_by)
    .execute(&mut *executor)
    .await?;
    Ok(())
}

pub async fn list_installs(
    executor: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<Vec<PluginInstall>> {
    let rows = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM plugin_installs i
         JOIN plugin_catalog c ON c.id = i.catalog_id
         WHERE i.org_id = $1 ORDER BY i.installed_at"
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    rows.iter().map(row_to_install).collect()
}

pub async fn get_install(
    executor: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<PluginInstall> {
    let row = sqlx::query(&format!(
        "SELECT {INSTALL_COLS} FROM plugin_installs i
         JOIN plugin_catalog c ON c.id = i.catalog_id
         WHERE i.org_id = $1 AND i.id = $2"
    ))
    .bind(org_id)
    .bind(install_id)
    .fetch_one(executor)
    .await?;
    row_to_install(&row)
}

/// Updates status / pinned version / grant set. `None` leaves a field
/// unchanged. Refuses to touch a revoked install (revocation is
/// terminal; re-install is the only path back). The row lock makes the
/// revoked check race-free.
pub async fn update_install(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    status: Option<PluginInstallStatus>,
    version: Option<&str>,
    granted_capabilities: Option<&[String]>,
) -> DbResult<()> {
    let current_status: String = sqlx::query_scalar(
        "SELECT status::text FROM plugin_installs WHERE org_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(DbError::NotFound)?;
    if current_status == "revoked" {
        return Err(DbError::Conflict(
            "a revoked install is terminal; re-install to adopt the entry again".into(),
        ));
    }
    // Wave 43 lane 4 (A): this function is the GUARD, never the transition.
    // `revoke_catalog_entry` is the only code in the repo that creates a
    // `revoked` status — and it is also the only place that tears down the
    // install's outbound webhooks beside the flip. Duplicating the teardown
    // here would give two sites to audit and a guaranteed future
    // divergence, so a caller passing `Revoked` fails loudly in dev
    // instead of shipping a half-teardown.
    debug_assert!(
        !matches!(status, Some(PluginInstallStatus::Revoked)),
        "a revoked status is created only by revoke_catalog_entry, which also tears down \
         the install's outbound webhooks; passing Revoked here would skip that teardown"
    );
    let granted = granted_capabilities
        .map(|caps| {
            serde_json::to_value(caps)
                .map_err(|e| DbError::Conflict(format!("grants do not serialize: {e}")))
        })
        .transpose()?;
    sqlx::query!(
        r#"UPDATE plugin_installs SET
             status = COALESCE($3::text::plugin_install_status, status),
             version = COALESCE($4, version),
             granted_capabilities = COALESCE($5::jsonb, granted_capabilities),
             updated_at = now()
           WHERE org_id = $1 AND id = $2"#,
        org_id,
        install_id,
        status.map(|s| s.as_str()),
        version,
        granted,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn delete_install(
    executor: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<()> {
    let result = sqlx::query!(
        "DELETE FROM plugin_installs WHERE org_id = $1 AND id = $2",
        org_id,
        install_id,
    )
    .execute(&mut *executor)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }
    Ok(())
}

// --- lifecycle: the digest pin, rollback and revoke --------------------------

/// Rolls the pin back to the target an update parked: version, digest and grant
/// set move together, and the target is cleared so a rollback is one step rather
/// than a toggle. A second rollback requires re-consenting the update, which is
/// the honest answer — the operator has to say which bytes they want, not
/// ping-pong between two.
///
/// No bytes move. The store keys modules by digest, so the old module is still
/// where the update left it.
pub async fn rollback_pin(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<(String, String)> {
    let row = sqlx::query(
        r#"UPDATE plugin_installs SET
             version = previous_version,
             module_digest = previous_module_digest,
             granted_capabilities = COALESCE(previous_capabilities, granted_capabilities),
             previous_version = NULL,
             previous_module_digest = NULL,
             previous_capabilities = NULL,
             updated_at = now()
           WHERE org_id = $1 AND id = $2
             AND status <> 'revoked'
             AND previous_version IS NOT NULL
           RETURNING version, module_digest"#,
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT status::text FROM plugin_installs WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(install_id)
        .fetch_optional(&mut *tx)
        .await?;
        return match exists.as_deref() {
            None => Err(DbError::NotFound),
            Some("revoked") => Err(DbError::Conflict(
                "a revoked install is terminal; a rollback cannot revive it".into(),
            )),
            Some(_) => Err(DbError::Conflict(
                "this install has no previous pin to roll back to".into(),
            )),
        };
    };
    Ok((row.get("version"), row.get("module_digest")))
}

/// Revokes ONE install, the org's own kill switch (the catalog-wide switch is
/// [`revoke_catalog_entry`]).
///
/// Revocation is terminal and it stops execution at the parent's gate: the
/// status is what [`plugin_host::authorize_invocation`] refuses on, and the
/// artifact load path refuses a non-enabled install before it reads a byte.
/// Nothing is deleted — the pin stays for forensics, and the bytes stay in the
/// content-addressed store, unreachable through this install.
///
/// The pending update columns clear in the same statement: a revoked install
/// with an awaiting-consent update would offer an operator a screen whose
/// commit is guaranteed to fail.
///
/// Outbound webhooks come down with it, for the reason
/// [`revoke_catalog_entry`] documents: revocation is a status flip, so the FK
/// cascade that covers uninstall never fires here.
pub async fn revoke_install(
    tx: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    revoked_by: Option<Uuid>,
    reason: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> DbResult<()> {
    if reason.trim().is_empty() {
        return Err(DbError::Conflict("a revocation reason is required".into()));
    }
    let row = sqlx::query(
        r#"UPDATE plugin_installs SET
             status = 'revoked',
             revoked_at = $3,
             revoked_by = $4,
             revoked_reason = $5,
             pending_version = NULL,
             pending_manifest = NULL,
             pending_digest = NULL,
             pending_capabilities = NULL,
             pending_consent_at = NULL,
             updated_at = now()
           WHERE org_id = $1 AND id = $2 AND status <> 'revoked'
           RETURNING id"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(now)
    .bind(revoked_by)
    .bind(reason)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT status::text FROM plugin_installs WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(install_id)
        .fetch_optional(&mut *tx)
        .await?;
        return match exists.as_deref() {
            None => Err(DbError::NotFound),
            Some(_) => Err(DbError::Conflict("the install is already revoked".into())),
        };
    };
    let revoked_id: Uuid = row.get("id");
    crate::webhooks::remove_install_webhooks(&mut *tx, org_id, revoked_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn grant_list_round_trips_through_jsonb_shape() {
        let caps = vec!["tickets.read".to_string(), "financial.read".to_string()];
        let value = serde_json::to_value(&caps).unwrap();
        assert!(value.is_array());
    }
}
