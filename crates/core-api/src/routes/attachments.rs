//! Attachment endpoints (TASKLIST 2.14): upload, list, stream-download,
//! author-gated delete.
//!
//! Purpose: the ONLY door for binary objects into Meridian. Every upload
//! passes the core-domain allowlist gate (size cap + magic bytes vs
//! declared type vs extension), hashes to sha256, reuses already-stored
//! objects per org (content-addressed dedup), and persists its metadata
//! row plus `attachment.added` in ONE org-bound transaction. Downloads
//! stream through this API behind authz — no public/presigned URLs in v1.
//!
//! Non-purpose: no multipart parsing (single raw octet-stream bodies only,
//! filename/content-type carried as headers), no virus scanning (email
//! path scans upstream via `core-mail::ScanProvider`), no image
//! processing, no quota accounting.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{Extensions, HeaderMap},
    response::Response,
    Json,
};
use core_domain::entities::Attachment;
use core_domain::events::{types, AttachmentAdded, AttachmentDeleted};
use core_domain::policy::{Action, BoardOp, Module, Permission};
/// Tickets-module permission shorthand (mirrors the sibling lane files).
pub fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) fn store_error(e: core_store::StoreError) -> ApiError {
    match e {
        core_store::StoreError::NotFound(_) => ApiError::NotFound,
        other => {
            tracing::error!(error = %other, "object store failure");
            ApiError::Internal(anyhow::anyhow!(other.to_string()))
        }
    }
}

/// Metadata returned by upload; identical to the entity minus nothing —
/// the client needs every field to render the paperclip row.
pub type UploadResponse = Attachment;

/// POST /api/tickets/{id}/attachments — stores one file against a ticket
/// (linked to a comment later by the composer submit).
///
/// Headers: `x-filename` (required), `content-type` (declared; verified
/// against sniffed magic bytes). Body: raw bytes, capped by
/// `MARULA_MAX_UPLOAD_MB` (default 25 MB).
#[utoipa::path(operation_id = "attachments_upload", post,
    path = "/api/tickets/{id}/attachments", tag = "attachments",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses((status = 200, body = Attachment)))]
pub async fn upload(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> ApiResult<Json<UploadResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::Edit))?;
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
    let storage_key = core_store::attachment_key(org_id, &sha256_hex);
    let store = std::sync::Arc::clone(&state.store);

    let attachment = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;

        // Content-addressed dedup: skip the PUT when this org already
        // holds these exact bytes; rows still get their own identity.
        let existing =
            core_db::attachments::find_existing_object_key(&mut ctx.tx, org_id, &sha256_hex)
                .await?;
        if existing.is_none() {
            store
                .put(&storage_key, &content_type, body.clone())
                .await
                .map_err(store_error)?;
        }

        let attachment = core_db::attachments::insert(
            &mut ctx.tx,
            core_db::attachments::NewAttachment {
                id: Uuid::now_v7(),
                org_id,
                kind: core_domain::entities::AttachmentKind::TicketComment,
                ticket_id: Some(id),
                ticket_comment_id: None,
                email_message_id: None,
                filename: &filename,
                content_type: &content_type,
                size_bytes: body.len() as i64,
                sha256: &sha256_hex,
                storage_key: &storage_key,
                uploaded_by_user_id: Some(user_id),
                uploaded_by_portal_user_id: None,
            },
        )
        .await?;
        ctx.emit(
            types::ATTACHMENT_ADDED,
            "attachment",
            Some(attachment.id.to_string().as_str()),
            &serde_json::to_value(AttachmentAdded {
                attachment_id: attachment.id,
                kind: attachment.kind.as_str().to_string(),
                ticket_id: Some(id),
                ticket_comment_id: None,
                email_message_id: None,
                filename: attachment.filename.clone(),
                content_type: attachment.content_type.clone(),
                size_bytes: attachment.size_bytes,
                sha256: attachment.sha256.clone(),
                uploaded_by_user_id: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, attachment))
    })
    .await?;

    Ok(Json(attachment))
}

