//! Board checklist-template configuration endpoints (TASKLIST 2.8):
//! create/replace, read, and delete a board's pre-close checklist template,
//! plus the per-board enforcement toggle. Board-admin gated
//! (`Permission::ManageBoards`); every mutation emits an outbox event in-tx.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::events::types;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn manage_boards() -> Permission {
    Permission::ManageBoards
}

fn board_view() -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Tickets,
        action: core_domain::policy::Action::View,
    }
}

/// GET /api/boards/{id}/checklist-template response: the template (if any),
/// its items, and the enforcement posture actually applied at close time.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChecklistTemplateView {
    pub template: Option<core_domain::entities::ChecklistTemplate>,
    pub items: Vec<core_domain::entities::ChecklistTemplateItem>,
    /// Mirrors `boards.require_checklist_close`.
    pub require_checklist_close: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PutChecklistTemplateRequest {
    pub name: String,
    /// The complete required-step list; replaces any previous items.
    pub items: Vec<String>,
    /// Enforcement toggle (defaults-first: OFF until explicitly enabled).
    #[serde(default)]
    pub require_checklist_close: bool,
}

/// GET /api/boards/{id}/checklist-template
#[utoipa::path(operation_id = "boards_get_checklist_template", get, path = "/api/boards/{id}/checklist-template", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ChecklistTemplateView)))]
pub async fn get_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ChecklistTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    // View permission: reading config needs no board-admin rights.
    require_permission(&authed.principal, board_view())?;
    let board = core_db::boards::get_board(&state.pool, org_id, id).await?;
    let template = core_db::checklists::get_template_for_board(&state.pool, org_id, id).await?;
    let items = match &template {
        Some(t) => core_db::checklists::list_template_items(&state.pool, org_id, t.id).await?,
        None => Vec::new(),
    };
    Ok(Json(ChecklistTemplateView {
        template,
        items,
        require_checklist_close: board.require_checklist_close,
    }))
}

/// PUT /api/boards/{id}/checklist-template — creates or replaces the
/// board's template wholesale and sets the enforcement toggle; emits
/// `board.checklist_template_updated` and audits the change.
#[utoipa::path(operation_id = "boards_put_checklist_template", put, path = "/api/boards/{id}/checklist-template", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PutChecklistTemplateRequest,
    responses((status = 200, body = ChecklistTemplateView)))]
pub async fn put_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PutChecklistTemplateRequest>,
) -> ApiResult<Json<ChecklistTemplateView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("template name is required".into()));
    }
    let cleaned: Vec<String> = req
        .items
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    if cleaned.is_empty() && req.require_checklist_close {
        return Err(ApiError::Conflict(
            "cannot enforce an empty checklist — add items or turn the toggle off".into(),
        ));
    }

    let view = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Board-level RBAC (deny wins) + RLS proves same-org.
        super::board_access::authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            id,
            core_domain::policy::BoardOp::ConfigureBoard,
        )
        .await?;
        let (template, items) = core_db::checklists::upsert_template(
            &mut ctx.tx,
            org_id,
            id,
            Uuid::now_v7(),
            req.name.trim(),
            &cleaned,
        )
        .await?;
        let require = core_db::checklists::set_require_checklist_close(
            &mut *ctx.tx,
            org_id,
            id,
            req.require_checklist_close,
        )
        .await?;

        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "board.checklist_template_update".into(),
                entity_type: Some("board".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "template": template.name,
                    "item_count": items.len(),
                    "require_checklist_close": require,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        ctx.emit(
            types::BOARD_CHECKLIST_TEMPLATE_UPDATED,
            "board",
            Some(id.to_string().as_str()),
            &serde_json::json!({
                "board_id": id,
                "template_id": template.id,
                "item_count": items.len(),
                "require_checklist_close": require,
            }),
        )
        .await?;

        Ok((
            ctx,
            ChecklistTemplateView {
                template: Some(template),
                items,
                require_checklist_close: require,
            },
        ))
    })
    .await?;
    Ok(Json(view))
}

/// DELETE /api/boards/{id}/checklist-template — removes the template.
/// Ticket instances demote to ad-hoc items; with no template left,
/// close-time enforcement is off by construction regardless of the toggle.
#[utoipa::path(operation_id = "boards_delete_checklist_template", delete, path = "/api/boards/{id}/checklist-template", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn delete_template(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, manage_boards())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        super::board_access::authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            id,
            core_domain::policy::BoardOp::ConfigureBoard,
        )
        .await?;
        core_db::checklists::delete_template(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::BOARD_CHECKLIST_TEMPLATE_UPDATED,
            "board",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "board_id": id, "deleted": true }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
