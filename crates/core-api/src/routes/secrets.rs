//! Native secrets vault endpoints (ADR 0006, Wave 16 lane 3, migration
//! 0078). SecretRecord generalization beside the shipped PasswordEntry
//! surface (which stays): the database stores ONLY AES-256-GCM
//! ciphertext (`crypto::seal`); new ciphertexts bind an AAD of
//! `(org_id, record_id, field_key)` (`seal_with_aad` — coordinate
//! replay protection); plaintext exists exactly twice: in the write
//! request and in the audited reveal response. Metadata/list DTOs NEVER
//! carry ciphertext or plaintext; reveal additionally requires
//! `secrets:edit` (the role-gated UI posture of `/passwords/{id}/reveal`
//! stays binding) and emits `secret.revealed`. Secrets NEVER reach
//! error envelopes, OTEL attributes, or webhook/event payloads — event
//! payloads carry ids and titles only.
//!
//! Category trees: the 5 seeded defaults are IMMUTABLE
//! (`is_default_seed` guard — fork, never edit-in-place, ADR 0004);
//! admin customization forks into a client scope via the fork endpoint;
//! user-added folders are ordinary rows. Category-level RBAC
//! (`secret_category_grants`, role × folder) rides the financial:read
//! gate shape: unrestricted chains ride the matrix, restricted chains
//! deny without a passing grant row for one of the caller's roles.

use crate::auth::middleware::{authed, require_permission};
use crate::crypto;
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{
    LinkKind, Principal, ResolvedLink, SecretCategoryGrant, SecretFolder, SecretFolderKind,
    SecretRecord, SecretRecordKind, SecretReveal, SecretRevealField,
};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sqlx::PgConnection;
use std::collections::HashMap;
use std::sync::{OnceLock, PoisonError};
use std::time::Instant;
use uuid::Uuid;

// --- A1: per-org vault write-budget admission control (Wave 16 lane 9) ------
//
// The noisy-neighbor floor: every secret-record/folder/grant mutation on
// `/api/secrets/*` (plus the chunked bulk-ingestion worker path) consumes
// one seat of its ORG'S fixed 60-second budget, so a runaway bulk
// importer degrades only its own org and interactive vault users keep a
// guaranteed share.
//
// Wave 19 lane 1 made the budget cross-replica: the Wave 16 in-memory
// fixed-window counters remain the SINGLE-REPLICA fast path (per
// process, same posture as the migration/dispatch budgets), but every
// admitted write now also consumes a DURABLE seat in
// `vault_write_budget_windows` (migration 0094) whose `FOR UPDATE`-
// locked verdict is the authoritative decision on `ensure_vault_budget`
// (`crate::vault_scale`). Budget config stays
// `MARULA_VAULT_WRITE_BUDGET_PER_MIN`; `0` disables everywhere. The
// secrets-flow suites assert the durable layer end-to-end.

const VAULT_BUDGET_WINDOW_SECS: u64 = 60;

struct BudgetWindow {
    started: Instant,
    count: u32,
}

static VAULT_BUDGETS: OnceLock<std::sync::Mutex<HashMap<Uuid, BudgetWindow>>> = OnceLock::new();

/// One seat of the org's write budget; `limit == 0` disables (documented
/// escape hatch). A breach is the typed 429 with `Retry-After`.
pub fn check_vault_budget(state: &AppState, org_id: Uuid) -> ApiResult<()> {
    let limit_per_min = state.config.vault_write_budget_per_min;
    if limit_per_min == 0 {
        return Ok(());
    }
    let buckets = VAULT_BUDGETS.get_or_init(Default::default);
    let mut map = buckets.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    let window = map.entry(org_id).or_insert(BudgetWindow {
        started: now,
        count: 0,
    });
    if now.duration_since(window.started).as_secs() >= VAULT_BUDGET_WINDOW_SECS {
        window.started = now;
        window.count = 0;
    }
    window.count += 1;
    if window.count > limit_per_min {
        let elapsed = now.duration_since(window.started).as_secs();
        let retry_after = VAULT_BUDGET_WINDOW_SECS.saturating_sub(elapsed).max(1);
        return Err(ApiError::RateLimited {
            retry_after_secs: i64::try_from(retry_after).unwrap_or(i64::MAX),
        });
    }
    Ok(())
}

fn secrets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Secrets,
        action,
    }
}

fn seal_error(e: crypto::CryptoError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("secret sealing failed: {e}"))
}

/// Typed event payloads serialize straight into the wire value — the
/// contract goldens pin these structs (core-events contract.rs); no ad
/// hoc `json!` payloads on the vault's event path.
fn typed_event<T: serde::Serialize>(payload: &T) -> serde_json::Value {
    serde_json::to_value(payload).expect("typed secret event payload serializes")
}

/// The closed field-key vocabulary (metadata entities never carry
/// plaintext after the write).
const FIELD_KEYS: [&str; 4] = ["secret", "notes", "username", "url"];

/// AAD coordinate binder: ciphertext bits cannot be replayed across
/// `(org, record, field_key)` positions; the AAD is never stored.
pub(crate) fn field_aad(org_id: Uuid, record_id: Uuid, field_key: &str) -> Vec<u8> {
    format!("secret_field:{org_id}:{record_id}:{field_key}").into_bytes()
}

fn seal_fields(
    org_id: Uuid,
    record_id: Uuid,
    key: &crate::config::SecretKey,
    entries: &[(String, String)],
) -> ApiResult<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::with_capacity(entries.len());
    for (field_key, value) in entries {
        if !FIELD_KEYS.contains(&field_key.as_str()) {
            return Err(ApiError::Validation(format!(
                "unknown field key '{}' (allowed: {})",
                field_key,
                FIELD_KEYS.join(", ")
            )));
        }
        let sealed = crypto::seal_with_aad(
            key,
            value.as_bytes(),
            &field_aad(org_id, record_id, field_key),
        )
        .map_err(seal_error)?;
        out.push((field_key.clone(), sealed));
    }
    Ok(out)
}

/// The category read gate (financial:read gate shape): unrestricted
/// chains ride the already-gated matrix; restricted chains deny without
/// a passing grant row for one of the caller's roles.
async fn folder_visible(
    conn: &mut PgConnection,
    org_id: Uuid,
    principal: &Principal,
    folder_id: Uuid,
) -> ApiResult<bool> {
    let role_ids: Vec<Uuid> = principal.roles.iter().map(|r| r.id).collect();
    core_db::secret_vault::folder_access(conn, org_id, &role_ids, folder_id, false)
        .await
        .map_err(ApiError::from)
}

// ---------------------------------------------------------------------------
// Folder routes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct SecretFolderListQuery {
    /// Client-scope tree; omitted = org-scope tree.
    #[serde(default)]
    pub client_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSecretFolderRequest {
    /// NULL = org scope.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub slug: String,
    pub label: String,
    pub kind: SecretFolderKind,
    #[serde(default)]
    pub restricted: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateSecretFolderRequest {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub restricted: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ForkSecretFolderRequest {
    pub client_id: Uuid,
}

/// GET /api/secrets/folders — metadata-only category tree.
#[utoipa::path(operation_id = "secrets_folders_list", get, path = "/api/secrets/folders", tag = "secrets", security(("bearer" = [])),
    params(SecretFolderListQuery),
    responses((status = 200, body = [SecretFolder])))]
