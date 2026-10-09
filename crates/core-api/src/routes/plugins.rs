//! Plugin runtime v2 + marketplace registry (Wave 14 lane 5): the
//! catalog, install, and revocation surfaces.
//!
//! Purpose: org adoption of marketplace catalog entries under the v2
//! declarative manifest/permissions model. Install = fork-like
//! ADOPTION of a catalog entry at a PINNED version — no plugin code is
//! ever uploaded, inlined, or injected through this surface; the
//! manifest is data and the capability grants resolve onto the
//! EXISTING permission matrix (no parallel auth system).
//!
//! Posture (mirrors the federation bundle interface):
//! * deny-by-default — installs are created DISABLED; only an explicit
//!   admin action enables; revocation (registry-level or org-level) is
//!   terminal;
//! * admin-enabled — every write here is `special:manage_plugins` +
//!   owner/admin; catalog reads are `plugins:view`;
//! * versioned interface — manifests carry `interface_version` and the
//!   runtime refuses mismatches (`PLUGIN_RUNTIME_INTERFACE_VERSION`);
//! * audit — every lifecycle step writes an append-only audit event;
//! * contract events — `plugin.*` events ride the transactional outbox
//!   in the same transaction as the state change.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{Extensions, StatusCode},
    Json,
};
use core_domain::entities::{
    PluginCapability, PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2,
    PluginVersion, PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_domain::events::{
    PluginCatalogEntryCreated, PluginInstallRevoked, PluginInstallUpdated, PluginInstalled,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Plugins-module permission shorthand.
fn plugins(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Plugins,
        action,
    }
}

/// Every mutating surface here is an explicit admin action.
fn require_plugin_admin(authed: &crate::auth::AuthedUser) -> ApiResult<()> {
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "plugin management is limited to org admins".into(),
        ));
    }
    Ok(())
}

// --- request/response bodies -------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PublishCatalogEntryRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub publisher: String,
    pub version: String,
    pub manifest: PluginManifestBody,
}

