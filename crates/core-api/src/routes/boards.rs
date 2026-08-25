//! Board endpoints: boards, their semantic-category-bound statuses, and
//! user favorites (board groups / my-work inputs).

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::{Module, Permission};
use core_domain::status::TicketStatusCategory;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

fn board_view() -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action: core_domain::policy::Action::View,
    }
}

fn board_manage() -> Permission {
    Permission::ManageBoards
}

/// GET /api/boards
#[utoipa::path(get, path = "/api/boards", tag = "boards", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::Board])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::Board>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    let boards = core_db::boards::list_boards(&state.pool, authed.principal.user.org_id).await?;
    Ok(Json(boards))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBoardRequest {
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
}

/// POST /api/boards — creates a board seeded with the four default statuses.
#[utoipa::path(post, path = "/api/boards", tag = "boards", security(("bearer" = [])),
    request_body = CreateBoardRequest,
    responses((status = 200, body = core_domain::entities::Board)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateBoardRequest>,
) -> ApiResult<Json<core_domain::entities::Board>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    // One connection for the board + its seeded statuses.
    let mut conn = state.pool.acquire().await?;
    let board = core_db::boards::create_board(
        &mut *conn,
        org_id,
        Uuid::now_v7(),
        &req.name,
        req.description.as_deref(),
        req.is_default,
    )
    .await?;
    // Every new board gets the semantic defaults; customization adds more.
    let defaults = [
        ("New", TicketStatusCategory::Open, 0, true),
        ("In Progress", TicketStatusCategory::InProgress, 10, false),
        ("Waiting", TicketStatusCategory::Waiting, 20, false),
        ("Resolved", TicketStatusCategory::Closed, 30, false),
    ];
    for (name, category, sort, is_entry) in defaults {
        core_db::boards::add_status(
            &mut conn,
            org_id,
            Uuid::now_v7(),
            board.id,
            name,
            category,
            sort,
            is_entry,
        )
        .await?;
    }
    Ok(Json(board))
}

/// GET /api/boards/{id}/statuses
#[utoipa::path(get, path = "/api/boards/{id}/statuses", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::BoardStatus])))]
pub async fn list_statuses(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::BoardStatus>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    let statuses = core_db::boards::list_statuses(&state.pool, id).await?;
    Ok(Json(statuses))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateStatusRequest {
    pub name: String,
    /// The semantic category this status binds to — behavior (SLA pauses,
    /// billing, automations) reads the category, never the label (§5).
    pub category: TicketStatusCategory,
    pub sort_order: i32,
    pub is_entry: bool,
}

/// POST /api/boards/{id}/statuses — adds a custom status bound to a
/// semantic category.
#[utoipa::path(post, path = "/api/boards/{id}/statuses", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = CreateStatusRequest,
    responses((status = 200, body = core_domain::entities::BoardStatus)))]
pub async fn add_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateStatusRequest>,
) -> ApiResult<Json<core_domain::entities::BoardStatus>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let mut conn = state.pool.acquire().await?;
    let status = core_db::boards::add_status(
        &mut conn,
        authed.principal.user.org_id,
        Uuid::now_v7(),
        id,
        &req.name,
        req.category,
        req.sort_order,
        req.is_entry,
    )
    .await?;
    Ok(Json(status))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FavoriteRequest {
    pub board_id: Uuid,
    pub favorite: bool,
}

/// POST /api/boards/favorite — toggles a board favorite for the current user.
#[utoipa::path(post, path = "/api/boards/favorite", tag = "boards", security(("bearer" = [])),
    request_body = FavoriteRequest, responses((status = 204)))]
pub async fn toggle_favorite(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<FavoriteRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let user_id = authed.principal.user.id;
    if req.favorite {
        core_db::boards::add_favorite(&state.pool, user_id, req.board_id, 0).await?;
    } else {
        core_db::boards::remove_favorite(&state.pool, user_id, req.board_id).await?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