pub async fn list_folders(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<SecretFolderListQuery>,
) -> ApiResult<Json<Vec<SecretFolder>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let folders = match q.client_id {
        Some(client) => core_db::secret_vault::list_client_folders(&mut tx, client).await?,
        None => core_db::secret_vault::list_org_folders(&mut tx, org_id).await?,
    };
    let mut visible = Vec::with_capacity(folders.len());
    for folder in folders {
        if folder_visible(&mut tx, org_id, &authed.principal, folder.id).await? {
            visible.push(folder);
        }
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(visible))
}

#[utoipa::path(operation_id = "secrets_folders_create", post, path = "/api/secrets/folders", tag = "secrets", security(("bearer" = [])),
    request_body = CreateSecretFolderRequest,
    responses((status = 200, body = SecretFolder),
              (status = 409, description = "slug already exists in this scope"),
              (status = 422, description = "validation error")))]
pub async fn create_folder(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateSecretFolderRequest>,
) -> ApiResult<Json<SecretFolder>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Create))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let slug = req.slug.trim().to_lowercase();
    if slug.is_empty() || req.label.trim().is_empty() {
        return Err(ApiError::Validation("slug and label are required".into()));
    }
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let folder = core_db::secret_vault::create_folder(
            &mut ctx.tx,
            org_id,
            req.client_id,
            req.parent_id,
            Uuid::now_v7(),
            &slug,
            req.label.trim(),
            req.kind,
            false,
            req.restricted,
            user_id,
        )
        .await?;
        // A restricted chain is deny-by-default for EVERYONE — including
        // the admin who just created it. Ensure the owner/admin grants
        // land in the SAME transaction (never a self-lockout window).
        if folder.restricted {
            core_db::secret_vault::seed_preset_role_grants_for_restricted_folders(
                &mut ctx.tx,
                org_id,
                user_id,
            )
            .await?;
        }
        ctx.emit(
            types::SECRET_FOLDER_CREATED,
            "secret_folder",
            Some(folder.id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretFolderCreated {
                folder_id: folder.id,
                client_id: req.client_id,
                slug: folder.slug.clone(),
                restricted: folder.restricted,
            }),
        )
        .await?;
        Ok((ctx, folder))
    })
    .await
    .map(Json)
}

#[utoipa::path(operation_id = "secrets_folders_update", patch, path = "/api/secrets/folders/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateSecretFolderRequest,
    responses((status = 200, body = SecretFolder),
              (status = 404, description = "folder not visible in scope"),
              (status = 409, description = "the shipped default is immutable; fork instead")))]
pub async fn update_folder(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSecretFolderRequest>,
) -> ApiResult<Json<SecretFolder>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    if req.slug.as_deref() == Some("") || req.label.as_deref() == Some("") {
        return Err(ApiError::Validation("slug/label cannot be empty".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
    let folder = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::secret_vault::get_folder(&mut *ctx.tx, org_id, id).await?;
        // The FOLDER-level category gate. A restricted chain denies edits to
        // the folder object itself, not only to the records inside it.
        // Without this, a principal the category denies could clear
        // `restricted` here and then read the chain the gate exists to
        // protect; the same route also retitles and re-slugs the chain.
        if before.restricted {
            let allowed =
                core_db::secret_vault::folder_access(&mut ctx.tx, org_id, &role_ids, id, true)
                    .await
                    .map_err(ApiError::from)?;
            if !allowed {
                return Err(ApiError::Forbidden(
                    "restricted chain locked to your role; request a grant to change it".into(),
                ));
            }
        }
        let folder = core_db::secret_vault::update_folder(
            &mut ctx.tx,
            org_id,
            id,
            req.label.as_deref(),
            req.slug.as_deref(),
            req.restricted,
            user_id,
        )
        .await?;
        // Restricting a previously-open chain narrows it back to
        // deny-by-default in the same transaction that restores owner/
        // admin access (the seeding idempotently upserts the grants).
        if !before.restricted && folder.restricted {
            core_db::secret_vault::seed_preset_role_grants_for_restricted_folders(
                &mut ctx.tx,
                org_id,
                user_id,
            )
            .await?;
        }
        ctx.emit(
            types::SECRET_FOLDER_UPDATED,
            "secret_folder",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretFolderUpdated {
                folder_id: id,
                slug: folder.slug.clone(),
                restricted: folder.restricted,
            }),
        )
        .await?;
        Ok((ctx, folder))
    })
    .await?;
    Ok(Json(folder))
}

#[utoipa::path(operation_id = "secrets_folders_delete", delete, path = "/api/secrets/folders/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "folder not visible in scope"),
              (status = 409, description = "seed refusal or non-empty subtree")))]
pub async fn delete_folder(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Delete))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::secret_vault::get_folder(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::secret_vault::delete_folder(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::SECRET_FOLDER_DELETED,
            "secret_folder",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretFolderDeleted {
                folder_id: id,
                slug: before.slug,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// POST /api/secrets/folders/{id}/fork — fork-the-seed customization
/// (a NEW row graph in the client's scope, never edit-in-place, ADR 0004).
#[utoipa::path(operation_id = "secrets_folders_fork", post, path = "/api/secrets/folders/{id}/fork", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ForkSecretFolderRequest,
    responses((status = 200, body = [SecretFolder]),
              (status = 404, description = "source folder or client not visible in scope"),
              (status = 409, description = "a fork of that slug already exists")))]
pub async fn fork_folder(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ForkSecretFolderRequest>,
) -> ApiResult<Json<Vec<SecretFolder>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Create))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let created = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let source = core_db::secret_vault::get_folder(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let source_slug = source.slug.clone();
        let created =
            core_db::secret_vault::fork_folder(&mut ctx.tx, org_id, id, req.client_id, user_id)
                .await?;
        // A restricted source forks into a fresh row graph — the new
        // chain's grant rows don't exist yet; seat owner/admin before
        // the tx commits (same deny-by-default-with-admin posture).
        if created.iter().any(|f| f.restricted) {
            core_db::secret_vault::seed_preset_role_grants_for_restricted_folders(
                &mut ctx.tx,
                org_id,
                user_id,
            )
            .await?;
        }
        ctx.emit(
            types::SECRET_FOLDER_CREATED,
            "secret_folder",
            Some(
                created
                    .first()
                    .map(|f| f.id.to_string())
                    .as_deref()
                    .unwrap_or(""),
            ),
            &typed_event(&core_domain::events::SecretFolderForked {
                client_id: Some(req.client_id),
                forked_from: id,
                slug: source_slug,
            }),
        )
        .await?;
        Ok((ctx, created))
    })
    .await?;
    Ok(Json(created))
}