/// The declarative manifest as submitted. `interface_version` is
/// server-pinned (the runtime speaks exactly one version), so a caller
/// cannot publish an entry the runtime would refuse.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PluginManifestBody {
    #[serde(default)]
    pub description: String,
    pub capabilities: Vec<String>,
    /// ADR 0009 §4 — optional JSON-Schema for manifest-rendered settings.
    #[serde(default)]
    pub settings_schema: Option<serde_json::Value>,
    /// ADR 0009 §6 full semantics (Wave 19 lane 2) — the modeled
    /// connector governance profile (webhook endpoints + scopes).
    /// Valid only on http_connector manifests; wasm manifests refuse it.
    #[serde(default)]
    pub connector_profile: Option<core_domain::entities::ConnectorProfile>,
    /// §4.6 (Wave 34 lane 4, amended per kind by ADR 0017 §3 in Wave 43
    /// lane 4) — the declarative surface profile. On a wasm manifest every
    /// section is available; on an http_connector the three UI sections
    /// refuse and the three executor sections
    /// (automation_action / automation_trigger / read) are allowed.
    #[serde(default)]
    pub surface_profile: Option<core_domain::entities::PluginSurfaceProfile>,
    /// ADR 0009 §6 — which runtime the published artifact speaks. Absent
    /// ⇒ `wasm`, byte-identical to every request body published before
    /// Wave 43 lane 4. Without this field the route FORCED wasm: a
    /// connector's `connector_profile` is refused on a wasm manifest, so
    /// `POST /api/plugins/catalog` could never publish the artifact kind
    /// Wave 43 lane 4's outbound seeding actually needs. `validate()` is
    /// called on the built manifest, so an incoherent combination is
    /// still rejected here rather than at install time.
    #[serde(default)]
    pub artifact_kind: Option<core_domain::entities::PluginRuntimeKind>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct AppendVersionRequest {
    pub version: String,
    pub manifest: PluginManifestBody,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokeCatalogEntryRequest {
    pub reason: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InstallPluginRequest {
    pub slug: String,
    /// Pin the adopted version; defaults to the entry's latest.
    pub version: Option<String>,
    /// Capability keys granted at install — a subset of the pinned
    /// manifest's declared set. Unknown keys are rejected (closed
    /// vocabulary).
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateInstallRequest {
    /// `enabled` | `disabled` (the admin-enabled gate). `revoked` is
    /// NOT settable here — revocation rides the dedicated revoke
    /// surface so the audit trail names it.
    pub status: Option<String>,
    /// Upgrade the pinned version (explicit, never automatic).
    pub version: Option<String>,
    pub granted_capabilities: Option<Vec<String>>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CatalogEntryView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub latest_version: Option<String>,
    pub interface_version: u32,
    pub revoked: bool,
    pub revoked_reason: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<PluginCatalogEntry> for CatalogEntryView {
    fn from(e: PluginCatalogEntry) -> Self {
        Self {
            id: e.id,
            slug: e.slug,
            name: e.name,
            description: e.description,
            publisher: e.publisher,
            latest_version: e.latest_version,
            interface_version: e.interface_version,
            revoked: e.revoked_at.is_some(),
            revoked_reason: e.revoked_reason,
            created_at: e.created_at,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InstallView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub status: PluginInstallStatus,
    pub granted_capabilities: Vec<String>,
    /// The governance profile from the PINNED manifest (ADR 0009 §6):
    /// `wasm` | `http_connector` — the roster-tab discriminator.
    pub artifact_kind: String,
    /// The effective permission keys the grants resolve onto (the
    /// EXISTING matrix vocabulary — `tickets:read`, `financial:read`,
    /// …). Informational; enforcement re-derives from the same matrix.
    pub effective_permissions: Vec<String>,
    #[schema(value_type = String, format = "date-time")]
    pub installed_at: chrono::DateTime<chrono::Utc>,
}

impl InstallView {
    fn from_install(install: PluginInstall, name: String, manifest: &PluginManifestV2) -> Self {
        let effective_permissions = install
            .granted_permissions()
            .iter()
            .map(|p| p.key())
            .collect();
        Self {
            id: install.id,
            slug: install.slug,
            name,
            version: install.version,
            status: install.status,
            granted_capabilities: install.granted_capabilities,
            artifact_kind: manifest.runtime_kind().as_str().to_string(),
            effective_permissions,
            installed_at: install.installed_at,
        }
    }
}

// --- validation helpers ------------------------------------------------------

fn parse_capability_keys(keys: &[String]) -> ApiResult<Vec<PluginCapability>> {
    keys.iter()
        .map(|k| {
            PluginCapability::parse(k).ok_or_else(|| {
                ApiError::Validation(format!(
                    "unknown capability {k:?} — the capability vocabulary is closed"
                ))
            })
        })
        .collect()
}

/// Grants must be a subset of the pinned manifest's declared set — a
/// grant outside the manifest cannot exist.
fn validate_grants_against_manifest(
    manifest: &PluginManifestV2,
    requested: &[PluginCapability],
) -> ApiResult<()> {
    let declared: Vec<&str> = manifest.capabilities.iter().map(String::as_str).collect();
    for cap in requested {
        if !declared.contains(&cap.as_str()) {
            return Err(ApiError::Validation(format!(
                "capability {:?} is not declared by manifest version {}",
                cap.as_str(),
                manifest.version
            )));
        }
    }
    Ok(())
}

fn build_manifest(
    name: &str,
    version: &str,
    body: &PluginManifestBody,
) -> ApiResult<PluginManifestV2> {
    let manifest = PluginManifestV2 {
        name: name.trim().to_string(),
        version: version.trim().to_string(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: body.description.clone(),
        capabilities: body.capabilities.clone(),
        settings_schema: body.settings_schema.clone(),
        artifact_kind: body.artifact_kind,
        connector_profile: body.connector_profile.clone(),
        surface_profile: body.surface_profile.clone(),
    };
    manifest.validate().map_err(ApiError::from)?;
    Ok(manifest)
}

// --- catalog surfaces --------------------------------------------------------

/// GET /api/plugins/catalog — the marketplace registry.
#[utoipa::path(operation_id = "plugin_catalog_list", get, path = "/api/plugins/catalog", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = Vec<CatalogEntryView>)))]
pub async fn list_catalog(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<CatalogEntryView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let entries = core_db::plugins::list_catalog(&mut tx).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entries.into_iter().map(Into::into).collect()))
}

/// GET /api/plugins/catalog/{slug}.
#[utoipa::path(operation_id = "plugin_catalog_get", get, path = "/api/plugins/catalog/{slug}", tag = "plugins", security(("bearer" = [])),
    params(("slug" = String, Path)),
    responses((status = 200, body = CatalogEntryView), (status = 404)))]
pub async fn get_catalog_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(slug): Path<String>,
) -> ApiResult<Json<CatalogEntryView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let entry = core_db::plugins::get_catalog_entry(&mut tx, &slug).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entry.into()))
}

