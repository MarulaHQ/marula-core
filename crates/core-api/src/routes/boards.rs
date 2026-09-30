//! Board endpoints: boards, their semantic-category-bound statuses,
//! explicit transition matrices, and user favorites (board groups /
//! my-work inputs). Every mutation emits an outbox event in-tx (§4.2).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::Actor;
use core_domain::events::{self, types};
use core_domain::policy::{BoardOp, Module, Permission};
use core_domain::status::TicketStatusCategory;
use core_domain::workflow::{self, TransitionEdge};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::board_access::authorize_board_op;

fn board_view() -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action: core_domain::policy::Action::View,
    }
}

fn board_manage() -> Permission {
    Permission::ManageBoards
}

fn validate_wip_limit(limit: i32) -> ApiResult<()> {
    if !(1..=1000).contains(&limit) {
        return Err(ApiError::Validation(
            "wip_limit must be between 1 and 1000".into(),
        ));
    }
    Ok(())
}

/// Maps the decoded WIP-cap patch onto the DB patch: Unchanged = `None`
/// (COALESCE), Clear = `Some(None)`, Set(n) = `Some(Some(n))`.
fn parse_wip_limit_update(req: &UpdateStatusRequest) -> Option<Option<i32>> {
    match req.wip_limit {
        WipLimitUpdate::Unchanged => None,
        WipLimitUpdate::Clear => Some(None),
        WipLimitUpdate::Set(limit) => Some(Some(limit)),
    }
}

/// GET /api/boards
#[utoipa::path(operation_id = "boards_list", get, path = "/api/boards", tag = "boards", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::Board])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::Board>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let boards = core_db::boards::list_boards(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(boards))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBoardRequest {
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
}

/// POST /api/boards — creates a board seeded with the four default statuses.
#[utoipa::path(operation_id = "boards_create", post, path = "/api/boards", tag = "boards", security(("bearer" = [])),
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
    let user_id = authed.principal.user.id;

    let board = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // Every new board gets the semantic defaults; customization adds more.
        let defaults = [
            ("New", TicketStatusCategory::Open, 0, true),
            ("In Progress", TicketStatusCategory::InProgress, 10, false),
            ("Waiting", TicketStatusCategory::Waiting, 20, false),
            ("Resolved", TicketStatusCategory::Closed, 30, false),
        ];
        let board = core_db::boards::create_board(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            &req.name,
            req.description.as_deref(),
            req.is_default,
        )
        .await?;
        for (name, category, sort, is_entry) in defaults {
            core_db::boards::add_status(
                &mut ctx.tx,
                org_id,
                Uuid::now_v7(),
                board.id,
                name,
                category,
                sort,
                is_entry,
                None,
            )
            .await?;
        }
        ctx.emit(
            types::BOARD_CREATED,
            "board",
            Some(board.id.to_string().as_str()),
            &serde_json::json!({ "board_id": board.id, "name": board.name }),
        )
        .await?;
        Ok((ctx, board))
    })
    .await?;

    Ok(Json(board))
}

/// GET /api/boards/{id}/statuses
#[utoipa::path(operation_id = "boards_list_statuses", get, path = "/api/boards/{id}/statuses", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::BoardStatus])))]
pub async fn list_statuses(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::BoardStatus>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let statuses = core_db::boards::list_statuses(&mut *tx, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
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
    /// Optional per-column WIP cap (0089). 1..=1000 — the DB CHECK is the
    /// backstop; this route validates before the write.
    pub wip_limit: Option<i32>,
}

/// GET /api/boards/{id}/statuses/usage — per-status ticket counts and
/// automation references (Wave 41 lane 12).
///
/// One read serves both the settings table's TICKETS column and the delete
/// confirm's "12 tickets use this status" and "2 automations mention this
/// status", so the table and the confirm can never quote different numbers for
/// the same status.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct BoardStatusUsage {
    pub status_id: Uuid,
    /// Tickets currently in this status, closed ones included: the column is
    /// where work has been, not a live queue.
    pub ticket_count: i64,
    /// Automation rules naming this status. They are PAUSED, never silently
    /// repointed, if the status is deleted.
    pub automation_count: i64,
}

