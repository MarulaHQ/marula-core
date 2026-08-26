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
use core_domain::entities::{Actor, AuditEvent, ChecklistItem, ChecklistProgress};
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

/// The board's close-enforcement posture: ACTIVE only when a checklist
/// template exists AND the board's require_checklist_close toggle is on
/// (defaults-first doctrine — see migration 0016's header).
async fn checklist_enforcement(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    board_id: Uuid,
) -> ApiResult<bool> {
    let board = core_db::boards::get_board(&mut *conn, org_id, board_id).await?;
    if !board.require_checklist_close {
        return Ok(false);
    }
    let has_template = core_db::checklists::get_template_for_board(&mut *conn, org_id, board_id)
        .await?
        .is_some();
    Ok(has_template)
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

        // Checklist templates instantiate eagerly on creation so staff see
        // required steps from minute one; template edits flow onto existing
        // tickets lazily via `instantiate_missing` (close path / reads).
        core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, req.board_id, ticket.id)
            .await?;

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
/// domain transition rules, pre-close checklist enforcement (typed 409
/// listing the incomplete required items when the board enforces), SLA
/// pause bookkeeping, and the `ticket.status_changed` event, all in one tx.
#[utoipa::path(operation_id = "tickets_change_status", post, path = "/api/tickets/{id}/status", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ChangeStatusRequest,
    responses((status = 200, body = core_domain::entities::Ticket),
              (status = 409, description = "pre-close checklist incomplete (board enforces)"),
              (status = 422, description = "invalid transition")))]
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

        // Pre-close checklist gate (defaults-first): lazy-instantiate any
        // template items this ticket has not picked up yet — both so older
        // tickets see template edits and so the verdict below judges the
        // complete required set — then apply the pure domain rule. Blocking
        // is a typed 409 that names every unchecked REQUIRED item.
        let instantiated =
            core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, ticket.board_id, id)
                .await?;
        let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        // Only closures are gated; other categories never consult the list.
        if target.category.requires_checklist() {
            let active = checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
            if let Err(blocked) = core_domain::notes::enforce_pre_close(&checklist, active) {
                return Err(ApiError::ChecklistBlocked {
                    missing: blocked.missing,
                });
            }
        }
        if instantiated > 0 {
            ctx.emit(
                types::TICKET_CHECKLIST_UPDATED,
                "ticket",
                Some(id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                    ticket_id: id,
                    items: checklist,
                    updated_by: None,
                })
                .unwrap_or_default(),
            )
            .await?;
        }

        // 3. SLA pause bookkeeping binds to the semantic category.
        let now = chrono::Utc::now();
        if let Ok(mut sla) = core_db::sla::get_ticket_state(&mut *ctx.tx, id).await {
            if let Some(mut sla_state) = sla.take() {
                let calendar = {
                    let policy = core_db::sla::get_default_policy(&mut *ctx.tx, org_id).await?;
                    policy.calendar
                };
                sla_state.apply_category(target.category, now, &calendar);
                core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla_state).await?;
            }
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

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CommentListQuery {
    /// Optional taxonomy filter (`internal`, `client_visible`,
    /// `in_house_update`). Omit for all kinds (staff view).
    pub kind: Option<NoteKind>,
}

/// GET /api/tickets/{id}/comments — optional kind filter; internal notes
/// require ticket view. The portal surface must consume the dedicated
/// client-visible read (`core-db::tickets::list_client_visible_comments`),
/// never this staff endpoint.
#[utoipa::path(operation_id = "tickets_list_comments", get, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path), CommentListQuery),
    responses((status = 200, body = [core_domain::entities::TicketComment])))]