// ---------------------------------------------------------------------------
// Record routes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct SecretRecordListQuery {
    /// Client-scope records; omitted = org-scope.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SecretFieldValue {
    pub field_key: String,
    /// Plaintext on the WIRE ONLY: sealed before any persistence, never
    /// returned outside the audited reveal response.
    pub value: String,
}

/// PATCH field entry (A5, Wave 16 lane 9): `value: null` CLEARS the
/// sealed field (row deleted, audited via the record-updated event);
/// a value rotates the sealed ciphertext.
#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SecretFieldValuePatch {
    pub field_key: String,
    /// None (json null) clears the sealed field; Some rotates it.
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CreateSecretRecordRequest {
    /// NULL = org-scope record.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    pub folder_id: Uuid,
    pub kind: SecretRecordKind,
    pub title: String,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Rotation schedule (Wave 19 lane 3): the worker sweep reminds once
    /// past this instant (at-most-once per schedule).
    #[serde(default)]
    pub rotate_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Material fields — sealed with coordinate AAD before persistence.
    #[serde(default)]
    pub fields: Vec<SecretFieldValue>,
    /// Document-kind ciphertext-file pointer + hash (object storage).
    #[serde(default)]
    pub object_key: Option<String>,
    #[serde(default)]
    pub file_sha256: Option<String>,
    #[serde(default)]
    pub file_bytes: Option<i64>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct UpdateSecretRecordRequest {
    pub title: String,
    /// absent = keep; null = clear; value = set.
    #[serde(default)]
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Rotation schedule (Wave 19 lane 3): absent = keep; null = clear;
    /// instant = set (stamps last_rotated_at in the same write).
    #[serde(default)]
    pub rotate_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Matching keys rotate (sealed); `value: null` clears (A5).
    #[serde(default)]
    pub fields: Vec<SecretFieldValuePatch>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): the metadata-only record plus
/// the write-trail/replay metadata (mirrors PasswordMutationResponse).
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SecretRecordMutationResponse {
    #[serde(flatten)]
    pub record: SecretRecord,
    pub mutation: instant::MutationMeta,
}

/// GET /api/secrets/records — metadata-only vault rows.
#[utoipa::path(operation_id = "secrets_records_list", get, path = "/api/secrets/records", tag = "secrets", security(("bearer" = [])),
    params(SecretRecordListQuery),
    responses((status = 200, body = [SecretRecord])))]
pub async fn list_records(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<SecretRecordListQuery>,
) -> ApiResult<Json<Vec<SecretRecord>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let records = match q.client_id {
        Some(client) => {
            core_db::secret_vault::list_client_records(&mut tx, client, q.folder_id).await?
        }
        None => core_db::secret_vault::list_org_records(&mut tx, org_id, q.folder_id).await?,
    };
    let mut visible = Vec::with_capacity(records.len());
    for record in records {
        // The CATEGORY gate narrows restricted chains record-by-record.
        if folder_visible(&mut tx, org_id, &authed.principal, record.folder_id).await? {
            visible.push(record);
        }
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(visible))
}

/// GET /api/secrets/records/{id} — metadata only.
#[utoipa::path(operation_id = "secrets_records_get", get, path = "/api/secrets/records/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = SecretRecord),
              (status = 404, description = "record not visible in scope")))]
pub async fn get_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SecretRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let record = core_db::secret_vault::get_record(&mut tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    if !folder_visible(&mut tx, org_id, &authed.principal, record.folder_id).await? {
        return Err(ApiError::NotFound);
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(record))
}

/// POST /api/secrets/records — seals fields with coordinate AAD;
/// instant-first (IF1–IF3) when an idempotency key rides the request.
#[utoipa::path(operation_id = "secrets_records_create", post, path = "/api/secrets/records", tag = "secrets", security(("bearer" = [])),
    request_body = CreateSecretRecordRequest,
    responses((status = 200, body = SecretRecordMutationResponse),
              (status = 403, description = "restricted category gate denied"),
              (status = 409, description = "title already exists in this folder / idempotency key reused"),
              (status = 422, description = "validation error")))]
pub async fn create_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateSecretRecordRequest>,
) -> ApiResult<Json<SecretRecordMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Create))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let record_id = Uuid::now_v7();
    let key = state.config.secret_key;
    // Seal BEFORE the claim so a crypto failure never consumes a write.
    let entries: Vec<(String, String)> = req
        .fields
        .iter()
        .map(|f| (f.field_key.clone(), f.value.clone()))
        .collect();
    let sealed = seal_fields(org_id, record_id, &key, &entries)?;
    let object_key = match req.kind {
        SecretRecordKind::Document => req.object_key.clone(),
        _ => None,
    };
    let file_sha256 = match req.kind {
        SecretRecordKind::Document => req.file_sha256.clone(),
        _ => None,
    };
    let file_bytes = match req.kind {
        SecretRecordKind::Document => req.file_bytes,
        _ => None,
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let idem_key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // IF1 replay gate. Must precede any write so a retry replays
        // byte-identically and never double-applies.
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "secret_record",
                entity_id: None,
                key: idem_key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // The CATEGORY edit gate BEFORE any write (restricted chains deny).
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            req.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden(
                "category locked to your role; request a grant to write here".into(),
            ));
        }
        let record = core_db::secret_vault::create_record(
            &mut ctx.tx,
            org_id,
            req.client_id,
            req.folder_id,
            record_id,
            req.kind,
            &title,
            req.expires_at,
            req.rotate_at,
            object_key.as_deref(),
            file_sha256.as_deref(),
            file_bytes,
            &sealed,
            user_id,
        )
        .await?;
        ctx.emit(
            types::SECRET_RECORD_CREATED,
            "secret_record",
            Some(record_id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretRecordCreated {
                record_id,
                client_id: req.client_id,
                folder_id: req.folder_id,
                title: title.clone(),
            }),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "secret_record",
                entity_id: record_id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "secret_record",
                entity_id: record_id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&record).unwrap_or(serde_json::Value::Null),
                updated_at: record.created_at,
                key: idem_key.as_deref(),
                payload: &record,
                provenance: Default::default(),
            },
        )
        .await?;
        let response = SecretRecordMutationResponse { record, mutation };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/secrets/records/{id} — metadata + field rotation/clear
/// (`value: null` clears — A5); instant-first envelope (IF1–IF3).
#[utoipa::path(operation_id = "secrets_records_update", patch, path = "/api/secrets/records/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateSecretRecordRequest,
    responses((status = 200, body = SecretRecordMutationResponse),
              (status = 404, description = "record not visible in scope"),
              (status = 403, description = "restricted category gate denied"),
              (status = 409, description = "title collision / version conflict / idempotency key reused"),
              (status = 422, description = "validation error")))]
