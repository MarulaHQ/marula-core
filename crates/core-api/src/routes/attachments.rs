//! Attachment endpoints (TASKLIST 2.14): upload, list, download,
//! author-gated delete.
//!
//! Purpose: the ONLY door for binary objects into Marula. Every upload
//! passes the core-domain allowlist gate (size cap + magic bytes vs
//! declared type vs extension), passes the malware SCAN GATE, hashes to
//! sha256, reuses already-stored objects per org (content-addressed dedup),
//! and persists its metadata row plus `attachment.added` in ONE org-bound
//! transaction. Downloads sit behind the same authz and either stream
//! through this API or hand out a short-lived store URL (see below).
//!
//! The scan gate (Wave 52 lane P6a) runs BEFORE anything is stored, and it
//! fails CLOSED: a scanner that cannot answer ends the request with a 503
//! and nothing is written. Bytes a scanner names infected never reach the
//! admitted namespace — they go to `org/<org>/quarantine/<sha256>` with an
//! `attachment_scans` row and `attachment.quarantined`, and the uploader
//! gets a typed 422. None mode (no scanner configured) admits with a
//! `skipped` verdict on record, never a silent `clean`.
//!
//! Non-purpose: no multipart parsing (single raw octet-stream bodies only,
//! filename/content-type carried as headers), no image processing, no
//! quota accounting, no quarantine administration (the P6c admin surface
//! owns list/release/delete over `attachment_scans`).
//!
//! Downloads (Wave 52 lane P7) ask the store for a presigned GET before they
//! agree to stream: where the backend can sign one (`s3`), the client gets a
//! short-lived URL and the API holds no task and no store connection for the
//! transfer; where it cannot (`local-fs`, the self-host default), the same
//! request streams as it always did. The choice is made per request from
//! [`ObjectStore::presign_get`]'s answer, never at compile time. A URL is
//! minted only for an object whose scan verdict has already been read and
//! admitted, because a URL outlives the request and a verdict read after the
//! fact is a verdict the URL already outran.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{Extensions, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use core_domain::entities::Attachment;
use core_domain::entities::ScanStatus;
use core_domain::events::types;
use core_domain::events::{AttachmentAdded, AttachmentDeleted, AttachmentQuarantined};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
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

    // --- the scan gate -------------------------------------------------
    // Before the transaction opens, so a refusal costs no connection and no
    // lock, and before any byte is stored. A scanner that cannot answer is a
    // REFUSAL, not an admission: "unknown" must never be recorded as clean.
    let verdict = crate::scan_gate::gate(&state, &body).await?;
    let status = verdict.status;
    let scanner_name = verdict.scanner;
    // Cloned rather than moved: the closure below still borrows the whole
    // verdict to hand the quarantine writer its detail.
    let detail = verdict.detail.clone();
    let storage_key = core_store::attachment_key(org_id, &sha256_hex);
    let store = std::sync::Arc::clone(&state.store);

    let gated = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;

        let new_scan = core_db::attachment_scans::NewScan {
            org_id,
            sha256: &sha256_hex,
            status,
            detail: detail.as_deref(),
            scanner: scanner_name,
            size_bytes: body.len() as i64,
            content_type: &content_type,
            filename: &filename,
            storage_key: &storage_key,
            ticket_id: Some(id),
            uploaded_by_user_id: Some(user_id),
            uploaded_by_portal_user_id: None,
        };

        if status == ScanStatus::Infected {
            // The bytes are held, never admitted: a separate namespace, a
            // verdict row that is the operator's record, a notification for
            // the uploader, and no attachment.
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
                    ticket_id: Some(id),
                    uploaded_by_user_id: Some(user_id),
                    verdict: &verdict,
                },
            )
            .await?;
            ctx.emit(
                types::ATTACHMENT_QUARANTINED,
                "attachment",
                Some(sha256_hex.as_str()),
                &serde_json::to_value(AttachmentQuarantined {
                    sha256: sha256_hex.clone(),
                    filename: filename.clone(),
                    content_type: content_type.clone(),
                    size_bytes: body.len() as i64,
                    scanner: scanner_name.to_string(),
                    detail: detail.clone(),
                    ticket_id: Some(id),
                    uploaded_by_user_id: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
            return Ok((ctx, crate::scan_gate::Gated::Quarantined(held)));
        }

        // Admitted: content-addressed dedup skips the PUT when this org
        // already holds these exact bytes; rows still get their own identity.
        let existing =
            core_db::attachments::find_existing_object_key(&mut ctx.tx, org_id, &sha256_hex)
                .await?;
        if existing.is_none() {
            store
                .put(&storage_key, &content_type, body.clone())
                .await
                .map_err(store_error)?;
        }
        // The verdict belongs to the object, so a second row for the same
        // digest keeps the first admission's record rather than re-writing it.
        core_db::attachment_scans::record_admitted(&mut ctx.tx, new_scan).await?;

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
        Ok((ctx, crate::scan_gate::Gated::Admitted(attachment)))
    })
    .await?;

    // The transaction above COMMITTED the quarantine record, so the refusal
    // that follows is a report about a fact on disk, not an aborted write.
    match gated {
        crate::scan_gate::Gated::Admitted(attachment) => Ok(Json(attachment)),
        crate::scan_gate::Gated::Quarantined(held) => Err(held.refusal()),
    }
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