#[utoipa::path(operation_id = "boards_status_usage", get, path = "/api/boards/{id}/statuses/usage", tag = "boards",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [BoardStatusUsage])))]
pub async fn status_usage(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<BoardStatusUsage>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::View).await?;
        let usage = core_db::boards::status_usage(&mut *ctx.tx, org_id, id).await?;
        Ok((ctx, usage))
    })
    .await
    .map(|usage| {
        Json(
            usage
                .into_iter()
                .map(|u| BoardStatusUsage {
                    status_id: u.status_id,
                    ticket_count: u.ticket_count,
                    automation_count: u.automation_count,
                })
                .collect(),
        )
    })
}

/// POST /api/boards/{id}/statuses — adds a custom status bound to a
/// semantic category; emits `board.status_added`.
#[utoipa::path(operation_id = "boards_add_status", post, path = "/api/boards/{id}/statuses", tag = "boards", security(("bearer" = [])),
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
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("status name is required".into()));
    }
    if let Some(limit) = req.wip_limit {
        validate_wip_limit(limit)?;
    }
    let status = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let status = core_db::boards::add_status(
            &mut ctx.tx,
            org_id,
            Uuid::now_v7(),
            id,
            req.name.trim(),
            req.category,
            req.sort_order,
            req.is_entry,
            req.wip_limit,
        )
        .await?;
        ctx.emit(
            types::BOARD_STATUS_ADDED,
            "board_status",
            Some(status.id.to_string().as_str()),
            &serde_json::to_value(events::BoardStatusAdded {
                board_id: id,
                status: status.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, status))
    })
    .await?;
    Ok(Json(status))
}

/// WIP-cap patch input (0089): `Default` (field absent) = unchanged;
/// JSON null = clear; a number = set. serde's `Option<Option>` mapping
/// makes a present-but-null field UNREPRESENTABLE from JSON (null folds
/// to the outer None), so the three states ride an explicit enum.
#[derive(Debug, Clone, Copy, PartialEq, Default, ToSchema)]
pub enum WipLimitUpdate {
    #[default]
    Unchanged,
    Clear,
    Set(i32),
}

impl<'de> serde::Deserialize<'de> for WipLimitUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::Null => Ok(Self::Clear),
            other => {
                let n = other.as_i64().ok_or_else(|| {
                    serde::de::Error::custom("wip_limit must be a number or null")
                })?;
                let limit = i32::try_from(n)
                    .map_err(|_| serde::de::Error::custom("wip_limit is out of range"))?;
                Ok(Self::Set(limit))
            }
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateStatusRequest {
    pub name: Option<String>,
    /// Re-binds the semantic category; behavior follows immediately (§5).
    pub category: Option<TicketStatusCategory>,
    pub sort_order: Option<i32>,
    pub is_entry: Option<bool>,
    /// WIP cap (0089): absent = unchanged; JSON `null` = CLEAR the cap; a
    /// number = set it (1..=1000 — the DB CHECK is the backstop).
    #[serde(default)]
    pub wip_limit: WipLimitUpdate,
    /// Wave 41 lane 12: whether the portal shows this status's own label to a
    /// client. Absent = unchanged.
    pub client_visible: Option<bool>,
}

/// PATCH /api/boards/{id}/statuses/{status_id} — rebinds/reorders/renames a
/// status; emits `board.status_updated`.
#[utoipa::path(operation_id = "boards_update_status", patch, path = "/api/boards/{id}/statuses/{status_id}", tag = "boards",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("status_id" = Uuid, Path)),
    request_body = UpdateStatusRequest,
    responses((status = 200, body = core_domain::entities::BoardStatus)))]
