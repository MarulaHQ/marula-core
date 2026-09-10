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
use core_domain::entities::{
    Document, DocumentLayout, DocumentVersion, LayoutField, LinkKind, ResolvedLink,
};
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

/// `?client_id=<uuid>|global` → `Some(Some(id))` / `Some(None)`;
/// absent → None (all scopes).
fn parse_scope(raw: &Option<String>) -> ApiResult<Option<Option<Uuid>>> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some("global") => Ok(Some(None)),
        Some(id) => Ok(Some(Some(Uuid::parse_str(id).map_err(|_| {
            ApiError::Validation("client_id must be a uuid or 'global'".into())
        })?))),
    }
}

/// Layout helpers shared by create/update: resolve the org's layout and
/// validate a custom-fields object against its schema (docs-platform v1).
async fn resolve_layout(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    layout_id: Uuid,
) -> ApiResult<DocumentLayout> {
    core_db::document_layouts::get_layout(&mut *tx, org_id, layout_id)
        .await
        .map_err(ApiError::from)
}

pub fn validate_fields(layout: &DocumentLayout, fields: &Value) -> ApiResult<()> {
    let obj = fields
        .as_object()
        .ok_or_else(|| ApiError::Validation("fields must be a JSON object".into()))?;
    let check = |f: &LayoutField, v: &Value| -> ApiResult<()> {
        let ok = match f.r#type.as_str() {
            "number" => v.is_number(),
            "bool" => v.is_boolean(),
            "text" | "date" | "email" | "url" | "select" => v.is_string(),
            _ => true,
        };
        if !ok {
            return Err(ApiError::Validation(format!(
                "field '{}' must be {}",
                f.key, f.r#type
            )));
        }
        if f.r#type == "select" && !f.options.is_empty() {
            let s = v.as_str().unwrap_or_default();
            if !f.options.iter().any(|o| o == s) {
                return Err(ApiError::Validation(format!(
                    "field '{}' must be one of: {}",
                    f.key,
                    f.options.join(", ")
                )));
            }
        }
        Ok(())
    };
    for f in &layout.fields {
        match obj.get(&f.key) {
            None | Some(Value::Null) => {
                if f.required {
                    return Err(ApiError::Validation(format!(
                        "field '{}' is required",
                        f.key
                    )));
                }
            }
            Some(v) => check(f, v)?,
        }
    }
    for key in obj.keys() {
        if !layout.fields.iter().any(|f| &f.key == key) {
            return Err(ApiError::Validation(format!(
                "unknown field '{}' for layout '{}'",
                key, layout.name
            )));
        }
    }
    Ok(())
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
    /// NULL = org-global (docs-platform v1).
    #[serde(default)]
    pub client_id: Option<Uuid>,
    /// Typed layout; when set, `fields` is validated against its schema.
    #[serde(default)]
    pub layout_id: Option<Uuid>,
    #[serde(default)]
    pub fields: Option<Value>,
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
    /// Absent = keep the current scope; explicit null moves to global.
    #[serde(default)]
    pub client_id: Option<Option<Uuid>>,
    /// Absent = keep; explicit null = untyped.
    #[serde(default)]
    pub layout_id: Option<Option<Uuid>>,
    /// Absent = keep; present = validated against the (current) layout.
    #[serde(default)]
    pub fields: Option<Value>,
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
    /// Scope narrow: a client uuid, or the literal `global` for org-wide
    /// documents (docs-platform v1). Absent = all scopes.
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub layout_id: Option<Uuid>,
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let docs = core_db::documents::list_documents(
        &mut *tx,
        authed.principal.user.org_id,
        &core_db::documents::DocumentFilter {
            q: q.q,
            client_id: parse_scope(&q.client_id)?,
            layout_id: q.layout_id,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let doc = core_db::documents::get_document(&mut *tx, org_id, id).await?;
    let current =
        core_db::documents::get_version(&mut *tx, org_id, id, doc.current_version).await?;
    tx.commit().await.map_err(ApiError::from)?;
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

    // Layout + fields validated OUTSIDE the org tx: a bad layout id or
    // schema-mismatched fields must not consume a mutation claim.
    let fields = req
        .fields
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if let Some(layout_id) = req.layout_id {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let layout = resolve_layout(&mut tx, org_id, layout_id).await?;
        tx.commit().await.map_err(ApiError::from)?;
        validate_fields(&layout, &fields)?;
    }

    let client_id = req.client_id;
    let layout_id = req.layout_id;

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
            client_id,
            layout_id,
            &fields,
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
                provenance: Default::default(),
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

        // Meta changes (docs-platform v1): scope, layout, custom fields.
        // Absent = keep; a layout change re-validates the effective
        // fields (kept fields vs the NEW schema).
        let next_fields = match (&req.fields, req.layout_id) {
            (Some(f), Some(Some(new_layout))) => {
                let layout =
                    core_db::document_layouts::get_layout(&mut *ctx.tx, org_id, new_layout)
                        .await
                        .map_err(|e| instant::mutation_scope_err(e.into()))?;
                validate_fields(&layout, f)?;
                f.clone()
            }
            (Some(f), _) => {
                if let Some(layout_id) = req.layout_id.flatten().or(before.layout_id) {
                    let layout =
                        core_db::document_layouts::get_layout(&mut *ctx.tx, org_id, layout_id)
                            .await
                            .map_err(|e| instant::mutation_scope_err(e.into()))?;
                    validate_fields(&layout, f)?;
                }
                f.clone()
            }
            (None, _) => before.fields.clone(),
        };
        let next_client = req.client_id.flatten().or(before.client_id);
        let next_layout = req.layout_id.flatten().or(before.layout_id);

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
        let after = core_db::documents::update_document_meta(
            &mut *ctx.tx,
            org_id,
            id,
            next_title,
            next_client,
            next_layout,
            &next_fields,
        )
        .await?;
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
                provenance: Default::default(),
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // A missing document must not read as an empty history.
    core_db::documents::get_document(&mut *tx, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let versions = core_db::documents::list_versions(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
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
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let v = core_db::documents::get_version(&mut *tx, org_id, id, version).await?;
    tx.commit().await.map_err(ApiError::from)?;
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
                provenance: Default::default(),
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

// ---- document layouts (docs-platform v1, 0053) ---------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LayoutMutationResponse {
    #[serde(flatten)]
    pub layout: DocumentLayout,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateLayoutRequest {
    /// Stable handle; derived from the name when absent.
    #[serde(default)]
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub fields: Vec<LayoutField>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

fn default_icon() -> String {
    "file".into()
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateLayoutRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub fields: Vec<LayoutField>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

fn slugify(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.trim_matches('-').to_lowercase()
}

/// GET /api/documents/layouts — the org's layout catalog (system first).
#[utoipa::path(operation_id = "documents_list_layouts", get, path = "/api/documents/layouts", tag = "documents", security(("bearer" = [])),
    responses((status = 200, body = [DocumentLayout])))]
pub async fn list_layouts(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<DocumentLayout>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let layouts = core_db::document_layouts::list_layouts(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(layouts))
}

/// POST /api/documents/layouts — instant-first; system slugs are reserved.
#[utoipa::path(operation_id = "documents_create_layout", post, path = "/api/documents/layouts", tag = "documents", security(("bearer" = [])),
    request_body = CreateLayoutRequest,
    responses((status = 200, body = LayoutMutationResponse),
              (status = 409, description = "idempotency key reused or slug taken"),
              (status = 422, description = "invalid field schema")))]
pub async fn create_layout(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateLayoutRequest>,
) -> ApiResult<Json<LayoutMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let slug = req
        .slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(slugify)
        .unwrap_or_else(|| slugify(&name));
    if slug.is_empty() {
        return Err(ApiError::Validation("slug could not be derived".into()));
    }
    let icon = if req.icon.trim().is_empty() {
        default_icon()
    } else {
        req.icon.trim().to_string()
    };
    let fields = serde_json::to_value(&req.fields).unwrap_or(Value::Array(Vec::new()));
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "document_layout",
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
        let layout = core_db::document_layouts::create_layout(
            &mut *ctx.tx,
            org_id,
            id,
            &slug,
            &name,
            req.description.trim(),
            &icon,
            &fields,
            false,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document_layout",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::DOCUMENT_LAYOUT_CREATED,
            "document_layout",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "layout_id": id, "slug": slug }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document_layout",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&layout).unwrap_or(Value::Null),
                updated_at: layout.created_at,
                key: key.as_deref(),
                payload: &layout,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(LayoutMutationResponse { layout, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/documents/layouts/{id} — instant-first; system layouts are
/// editable (schema and branding), never deletable.
#[utoipa::path(operation_id = "documents_update_layout", patch, path = "/api/documents/layouts/{id}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateLayoutRequest,
    responses((status = 200, body = LayoutMutationResponse),
              (status = 404, description = "layout not visible in scope")))]
pub async fn update_layout(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLayoutRequest>,
) -> ApiResult<Json<LayoutMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let icon = if req.icon.trim().is_empty() {
        default_icon()
    } else {
        req.icon.trim().to_string()
    };
    let fields = serde_json::to_value(&req.fields).unwrap_or(Value::Array(Vec::new()));
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "document_layout",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::document_layouts::get_layout(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let layout = core_db::document_layouts::update_layout(
            &mut *ctx.tx,
            org_id,
            id,
            &name,
            req.description.trim(),
            &icon,
            &fields,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document_layout",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::DOCUMENT_LAYOUT_UPDATED,
            "document_layout",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "layout_id": id, "slug": layout.slug }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document_layout",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&layout).unwrap_or(Value::Null),
                updated_at: layout.updated_at,
                key: key.as_deref(),
                payload: &layout,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(LayoutMutationResponse { layout, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/documents/layouts/{id} — custom layouts only; system
/// layouts are a typed 409 (imports and the DMS seam rely on them).
#[utoipa::path(operation_id = "documents_delete_layout", delete, path = "/api/documents/layouts/{id}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "layout not visible in scope"),
              (status = 409, description = "system layouts cannot be deleted")))]
pub async fn delete_layout(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::document_layouts::get_layout(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.is_system {
            return Err(ApiError::Conflict(
                "system layouts cannot be deleted; edit them instead".into(),
            ));
        }
        core_db::document_layouts::delete_layout(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "document_layout",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::DOCUMENT_LAYOUT_REMOVED,
            "document_layout",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "layout_id": id, "slug": before.slug }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "document_layout",
                entity_id: id,
                op: "delete",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: Value::Null,
                updated_at: before.updated_at,
                key: None,
                payload: &before,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((ctx, MutationOutcome::Applied(mutation)))
    })
    .await?;
    outcome.into_response()?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ---- cross-entity relationship links (docs-platform v1) ------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateLinkRequest {
    /// One of: document | password | client | ticket | kb_article.
    pub to_kind: String,
    pub to_id: Uuid,
}

/// Shared link read/create for both from-kinds (documents mount it at
/// /documents/{id}/links, passwords at /passwords/{id}/links).
pub(crate) async fn links_for(
    state: &AppState,
    extensions: &Extensions,
    from_kind: LinkKind,
    from_id: Uuid,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(extensions)?;
    require_permission(&authed.principal, documents(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let links = core_db::document_links::list_links(&mut *tx, org_id, from_kind, from_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(links))
}

#[utoipa::path(operation_id = "documents_list_links", get, path = "/api/documents/{id}/links", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [ResolvedLink])))]
pub async fn list_links(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    links_for(&state, &extensions, LinkKind::Document, id).await
}

/// POST /api/documents/{id}/links — target existence is enforced by the
/// resolution join (the link row itself allows any uuid, so the route
/// checks the target kind's table exists for the org before insert).
#[utoipa::path(operation_id = "documents_add_link", post, path = "/api/documents/{id}/links", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateLinkRequest,
    responses((status = 200, body = [ResolvedLink]),
              (status = 404, description = "document or link target not visible in scope"),
              (status = 409, description = "link already exists")))]
pub async fn add_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateLinkRequest>,
) -> ApiResult<Json<Vec<ResolvedLink>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let to_kind = LinkKind::parse(&req.to_kind)
        .ok_or_else(|| ApiError::Validation(format!("unknown link kind '{}'", req.to_kind)))?;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Both endpoints must be visible in scope, else typed 404.
        core_db::documents::get_document(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        link_target_exists(&mut ctx.tx, org_id, to_kind, req.to_id).await?;
        let link = core_db::document_links::create_link(
            &mut *ctx.tx, org_id, Uuid::now_v7(), LinkKind::Document, id, to_kind, req.to_id, user_id,
        )
        .await?;
        ctx.emit(types::LINK_ADDED, "link", Some(link.id.to_string().as_str()),
            &serde_json::json!({ "from_kind": "document", "from_id": id, "to_kind": to_kind.as_str(), "to_id": req.to_id }))
            .await?;
        Ok((ctx, ()))
    })
    .await?;
    // Return the refreshed panel: reads ride the pooled RLS path.
    links_for(&state, &extensions, LinkKind::Document, id).await
}

