//! The plugin & skill DISTRIBUTION surfaces (Wave 16 lane 7, ADR 0009):
//! the source adapters, the staged-import pipeline, the update
//! re-consent flow, the per-install settings surface and the provenance
//! chain reads. EXTENDS the Wave 14 seam (routes/plugins.rs) — never
//! forks it: every lifecycle write lands in the SAME deny-by-default
//! machinery (installs created DISABLED, grants subset the manifest's
//! declared set, unknown capabilities refused).
//!
//! Posture (mirrors the seam):
//! * every lifecycle write is an explicit owner/admin action
//!   (`special:manage_plugins` + owner/admin), EXCEPT staging: any org
//!   member may upload, and pending rows are visible only to their
//!   uploader and admins;
//! * staged imports are validated by the plugin-host SOURCE ADAPTERS
//!   BEFORE any row exists, never executed, and minted only at approval
//!   (approval still creates the adopt DISABLED — staging is the row
//!   before install, not a bypass);
//! * GitHub pulls are tag/commit-pinned, NEVER HEAD — unpinned pulls are
//!   refused with the typed `distribution_source_not_pinned` reason;
//! * every lifecycle step appends the same provenance chain
//!   `{source_kind, source_url, fetched_ref, artifact_digest, uploader}`
//!   (append-only; rows survive terminal uninstalls by design) and
//!   writes an append-only audit event;
//! * per-install settings render FROM the manifest's declared
//!   JSON-Schema (typed-registry doctrine; no hand-authored pages) and
//!   every stored config value is validated against that schema at the
//!   write boundary;
//! * an update proposal pins the OLD version (still running) until the
//!   owner/admin consents; the consent IS the swap transaction;
//! * skills reuse ONLY the skills half (0040 versioned rows:
//!   import/edit/enable/version/rollback — no executable lifecycle);
//!   skill content is untrusted RAG-class data per
//!   docs/10-agent-platform.md §4: persisted verbatim, never executed
//!   as instructions.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{Extensions, StatusCode},
    Json,
};
use core_domain::entities::agent_skills::AgentSkillVersion;
use core_domain::entities::{
    ConsentDiff, PluginArtifactKind, PluginCapability, PluginCatalogEntry, PluginInstall,
    PluginInstallStatus, PluginManifestV2, PluginProvenanceEntry, PluginPublisherVerification,
    PluginRating, PluginSourceKind, PluginStagedItem, PluginStagedStatus, PluginVersion,
    PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use core_domain::events::{
    types, PluginPublisherRevoked, PluginPublisherVerified, PluginRatingRecorded,
    PluginSettingsUpdated, PluginStagedImportApproved, PluginStagedImportCreated,
    PluginStagedImportRefused, PluginUpdatePending,
};
use core_domain::policy::{Action, Module, Permission};
use plugin_host::distribution::{
    ingest_github_pull, ingest_local_zip, ingest_registry_entry, ingest_skill_manifest,
    MAX_ARCHIVE_BYTES,
};
use plugin_host::lifecycle::{ArtifactError, LifecycleAction, LifecycleRefusal, LifecycleState};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::routes::plugins::{fallback_manifest, InstallView};

fn plugins(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Plugins,
        action,
    }
}

/// Every lifecycle-touching surface here is an explicit owner/admin
/// action — mirrors `require_plugin_admin` in routes/plugins.rs.
fn require_plugin_admin(authed: &crate::auth::AuthedUser) -> ApiResult<()> {
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "plugin management is limited to org admins".into(),
        ));
    }
    Ok(())
}

fn ingest_reason_api(reason: plugin_host::distribution::IngestReason) -> ApiError {
    ApiError::Validation(format!("distribution intake refused: {reason}"))
}

/// The artifact store's refusals, as an API answer. The reason code rides in
/// the message so a client sees a stable token before the prose.
fn artifact_api(error: ArtifactError) -> ApiError {
    ApiError::Conflict(format!(
        "plugin artifact refused ({}): {error}",
        error.as_str()
    ))
}

/// A lifecycle refusal, as an API answer. `409` for a state conflict (revoked,
/// already installed, nothing to roll back), `422` for an incoherent request.
fn lifecycle_api(error: LifecycleRefusal) -> ApiError {
    match &error {
        LifecycleRefusal::Invalid(_) => {
            ApiError::Validation(format!("lifecycle refused ({}): {error:?}", error.as_str()))
        }
        _ => ApiError::Conflict(format!("lifecycle refused ({}): {error:?}", error.as_str())),
    }
}

/// The LIVE signing-key trust set (Wave 19 lane 2 rotation semantics):
/// the DB registry (`plugin_signing_keys` — active + retired rows) is
/// the source of truth; the static env bootstrap set is the fallback
/// while the registry is empty (the operator runbook migrates env keys
/// INTO the registry, then the env set goes away). Verified-by-named-key_id
/// and by-trust-set semantics live in the distribution adapters.
async fn trusted_signing_keys(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
) -> Vec<plugin_host::distribution::TrustedSigningKey> {
    let from_registry = core_db::plugin_signing_keys::trusted_verification_keys(tx)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|k| plugin_host::distribution::TrustedSigningKey {
            key_id: k.key_id,
            public_key: k.public_key,
        })
        .collect::<Vec<_>>();
    if !from_registry.is_empty() {
        return from_registry;
    }
    crate::config::bootstrap_signing_keys(&state.config)
}

fn capability_or_refuse(key: &str) -> ApiResult<PluginCapability> {
    PluginCapability::parse(key).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown capability {key:?} — the vocabulary is closed"
        ))
    })
}

// --- request/response bodies ---------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StageFromRegistryRequest {
    pub url: String,
    /// Optional direct artifact URL (marketplace CDN / release asset).
    /// When present the server RE-FETCHES the bytes and verifies their
    /// sha256 against the catalog-pinned digest BEFORE the staged row
    /// exists (B2 cross-digest verification; typed
    /// `distribution_digest_invalid` on mismatch).
    #[serde(default)]
    pub artifact_url: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StageFromGithubRequest {
    pub owner: String,
    pub repo: String,
    /// Pinned git tag — HEAD is refused, never accepted.
    pub tag: String,
    /// Pinned 40-hex commit the tag resolves to.
    pub commit: String,
    /// Optional release-asset URL for B2 cross-digest verification of
    /// the actual artifact bytes (same posture as the registry source).
    #[serde(default)]
    pub artifact_url: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct StageSkillRequest {
    /// The inline manifest for `manual_upload` (community origins are
    /// re-fetched server-side instead).
    pub manifest: Option<serde_json::Value>,
    /// `skill_community` | `manual_upload`.
    pub source_kind: String,
    pub source_url: String,
    /// The pinned reference (a version tag / commit for community pulls).
    pub fetched_ref: Option<String>,
    /// The claimed sha256 for community pulls — reverified against the
    /// re-fetched bytes before the row exists.
    pub artifact_digest: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct StagedItemView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub artifact_kind: String,
    pub source_kind: String,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    /// The digest of the executable module this import carries, when it carries
    /// one. This is the value the install will pin and the executor will load;
    /// the consent screen shows it so an operator approves bytes, not a name.
    pub module_digest: String,
    pub manifest: serde_json::Value,
    pub declared_capabilities: Vec<String>,
    pub status: String,
    #[schema(value_type = uuid::Uuid)]
    pub uploaded_by: Uuid,
    pub review_note: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub adopted_install_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub adopted_skill_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub adopted_theme_pack_id: Option<Uuid>,
}

impl From<PluginStagedItem> for StagedItemView {
    fn from(i: PluginStagedItem) -> Self {
        Self {
            id: i.id,
            artifact_kind: i.artifact_kind.as_str().into(),
            source_kind: i.source_kind.as_str().into(),
            source_url: i.source_url,
            fetched_ref: i.fetched_ref,
            artifact_digest: i.artifact_digest,
            module_digest: i.module_digest,
            manifest: i.manifest,
            declared_capabilities: i.declared_capabilities,
            status: i.status.as_str().into(),
            uploaded_by: i.uploaded_by,
            review_note: i.review_note,
            adopted_install_id: i.adopted_install_id,
            adopted_skill_id: i.adopted_skill_id,
            adopted_theme_pack_id: i.adopted_theme_pack_id,
        }
    }
}

/// The consent screen payload — ONE component shared by every source
/// (same shape the update re-consent flow shows).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConsentView {
    pub artifact_kind: String,
    pub source_kind: String,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    pub declared_capabilities: Vec<String>,
    pub settings_schema: Option<serde_json::Value>,
    /// ADR 0009 §6 full semantics — the modeled connector governance
    /// profile (webhook endpoints + scopes) when the manifest declared
    /// one, for verbatim consent rendering.
    pub connector_profile: Option<serde_json::Value>,
    /// §4.6 (Wave 34 lane 4) — the declarative UI-surface profile
    /// (widgets / automation action kinds / report render kinds /
    /// field validators), rendered on consent verbatim so an admin
    /// consents to exactly the surfaces the plugin contributes.
    pub surface_profile: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ApproveStagedRequest {
    /// Capability keys granted at adopt time (a subset of the staged
    /// manifest's declared set; unknown keys are refused).
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
    #[serde(default)]
    pub review_note: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RefuseStagedRequest {
    pub review_note: String,
}

// --- Wave 24: installed org theme packs --------------------------------------

/// One installed theme pack for the org. The `manifest` carries the FULL
/// validated manifest snapshot (token overrides only); the ACTIVE pack
/// is identified through the typed `appearance.theme` setting value
/// `pack:<slug>`, not this table.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ThemePackView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub schema_version: i32,
    pub manifest: serde_json::Value,
    pub source_kind: String,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: Option<String>,
    /// Present when this pack is the org's active theme (i.e. the
    /// typed `appearance.theme` setting names `pack:<slug>`).
    pub active: bool,
}

impl ThemePackView {
    fn from_pack(pack: core_db::theme_packs::ThemePack, active_theme: Option<String>) -> Self {
        let active = active_theme
            .as_deref()
            .and_then(|a| a.strip_prefix("pack:"))
            .map(|a| a == pack.slug)
            .unwrap_or(false)
            || active_theme
                .as_deref()
                .map(|a| a == pack.slug)
                .unwrap_or(false);
        Self {
            id: pack.id,
            slug: pack.slug,
            name: pack.name,
            version: pack.version,
            description: pack.description,
            schema_version: pack.schema_version,
            manifest: pack.manifest,
            source_kind: pack.provenance_source_kind,
            source_url: pack.provenance_source_url,
            fetched_ref: pack.provenance_fetched_ref,
            artifact_digest: pack.provenance_digest,
            active,
        }
    }
}

/// GET /api/plugins/distribution/theme-packs — the org's installed
/// theme packs (ADR 0009 V1 distribution; restyle-only manifests).
#[utoipa::path(operation_id = "theme_packs_list", get, path = "/api/plugins/distribution/theme-packs", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = ThemePackList)))]
pub async fn list_theme_packs(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ThemePackList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut read_tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut read_tx, org_id).await?;
    let packs = core_db::theme_packs::list_packs(&mut read_tx, org_id).await?;
    let active_theme = core_db::settings::list_org_overrides(&mut *read_tx, org_id)
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .find(|o| o.key == core_domain::settings::APPEARANCE_THEME)
                .and_then(|o| o.value.as_str().map(String::from))
        });
    read_tx.commit().await.map_err(ApiError::from)?;
    let packs = packs
        .into_iter()
        .map(|p| ThemePackView::from_pack(p, active_theme.clone()))
        .collect();
    Ok(Json(ThemePackList { packs }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ThemePackList {
    pub packs: Vec<ThemePackView>,
}

/// DELETE /api/plugins/distribution/theme-packs/{slug} — uninstall.
/// Refused while the pack is the org's active theme (repoint the typed
/// `appearance.theme` setting first).
#[utoipa::path(operation_id = "theme_packs_delete", delete, path = "/api/plugins/distribution/theme-packs/{slug}", tag = "plugins", security(("bearer" = [])),
    params(("slug" = String, Path, description = "installed pack slug")),
    responses((status = 204), (status = 404), (status = 409)))]