pub async fn update_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, status_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateStatusRequest>,
) -> ApiResult<Json<core_domain::entities::BoardStatus>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("status name cannot be empty".into()));
        }
    }
    if let WipLimitUpdate::Set(limit) = req.wip_limit {
        validate_wip_limit(limit)?;
    }
    let status = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let status = core_db::boards::update_status(
            &mut ctx.tx,
            status_id,
            core_db::boards::StatusPatch {
                name: req.name.as_deref().map(str::trim),
                category: req.category,
                sort_order: req.sort_order,
                is_entry: req.is_entry,
                wip_limit: parse_wip_limit_update(&req),
                client_visible: req.client_visible,
            },
        )
        .await?;
        // The RLS-scoped read above already proved same-org; keep the path
        // honest by confirming the status belongs to the routed board.
        if status.board_id != id {
            return Err(ApiError::NotFound);
        }
        ctx.emit(
            types::BOARD_STATUS_UPDATED,
            "board_status",
            Some(status.id.to_string().as_str()),
            &serde_json::to_value(events::BoardStatusUpdated {
                board_id: id,
                status: status.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, status))
    })
    .await?;
    Ok(Json(status))
}

/// What a delete with reassignment actually did (Wave 41 lane 12).
///
/// Returned instead of a bare 204 because the confirm has to say what
/// happened, and the admin is the one who has to believe it.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct DeleteStatusResult {
    /// Tickets moved to the replacement status before the delete.
    pub moved_tickets: i64,
    /// Automation rules that named this status and were PAUSED. Never
    /// repointed: an automation's conditions are the org's own logic.
    pub paused_automations: i64,
    /// SLA pause states opened or closed by the move.
    pub sla_pauses_changed: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct DeleteStatusQuery {
    /// Move every ticket in this status to `reassign_to` and then delete it.
    /// Must be another status on the same board. Absent, a status holding
    /// tickets is refused with 409 and nothing changes.
    pub reassign_to: Option<Uuid>,
}

/// DELETE /api/boards/{id}/statuses/{status_id} — removes a custom status.
///
/// Without `reassign_to` a status holding tickets is refused (409), as before.
/// With it, every ticket moves first — set-based, mirroring the per-ticket
/// status-change invariants exactly (denormalised category, one-shot
/// `resolved_at`, SLA pause bookkeeping) — and each moved ticket gets its own
/// `ticket.status_changed` outbox row, so a bulk move is as auditable as a
/// single one. Automations naming the status are paused, never repointed.
/// Emits `board.status_removed`.
#[utoipa::path(operation_id = "boards_delete_status", delete, path = "/api/boards/{id}/statuses/{status_id}", tag = "boards",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("status_id" = Uuid, Path), DeleteStatusQuery),
    responses((status = 200, body = DeleteStatusResult), (status = 409)))]
