//! Board organization + board-level RBAC endpoints (the private build tracker 2.11):
//! nav groups, group assignment, per-board membership roles, and the
//! annotated nav list. Every mutation emits its outbox event in-tx (§4.2)
//! and flows authorization through `core_domain::policy` — a board-role
//! denial always wins over org-level grants.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::entities::Actor;
use core_domain::events::{self, types};
use core_domain::policy::{self, Action, BoardOp, BoardRole, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

/// The org-level permission each board-scoped operation falls back to when
/// the caller holds no explicit membership row (defaults-first passthrough).
pub fn org_permission_for(op: BoardOp) -> Permission {
    match op {
        BoardOp::View => tickets(Action::View),
        BoardOp::CreateTicket => tickets(Action::Create),
        BoardOp::WorkTicket | BoardOp::CloseTicket => tickets(Action::Edit),
        BoardOp::ConfigureBoard | BoardOp::ManageMembers => Permission::ManageBoards,
    }
}

/// The single board-RBAC gate used by every mutating/reading handler in
/// this lane: loads the caller's membership role (if any), composes it
/// with the org-level verdict via `policy::authorize_board_scoped`
/// (deny wins; a row never expands access), and maps denial to 403.
pub async fn authorize_board_op(
    conn: &mut sqlx::PgConnection,
    principal: &core_domain::entities::Principal,
    board_id: Uuid,
    op: BoardOp,
) -> ApiResult<()> {
    let role = core_db::boards::role_for_user(
        &mut *conn,
        principal.user.org_id,
        board_id,
        principal.user.id,
    )
    .await?;
    let decision = policy::authorize_board_scoped(
        policy::check(&principal.permissions, org_permission_for(op)),
        role,
        op,
    );
    decision.expect().map_err(ApiError::Forbidden)
}

// --- annotated nav list ----------------------------------------------------

/// One left-nav entry: the board plus the CALLER's annotations.
#[derive(Debug, Serialize, ToSchema)]
pub struct BoardNavEntry {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    /// Group assignment for the collapsible tree (`None` = ungrouped).
    pub group_id: Option<Uuid>,
    pub group_name: Option<String>,
    /// Whether THIS user favorited the board (star).
    pub favorite: bool,
    /// The caller's board role (`None` = no membership row ⇒ org-governed).
    pub my_role: Option<BoardRole>,
}

/// GET /api/boards/nav — every org board annotated with the current
/// user's favorite mark, board role, and group assignment.
#[utoipa::path(operation_id = "boards_nav", get, path = "/api/boards/nav", tag = "boards",
    security(("bearer" = [])),
    responses((status = 200, body = [BoardNavEntry])))]
pub async fn nav(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<BoardNavEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, org_permission_for(BoardOp::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let entries = core_db::boards::list_boards_annotated(&mut *tx, org_id, user_id).await?;
    let groups = core_db::boards::list_groups(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let group_names: std::collections::HashMap<Uuid, String> =
        groups.into_iter().map(|g| (g.id, g.name)).collect();

    Ok(Json(
        entries
            .into_iter()
            .map(|e| BoardNavEntry {
                id: e.board.id,
                name: e.board.name,
                description: e.board.description,
                is_default: e.board.is_default,
                group_id: e.group_id,
                group_name: e.group_id.and_then(|gid| group_names.get(&gid).cloned()),
                favorite: e.favorite,
                my_role: e.my_role,
            })
            .collect(),
    ))
}

// --- board groups ------------------------------------------------------------

/// GET /api/board-groups — the org's groups in nav order (any member may
/// read; rendering input for the tree).
#[utoipa::path(operation_id = "board_groups_list", get, path = "/api/board-groups", tag = "boards",
    security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::BoardGroup])))]
pub async fn list_groups(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::BoardGroup>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, org_permission_for(BoardOp::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let groups = core_db::boards::list_groups(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(groups))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    pub name: String,
    pub position: Option<i32>,
}

/// POST /api/board-groups — creates a nav group (ManageBoards gated);
/// emits `board.group_created`.
#[utoipa::path(operation_id = "board_groups_create", post, path = "/api/board-groups", tag = "boards",
    security(("bearer" = [])), request_body = CreateGroupRequest,
    responses((status = 200, body = core_domain::entities::BoardGroup)))]
pub async fn create_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateGroupRequest>,
) -> ApiResult<Json<core_domain::entities::BoardGroup>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManageBoards)?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("group name is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let position = req.position.unwrap_or(0);

    let group = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let group = core_db::boards::create_group(
            &mut ctx.tx,
            org_id,
            Uuid::now_v7(),
            user_id,
            req.name.trim(),
            position,
        )
        .await?;
        ctx.emit(
            types::BOARD_GROUP_CREATED,
            "board_group",
            Some(group.id.to_string().as_str()),
            &serde_json::to_value(events::BoardGroupCreated {
                group: group.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, group))
    })
    .await?;
    Ok(Json(group))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub position: Option<i32>,
}

/// PATCH /api/board-groups/{id} — renames/reorders a group. ManageBoards
/// OR the group's owner may do this; emits `board.group_updated`.
#[utoipa::path(operation_id = "board_groups_update", patch, path = "/api/board-groups/{id}", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    request_body = UpdateGroupRequest,
    responses((status = 200, body = core_domain::entities::BoardGroup)))]