/// POST /api/plugins/catalog — publish a catalog entry with its first
/// version. Operator action on a self-hosted core (the catalog is
/// global); audited, and the manifest is validated against the closed
/// capability vocabulary and the pinned interface version BEFORE any
/// row exists.
#[utoipa::path(operation_id = "plugin_catalog_publish", post, path = "/api/plugins/catalog", tag = "plugins", security(("bearer" = [])),
    request_body = PublishCatalogEntryRequest,
    responses((status = 201, body = CatalogEntryView), (status = 409)))]
pub async fn publish_catalog_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PublishCatalogEntryRequest>,
) -> ApiResult<(StatusCode, Json<CatalogEntryView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let slug = req.slug.trim().to_lowercase();
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(ApiError::Validation(
            "slug must be lowercase letters, digits, '-' or '_'".into(),
        ));
    }
    let manifest = build_manifest(&req.name, &req.version, &req.manifest)?;
    parse_capability_keys(&manifest.capabilities)?;
    let now = chrono::Utc::now();
    let entry = PluginCatalogEntry {
        id: Uuid::now_v7(),
        slug: slug.clone(),
        name: manifest.name.clone(),
        description: req.description.trim().to_string(),
        publisher: req.publisher.trim().to_string(),
        latest_version: Some(manifest.version.clone()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: now,
        updated_at: now,
    };
    let version = PluginVersion {
        id: Uuid::now_v7(),
        catalog_id: entry.id,
        version: manifest.version.clone(),
        manifest: manifest.clone(),
        created_at: now,
    };
    let entry_for_tx = entry.clone();
    org_tx::with_org_tx(
        &state,
        authed.principal.user.org_id,
        actor,
        |mut tx| async move {
            core_db::plugins::create_catalog_entry(&mut tx.tx, &entry_for_tx, &version).await?;
            core_db::audit::insert(
                &mut *tx.tx,
                &core_domain::entities::AuditEvent {
                    id: Uuid::now_v7(),
                    org_id: authed.principal.user.org_id,
                    actor: core_domain::entities::Actor::Human(actor),
                    action: "plugin.catalog_entry_created".into(),
                    entity_type: Some("plugin_catalog_entry".into()),
                    entity_id: Some(entry_for_tx.id.to_string()),
                    detail: serde_json::json!({
                        "slug": slug,
                        "version": version.version,
                        "capabilities": manifest_capabilities(&version.manifest),
                    }),
                    ip: None,
                    occurred_at: now,
                },
            )
            .await?;
            tx.emit(
                core_domain::events::types::PLUGIN_CATALOG_ENTRY_CREATED,
                "plugin_catalog_entry",
                Some(entry_for_tx.id.to_string().as_str()),
                &serde_json::to_value(PluginCatalogEntryCreated {
                    catalog_id: entry_for_tx.id,
                    slug: slug.clone(),
                    name: entry_for_tx.name.clone(),
                    publisher: entry_for_tx.publisher.clone(),
                    version: version.version.clone(),
                    interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION as i64,
                    capabilities: version.manifest.capabilities.clone(),
                    created_at: now,
                })
                .unwrap_or(serde_json::Value::Null),
            )
            .await?;
            Ok((tx, ()))
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}

fn manifest_capabilities(manifest: &PluginManifestV2) -> Vec<String> {
    manifest.capabilities.clone()
}

/// POST /api/plugins/catalog/{slug}/versions — append a version to a
/// live entry (revoked entries accept no new versions).
#[utoipa::path(operation_id = "plugin_catalog_append_version", post, path = "/api/plugins/catalog/{slug}/versions", tag = "plugins", security(("bearer" = [])),
    params(("slug" = String, Path)), request_body = AppendVersionRequest,
    responses((status = 201, body = CatalogEntryView), (status = 404), (status = 409)))]
