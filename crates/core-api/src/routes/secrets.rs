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
use crate::instant;
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
use sqlx::PgConnection;
use uuid::Uuid;

fn secrets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Secrets,
        action,
    }
}

fn seal_error(e: crypto::CryptoError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("secret sealing failed: {e}"))
}

/// The closed field-key vocabulary (metadata entities never carry
/// plaintext after the write).
const FIELD_KEYS: [&str; 4] = ["secret", "notes", "username", "url"];

/// AAD coordinate binder: ciphertext bits cannot be replayed across
/// `(org, record, field_key)` positions; the AAD is never stored.
fn field_aad(org_id: Uuid, record_id: Uuid, field_key: &str) -> Vec<u8> {
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
        ctx.emit(
            types::SECRET_FOLDER_CREATED,
            "secret_folder",
            Some(folder.id.to_string().as_str()),
            &serde_json::json!({
                "folder_id": folder.id,
                "client_id": req.client_id,
                "slug": folder.slug,
                "restricted": folder.restricted,
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
    if req.slug.as_deref() == Some("") || req.label.as_deref() == Some("") {
        return Err(ApiError::Validation("slug/label cannot be empty".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let folder = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
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
        ctx.emit(
            types::SECRET_FOLDER_UPDATED,
            "secret_folder",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "folder_id": id,
                "slug": folder.slug,
                "restricted": folder.restricted,
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
            &serde_json::json!({ "folder_id": id, "slug": before.slug }),
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
            &serde_json::json!({
                "client_id": req.client_id,
                "forked_from": id,
                "slug": source_slug,
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSecretRecordRequest {
    /// NULL = org-scope record.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    pub folder_id: Uuid,
    pub kind: SecretRecordKind,
    pub title: String,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
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
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateSecretRecordRequest {
    pub title: String,
    /// absent = keep; null = clear; value = set.
    #[serde(default)]
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Matching keys rotate (sealed); `value: null` clears.
    #[serde(default)]
    pub fields: Vec<SecretFieldValue>,
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

/// POST /api/secrets/records — seals fields with coordinate AAD.
#[utoipa::path(operation_id = "secrets_records_create", post, path = "/api/secrets/records", tag = "secrets", security(("bearer" = [])),
    request_body = CreateSecretRecordRequest,
    responses((status = 200, body = SecretRecord),
              (status = 403, description = "restricted category gate denied"),
              (status = 409, description = "title already exists in this folder"),
              (status = 422, description = "validation error")))]
pub async fn create_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateSecretRecordRequest>,
) -> ApiResult<Json<SecretRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Create))?;
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
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
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
            &serde_json::json!({
                "record_id": record_id,
                "client_id": req.client_id,
                "folder_id": req.folder_id,
                "title": title,
            }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
}

/// PATCH /api/secrets/records/{id} — metadata + field rotation/clear.
#[utoipa::path(operation_id = "secrets_records_update", patch, path = "/api/secrets/records/{id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateSecretRecordRequest,
    responses((status = 200, body = SecretRecord),
              (status = 404, description = "record not visible in scope"),
              (status = 403, description = "restricted category gate denied"),
              (status = 409, description = "title collision"),
              (status = 422, description = "validation error")))]
pub async fn update_record(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSecretRecordRequest>,
) -> ApiResult<Json<SecretRecord>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, secrets(Action::Edit))?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let key = state.config.secret_key;
    // Seal BEFORE the tx so a crypto failure never consumes a write.
    let entries: Vec<(String, String)> = req
        .fields
        .iter()
        .map(|f| (f.field_key.clone(), f.value.clone()))
        .collect();
    let title = req.title.trim().to_string();
    if let Some((key, _)) = entries
        .iter()
        .find(|(k, _)| !FIELD_KEYS.contains(&k.as_str()))
    {
        return Err(ApiError::Validation(format!(
            "unknown field key '{}' (allowed: {})",
            key,
            FIELD_KEYS.join(", ")
        )));
    }
    let sealed = seal_fields(org_id, id, &key, &entries)?;
    let record = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
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
        let field_updates: Vec<(String, Option<Vec<u8>>)> =
            sealed.into_iter().map(|(k, v)| (k, Some(v))).collect();
        let record = core_db::secret_vault::update_record(
            &mut ctx.tx,
            org_id,
            id,
            &title,
            req.expires_at,
            &field_updates,
            user_id,
        )
        .await
        .map_err(ApiError::from)?;
        ctx.emit(
            types::SECRET_RECORD_UPDATED,
            "secret_record",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "record_id": id,
                "fields_rotated": !field_updates.is_empty(),
            }),
        )
        .await?;
        Ok((ctx, record))
    })
    .await?;
    Ok(Json(record))
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
            &serde_json::json!({ "record_id": id, "title": before.title }),
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
            &serde_json::json!({ "record_id": id, "title": record.title }),
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
            &serde_json::json!({
                "grant_id": grant.id,
                "role_id": req.role_id,
                "folder_id": req.folder_id,
                "can_read": grant.can_read,
                "can_edit": grant.can_edit,
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
            &serde_json::json!({ "role_id": q.role_id, "folder_id": q.folder_id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}