pub async fn delete_theme_pack(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Path(slug): axum::extract::Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let user_slug = slug.clone();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let slug = user_slug.clone();
        let active_theme = core_db::settings::list_org_overrides(&mut *tx.tx, org_id)
            .await
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|o| o.key == core_domain::settings::APPEARANCE_THEME)
                    .and_then(|o| o.value.as_str().map(String::from))
            });
        core_db::theme_packs::delete_pack(&mut tx.tx, org_id, &slug, active_theme.as_deref())
            .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.theme_pack_uninstalled".into(),
                entity_type: Some("theme_pack".into()),
                entity_id: Some(slug.clone()),
                detail: serde_json::json!({ "slug": slug }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "uninstalled": slug })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ProposeUpdateRequest {
    /// Pin this catalog version as the update candidate.
    pub version: String,
    /// The grant set that WOULD apply post-consent (defaults to the
    /// current grants; a widening validates against the new manifest).
    pub granted_capabilities: Option<Vec<String>>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UpdateProposalView {
    #[schema(value_type = uuid::Uuid)]
    pub install_id: Uuid,
    pub slug: String,
    /// Still running — the OLD version is pinned until consented.
    pub current_version: String,
    pub current_granted_capabilities: Vec<String>,
    pub pending_version: Option<String>,
    pub pending_digest: Option<String>,
    pub consent_diff: ConsentDiffView,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConsentDiffView {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PutInstallSettingsRequest {
    pub config: serde_json::Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct InstallSettingsView {
    pub slug: String,
    pub version: String,
    /// The pinned manifest's declared JSON-Schema (the render source).
    pub settings_schema: Option<serde_json::Value>,
    pub config: serde_json::Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProvenanceView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub install_id: Uuid,
    pub source_kind: String,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub uploader: Option<Uuid>,
    pub action: String,
    #[schema(value_type = String, format = "date-time")]
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

impl From<PluginProvenanceEntry> for ProvenanceView {
    fn from(e: PluginProvenanceEntry) -> Self {
        Self {
            id: e.id,
            install_id: e.install_id,
            source_kind: e.source_kind.as_str().into(),
            source_url: e.source_url,
            fetched_ref: e.fetched_ref,
            artifact_digest: e.artifact_digest,
            uploader: e.uploader,
            action: e.action,
            recorded_at: e.recorded_at,
        }
    }
}

// --- fetching (+ SSRF posture) ---------------------------------------------------

/// Cap on re-fetched artifact bytes (B2): an artifact (WASM plugin /
/// skill manifest bundle) is not a dataset; the same order as the zip
/// sandbox cap. Larger refs must go through a release pipeline that
/// serves smaller artifacts.
const MAX_FETCH_ARTIFACT_BYTES: i64 = 8 * 1024 * 1024;

async fn fetch_bytes(url: &str) -> ApiResult<Vec<u8>> {
    url_guard(url)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ApiError::Validation(format!("http client unavailable: {e}")))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::Validation(format!("distribution fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::NotFound);
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ApiError::Validation(format!("distribution fetch failed: {e}")))?;
    Ok(bytes.to_vec())
}

/// SSRF posture: http(s)-only; localhost / private-network targets are
/// never accepted — the distribution seam must not become an internal
/// scanner for an unauthenticated surface.
fn url_guard(url: &str) -> ApiResult<()> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(ApiError::Validation("source url must be http(s)".into()));
    }
    let after = url.split_once("://").map(|(_s, rest)| rest).unwrap_or("");
    let authority = after.split('/').next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("").to_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host == "0.0.0.0"
        || host.ends_with(".local")
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host == "metadata.google.internal"
    {
        return Err(ApiError::Validation(
            "source url host is not reachable from the distribution seam".into(),
        ));
    }
    Ok(())
}

fn validate_repo_component(value: &str) -> ApiResult<()> {
    let bad = value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        || value.starts_with('.');
    if bad {
        return Err(ApiError::Validation(
            "github owner/repo must be plain components".into(),
        ));
    }
    Ok(())
}

// --- staging: every source funnels through ONE pending path -----------------------

/// Validated bundle → org-scoped PENDING row → audit + event. Staging is the
/// row BEFORE install; it executes nothing and grants nothing.
///
/// Stages a verified bundle, persisting its executable module when it carries
/// one.
///
/// The bytes are written HERE because this is the only place they exist: the
/// registry and GitHub adapters fetch a manifest and (optionally) the artifact
/// binary, and the zip adapter holds the archive. Everything downstream keeps a
/// digest, so a staged row without its object would be a consent screen for
/// something that can never run.
///
/// The write is content-addressed and idempotent, and it happens before the row
/// exists — so a staging failure cannot leave a row pointing at nothing. The
/// install is still DISABLED until an admin approves: writing bytes grants no
/// authority, and nothing executes from a staged row.
async fn stage_bundle(
    state: &AppState,
    authed: &crate::auth::AuthedUser,
    mut bundle: plugin_host::distribution::DistributionBundle,
    module: Option<&[u8]>,
    source_url_override: Option<String>,
) -> ApiResult<Json<StagedItemView>> {
    if let Some(url) = source_url_override {
        bundle.source_url = url;
    }
    for key in &bundle.declared_capabilities {
        capability_or_refuse(key)?;
    }
    if bundle.manifest_name().unwrap_or_default().trim().is_empty() {
        return Err(ApiError::Validation("manifest name is missing".into()));
    }
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    // Only an executable artifact gets a pin. A skill, a theme or an identity
    // bundle is data; a plugin manifest with no `.wasm` member is declarative.
    let module_digest = match (bundle.artifact_kind, module) {
        (PluginArtifactKind::Plugin, Some(bytes)) => {
            let digest = plugin_host::isolation::ModuleArtifact::digest_of(bytes);
            plugin_host::lifecycle::admit_module(&*state.store, org_id, bytes, &digest)
                .await
                .map_err(artifact_api)?;
            digest
        }
        _ => String::new(),
    };
    let item = PluginStagedItem {
        id: Uuid::now_v7(),
        org_id,
        artifact_kind: bundle.artifact_kind,
        source_kind: bundle.source_kind,
        source_url: bundle.source_url,
        fetched_ref: bundle.fetched_ref,
        artifact_digest: bundle.artifact_digest,
        module_digest,
        manifest: bundle.manifest,
        declared_capabilities: bundle.declared_capabilities,
        status: PluginStagedStatus::Pending,
        uploaded_by: actor,
        uploaded_at: chrono::Utc::now(),
        review_note: None,
        reviewed_by: None,
        reviewed_at: None,
        adopted_install_id: None,
        adopted_skill_id: None,
        adopted_theme_pack_id: None,
        adopted_identity_bundle_id: None,
    };
    let view = StagedItemView::from(item.clone());
    org_tx::with_org_tx(state, org_id, actor, |mut tx| async move {
        core_db::plugin_distribution::create_staged_item(&mut tx.tx, &item).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.staged_import_created".into(),
                entity_type: Some("plugin_staged_item".into()),
                entity_id: Some(item.id.to_string()),
                detail: serde_json::json!({
                    "artifact_kind": item.artifact_kind.as_str(),
                    "source_kind": item.source_kind.as_str(),
                    "source_url": item.source_url,
                    "fetched_ref": item.fetched_ref,
                    "artifact_digest": item.artifact_digest,
                    "module_digest": item.module_digest,
                    "declared_capabilities": item.declared_capabilities,
                    "status": "pending",
                }),
                ip: None,
                occurred_at: item.uploaded_at,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_STAGED_IMPORT_CREATED,
            "plugin_staged_item",
            Some(item.id.to_string().as_str()),
            &serde_json::to_value(PluginStagedImportCreated {
                staged_item_id: item.id,
                artifact_kind: item.artifact_kind.as_str().into(),
                source_kind: item.source_kind.as_str().into(),
                source_url: item.source_url.clone(),
                fetched_ref: item.fetched_ref.clone(),
                artifact_digest: item.artifact_digest.clone(),
                declared_capabilities: item.declared_capabilities.clone(),
                uploaded_by: actor,
                uploaded_at: item.uploaded_at,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(Json(view))
}

/// POST /api/plugins/distribution/staged/registry — a public marketplace
/// registry entry (static JSON, git-tag pinned, code-signature/digest
/// shape: the BUILD-PROMPT v1 fixture). The SERVER fetches and the
/// adapter validates BEFORE anything registers.
#[utoipa::path(operation_id = "plugin_distribution_stage_registry", post, path = "/api/plugins/distribution/staged/registry", tag = "plugins", security(("bearer" = [])),
    request_body = StageFromRegistryRequest,
    responses((status = 201, body = StagedItemView), (status = 404), (status = 422)))]
pub async fn stage_from_registry(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<StageFromRegistryRequest>,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let url = req.url.trim().to_string();
    let bytes = fetch_bytes(&url).await?;
    let bundle = {
        let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
        let keys = trusted_signing_keys(&state, &mut conn).await;
        ingest_registry_entry(&bytes, &keys).map_err(ingest_reason_api)?
    };
    // B2: re-fetch the artifact bytes and verify the digest myself —
    // the catalog's pinned digest is a claim until these bytes agree.
    let mut module: Option<Vec<u8>> = None;
    if let Some(artifact_url) = req
        .artifact_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        let artifact_bytes = fetch_bytes(artifact_url).await?;
        if artifact_bytes.len() as i64 > MAX_FETCH_ARTIFACT_BYTES {
            return Err(ingest_reason_api(
                plugin_host::distribution::IngestReason::ArchiveCap,
            ));
        }
        plugin_host::distribution::verify_artifact_bytes(&artifact_bytes, &bundle.artifact_digest)
            .map_err(ingest_reason_api)?;
        module = Some(artifact_bytes);
    }
    stage_bundle(&state, &authed, bundle, module.as_deref(), Some(url)).await
}

/// POST /api/plugins/distribution/staged/github — the pinned GitHub
/// pull: the manifest is fetched AT THE PINNED COMMIT (never HEAD), the
/// digest is computed over the fetched bytes, the pin is recorded on the
/// staged row; a no-pin call is refused.
#[utoipa::path(operation_id = "plugin_distribution_stage_github", post, path = "/api/plugins/distribution/staged/github", tag = "plugins", security(("bearer" = [])),
    request_body = StageFromGithubRequest,
    responses((status = 201, body = StagedItemView), (status = 404), (status = 422)))]
pub async fn stage_from_github(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<StageFromGithubRequest>,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let owner = req.owner.trim().to_string();
    let repo = req.repo.trim().to_string();
    validate_repo_component(&owner)?;
    validate_repo_component(&repo)?;
    let sha = req.commit.trim().to_lowercase();
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::Validation(
            "a pinned 40-hex commit is required (never HEAD)".into(),
        ));
    }
    let source_url = format!("https://github.com/{owner}/{repo}");
    let raw = format!("https://raw.githubusercontent.com/{owner}/{repo}/{sha}/manifest.json");
    let bytes = fetch_bytes(&raw).await?;
    let json = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|_| ApiError::Validation("the pinned manifest is not valid JSON".into()))?;
    let digest = plugin_host::distribution::sha256_hex(&bytes);
    let bundle = {
        let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
        let keys = trusted_signing_keys(&state, &mut conn).await;
        ingest_github_pull(&owner, &repo, req.tag.trim(), &sha, json, &digest, &keys)
            .map_err(ingest_reason_api)?
    };
    // B2: optional artifact-bytes refetch + cross-digest verification.
    let mut module: Option<Vec<u8>> = None;
    if let Some(artifact_url) = req
        .artifact_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        let artifact_bytes = fetch_bytes(artifact_url).await?;
        if artifact_bytes.len() as i64 > MAX_FETCH_ARTIFACT_BYTES {
            return Err(ingest_reason_api(
                plugin_host::distribution::IngestReason::ArchiveCap,
            ));
        }
        plugin_host::distribution::verify_artifact_bytes(&artifact_bytes, &bundle.artifact_digest)
            .map_err(ingest_reason_api)?;
        module = Some(artifact_bytes);
    }
    stage_bundle(&state, &authed, bundle, module.as_deref(), Some(source_url)).await
}

