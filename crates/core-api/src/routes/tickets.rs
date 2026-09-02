//! Ticket endpoints: lifecycle, structured notes, pre-close checklists.
//! Every mutation writes the outbox event in the SAME transaction as the
//! data change (§4.2) and updates SLA clock state (§1).

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
use core_domain::entities::{Actor, AuditEvent, ChecklistItem, ChecklistProgress};
use core_domain::events::types;
use core_domain::policy::{Module, Permission};
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use core_domain::workflow;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::board_access::authorize_board_op;
use super::custom_fields::{TicketFieldInput, TicketWithFields};
use core_domain::policy::BoardOp;

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

#[derive(Debug, Deserialize, Serialize, ToSchema)]
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
    /// Initial custom-field values; each is validated against its
    /// definition. REQUIRED definitions must all be present (TASKLIST 2.17).
    #[serde(default)]
    pub custom_fields: Vec<TicketFieldInput>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
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
    /// Custom-field values to upsert. Partial semantics: only listed fields
    /// change, each validated against its definition.
    pub custom_fields: Option<Vec<TicketFieldInput>>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ChangeStatusRequest {
    pub status_id: Uuid,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AddCommentRequest {
    pub body: String,
    pub kind: NoteKind,
    /// Attachments uploaded earlier against this ticket (composer flow:
    /// upload first, link when the comment is created). Absent/empty in
    /// older clients — purely additive.
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
    /// Instant-first autosave parameters (IF1): idempotency key. Absent in
    /// legacy clients — purely additive. (Comments are fresh entities, so
    /// there is no version to match.)
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddChecklistItemRequest {
    pub text: String,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetChecklistItemRequest {
    pub checked: bool,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
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

/// Instant-first mutation response (IF2): the enriched ticket fields at the
/// top level (unchanged shape) plus the additive `mutation` metadata — the
/// applied snapshot, monotonic per-entity `seq`/`version`, and the prior
/// and new values that make undo a core capability.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketMutationResponse {
    #[serde(flatten)]
    pub ticket: TicketWithFields,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for status transitions (plain ticket body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketStatusMutationResponse {
    #[serde(flatten)]
    pub ticket: core_domain::entities::Ticket,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for comments (plain comment body).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketCommentMutationResponse {
    #[serde(flatten)]
    pub comment: core_domain::entities::TicketComment,
    pub mutation: instant::MutationMeta,
}

/// Same additive envelope for checklist writes: the full checklist view at
/// the top level (unchanged shape) plus the read-your-write metadata for
/// the affected item.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TicketChecklistMutationResponse {
    #[serde(flatten)]
    pub view: TicketChecklistView,
    pub mutation: instant::MutationMeta,
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

/// GET /api/tickets — filtered, priority-ordered list; each row carries its
/// custom-field values (TASKLIST 2.17) fetched in one batched query.
#[utoipa::path(operation_id = "tickets_list", get, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    params(TicketListQuery),
    responses((status = 200, body = [TicketWithFields])))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TicketListQuery>,
) -> ApiResult<Json<Vec<TicketWithFields>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let rows = core_db::tickets::list_tickets(
        &state.pool,
        org_id,
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
    let enriched = TicketWithFields::load_many(&state.pool, org_id, rows).await?;
    Ok(Json(enriched))
}

/// GET /api/tickets/my-work — everything assigned to me, not closed,
/// across all boards, priority-then-SLA-due ordered (the unified
/// cross-board view from §1; TASKLIST 2.11 adds client/board annotations).
#[utoipa::path(operation_id = "tickets_my_work", get, path = "/api/tickets/my-work", tag = "tickets", security(("bearer" = [])),
    responses((status = 200, body = [MyWorkItem])))]
pub async fn my_work(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<MyWorkItem>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let rows = core_db::tickets::my_work_annotated(
        &state.pool,
        authed.principal.user.org_id,
        authed.principal.user.id,
        100,
    )
    .await?;
    Ok(Json(rows.into_iter().map(MyWorkItem::from_row).collect()))
}

/// One cross-board my-work queue row (TASKLIST 2.11): the ticket plus the
/// annotations the Today screen renders per the mockup.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyWorkItem {
    pub id: Uuid,
    pub number: String,
    pub title: String,
    pub priority: Priority,
    pub category: TicketStatusCategory,
    pub status_name: String,
    /// The source board of this cross-board row.
    pub board_id: Uuid,
    pub board_name: String,
    /// Client short label for the squared tag (`None` = internal ticket).
    pub client_name: Option<String>,
    /// Manual deadline set on the ticket itself.
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// SLA resolution deadline when state exists; the UI prefers this over
    /// `due_at` for the right-aligned countdown column.
    pub sla_resolution_due: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl MyWorkItem {
    pub(crate) fn from_row(row: core_db::tickets::MyWorkRow) -> Self {
        let t = row.ticket;
        MyWorkItem {
            id: t.id,
            number: t.number.0,
            title: t.title,
            priority: t.priority,
            category: t.category,
            status_name: row.status_name,
            board_id: t.board_id,
            board_name: row.board_name,
            client_name: row.client_name,
            due_at: t.due_at,
            sla_resolution_due: row.sla_resolution_due,
            updated_at: t.updated_at,
        }
    }
}

/// POST /api/tickets — allocates the human ticket number, initializes SLA
/// clocks, validates + stores any initial custom-field values (REQUIRED
/// definitions enforced), and emits `ticket.created` — all in one transaction.
/// Instant-first (IF1/IF2): routed through the shared idempotency gate and
/// returns the read-your-write `mutation` metadata.
#[utoipa::path(operation_id = "tickets_create", post, path = "/api/tickets", tag = "tickets", security(("bearer" = [])),
    request_body = CreateTicketRequest,
    responses((status = 200, body = TicketMutationResponse),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "missing required custom fields or invalid value")))]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTicketRequest>,
) -> ApiResult<Json<TicketMutationResponse>> {
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
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Board-level RBAC first (deny wins): a viewer-role membership row
        // blocks creation even when the org matrix would allow it.
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            req.board_id,
            BoardOp::CreateTicket,
        )
        .await?;
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
        // Fresh entity: allocate the seq the response reports. A create
        // presenting a non-zero version is a client bug — a fresh entity
        // can only ever match version 0 — and is rejected before any row
        // is written.
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: ticket.id,
                version: req.instant.version,
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

        // Initial custom-field values: validated per definition, stored
        // canonically, then the REQUIRED set is completeness-checked.
        let applied = super::custom_fields::apply_ticket_field_inputs(
            &mut ctx.tx,
            org_id,
            req.board_id,
            ticket.id,
            &req.custom_fields,
        )
        .await?;
        for value in &applied {
            ctx.emit(
                types::TICKET_FIELD_SET,
                "ticket",
                Some(ticket.id.to_string().as_str()),
                &serde_json::json!({
                    "ticket_id": ticket.id,
                    "board_id": req.board_id,
                    "field_id": value.field_id,
                    "value": value.value,
                }),
            )
            .await?;
        }
        super::custom_fields::enforce_required_present(
            &mut ctx.tx,
            org_id,
            req.board_id,
            ticket.id,
        )
        .await?;

        let enriched = TicketWithFields::new(ticket, applied);
        let after = serde_json::to_value(&enriched.ticket).unwrap_or(Value::Null);
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: enriched.ticket.id,
                op: "create",
                seq: write_seq,
                before: None,
                after,
                updated_at: enriched.ticket.updated_at,
                key: key.as_deref(),
                payload: &enriched,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketMutationResponse {
                ticket: enriched,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/tickets/{id}
#[utoipa::path(operation_id = "tickets_get_one", get, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = TicketWithFields)))]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TicketWithFields>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::View),
    )?;
    let org_id = authed.principal.user.org_id;
    let ticket = core_db::tickets::get_ticket(&state.pool, org_id, id).await?;
    let enriched = TicketWithFields::load(&state.pool, org_id, ticket).await?;
    Ok(Json(enriched))
}