pub async fn append_catalog_version(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(slug): Path<String>,
    Json(req): Json<AppendVersionRequest>,
) -> ApiResult<(StatusCode, Json<CatalogEntryView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let entry = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let entry = core_db::plugins::get_catalog_entry(&mut tx.tx, &slug).await?;
        // The manifest carries the entry's name; validate after fetch.
        let manifest = build_manifest(&entry.name, &req.version, &req.manifest)?;
        parse_capability_keys(&manifest.capabilities)?;
        let version = PluginVersion {
            id: Uuid::now_v7(),
            catalog_id: entry.id,
            version: manifest.version.clone(),
            manifest,
            created_at: now,
        };
        core_db::plugins::append_version(&mut tx.tx, entry.id, &version).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.catalog_version_added".into(),
                entity_type: Some("plugin_catalog_entry".into()),
                entity_id: Some(entry.id.to_string()),
                detail: serde_json::json!({
                    "slug": slug,
                    "version": version.version,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        Ok((tx, entry))
    })
    .await?;
    Ok((StatusCode::CREATED, Json(entry.into())))
}

/// POST /api/plugins/catalog/{slug}/revoke — the registry kill switch.
/// Denies new installs and marks every existing install revoked
/// (deny-by-default: a revoked plugin never runs); one
/// `plugin.install_revoked` event per affected install.
#[utoipa::path(operation_id = "plugin_catalog_revoke", post, path = "/api/plugins/catalog/{slug}/revoke", tag = "plugins", security(("bearer" = [])),
    params(("slug" = String, Path)), request_body = RevokeCatalogEntryRequest,
    responses((status = 200, body = CatalogEntryView), (status = 404), (status = 409)))]
pub async fn revoke_catalog_entry(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(slug): Path<String>,
    Json(req): Json<RevokeCatalogEntryRequest>,
) -> ApiResult<Json<CatalogEntryView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err(ApiError::Validation(
            "a revocation reason is required".into(),
        ));
    }
    let now = chrono::Utc::now();
    let entry = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let entry = core_db::plugins::get_catalog_entry(&mut tx.tx, &slug).await?;
        let affected =
            core_db::plugins::revoke_catalog_entry(&mut tx.tx, entry.id, &reason, now).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.catalog_entry_revoked".into(),
                entity_type: Some("plugin_catalog_entry".into()),
                entity_id: Some(entry.id.to_string()),
                detail: serde_json::json!({
                    "slug": slug,
                    "reason": reason,
                    "installs_revoked": affected.len(),
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        for (install_id, _install_org) in &affected {
            tx.emit(
                core_domain::events::types::PLUGIN_INSTALL_REVOKED,
                "plugin_install",
                Some(install_id.to_string().as_str()),
                &serde_json::to_value(PluginInstallRevoked {
                    install_id: *install_id,
                    catalog_id: entry.id,
                    slug: slug.clone(),
                    reason: Some(reason.clone()),
                    revoked_at: now,
                })
                .unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        // Re-read so the response reflects the revoked state.
        let entry = core_db::plugins::get_catalog_entry(&mut tx.tx, &slug).await?;
        Ok((tx, entry))
    })
    .await?;
    Ok(Json(entry.into()))
}

// --- install surfaces --------------------------------------------------------

/// GET /api/plugins/installed — this org's adoptions.
#[utoipa::path(operation_id = "plugin_install_list", get, path = "/api/plugins/installed", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = Vec<InstallView>)))]
pub async fn list_installs(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<InstallView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let installs = core_db::plugins::list_installs(&mut tx, org_id).await?;
    let mut views = Vec::with_capacity(installs.len());
    for install in installs {
        let name = core_db::plugins::get_catalog_entry(&mut tx, &install.slug)
            .await
            .map(|e| e.name)
            .unwrap_or_else(|_| install.slug.clone());
        let manifest = core_db::plugins::get_version(&mut tx, install.catalog_id, &install.version)
            .await
            .map(|v| v.manifest)
            .unwrap_or_else(|_| fallback_manifest(&install));
        views.push(InstallView::from_install(install, name, &manifest));
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(views))
}

/// Degraded manifest for an orphaned pin (the catalog entry vanished):
/// the roster still renders the install (the tab split degrades to the
/// wasm baseline) rather than failing the whole roster read.
fn fallback_manifest(install: &PluginInstall) -> PluginManifestV2 {
    PluginManifestV2 {
        name: install.slug.clone(),
        version: install.version.clone(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: String::new(),
        capabilities: install.granted_capabilities.clone(),
        settings_schema: None,
        artifact_kind: None,
        connector_profile: None,
        surface_profile: None,
    }
}

/// POST /api/plugins/installed — adopt a catalog entry at a pinned
/// version. Created DISABLED (deny-by-default); enabling is a separate
/// explicit admin action.
#[utoipa::path(operation_id = "plugin_install_create", post, path = "/api/plugins/installed", tag = "plugins", security(("bearer" = [])),
    request_body = InstallPluginRequest,
    responses((status = 201, body = InstallView), (status = 404), (status = 409)))]