pub async fn update_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSecretRecordRequest>,
) -> ApiResult<Json<SecretRecordMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let key = state.config.secret_key;
    // Seal BEFORE the tx so a crypto failure never consumes a write.
    if let Some(f) = req
        .fields
        .iter()
        .find(|f| !FIELD_KEYS.contains(&f.field_key.as_str()))
    {
        return Err(ApiError::Validation(format!(
            "unknown field key '{}' (allowed: {})",
            f.field_key,
            FIELD_KEYS.join(", ")
        )));
    }
    let to_rotated: Vec<(String, String)> = req
        .fields
        .iter()
        .filter_map(|f| f.value.as_ref().map(|v| (f.field_key.clone(), v.clone())))
        .collect();
    let title = req.title.trim().to_string();
    let sealed = seal_fields(org_id, id, &key, &to_rotated)?;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let idem_key = req.instant.idempotency_key.clone();
    let provided_version = req.instant.version;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // IF1 replay gate first: a retry replays the stored response
        // without touching state.
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "secret_record",
                entity_id: Some(id),
                key: idem_key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let before_folded = serde_json::to_value(&before).unwrap_or(serde_json::Value::Null);
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            before.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden("category locked to your role".into()));
        }
        let field_updates: Vec<(String, Option<Vec<u8>>)> = req
            .fields
            .iter()
            .map(|f| {
                (
                    f.field_key.clone(),
                    f.value.as_ref().map(|_| {
                        sealed
                            .iter()
                            .find(|(k, _)| k == &f.field_key)
                            .map(|(_, v)| v.clone())
                            .expect("rotated field was sealed above")
                    }),
                )
            })
            .collect();
        let record = core_db::secret_vault::update_record(
            &mut ctx.tx,
            org_id,
            id,
            &title,
            req.expires_at,
            req.rotate_at,
            &field_updates,
            user_id,
        )
        .await
        .map_err(ApiError::from)?;
        ctx.emit(
            types::SECRET_RECORD_UPDATED,
            "secret_record",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretRecordUpdated {
                record_id: id,
                fields_rotated: !field_updates.is_empty(),
            }),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "secret_record",
                entity_id: id,
                version: provided_version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "secret_record",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(before_folded),
                after: serde_json::to_value(&record).unwrap_or(serde_json::Value::Null),
                updated_at: record.updated_at,
                key: idem_key.as_deref(),
                payload: &record,
                provenance: Default::default(),
            },
        )
        .await?;
        let response = SecretRecordMutationResponse { record, mutation };
        Ok((ctx, MutationOutcome::Applied(response)))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/secrets/records/{id} — records retire; their fields
/// cascade; the audit keeps the trail.
#[utoipa::path(operation_id = "secrets_records_delete", delete, path = "/api/secrets/records/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "record not visible in scope"),
              (status = 403, description = "restricted category gate denied")))]
pub async fn delete_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Delete))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            before.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden("category locked to your role".into()));
        }
        core_db::secret_vault::delete_record(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::SECRET_RECORD_DELETED,
            "secret_record",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretRecordDeleted {
                record_id: id,
                title: before.title,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// POST /api/secrets/records/{id}/reveal — the ONLY plaintext path,
/// mirroring `/passwords/{id}/reveal`: requires `secrets:edit`, runs
/// inside the org tx so the reveal event commits atomically with the
/// read, decrypts with coordinate AAD first (falling back to the legacy
/// seal for rows the deterministic importer forwarded) and the response
/// is the sole carrier of the decrypted fields.
#[utoipa::path(operation_id = "secrets_records_reveal", post, path = "/api/secrets/records/{id}/reveal", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = SecretReveal),
              (status = 403, description = "reveal requires secrets:edit (and a category grant on restricted chains)"),
              (status = 404, description = "record not visible in scope")))]
pub async fn reveal_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<SecretReveal>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let secret_key = state.config.secret_key;
    let reveal = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            record.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden(
                "category locked to your role; request a grant to reveal here".into(),
            ));
        }
        let sealed = core_db::secret_vault::list_sealed_fields(&mut ctx.tx, org_id, id)
            .await
            .map_err(ApiError::from)?;
        let mut fields = Vec::with_capacity(sealed.len());
        for (field_key, ciphertext) in sealed {
            // Coordinate-bound seal first; the deterministic importer's
            // forwarded rows keep the shipped passwords seal (no AAD)
            // and open through the legacy path.
            let aad = field_aad(org_id, id, &field_key);
            let plaintext = match crypto::open_with_aad(&secret_key, &ciphertext, &aad) {
                Ok(v) => Ok(v),
                Err(_) => crypto::open(&secret_key, &ciphertext),
            }
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("secret unsealing failed: {e}")))?;
            fields.push(SecretRevealField {
                field_key,
                value: String::from_utf8_lossy(&plaintext).into_owned(),
            });
        }
        ctx.emit(
            types::SECRET_REVEALED,
            "secret_record",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretRevealed {
                record_id: id,
                title: record.title.clone(),
            }),
        )
        .await?;
        Ok((
            ctx,
            SecretReveal {
                id,
                title: record.title,
                fields,
            },
        ))
    })
    .await?;
    Ok(Json(reveal))
}

// ---------------------------------------------------------------------------
// Document-kind file bytes (A4, Wave 16 lane 9): ciphertext PUT to the
// object store + audited reveal-file download. The record row carries
// ONLY the pointer (`object_key`) + `file_sha256` + `file_bytes` of the
// SEALED payload; plaintext exists exactly twice — in the PUT request and
// in the audited GET response.
// ---------------------------------------------------------------------------

/// The storage key for one org's sealed document payload,
/// content-addressed by the sha256 of the CIPHERTEXT (dedup-safe).
fn sealed_file_key(org_id: Uuid, ciphertext_sha256_hex: &str) -> String {
    format!("org/{org_id}/secrets/{ciphertext_sha256_hex}")
}

fn file_aad(org_id: Uuid, record_id: Uuid) -> Vec<u8> {
    format!("secret_file:{org_id}:{record_id}").into_bytes()
}

/// PUT /api/secrets/records/{id}/file — document-kind ONLY. Seals the
/// uploaded bytes (AAD-bound to the record coordinate), PUTs the
/// ciphertext to the object store behind the core-store S3/local seam,
/// and pins the pointer + hash + byte length on the record in the same
/// tx (event emitted in-commit).
#[utoipa::path(operation_id = "secrets_records_put_file", put, path = "/api/secrets/records/{id}/file", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content = String, content_type = "application/octet-stream"),
    responses((status = 200, body = SecretRecordMutationResponse),
              (status = 404, description = "record not visible in scope"),
              (status = 403, description = "restricted category gate denied"),
              (status = 422, description = "record is not document-kind / body over cap")))]
