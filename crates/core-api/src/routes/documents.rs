//! Document endpoints (TASKLIST D4, agent-platform §6): documents whose
//! content exists only as immutable, append-only, diffable versions.
//!
//! Purpose: create → edit (new version) → inspect history → restore (new
//! version). The §6 binding rules are enforced end to end:
//!
//! * versions are append-only — an edit NEVER mutates or deletes an
//!   existing version row, it appends the next one;
//! * restores create NEW versions (`restored_from_version` names the
//!   source) — history is never rewritten;
//! * every version stores its full body, so any pair is diffable
//!   client-side (core-documents ships the diff helper).
//!
//! Mutations ride the instant-first contract (IF1–IF3): idempotency-keyed
//! claims, If-Match-style version gates, read-your-write `mutation`
//! metadata, typed errors only. Writes emit `document.*` events; the
//! worker's indexer (2.16 pattern) mirrors the current version into the
//! unified FTS index as kind `document`.
//!
//! Non-purpose: no delete (documents are history, not disposable rows —
//! removal would orphan immutable versions), no external rendering, and
//! no agent proposal flow here (IF8 rides the MCP lane later).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Document, DocumentVersion};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn documents(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Documents,
        action,
    }
}

/// Document-rule violations arrive as typed `validation_failed` errors.
impl From<core_documents::DocumentError> for ApiError {
    fn from(e: core_documents::DocumentError) -> Self {
        ApiError::Validation(e.to_string())
    }
}

/// GET /api/documents/{id} response: the living handle plus the CURRENT
/// version's content (read-your-write on the body the last write stored).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DocumentView {
    #[serde(flatten)]
    pub document: Document,
    pub body: String,
    pub change_summary: String,
    pub content_updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_version: Option<i32>,
}

/// Instant-first mutation response (IF2): document fields at the top
/// level plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DocumentMutationResponse {
    #[serde(flatten)]
    pub document: Document,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateDocumentRequest {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub change_summary: Option<String>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateDocumentRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub change_summary: Option<String>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RestoreDocumentRequest {
    /// The historical version whose content the NEW version copies.
    pub version: i32,
    #[serde(default)]
    pub change_summary: Option<String>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct DocumentListQuery {
    /// Title substring narrowing; ranked full-text search rides /api/search.
    #[serde(default)]
    pub q: Option<String>,
}

/// GET /api/documents
#[utoipa::path(operation_id = "documents_list", get, path = "/api/documents", tag = "documents", security(("bearer" = [])),
    params(DocumentListQuery),
    responses((status = 200, body = [Document])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DocumentListQuery>,
) -> ApiResult<Json<Vec<Document>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let docs = core_db::documents::list_documents(
        &state.pool,
        authed.principal.user.org_id,
        &core_db::documents::DocumentFilter { q: q.q },
    )
    .await?;
    Ok(Json(docs))
}

/// GET /api/documents/{id}
#[utoipa::path(operation_id = "documents_get", get, path = "/api/documents/{id}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = DocumentView),
              (status = 404, description = "document not visible in scope")))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DocumentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let doc = core_db::documents::get_document(&state.pool, org_id, id).await?;
    let current =
        core_db::documents::get_version(&state.pool, org_id, id, doc.current_version).await?;
    Ok(Json(DocumentView {
        document: doc,
        body: current.body,
        change_summary: current.change_summary,
        content_updated_at: current.created_at,
        restored_from_version: current.restored_from_version,
    }))
}

