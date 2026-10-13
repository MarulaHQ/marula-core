//! Customer sign-off endpoints (Wave 43 track 2 lane 12, migration 0127).
//!
//! Purpose: the door for a witnessed signature capture. One route carries
//! the whole capture — the PNG bytes (base64), the typed signer name, the
//! device capture time, and the consent version the signer saw — and lands
//! the payload row, the evidence row, the domain event and the audit entry in
//! ONE org-bound transaction. A single route matters for offline replay: the
//! tech's queue holds exactly one op, so there is no partial capture where
//! bytes exist without their fact.
//!
//! Non-purpose: no multipart (JSON only), no image processing (the client
//! rasterises; the server validates magic bytes and content-addresses what
//! it was given), no stroke-point persistence (the PNG IS the payload — see
//! LANDING.md for why the vector is not a second record here), and no UPDATE
//! or DELETE of a sign-off: revocation appends to the trail, a correction is
//! a second sign-off.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, State},
    http::Extensions,
    response::Response,
    Json,
};
use core_domain::entities::{
    AttachmentKind, SignOff, SignOffConsent, SignOffRevocation, SignOffSignerKind, SignOffView,
};
use core_domain::events::{
    types, SignOffConsentCreated, SignOffCreated, SignOffRevoked, SignOffSuperseded,
};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Same permission shorthand the attachment surface uses: capturing a
/// signature is a ticket edit, not a new module.
pub fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

fn store_error(e: core_store::StoreError) -> ApiError {
    match e {
        core_store::StoreError::NotFound(_) => ApiError::NotFound,
        other => ApiError::Internal(anyhow::anyhow!(other.to_string())),
    }
}

/// Filename the payload row gets. A signature has no client filename — the
/// server mints a deterministic one so the mark is identifiable in a listing.
fn payload_filename(sign_off_id: Uuid) -> String {
    format!("signature-{sign_off_id}.png")
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

/// POST /api/tickets/{id}/sign-offs
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSignOffRequest {
    /// Base64 PNG of the trimmed, ink-only mark.
    pub image_base64: String,
    /// Who signed, as they typed it on the pad.
    pub signer_name: String,
    /// Device-clock reading when the stroke finished. Recorded verbatim.
    pub captured_at: chrono::DateTime<chrono::Utc>,
    /// Consent wording version the signer saw. The server rejects an
    /// unknown version so a stale pad cannot capture against wording the
    /// signer never read.
    pub consent_version: i32,
    /// Optional explicit consent id — pins the exact wording row when the
    /// pad fetched it before a concurrent revision landed.
    #[serde(default)]
    pub consent_id: Option<Uuid>,
    /// Set only when this capture corrects an earlier sign-off.
    #[serde(default)]
    pub supersedes_sign_off_id: Option<Uuid>,
    /// Client-minted replay anchor (offline queue). Enforced unique per org.
    pub idempotency_key: String,
    /// Reserved for the portal path; rejected today.
    #[serde(default)]
    pub signer_kind: Option<String>,
}

/// POST /api/sign-offs/{id}/revoke
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokeSignOffRequest {
    /// Why the sign-off no longer stands. Part of the audit record.
    pub reason: String,
}

/// POST /api/org/sign-off-consents
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateConsentRequest {
    pub body: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decodes the base64 PNG payload. Accepts the standard and URL-safe
/// alphabets because the two clients serialise the same mark differently
/// (web `btoa` output vs RN's encoder).
fn decode_payload(image_base64: &str) -> Result<Vec<u8>, ApiError> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE};
    use base64::Engine;

    let trimmed = image_base64.trim();
    if trimmed.is_empty() {
        return Err(ApiError::Validation("image_base64 is empty".into()));
    }
    // Both engines carry padding, and `btoa` output is standard alphabet while
    // the native writer emits the URL-safe form. `decode` tolerates missing
    // padding on both, so one call each covers what either client sends.
    STANDARD
        .decode(trimmed)
        .or_else(|_| URL_SAFE.decode(trimmed))
        .map_err(|_| ApiError::Validation("image_base64 is not valid base64".into()))
}

