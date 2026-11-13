//! One-time share links for vault material (Wave 19 lane 3, migration
//! 0096).
//!
//! Posture this module pins: the raw token exists EXACTLY once on the
//! wire — inside the create response (`SecretShareLinkIssued`) — and
//! the DB stores a SHA-256 hash only (the secret-handling posture of
//! `routes/secrets.rs` is binding: tokens share the "never in tables,
//! logs, events, or error envelopes" rule of secret plaintext). A link
//! is single-use by construction (one transactional `UPDATE` is the
//! exclusive gate), org-scoped (the claimer must carry the link's home
//! org membership, verified EXPLICITLY after the SECURITY DEFINER
//! hash→org resolve in 0096 — membership is verification, not a scope
//! grant), expiring by `expires_at`, and revocable. Every lifecycle
//! step rides `audit_events`; create + claim also emit domain events
//! (ids and titles only — never tokens, never secret material).
//!
//! Sharing and claiming use the SAME permission gate as the reveal
//! endpoints (a claim IS a reveal: `secrets:edit` for record subjects,
//! `passwords:edit` for legacy entries — verified per subject kind), plus
//! the same category-grant gate for restricted record chains.

use crate::auth::middleware::{authed, require_permission};
use crate::crypto;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::extract::Path;
use axum::extract::State;
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use core_domain::entities::{
    Actor, AuditEvent, SecretRevealField, SecretShareLink, SecretShareLinkIssued, ShareSubjectKind,
};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::AuthedUser;
use axum::http::Extensions;

// --- tokens -----------------------------------------------------------------

/// 32 bytes of OS randomness, URL-safe — unguessable by construction
/// (mirrors the scheduling-links token shape).
fn generate_token() -> String {
    use rand::TryRng;
    let mut bytes = [0_u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("OS CSPRNG unavailable");
    use base64::Engine as _;
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn share_permission(kind: ShareSubjectKind) -> Permission {
    match kind {
        ShareSubjectKind::SecretRecord => secrets(Action::Edit),
        ShareSubjectKind::Password => passwords(Action::Edit),
    }
}

fn secrets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Secrets,
        action,
    }
}

fn passwords(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Passwords,
        action,
    }
}

fn claim_path(token: &str) -> String {
    format!("/share?token={token}")
}

const LINK_MAX_LABEL: usize = 200;

// --- DTOs -------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CreateSecretShareLinkRequest {
    /// Optional provenance shown to link managers and the audit trail
    /// ("kiosk roll-out window", …). Secrets NEVER belong here — the
    /// audit stores detail, never material.
    #[serde(default)]
    pub label: String,
    /// RFC 3339 instant; the server typed-refuses dates in the past or
    /// absurdly far out (> 90 days — a share link is a bounded window,
    /// not a chronic credential).
    pub expires_at: DateTime<Utc>,
}

/// The unified claim response: the material exactly once (the claim
/// consumes the link). Mirrors `SecretReveal` / `PasswordReveal`
/// contents under one shape — `fields` carries `field_key` → plaintext.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ShareClaimMaterial {
    pub subject_kind: ShareSubjectKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub subject_id: Uuid,
    pub title: String,
    pub fields: Vec<SecretRevealField>,
}

// --- record subjects (native vault) -----------------------------------------

/// POST /api/secrets/records/{id}/share-links — mint a one-time share
/// link; the token is returned ONCE (re-issuing is the flow for mistakes).
#[utoipa::path(operation_id = "secrets_records_create_share_link", post, path = "/api/secrets/records/{id}/share-links", tag = "secrets", security(("bearer" = [])),
    request_body = CreateSecretShareLinkRequest,
    responses((status = 201, body = SecretShareLinkIssued),
              (status = 403, description = "category locked to your role"),
              (status = 404, description = "record not visible in scope"),
              (status = 422, description = "expiry/label rejected")))]