pub async fn put_record_file(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    body: axum::body::Bytes,
) -> ApiResult<Json<SecretRecordMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let max_bytes = state.config.max_upload_bytes;
    if body.len() as i64 > max_bytes {
        return Err(ApiError::Validation(format!(
            "file exceeds the {} MB upload cap",
            max_bytes / (1024 * 1024)
        )));
    }
    if body.is_empty() {
        return Err(ApiError::Validation("file body must not be empty".into()));
    }

    // --- the scan gate (Wave 52 lane P6d) ------------------------------
    // This note is here rather than in the doc comment above because that
    // comment is the pinned OpenAPI summary; editing it would move the pin.
    // The vault is the one door that seals before storing, so the scanner
    // runs on the PLAINTEXT here, before `seal_file_frame`. This door also
    // refuses instead of holding: the quarantine namespace stores bytes
    // UNSEALED, so "holding" a vault upload would put the plaintext of a
    // record the user marked secret into the one bucket copy that is not
    // encrypted. A refusal therefore writes nothing at all — no object, no
    // verdict row, no notification — and the client gets a 422 that says so.
    //
    // The record is read first for its title and kind. The title is what
    // names the file here (this door has no `x-filename` header and the
    // download is named after the record), and a non-document record is
    // refused before the org pays for a scan. The read is a preflight, not
    // an authorization gate: the write transaction below still owns the
    // authoritative kind and category checks.
    let title = {
        let mut preflight = org_tx::begin(&state, org_id, user_id).await?;
        let record = core_db::secret_vault::get_record(&mut preflight.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if record.kind != SecretRecordKind::Document {
            return Err(ApiError::Validation(
                "file bytes belong on document-kind records".into(),
            ));
        }
        // Dropped, not committed: nothing was written, and the read is over.
        record.title
    };
    let filename = if title.trim().is_empty() {
        format!("vault-document-{id}")
    } else {
        title
    };
    let plain_sha256 = {
        let mut hasher = sha2::Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let verdict = crate::scan_gate::gate(&state, &body).await?;
    if verdict.is_rejected() {
        // Nothing above this line wrote anything, so the refusal reports a
        // scan and an absence, not a held object.
        tracing::warn!(
            sha256 = %plain_sha256,
            scanner = verdict.scanner,
            "vault file refused by the scan gate; nothing stored"
        );
        return Err(verdict.refusal(&plain_sha256, &filename));
    }

    // Seal AFTER the gate: refused bytes are never sealed, and a crypto
    // failure still never consumes a write.
    let ciphertext =
        crypto::seal_file_frame(&state.config.secret_key, &body, &file_aad(org_id, id))
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("file sealing failed: {e}")))?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&ciphertext);
    let ct_sha256 = hex::encode(hasher.finalize());
    let storage_key = sealed_file_key(org_id, &ct_sha256);
    let ct_len = i64::try_from(ciphertext.len()).unwrap_or(i64::MAX);
    let store = std::sync::Arc::clone(&state.store);
    let scan_status = verdict.status;
    let scanner_name = verdict.scanner;
    let scan_detail = verdict.detail.clone();
    let plain_len = i64::try_from(body.len()).unwrap_or(i64::MAX);

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.kind != SecretRecordKind::Document {
            return Err(ApiError::Validation(
                "file bytes belong on document-kind records".into(),
            ));
        }
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            before.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden("category locked to your role".into()));
        }
        // Store first (content-addressed, last-writer-wins on identical
        // bytes is a no-op), then pin the pointer in-commit.
        store
            .put(
                &storage_key,
                "application/octet-stream",
                ciphertext.clone().into(),
            )
            .await
            .map_err(crate::routes::attachments::store_error)?;
        let record = core_db::secret_vault::set_file_pointer(
            &mut ctx.tx,
            org_id,
            id,
            &storage_key,
            &ct_sha256,
            ct_len,
            user_id,
        )
        .await
        .map_err(ApiError::from)?;
        // The verdict belongs to the PLAINTEXT, keyed by (org_id, sha256) on
        // the same table every other door writes, so "was this checked?" has
        // one answer per object. `storage_key` is where this upload's sealed
        // copy lives; the vault binds ciphertext to the record's AAD, so the
        // same plaintext on two records is two keys and this column keeps the
        // first admission's. First-wins: a second sighting does not rewrite
        // what the first scan said.
        core_db::attachment_scans::record_admitted(
            &mut ctx.tx,
            core_db::attachment_scans::NewScan {
                org_id,
                sha256: &plain_sha256,
                status: scan_status,
                detail: scan_detail.as_deref(),
                scanner: scanner_name,
                size_bytes: plain_len,
                content_type: "application/octet-stream",
                filename: &filename,
                storage_key: &storage_key,
                // A vault document belongs to no ticket.
                ticket_id: None,
                uploaded_by_user_id: Some(user_id),
                uploaded_by_portal_user_id: None,
            },
        )
        .await?;
        ctx.emit(
            types::SECRET_RECORD_UPDATED,
            "secret_record",
            Some(id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretRecordUpdated {
                record_id: id,
                fields_rotated: true,
            }),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "secret_record",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "secret_record",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&record).unwrap_or(serde_json::Value::Null),
                updated_at: record.updated_at,
                key: None,
                payload: &record,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied((record, mutation))))
    })
    .await?;
    let (record, mutation) = outcome.into_response()?;
    Ok(Json(SecretRecordMutationResponse { record, mutation }))
}

/// GET /api/secrets/records/{id}/file — the reveal-file path (document
/// kind ONLY): requires `secrets:edit` + the category gate (the same
/// posture as field reveal), decrypts the object-store ciphertext, and
/// emits `secret.revealed` in-commit with the read. The response is the
/// sole carrier of the plaintext bytes.
#[utoipa::path(operation_id = "secrets_records_get_file", get, path = "/api/secrets/records/{id}/file", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, content_type = "application/octet-stream"),
              (status = 404, description = "record or object not found"),
              (status = 403, description = "restricted category gate denied"),
              (status = 409, description = "record has no stored file")))]