fn validate_signer_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::Validation("the signer's name is required".into()));
    }
    if name.chars().count() > 120 {
        return Err(ApiError::Validation("the signer's name is too long".into()));
    }
    Ok(name.to_string())
}

fn validate_reason(raw: &str) -> Result<String, ApiError> {
    let reason = raw.trim();
    if reason.is_empty() {
        return Err(ApiError::Validation(
            "a revocation reason is required".into(),
        ));
    }
    if reason.chars().count() > 500 {
        return Err(ApiError::Validation(
            "the revocation reason is too long".into(),
        ));
    }
    Ok(reason.to_string())
}

/// Consent wording the org controls. Rejects an unknown version so a stale
/// pad cannot frame the signer's agreement against wording they never saw.
async fn resolve_consent(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    consent_version: i32,
    consent_id: Option<Uuid>,
) -> Result<SignOffConsent, ApiError> {
    let current = core_db::sign_offs::current_consent(conn, org_id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => {
                ApiError::Validation("no consent wording is configured".into())
            }
            other => ApiError::from(other),
        })?;
    if let Some(id) = consent_id {
        if id != current.id {
            return Err(ApiError::Validation(
                "the consent wording changed; reload the pad before signing".into(),
            ));
        }
    }
    if consent_version != current.version {
        return Err(ApiError::Validation(
            "that consent version is no longer current; reload the pad before signing".into(),
        ));
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// POST /api/tickets/{id}/sign-offs — capture one signature.
///
/// The whole capture lands in one transaction: the payload row in the
/// content-addressed store, the evidence row, `sign_off.created`, and the
/// audit entry. The body's `idempotency_key` is the anchor the offline queue
/// replays on, so a retried queue op is a no-op rather than a second
/// signature.
#[utoipa::path(operation_id = "sign_offs_create", post,
    path = "/api/tickets/{id}/sign-offs", tag = "sign_offs",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateSignOffRequest,
    responses(
        (status = 200, body = SignOff),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted on this board"),
        (status = 404, description = "ticket or sign-off not found"),
        (status = 409, description = "already captured, revoked, or a version collision"),
        (status = 422, description = "validation failed")
    ))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateSignOffRequest>,
) -> ApiResult<Json<SignOff>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // The portal path is recorded, not shipped: the reserved signer kind is
    // rejected rather than silently accepted.
    let requested_kind = body
        .signer_kind
        .as_deref()
        .unwrap_or_else(|| SignOffSignerKind::Customer.as_str());
    let signer_kind: SignOffSignerKind = requested_kind
        .parse()
        .map_err(|e: core_domain::DomainError| ApiError::Validation(e.to_string()))?;
    if !signer_kind.is_supported() {
        return Err(ApiError::Validation(
            "portal signing is not enabled yet; the customer signs on the tech's device".into(),
        ));
    }
    let signer_name = validate_signer_name(&body.signer_name)?;
    let idempotency_key = body.idempotency_key.trim();
    if idempotency_key.is_empty() {
        return Err(ApiError::Validation(
            "an idempotency key is required".into(),
        ));
    }
    if idempotency_key.chars().count() > 120 {
        return Err(ApiError::Validation(
            "the idempotency key is too long".into(),
        ));
    }

    let sign_off_id = Uuid::now_v7();

    // Two capture paths, both first-class: the customer draws, or the customer
    // types their name to agree. A pad that only accepts ink excludes anyone
    // who cannot draw, so the empty-image case is a valid capture rather than
    // a validation failure.
    let typed_agreement = body.image_base64.trim().is_empty();
    let max_bytes = state.config.max_upload_bytes;
    let (filename, content_type, sha256_hex, payload_size, store_bytes) = if typed_agreement {
        // An audit line that says "none" is worth more than one that says
        // "" next to a zero, which reads like a failed read.
        (String::new(), String::new(), "none".to_string(), 0, None)
    } else {
        let bytes = decode_payload(&body.image_base64)?;
        if bytes.len() as i64 > max_bytes {
            return Err(ApiError::Validation(format!(
                "signature image exceeds the size cap ({} > {max_bytes} bytes)",
                bytes.len()
            )));
        }
        let head_len = bytes.len().min(core_domain::attachments::MAGIC_HEAD_LEN);
        let (filename, content_type) = core_domain::attachments::validate_upload(
            &payload_filename(sign_off_id),
            "image/png",
            &bytes[..head_len],
            bytes.len() as i64,
            max_bytes,
        )?;
        let sha256_hex = hex::encode(Sha256::digest(&bytes));
        let size = bytes.len() as i64;
        // The store takes Bytes; converting once means the transaction owns
        // the only copy rather than cloning a Vec per attempt.
        (
            filename,
            content_type,
            sha256_hex,
            size,
            Some(axum::body::Bytes::from(bytes)),
        )
    };
    let storage_key = core_store::attachment_key(org_id, &sha256_hex);
    let store = std::sync::Arc::clone(&state.store);

    let sign_off = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;

        // Pin the exact consent wording. Snapshot, not a live join, so an org
        // revising its wording can never rewrite what this signer agreed to.
        let consent =
            resolve_consent(&mut ctx.tx, org_id, body.consent_version, body.consent_id).await?;
        let consent_version = consent.version;
        let consent_text_snapshot = consent.body.clone();

        if let Some(original_id) = body.supersedes_sign_off_id {
            // A correction must name an older sign-off on the SAME ticket.
            let original_ticket =
                core_db::sign_offs::sign_off_ticket(&mut ctx.tx, org_id, original_id)
                    .await
                    .map_err(|e| match e {
                        core_db::DbError::NotFound => {
                            ApiError::Validation("the sign-off to correct does not exist".into())
                        }
                        other => ApiError::from(other),
                    })?;
            if original_ticket != id {
                return Err(ApiError::Validation(
                    "a correction must target a sign-off on the same ticket".into(),
                ));
            }
        }

        // Content-addressed dedup: reuse an object this org already holds.
        // Only the drawn path touches the store at all.
        let attachment_id = match &store_bytes {
            Some(bytes) => {
                let existing = core_db::attachments::find_existing_object_key(
                    &mut ctx.tx,
                    org_id,
                    &sha256_hex,
                )
                .await?;
                let existing_doc = core_db::document_attachments::find_existing_object_key(
                    &mut ctx.tx,
                    org_id,
                    &sha256_hex,
                )
                .await?;
                if existing.is_none() && existing_doc.is_none() {
                    store
                        .put(&storage_key, &content_type, bytes.clone())
                        .await
                        .map_err(store_error)?;
                }

                let attachment_id = Uuid::now_v7();
                // The returned row is not read back here: the payload's size
                // and digest were computed from the bytes we already hold,
                // and echoing the server's copy would only be a second
                // source of truth.
                core_db::attachments::insert(
                    &mut ctx.tx,
                    core_db::attachments::NewAttachment {
                        id: attachment_id,
                        org_id,
                        kind: AttachmentKind::SignOff,
                        ticket_id: Some(id),
                        ticket_comment_id: None,
                        email_message_id: None,
                        filename: &filename,
                        content_type: &content_type,
                        size_bytes: payload_size,
                        sha256: &sha256_hex,
                        storage_key: &storage_key,
                        // The tech is the uploader; the customer is the signer.
                        uploaded_by_user_id: Some(user_id),
                        uploaded_by_portal_user_id: None,
                    },
                )
                .await?;
                Some(attachment_id)
            }
            None => None,
        };

        let sign_off = core_db::sign_offs::insert_sign_off(
            &mut ctx.tx,
            core_db::sign_offs::NewSignOff {
                id: sign_off_id,
                org_id,
                ticket_id: id,
                attachment_id,
                typed_agreement,
                signer_name: &signer_name,
                signer_kind,
                witnessed_by_user_id: Some(user_id),
                captured_at: body.captured_at,
                consent_version,
                consent_text_snapshot: &consent_text_snapshot,
                supersedes_sign_off_id: body.supersedes_sign_off_id,
                idempotency_key,
            },
        )
        .await?;

        ctx.emit(
            types::SIGN_OFF_CREATED,
            "sign_off",
            Some(sign_off_id.to_string().as_str()),
            &serde_json::to_value(SignOffCreated {
                sign_off_id,
                ticket_id: id,
                attachment_id,
                typed_agreement,
                signer_name: sign_off.signer_name.clone(),
                signer_kind: sign_off.signer_kind.as_str().to_string(),
                witnessed_by_user_id: Some(user_id),
                captured_at: sign_off.captured_at,
                consent_version,
                sha256: sha256_hex.clone(),
                size_bytes: payload_size,
                supersedes_sign_off_id: body.supersedes_sign_off_id,
            })
            .unwrap_or_default(),
        )
        .await?;

        // A correction announces itself on the ORIGINAL's subscribers without
        // the original row ever being written.
        if let Some(original_id) = body.supersedes_sign_off_id {
            ctx.emit(
                types::SIGN_OFF_SUPERSEDED,
                "sign_off",
                Some(original_id.to_string().as_str()),
                &serde_json::to_value(SignOffSuperseded {
                    sign_off_id: original_id,
                    ticket_id: id,
                    corrected_by_sign_off_id: sign_off_id,
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "sign_off.created".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "sign_off_id": sign_off_id,
                    "attachment_id": attachment_id,
                    "typed_agreement": typed_agreement,
                    "signer_name": sign_off.signer_name,
                    "consent_version": consent_version,
                    "sha256": sha256_hex,
                    "size_bytes": payload_size,
                    "supersedes_sign_off_id": body.supersedes_sign_off_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, sign_off))
    })
    .await?;
    Ok(Json(sign_off))
}

