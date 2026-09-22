//! Password vault endpoints (docs-platform v1, Wave 11 lane 7, migration
//! 0053). The database stores ONLY AES-256-GCM ciphertext (core-api
//! `crypto::seal`, same posture as OIDC/TOTP secrets); plaintext exists
//! exactly twice: in the write request and in the audited reveal
//! response. Reveal requires `passwords:edit` and emits
//! `password.revealed` (provenance rail: who looked, when — provenance
//! queries never carry the secret).
//!
//! Mutations ride the instant-first contract (IF1–IF3). The vault is
//! deletion-capable (unlike documents): credentials retire, and their
//! history cascade-deletes with them — the audit trail keeps the events.

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
    LinkKind, PasswordEntry, PasswordHistoryEntry, PasswordReveal, ResolvedLink,
};

use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn passwords(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Passwords,
        action,
    }
}

fn seal_error(e: crypto::CryptoError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("secret sealing failed: {e}"))
}

/// Instant-first mutation response (IF2).
#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PasswordMutationResponse {
    #[serde(flatten)]
    pub entry: PasswordEntry,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CreatePasswordRequest {
    /// NULL = org-level vault entry.
    #[serde(default)]
    pub client_id: Option<Uuid>,
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub url: String,
    pub secret: String,
    /// Encrypted at rest; returned only through the reveal endpoint.
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Rotation schedule (Wave 19 lane 3): absent/null = never remide.
    #[serde(default)]
    pub rotate_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct UpdatePasswordRequest {
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub url: String,
    /// Absent = keep the current secret; present (even empty string is
    /// rejected) = rotate and archive the previous ciphertext to history.
    #[serde(default)]
    pub secret: Option<String>,
    /// absent = keep; null = clear; string = replace (encrypted at rest).
    #[serde(default)]
    pub notes: Option<Option<String>>,
    /// absent = keep; null = clear; value = set.
    #[serde(default)]
    pub expires_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Rotation schedule (Wave 19 lane 3): absent = keep; null = clear;
    /// instant = set (stamps last_rotated_at in the same write).
    #[serde(default)]
    pub rotate_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct PasswordListQuery {
    #[serde(default)]
    pub client_id: Option<Uuid>,
    /// `true` = org-level entries only.
    #[serde(default)]
    pub global: Option<bool>,
}

/// GET /api/passwords — vault browser rows; never carries secrets.
#[utoipa::path(operation_id = "passwords_list", get, path = "/api/passwords", tag = "passwords", security(("bearer" = [])),
    params(PasswordListQuery),
    responses((status = 200, body = [PasswordEntry])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<PasswordListQuery>,
) -> ApiResult<Json<Vec<PasswordEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entries = core_db::passwords::list_passwords(
        &mut *tx,
        org_id,
        &core_db::passwords::PasswordFilter {
            client_id: q.client_id,
            global_only: q.global.unwrap_or(false),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entries))
}

/// GET /api/passwords/{id}
#[utoipa::path(operation_id = "passwords_get", get, path = "/api/passwords/{id}", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PasswordEntry),
              (status = 404, description = "entry not visible in scope")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PasswordEntry>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entry = core_db::passwords::get_password(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entry))
}