pub async fn get_record_file(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Response> {
    use axum::response::Response;
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let store = std::sync::Arc::clone(&state.store);

    let unseal_key = state.config.secret_key;
    let (body, ct_sha256, content_len) =
        org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
            let record = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
            if record.kind != SecretRecordKind::Document {
                return Err(ApiError::Validation(
                    "file bytes belong on document-kind records".into(),
                ));
            }
            let (object_key, file_sha256) =
                match (record.object_key.as_deref(), record.file_sha256.as_deref()) {
                    (Some(k), Some(h)) => (k.to_string(), h.to_string()),
                    _ => return Err(ApiError::Conflict("record has no stored file".into())),
                };
            let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
            let allowed = core_db::secret_vault::folder_access(
                &mut ctx.tx,
                org_id,
                &role_ids,
                record.folder_id,
                true,
            )
            .await
            .map_err(ApiError::from)?;
            if !allowed {
                return Err(ApiError::Forbidden("category locked to your role".into()));
            }
            // Reveal event commits with the read (the provenance rail).
            ctx.emit(
                types::SECRET_REVEALED,
                "secret_record",
                Some(id.to_string().as_str()),
                &typed_event(&core_domain::events::SecretRevealed {
                    record_id: id,
                    title: record.title.clone(),
                }),
            )
            .await?;
            let obj = store.get(&object_key).await.map_err(store_error_fn)?;
            let mut stream = obj.stream;

            // Legacy Wave 16 files are single-envelope seal_with_aad
            // buffers; new PUTs (Wave 19 lane 1) store `[MRDV2]` chunked
            // frames. Branch on the FIRST stored chunk, letting only
            // the framed path stream — legacy buffers wholesale
            // (bounded by the PUT upload cap), framed streams
            // chunk by chunk (the objective: never a full-file buffer).
            use futures_util::StreamExt as _;
            let first: axum::body::Bytes = stream
                .next()
                .await
                .transpose()
                .map_err(store_error_fn)?
                .unwrap_or_default();
            let base_aad = file_aad(org_id, id);
            let body;
            let plain_len;
            if first.len() >= 5 && first[..5] == crypto::STREAM_MAGIC {
                // Framed `[MRDV2]`: the 25-byte header is the head of
                // `first`. Its plaintext_len is authenticated per-chunk
                // when decryption arrives; here it only pre-sizes the
                // Content-Length — a mismatched (forged) header fails
                // closed at the first chunk anyway (header is AAD of
                // every chunk), turning into a truncated response.
                let mut leftover = first.to_vec();
                while leftover.len() < 25 {
                    match stream.next().await {
                        Some(Ok(chunk)) => leftover.extend_from_slice(&chunk),
                        Some(Err(e)) => return Err(store_error_fn(e)),
                        None => {
                            return Err(ApiError::Internal(anyhow::anyhow!(
                                "framed file truncated at the header"
                            )));
                        }
                    }
                }
                plain_len = u64::from_be_bytes(
                    leftover[13..21]
                        .try_into()
                        .map_err(|_| ApiError::Validation("framed file malformed".into()))?,
                );
                body = FileBody::Framed {
                    decryptor: crypto::OpenStream::new(unseal_key, base_aad),
                    tail: leftover,
                    stream,
                };
            } else {
                // Legacy single-envelope: buffer + decrypt wholescale
                // (the Wave 16 posture exactly, unchanged).
                let mut ciphertext = first.to_vec();
                while let Some(chunk) = stream.next().await {
                    ciphertext.extend_from_slice(chunk.map_err(store_error_fn)?.as_ref());
                }
                let plain =
                    crypto::open_with_aad(&unseal_key, &ciphertext, &base_aad).map_err(|e| {
                        ApiError::Internal(anyhow::anyhow!("file unsealing failed: {e}"))
                    })?;
                plain_len = plain.len() as u64;
                body = FileBody::Legacy(plain);
            }
            let i64_len = i64::try_from(plain_len).unwrap_or(i64::MAX);
            Ok((ctx, (body, file_sha256, i64_len)))
        })
        .await?;
    let response = match body {
        FileBody::Legacy(plaintext) => Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
            .header("x-file-sha256", ct_sha256)
            .header(axum::http::header::CONTENT_LENGTH, content_len)
            .body(axum::body::Body::from(plaintext))
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("response build failed: {e}")))?,
        FileBody::Framed {
            decryptor,
            tail,
            stream,
        } => {
            let plaintext_stream = frame_stream(decryptor, tail, stream);
            Response::builder()
                .status(axum::http::StatusCode::OK)
                .header(axum::http::header::CONTENT_TYPE, "application/octet-stream")
                .header("x-file-sha256", ct_sha256)
                .header(axum::http::header::CONTENT_LENGTH, content_len)
                .body(axum::body::Body::from_stream(plaintext_stream))
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("response build failed: {e}")))?
        }
    };
    Ok(response)
}

/// The GET file body: legacy files decrypt wholesale (Wave 16 posture,
/// unchanged); framed `[MRDV2]` files stream chunk by chunk.
enum FileBody {
    /// Legacy seal_with_aad envelope — buffered plaintext.
    Legacy(Vec<u8>),
    /// `[MRDV2]` frame: streaming decryptor over the store's ciphertext
    /// stream. `tail` is the ciphertext left over in the first store
    /// chunk beyond the header. Content-Length came from the
    /// authenticated header; a forged header fails closed on the
    /// first chunk (it is AAD of every chunk) — the connection breaks
    /// rather than serving hardened body lengths.
    Framed {
        decryptor: crypto::OpenStream,
        tail: Vec<u8>,
        stream: std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<axum::body::Bytes, core_store::StoreError>>
                    + Send,
            >,
        >,
    },
}

fn frame_stream(
    decryptor: crypto::OpenStream,
    tail: Vec<u8>,
    stream: std::pin::Pin<
        Box<
            dyn futures_util::Stream<Item = Result<axum::body::Bytes, core_store::StoreError>>
                + Send,
        >,
    >,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send + 'static {
    struct FrameState {
        pending: Vec<u8>,
        stream: std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<axum::body::Bytes, core_store::StoreError>>
                    + Send,
            >,
        >,
        decryptor: crypto::OpenStream,
    }

    let state = FrameState {
        pending: tail,
        stream,
        decryptor,
    };
    {
        use futures_util::{stream, StreamExt as _};
        stream::unfold(state, |mut st| async move {
            loop {
                match st.decryptor.feed(&[]) {
                    Ok(None) => {}
                    Ok(Some(plain)) => {
                        return Some((Ok(axum::body::Bytes::from(plain)), st));
                    }
                    Err(e) => return Some((Err(stream_io_err(&e)), st)),
                }
                if !st.pending.is_empty() {
                    let tail = std::mem::take(&mut st.pending);
                    match st.decryptor.feed(&tail) {
                        Ok(None) => {}
                        Ok(Some(plain)) => {
                            return Some((Ok(axum::body::Bytes::from(plain)), st));
                        }
                        Err(e) => return Some((Err(stream_io_err(&e)), st)),
                    }
                    continue;
                }
                match st.stream.next().await {
                    Some(Ok(chunk)) => {
                        if let Err(e) = st.decryptor.feed(&chunk) {
                            return Some((Err(stream_io_err(&e)), st));
                        }
                    }
                    Some(Err(e)) => {
                        return Some((Err(std::io::Error::other(e.to_string())), st));
                    }
                    None => {
                        if let Err(e) = st.decryptor.finish() {
                            return Some((
                                Err(std::io::Error::other(format!(
                                    "vault file truncated or tampered at the frame tail: {e}"
                                ))),
                                st,
                            ));
                        }
                        return None;
                    }
                }
            }
        })
    }
}

fn stream_io_err(e: &crypto::CryptoError) -> std::io::Error {
    std::io::Error::other(format!("vault file decrypt failed: {e}"))
}

fn store_error_fn(e: core_store::StoreError) -> ApiError {
    crate::routes::attachments::store_error(e)
}

// ---------------------------------------------------------------------------
// Chunked bulk-secret ingestion (A2, Wave 16 lane 9). The WORKER path for
// one-shot migrations into the vault (e.g. exporting from a legacy
// password manager): the caller drives chunk by chunk; each ROW commits
// in its OWN transaction (per-row atomicity — a failed row never rolls
// back the chunk) behind a stable per-row idempotency key. The checkpoint
// story is therefore durable and exact: the idempotency keys live in
// Postgres, so a resumed chunk replays rows whose keys already committed
// (byte-identical, via the IF1 gate) and only APPLIES the remaining ones.
// Wave 19 lane 1 upgraded pacing from a fixed per-row sleep to a DURABLE
// token bucket (`vault_bulk_pacers`, migration 0094, rate =
// `vault_bulk_rows_per_sec`, refill & wait hints on the DB clock): a
// restarted driver resumes its pace instead of rebounding a fresh one,
// and replicas pace a single shared bucket. Pacing rides
// `pace_bulk_token` (crate::vault_scale) and every row consumes one seat
// of the org's A1 write budget.
// ---------------------------------------------------------------------------