/// GET /api/attachments/{id}/download — streams the stored bytes through the
/// API behind board-view authz, or answers with a presigned URL when the
/// caller asks for one (`?presign=true`) and the store can sign it.
///
/// The ticket is the optimisation and the stream is the contract: without the
/// flag the request gets the same bytes, headers and status it always did,
/// and on a deployment that cannot presign the ticket request answers
/// `mode: "stream"` rather than failing. [`AttachmentDownloadTicket`] carries
/// the filename the stream would have carried in `content-disposition`,
/// because a presigned GET names the object key — a digest — instead.
#[utoipa::path(operation_id = "attachments_download", get,
    path = "/api/attachments/{id}/download", tag = "attachments",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path),
        ("presign" = Option<bool>, Query, description = "`true` asks for a download ticket instead of the bytes; the answer says whether a URL was minted"),
    ),
    responses(
        (status = 200, content(
            (Vec<u8> = "application/octet-stream"),
            (AttachmentDownloadTicket = "application/json"),
        )),
        (status = 409, description = "the object is held by the scan gate: neither a URL nor the bytes are issued"),
    ))]
pub async fn download(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(query): Query<DownloadTicketQuery>,
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
    // Same transaction as the row above: the verdict and the attachment come
    // from one snapshot, so a scan that escalated between the two reads
    // cannot slip through.
    let admission = verdict_for(&mut tx, org_id, &attachment.sha256).await?;
    tx.commit().await.map_err(ApiError::from)?;

    if admission == DownloadAdmission::Held {
        return Err(held_refusal());
    }

    if query.presign {
        let ticket = download_ticket(
            &state,
            admission,
            DownloadSubject {
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

/// The ticket request's query: `?presign=true` asks for a URL, absent streams.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DownloadTicketQuery {
    /// `true` asks for a download ticket. Anything else (or no query at all)
    /// keeps the byte contract untouched.
    #[serde(default)]
    pub presign: bool,
}

/// How long a minted download URL stays valid. Deliberately short, and
/// deliberately not the thing that makes the URL safe: five minutes bounds
/// the life of a leaked link, while refusing the URL for a held object is
/// what keeps bytes behind the gate. A TTL alone would just be a race.
pub const DOWNLOAD_URL_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// What a downloader gets instead of the bytes: where to fetch them, plus the
/// metadata it would otherwise read off the streamed response's headers.
#[derive(Debug, Serialize, ToSchema)]
pub struct AttachmentDownloadTicket {
    /// `presigned` — GET `url` directly and send it NO bearer token.
    /// `stream` — this deployment cannot sign one; call the endpoint again
    /// without `presign`, or read the bytes out of that same request.
    pub mode: DownloadMode,
    /// Signed URL, present exactly when `mode` is `presigned`.
    pub url: Option<String>,
    /// When `url` stops working. The client need not act on it: the next
    /// download asks for a fresh ticket.
    #[schema(value_type = Option<String>)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The name the streamed response would have put in
    /// `content-disposition`. A presigned GET names the object key, which is
    /// a digest, so a client that saves the file must use this instead.
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

/// Which way this deployment can hand a download out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadMode {
    /// The store signed a URL: the bytes never touch the API.
    Presigned,
    /// No URL here (local-fs, or a store that could not sign): stream.
    Stream,
}

/// What the object's scan verdict permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadAdmission {
    /// A scanner passed it, none mode admitted it, or an operator released it.
    Admitted,
    /// Held for an operator: the bytes stay behind the gate.
    Held,
    /// No verdict on record at all — pre-0139 rows, and anything that wrote
    /// the shared key space without going through the gate.
    Unscanned,
}

/// Reads a verdict as permission to hand out a URL.
fn admission(scan: Option<&core_domain::entities::AttachmentScan>) -> DownloadAdmission {
    match scan {
        None => DownloadAdmission::Unscanned,
        // A release keeps the `infected` verdict on record and adds the
        // operator's override; that override is what admits the object.
        Some(s) if s.is_quarantined() && s.released_at.is_none() => DownloadAdmission::Held,
        Some(_) => DownloadAdmission::Admitted,
    }
}

/// Reads the object's verdict inside the caller's transaction (the org scope
/// is already applied), so the decision and the row it governs share one
/// snapshot.
pub(crate) async fn verdict_for(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    sha256: &str,
) -> ApiResult<DownloadAdmission> {
    let scan = core_db::attachment_scans::get(conn, org_id, sha256).await?;
    Ok(admission(scan.as_ref()))
}

pub(crate) fn held_refusal() -> ApiError {
    ApiError::Conflict(
        "these bytes are held by the scan gate; an operator must release them first".into(),
    )
}

/// The fields both download doors share, borrowed from whichever row owns
/// them.
pub(crate) struct DownloadSubject<'a> {
    pub storage_key: &'a str,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
}

/// Answers the ticket request for one admitted object. A store that cannot
/// sign one (`local-fs`) is not an error: the answer says `stream`, and the
/// caller takes the path it always took.
pub(crate) async fn download_ticket(
    state: &AppState,
    admission: DownloadAdmission,
    subject: DownloadSubject<'_>,
) -> ApiResult<AttachmentDownloadTicket> {
    let presigned = match admission {
        DownloadAdmission::Held => return Err(held_refusal()),
        // Never a URL without a verdict: an unscanned object keeps the
        // pre-existing streaming posture and gains nothing here.
        DownloadAdmission::Unscanned => None,
        DownloadAdmission::Admitted => {
            match state
                .store
                .presign_get(subject.storage_key, DOWNLOAD_URL_TTL)
                .await
            {
                Ok(url) => url,
                // A store that cannot sign is not a failed download: the
                // stream still works, so degrade to it and say so.
                Err(e) => {
                    tracing::warn!(error = %e, "presign failed; streaming instead");
                    None
                }
            }
        }
    };
    Ok(AttachmentDownloadTicket {
        mode: if presigned.is_some() {
            DownloadMode::Presigned
        } else {
            DownloadMode::Stream
        },
        expires_at: presigned.as_ref().and_then(|_| presigned_expiry()),
        url: presigned,
        filename: subject.filename.to_string(),
        content_type: subject.content_type.to_string(),
        size_bytes: subject.size_bytes,
        sha256: subject.sha256.to_string(),
    })
}

fn presigned_expiry() -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::Duration::from_std(DOWNLOAD_URL_TTL)
        .ok()
        .map(|ttl| chrono::Utc::now() + ttl)
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