/// POST /api/passwords — instant-first (IF1/IF2); seals the secret before
/// the claim so a crypto failure never consumes an idempotency key.
#[utoipa::path(operation_id = "passwords_create", post, path = "/api/passwords", tag = "passwords", security(("bearer" = [])),
    request_body = CreatePasswordRequest,
    responses((status = 200, body = PasswordMutationResponse),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "title/secret rejected")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreatePasswordRequest>,
) -> ApiResult<Json<PasswordMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::Create))?;
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if req.secret.is_empty() {
        return Err(ApiError::Validation("secret is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let secret_ciphertext =
        crypto::seal(&state.config.secret_key, req.secret.as_bytes()).map_err(seal_error)?;
    let notes_ciphertext = match &req.notes {
        Some(n) if !n.is_empty() => {
            Some(crypto::seal(&state.config.secret_key, n.as_bytes()).map_err(seal_error)?)
        }
        _ => None,
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "password",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        let entry = core_db::passwords::create_password(
            &mut *ctx.tx,
            org_id,
            id,
            req.client_id,
            &title,
            req.username.trim(),
            req.url.trim(),
            &secret_ciphertext,
            notes_ciphertext.as_deref(),
            req.expires_at,
            req.rotate_at,
            user_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "password",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::PASSWORD_CREATED,
            "password",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "password_id": id, "client_id": req.client_id, "title": title }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "password",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&entry).unwrap_or(serde_json::Value::Null),
                updated_at: entry.created_at,
                key: key.as_deref(),
                payload: &entry,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PasswordMutationResponse { entry, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/passwords/{id} — instant-first; a present `secret` rotates
/// the ciphertext and archives the previous one to history in-commit.
#[utoipa::path(operation_id = "passwords_update", patch, path = "/api/passwords/{id}", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdatePasswordRequest,
    responses((status = 200, body = PasswordMutationResponse),
              (status = 404, description = "entry not visible in scope"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "validation error")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdatePasswordRequest>,
) -> ApiResult<Json<PasswordMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::Edit))?;
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if req.secret.as_deref() == Some("") {
        return Err(ApiError::Validation(
            "secret cannot be empty; clear it by deleting the entry".into(),
        ));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let new_secret = match &req.secret {
        Some(s) => Some(crypto::seal(&state.config.secret_key, s.as_bytes()).map_err(seal_error)?),
        None => None,
    };
    let new_notes: Option<Option<Vec<u8>>> = match &req.notes {
        None => None,
        Some(None) => Some(None),
        Some(Some(n)) if n.is_empty() => Some(None),
        Some(Some(n)) => Some(Some(
            crypto::seal(&state.config.secret_key, n.as_bytes()).map_err(seal_error)?,
        )),
    };
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "password",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::passwords::get_password(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let entry = core_db::passwords::update_password(
            &mut ctx.tx,
            org_id,
            id,
            &title,
            req.username.trim(),
            req.url.trim(),
            new_secret.as_deref(),
            new_notes.as_ref().map(|n| n.as_deref()),
            req.expires_at,
            req.rotate_at,
            user_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "password",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::PASSWORD_UPDATED,
            "password",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "password_id": id, "secret_rotated": new_secret.is_some() }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "password",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&entry).unwrap_or(serde_json::Value::Null),
                updated_at: entry.updated_at,
                key: key.as_deref(),
                payload: &entry,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PasswordMutationResponse { entry, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/passwords/{id}/reveal — the ONLY plaintext path. Requires
/// `passwords:edit`, runs inside the org tx so the reveal event commits
/// atomically with the read, and the response is the sole carrier of the
/// decrypted secret.
#[utoipa::path(operation_id = "passwords_reveal", post, path = "/api/passwords/{id}/reveal", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = PasswordReveal),
              (status = 403, description = "reveal requires passwords:edit"),
              (status = 404, description = "entry not visible in scope")))]
pub async fn reveal(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PasswordReveal>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let secret_key = state.config.secret_key;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let entry = core_db::passwords::get_password(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let sealed = core_db::passwords::get_sealed(&mut *ctx.tx, org_id, id).await?;
        let secret = crypto::open(&secret_key, &sealed.secret)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("secret unsealing failed: {e}")))?;
        let notes = match &sealed.notes {
            Some(n) => Some(
                String::from_utf8_lossy(&crypto::open(&secret_key, n).map_err(|e| {
                    ApiError::Internal(anyhow::anyhow!("notes unsealing failed: {e}"))
                })?)
                .into_owned(),
            ),
            None => None,
        };
        ctx.emit(
            types::PASSWORD_REVEALED,
            "password",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "password_id": id, "title": entry.title }),
        )
        .await?;
        Ok((
            ctx,
            PasswordReveal {
                id,
                title: entry.title,
                secret: String::from_utf8_lossy(&secret).into_owned(),
                notes,
            },
        ))
    })
    .await?;
    Ok(Json(outcome))
}

/// DELETE /api/passwords/{id} — the vault is deletion-capable; history
/// cascade-deletes. Emits `password.deleted`.
#[utoipa::path(operation_id = "passwords_delete", delete, path = "/api/passwords/{id}", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "entry not visible in scope")))]
pub async fn delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::passwords::get_password(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::passwords::delete_password(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::PASSWORD_DELETED,
            "password",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "password_id": id, "title": before.title }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// GET /api/passwords/{id}/history — change ledger metadata (the
/// ciphertexts are retained for forensic re-derivation, never exposed).
#[utoipa::path(operation_id = "passwords_history", get, path = "/api/passwords/{id}/history", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [PasswordHistoryEntry])))]
pub async fn history(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<PasswordHistoryEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let entries = core_db::passwords::list_history(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entries))
}

/// GET /api/passwords/{id}/links
#[utoipa::path(operation_id = "passwords_list_links", get, path = "/api/passwords/{id}/links", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ResolvedLink])))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    crate::routes::documents::links_for(&state, &extensions, LinkKind::Password, id).await
}

/// POST /api/passwords/{id}/links
#[utoipa::path(operation_id = "passwords_add_link", post, path = "/api/passwords/{id}/links", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = crate::routes::documents::CreateLinkRequest,
    responses((status = 200, body = [ResolvedLink]),
              (status = 404, description = "password or link target not visible in scope")))]
pub async fn add_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<crate::routes::documents::CreateLinkRequest>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, passwords(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let to_kind = LinkKind::parse(&req.to_kind)
        .ok_or_else(|| ApiError::Validation(format!("unknown link kind '{}'", req.to_kind)))?;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::passwords::get_password(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        crate::routes::documents::link_target_exists(&mut ctx.tx, org_id, to_kind, req.to_id)
            .await?;
        let link = core_db::document_links::create_link(
            &mut *ctx.tx, org_id, Uuid::now_v7(), LinkKind::Password, id, to_kind, req.to_id, user_id,
        )
        .await?;
        ctx.emit(
            types::LINK_ADDED,
            "link",
            Some(link.id.to_string().as_str()),
            &serde_json::json!({ "from_kind": "password", "from_id": id, "to_kind": to_kind.as_str(), "to_id": req.to_id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    crate::routes::documents::links_for(&state, &extensions, LinkKind::Password, id).await
}
