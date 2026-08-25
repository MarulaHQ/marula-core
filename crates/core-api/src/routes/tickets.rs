//! Ticket endpoints: lifecycle, structured notes, pre-close checklists.
//! Every mutation writes the outbox event in the SAME transaction as the
//! data change (§4.2) and updates SLA clock state (§1).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::{self, types};
use core_domain::policy::{Module, Permission};
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix { module: Module::Tickets, action }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTicketRequest {
    pub board_id: Uuid,
    pub title: String,
    pub body: Option<String>,
    pub priority: Priority,
    pub client_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub contact_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchTicketRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<Priority>,
    /// `Some(Some(id))` assigns; `Some(None)` unassigns; `None` = no change.
    pub assignee_id: Option<Option<Uuid>>,
    pub client_id: Option<Option<Uuid>>,
    pub site_id: Option<Option<Uuid>>,
    pub contact_id: Option<Option<Uuid>>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangeStatusRequest {
    pub status_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddCommentRequest {
    pub body: String,
    pub kind: NoteKind,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChecklistItemRequest {
    pub text: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetChecklistItemRequest {
    pub checked: bool,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TicketListQuery {
    pub board_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub client_id: Option<Uuid>,
    pub category: Option<TicketStatusCategory>,
    pub priority: Option<Priority>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/tickets — filtered, priority-ordered list.
#[utoipa::path(get, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    params(TicketListQuery),
    responses((status = 200, body = [core_domain::entities::Ticket])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TicketListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::Ticket>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let rows = core_db::tickets::list_tickets(
        &state.pool,
        authed.principal.user.org_id,
        core_db::tickets::TicketFilter {
            board_id: q.board_id,
            assignee_id: q.assignee_id,
            client_id: q.client_id,
            category: q.category,
            priority: q.priority,
            limit: q.limit.clamp(1, 200),
            offset: q.offset.max(0),
        },
    )
    .await?;
    Ok(Json(rows))
}

/// GET /api/tickets/my-work — everything assigned to me, not closed,
/// across all boards (the unified cross-board view from §1).
#[utoipa::path(get, path = "/api/tickets/my-work", tag = "tickets", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::Ticket])))]
pub async fn my_work(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::Ticket>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let rows =
        core_db::tickets::my_work(&state.pool, authed.principal.user.org_id, authed.principal.user.id, 100).await?;
    Ok(Json(rows))
}

/// POST /api/tickets — allocates the human ticket number, initializes SLA
/// clocks, and emits `ticket.created` — all in one transaction.
#[utoipa::path(post, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    request_body = CreateTicketRequest,
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTicketRequest>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Create))?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let status = core_db::boards::default_status_for_board(&mut ctx.tx, req.board_id).await?;
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut ctx.tx, org_id).await?;
        let number = format!("{prefix}-{seq}");
        let ticket = core_db::tickets::insert_ticket(
            &mut ctx.tx,
            core_db::tickets::NewTicket {
                id: Uuid::now_v7(),
                org_id,
                number: &number,
                board_id: req.board_id,
                status_id: status.id,
                title: req.title.trim(),
                body: req.body.as_deref(),
                priority: req.priority,
                client_id: req.client_id,
                site_id: req.site_id,
                contact_id: req.contact_id,
                assignee_id: req.assignee_id,
                requester_id: None,
                due_at: req.due_at,
            },
        )
        .await?;

        // SLA clocks start now, under the org's default policy.
        if let Ok(policy) = core_db::sla::get_default_policy(&mut ctx.tx, org_id).await {
            let sla = core_domain::entities::TicketSlaState::init(
                org_id,
                ticket.id,
                &policy,
                ticket.priority,
                ticket.created_at,
            );
            core_db::sla::upsert_ticket_state(&mut ctx.tx, &sla).await?;
        }

        ctx.emit(
            types::TICKET_CREATED,
            "ticket",
            Some(ticket.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCreated {
                ticket_id: ticket.id,
                number: ticket.number.0.clone(),
                board_id: ticket.board_id,
                title: ticket.title.clone(),
                priority: ticket.priority,
                client_id: ticket.client_id,
                assignee_id: ticket.assignee_id,
                created_at: ticket.created_at,
            })
            .unwrap_or_default(),
        )
        .await?;

        Ok((ctx, ticket))
    })
    .await?;

    Ok(Json(ticket))
}