pub async fn create_install(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<InstallPluginRequest>,
) -> ApiResult<(StatusCode, Json<InstallView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let requested = parse_capability_keys(&req.granted_capabilities)?;
    let now = chrono::Utc::now();
    // The at-rest key is Copy, so it is bound out here rather than reached
    // through `&state` inside the closure below (which `with_org_tx` has
    // already borrowed). Wave 43 lane 4 (A) needs it to seal a seeded
    // connector's signing secret.
    let at_rest_key = state.config.secret_key;
    let view = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let entry = core_db::plugins::get_catalog_entry(&mut tx.tx, req.slug.trim()).await?;
        if entry.revoked_at.is_some() {
            return Err(ApiError::Conflict(
                "this catalog entry is revoked; new installs are denied".into(),
            ));
        }
        if entry.interface_version != PLUGIN_RUNTIME_INTERFACE_VERSION {
            return Err(ApiError::Conflict(
                "catalog entry speaks a different plugin interface version".into(),
            ));
        }
        let version = match &req.version {
            Some(v) => v.trim().to_string(),
            None => entry.latest_version.clone().ok_or_else(|| {
                ApiError::Conflict("catalog entry has no published version".into())
            })?,
        };
        let pinned = core_db::plugins::get_version(&mut tx.tx, entry.id, &version).await?;
        validate_grants_against_manifest(&pinned.manifest, &requested)?;
        let install = PluginInstall {
            id: Uuid::now_v7(),
            org_id,
            catalog_id: entry.id,
            slug: entry.slug.clone(),
            version: version.clone(),
            status: PluginInstallStatus::Disabled,
            granted_capabilities: req.granted_capabilities.clone(),
            installed_by: Some(actor),
            installed_at: now,
            updated_at: now,
            revoked_at: None,
            revoked_by: None,
        };
        core_db::plugins::create_install(&mut tx.tx, &install).await?;
        // §4.6: derive + persist the surface registrations the pinned
        // manifest declares under the granted capabilities (data rows
        // only; they render nowhere until the install enables).
        plugin_host::surface::sync_install_surface(&mut tx.tx, &install, &pinned.manifest)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
        // Wave 43 lane 4 (A): make `connector_profile.webhook_endpoints`
        // real. The consent screen has always promised that "declared
        // webhook endpoints seed the connector's outbound posture"; this is
        // where that becomes true. Seeding needs a SEALED signing secret,
        // which only core-api can produce (the at-rest key lives in
        // AppState.config) — that is why the seed lives here and not in
        // core-db or plugin-host.
        //
        // The fresh install is created DISABLED, and `WebhookRow.enabled`
        // defaults to true, so the rows are inert through the install
        // lifecycle until an admin enables the connector: the delivery
        // match (`webhooks::find_enabled_for_event`) requires
        // `i.status = 'enabled' AND i.revoked_at IS NULL` for an
        // install-attributed row. Human-created subscriptions are
        // unaffected — they carry no owner.
        let seeded = pinned
            .manifest
            .connector_profile
            .as_ref()
            .map(|p| p.webhook_endpoints.clone())
            .unwrap_or_default();
        if !seeded.is_empty() {
            let raw_secret = format!("whsec_{}", uuid::Uuid::now_v7().simple());
            let sealed = crate::crypto::seal(&at_rest_key, raw_secret.as_bytes())
                .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
            core_db::webhooks::seed_install_webhooks(
                &mut *tx.tx,
                org_id,
                install.id,
                &seeded,
                &sealed,
                actor,
            )
            .await?;
        }
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.installed".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(install.id.to_string()),
                detail: serde_json::json!({
                    "slug": entry.slug,
                    "version": version,
                    "granted_capabilities": req.granted_capabilities,
                    "status": "disabled",
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            core_domain::events::types::PLUGIN_INSTALLED,
            "plugin_install",
            Some(install.id.to_string().as_str()),
            &serde_json::to_value(PluginInstalled {
                install_id: install.id,
                catalog_id: entry.id,
                slug: entry.slug.clone(),
                version,
                granted_capabilities: req.granted_capabilities.clone(),
                installed_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((
            tx,
            InstallView::from_install(install, entry.name, &pinned.manifest),
        ))
    })
    .await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// PATCH /api/plugins/installed/{id} — enable/disable, upgrade the
/// pinned version, or adjust the grant set. Widening grants
/// re-validates against the NEW pinned manifest. Revoked installs are
/// terminal.
#[utoipa::path(operation_id = "plugin_install_update", patch, path = "/api/plugins/installed/{id}", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = UpdateInstallRequest,
    responses((status = 200, body = InstallView), (status = 404), (status = 409)))]
pub async fn update_install(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateInstallRequest>,
) -> ApiResult<Json<InstallView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let status = match &req.status {
        Some(s) => match s.as_str() {
            "enabled" => Some(PluginInstallStatus::Enabled),
            "disabled" => Some(PluginInstallStatus::Disabled),
            other => {
                return Err(ApiError::Validation(format!(
                    "status {other:?} is not settable here; revocation rides the revoke surface"
                )))
            }
        },
        None => None,
    };
    let requested_caps = match &req.granted_capabilities {
        Some(keys) => Some(parse_capability_keys(keys)?),
        None => None,
    };
    let now = chrono::Utc::now();
    let view = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        // Grant/version changes validate against the TARGET manifest:
        // the new version when upgrading, else the current pinned one.
        let target_version = req
            .version
            .clone()
            .unwrap_or_else(|| existing.version.clone());
        let target_manifest =
            core_db::plugins::get_version(&mut tx.tx, existing.catalog_id, &target_version)
                .await?
                .manifest;
        if let Some(caps) = &requested_caps {
            validate_grants_against_manifest(&target_manifest, caps)?;
        }
        core_db::plugins::update_install(
            &mut tx.tx,
            org_id,
            id,
            status,
            req.version.as_deref(),
            req.granted_capabilities.as_deref(),
        )
        .await?;
        let updated = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        // §4.6: grant/version changes rewrite the surface registrations
        // to exactly what the new (manifest, grants) pair declares — the
        // deny-by-default residue is deletion, not retention.
        plugin_host::surface::sync_install_surface(&mut tx.tx, &updated, &target_manifest)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.install_updated".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": updated.slug,
                    "version": updated.version,
                    "status": updated.status.as_str(),
                    "granted_capabilities": updated.granted_capabilities,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            core_domain::events::types::PLUGIN_INSTALL_UPDATED,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(PluginInstallUpdated {
                install_id: id,
                catalog_id: updated.catalog_id,
                slug: updated.slug.clone(),
                version: updated.version.clone(),
                status: updated.status.as_str().into(),
                granted_capabilities: updated.granted_capabilities.clone(),
                occurred_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let name = core_db::plugins::get_catalog_entry(&mut tx.tx, &updated.slug)
            .await
            .map(|e| e.name)
            .unwrap_or_else(|_| updated.slug.clone());
        Ok((
            tx,
            InstallView::from_install(updated, name, &target_manifest),
        ))
    })
    .await?;
    Ok(Json(view))
}