/// POST /api/plugins/distribution/staged/zip — the local zip import.
/// ALL the sandbox validation (manifest schema, artifact digest,
/// size/entry caps, exactly one root manifest.json, traversal safety)
/// runs BEFORE the row exists; the declared capability list rides into
/// the SAME consent screen.
#[utoipa::path(operation_id = "plugin_distribution_stage_zip", post,
    path = "/api/plugins/distribution/staged/zip", tag = "plugins",
    security(("bearer" = [])),
    request_body(content = Vec<u8>, content_type = "application/zip"),
    responses((status = 201, body = StagedItemView), (status = 400), (status = 413), (status = 422)))]
pub async fn stage_from_zip(
    State(state): State<AppState>,
    extensions: Extensions,
    body: Bytes,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    if body.len() as i64 > MAX_ARCHIVE_BYTES {
        return Err(ApiError::Validation(format!(
            "zip exceeds the sandbox cap of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    let bundle = ingest_local_zip(&body).map_err(ingest_reason_api)?;
    // The archive digest identifies what was imported; the module inside it is
    // what would run. Extraction refuses an archive with two candidate modules
    // rather than choosing one, and a declarative-only import legitimately has
    // none.
    let module =
        plugin_host::distribution::extract_module_from_zip(&body).map_err(ingest_reason_api)?;
    stage_bundle(&state, &authed, bundle, module.as_deref(), None).await
}

/// POST /api/plugins/distribution/staged/skill — skills from free/open
/// community marketplaces/repo APIs (`skill_community`: the server
/// refetches the manifest and digest-verifies it before the row exists)
/// and manual member uploads (`manual_upload`: inline manifest). Skill
/// content is UNTRUSTED RAG-class data — staged, never executed.
#[utoipa::path(operation_id = "plugin_distribution_stage_skill", post, path = "/api/plugins/distribution/staged/skill", tag = "plugins", security(("bearer" = [])),
    request_body = StageSkillRequest,
    responses((status = 201, body = StagedItemView), (status = 404), (status = 422)))]
pub async fn stage_from_skill(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<StageSkillRequest>,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let source_kind = PluginSourceKind::parse(&req.source_kind).ok_or_else(|| {
        ApiError::Validation(format!(
            "unknown source_kind {:?} — the vocabulary is closed",
            req.source_kind
        ))
    })?;
    match source_kind {
        PluginSourceKind::ManualUpload => {
            let manifest = req.manifest.clone().ok_or_else(|| {
                ApiError::Validation("a manual upload carries its manifest inline".into())
            })?;
            let bundle = ingest_skill_manifest(
                manifest,
                source_kind,
                req.source_url.trim(),
                req.fetched_ref.as_deref(),
                req.artifact_digest.as_deref().unwrap_or(""),
            )
            .map_err(ingest_reason_api)?;
            stage_bundle(&state, &authed, bundle, None, None).await
        }
        PluginSourceKind::SkillCommunity => {
            let url = req.source_url.trim().to_string();
            if url.is_empty() {
                return Err(ApiError::Validation(
                    "skill_community requires source_url".into(),
                ));
            }
            let bytes = fetch_bytes(&url).await?;
            let digest = plugin_host::distribution::sha256_hex(&bytes);
            if let Some(claimed) = &req.artifact_digest {
                if claimed.trim() != digest {
                    return Err(ApiError::Validation(
                        "community skill digest mismatch — refusing to stage".into(),
                    ));
                }
            }
            let json = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                ApiError::Validation("community skill manifest is unreadable".into())
            })?;
            let bundle =
                ingest_skill_manifest(json, source_kind, &url, req.fetched_ref.as_deref(), &digest)
                    .map_err(ingest_reason_api)?;
            stage_bundle(&state, &authed, bundle, None, None).await
        }
        _ => Err(ApiError::Validation(
            "source_kind is not a skills source (closed vocabulary)".into(),
        )),
    }
}

// --- staged review: uploader+admin visibility, owner/admin consent ----------------

/// GET /api/plugins/distribution/staged — pending + terminal rows.
/// Owners/admins see everything; plain members see only their OWN
/// uploads (the deny-by-default staged visibility).
#[utoipa::path(operation_id = "plugin_distribution_staged_list", get, path = "/api/plugins/distribution/staged", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = Vec<StagedItemView>)))]
pub async fn list_staged(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<StagedItemView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::plugin_distribution::list_staged_items(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let admin = authed.principal.is_owner_or_admin();
    let one = authed.principal.user.id;
    let views: Vec<StagedItemView> = rows
        .into_iter()
        .filter(|r| admin || r.uploaded_by == one)
        .map(Into::into)
        .collect();
    Ok(Json(views))
}

/// Loads a staged row with uploader visibility (deny-by-default: a
/// non-admin may only read their own upload; everyone else gets 404).
async fn get_staged_checked(
    state: &AppState,
    req_authed: &crate::auth::AuthedUser,
    org_id: Uuid,
    staged_id: Uuid,
) -> ApiResult<PluginStagedItem> {
    let admin = req_authed.principal.is_owner_or_admin();
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let item = core_db::plugin_distribution::get_staged_item(&mut tx, org_id, staged_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    if !admin && item.uploaded_by != req_authed.principal.user.id {
        return Err(ApiError::NotFound);
    }
    Ok(item)
}

/// GET /api/plugins/distribution/staged/{id}/consent — the SAME consent
/// shape for every source: declared capabilities + the manifest's
/// settings JSON-Schema (the typed-registry render source).
#[utoipa::path(operation_id = "plugin_distribution_staged_consent", get, path = "/api/plugins/distribution/staged/{id}/consent", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = ConsentView), (status = 404)))]
pub async fn get_staged_consent(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ConsentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let item = get_staged_checked(&state, &authed, org_id, id).await?;
    Ok(Json(ConsentView {
        artifact_kind: item.artifact_kind.as_str().into(),
        source_kind: item.source_kind.as_str().into(),
        source_url: item.source_url,
        fetched_ref: item.fetched_ref,
        artifact_digest: item.artifact_digest,
        declared_capabilities: item.declared_capabilities,
        settings_schema: item
            .manifest
            .get("settings_schema")
            .cloned()
            .filter(|v| !v.is_null()),
        // ADR 0009 §6 full semantics — the modeled connector governance
        // profile renders on the consent screen verbatim (ends the era
        // of webhook endpoints + scopes living outside the consent).
        connector_profile: item
            .manifest
            .get("connector_profile")
            .cloned()
            .filter(|v| !v.is_null()),
        // §4.6 — the surface declarations render on consent verbatim
        // (the same posture connector_profile holds for connectors).
        surface_profile: item
            .manifest
            .get("surface_profile")
            .cloned()
            .filter(|v| !v.is_null()),
    }))
}

// --- staged review (owner/admin) ----------------------------------------------------

/// POST /api/plugins/distribution/staged/{id}/approve — the owner/admin
/// review. Approve MINTS but remains deny-by-default: a plugin adopt is
/// created DISABLED (a public-registry source lands the SAME staged
/// pipeline — browsing is help, not bypass); a skill lands disabled
/// until enabled. Grants must subset the staged manifest's declared set.
#[utoipa::path(operation_id = "plugin_distribution_staged_approve", post, path = "/api/plugins/distribution/staged/{id}/approve", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = ApproveStagedRequest,
    responses((status = 200, body = StagedItemView), (status = 404), (status = 409)))]