/// GET /api/tickets/{id}/sign-offs — the ticket's sign-offs with payload
/// facts, revocation state and the correction relation. Renders a timeline
/// row without ever moving the mark's bytes.
#[utoipa::path(operation_id = "sign_offs_list", get,
    path = "/api/tickets/{id}/sign-offs", tag = "sign_offs",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = [SignOffView]),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted on this board"),
        (status = 404, description = "ticket not found")
    ))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<SignOffView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    let views = core_db::sign_offs::list_sign_off_views_for_ticket(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(views))
}

/// GET /api/sign-offs/{id}/payload — streams the mark's bytes. Staff-only and
/// audit-free by intent: reading evidence is not the same act as capturing
/// it, and the row's own trail is the record of decisions.
#[utoipa::path(operation_id = "sign_offs_payload", get,
    path = "/api/sign-offs/{id}/payload", tag = "sign_offs",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "the captured PNG bytes",
            content_type = "image/png"),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted on this board"),
        (status = 404, description = "sign-off not found")
    ))]
pub async fn payload(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(sign_off_id): Path<Uuid>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let payload = core_db::sign_offs::payload_for_sign_off(&mut tx, org_id, sign_off_id)
        .await
        .map_err(|e| match e {
            core_db::DbError::NotFound => ApiError::NotFound,
            other => ApiError::from(other),
        })?;
    let ticket_id = core_db::sign_offs::sign_off_ticket(&mut tx, org_id, sign_off_id)
        .await
        .map_err(ApiError::from)?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id)
        .await
        .map_err(ApiError::from)?;
    authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let obj = state
        .store
        .get(&payload.storage_key)
        .await
        .map_err(store_error)?;
    let builder = Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, payload.content_type)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", payload.filename),
        )
        .header(axum::http::header::CONTENT_LENGTH, obj.size_bytes)
        .header("x-marula-sha256", payload.sha256);
    builder
        .body(Body::from_stream(obj.stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
}

/// POST /api/tickets/{id}/sign-offs/{sign_off_id}/revoke
///
/// Appends one entry to the sign-off's revocation trail. The row and its
/// bytes are untouched — this is why a revoked signature reads as revoked
/// rather than as gone, which is the whole point of keeping evidence.
#[utoipa::path(operation_id = "sign_offs_revoke", post,
    path = "/api/tickets/{id}/sign-offs/{sign_off_id}/revoke", tag = "sign_offs",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("sign_off_id" = Uuid, Path)),
    request_body = RevokeSignOffRequest,
    responses(
        (status = 200, body = SignOffRevocation),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted on this board"),
        (status = 404, description = "ticket or sign-off not found"),
        (status = 409, description = "already captured, revoked, or a version collision"),
        (status = 422, description = "validation failed")
    ))]