/// DELETE /api/plugins/installed/{id} — uninstall (the adoption row
/// goes away; re-install is a fresh adoption).
#[utoipa::path(operation_id = "plugin_install_delete", delete, path = "/api/plugins/installed/{id}", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 204), (status = 404)))]
pub async fn delete_install(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        core_db::plugins::delete_install(&mut tx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.uninstalled".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": existing.slug,
                    "version": existing.version,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::policy::{check, Action, Module, Permission};

    #[test]
    fn capability_grants_should_authorize_through_the_standard_policy_check() {
        // The whole point: a plugin's grant resolves to a matrix
        // permission and `policy::check` decides — no parallel auth.
        let granted = vec![PluginCapability::TicketsRead.permission()];
        assert!(check(
            &granted,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::View
            }
        )
        .is_allowed());
        assert!(!check(
            &granted,
            Permission::Matrix {
                module: Module::Tickets,
                action: Action::Edit
            }
        )
        .is_allowed());
    }

    #[test]
    fn admin_specials_should_never_be_grantable() {
        // The closed vocabulary has no key that resolves to a special.
        for cap in PluginCapability::ALL {
            assert!(!matches!(
                cap.permission(),
                Permission::ManagePlugins
                    | Permission::ManageUsers
                    | Permission::ManageRoles
                    | Permission::ManageBilling
                    | Permission::ManageApiKeys
                    | Permission::ViewAuditLog
                    | Permission::PortalAdmin
            ));
        }
    }
}