pub async fn approve_staged(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ApproveStagedRequest>,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    // Cloned out of `state` because `with_org_tx` borrows it for the closure.
    let store = state.store.clone();
    let requested: Vec<PluginCapability> = req
        .granted_capabilities
        .iter()
        .map(|k| capability_or_refuse(k))
        .collect::<ApiResult<Vec<_>>>()?;
    let refreshed = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let item = core_db::plugin_distribution::get_staged_item(&mut tx.tx, org_id, id).await?;
        if item.status != PluginStagedStatus::Pending {
            return Err(ApiError::Conflict(format!(
                "staged import is terminal ({:?}); it cannot restart the review",
                item.status.as_str()
            )));
        }
        let declared: Vec<PluginCapability> = item
            .declared_capabilities
            .iter()
            .filter_map(|k| PluginCapability::parse(k))
            .collect();
        for cap in &requested {
            if !declared.iter().any(|d| d == cap) {
                return Err(ApiError::Conflict(format!(
                    "capability {:?} is not declared by the staged manifest",
                    cap.as_str()
                )));
            }
        }
        let granted_keys: Vec<String> = requested.iter().map(|c| c.as_str().to_string()).collect();
        let (
            adopted_install_id,
            adopted_skill_id,
            adopted_theme_pack_id,
            adopted_identity_bundle_id,
        ) = match item.artifact_kind {
            PluginArtifactKind::Plugin => {
                let manifest: PluginManifestV2 = serde_json::from_value(item.manifest.clone())
                    .map_err(|e| {
                        ApiError::Validation(format!("staged plugin manifest is unreadable: {e}"))
                    })?;
                manifest
                    .validate()
                    .map_err(|e| ApiError::Validation(e.to_string()))?;
                // The pin must be loadable BEFORE it becomes an install's
                // digest. The bytes were written at staging and verified then;
                // this re-reads them, so an object deleted or corrupted between
                // staging and approval is refused here rather than at the first
                // invocation.
                if !item.module_digest.is_empty() {
                    plugin_host::lifecycle::verify_stored_module(
                        &*store,
                        org_id,
                        &item.module_digest,
                    )
                    .await
                    .map_err(artifact_api)?;
                }
                // The adoption handle: slug derived from the artifact's
                // own declared name (lowercase, install-safe).
                let slug: String = manifest
                    .name
                    .trim()
                    .to_lowercase()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect();
                let slug = slug.trim_matches('-').to_string();
                let entry = match core_db::plugins::get_catalog_entry(&mut tx.tx, &slug)
                    .await
                    .map_err(core_db::DbError::normalized)
                {
                    Ok(existing) => {
                        if existing.revoked_at.is_some() {
                            return Err(ApiError::Conflict(
                                "a revoked catalog entry accepts no new versions or installs"
                                    .into(),
                            ));
                        }
                        // A staged import appends only to the entry its OWN org
                        // created through this path. The slug is derived from
                        // untrusted manifest text, so a collision must never
                        // silently adopt another publisher's identity: a `None`
                        // origin is an operator or seeded entry and is not an
                        // append target here either.
                        if existing.origin_org_id != Some(org_id) {
                            return Err(ApiError::Conflict(
                                "that catalog slug belongs to another publisher; publish \
                                 under a name this org owns"
                                    .into(),
                            ));
                        }
                        core_db::plugins::append_version(
                            &mut tx.tx,
                            existing.id,
                            &PluginVersion {
                                id: Uuid::now_v7(),
                                catalog_id: existing.id,
                                version: manifest.version.clone(),
                                manifest: manifest.clone(),
                                module_digest: item.module_digest.clone(),
                                created_at: now,
                            },
                        )
                        .await?;
                        existing
                    }
                    Err(core_db::DbError::NotFound) => {
                        let entry = PluginCatalogEntry {
                            id: Uuid::now_v7(),
                            slug: slug.clone(),
                            name: manifest.name.clone(),
                            description: manifest.description.clone(),
                            publisher: "staged-import".into(),
                            // Stamped with the importing org, so this org alone
                            // can append to or revoke the entry it created.
                            origin_org_id: Some(org_id),
                            latest_version: Some(manifest.version.clone()),
                            interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
                            revoked_at: None,
                            revoked_reason: None,
                            created_at: now,
                            updated_at: now,
                        };
                        core_db::plugins::create_catalog_entry(
                            &mut tx.tx,
                            &entry,
                            &PluginVersion {
                                id: Uuid::now_v7(),
                                catalog_id: entry.id,
                                version: manifest.version.clone(),
                                manifest: manifest.clone(),
                                module_digest: item.module_digest.clone(),
                                created_at: now,
                            },
                        )
                        .await?;
                        entry
                    }
                    Err(e) => return Err(e.into()),
                };
                let install = PluginInstall {
                    id: Uuid::now_v7(),
                    org_id,
                    catalog_id: entry.id,
                    slug: entry.slug.clone(),
                    version: manifest.version.clone(),
                    module_digest: item.module_digest.clone(),
                    status: PluginInstallStatus::Disabled,
                    granted_capabilities: granted_keys.clone(),
                    installed_by: Some(actor),
                    installed_at: now,
                    updated_at: now,
                    revoked_at: None,
                    revoked_by: None,
                    revoked_reason: None,
                    previous_version: None,
                    previous_module_digest: None,
                    previous_capabilities: Vec::new(),
                };
                core_db::plugins::create_install(&mut tx.tx, &install).await?;
                // §4.6: the adopt's surface registrations derive from the
                // staged manifest's surface_profile under the approved
                // grants (data rows; they render nowhere until enable).
                plugin_host::surface::sync_install_surface(&mut tx.tx, &install, &manifest)
                    .await
                    .map_err(anyhow::Error::msg)?;
                // The provenance chain — the SAME shape every source
                // produces, appended right at adoption time.
                core_db::plugin_distribution::insert_provenance(
                    &mut tx.tx,
                    &PluginProvenanceEntry {
                        id: Uuid::now_v7(),
                        org_id,
                        install_id: install.id,
                        source_kind: item.source_kind,
                        source_url: item.source_url.clone(),
                        fetched_ref: item.fetched_ref.clone(),
                        artifact_digest: item.artifact_digest.clone(),
                        uploader: Some(item.uploaded_by),
                        action: "installed".into(),
                        detail: Some(serde_json::json!({
                            "granted_capabilities": granted_keys,
                            "staged_item_id": item.id,
                        })),
                        recorded_at: now,
                    },
                )
                .await?;
                (Some(install.id), None, None, None)
            }
            PluginArtifactKind::Skill => {
                // Skills reuse the 0040 versioned-rows seam (append-only
                // history; rollback = a NEW version). Content is
                // untrusted RAG-class data — persisted verbatim.
                let slug: String = item
                    .manifest
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase()
                    .chars()
                    .map(|c| {
                        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect();
                let slug = slug.trim_matches('-').to_string();
                if slug.is_empty() {
                    return Err(ApiError::Validation(
                        "staged skill manifest is unreadable".into(),
                    ));
                }
                let max =
                    core_db::agent_platform::max_skill_version(&mut *tx.tx, org_id, &slug).await?;
                let row = core_db::agent_platform::insert_skill(
                    &mut tx.tx,
                    Uuid::now_v7(),
                    org_id,
                    actor,
                    &slug,
                    max + 1,
                    &item.manifest,
                )
                .await?;
                (None, Some(row.id), None, None)
            }
            PluginArtifactKind::Theme => {
                // Wave 24: approving a staged theme pack MINTS the org's
                // theme_packs row. Themes carry ZERO capabilities (the
                // ingestion gate refuses anything else), so there is no
                // grant screen beyond the same owner/admin review the
                // other kinds ride; the pack rides the SAME audit event.
                let manifest = item.manifest.clone();
                let name = manifest
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let version = manifest
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if name.is_empty() || version.is_empty() {
                    return Err(ApiError::Validation(
                        "staged theme manifest is unreadable".into(),
                    ));
                }
                let pack = core_db::theme_packs::install_from_staged(
                    &mut tx.tx,
                    org_id,
                    Uuid::now_v7(),
                    item.source_kind.as_str(),
                    &name,
                    &version,
                    &manifest,
                    &item.source_url,
                    item.fetched_ref.as_deref(),
                    &item.artifact_digest,
                    actor,
                )
                .await?;
                (None, None, Some(pack.id), None)
            }
            PluginArtifactKind::IdentityBundle => {
                // Approving a staged identity bundle MINTS the install
                // row, landing DISABLED — approval adopts the artifact,
                // activation is the separate audited admin act that
                // `auth::identity_bundle_service::apply_action` performs.
                //
                // The capability refusal is restated HERE rather than
                // trusted to ingest: a staged row can outlive a code
                // change, and "an identity bundle confers no Marula
                // capability" has to hold at the moment of adoption, not
                // only at the moment of upload.
                if !item.declared_capabilities.is_empty() || !granted_keys.is_empty() {
                    return Err(ApiError::Conflict(
                        "an identity bundle confers no Marula capability; the consent screen \
                         has nothing to grant"
                            .into(),
                    ));
                }
                let manifest =
                    core_domain::entities::identity_bundle::IdentityBundleManifest::from_value(
                        &item.manifest,
                    )
                    .map_err(|e| {
                        ApiError::Validation(format!("staged identity bundle manifest: {e}"))
                    })?;
                let install = core_db::identity_bundles::install_from_staged(
                    &mut tx.tx,
                    org_id,
                    Uuid::now_v7(),
                    Some(item.id),
                    item.source_kind.as_str(),
                    &item.source_url,
                    item.fetched_ref.as_deref(),
                    &item.artifact_digest,
                    &item.manifest,
                    actor,
                )
                .await?;
                core_db::audit::insert(
                    &mut *tx.tx,
                    &core_domain::entities::AuditEvent {
                        id: Uuid::now_v7(),
                        org_id,
                        actor: core_domain::entities::Actor::Human(actor),
                        action: core_domain::events::types::IDENTITY_BUNDLE_INSTALLED.into(),
                        entity_type: Some("identity_bundle_install".into()),
                        entity_id: Some(install.id.to_string()),
                        detail: core_db::identity_bundles::install_audit_detail(&install),
                        ip: None,
                        occurred_at: now,
                    },
                )
                .await?;
                tx.emit(
                    core_domain::events::types::IDENTITY_BUNDLE_INSTALLED,
                    "identity_bundle_install",
                    Some(install.id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::IdentityBundleInstalled {
                        install_id: install.id,
                        name: install.name.clone(),
                        version: install.version.clone(),
                        interface_version: manifest.interface_version,
                        provider_kinds: install.provider_kinds.clone(),
                        artifact_digest: install.provenance_digest.clone(),
                        status: install.status.as_str().to_string(),
                        occurred_at: now,
                    })
                    .unwrap_or(serde_json::Value::Null),
                )
                .await?;
                (None, None, None, Some(install.id))
            }
        };
        core_db::plugin_distribution::update_staged_status(
            &mut tx.tx,
            org_id,
            item.id,
            PluginStagedStatus::Approved,
            req.review_note.as_deref(),
            Some(actor),
            now,
            adopted_install_id,
            adopted_skill_id,
            adopted_theme_pack_id,
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.staged_import_approved".into(),
                entity_type: Some("plugin_staged_item".into()),
                entity_id: Some(item.id.to_string()),
                detail: serde_json::json!({
                    "artifact_kind": item.artifact_kind.as_str(),
                    "source_kind": item.source_kind.as_str(),
                    "artifact_digest": item.artifact_digest,
                    "granted_capabilities": granted_keys,
                    "adopted_install_id": adopted_install_id,
                    "adopted_skill_id": adopted_skill_id,
                    "adopted_theme_pack_id": adopted_theme_pack_id,
                    "adopted_identity_bundle_id": adopted_identity_bundle_id,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_STAGED_IMPORT_APPROVED,
            "plugin_staged_item",
            Some(item.id.to_string().as_str()),
            &serde_json::to_value(PluginStagedImportApproved {
                staged_item_id: item.id,
                reviewed_by: actor,
                review_note: req.review_note.clone(),
                artifact_kind: item.artifact_kind.as_str().into(),
                adopted_install_id,
                adopted_skill_id,
                granted_capabilities: granted_keys.clone(),
                approved_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let refreshed =
            core_db::plugin_distribution::get_staged_item(&mut tx.tx, org_id, item.id).await?;
        Ok((tx, refreshed))
    })
    .await?;
    Ok(Json(refreshed.into()))
}

/// POST /api/plugins/distribution/staged/{id}/refuse — the reviewer
/// declines: terminal for the staged row (the uploader may re-stage a
/// fresh upload). A typed note is required.
#[utoipa::path(operation_id = "plugin_distribution_staged_refuse", post, path = "/api/plugins/distribution/staged/{id}/refuse", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = RefuseStagedRequest,
    responses((status = 200, body = StagedItemView), (status = 404), (status = 409)))]
pub async fn refuse_staged(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RefuseStagedRequest>,
) -> ApiResult<Json<StagedItemView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let note = req.review_note.trim().to_string();
    if note.is_empty() {
        return Err(ApiError::Validation("a refusal note is required".into()));
    }
    let now = chrono::Utc::now();
    let refreshed = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let item = core_db::plugin_distribution::get_staged_item(&mut tx.tx, org_id, id).await?;
        if item.status != PluginStagedStatus::Pending {
            return Err(ApiError::Conflict(format!(
                "staged import is terminal ({:?}); refusal is not a reachable state",
                item.status.as_str()
            )));
        }
        core_db::plugin_distribution::update_staged_status(
            &mut tx.tx,
            org_id,
            item.id,
            PluginStagedStatus::Refused,
            Some(&note),
            Some(actor),
            now,
            None,
            None,
            None,
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.staged_import_refused".into(),
                entity_type: Some("plugin_staged_item".into()),
                entity_id: Some(item.id.to_string()),
                detail: serde_json::json!({
                    "source_kind": item.source_kind.as_str(),
                    "artifact_digest": item.artifact_digest,
                    "review_note": note,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_STAGED_IMPORT_REFUSED,
            "plugin_staged_item",
            Some(item.id.to_string().as_str()),
            &serde_json::to_value(PluginStagedImportRefused {
                staged_item_id: item.id,
                reviewed_by: actor,
                review_note: Some(note),
                refused_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let refreshed =
            core_db::plugin_distribution::get_staged_item(&mut tx.tx, org_id, item.id).await?;
        Ok((tx, refreshed))
    })
    .await?;
    Ok(Json(refreshed.into()))
}

// --- update re-consent flow (ADR 0009 lifecycle completeness) -----------------------

/// POST /api/plugins/installed/{id}/update — propose the update: the
/// candidate version lands in the pending columns while the OLD version
/// keeps running; the response IS the re-consent diff the consent screen
/// renders.
#[utoipa::path(operation_id = "plugin_install_update_propose", post, path = "/api/plugins/installed/{id}/update", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = ProposeUpdateRequest,
    responses((status = 200, body = UpdateProposalView), (status = 404), (status = 409)))]
pub async fn propose_update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ProposeUpdateRequest>,
) -> ApiResult<Json<UpdateProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let store = state.store.clone();
    let requested_caps = match &req.granted_capabilities {
        Some(keys) => Some(
            keys.iter()
                .map(|k| capability_or_refuse(k))
                .collect::<ApiResult<Vec<_>>>()?,
        ),
        None => None,
    };
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        if existing.status == PluginInstallStatus::Revoked {
            return Err(ApiError::Conflict(
                "a revoked install is terminal; re-install to adopt the entry again".into(),
            ));
        }
        let pinned =
            core_db::plugins::get_version(&mut tx.tx, existing.catalog_id, req.version.trim())
                .await
                .map_err(|_| {
                    ApiError::Validation(format!(
                        "version {:?} is not a catalog version of this entry",
                        req.version.trim()
                    ))
                })?;
        // The proposal carries the candidate's DIGEST, and a candidate with no
        // module cannot take over an install that has one: consenting to it
        // would leave the install naming bytes it does not run. Refused here,
        // at the point an operator can still see why.
        if pinned.module_digest.is_empty() && !existing.module_digest.is_empty() {
            return Err(ApiError::Conflict(format!(
                "version {} ships no module, and this install runs a pinned one; an update may \
                 not clear a pin",
                pinned.manifest.version
            )));
        }
        if !pinned.module_digest.is_empty() {
            plugin_host::lifecycle::verify_stored_module(&*store, org_id, &pinned.module_digest)
                .await
                .map_err(artifact_api)?;
        }
        let new_caps: Vec<String> = match &requested_caps {
            Some(caps) => {
                for cap in caps {
                    if !pinned
                        .manifest
                        .capabilities
                        .contains(&cap.as_str().to_string())
                    {
                        return Err(ApiError::Conflict(format!(
                            "capability {:?} is not declared by version {}",
                            cap.as_str(),
                            pinned.manifest.version
                        )));
                    }
                }
                caps.iter().map(|c| c.as_str().to_string()).collect()
            }
            None => existing.granted_capabilities.clone(),
        };
        core_db::plugin_distribution::set_pending_version(
            &mut tx.tx,
            org_id,
            id,
            pinned.manifest.version.as_str(),
            &serde_json::to_value(&pinned.manifest).unwrap_or(serde_json::Value::Null),
            &pinned.module_digest,
            Some(&new_caps),
            now,
        )
        .await?;
        // The provenance chain shows the proposal (the consent commit
        // appends the re-consent link later).
        let last = core_db::plugin_distribution::latest_provenance(&mut tx.tx, org_id, id).await;
        let (last_kind, last_url) = match &last {
            Ok(l) => (l.source_kind, l.source_url.clone()),
            Err(_) => (PluginSourceKind::MarketplaceRegistry, String::new()),
        };
        core_db::plugin_distribution::insert_provenance(
            &mut tx.tx,
            &PluginProvenanceEntry {
                id: Uuid::now_v7(),
                org_id,
                install_id: id,
                source_kind: last_kind,
                source_url: last_url,
                fetched_ref: Some(pinned.manifest.version.clone()),
                artifact_digest: pinned.module_digest.clone(),
                uploader: Some(actor),
                action: "updated".into(),
                detail: Some(serde_json::json!({
                    "from_version": existing.version,
                    "to_version": pinned.manifest.version,
                    "module_digest": pinned.module_digest,
                    "state": "pending_consent",
                })),
                recorded_at: now,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.update_pending".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": existing.slug,
                    "from_version": existing.version,
                    "to_version": pinned.manifest.version,
                    "state": "pending_consent",
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        let diff = ConsentDiff::compute(
            &existing.granted_capabilities,
            &pinned.manifest.capabilities,
            pinned.manifest.settings_schema.clone(),
        );
        tx.emit(
            types::PLUGIN_UPDATE_PENDING,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(PluginUpdatePending {
                install_id: id,
                catalog_id: existing.catalog_id,
                slug: existing.slug.clone(),
                current_version: existing.version.clone(),
                pending_version: pinned.manifest.version.clone(),
                pending_digest: pinned.module_digest.clone(),
                added_capabilities: diff.added.clone(),
                removed_capabilities: diff.removed.clone(),
                occurred_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    get_update_view(&state, org_id, id).await
}

/// GET /api/plugins/installed/{id}/update — the pending-state read (the
/// consent screen shows exactly what consent would do).
#[utoipa::path(operation_id = "plugin_install_update_get", get, path = "/api/plugins/installed/{id}/update", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = UpdateProposalView), (status = 404)))]
pub async fn get_update_proposal(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UpdateProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    get_update_view(&state, org_id, id).await
}

/// POST /api/plugins/installed/{id}/consent — commit the CONSENTED
/// update: the swap IS the consent action (deny-by-default — until the
/// owner/admin consents, the OLD version is what runs). Consent appends
/// the provenance chain and rides the existing update event.
#[utoipa::path(operation_id = "plugin_install_update_consent", post, path = "/api/plugins/installed/{id}/consent", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = UpdateProposalView), (status = 404), (status = 409)))]
pub async fn consent_update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<UpdateProposalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let store = state.store.clone();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let pending = core_db::plugin_distribution::get_pending_manifest(&mut tx.tx, org_id, id)
            .await?
            .ok_or_else(|| {
                ApiError::Conflict("this install has no update proposal awaiting consent".into())
            })?;
        let (pending_version, _pending_manifest, pending_digest, _pending_caps) = pending;
        // The candidate's bytes are re-verified at the moment they would become
        // the pin. Between proposal and consent an operator can delete an
        // object; consenting to a digest the store cannot serve would produce an
        // install that enables and then fails.
        if !pending_digest.is_empty() {
            plugin_host::lifecycle::verify_stored_module(&*store, org_id, &pending_digest)
                .await
                .map_err(artifact_api)?;
        }
        let before = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        core_db::plugin_distribution::commit_pending_version(&mut tx.tx, org_id, id).await?;
        let updated = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        // §4.6: the re-consented manifest rewrites the surface
        // registrations to the NEW pinned profile under the same grants.
        let target_manifest =
            core_db::plugins::get_version(&mut tx.tx, updated.catalog_id, &updated.version)
                .await?
                .manifest;
        plugin_host::surface::sync_install_surface(&mut tx.tx, &updated, &target_manifest)
            .await
            .map_err(anyhow::Error::msg)?;
        let last = core_db::plugin_distribution::latest_provenance(&mut tx.tx, org_id, id).await;
        let (last_kind, last_url) = match &last {
            Ok(l) => (l.source_kind, l.source_url.clone()),
            Err(_) => (PluginSourceKind::MarketplaceRegistry, String::new()),
        };
        core_db::plugin_distribution::insert_provenance(
            &mut tx.tx,
            &PluginProvenanceEntry {
                id: Uuid::now_v7(),
                org_id,
                install_id: id,
                source_kind: last_kind,
                source_url: last_url,
                fetched_ref: Some(pending_version.clone()),
                artifact_digest: pending_digest,
                uploader: Some(actor),
                action: "re_consented".into(),
                detail: Some(serde_json::json!({
                    "from_version": before.version,
                    "to_version": pending_version,
                })),
                recorded_at: now,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.update_consented".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": updated.slug,
                    "from_version": before.version,
                    "to_version": pending_version,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_INSTALL_UPDATED,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PluginInstallUpdated {
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
        Ok((tx, ()))
    })
    .await?;
    get_update_view(&state, org_id, id).await
}

/// POST /api/plugins/installed/{id}/discard-update — abandon the pending
/// re-consent (the OLD version keeps running; the pending columns clear).
#[utoipa::path(operation_id = "plugin_install_update_discard", post, path = "/api/plugins/installed/{id}/discard-update", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 204), (status = 404)))]
pub async fn discard_update(
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
        core_db::plugin_distribution::clear_pending_version(&mut tx.tx, org_id, id).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.update_discarded".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "slug": existing.slug }),
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokeInstallRequest {
    /// Required. A kill switch with no recorded reason is an incident nobody
    /// can review.
    pub reason: String,
}

/// POST /api/plugins/installed/{id}/rollback — move the pin back to the
/// previous digest.
///
/// A rollback is a POINTER MOVE: version, digest and grants return to what they
/// were before the last update, and no bytes are fetched. The old module is
/// still in the store because keys are content-addressed, so an update wrote a
/// new key rather than overwriting the old one.
///
/// One step, not a toggle: the parked target is cleared, so a second rollback
/// answers `409` until the update is re-consented. That is deliberate — the
/// operator should say which bytes they want, not ping-pong between two.
#[utoipa::path(operation_id = "plugin_install_rollback", post,
    path = "/api/plugins/installed/{id}/rollback", tag = "plugins",
    security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = InstallView), (status = 404), (status = 409)))]
pub async fn rollback_install(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InstallView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let view = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let before = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        // The pure transition table answers first, so the refusal is the same
        // typed one the model and the tests describe.
        let state_before = LifecycleState::from_install(&before);
        plugin_host::lifecycle::apply(Some(&state_before), &LifecycleAction::Rollback)
            .map_err(lifecycle_api)?;
        let (version, digest) = core_db::plugins::rollback_pin(&mut tx.tx, org_id, id).await?;
        let updated = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        // The older manifest's surface profile is what should render now.
        let target_manifest =
            core_db::plugins::get_version(&mut tx.tx, updated.catalog_id, &version)
                .await?
                .manifest;
        plugin_host::surface::sync_install_surface(&mut tx.tx, &updated, &target_manifest)
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
        let last = core_db::plugin_distribution::latest_provenance(&mut tx.tx, org_id, id).await;
        let (last_kind, last_url) = match &last {
            Ok(l) => (l.source_kind, l.source_url.clone()),
            Err(_) => (PluginSourceKind::MarketplaceRegistry, String::new()),
        };
        core_db::plugin_distribution::insert_provenance(
            &mut tx.tx,
            &PluginProvenanceEntry {
                id: Uuid::now_v7(),
                org_id,
                install_id: id,
                source_kind: last_kind,
                source_url: last_url,
                fetched_ref: Some(version.clone()),
                artifact_digest: digest.clone(),
                uploader: Some(actor),
                action: "rolled_back".into(),
                detail: Some(serde_json::json!({
                    "from_version": before.version,
                    "to_version": version,
                    "module_digest": digest,
                })),
                recorded_at: now,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.install_rolled_back".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": updated.slug,
                    "from_version": before.version,
                    "to_version": updated.version,
                    "module_digest": updated.module_digest,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_INSTALL_UPDATED,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PluginInstallUpdated {
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

/// POST /api/plugins/installed/{id}/revoke — the org's own kill switch for ONE
/// install.
///
/// Distinct from disabling (reversible, grants untouched) and from the
/// catalog-wide revocation (which cascades to every tenant's installs). Revoke
/// is terminal, the reason is required, and execution stops at the parent's
/// gate: `authorize_invocation` refuses a revoked status, and the artifact load
/// path refuses before it reads a byte.
#[utoipa::path(operation_id = "plugin_install_revoke", post,
    path = "/api/plugins/installed/{id}/revoke", tag = "plugins",
    security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    request_body = RevokeInstallRequest,
    responses((status = 200, body = InstallView), (status = 404), (status = 409)))]
pub async fn revoke_install(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RevokeInstallRequest>,
) -> ApiResult<Json<InstallView>> {
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
    let view = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let before = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        core_db::plugins::revoke_install(&mut tx.tx, org_id, id, Some(actor), &reason, now).await?;
        let revoked = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        let manifest =
            core_db::plugins::get_version(&mut tx.tx, revoked.catalog_id, &revoked.version)
                .await
                .map(|v| v.manifest)
                .unwrap_or_else(|_| fallback_manifest(&revoked));
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.install_revoked".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": revoked.slug,
                    "reason": reason,
                    "from_status": before.status.as_str(),
                    "module_digest": revoked.module_digest,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_INSTALL_REVOKED,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::PluginInstallRevoked {
                install_id: id,
                catalog_id: revoked.catalog_id,
                slug: revoked.slug.clone(),
                reason: Some(reason.clone()),
                revoked_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        let name = core_db::plugins::get_catalog_entry(&mut tx.tx, &revoked.slug)
            .await
            .map(|e| e.name)
            .unwrap_or_else(|_| revoked.slug.clone());
        Ok((tx, InstallView::from_install(revoked, name, &manifest)))
    })
    .await?;
    Ok(Json(view))
}
/// The shared read for the update view: install + pending columns →
/// consent diff against the pending manifest's declared set.
async fn get_update_view(
    state: &AppState,
    org_id: Uuid,
    install_id: Uuid,
) -> ApiResult<Json<UpdateProposalView>> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let existing = core_db::plugins::get_install(&mut tx, org_id, install_id).await?;
    let pending =
        core_db::plugin_distribution::get_pending_manifest(&mut tx, org_id, install_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let (pending_version, pending_manifest, pending_digest, pending_caps) = match pending {
        Some(p) => (Some(p.0), Some(p.1), Some(p.2), p.3),
        None => (None, None, None, Vec::new()),
    };
    let diff = match (&pending_manifest, &pending_caps) {
        (Some(m), c) => {
            let declared: Vec<String> = c
                .iter()
                .filter_map(|k| PluginCapability::parse(k).map(|cap| cap.as_str().to_string()))
                .collect();
            let cap_keys = m
                .get("capabilities")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();
            let _ = cap_keys;
            let d = ConsentDiff::compute(&existing.granted_capabilities, &declared, None);
            (d.added, d.removed, d.unchanged)
        }
        _ => ([].to_vec(), [].to_vec(), [].to_vec()),
    };
    Ok(Json(UpdateProposalView {
        install_id: existing.id,
        slug: existing.slug,
        current_version: existing.version,
        current_granted_capabilities: existing.granted_capabilities,
        pending_version,
        pending_digest,
        consent_diff: ConsentDiffView {
            added: diff.0,
            removed: diff.1,
            unchanged: diff.2,
        },
    }))
}

// --- per-install settings (rendered FROM the manifest JSON-Schema) ------------------

/// GET /api/plugins/installed/{id}/settings — the render source: the
/// pinned manifest's declared JSON-Schema + the validated config.
#[utoipa::path(operation_id = "plugin_install_settings_get", get, path = "/api/plugins/installed/{id}/settings", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = InstallSettingsView), (status = 404)))]
pub async fn get_install_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InstallSettingsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    settings_view(&state, org_id, id).await
}

async fn settings_view(
    state: &AppState,
    org_id: Uuid,
    install_id: Uuid,
) -> ApiResult<Json<InstallSettingsView>> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let install = core_db::plugins::get_install(&mut tx, org_id, install_id).await?;
    let version =
        core_db::plugins::get_version(&mut tx, install.catalog_id, &install.version).await?;
    let config =
        core_db::plugin_distribution::get_install_settings(&mut tx, org_id, install_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(InstallSettingsView {
        slug: install.slug,
        version: install.version,
        settings_schema: version.manifest.settings_schema,
        config,
    }))
}

/// PATCH /api/plugins/installed/{id}/settings — the config is validated
/// against the pinned manifest's schema AT THE WRITE BOUNDARY (the
/// typed-registry doctrine: no hand-authored per-plugin pages, and a
/// plugin with no declared schema gets consent/enable/disable only —
/// this endpoint refuses its config writes). Every write is audited.
#[utoipa::path(operation_id = "plugin_install_settings_put", patch, path = "/api/plugins/installed/{id}/settings", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = PutInstallSettingsRequest,
    responses((status = 200, body = InstallSettingsView), (status = 404), (status = 409), (status = 422)))]