/// Max records per bulk call — a chunk, never the whole job.
const BULK_CHUNK_CAP: usize = 100;

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct BulkSecretRecord {
    pub title: String,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Material fields — sealed with coordinate AAD before persistence.
    #[serde(default)]
    pub fields: Vec<SecretFieldValue>,
    /// Stable per-row idempotency key — THE resume checkpoint. Generate
    /// once per source row (e.g. the legacy row's stable id) and reuse it
    /// verbatim on every chunk retry.
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BulkSecretIngestRequest {
    /// NULL = org-scope records.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    pub folder_id: Uuid,
    pub kind: SecretRecordKind,
    /// The chunk: 1..=[`BULK_CHUNK_CAP`] rows.
    pub records: Vec<BulkSecretRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BulkRowOutcome {
    /// Applied in this call.
    Created,
    /// The row's idempotency key had already committed an identical row
    /// (a resumed chunk) — replayed, not re-applied.
    Replayed,
    /// The row was refused; see `reason`.
    Refused,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BulkRowResult {
    pub idempotency_key: String,
    pub outcome: BulkRowOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<Uuid>,
    /// Typed refusal reason on `refused` rows (validation/unique/key
    /// reuse); never carries secret material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BulkSecretIngestResponse {
    pub results: Vec<BulkRowResult>,
    /// Convenience counts for the driver loop.
    pub created: i64,
    pub replayed: i64,
    pub refused: i64,
    /// True when the caller should send another chunk (informational —
    /// the driver knows its own queue; the row ceiling is the pacing
    /// signal to stay under).
    pub rows_per_sec_ceiling: i64,
}

/// POST /api/secrets/records/bulk — the chunked worker path.
#[utoipa::path(operation_id = "secrets_records_bulk", post, path = "/api/secrets/records/bulk", tag = "secrets", security(("bearer" = [])),
    request_body = BulkSecretIngestRequest,
    responses((status = 200, body = BulkSecretIngestResponse),
              (status = 403, description = "restricted category gate denied"),
              (status = 422, description = "chunk over cap / validation error"),
              (status = 429, description = "org write budget exhausted")))]
pub async fn bulk_ingest(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<BulkSecretIngestRequest>,
) -> ApiResult<Json<BulkSecretIngestResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.records.is_empty() {
        return Err(ApiError::Validation("chunk must not be empty".into()));
    }
    if req.records.len() > BULK_CHUNK_CAP {
        return Err(ApiError::Validation(format!(
            "chunk exceeds the {BULK_CHUNK_CAP}-row cap; send more chunks"
        )));
    }
    for row in &req.records {
        if row.title.trim().is_empty() {
            return Err(ApiError::Validation("every row needs a title".into()));
        }
        if row.idempotency_key.trim().is_empty() {
            return Err(ApiError::Validation(
                "every row needs a stable idempotency_key".into(),
            ));
        }
        if row
            .fields
            .iter()
            .any(|f| !FIELD_KEYS.contains(&f.field_key.as_str()))
        {
            return Err(ApiError::Validation(format!(
                "unknown field key (allowed: {})",
                FIELD_KEYS.join(", ")
            )));
        }
    }
    // The CATEGORY edit gate ONCE before any row (restricted chains deny).
    {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed =
            core_db::secret_vault::folder_access(&mut tx, org_id, &role_ids, req.folder_id, true)
                .await
                .map_err(ApiError::from)?;
        tx.commit().await.map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden(
                "category locked to your role; request a grant to write here".into(),
            ));
        }
    }
    let key = state.config.secret_key;
    let mut results: Vec<BulkRowResult> = Vec::with_capacity(req.records.len());
    for row in req.records.iter() {
        // One durable token per row (Wave 19 lane 1): the token-bucket
        // state rides `vault_bulk_pacers`, so pacing survives driver
        // restarts and coordinates across replicas — every row INCLUDING
        // the first consumes a seat of both the pace bucket and the A1
        // write budget (the budget seat was consumed above).
        crate::vault_scale::pace_bulk_token(&state, authed.principal.user.org_id).await?;
        let record_id = Uuid::now_v7();
        let title = row.title.trim().to_string();
        let entries: Vec<(String, String)> = row
            .fields
            .iter()
            .map(|f| (f.field_key.clone(), f.value.clone()))
            .collect();
        // Seal BEFORE the tx (crypto failure = row refusal, never a write).
        let sealed = match seal_fields(org_id, record_id, &key, &entries) {
            Ok(s) => s,
            Err(e) => {
                results.push(BulkRowResult {
                    idempotency_key: row.idempotency_key.clone(),
                    outcome: BulkRowOutcome::Refused,
                    record_id: None,
                    reason: Some(format!("seal rejected: {e}")),
                });
                continue;
            }
        };
        let fingerprint = serde_json::to_value(row).unwrap_or(serde_json::Value::Null);
        let row_key = row.idempotency_key.clone();
        let folder_id = req.folder_id;
        let client_id = req.client_id;
        let kind = req.kind;
        let expires_at = row.expires_at;
        let outcome: Result<(Option<Uuid>, BulkRowOutcome), ApiError> =
            org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
                if let InstantClaim::Replay(stored) = instant::claim(
                    &mut ctx,
                    instant::ClaimSpec {
                        entity: "secret_record",
                        entity_id: None,
                        key: Some(row_key.as_str()),
                        fingerprint: &fingerprint,
                    },
                )
                .await?
                {
                    // Surface the COMMITTED row's id so the driver can
                    // correlate; the fresh probe id is discarded.
                    let committed = stored
                        .get("id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());
                    return Ok((ctx, (committed, BulkRowOutcome::Replayed)));
                }
                let record = core_db::secret_vault::create_record(
                    &mut ctx.tx,
                    org_id,
                    client_id,
                    folder_id,
                    record_id,
                    kind,
                    &title,
                    expires_at,
                    // Bulk importer rows carry no rotation schedule.
                    None,
                    None,
                    None,
                    None,
                    &sealed,
                    user_id,
                )
                .await
                .map_err(|e| match e {
                    core_db::DbError::Conflict(msg) => ApiError::Conflict(msg),
                    other => ApiError::from(other),
                })?;
                ctx.emit(
                    types::SECRET_RECORD_CREATED,
                    "secret_record",
                    Some(record_id.to_string().as_str()),
                    &typed_event(&core_domain::events::SecretRecordCreated {
                        record_id,
                        client_id,
                        folder_id,
                        title: title.clone(),
                    }),
                )
                .await?;
                let write_seq = instant::entity_seq(
                    &mut ctx,
                    instant::SeqSpec {
                        entity: "secret_record",
                        entity_id: record_id,
                        version: None,
                    },
                )
                .await?;
                instant::record(
                    &mut ctx,
                    instant::RecordSpec {
                        entity: "secret_record",
                        entity_id: record_id,
                        op: "create",
                        seq: write_seq,
                        before: None,
                        after: serde_json::to_value(&record).unwrap_or(serde_json::Value::Null),
                        updated_at: record.created_at,
                        key: Some(row_key.as_str()),
                        payload: &record,
                        provenance: Default::default(),
                    },
                )
                .await?;
                Ok((ctx, (Some(record_id), BulkRowOutcome::Created)))
            })
            .await;
        match outcome {
            Ok((committed_id, row_outcome)) => results.push(BulkRowResult {
                idempotency_key: row.idempotency_key.clone(),
                outcome: row_outcome,
                record_id: committed_id,
                reason: None,
            }),
            Err(e) => results.push(BulkRowResult {
                idempotency_key: row.idempotency_key.clone(),
                outcome: BulkRowOutcome::Refused,
                record_id: None,
                reason: Some(e.to_string()),
            }),
        }
    }
    let created = i64::try_from(
        results
            .iter()
            .filter(|r| r.outcome == BulkRowOutcome::Created)
            .count(),
    )
    .unwrap_or(i64::MAX);
    let replayed = i64::try_from(
        results
            .iter()
            .filter(|r| r.outcome == BulkRowOutcome::Replayed)
            .count(),
    )
    .unwrap_or(i64::MAX);
    let refused = i64::try_from(
        results
            .iter()
            .filter(|r| r.outcome == BulkRowOutcome::Refused)
            .count(),
    )
    .unwrap_or(i64::MAX);
    Ok(Json(BulkSecretIngestResponse {
        rows_per_sec_ceiling: i64::from(state.config.vault_bulk_rows_per_sec),
        results,
        created,
        replayed,
        refused,
    }))
}