pub async fn delete_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, status_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<DeleteStatusQuery>,
) -> ApiResult<Json<DeleteStatusResult>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let result = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, id).await?;
        let target = statuses
            .iter()
            .find(|s| s.id == status_id)
            .ok_or(ApiError::NotFound)?;
        let entry_count = statuses.iter().filter(|s| s.is_entry).count();
        if target.is_entry && entry_count <= 1 {
            return Err(ApiError::Conflict(
                "cannot delete the board's only entry status — promote another first".into(),
            ));
        }
        let name = target.name.clone();
        let from_category = target.category;

        // The replacement, when asked for, must exist on this board and must
        // not be the status being deleted.
        let replacement = match query.reassign_to {
            None => None,
            Some(reassign_to) if reassign_to == status_id => {
                return Err(ApiError::Validation(
                    "a status cannot replace itself".into(),
                ));
            }
            Some(reassign_to) => {
                let row = statuses
                    .iter()
                    .find(|s| s.id == reassign_to)
                    .ok_or_else(|| {
                        ApiError::Validation(
                            "the replacement status is not on this board".into(),
                        )
                    })?;
                Some(row.clone())
            }
        };

        let now = chrono::Utc::now();
        let mut moved = 0i64;
        let mut sla_pauses_changed = 0u64;
        if let Some(to) = replacement.as_ref() {
            // 1. Tickets first: a bulk move that mirrors `update_status`.
            let moved_ids =
                core_db::tickets::reassign_status_bulk(&mut *ctx.tx, org_id, status_id, to.id)
                    .await?;
            moved = moved_ids.len() as i64;

            // 2. SLA pause bookkeeping, the same category rule the per-ticket
            //    path applies, set-based. Closing an open pause costs business
            //    seconds, so the seconds are computed per policy in Rust.
            if to.category.pauses_sla() {
                sla_pauses_changed +=
                    core_db::sla::apply_pause_bulk(&mut *ctx.tx, &moved_ids, now).await?;
            } else {
                let pauses =
                    core_db::sla::open_pauses_for_tickets(&mut *ctx.tx, &moved_ids).await?;
                let mut elapsed: Vec<(Uuid, i64)> = Vec::with_capacity(pauses.len());
                for pause in pauses {
                    let calendar =
                        match core_db::sla::get_policy_record(&mut *ctx.tx, org_id, pause.policy_id)
                            .await
                        {
                            Ok(record) => core_db::sla::resolve_policy_calendar(
                                &mut *ctx.tx,
                                &record,
                            )
                            .await
                            .unwrap_or_else(|_| record.calendar.clone()),
                            Err(_) => continue,
                        };
                    elapsed.push((
                        pause.ticket_id,
                        calendar.elapsed_business_secs(pause.paused_since, now),
                    ));
                }
                sla_pauses_changed += core_db::sla::apply_resume_bulk(&mut *ctx.tx, &elapsed).await?;
            }

            // 3. One timeline row per moved ticket. A bulk move that wrote no
            //    events would be the one status change an admin cannot audit.
            for ticket_id in &moved_ids {
                ctx.emit(
                    types::TICKET_STATUS_CHANGED,
                    "ticket",
                    Some(ticket_id.to_string().as_str()),
                    &serde_json::to_value(events::TicketStatusChanged {
                        ticket_id: *ticket_id,
                        board_id: id,
                        assignee_id: None,
                        from_status: name.clone(),
                        to_status: to.name.clone(),
                        from_category: from_category,
                        to_category: to.category,
                        changed_by: Some(user_id),
                        occurred_at: now,
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        }

        // 4. Automations that named the status are paused, so none is left
        //    pointing at something that no longer exists.
        let paused_automations =
            core_db::boards::pause_automations_mentioning(&mut *ctx.tx, org_id, status_id).await?;

        // 5. The delete itself. Still RESTRICT-guarded: anything the bulk move
        //    did not cover (a status in another board, say) is refused rather
        //    than silently orphaned.
        match core_db::boards::delete_status(&mut *ctx.tx, status_id).await {
            Ok(()) => {}
            Err(core_db::DbError::Sql(sqlx::Error::Database(db))) if db.constraint().is_some() => {
                return Err(ApiError::Conflict(
                    "status is referenced by tickets and cannot be deleted".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        }
        ctx.emit(
            types::BOARD_STATUS_REMOVED,
            "board_status",
            Some(status_id.to_string().as_str()),
            &serde_json::to_value(events::BoardStatusRemoved {
                board_id: id,
                status_id,
                name,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((
            ctx,
            DeleteStatusResult {
                moved_tickets: moved,
                paused_automations,
                sla_pauses_changed: sla_pauses_changed as i64,
            },
        ))
    })
    .await?;
    Ok(Json(result))
}

/// GET response for a board's workflow: either its explicit matrix or the
/// marker that the semantic-category floor governs.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct BoardTransitionsResponse {
    /// `"explicit"` when the listed edges are authoritative; `"floor"` when
    /// the semantic-category default governs (any→any except closed→open).
    pub mode: String,
    pub edges: Vec<TransitionEdge>,
}

/// GET /api/boards/{id}/transitions — the board's effective workflow.
#[utoipa::path(operation_id = "boards_list_transitions", get, path = "/api/boards/{id}/transitions", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = BoardTransitionsResponse)))]
pub async fn list_transitions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<BoardTransitionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_view())?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let edges = core_db::boards::list_transitions(&mut *tx, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let mode = if edges.is_empty() {
        "floor"
    } else {
        "explicit"
    };
    Ok(Json(BoardTransitionsResponse {
        mode: mode.into(),
        edges,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTransitionsRequest {
    /// The complete allowed matrix. Empty list returns the board to
    /// floor-mode governance.
    pub edges: Vec<TransitionEdge>,
}

/// PUT /api/boards/{id}/transitions — atomically replaces the explicit
/// transition matrix; emits `board.transitions_updated`. Endpoints must be
/// statuses of THIS board and self-transitions are rejected.
#[utoipa::path(operation_id = "boards_set_transitions", put, path = "/api/boards/{id}/transitions", tag = "boards", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = SetTransitionsRequest,
    responses((status = 200, body = BoardTransitionsResponse)))]
pub async fn set_transitions(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<SetTransitionsRequest>,
) -> ApiResult<Json<BoardTransitionsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, board_manage())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let response = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        authorize_board_op(&mut ctx.tx, &authed.principal, id, BoardOp::ConfigureBoard).await?;
        // Validate every edge endpoint against this board's statuses before
        // writing; storage FKs are only the backstop.
        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, id).await?;
        let mut edges: Vec<TransitionEdge> = Vec::with_capacity(req.edges.len());
        for edge in req.edges {
            workflow::validate_transition(&statuses, &[], edge.from_status_id, edge.to_status_id)
                .map_err(|rej| ApiError::Validation(rej.message()))?;
            if !edges.contains(&edge) {
                edges.push(edge);
            }
        }
        core_db::boards::replace_transitions(&mut ctx.tx, org_id, id, &edges).await?;

        // Audit the workflow change (board-admin action).
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "board.transitions_update".into(),
                entity_type: Some("board".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "edge_count": edges.len() }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        ctx.emit(
            types::BOARD_TRANSITIONS_UPDATED,
            "board",
            Some(id.to_string().as_str()),
            &serde_json::to_value(events::BoardTransitionsUpdated {
                board_id: id,
                edges: edges.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mode = if edges.is_empty() {
            "floor"
        } else {
            "explicit"
        };
        Ok((
            ctx,
            BoardTransitionsResponse {
                mode: mode.into(),
                edges,
            },
        ))
    })
    .await?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FavoriteRequest {
    pub board_id: Uuid,
    pub favorite: bool,
}

/// POST /api/boards/favorite — toggles a board favorite for the current
/// user (user-scoped; any authenticated member may star boards they can
/// see). Emits `board.favorite_toggled` in-tx.
#[utoipa::path(operation_id = "boards_toggle_favorite", post, path = "/api/boards/favorite", tag = "boards", security(("bearer" = [])),
    request_body = FavoriteRequest, responses((status = 204)))]
pub async fn toggle_favorite(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<FavoriteRequest>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // The starred board must exist in the caller's org — the favorites
    // table's own RLS cannot see cross-org board ids. RLS-scoped read +
    // write (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::boards::get_board(&mut *tx, org_id, req.board_id).await?;
    if req.favorite {
        core_db::boards::add_favorite(&mut *tx, user_id, req.board_id, 0).await?;
    } else {
        core_db::boards::remove_favorite(&mut *tx, user_id, req.board_id).await?;
    }
    // Favorites are per-user presentation state; a dedicated event keeps
    // nav clients consistent without them polling (§4.2 every-mutation).
    core_db::outbox::insert(
        &mut tx,
        Uuid::now_v7(),
        org_id,
        types::BOARD_FAVORITE_TOGGLED,
        "board_favorite",
        Some(req.board_id.to_string().as_str()),
        &serde_json::to_value(events::BoardFavoriteToggled {
            user_id,
            board_id: req.board_id,
            favorite: req.favorite,
        })
        .unwrap_or_default(),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