pub async fn create_record_share_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSecretShareLinkRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<SecretShareLinkIssued>)> {
    create_share_link_inner(
        &state,
        &extensions,
        id,
        &req,
        ShareSubjectKind::SecretRecord,
    )
    .await
}

/// GET /api/secrets/records/{id}/share-links — the lifecycle ledger
/// (metadata only; NEVER the tokens).
#[utoipa::path(operation_id = "secrets_records_list_share_links", get, path = "/api/secrets/records/{id}/share-links", tag = "secrets", security(("bearer" = [])),
    responses((status = 200, body = [SecretShareLink])))]
pub async fn list_record_share_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<SecretShareLink>>> {
    list_share_links_inner(&state, &extensions, id, ShareSubjectKind::SecretRecord).await
}

/// DELETE /api/secrets/records/{id}/share-links/{share_id} — revoke a
/// not-yet-consumed link; consumed links are immutable history.
#[utoipa::path(operation_id = "secrets_records_delete_share_link", delete, path = "/api/secrets/records/{id}/share-links/{share_id}", tag = "secrets", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("share_id" = Uuid, Path)),
    responses((status = 200), (status = 404)))]
pub async fn delete_record_share_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, share_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    revoke_share_link_inner(
        &state,
        &extensions,
        id,
        share_id,
        ShareSubjectKind::SecretRecord,
    )
    .await?;
    Ok(axum::http::StatusCode::OK)
}

// --- legacy password subjects ------------------------------------------------

/// POST /api/passwords/{id}/share-links — one-time link for the legacy
/// byte-compatible entry surface (the same token posture as records).
#[utoipa::path(operation_id = "passwords_create_share_link", post, path = "/api/passwords/{id}/share-links", tag = "passwords", security(("bearer" = [])),
    request_body = CreateSecretShareLinkRequest,
    responses((status = 201, body = SecretShareLinkIssued),
              (status = 404, description = "entry not visible in scope"),
              (status = 422, description = "expiry/label rejected")))]
pub async fn create_password_share_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateSecretShareLinkRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<SecretShareLinkIssued>)> {
    create_share_link_inner(&state, &extensions, id, &req, ShareSubjectKind::Password).await
}

/// GET /api/passwords/{id}/share-links
#[utoipa::path(operation_id = "passwords_list_share_links", get, path = "/api/passwords/{id}/share-links", tag = "passwords", security(("bearer" = [])),
    responses((status = 200, body = [SecretShareLink])))]
pub async fn list_password_share_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<SecretShareLink>>> {
    list_share_links_inner(&state, &extensions, id, ShareSubjectKind::Password).await
}

/// DELETE /api/passwords/{id}/share-links/{share_id}
#[utoipa::path(operation_id = "passwords_delete_share_link", delete, path = "/api/passwords/{id}/share-links/{share_id}", tag = "passwords", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("share_id" = Uuid, Path)),
    responses((status = 200), (status = 404)))]
pub async fn delete_password_share_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, share_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    revoke_share_link_inner(
        &state,
        &extensions,
        id,
        share_id,
        ShareSubjectKind::Password,
    )
    .await?;
    Ok(axum::http::StatusCode::OK)
}

// --- claim -------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ClaimShareLinkRequest {
    /// The one-time token, verbatim from the create response.
    pub token: String,
}

/// POST /api/secrets/share-links/claim — consume a link exactly once,
/// unseal its subject behind the org + permission + category gates, and
/// stamp the audit trail + `secret.share_link_claimed` event.
#[utoipa::path(operation_id = "secrets_share_links_claim", post, path = "/api/secrets/share-links/claim", tag = "secrets", security(("bearer" = [])),
    request_body = ClaimShareLinkRequest,
    responses((status = 200, body = ShareClaimMaterial),
              (status = 404, description = "unknown token / foreign org / expired"),
              (status = 409, description = "already claimed once / revoked"),
              (status = 403, description = "subject not revealable in this scope")))]