// ---------------------------------------------------------------------------
// Record links (the cross-entity RecordLink seam — Wave 13 lane 5 pattern)
// ---------------------------------------------------------------------------

/// GET /api/secrets/records/{id}/links
#[utoipa::path(operation_id = "secrets_records_list_links", get, path = "/api/secrets/records/{id}/links", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ResolvedLink])))]
pub async fn list_record_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    crate::routes::documents::links_for(&state, &extensions, LinkKind::Secret, id).await
}

/// POST /api/secrets/records/{id}/links — typed labels via the seam;
/// broken targets ride the asset-docs semantic (never cascade-deleted).
#[utoipa::path(operation_id = "secrets_records_add_link", post, path = "/api/secrets/records/{id}/links", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = crate::routes::documents::CreateLinkRequest,
    responses((status = 200, body = [ResolvedLink]),
              (status = 404, description = "secret record or link target not visible in scope"),
              (status = 403, description = "restricted category gate denied")))]
pub async fn add_record_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<crate::routes::documents::CreateLinkRequest>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let to_kind = LinkKind::parse(&req.to_kind)
        .ok_or_else(|| ApiError::Validation(format!("unknown link kind '{}'", req.to_kind)))?;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let record = core_db::secret_vault::get_record(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let role_ids: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
        let allowed = core_db::secret_vault::folder_access(
            &mut ctx.tx,
            org_id,
            &role_ids,
            record.folder_id,
            true,
        )
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden("category locked to your role".into()));
        }
        crate::routes::documents::link_target_exists(&mut ctx.tx, org_id, to_kind, req.to_id)
            .await?;
        core_db::document_links::create_link(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            LinkKind::Secret,
            id,
            to_kind,
            req.to_id,
            user_id,
        )
        .await
        .map_err(ApiError::from)?;
        ctx.emit(
            types::LINK_ADDED,
            "link",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "from_kind": "secret",
                "from_id": id,
                "to_kind": to_kind.as_str(),
                "to_id": req.to_id,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    crate::routes::documents::links_for(&state, &extensions, LinkKind::Secret, id).await
}

// ---------------------------------------------------------------------------
// Category grants (role × folder — the financial:read gate shape)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SetSecretGrantRequest {
    pub role_id: Uuid,
    pub folder_id: Uuid,
    pub can_read: bool,
    pub can_edit: bool,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct DeleteSecretGrantQuery {
    pub role_id: Uuid,
    pub folder_id: Uuid,
}

/// GET /api/secrets/grants — the grant matrix reads for admins (Owner
/// or Admin only; grants are policy data, not vault contents).
#[utoipa::path(operation_id = "secrets_grants_list", get, path = "/api/secrets/grants", tag = "secrets", security(("bearer" = [])),
    responses((status = 200, body = [SecretCategoryGrant]),
              (status = 403, description = "grant administration is Owner/Admin only")))]
pub async fn list_grants(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<SecretCategoryGrant>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::View))?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "grant administration is Owner/Admin only".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let grants = core_db::secret_vault::list_grants(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(grants))
}

/// PUT /api/secrets/grants — upserts ONE role × folder grant row
/// (can_edit implies read). Owner/Admin only.
#[utoipa::path(operation_id = "secrets_grants_set", put, path = "/api/secrets/grants", tag = "secrets", security(("bearer" = [])),
    request_body = SetSecretGrantRequest,
    responses((status = 200, body = SecretCategoryGrant),
              (status = 403, description = "grant administration is Owner/Admin only"),
              (status = 404, description = "folder or role not visible in scope"),
              (status = 422, description = "validation error")))]
pub async fn set_grant(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SetSecretGrantRequest>,
) -> ApiResult<Json<SecretCategoryGrant>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "grant administration is Owner/Admin only".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let grant = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let grant = core_db::secret_vault::upsert_grant(
            &mut ctx.tx,
            org_id,
            req.role_id,
            req.folder_id,
            req.can_read,
            req.can_edit,
            user_id,
        )
        .await
        .map_err(ApiError::from)?;
        ctx.emit(
            types::SECRET_GRANT_UPDATED,
            "secret_grant",
            Some(grant.id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretGrantUpdated {
                grant_id: grant.id,
                role_id: req.role_id,
                folder_id: req.folder_id,
                can_read: grant.can_read,
                can_edit: grant.can_edit,
            }),
        )
        .await?;
        Ok((ctx, grant))
    })
    .await?;
    Ok(Json(grant))
}

/// DELETE /api/secrets/grants?role_id=&folder_id= — revoke one grant.
#[utoipa::path(operation_id = "secrets_grants_delete", delete, path = "/api/secrets/grants", tag = "secrets", security(("bearer" = [])),
    params(DeleteSecretGrantQuery),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "grant not found"),
              (status = 403, description = "grant administration is Owner/Admin only")))]
pub async fn delete_grant(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<DeleteSecretGrantQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    crate::vault_scale::ensure_vault_budget(&state, authed.principal.user.org_id).await?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "grant administration is Owner/Admin only".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::secret_vault::delete_grant(&mut ctx.tx, org_id, q.role_id, q.folder_id)
            .await
            .map_err(ApiError::from)?;
        ctx.emit(
            types::SECRET_GRANT_DELETED,
            "secret_grant",
            Some(q.role_id.to_string().as_str()),
            &typed_event(&core_domain::events::SecretGrantDeleted {
                role_id: q.role_id,
                folder_id: q.folder_id,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