/// PATCH /api/tickets/{id} — partial update; validates + upserts any
/// provided custom-field values; emits `ticket.updated`,
/// `ticket.field_set` (per changed field), and `ticket.assigned` when the
/// assignee changes. Instant-first (IF1/IF2): idempotency-keyed, version-
/// gated, and returns prior + new values for undo.
#[utoipa::path(operation_id = "tickets_patch", patch, path = "/api/tickets/{id}", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchTicketRequest,
    responses((status = 200, body = TicketMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "invalid custom-field value")))]
pub async fn patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchTicketRequest>,
) -> ApiResult<Json<TicketMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::tickets::update_ticket(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::tickets::TicketPatch {
                title: req.title.as_deref(),
                body: req.body.as_deref().map(Some),
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

        // Partial custom-field update: only listed fields change; each is
        // validated against its definition before storage. Clearing a
        // REQUIRED value stays blocked at the dedicated clear endpoint.
        if let Some(inputs) = &req.custom_fields {
            let applied = super::custom_fields::apply_ticket_field_inputs(
                &mut ctx.tx,
                org_id,
                after.board_id,
                id,
                inputs,
            )
            .await?;
            for value in &applied {
                ctx.emit(
                    types::TICKET_FIELD_SET,
                    "ticket",
                    Some(after.id.to_string().as_str()),
                    &serde_json::json!({
                        "ticket_id": after.id,
                        "board_id": after.board_id,
                        "field_id": value.field_id,
                        "value": value.value,
                    }),
                )
                .await?;
            }
        }

        ctx.emit(
            types::TICKET_UPDATED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({
                "ticket_id": after.id,
                "board_id": after.board_id,
                "assignee_id": after.assignee_id,
            }),
        )
        .await?;
        if before.assignee_id != after.assignee_id {
            ctx.emit(
                types::TICKET_ASSIGNED,
                "ticket",
                Some(after.id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::TicketAssigned {
                    ticket_id: after.id,
                    board_id: after.board_id,
                    assignee_id: after.assignee_id,
                    assigned_by: Some(user_id),
                })
                .unwrap_or_default(),
            )
            .await?;
        }
        let enriched = TicketWithFields::load(&mut *ctx.tx, org_id, after).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&enriched.ticket).unwrap_or(Value::Null),
                updated_at: enriched.ticket.updated_at,
                key: key.as_deref(),
                payload: &enriched,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketMutationResponse {
                ticket: enriched,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/tickets/{id}/status — the guarded lifecycle transition:
/// domain transition rules, pre-close checklist enforcement (typed 409
/// listing the incomplete required items when the board enforces), SLA
/// pause bookkeeping, and the `ticket.status_changed` event, all in one tx.
/// Instant-first (IF1/IF2): idempotency-keyed, version-gated, read-your-write.
#[utoipa::path(operation_id = "tickets_change_status", post, path = "/api/tickets/{id}/status", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = ChangeStatusRequest,
    responses((status = 200, body = TicketStatusMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "pre-close checklist blocked, version conflict, or idempotency key reused"),
              (status = 422, description = "invalid transition")))]