#[allow(clippy::type_complexity)]
pub async fn claim_share_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ClaimShareLinkRequest>,
) -> ApiResult<Json<ShareClaimMaterial>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    let claimer = authed.principal.user.id;
    let claimer_org = authed.principal.user.org_id;
    let token = req.token.trim();
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::Validation("token is invalid".into()));
    }
    let token_hash = hash_token(token);
    let secret_key = state.config.secret_key;
    let roles: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();

    // The resolve is org-agnostic (SECURITY DEFINER bridge): a hash
    // learns its home org; everything AFTER this stay-step is stamped
    // into the link's org with the org/user GUC and its RLS policies.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let resolved: Option<(Uuid, Uuid, String, Uuid, DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
        sqlx::query_as::<_, (Uuid, Uuid, String, Uuid, DateTime<Utc>, Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
            "SELECT r_id, r_org_id, r_subject_kind, r_subject_id, r_expires_at, r_revoked_at, r_consumed_at
             FROM resolve_secret_share_link($1)",
        )
        .bind(&token_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::from)?;
    let row = resolved.ok_or(ApiError::NotFound)?;
    let (share_id, link_org, kind, subject_id, expires_at, revoked_at, consumed_at) = row;
    if link_org != claimer_org {
        // DO NOT leak the link's existence others outside the org.
        return Err(ApiError::NotFound);
    }
    let subject_kind = if kind == "password" {
        ShareSubjectKind::Password
    } else {
        ShareSubjectKind::SecretRecord
    };
    // The claim IS a reveal — the same permission ladder applies before
    // any unsealing happens (per-subject gate).
    require_permission(&authed.principal, share_permission(subject_kind))?;

    if revoked_at.is_some() {
        return Err(ApiError::Conflict(
            "this share link was revoked before it was claimed".into(),
        ));
    }
    if consumed_at.is_some() {
        return Err(ApiError::Conflict(
            "this one-time link was already claimed".into(),
        ));
    }
    let now = Utc::now();
    if expires_at <= now {
        return Err(ApiError::Validation("this share link has expired".into()));
    }

    core_db::set_org_and_user(&mut tx, claimer_org, claimer)
        .await
        .map_err(ApiError::from)?;

    let (title, fields) = match subject_kind {
        ShareSubjectKind::Password => {
            let entry = core_db::passwords::get_password(&mut *tx, claimer_org, subject_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            let sealed = core_db::passwords::get_sealed(&mut *tx, claimer_org, subject_id).await?;
            let secret = crypto::open(&secret_key, &sealed.secret).map_err(unseal_err)?;
            let mut fields = vec![SecretRevealField {
                field_key: "secret".into(),
                value: String::from_utf8_lossy(&secret).into_owned(),
            }];
            if let Some(n) = &sealed.notes {
                let notes = crypto::open(&secret_key, n).map_err(unseal_err)?;
                fields.push(SecretRevealField {
                    field_key: "notes".into(),
                    value: String::from_utf8_lossy(&notes).into_owned(),
                });
            }
            (entry.title, fields)
        }
        ShareSubjectKind::SecretRecord => {
            let record = core_db::secret_vault::get_record(&mut tx, claimer_org, subject_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            let allowed = core_db::secret_vault::folder_access(
                &mut tx,
                claimer_org,
                &roles,
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
            let sealed =
                core_db::secret_vault::list_sealed_fields(&mut tx, claimer_org, subject_id)
                    .await
                    .map_err(ApiError::from)?;
            let mut fields = Vec::with_capacity(sealed.len());
            for (field_key, ciphertext) in sealed {
                let aad = crate::routes::secrets::field_aad(claimer_org, subject_id, &field_key);
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
            (record.title, fields)
        }
    };

    let claimed = core_db::share_links::claim(&mut tx, claimer_org, share_id, claimer)
        .await
        .map_err(ApiError::from)?;
    if !claimed {
        // Some racing claimer won between resolve and consume.
        return Err(ApiError::Conflict(
            "this one-time link was already claimed".into(),
        ));
    }

    audit_link(
        &mut tx,
        claimer_org,
        &claimer,
        "secret.share_link_claim",
        share_id,
        serde_json::json!({
            "subject_kind": subject_kind.as_str(),
            "subject_id": subject_id,
            "title": title,
        }),
    )
    .await?;
    emit_into(
        &mut tx,
        claimer_org,
        types::SECRET_SHARE_LINK_CLAIMED,
        "secret_share_link",
        Some(share_id.to_string().as_str()),
        &serde_json::json!({
            "share_id": share_id,
            "subject_kind": subject_kind.as_str(),
            "subject_id": subject_id,
            "title": title,
        }),
    )
    .await?;

    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ShareClaimMaterial {
        subject_kind,
        subject_id,
        title,
        fields,
    }))
}

// --- shared internals --------------------------------------------------------

fn unseal_err(e: crypto::CryptoError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("secret unsealing failed: {e}"))
}