/// GET /api/tickets/{id}/attachments — every attachment on one ticket.
#[utoipa::path(operation_id = "attachments_list_for_ticket", get,
    path = "/api/tickets/{id}/attachments", tag = "attachments",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [Attachment])))]
pub async fn list_for_ticket(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<Attachment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    let items = core_db::attachments::list_for_ticket(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(items))
}

/// GET /api/tickets/{id}/comments/{comment_id}/attachments — one comment's
/// attachments.
#[utoipa::path(operation_id = "attachments_list_for_comment", get,
    path = "/api/tickets/{id}/comments/{comment_id}/attachments", tag = "attachments",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("comment_id" = Uuid, Path)),
    responses((status = 200, body = [Attachment])))]
pub async fn list_for_comment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, comment_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Vec<Attachment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    let items = core_db::attachments::list_for_comment(&mut *tx, org_id, comment_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(items))
}

/// GET /api/attachments/{id}/download — streams the stored bytes through
/// the API behind board-view authz (no public URLs in v1). Integrity is
/// verifiable against the recorded sha256 (`x-marula-sha256` header).
#[utoipa::path(operation_id = "attachments_download", get,
    path = "/api/attachments/{id}/download", tag = "attachments",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = Vec<u8>, content_type = "application/octet-stream")))]
pub async fn download(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let attachment = match core_db::attachments::get(&mut *tx, org_id, id).await {
        Ok(a) => a,
        Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
        Err(e) => return Err(e.into()),
    };
    if let Some(ticket_id) = attachment.ticket_id {
        let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id).await?;
        authorize_board_op(&mut tx, &authed.principal, ticket.board_id, BoardOp::View).await?;
    } else if !authed.principal.is_owner_or_admin() && attachment.email_message_id.is_none() {
        return Err(ApiError::Forbidden(
            "download requires owner or admin".into(),
        ));
    }
    tx.commit().await.map_err(ApiError::from)?;

    let obj = state
        .store
        .get(&attachment.storage_key)
        .await
        .map_err(store_error)?;
    let content_disposition = format!(
        "attachment; filename=\"{}\"",
        attachment.filename.replace(['"', '\\'], "")
    );
    let mut builder = Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            attachment.content_type.clone(),
        )
        .header(axum::http::header::CONTENT_DISPOSITION, content_disposition)
        .header("x-marula-sha256", attachment.sha256.clone());
    builder = builder.header(axum::http::header::CONTENT_LENGTH, obj.size_bytes);
    let body = Body::from_stream(obj.stream);
    builder
        .body(body)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))
}

/// DELETE /api/attachments/{id} — author-gated (the uploader, or an org
/// owner/admin). The stored object is reclaimed only when no sibling row
/// still references the same content-addressed key.
#[utoipa::path(operation_id = "attachments_delete", delete,
    path = "/api/attachments/{id}", tag = "attachments",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = AttachmentDeleted)))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<AttachmentDeleted>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let store = std::sync::Arc::clone(&state.store);

    let result = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let attachment = core_db::attachments::get(&mut *ctx.tx, org_id, id).await?;
        let is_author = attachment.uploaded_by_user_id == Some(user_id);
        if !is_author && !authed.principal.is_owner_or_admin() {
            return Err(ApiError::Forbidden(
                "only the uploader or an owner/admin can delete an attachment".into(),
            ));
        }
        let remaining = core_db::attachments::count_other_refs(
            &mut ctx.tx,
            org_id,
            &attachment.storage_key,
            id,
        )
        .await?;
        let object_removed = remaining == 0;
        core_db::attachments::delete(&mut ctx.tx, org_id, id).await?;

        if object_removed {
            store
                .delete(&attachment.storage_key)
                .await
                .map_err(store_error)?;
        }
        let event = AttachmentDeleted {
            attachment_id: id,
            ticket_id: attachment.ticket_id,
            object_removed,
        };
        ctx.emit(
            types::ATTACHMENT_DELETED,
            "attachment",
            Some(id.to_string().as_str()),
            &serde_json::to_value(&event).unwrap_or_default(),
        )
        .await?;
        Ok((ctx, event))
    })
    .await?;

    Ok(Json(result))
}
