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
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use core_domain::workflow;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn tickets(action: core_domain::policy::Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
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

/// Live SLA clock readout for one ticket (badge + countdown surfaces).
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketSlaStatus {
    pub state: core_domain::entities::TicketSlaState,
    /// Business minutes left on the resolution budget (negative = over).
    pub minutes_remaining: Option<i64>,
    /// True while a Waiting-category pause is open.
    pub paused: bool,
    pub breached: bool,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/tickets — filtered, priority-ordered list.
#[utoipa::path(operation_id = "tickets_list", get, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    params(TicketListQuery),
    responses((status = 200, body = [core_domain::entities::Ticket])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TicketListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::Ticket>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
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
#[utoipa::path(operation_id = "tickets_my_work", get, path = "/api/tickets/my-work", tag = "tickets", security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::Ticket])))]
pub async fn my_work(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::Ticket>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let rows = core_db::tickets::my_work(
        &state.pool,
        authed.principal.user.org_id,
        authed.principal.user.id,
        100,
    )
    .await?;
    Ok(Json(rows))
}

/// POST /api/tickets — allocates the human ticket number, initializes SLA
/// clocks, and emits `ticket.created` — all in one transaction.
#[utoipa::path(operation_id = "tickets_create", post, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    request_body = CreateTicketRequest,
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTicketRequest>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Create),
    )?;
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let status = core_db::boards::default_status_for_board(&mut *ctx.tx, req.board_id).await?;
        let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, org_id).await?;
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
        if let Ok(policy) = core_db::sla::get_default_policy(&mut *ctx.tx, org_id).await {
            let sla = core_domain::entities::TicketSlaState::init(
                org_id,
                ticket.id,
                &policy,
                ticket.priority,
                ticket.created_at,
            );
            core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla).await?;
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
#[utoipa::path(operation_id = "tickets_get_one", get, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let ticket =
        core_db::tickets::get_ticket(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(ticket))
}

/// PATCH /api/tickets/{id} — partial update; emits `ticket.updated` and
/// `ticket.assigned` when the assignee changes.
#[utoipa::path(operation_id = "tickets_patch", patch, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTicketRequest,
    responses((status = 200, body = core_domain::entities::Ticket)))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTicketRequest>,
) -> ApiResult<Json<core_domain::entities::Ticket>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
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
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

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
#[utoipa::path(operation_id = "tickets_change_status", post, path = "/api/tickets/{id}/status", tag = "tickets", security(("bearer" = [])),
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
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let ticket = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        // The board's statuses + explicit transition matrix define what is
        // legal here; with no explicit matrix the semantic-category floor
        // governs (core-domain `workflow`).
        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, ticket.board_id).await?;
        let edges = core_db::boards::list_transitions(&mut *ctx.tx, ticket.board_id).await?;
        if let Err(rejection) =
            workflow::validate_transition(&statuses, &edges, ticket.status_id, req.status_id)
        {
            return Err(ApiError::Validation(rejection.message()));
        }
        let target = statuses
            .iter()
            .find(|s| s.id == req.status_id)
            .ok_or(ApiError::NotFound)?
            .clone();

        // Pre-close checklist: closing requires every item checked (§1).
        let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        core_domain::entities::validate_pre_close(&checklist, &target)?;

        // SLA pause bookkeeping binds to the semantic category, under the
        // policy the ticket is actually bound to (attached calendar wins
        // over the embedded default document).
        let now = chrono::Utc::now();
        if let Ok(Some(mut sla_state)) = core_db::sla::get_ticket_state(&mut *ctx.tx, id).await {
            let calendar =
                match core_db::sla::get_policy_record(&mut *ctx.tx, org_id, sla_state.policy_id)
                    .await
                {
                    Ok(record) => core_db::sla::resolve_policy_calendar(&mut *ctx.tx, &record)
                        .await
                        .unwrap_or_else(|_| record.calendar.clone()),
                    Err(_) => return Err(ApiError::NotFound),
                };
            sla_state.apply_category(target.category, now, &calendar);
            core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla_state).await?;
        }

        core_db::tickets::update_status(&mut *ctx.tx, org_id, id, req.status_id).await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

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

        // Audit the lifecycle change with full from→to provenance.
        let from_name = statuses
            .iter()
            .find(|s| s.id == ticket.status_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.status_change".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(after.id.to_string()),
                detail: serde_json::json!({
                    "from_status": from_name,
                    "from_status_id": ticket.status_id,
                    "from_category": ticket.category.as_str(),
                    "to_status": target.name,
                    "to_status_id": target.id,
                    "to_category": target.category.as_str(),
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        Ok((ctx, after))
    })
    .await?;

    Ok(Json(ticket))
}

/// GET /api/tickets/{id}/sla — clock state plus a live business-time
/// remaining estimate for the resolution budget (drives the UI badge).
#[utoipa::path(operation_id = "tickets_sla_status", get, path = "/api/tickets/{id}/sla", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketSlaStatus)))]
pub async fn sla_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketSlaStatus>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;

    let status = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut ctx| async move {
            let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
            let state_row = core_db::sla::get_ticket_state(&mut *ctx.tx, id)
                .await?
                .ok_or(ApiError::NotFound)?;
            // Attached row wins; embedded document keeps governing otherwise.
            let record = core_db::sla::get_policy_record(&mut *ctx.tx, org_id, state_row.policy_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
            let calendar = core_db::sla::resolve_policy_calendar(&mut *ctx.tx, &record)
                .await
                .unwrap_or_else(|_| record.calendar.clone());
            let now = chrono::Utc::now();
            let position = core_domain::sla_engine::position(
                &calendar,
                ticket.created_at,
                now,
                state_row.resolution_due,
                state_row.paused_business_secs,
                state_row.paused_since,
            );
            let minutes_remaining =
                position.map(|p| ((p.budget_secs - p.consumed_secs) / 60).max(i64::MIN / 2));
            Ok((
                ctx,
                TicketSlaStatus {
                    minutes_remaining,
                    paused: state_row.paused_since.is_some(),
                    breached: state_row.breached,
                    state: state_row,
                },
            ))
        },
    )
    .await?;
    Ok(Json(status))
}