pub async fn change_status(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeStatusRequest>,
) -> ApiResult<Json<TicketStatusMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // The board's statuses + explicit transition matrix define what is
        // legal here; with no explicit matrix the semantic-category floor
        // governs (core-domain `workflow`).
        let statuses = core_db::boards::list_statuses(&mut *ctx.tx, before.board_id).await?;
        let edges = core_db::boards::list_transitions(&mut *ctx.tx, before.board_id).await?;
        if let Err(rejection) =
            workflow::validate_transition(&statuses, &edges, before.status_id, req.status_id)
        {
            return Err(ApiError::Validation(rejection.message()));
        }
        let target = statuses
            .iter()
            .find(|s| s.id == req.status_id)
            .ok_or(ApiError::NotFound)?
            .clone();

        // Board-level RBAC (deny wins): closing is a distinct board-scoped
        // operation from ordinary work, so the role matrix can forbid
        // closure while still allowing edits.
        let close_op = target.category == TicketStatusCategory::Closed;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            before.board_id,
            if close_op {
                BoardOp::CloseTicket
            } else {
                BoardOp::WorkTicket
            },
        )
        .await?;

        // Pre-close checklist gate (defaults-first): lazy-instantiate any
        // template items this ticket has not picked up yet — both so older
        // tickets see template edits and so the verdict below judges the
        // complete required set — then apply the pure domain rule. Blocking
        // is a typed 409 that names every unchecked REQUIRED item.
        let instantiated =
            core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, before.board_id, id)
                .await?;
        let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        // Only closures are gated; other categories never consult the list.
        if target.category.requires_checklist() {
            let active = checklist_enforcement(&mut ctx.tx, org_id, before.board_id).await?;
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
                    board_id: before.board_id,
                    items: checklist,
                    updated_by: None,
                })
                .unwrap_or_default(),
            )
            .await?;
        }

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

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::tickets::update_status(&mut *ctx.tx, org_id, id, req.status_id).await?;
        let after = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id).await?;

        ctx.emit(
            types::TICKET_STATUS_CHANGED,
            "ticket",
            Some(after.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketStatusChanged {
                ticket_id: after.id,
                board_id: after.board_id,
                assignee_id: after.assignee_id,
                from_status: before.status_id.to_string(),
                to_status: target.name.clone(),
                from_category: before.category,
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
                &serde_json::json!({
                    "ticket_id": after.id,
                    "board_id": after.board_id,
                    "resolved_at": after.resolved_at
                }),
            )
            .await?;
        }

        // Audit the lifecycle change with full from→to provenance.
        let from_name = statuses
            .iter()
            .find(|s| s.id == before.status_id)
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
                    "from_status_id": before.status_id,
                    "from_category": before.category.as_str(),
                    "to_status": target.name,
                    "to_status_id": target.id,
                    "to_category": target.category.as_str(),
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;

        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket",
                entity_id: id,
                op: "status_change",
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
            MutationOutcome::Applied(TicketStatusMutationResponse {
                ticket: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
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
/// fan out into the notification center. Instant-first (IF1/IF2):
/// idempotency-keyed and returns the read-your-write mutation metadata.
#[utoipa::path(operation_id = "tickets_add_comment", post, path = "/api/tickets/{id}/comments", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddCommentRequest,
    responses((status = 200, body = TicketCommentMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn add_comment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddCommentRequest>,
) -> ApiResult<Json<TicketCommentMutationResponse>> {
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
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_comment",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
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

        // Bind composer-uploaded files to this comment (one-shot links;
        // foreign/duplicate ids are skipped rather than failing the note).
        for attachment_id in &req.attachment_ids {
            if let Some(linked) = core_db::attachments::link_to_comment(
                &mut ctx.tx,
                org_id,
                *attachment_id,
                id,
                comment.id,
            )
            .await?
            {
                ctx.emit(
                    types::ATTACHMENT_LINKED,
                    "attachment",
                    Some(linked.id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AttachmentLinked {
                        attachment_id: linked.id,
                        ticket_comment_id: comment.id,
                        ticket_id: id,
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        }

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
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_comment",
                entity_id: comment.id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_comment",
                entity_id: comment.id,
                op: "comment_add",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&comment).unwrap_or(Value::Null),
                updated_at: comment.created_at,
                key: key.as_deref(),
                payload: &comment,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketCommentMutationResponse { comment, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
/// Instant-first (IF1/IF2): idempotency-keyed and returns the
/// read-your-write mutation metadata for the created item.
#[utoipa::path(operation_id = "tickets_add_checklist_item", post, path = "/api/tickets/{id}/checklist", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = AddChecklistItemRequest,
    responses((status = 200, body = TicketChecklistMutationResponse),
              (status = 404, description = "ticket not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn add_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistMutationResponse>> {
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
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    // Hoisted so a retried create replays the SAME stored item result.
    let item_id = Uuid::now_v7();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "checklist_item",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let existing = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
        core_db::tickets::add_checklist_item(
            &mut *ctx.tx,
            item_id,
            org_id,
            id,
            req.text.trim(),
            existing.len() as i32,
        )
        .await?;
        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        let created = items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!("checklist item vanished in-transaction"))
            })?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                board_id: ticket.board_id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "checklist_item",
                entity_id: item_id,
                version: req.instant.version,
            },
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        let view = TicketChecklistView {
            items,
            progress,
            enforcement_active,
            blocking,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "checklist_item",
                entity_id: item_id,
                op: "checklist_item_add",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&created).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketChecklistMutationResponse { view, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/tickets/{id}/checklist/{item_id} — toggles one checklist item;
/// emits `ticket.checklist_updated` in the same transaction. Instant-first
/// (IF1/IF2): idempotency-keyed and returns the read-your-write mutation
/// metadata for the toggled item.
#[utoipa::path(operation_id = "tickets_set_checklist_item", post, path = "/api/tickets/{id}/checklist/{item_id}", tag = "tickets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path), ("item_id" = Uuid, Path)),
    request_body = SetChecklistItemRequest,
    responses((status = 200, body = TicketChecklistMutationResponse),
              (status = 404, description = "ticket or item not visible in scope (rls_sparse)"),
              (status = 409, description = "idempotency key reused")))]
pub async fn set_checklist_item(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, item_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<SetChecklistItemRequest>,
) -> ApiResult<Json<TicketChecklistMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        tickets(core_domain::policy::Action::Edit),
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "checklist_item",
                entity_id: Some(item_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        authorize_board_op(
            &mut ctx.tx,
            &authed.principal,
            ticket.board_id,
            BoardOp::WorkTicket,
        )
        .await?;
        let before = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|item| item.id == item_id)
            .ok_or(ApiError::NotFound)?;
        core_db::tickets::set_checklist_item(
            &mut *ctx.tx,
            org_id,
            item_id,
            req.checked,
            Some(user_id),
        )
        .await?;
        let (items, progress) = core_db::checklists::view(&mut ctx.tx, org_id, id).await?;
        let updated = items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::Internal(anyhow::anyhow!("checklist item vanished in-transaction"))
            })?;
        ctx.emit(
            types::TICKET_CHECKLIST_UPDATED,
            "ticket",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketChecklistUpdated {
                ticket_id: id,
                board_id: ticket.board_id,
                items: items.clone(),
                updated_by: Some(user_id),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "checklist_item",
                entity_id: item_id,
                version: req.instant.version,
            },
        )
        .await?;
        let enforcement_active =
            checklist_enforcement(&mut ctx.tx, org_id, ticket.board_id).await?;
        let blocking = blocking_of(&items, enforcement_active);
        let view = TicketChecklistView {
            items,
            progress,
            enforcement_active,
            blocking,
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "checklist_item",
                entity_id: item_id,
                op: "checklist_item_toggle",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&updated).unwrap_or(Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TicketChecklistMutationResponse { view, mutation }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
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