pub async fn revoke(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, sign_off_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<RevokeSignOffRequest>,
) -> ApiResult<Json<SignOffRevocation>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let reason = validate_reason(&body.reason)?;

    let revocation = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let subject = core_db::sign_offs::sign_off_ticket(&mut ctx.tx, org_id, sign_off_id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => ApiError::NotFound,
                other => ApiError::from(other),
            })?;
        if subject != id {
            return Err(ApiError::NotFound);
        }
        // Already revoked: a second attempt is a data error, not a rewrite.
        if core_db::sign_offs::latest_revocation(&mut ctx.tx, sign_off_id)
            .await?
            .is_some()
        {
            return Err(ApiError::Conflict(
                "this sign-off is already revoked".into(),
            ));
        }
        let revocation = core_db::sign_offs::insert_revocation(
            &mut ctx.tx,
            Uuid::now_v7(),
            org_id,
            sign_off_id,
            &reason,
            user_id,
        )
        .await?;
        ctx.emit(
            types::SIGN_OFF_REVOKED,
            "sign_off",
            Some(sign_off_id.to_string().as_str()),
            &serde_json::to_value(SignOffRevoked {
                sign_off_id,
                revocation_id: revocation.id,
                ticket_id: id,
                reason: revocation.reason.clone(),
                seq: revocation.seq,
                revoked_by_user_id: user_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "sign_off.revoked".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "sign_off_id": sign_off_id,
                    "revocation_id": revocation.id,
                    "seq": revocation.seq,
                    "reason": revocation.reason,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, revocation))
    })
    .await?;
    Ok(Json(revocation))
}