/// GET /api/tickets/{id}
#[utoipa::path(get, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let ticket = core_db::tickets::get_ticket(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(ticket))
}

/// PATCH /api/tickets/{id} — partial update; emits `ticket.updated` and
/// `ticket.assigned` when the assignee changes.
#[utoipa::path(patch, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTicketRequest,
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTicketRequest>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::tickets::get_ticket(&mut ctx.tx, org_id, id).await?;
        core_db::tickets::update_ticket(
            &mut ctx.tx,
            org_id,
            id,
            core_db::tickets::TicketPatch {
                title: req.title.as_deref(),
                body: req.body.as_deref(),
                priority: req.priority,
                assignee_id: req.assignee_id,
                client_id: req.client_id,
                site_id: req.site_id,
                contact_id: req.contact_id,
                due_at: req.due_at,
            },
        )
        .await?;
        let after = core_db::tickets::get_ticket(&mut ctx.tx, org_id, id).await?;

        ctx.emit(
            types::TICKET_UPDATED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "ticket_id": after.id }),
        )
        .await?;
        if before.assignee_id != after.assignee_id {
            ctx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketAssigned {
                    ticket_id: after.id,
                    assignee_id: after.assignee_id,
                    assigned_by: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
        }
        Ok((ctx, after))
    })
    .await?;

    Ok(Json(ticket))
}

/// POST /api/tickets/{id}/status — the guarded lifecycle transition:
/// domain transition rules, pre-close checklist enforcement, SLA pause
/// bookkeeping, and the `ticket.status_changed` event, all in one tx.
#[utoipa::path(post, path = "/api/tickets/{id}/status", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ChangeStatusRequest,
    responses((status = 200, body = core_domain::entities::Ticket),
              (status = 422, description = "checklist incomplete or invalid transition")))]
pub async fn change_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeStatusRequest>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut ctx.tx, org_id, id).await?;
        let target = core_db::boards::get_status(&mut ctx.tx, req.status_id).await?;

        // 1. Domain transition rules (closed tickets cannot go straight back to Open).
        if !ticket.can_transition_to(&target) {
            return Err(ApiError::Validation(format!(
                "transition {} → {} not allowed",
                ticket.category, target.category
            )));
        }
        // 2. Pre-close checklist: closing requires every item checked (§1).
        let checklist = core_db::tickets::list_checklist(&mut ctx.tx, org_id, id).await?;
        core_domain::entities::validate_pre_close(&checklist, &target)?;

        // 3. SLA pause bookkeeping binds to the semantic category.
        let now = chrono::Utc::now();
        if let Ok(mut sla) = core_db::sla::get_ticket_state(&mut ctx.tx, id).await {
            if let Some(mut sla_state) = sla.take() {
                let calendar = {
                    let policy = core_db::sla::get_default_policy(&mut ctx.tx, org_id).await?;
                    policy.calendar
                };
                sla_state.apply_category(target.category, now, &calendar);
                core_db::sla::upsert_ticket_state(&mut ctx.tx, &sla_state).await?;
            }
        }

        core_db::tickets::update_status(&mut ctx.tx, org_id, id, req.status_id).await?;
        let after = core_db::tickets::get_ticket(&mut ctx.tx, org_id, id).await?;

        ctx.emit(
            types::TICKET_STATUS_CHANGED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketStatusChanged {
                ticket_id: after.id,
                from_status: ticket.status_id.to_string(),
                to_status: target.name.clone(),
                from_category: ticket.category,
                to_category: target.category,
                changed_by: Some(user_id),
                occurred_at: now,
            })
            .unwrap_or_default(),
        )
        .await?;
        if target.category == TicketStatusCategory::Closed {
            ctx.emit(
                types::TICKET_CLOSED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::json!({ "ticket_id": after.id, "resolved_at": after.resolved_at }),
            )
            .await?;
        }

        // Audit the lifecycle change.
        core_db::audit::insert(&mut ctx.tx, &AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: Actor::Human(user_id),
            action: "ticket.status_change".into(),
            entity_type: Some("ticket".into()),
            entity_id: Some(after.id.to_string()),
            detail: serde_json::json!({ "from": ticket.category.as_str(), "to": target.category.as_str() }),
            ip: None,
            occurred_at: now,
        })
        .await?;

        Ok((ctx, after))
    })
    .await?;

    Ok(Json(ticket))
}