async fn emit_into(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org_id: Uuid,
    event_type: &str,
    entity: &str,
    entity_id: Option<&str>,
    payload: &Value,
) -> ApiResult<()> {
    core_db::outbox::insert(
        tx,
        Uuid::now_v7(),
        org_id,
        event_type,
        entity,
        entity_id,
        payload,
    )
    .await?;
    Ok(())
}

async fn audit_link(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    actor_id: &Uuid,
    action: &str,
    share_id: Uuid,
    detail: serde_json::Value,
) -> ApiResult<()> {
    core_db::audit::insert(
        conn,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(*actor_id),
            action: action.into(),
            entity_type: Some("secret_share_link".into()),
            entity_id: Some(share_id.to_string()),
            detail,
            ip: None,
            occurred_at: Utc::now(),
        },
    )
    .await?;
    Ok(())
}

fn validate_link_window(expires_at: DateTime<Utc>) -> ApiResult<DateTime<Utc>> {
    let now = Utc::now();
    if expires_at <= now {
        return Err(ApiError::Validation("expiry must be in the future".into()));
    }
    if expires_at > now + chrono::Duration::days(90) {
        return Err(ApiError::Validation(
            "expiry more than 90 days out; share links stay bounded".into(),
        ));
    }
    Ok(expires_at)
}

