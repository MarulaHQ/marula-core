//! Workflow document authoring (TASKLIST Wave 43 lane 16,
//! `p43-workflow-canvas`).
//!
//! Purpose: admin-gated CRUD over the multi-step workflow DOCUMENTS a canvas
//! edits — the `automation_rules` rows whose `definition` is set. Reads list
//! and fetch a document; writes validate the document through core-domain
//! (`WorkflowDocument::parse`) BEFORE storage, so a stored document is always
//! runnable, and emit the config-change event in the SAME transaction, so the
//! executor sees one consistent stream.
//!
//! Non-purpose: no execution (the executor owns runs), no simple-tier rule
//! editing (that is `sla_admin`, over rows whose `definition` is NULL), and no
//! document semantics (core-domain `workflow_document` owns parsing and
//! every author-time refusal).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_db::automations::{WorkflowDocumentRecord, WorkflowRulePatch};
use core_domain::automations::AutomationSubjectKind;
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::workflow_document::WorkflowDocument;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Admin-surface permission helper, the same shape the SLA and automation
/// admin routes use: authoring is operator tooling, not a per-board read.
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// A workflow document as the canvas reads it. `revision` is the document
/// version the executor records on each run.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WorkflowDocumentView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub trigger_event: String,
    pub revision: i32,
    /// `"ticket"` or `"board"` — the tier of subject the document runs against.
    pub subject_kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub created_by: Option<Uuid>,
    #[schema(value_type = Object)]
    pub definition: serde_json::Value,
}

fn document_view(rec: WorkflowDocumentRecord) -> WorkflowDocumentView {
    WorkflowDocumentView {
        id: rec.id,
        name: rec.name,
        enabled: rec.enabled,
        trigger_event: rec.trigger_event,
        revision: rec.revision,
        subject_kind: rec.subject_kind.as_str().to_string(),
        created_by: rec.created_by,
        definition: rec.definition,
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct WorkflowDocumentListView {
    pub documents: Vec<WorkflowDocumentView>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateWorkflowDocumentRequest {
    pub name: String,
    /// Domain event type the document is triggered by, e.g. "ticket.created".
    pub trigger_event: String,
    /// `"ticket"` or `"board"`; omitted means `"ticket"` (the simple tier's
    /// default). A board-subject document runs against events with no ticket.
    pub subject_kind: Option<String>,
    /// The workflow document (see WorkflowDocument schema); parsed and
    /// validated before storage.
    pub definition: serde_json::Value,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateWorkflowDocumentRequest {
    pub name: Option<String>,
    pub trigger_event: Option<String>,
    /// Replacing the document advances the revision by one.
    pub definition: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub subject_kind: Option<String>,
}

/// Parses and validates a document through core-domain. Every author-time
/// refusal is surfaced as a validation error rather than stored.
fn validate_definition(definition: &serde_json::Value) -> ApiResult<()> {
    WorkflowDocument::parse(definition)
        .map_err(|e| ApiError::Validation(format!("workflow document does not validate: {e}")))?;
    Ok(())
}

/// Maps the request's `subject_kind` through the domain enum, rejecting an
/// unrecognized value rather than silently coercing it to a ticket subject
/// (the CHECK constraint admits only `ticket`/`board`).
fn subject_kind_of(raw: Option<&str>) -> ApiResult<AutomationSubjectKind> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("ticket") => Ok(AutomationSubjectKind::Ticket),
        Some("board") => Ok(AutomationSubjectKind::Board),
        Some(other) => Err(ApiError::Validation(format!(
            "subject_kind must be \"ticket\" or \"board\", got \"{other}\""
        ))),
    }
}

/// GET /api/workflows/documents
#[utoipa::path(operation_id = "workflow_documents_list", get, path = "/api/workflows/documents", tag = "workflows", security(("bearer" = [])),
    responses((status = 200, body = WorkflowDocumentListView)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<WorkflowDocumentListView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        core_db::automations::list_workflow_documents(&mut *tx, authed.principal.user.org_id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(WorkflowDocumentListView {
        documents: rows.into_iter().map(document_view).collect(),
    }))
}

/// GET /api/workflows/documents/{id}
#[utoipa::path(operation_id = "workflow_documents_get", get, path = "/api/workflows/documents/{id}", tag = "workflows", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = WorkflowDocumentView),
              (status = 404, description = "document not visible in scope")))]
pub async fn get(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<WorkflowDocumentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rec =
        core_db::automations::get_workflow_document(&mut *tx, authed.principal.user.org_id, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(document_view(rec)))
}

/// POST /api/workflows/documents — the document is validated before storage
/// (see [`validate_definition`]); the row is created as revision 1.
#[utoipa::path(operation_id = "workflow_documents_create", post, path = "/api/workflows/documents", tag = "workflows", security(("bearer" = [])),
    request_body = CreateWorkflowDocumentRequest,
    responses((status = 200, body = WorkflowDocumentView)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateWorkflowDocumentRequest>,
) -> ApiResult<Json<WorkflowDocumentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Create))?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::Validation(
            "workflow document name is required".into(),
        ));
    }
    let trigger_event = req.trigger_event.trim().to_string();
    if trigger_event.is_empty() {
        return Err(ApiError::Validation("trigger_event is required".into()));
    }
    validate_definition(&req.definition)?;
    let subject_kind = subject_kind_of(req.subject_kind.as_deref())?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let id = Uuid::now_v7();
    let saved = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rec = core_db::automations::insert_workflow(
            &mut *ctx.tx,
            id,
            org_id,
            Some(user_id),
            &name,
            &trigger_event,
            subject_kind,
            req.definition.clone(),
        )
        .await?;
        ctx.emit(
            types::AUTOMATION_RULE_UPDATED,
            "automation_rule",
            Some(rec.id.to_string().as_str()),
            &serde_json::json!({ "rule_id": rec.id }),
        )
        .await?;
        Ok((ctx, rec))
    })
    .await?;
    Ok(Json(document_view(saved)))
}

/// PATCH /api/workflows/documents/{id} — replacing the document validates it
/// and advances the revision.
#[utoipa::path(operation_id = "workflow_documents_update", patch, path = "/api/workflows/documents/{id}", tag = "workflows", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateWorkflowDocumentRequest,
    responses((status = 200, body = WorkflowDocumentView),
              (status = 404, description = "document not visible in scope")))]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateWorkflowDocumentRequest>,
) -> ApiResult<Json<WorkflowDocumentView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    if let Some(definition) = &req.definition {
        validate_definition(definition)?;
    }
    let subject_kind = req
        .subject_kind
        .as_deref()
        .map(|raw| subject_kind_of(Some(raw)))
        .transpose()?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let patch = WorkflowRulePatch {
        name: req.name.clone(),
        trigger_event: req.trigger_event.as_ref().map(|s| s.trim().to_string()),
        definition: req.definition.clone(),
        enabled: req.enabled,
        subject_kind,
    };
    let saved = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rec = core_db::automations::update_workflow(&mut *ctx.tx, org_id, id, patch).await?;
        ctx.emit(
            types::AUTOMATION_RULE_UPDATED,
            "automation_rule",
            Some(rec.id.to_string().as_str()),
            &serde_json::json!({ "rule_id": rec.id }),
        )
        .await?;
        Ok((ctx, rec))
    })
    .await?;
    Ok(Json(document_view(saved)))
}