/// POST /api/documents — instant-first (IF1/IF2); creates the document and
/// its immutable version 1 in one commit.
#[utoipa::path(operation_id = "documents_create", post, path = "/api/documents", tag = "documents", security(("bearer" = [])),
    request_body = CreateDocumentRequest,
    responses((status = 200, body = DocumentMutationResponse),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "title/body rejected by document rules")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateDocumentRequest>,
) -> ApiResult<Json<DocumentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Create))?;
    let title = req.title.trim().to_string();
    let body = req.body.clone().unwrap_or_default();
    let summary = req.change_summary.as_deref().unwrap_or("Initial version");
    core_documents::validate_version_content(&title, &body, summary)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "document",
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
        let (document, _version) = core_db::documents::create_document(
            &mut ctx.tx,
            org_id,
            id,
            &title,
            &body,
            summary,
            user_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::DOCUMENT_CREATED,
            "document",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "document_id": id, "version": document.current_version }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&document).unwrap_or(Value::Null),
                updated_at: document.created_at,
                key: key.as_deref(),
                payload: &document,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DocumentMutationResponse { document, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/documents/{id} — instant-first (IF1/IF2); an edit APPENDS
/// the next immutable version and moves the document's head to it. No
/// existing version row is touched.
#[utoipa::path(operation_id = "documents_update", patch, path = "/api/documents/{id}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateDocumentRequest,
    responses((status = 200, body = DocumentMutationResponse),
              (status = 404, description = "document not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "title/body rejected by document rules")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDocumentRequest>,
) -> ApiResult<Json<DocumentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "document",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::documents::get_document(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let before_head =
            core_db::documents::get_version(&mut *ctx.tx, org_id, id, before.current_version)
                .await?;
        // Validate the composed next content against the document rules.
        let next_title = req.title.as_deref().map(str::trim).unwrap_or(&before.title);
        let next_body = req.body.as_deref().unwrap_or(&before_head.body);
        let next_summary = req.change_summary.as_deref().unwrap_or("");
        core_documents::validate_version_content(next_title, next_body, next_summary)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document",
                entity_id: id,
                version,
            },
        )
        .await?;
        // Gap-free numbering: the entity seq lock above serializes
        // writers, so MAX(version)+1 is safe inside this transaction.
        let new_version_number = core_db::documents::max_version(&mut *ctx.tx, org_id, id)
            .await?
            .ok_or(
                ApiError::RlsSparse, // vanished between read and write: scope changed
            )?
            + 1;
        let _appended = core_db::documents::insert_version(
            &mut *ctx.tx,
            org_id,
            id,
            new_version_number,
            next_title,
            next_body,
            next_summary,
            user_id,
            None,
        )
        .await?;
        let after = core_db::documents::get_document(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::DOCUMENT_UPDATED,
            "document",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "document_id": id, "version": after.current_version }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DocumentMutationResponse {
                document: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/documents/{id}/versions — the full immutable history, newest
/// first, complete bodies included so any pair is diffable.
#[utoipa::path(operation_id = "documents_list_versions", get, path = "/api/documents/{id}/versions", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [DocumentVersion]),
              (status = 404, description = "document not visible in scope")))]
pub async fn list_versions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<DocumentVersion>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    // A missing document must not read as an empty history.
    core_db::documents::get_document(&state.pool, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let versions = core_db::documents::list_versions(&state.pool, org_id, id).await?;
    Ok(Json(versions))
}

/// GET /api/documents/{id}/versions/{version} — one immutable snapshot.
#[utoipa::path(operation_id = "documents_get_version", get, path = "/api/documents/{id}/versions/{version}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("version" = i32, Path)),
    responses((status = 200, body = DocumentVersion),
              (status = 404, description = "document or version not visible in scope")))]
pub async fn get_version(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, version)): Path<(Uuid, i32)>,
) -> ApiResult<Json<DocumentVersion>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let v = core_db::documents::get_version(&state.pool, org_id, id, version).await?;
    Ok(Json(v))
}

/// POST /api/documents/{id}/restore — instant-first (IF1/IF2); §6 rule:
/// the restore APPENDS a new version copying the target's content. The
/// target's title comes along, renames after it are not silently undone.
#[utoipa::path(operation_id = "documents_restore", post, path = "/api/documents/{id}/restore", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = RestoreDocumentRequest,
    responses((status = 200, body = DocumentMutationResponse),
              (status = 404, description = "document or restore target not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "restore rejected by document rules")))]
pub async fn restore(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RestoreDocumentRequest>,
) -> ApiResult<Json<DocumentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "document",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::documents::get_document(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let history = core_db::documents::list_versions(&mut *ctx.tx, org_id, id).await?;
        // The plan validates the target exists and carries the exact
        // content the new version must copy (core-documents policy).
        let plan = core_documents::RestorePlan::build(
            &history,
            Some(before.current_version),
            req.version,
            req.change_summary.as_deref().unwrap_or(""),
        )
        .map_err(|e| match e {
            core_documents::DocumentError::RestoreTargetMissing { .. } => ApiError::NotFound,
            other => other.into(),
        })?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::documents::insert_version(
            &mut *ctx.tx,
            org_id,
            id,
            plan.new_version,
            &plan.title,
            &plan.body,
            &plan.change_summary,
            user_id,
            Some(plan.restored_from_version),
        )
        .await?;
        let after = core_db::documents::get_document(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::DOCUMENT_RESTORED,
            "document",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "document_id": id,
                "version": after.current_version,
                "restored_from_version": plan.restored_from_version,
            }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document",
                entity_id: id,
                op: "restore",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(DocumentMutationResponse {
                document: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}