/// Raw-binding-invariant keeper: label trim is done by the callers' DTO. The
/// subject exists? gate is kind-specific and runs INSIDE the org GUC.
async fn create_share_link_inner(
    state: &AppState,
    extensions: &Extensions,
    subject_id: Uuid,
    req: &CreateSecretShareLinkRequest,
    subject_kind: ShareSubjectKind,
) -> ApiResult<(axum::http::StatusCode, Json<SecretShareLinkIssued>)> {
    let authed = authed::<AuthedUser>(extensions)?;
    require_permission(&authed.principal, share_permission(subject_kind))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let label = req
        .label
        .trim()
        .chars()
        .take(LINK_MAX_LABEL)
        .collect::<String>();
    let expires_at = validate_link_window(req.expires_at)?;
    let roles: Vec<Uuid> = authed.roles.iter().map(|r| r.id).collect();
    let token = generate_token();
    let token_hash = hash_token(&token);

    let outcome = org_tx::with_org_tx(state, org_id, actor, |mut ctx| async move {
        let title = match subject_kind {
            ShareSubjectKind::Password => {
                let entry = core_db::passwords::get_password(&mut *ctx.tx, org_id, subject_id)
                    .await
                    .map_err(|_| ApiError::NotFound)?;
                entry.title
            }
            ShareSubjectKind::SecretRecord => {
                let record = core_db::secret_vault::get_record(&mut ctx.tx, org_id, subject_id)
                    .await
                    .map_err(|_| ApiError::NotFound)?;
                // The sharer re-arms the reveal gate: sharing IS
                // revealing, restricted chains need the same grant.
                let allowed = core_db::secret_vault::folder_access(
                    &mut ctx.tx,
                    org_id,
                    &roles,
                    record.folder_id,
                    true,
                )
                .await
                .map_err(ApiError::from)?;
                if !allowed {
                    return Err(ApiError::Forbidden(
                        "category locked to your role; request a grant to share from here".into(),
                    ));
                }
                record.title
            }
        };

        let share_id = Uuid::now_v7();
        core_db::share_links::insert(
            &mut *ctx.tx,
            &core_db::share_links::NewShareLink {
                id: share_id,
                org_id,
                subject_kind,
                subject_id,
                token_hash: token_hash.clone(),
                label: label.clone(),
                created_by: actor,
                expires_at,
            },
        )
        .await
        .map_err(|e| match e {
            core_db::DbError::Conflict(_) => {
                ApiError::Conflict("share token collision; retry — tokens are unique".into())
            }
            other => ApiError::from(other),
        })?;

        audit_link(
            &mut ctx.tx,
            org_id,
            &actor,
            "secret.share_link_create",
            share_id,
            serde_json::json!({
                "subject_kind": subject_kind.as_str(),
                "subject_id": subject_id,
                "label": label,
                "expires_at": expires_at.to_rfc3339(),
            }),
        )
        .await?;

        emit_into(
            &mut ctx.tx,
            org_id,
            types::SECRET_SHARE_LINK_CREATED,
            "secret_share_link",
            Some(share_id.to_string().as_str()),
            &serde_json::json!({
                "share_id": share_id,
                "subject_kind": subject_kind.as_str(),
                "subject_id": subject_id,
                "title": title,
                "expires_at": expires_at.to_rfc3339(),
            }),
        )
        .await?;

        Ok((ctx, {
            let link_path = claim_path(&token);
            SecretShareLinkIssued {
                token,
                link_path,
                id: share_id,
                expires_at,
                label: label.clone(),
            }
        }))
    })
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(outcome)))
}

async fn list_share_links_inner(
    state: &AppState,
    extensions: &Extensions,
    subject_id: Uuid,
    subject_kind: ShareSubjectKind,
) -> ApiResult<Json<Vec<SecretShareLink>>> {
    let authed = authed::<AuthedUser>(extensions)?;
    require_permission(&authed.principal, share_permission(subject_kind))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let links =
        core_db::share_links::list_for_subject(&mut *tx, org_id, subject_kind, subject_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(links))
}

async fn revoke_share_link_inner(
    state: &AppState,
    extensions: &Extensions,
    subject_id: Uuid,
    share_id: Uuid,
    subject_kind: ShareSubjectKind,
) -> ApiResult<()> {
    let authed = authed::<AuthedUser>(extensions)?;
    require_permission(&authed.principal, share_permission(subject_kind))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    org_tx::with_org_tx(state, org_id, actor, |ctx| async move {
        let mut ctx = ctx;
        // The link must belong to THIS subject (visibility through its
        // subject scope — revoking another subject's link is a 404).
        let link = core_db::share_links::get_by_id(&mut *ctx.tx, org_id, share_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
        if link.subject_id != subject_id || link.subject_kind != subject_kind {
            return Err(ApiError::NotFound);
        }
        if link.consumed_at.is_some() {
            return Err(ApiError::Conflict(
                "a claimed link is closed forever — the trail is the record".into(),
            ));
        }
        if link.revoked_at.is_some() {
            return Err(ApiError::Conflict("this link is already revoked".into()));
        }
        core_db::share_links::revoke(&mut ctx.tx, org_id, share_id).await?;
        audit_link(
            &mut ctx.tx,
            org_id,
            &actor,
            "secret.share_link_revoke",
            share_id,
            serde_json::json!({
                "subject_kind": subject_kind.as_str(),
                "subject_id": subject_id,
            }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(())
}