pub async fn put_install_settings(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PutInstallSettingsRequest>,
) -> ApiResult<Json<InstallSettingsView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let install = core_db::plugins::get_install(&mut tx.tx, org_id, id).await?;
        let version =
            core_db::plugins::get_version(&mut tx.tx, install.catalog_id, &install.version).await?;
        let Some(schema_value) = &version.manifest.settings_schema else {
            return Err(ApiError::Conflict(
                "this plugin declares no settings schema; it gets consent/enable/disable only"
                    .into(),
            ));
        };
        let schema = core_domain::entities::plugins::SettingsSchema::new(schema_value.clone())
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        schema
            .validate_config(&req.config)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
        core_db::plugin_distribution::upsert_install_settings(
            &mut tx.tx,
            org_id,
            id,
            &req.config,
            actor,
        )
        .await?;
        let keys: Vec<String> = req
            .config
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.settings_updated".into(),
                entity_type: Some("plugin_install".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "slug": install.slug,
                    "version": install.version,
                    "config_keys": keys,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_SETTINGS_UPDATED,
            "plugin_install",
            Some(id.to_string().as_str()),
            &serde_json::to_value(PluginSettingsUpdated {
                install_id: id,
                slug: install.slug.clone(),
                version: install.version.clone(),
                config_keys: keys,
                updated_by: actor,
                occurred_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    settings_view(&state, org_id, id).await
}

// --- provenance read (the same chain every source records) --------------------------

/// GET /api/plugins/installed/{id}/provenance — the append-only
/// `{source_kind, source_url, fetched_ref, artifact_digest, uploader}`
/// for one install. The chain SURVIVES terminal uninstalls (no cascade).
#[utoipa::path(operation_id = "plugin_distribution_provenance_list", get, path = "/api/plugins/installed/{id}/provenance", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = Vec<ProvenanceView>), (status = 404)))]
pub async fn list_install_provenance(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ProvenanceView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::plugin_distribution::list_provenance(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// POST /api/plugins/distribution/agent-skills/{id}/enabled — the skills
/// half's enable gate (deny-by-default: enable is an explicit
/// owner/admin action on the active version row; version rows other than
/// the active one are never enabled).
#[utoipa::path(operation_id = "plugin_distribution_skill_enabled", post, path = "/api/plugins/distribution/agent-skills/{id}/enabled", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = RefuseStagedRequest,
    responses((status = 204), (status = 404)))]
pub async fn set_skill_enabled(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SetSkillEnabledRequest>,
) -> ApiResult<StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        // Locate the ACTIVE (latest) row of the slug to gate on.
        let skill = core_db::plugin_distribution::get_skill_version(&mut tx.tx, org_id, id).await?;
        core_db::plugin_distribution::set_skill_enabled(
            &mut tx.tx,
            org_id,
            &skill.slug,
            req.enabled,
            actor,
            now,
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: if req.enabled {
                    "agent_skill.enabled"
                } else {
                    "agent_skill.disabled"
                }
                .into(),
                entity_type: Some("agent_skill".into()),
                entity_id: Some(skill.id.to_string()),
                detail: serde_json::json!({
                    "slug": skill.slug,
                    "version": skill.version,
                    "enabled": req.enabled,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::AGENT_SKILL_UPDATED,
            "agent_skill",
            Some(skill.id.to_string().as_str()),
            &serde_json::to_value(&core_domain::events::AgentSkillEnabledPayload {
                skill_id: skill.id,
                slug: skill.slug,
                version: skill.version,
                enabled: req.enabled,
                updated_by: actor,
            })
            .expect("typed agent-skill enable payload serializes"),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetSkillEnabledRequest {
    pub enabled: bool,
}

// --- GET /api/plugins/distribution/agent-skills — the org's skill read ---------------

/// GET /api/plugins/distribution/agent-skills — the ACTIVE version of
/// every org skill, with its provenance.
#[utoipa::path(operation_id = "plugin_distribution_skills_list", get, path = "/api/plugins/distribution/agent-skills", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = Vec<SkillVersionView>)))]
pub async fn list_org_skills(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<SkillVersionView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::plugin_distribution::list_latest_skills(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SkillVersionView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub slug: String,
    pub version: i32,
    pub manifest: serde_json::Value,
    pub source_kind: String,
    pub source_url: String,
    pub fetched_ref: Option<String>,
    pub artifact_digest: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<AgentSkillVersion> for SkillVersionView {
    fn from(v: AgentSkillVersion) -> Self {
        Self {
            id: v.id,
            slug: v.slug,
            version: v.version,
            manifest: serde_json::to_value(v.manifest).unwrap_or(serde_json::Value::Null),
            source_kind: v.source_kind.as_str().into(),
            source_url: v.source_url,
            fetched_ref: v.fetched_ref,
            artifact_digest: v.artifact_digest,
            enabled: v.enabled,
            created_at: v.created_at,
        }
    }
}

// --- Marketplace v2 (Wave 22): verified publishers + ratings ------------------------

/// Normalizes a publisher source identity: `registry://slug` verbatim;
/// `github.com/{owner}/{repo}` collapses to `github.com/{owner}` (the
/// publisher is the owner, not one repo); any bare URL collapses to its
/// host (zip uploads are vouched per host). Empty results are refusals.
fn normalize_publisher(publisher: &str) -> ApiResult<String> {
    let value = publisher.trim().to_lowercase();
    if value.is_empty() {
        return Err(ApiError::Validation(
            "a publisher identity is required".into(),
        ));
    }
    if let Some(slug) = value
        .strip_prefix("registry://")
        .filter(|s| validate_repo_component(s).is_ok())
    {
        return Ok(format!("registry://{slug}"));
    }
    let no_scheme = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .unwrap_or(&value)
        .trim_matches('/');
    let mut parts = no_scheme.split('/');
    let host = parts
        .next()
        .ok_or_else(|| ApiError::Validation("the publisher identity is not a URL".into()))?;
    let owner = parts.next().unwrap_or("");
    if host == "github.com" {
        if owner.is_empty() {
            return Err(ApiError::Validation(
                "github.com needs an owner path".into(),
            ));
        }
        validate_repo_component(owner)?;
        return Ok(format!("github.com/{owner}"));
    }
    // Host-only identity for everything else (zip contributors and
    // unknown hosts vouch at host granularity).
    if host.contains('/') || host.is_empty() || host.len() > 253 {
        return Err(ApiError::Validation(
            "the publisher identity is not a URL".into(),
        ));
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(ApiError::Validation(
            "the publisher host is not a valid hostname".into(),
        ));
    }
    Ok(host.to_string())
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VerifyPublisherRequest {
    /// GitHub `owner/repo` URL or bare path, `registry://slug`, or
    /// a host (zips) — normalized server-side to the publisher identity.
    pub publisher: String,
    pub display_name: String,
    #[schema(value_type = Option<String>)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PublisherVerificationView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub publisher: String,
    pub display_name: String,
    pub status: String,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub verified_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub note: String,
}

impl From<PluginPublisherVerification> for PublisherVerificationView {
    fn from(v: PluginPublisherVerification) -> Self {
        Self {
            id: v.id,
            publisher: v.publisher,
            display_name: v.display_name,
            status: v.status,
            verified_at: v.verified_at,
            revoked_at: v.revoked_at,
            note: v.note,
        }
    }
}

/// GET /api/plugins/distribution/publishers — the org's publisher
/// registry (vouched history included; `verified` = the active badge).
#[utoipa::path(operation_id = "plugin_distribution_publishers_list", get, path = "/api/plugins/distribution/publishers", tag = "plugins", security(("bearer" = [])),
    responses((status = 200, body = [PublisherVerificationView])))]
pub async fn list_publishers(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PublisherVerificationView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows =
        core_db::plugin_distribution::list_publisher_verifications(&mut *tx, org_id, false).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

/// POST /api/plugins/distribution/publishers — admin vouches a
/// publisher identity on the org's behalf (re-vouch flips a revoked row
/// back; a double-verify conflicts).
#[utoipa::path(operation_id = "plugin_distribution_publishers_verify", post, path = "/api/plugins/distribution/publishers", tag = "plugins", security(("bearer" = [])),
    request_body = VerifyPublisherRequest,
    responses((status = 200, body = PublisherVerificationView), (status = 409)))]
pub async fn verify_publisher(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<VerifyPublisherRequest>,
) -> ApiResult<Json<PublisherVerificationView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let publisher = normalize_publisher(&req.publisher)?;
    let display_name = req.display_name.trim().to_string();
    if display_name.is_empty() || display_name.len() > 200 {
        return Err(ApiError::Validation(
            "display_name is required (max 200 chars)".into(),
        ));
    }
    let now = chrono::Utc::now();
    let row = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let created = match core_db::plugin_distribution::get_publisher_by_identity(
            &mut tx.tx, org_id, &publisher,
        )
        .await
        {
            Ok(existing) => {
                if existing.status == "verified" {
                    return Err(ApiError::Conflict(format!(
                        "{publisher} is already verified by this org"
                    )));
                }
                core_db::plugin_distribution::set_publisher_status(
                    &mut tx.tx,
                    org_id,
                    existing.id,
                    "verified",
                    Some(actor),
                    now,
                )
                .await?;
                core_db::plugin_distribution::get_publisher_by_identity(
                    &mut tx.tx, org_id, &publisher,
                )
                .await?
            }
            Err(core_db::DbError::NotFound) => {
                let model = PluginPublisherVerification {
                    id: Uuid::now_v7(),
                    org_id,
                    publisher: publisher.clone(),
                    display_name: display_name.clone(),
                    status: "verified".into(),
                    verified_by: Some(actor),
                    verified_at: now,
                    revoked_at: None,
                    revoked_by: None,
                    note: req.note.clone().unwrap_or_default(),
                };
                model
                    .validate()
                    .map_err(|e| ApiError::Validation(e.to_string()))?;
                core_db::plugin_distribution::create_publisher_verification(&mut tx.tx, &model)
                    .await?;
                model
            }
            Err(e) => return Err(e.into()),
        };
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.publisher_verified".into(),
                entity_type: Some("plugin_publisher_verification".into()),
                entity_id: Some(created.id.to_string()),
                detail: serde_json::json!({
                    "publisher": publisher,
                    "display_name": display_name,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_PUBLISHER_VERIFIED,
            "plugin_publisher_verification",
            Some(created.id.to_string().as_str()),
            &serde_json::to_value(PluginPublisherVerified {
                verification_id: Some(created.id),
                publisher: publisher.clone(),
                display_name,
                verified_by: actor,
                verified_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, created))
    })
    .await?;
    Ok(Json(row.into()))
}

/// DELETE /api/plugins/distribution/publishers/{id} — withdraws the
/// vouch; the badge disappears and the row persists as history.
#[utoipa::path(operation_id = "plugin_distribution_publishers_revoke", delete, path = "/api/plugins/distribution/publishers/{id}", tag = "plugins", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = PublisherVerificationView), (status = 404)))]
pub async fn revoke_publisher(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PublisherVerificationView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_plugin_admin(&authed)?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let now = chrono::Utc::now();
    let existing = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id).await?;
        let row =
            core_db::plugin_distribution::list_publisher_verifications(&mut *tx, org_id, false)
                .await?
                .into_iter()
                .find(|v| v.id == id)
                .ok_or(ApiError::NotFound)?;
        tx.commit().await.map_err(ApiError::from)?;
        row
    };
    if existing.status == "revoked" {
        return Err(ApiError::Conflict("the vouch is already withdrawn".into()));
    }
    let revoked_publisher = existing.publisher.clone();
    let revoked_publisher_ref = revoked_publisher.clone();
    org_tx::with_org_tx(&state, org_id, actor, move |mut tx| async move {
        core_db::plugin_distribution::set_publisher_status(
            &mut tx.tx,
            org_id,
            id,
            "revoked",
            Some(actor),
            now,
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.publisher_revoked".into(),
                entity_type: Some("plugin_publisher_verification".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "publisher": revoked_publisher.clone() }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_PUBLISHER_REVOKED,
            "plugin_publisher_verification",
            Some(id.to_string().as_str()),
            &serde_json::to_value(PluginPublisherRevoked {
                verification_id: Some(id),
                publisher: revoked_publisher.clone(),
                revoked_by: actor,
                revoked_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let refreshed = core_db::plugin_distribution::get_publisher_by_identity(
        &mut tx,
        org_id,
        &revoked_publisher_ref,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(refreshed.into()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListRatingsQuery {
    #[schema(value_type = Option<String>)]
    pub source_url: Option<String>,
    #[schema(value_type = Option<String>)]
    pub artifact_digest: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RatingSummary {
    /// Aggregated over the org: count of ratings and the mean.
    pub count: i64,
    #[schema(value_type = Option<f64>)]
    pub average: Option<f64>,
    /// The caller's own rating on this target, when one exists.
    #[schema(value_type = Option<RatingMineView>)]
    pub mine: Option<RatingMineView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RatingMineView {
    pub rating: i16,
    pub comment: String,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RatingListView {
    pub ratings: Vec<PluginRating>,
    /// Aggregate for the requested target (or the whole org list when
    /// unfiltered — the aggregate then spans targets and is advisory).
    pub summary: RatingSummary,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PutRatingRequest {
    /// `marketplace_registry` | `github_repo` | `local_zip` |
    /// `skill_community` | `manual_upload`.
    pub source_kind: String,
    pub source_url: String,
    /// sha256 hex of the artifact release the rating pins.
    pub artifact_digest: String,
    pub rating: i16,
    #[schema(value_type = Option<String>)]
    pub comment: Option<String>,
}

/// GET /api/plugins/distribution/ratings — the org's ratings, optionally
/// narrowed to one artifact release.
#[utoipa::path(operation_id = "plugin_distribution_ratings_list", get, path = "/api/plugins/distribution/ratings", tag = "plugins", security(("bearer" = [])),
    params(("source_url" = Option<String>, Query), ("artifact_digest" = Option<String>, Query)),
    responses((status = 200, body = RatingListView)))]
pub async fn list_ratings(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<ListRatingsQuery>,
) -> ApiResult<Json<RatingListView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::plugin_distribution::list_ratings(
        &mut *tx,
        org_id,
        q.source_url.as_deref(),
        q.artifact_digest.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rating_view(rows, &authed.principal.user.id)))
}

fn rating_view(rows: Vec<core_domain::entities::PluginRating>, caller: &Uuid) -> RatingListView {
    let count = rows.len() as i64;
    let mean = if count == 0 {
        None
    } else {
        Some(rows.iter().map(|r| f64::from(r.rating)).sum::<f64>() / f64::from(count as u32))
    };
    let mine = rows
        .iter()
        .find(|r| r.rated_by == Some(*caller))
        .map(|r| RatingMineView {
            rating: r.rating,
            comment: r.comment.clone(),
            updated_at: r.updated_at,
        });
    RatingListView {
        summary: RatingSummary {
            count,
            average: mean,
            mine,
        },
        ratings: rows,
    }
}

/// PUT /api/plugins/distribution/ratings — upserts the caller's rating
/// on one artifact release (one per user per release; re-rate wins).
#[utoipa::path(operation_id = "plugin_distribution_ratings_put", put, path = "/api/plugins/distribution/ratings", tag = "plugins", security(("bearer" = [])),
    request_body = PutRatingRequest,
    responses((status = 200, body = RatingSummary), (status = 400), (status = 409)))]
pub async fn put_rating(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutRatingRequest>,
) -> ApiResult<Json<RatingSummary>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, plugins(Action::View))?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let source_kind = PluginSourceKind::parse(&req.source_kind)
        .ok_or_else(|| ApiError::Validation("unknown source_kind for the rated artifact".into()))?;
    let digest = req.artifact_digest.trim().to_lowercase();
    if !core_domain::entities::plugins::is_sha256_hex(&digest) {
        return Err(ApiError::Validation(
            "artifact_digest must be a sha256 hex digest".into(),
        ));
    }
    let source_url = req.source_url.trim().to_string();
    let source_url_query = source_url.clone();
    let digest_query = digest.clone();
    if source_url.is_empty() || source_url.len() > 500 {
        return Err(ApiError::Validation("source_url is required".into()));
    }
    let comment = req.comment.unwrap_or_default();
    if comment.len() > 2000 {
        return Err(ApiError::Validation(
            "comment must be at most 2000 chars".into(),
        ));
    }
    let now = chrono::Utc::now();
    let model = PluginRating {
        id: Uuid::now_v7(),
        org_id,
        source_kind,
        source_url: source_url.clone(),
        artifact_digest: digest.clone(),
        rating: req.rating,
        comment: comment.clone(),
        rated_by: Some(actor),
        rated_at: now,
        updated_at: now,
    };
    model
        .validate()
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let rating = model.rating;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::plugin_distribution::upsert_rating(&mut tx.tx, &model).await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: "plugin.rating_recorded".into(),
                entity_type: Some("plugin_rating".into()),
                entity_id: Some(model.id.to_string()),
                detail: serde_json::json!({
                    "source_kind": source_kind.as_str(),
                    "source_url": source_url,
                    "artifact_digest": digest,
                    "rating": rating,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::PLUGIN_RATING_RECORDED,
            "plugin_rating",
            Some(model.id.to_string().as_str()),
            &serde_json::to_value(PluginRatingRecorded {
                source_kind: source_kind.as_str().into(),
                source_url: source_url.clone(),
                artifact_digest: digest,
                rating,
                comment: comment.clone(),
                rated_by: actor,
                rated_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    let rows = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id).await?;
        let rows = core_db::plugin_distribution::list_ratings(
            &mut *tx,
            org_id,
            Some(&source_url_query),
            Some(&digest_query),
        )
        .await?;
        tx.commit().await.map_err(ApiError::from)?;
        rows
    };
    Ok(Json(rating_view(rows, &actor).summary))
}