pub async fn update_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateGroupRequest>,
) -> ApiResult<Json<core_domain::entities::BoardGroup>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("group name cannot be empty".into()));
        }
    }

    let group = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::boards::get_group(&mut *ctx.tx, org_id, id).await?;
        let is_owner = existing.owner_id == user_id;
        if !is_owner {
            require_permission(&authed.principal, Permission::ManageBoards)?;
        }
        let group = core_db::boards::update_group(
            &mut ctx.tx,
            org_id,
            id,
            core_db::boards::GroupPatch {
                name: req.name.as_deref().map(str::trim),
                position: req.position,
            },
        )
        .await?;
        ctx.emit(
            types::BOARD_GROUP_UPDATED,
            "board_group",
            Some(group.id.to_string().as_str()),
            &serde_json::to_value(events::BoardGroupUpdated {
                group_id: group.id,
                name: group.name.clone(),
                position: group.position,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, group))
    })
    .await?;
    Ok(Json(group))
}

/// DELETE /api/board-groups/{id} — removes a group; member boards become
/// ungrouped, never deleted. ManageBoards OR the group's owner; emits
/// `board.group_removed`.
#[utoipa::path(operation_id = "board_groups_delete", delete, path = "/api/board-groups/{id}", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn delete_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::boards::get_group(&mut *ctx.tx, org_id, id).await?;
        if existing.owner_id != user_id {
            require_permission(&authed.principal, Permission::ManageBoards)?;
        }
        core_db::boards::delete_group(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::BOARD_GROUP_REMOVED,
            "board_group",
            Some(id.to_string().as_str()),
            &serde_json::to_value(events::BoardGroupRemoved { group_id: id }).unwrap_or_default(),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignGroupRequest {
    /// `Some(group)` moves the board into that group; `None` ungroups it.
    pub group_id: Option<Uuid>,
}

/// PUT /api/boards/{id}/group — assigns/unassigns the board's nav group.
/// Board-admin gated (ConfigureBoard); emits `board.group_updated`.
#[utoipa::path(operation_id = "boards_assign_group", put, path = "/api/boards/{id}/group", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    request_body = AssignGroupRequest, responses((status = 204)))]
pub async fn assign_group(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AssignGroupRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        core_db::boards::set_board_group(&mut ctx.tx, org_id, id, req.group_id).await?;
        ctx.emit(
            types::BOARD_GROUP_UPDATED,
            "board",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "board_id": id, "group_id": req.group_id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- per-board membership --------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct BoardMemberView {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: BoardRole,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/boards/{id}/members — the roster (ConfigureBoard gated).
#[utoipa::path(operation_id = "boards_list_members", get, path = "/api/boards/{id}/members", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [BoardMemberView])))]
pub async fn list_members(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<BoardMemberView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, org_permission_for(BoardOp::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    authorize_board_op(&mut tx, &authed.principal, id, BoardOp::View).await?;
    let members = core_db::boards::list_members(&mut tx, org_id, id).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(
        members
            .into_iter()
            .map(|(m, display_name)| BoardMemberView {
                user_id: m.user_id,
                display_name,
                role: m.role,
                created_at: m.created_at,
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertMemberRequest {
    pub user_id: Uuid,
    pub role: BoardRole,
}

/// PUT /api/boards/{id}/members — adds or re-roles one membership.
/// ManageMembers gated (board admin); emits `board.membership_updated`.
#[utoipa::path(operation_id = "boards_upsert_member", put, path = "/api/boards/{id}/members", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    request_body = UpsertMemberRequest,
    responses((status = 200, body = BoardMemberView)))]
pub async fn upsert_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpsertMemberRequest>,
) -> ApiResult<Json<BoardMemberView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // The member must exist in THIS org; RLS-scoped lookup proves it.
    let target = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        let t = core_db::users::get_user(&mut *tx, org_id, req.user_id)
            .await
            .map_err(|_| ApiError::NotFound)?;
        tx.commit().await.map_err(ApiError::from)?;
        t
    };

    let view = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ManageMembers).await?;
        let member =
            core_db::boards::upsert_member(&mut ctx.tx, org_id, id, req.user_id, req.role).await?;
        ctx.emit(
            types::BOARD_MEMBERSHIP_UPDATED,
            "board_member",
            Some(member.user_id.to_string().as_str()),
            &serde_json::to_value(events::BoardMembershipUpdated {
                board_id: id,
                user_id: member.user_id,
                role: member.role,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((
            ctx,
            BoardMemberView {
                user_id: member.user_id,
                display_name: target.display_name,
                role: member.role,
                created_at: member.created_at,
            },
        ))
    })
    .await?;
    Ok(Json(view))
}

/// DELETE /api/boards/{id}/members/{user_id} — removes one membership;
/// the user falls back to org-governed access. Emits
/// `board.membership_removed`.
#[utoipa::path(operation_id = "boards_remove_member", delete, path = "/api/boards/{id}/members/{user_id}", tag = "boards",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("user_id" = Uuid, Path)),
    responses((status = 204)))]
pub async fn remove_member(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let actor_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, actor_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ManageMembers).await?;
        core_db::boards::remove_member(&mut ctx.tx, org_id, id, user_id).await?;
        ctx.emit(
            types::BOARD_MEMBERSHIP_REMOVED,
            "board_member",
            Some(user_id.to_string().as_str()),
            &serde_json::to_value(events::BoardMembershipRemoved {
                board_id: id,
                user_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        // Audit the roster change (permission-adjacent mutation, §6).
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor_id),
                action: "board.member_remove".into(),
                entity_type: Some("board".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "user_id": user_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