/// GET /api/tickets/{id}/comments — internal notes require ticket view;
/// the portal surface filters kinds separately (portal v1).
#[utoipa::path(operation_id = "tickets_list_comments", get, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::TicketComment])))]
pub async fn list_comments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TicketComment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let comments =
        core_db::tickets::list_comments(&state.pool, authed.principal.user.org_id, id, None)
            .await?;
    Ok(Json(comments))
}

/// POST /api/tickets/{id}/comments — adds a structured note; @mentions
/// fan out into the notification center.
#[utoipa::path(operation_id = "tickets_add_comment", post, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddCommentRequest,
    responses((status = 200, body = core_domain::entities::TicketComment)))]
pub async fn add_comment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> ApiResult<Json<core_domain::entities::TicketComment>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.body.trim().is_empty() {
        return Err(ApiError::Validation("comment body is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let author_name = authed.principal.user.display_name.clone();

    let comment = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        let comment = core_db::tickets::insert_comment(
            &mut *ctx.tx,
            Uuid::now_v7(),
            org_id,
            id,
            Some(user_id),
            req.kind,
            req.body.trim(),
        )
        .await?;

        // Resolve @mentions against active org members (visibility is the
        // member directory itself until board RBAC lands), ignoring the
        // author's own handle and unknown handles. Notifications are
        // created in THIS transaction and each emits its own
        // `notification.created` so the Redis→hub chain fans out per-user.
        let tokens = core_domain::mentions::parse(req.body.trim());
        let mentioned = if tokens.is_empty() {
            Vec::new()
        } else {
            let directory: Vec<core_domain::mentions::MemberHandles> =
                core_db::users::list_users(&mut *ctx.tx, org_id)
                    .await?
                    .into_iter()
                    .filter(|u| u.is_active)
                    .map(|u| {
                        core_domain::mentions::MemberHandles::from_identity(
                            u.id,
                            &u.email,
                            &u.display_name,
                        )
                    })
                    .collect();
            core_domain::mentions::resolve(&tokens, &directory, user_id)
        };

        for mentioned_user in &mentioned {
            // The in-app channel toggle gates whether the inbox row exists
            // at all (defaults-first chain); email fan-out happens later in
            // the worker against the same preference chain.
            let pref = core_db::notifications::effective_preference(
                &mut *ctx.tx,
                org_id,
                *mentioned_user,
                core_domain::entities::NotificationKind::Mention.as_str(),
            )
            .await?;
            if !pref.in_app {
                continue;
            }
            let notification = core_db::notifications::insert(
                &mut *ctx.tx,
                Uuid::now_v7(),
                org_id,
                *mentioned_user,
                "mention",
                Some(id),
                &format!("{} mentioned you on {}", author_name, ticket.number.0),
                Some(comment_excerpt(req.body.trim())),
            )
            .await?;
            ctx.emit(
                types::NOTIFICATION_CREATED,
                "notification",
                Some(notification.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::NotificationCreated {
                    user_id: *mentioned_user,
                    kind: notification.kind.as_str().to_string(),
                    ticket_id: Some(id),
                    title: notification.title.clone(),
                })
                .unwrap_or_default(),
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

/// First ~140 characters of a note body for notification previews.
fn comment_excerpt(body: &str) -> &str {
    match body.char_indices().nth(140) {
        Some((idx, _)) => body[..idx].trim_end(),
        None => body,
    }
}

/// GET /api/tickets/{id}/checklist
#[utoipa::path(operation_id = "tickets_get_checklist", get, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::ChecklistItem])))]
pub async fn get_checklist(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::ChecklistItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let items =
        core_db::tickets::list_checklist(&state.pool, authed.principal.user.org_id, id).await?;
    Ok(Json(items))
}

/// POST /api/tickets/{id}/checklist — appends a pre-close checklist item.
#[utoipa::path(operation_id = "tickets_add_checklist_item", post, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddChecklistItemRequest,
    responses((status = 200, body = [core_domain::entities::ChecklistItem])))]
pub async fn add_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChecklistItemRequest>,
) -> ApiResult<Json<Vec<core_domain::entities::ChecklistItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
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
#[utoipa::path(operation_id = "tickets_set_checklist_item", post, path = "/api/tickets/{id}/checklist/{item_id}", tag = "tickets",
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
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    core_db::tickets::set_checklist_item(
        &state.pool,
        org_id,
        item_id,
        req.checked,
        Some(authed.principal.user.id),
    )
    .await?;
    let items = core_db::tickets::list_checklist(&state.pool, org_id, id).await?;
    Ok(Json(items))
}

/// GET /api/tickets/{id}/time-entries
#[utoipa::path(operation_id = "tickets_ticket_time_entries", get, path = "/api/tickets/{id}/time-entries", tag = "tickets",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = [core_domain::entities::TimeEntry])))]
pub async fn ticket_time_entries(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<core_domain::entities::TimeEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
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