/// GET /api/tickets/{id}/comments — internal notes require ticket view;
/// the portal surface filters kinds separately (portal v1).
#[utoipa::path(get, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::TicketComment])))]
pub async fn list_comments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TicketComment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let comments =
        core_db::tickets::list_comments(&state.pool, authed.principal.user.org_id, id, None).await?;
    Ok(Json(comments))
}

/// POST /api/tickets/{id}/comments — adds a structured note; @mentions
/// fan out into the notification center.
#[utoipa::path(post, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddCommentRequest,
    responses((status = 200, body = core_domain::entities::TicketComment)))]
pub async fn add_comment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> ApiResult<Json<core_domain::entities::TicketComment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Edit))?;
    if req.body.trim().is_empty() {
        return Err(ApiError::Validation("comment body is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let comment = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let comment = core_db::tickets::insert_comment(
            &mut ctx.tx,
            Uuid::now_v7(),
            org_id,
            id,
            Some(user_id),
            req.kind,
            req.body.trim(),
        )
        .await?;

        // Resolve @mentions to org users → notifications.
        let handles = events::extract_mentions(req.body.trim());
        let mut mentioned = Vec::new();
        if !handles.is_empty() {
            let users = core_db::users::list_users(&mut ctx.tx, org_id).await?;
            for u in users {
                let handle = u.email.split('@').next().unwrap_or_default().to_lowercase();
                if handles.iter().any(|h| h.eq_ignore_ascii_case(&handle)) && u.id != user_id {
                    mentioned.push(u.id);
                }
            }
        }
        for mentioned_user in &mentioned {
            core_db::notifications::insert(
                &mut ctx.tx,
                Uuid::now_v7(),
                org_id,
                *mentioned_user,
                "mention",
                Some(id),
                &format!("mentioned on {}", comment.ticket_id),
                Some(req.body.trim()),
            )
            .await?;
        }

        ctx.emit(
            types::TICKET_COMMENT_ADDED,
            "ticket_comment",
            Some(comment.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCommentAdded {
                comment: comment.clone(),
                mentioned_user_ids: mentioned,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, comment))
    })
    .await?;

    Ok(Json(comment))
}

/// GET /api/tickets/{id}/checklist
#[utoipa::path(get, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::ChecklistItem])))]
pub async fn get_checklist(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::ChecklistItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let items = core_db::tickets::list_checklist(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(items))
}

/// POST /api/tickets/{id}/checklist — appends a pre-close checklist item.
#[utoipa::path(post, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddChecklistItemRequest,
    responses((status = 200, body = [core_domain::entities::ChecklistItem])))]
pub async fn add_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChecklistItemRequest>,
) -> ApiResult<Json<Vec<core_domain::entities::ChecklistItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let existing = core_db::tickets::list_checklist(&state.pool, org_id, id).await?;
    core_db::tickets::add_checklist_item(
        &state.pool,
        Uuid::now_v7(),
        org_id,
        id,
        req.text.trim(),
        existing.len() as i32,
    )
    .await?;
    let items = core_db::tickets::list_checklist(&state.pool, org_id, id).await?;
    Ok(Json(items))
}

/// POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item.
#[utoipa::path(post, path = "/api/tickets/{id}/checklist/{item_id}", tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("item_id" = Uuid, Path)),
    request_body = SetChecklistItemRequest,
    responses((status = 200, body = [core_domain::entities::ChecklistItem])))]
pub async fn set_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetChecklistItemRequest>,
) -> ApiResult<Json<Vec<core_domain::entities::ChecklistItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    core_db::tickets::set_checklist_item(&state.pool, org_id, item_id, req.checked, Some(authed.principal.user.id))
        .await?;
    let items = core_db::tickets::list_checklist(&state.pool, org_id, id).await?;
    Ok(Json(items))
}

/// GET /api/tickets/{id}/time-entries
#[utoipa::path(get, path = "/api/tickets/{id}/time-entries", tag = "tickets",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::TimeEntry])))]
pub async fn ticket_time_entries(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TimeEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(core_domain::policy::Action::View))?;
    let entries = core_db::timers::list_time_entries(
        &state.pool,
        authed.principal.user.org_id,
        Some(id),
        None,
        None,
        200,
    )
    .await?;
    Ok(Json(entries))
}