/// GET /api/org/sign-off-consents/current — the wording a pad must show. The
/// pad fetches this before capture so the version it sends back is the one
/// the signer actually read.
#[utoipa::path(operation_id = "sign_off_consents_current", get,
    path = "/api/org/sign-off-consents/current", tag = "sign_offs",
    security(("bearer" = [])),
    responses(
        (status = 200, body = SignOffConsent),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted")
    ))]
pub async fn current_consent(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<SignOffConsent>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let consent = core_db::sign_offs::current_consent(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(consent))
}

/// POST /api/org/sign-off-consents — append the next consent version. The
/// version comes from the current max; existing sign-offs keep their own
/// snapshot, so revising the wording never rewrites a past agreement.
#[utoipa::path(operation_id = "sign_off_consents_create", post,
    path = "/api/org/sign-off-consents", tag = "sign_offs",
    security(("bearer" = [])),
    request_body = CreateConsentRequest,
    responses(
        (status = 200, body = SignOffConsent),
        (status = 401, description = "missing or invalid credentials"),
        (status = 403, description = "not permitted on this board"),
        (status = 404, description = "ticket or sign-off not found"),
        (status = 409, description = "already captured, revoked, or a version collision"),
        (status = 422, description = "validation failed")
    ))]
pub async fn create_consent(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(body): Json<CreateConsentRequest>,
) -> ApiResult<Json<SignOffConsent>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    // Org-controlled wording: admin-grade permission, not a ticket edit.
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Admin,
            action: Action::Edit,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let text = body.body.trim();
    if text.is_empty() {
        return Err(ApiError::Validation(
            "consent wording cannot be empty".into(),
        ));
    }
    if text.chars().count() > 2000 {
        return Err(ApiError::Validation("consent wording is too long".into()));
    }

    let consent = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let current = core_db::sign_offs::current_consent(&mut ctx.tx, org_id)
            .await
            .map_err(|e| match e {
                core_db::DbError::NotFound => {
                    ApiError::Validation("no consent wording is configured".into())
                }
                other => ApiError::from(other),
            })?;
        let consent = core_db::sign_offs::insert_consent(
            &mut ctx.tx,
            Uuid::now_v7(),
            org_id,
            current.version + 1,
            text,
            user_id,
        )
        .await?;
        ctx.emit(
            types::SIGN_OFF_CONSENT_CREATED,
            "sign_off_consent",
            Some(consent.id.to_string().as_str()),
            &serde_json::to_value(SignOffConsentCreated {
                consent_id: consent.id,
                version: consent.version,
                body: consent.body.clone(),
                created_by: user_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, consent))
    })
    .await?;
    Ok(Json(consent))
}