/// DELETE /api/documents/links/{id}
#[utoipa::path(operation_id = "documents_delete_link", delete, path = "/api/documents/links/{id}", tag = "documents", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "deleted"),
              (status = 404, description = "link not visible in scope")))]
pub async fn delete_link(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, documents(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        core_db::document_links::delete_link(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::LINK_REMOVED,
            "link",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "link_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Typed existence checks for link targets (RLS-scoped).
pub(crate) async fn link_target_exists(
    tx: &mut sqlx::PgConnection,
    org_id: Uuid,
    kind: LinkKind,
    id: Uuid,
) -> ApiResult<()> {
    let found: Option<bool> = match kind {
        LinkKind::Document => Some(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM documents WHERE org_id = $1 AND id = $2)",
            )
            .bind(org_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?,
        ),
        LinkKind::Password => Some(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM passwords WHERE org_id = $1 AND id = $2)",
            )
            .bind(org_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?,
        ),
        LinkKind::Client => Some(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM clients WHERE org_id = $1 AND id = $2)",
            )
            .bind(org_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?,
        ),
        LinkKind::Ticket => Some(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM tickets WHERE org_id = $1 AND id = $2)",
            )
            .bind(org_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?,
        ),
        LinkKind::KbArticle => Some(
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM kb_articles WHERE org_id = $1 AND id = $2)",
            )
            .bind(org_id)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?,
        ),
    };
    match found {
        Some(true) => Ok(()),
        _ => Err(ApiError::NotFound),
    }
}
