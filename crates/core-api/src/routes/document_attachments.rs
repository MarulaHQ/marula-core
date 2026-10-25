//! Documents attachments (Wave 17 lane 13, migration 0089): the
//! documents surface over the core-store ObjectStore seam (2.14).
//!
//! Purpose: the SAME doctrine as the ticket attachment surface, pointed
//! at documents — every upload passes the core-domain allowlist gate AND
//! the scan gate (Wave 52 lane P6a; the key space is SHARED with ticket
//! attachments, so one object must never be admitted through this door and
//! held at the other), hashes to sha256, reuses already-stored objects per
//! org (content-addressed dedup — delete reclaims the object only when NO
//! row in EITHER table still references the key), and persists its metadata
//! row plus `document_attachment.added` in ONE org-bound transaction.
//! Downloads stream behind documents authz, or hand out a short-lived store
//! URL when the client asks for one (Wave 52 lane P7).
//!
//! Non-purpose: no multipart parsing, no versioning ties — attachments are
//! independent of the document's version history (deleting an attachment
//! never touches document_versions).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{Extensions, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use core_domain::policy::{Action, Module, Permission};
use serde::Serialize;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

fn documents(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Documents,
        action,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentAttachmentView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub document_id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// sha256 hex — integrity verifiable after download.
    pub sha256: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub uploaded_by: Option<Uuid>,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<core_db::document_attachments::DocumentAttachmentRow> for DocumentAttachmentView {
    fn from(r: core_db::document_attachments::DocumentAttachmentRow) -> Self {
        Self {
            id: r.id,
            org_id: r.org_id,
            document_id: r.document_id,
            filename: r.filename,
            content_type: r.content_type,
            size_bytes: r.size_bytes,
            sha256: r.sha256,
            uploaded_by: r.uploaded_by,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentAttachmentList {
    pub attachments: Vec<DocumentAttachmentView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentAttachmentDeleted {
    pub deleted: bool,
    /// Whether the backing object was reclaimed too (refcount hit 0).
    pub object_removed: bool,
}

/// POST /api/documents/{id}/attachments — raw octet-stream upload (same
/// header contract as the ticket surface: `x-filename` + declared
/// content-type verified against sniffed magic bytes).
#[utoipa::path(operation_id = "document_attachments_upload", post,
    path = "/api/documents/{id}/attachments", tag = "documents",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 200, body = DocumentAttachmentView),
              (status = 404, description = "document not visible in scope"),
              (status = 422, description = "validation failed")))]
pub async fn upload(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(document_id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<DocumentAttachmentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let max_bytes = state.config.max_upload_bytes;
    if body.len() as i64 > max_bytes {
        return Err(ApiError::Validation(format!(
            "attachment exceeds size cap ({} > {max_bytes} bytes)",
            body.len()
        )));
    }
    let raw_filename = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if raw_filename.trim().is_empty() {
        return Err(ApiError::Validation("x-filename header is required".into()));
    }
    let declared = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let head_len = body.len().min(core_domain::attachments::MAGIC_HEAD_LEN);
    let (filename, content_type) = core_domain::attachments::validate_upload(
        raw_filename,
        declared,
        &body[..head_len],
        body.len() as i64,
        max_bytes,
    )?;
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let sha256_hex = hex::encode(hasher.finalize());

    // The scan gate (Wave 52 lane P6a), before the transaction and before any
    // byte is stored: the key space here is SHARED with ticket attachments, so
    // one object must never be admitted through this door and held at the
    // other. A scanner that cannot answer refuses the upload outright.
    let verdict = crate::scan_gate::gate(&state, &body).await?;
    let status = verdict.status;
    let scanner_name = verdict.scanner;
    let detail = verdict.detail.clone();
    let storage_key = core_store::attachment_key(org_id, &sha256_hex);
    let store = std::sync::Arc::clone(&state.store);

    let gated = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Visibility gate: the document must exist in THIS org (RLS).
        core_db::documents::get_document(&mut *ctx.tx, org_id, document_id)
            .await
            .map_err(ApiError::from)?;

        if status == core_domain::entities::ScanStatus::Infected {
            let held = crate::scan_gate::quarantine(
                &mut ctx,
                store.as_ref(),
                crate::scan_gate::Rejection {
                    org_id,
                    sha256: &sha256_hex,
                    filename: &filename,
                    content_type: &content_type,
                    size_bytes: body.len() as i64,
                    bytes: body.clone(),
                    // Documents live in the org key space, not on a ticket.
                    ticket_id: None,
                    uploaded_by_user_id: Some(user_id),
                    verdict: &verdict,
                },
            )
            .await?;
            // The same event the ticket door emits, so an operator watching
            // the stream sees every held object whichever door it arrived at.
            ctx.emit(
                core_domain::events::types::ATTACHMENT_QUARANTINED,
                "attachment",
                Some(sha256_hex.as_str()),
                &serde_json::to_value(core_domain::events::AttachmentQuarantined {
                    sha256: sha256_hex.clone(),
                    filename: filename.clone(),
                    content_type: content_type.clone(),
                    size_bytes: body.len() as i64,
                    scanner: scanner_name.to_string(),
                    detail: detail.clone(),
                    ticket_id: None,
                    uploaded_by_user_id: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
            return Ok((ctx, crate::scan_gate::Gated::Quarantined(held)));
        }

        // Content-addressed dedup on the shared key space: consult BOTH
        // tables before putting bytes.
        let existing_doc = core_db::document_attachments::find_existing_object_key(
            &mut ctx.tx,
            org_id,
            &sha256_hex,
        )
        .await?;
        let existing_shared =
            core_db::attachments::find_existing_object_key(&mut ctx.tx, org_id, &sha256_hex)
                .await?;
        if existing_doc.is_none() && existing_shared.is_none() {
            store
                .put(&storage_key, &content_type, body.clone())
                .await
                .map_err(crate::routes::attachments::store_error)?;
        }
        // The verdict belongs to the object; the first admission is what
        // happened, so a second surface's upload does not rewrite it.
        core_db::attachment_scans::record_admitted(
            &mut ctx.tx,
            core_db::attachment_scans::NewScan {
                org_id,
                sha256: &sha256_hex,
                status,
                detail: detail.as_deref(),
                scanner: scanner_name,
                size_bytes: body.len() as i64,
                content_type: &content_type,
                filename: &filename,
                storage_key: &storage_key,
                ticket_id: None,
                uploaded_by_user_id: Some(user_id),
                uploaded_by_portal_user_id: None,
            },
        )
        .await?;

        let row = core_db::document_attachments::insert(
            &mut ctx.tx,
            core_db::document_attachments::NewDocumentAttachment {
                id: Uuid::now_v7(),
                org_id,
                document_id,
                filename: &filename,
                content_type: &content_type,
                size_bytes: body.len() as i64,
                sha256: &sha256_hex,
                storage_key: &storage_key,
                uploaded_by: Some(user_id),
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::DOCUMENT_ATTACHMENT_ADDED,
            "document_attachment",
            Some(row.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DocumentAttachmentAdded {
                attachment_id: row.id,
                org_id,
                document_id,
                filename: row.filename.clone(),
                content_type: row.content_type.clone(),
                size_bytes: row.size_bytes,
                sha256: row.sha256.clone(),
                uploaded_by: Some(user_id),
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
                action: "document_attachment.added".into(),
                entity_type: Some("document".into()),
                entity_id: Some(document_id.to_string()),
                detail: serde_json::json!({
                    "attachment_id": row.id,
                    "filename": row.filename,
                    "size_bytes": row.size_bytes,
                    "sha256": row.sha256,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, crate::scan_gate::Gated::Admitted(row)))
    })
    .await?;
    match gated {
        crate::scan_gate::Gated::Admitted(row) => Ok(Json(row.into())),
        crate::scan_gate::Gated::Quarantined(held) => Err(held.refusal()),
    }
}

/// GET /api/documents/{id}/attachments — newest-first list for one doc.
#[utoipa::path(operation_id = "document_attachments_list", get,
    path = "/api/documents/{id}/attachments", tag = "documents",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = DocumentAttachmentList)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(document_id): Path<Uuid>,
) -> ApiResult<Json<DocumentAttachmentList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The RLS read doubles as the visibility gate: a doc outside scope
    // surfaces as a typed 404, never an empty list faking success.
    core_db::documents::get_document(&mut *tx, org_id, document_id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::document_attachments::list_for_document(
        &mut tx,
        org_id,
        document_id,
        crate::routes::attachments::scanner_installed(&state),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(DocumentAttachmentList {
        attachments: rows.into_iter().map(Into::into).collect(),
    }))
}

/// GET /api/documents/{id}/attachments/{attachment_id}/download — streams
/// the stored object behind documents authz, or answers with a presigned URL
/// when the caller asks for one (`?presign=true`) and the store can sign it.
///
/// Same rule as the ticket attachment door: the stream is the contract and
/// the ticket is the optimisation, and no URL is minted for an object whose
/// scan verdict has not been read and admitted.
#[utoipa::path(operation_id = "document_attachments_download", get,
    path = "/api/documents/{id}/attachments/{attachment_id}/download", tag = "documents",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path),
        ("attachment_id" = Uuid, Path),
        ("presign" = Option<bool>, Query, description = "`true` asks for a download ticket instead of the bytes; the answer says whether a URL was minted"),
    ),
    responses(
        (status = 200, content(
            (Vec<u8> = "application/octet-stream"),
            (crate::routes::attachments::AttachmentDownloadTicket = "application/json"),
        )),
        (status = 409, description = "the object is held by the scan gate: neither a URL nor the bytes are issued"),
    ))]
pub async fn download(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((document_id, attachment_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<crate::routes::attachments::DownloadTicketQuery>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let attachment =
        core_db::document_attachments::get(&mut tx, org_id, document_id, attachment_id).await?;
    // One snapshot for the row and its verdict: a scan that escalated in
    // between cannot be missed by a download that already passed the gate.
    let admission =
        crate::routes::attachments::verdict_for(&mut tx, org_id, &attachment.sha256).await?;
    tx.commit().await.map_err(ApiError::from)?;

    // The same decision the ticket door makes, from the same function: held
    // and unscanned objects leave through neither a URL nor the stream.
    crate::routes::attachments::check_admission(
        admission,
        crate::routes::attachments::scanner_installed(&state),
    )?;

    if query.presign {
        let ticket = crate::routes::attachments::download_ticket(
            &state,
            admission,
            crate::routes::attachments::DownloadSubject {
                storage_key: &attachment.storage_key,
                filename: &attachment.filename,
                content_type: &attachment.content_type,
                size_bytes: attachment.size_bytes,
                sha256: &attachment.sha256,
            },
        )
        .await?;
        return Ok(Json(ticket).into_response());
    }

    let obj = state
        .store
        .get(&attachment.storage_key)
        .await
        .map_err(crate::routes::attachments::store_error)?;
    let content_disposition = format!(
        "attachment; filename=\"{}\"",
        attachment.filename.replace(['"', '\\'], "")
    );
    let builder = Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            attachment.content_type.clone(),
        )
        .header(axum::http::header::CONTENT_DISPOSITION, content_disposition)
        .header("x-marula-sha256", attachment.sha256.clone())
        .header(axum::http::header::CONTENT_LENGTH, obj.size_bytes);
    builder
        .body(Body::from_stream(obj.stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
}

/// DELETE /api/documents/{id}/attachments/{attachment_id} — author-gated
/// (the uploader, or an org owner/admin). The stored object is reclaimed
/// only when NO row in EITHER attachments table still references the same
/// content-addressed key.
#[utoipa::path(operation_id = "document_attachments_delete", delete,
    path = "/api/documents/{id}/attachments/{attachment_id}", tag = "documents",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("attachment_id" = Uuid, Path)),
    responses((status = 200, body = DocumentAttachmentDeleted),
              (status = 404, description = "attachment not visible in scope")))]
pub async fn delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((document_id, attachment_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentAttachmentDeleted>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let authorizes = authed.principal.is_owner_or_admin();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let store = std::sync::Arc::clone(&state.store);
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let row =
            core_db::document_attachments::get(&mut ctx.tx, org_id, document_id, attachment_id)
                .await
                .map_err(ApiError::from)?;
        if !authorizes && row.uploaded_by != Some(user_id) {
            return Err(ApiError::Forbidden(
                "only the uploader or an owner/admin can delete this attachment".into(),
            ));
        }
        let deleted =
            core_db::document_attachments::delete(&mut ctx.tx, org_id, document_id, attachment_id)
                .await?
                .expect("the pre-read proved the row exists");
        let remaining = core_db::document_attachments::count_other_refs(
            &mut ctx.tx,
            org_id,
            &deleted.storage_key,
            attachment_id,
        )
        .await?;
        let object_removed = if remaining == 0 {
            store.delete(&deleted.storage_key).await.is_ok()
        } else {
            false
        };
        ctx.emit(
            core_domain::events::types::DOCUMENT_ATTACHMENT_DELETED,
            "document_attachment",
            Some(attachment_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::DocumentAttachmentDeleted {
                attachment_id,
                document_id,
                object_removed,
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
                action: "document_attachment.deleted".into(),
                entity_type: Some("document".into()),
                entity_id: Some(document_id.to_string()),
                detail: serde_json::json!({
                    "attachment_id": attachment_id,
                    "filename": deleted.filename,
                    "object_removed": object_removed,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((
            ctx,
            DocumentAttachmentDeleted {
                deleted: true,
                object_removed,
            },
        ))
    })
    .await?;
    Ok(Json(outcome))
}