pub async fn list_comments(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<CommentListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::TicketComment>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let kinds: Option<Vec<NoteKind>> = q.kind.map(|k| vec![k]);
    let comments = core_db::tickets::list_comments(
        &state.pool,
        authed.principal.user.org_id,
        id,
        kinds.as_deref(),
    )
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
    let requester_email = authed.principal.user.email.clone();
    let mail = std::sync::Arc::clone(&state.mail);

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

        // Email-out v1: client-visible replies notify the requester
        // (skipped when the author IS the requester or the requester has
        // no account email). Enqueue-only here; the worker transports.
        if req.kind == NoteKind::ClientVisible {
            if let Ok(ticket) = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await {
                if let Some(requester_id) = ticket.requester_id {
                    if let Ok(requester) =
                        core_db::users::get_user(&mut *ctx.tx, org_id, requester_id).await
                    {
                        if requester.is_active
                            && !requester.email.eq_ignore_ascii_case(&requester_email)
                        {
                            crate::email::enqueue_outbound(
                                &mut ctx.tx,
                                crate::email::OutboundEnqueue {
                                    mail: &mail,
                                    org_id,
                                    to_addresses: vec![requester.email],
                                    kind: &core_mail::outbound::OutboundKind::CommentNotification {
                                        ticket_number: ticket.number.0.clone(),
                                        ticket_title: ticket.title.clone(),
                                        excerpt: core_mail::outbound::excerpt(req.body.trim(), 280),
                                    },
                                    in_reply_to: None,
                                    references: Vec::new(),
                                    ticket_id: Some(id),
                                },
                            )
                            .await?;
                        }
                    }
                }
            }
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

/// Unchecked REQUIRED item texts under the given posture (empty when
/// inactive) — the UI-facing projection of the close gate.
fn blocking_of(items: &[ChecklistItem], enforcement_active: bool) -> Vec<String> {
    if !enforcement_active {
        return Vec::new();
    }
    core_domain::notes::enforce_pre_close(items, true)
        .err()
        .map(|b| b.missing)
        .unwrap_or_default()
}

/// GET response for a ticket's checklist: instances, derived progress,
/// and the board's live enforcement posture. `blocking` lists the unchecked
/// REQUIRED items when enforcement is active (empty otherwise).
#[derive(Debug, Serialize, ToSchema)]
pub struct TicketChecklistView {
    pub items: Vec<ChecklistItem>,
    pub progress: ChecklistProgress,
    pub enforcement_active: bool,
    pub blocking: Vec<String>,
}

async fn checklist_view(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    ticket: &core_domain::entities::Ticket,
) -> ApiResult<TicketChecklistView> {
    let (items, progress) = core_db::checklists::view(&mut *conn, org_id, ticket.id).await?;
    let enforcement_active = checklist_enforcement(conn, org_id, ticket.board_id).await?;
    let blocking = blocking_of(&items, enforcement_active);
    Ok(TicketChecklistView {
        items,
        progress,
        enforcement_active,
        blocking,
    })
}

/// GET /api/tickets/{id}/checklist — lazily instantiates any template items
/// the ticket has not picked up yet, then returns the full view.
#[utoipa::path(operation_id = "tickets_get_checklist", get, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketChecklistView)))]
pub async fn get_checklist(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketChecklistView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    // Instantiation is idempotent; a plain read must never fail because the
    // template changed, so it runs inside its own committed transaction.
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    core_db::checklists::instantiate_missing(&mut tx, org_id, ticket.board_id, id).await?;
    let view = checklist_view(&mut tx, org_id, &ticket).await?;
    tx.commit()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(view))
}

/// POST /api/tickets/{id}/checklist — appends an ad-hoc checklist item.
/// Ad-hoc items never block closure; emits `ticket.checklist_updated`.
#[utoipa::path(operation_id = "tickets_add_checklist_item", post, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddChecklistItemRequest,
    responses((status = 200, body = TicketChecklistView)))]
pub async fn add_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    if req.text.trim().is_empty() {
        return Err(ApiError::Validation("checklist text is required".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let view = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        let existing = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        core_db::tickets::add_checklist_item(
            &mut *ctx.tx,
            Uuid::now_v7(),
            org_id,
            id,
            req.text.trim(),
            existing.len() as i32,
        )
        .await?;
        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        Ok((
            ctx,
            TicketChecklistView {
                items,
                progress,
                enforcement_active,
                blocking,
            },
        ))
    })
    .await?;
    Ok(Json(view))
}

/// POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item;
/// emits `ticket.checklist_updated` in the same transaction.
#[utoipa::path(operation_id = "tickets_set_checklist_item", post, path = "/api/tickets/{id}/checklist/{item_id}", tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("item_id" = Uuid, Path)),
    request_body = SetChecklistItemRequest,
    responses((status = 200, body = TicketChecklistView)))]
pub async fn set_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let view = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;
        core_db::tickets::set_checklist_item(
            &mut *ctx.tx,
            org_id,
            item_id,
            req.checked,
            Some(user_id),
        )
        .await?;
        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        Ok((
            ctx,
            TicketChecklistView {
                items,
                progress,
                enforcement_active,
                blocking,
            },
        ))
    })
    .await?;
    Ok(Json(view))
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
