//! The ticket status transition, shared by the HTTP route and the workflow
//! executor (Wave 43 lane 20, item E).
//!
//! `change_status` used to own the whole legal transition inline: the
//! instant-first claim and replay, the board's transition matrix, the target
//! column's WIP cap, board-level RBAC, the pre-close checklist and workbar
//! task gates, SLA bookkeeping, the `ticket.status_changed` / `ticket.closed`
//! events and the audit row. A workflow's `ticket.transition` node needs
//! exactly that path — an executor arm that reimplemented it would let a
//! workflow close a ticket past the gate the ticket surface enforces. So the
//! body lives here and both callers share it.
//!
//! Two details keep the sharing honest. Permission is re-derived from the
//! principal here as well as in the route, and board RBAC is derived from
//! the SAME principal inside the shared body — so a workflow authored by a
//! technician without close rights refuses rather than bypassing the board
//! matrix. And the node endpoint builds that principal from the run's
//! author (`actor_user_id`), which is the documented posture: a run acts as
//! its author, never as a more powerful system user.

use crate::auth::middleware::require_permission;
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::routes::tickets::{
    checklist_enforcement, tickets, ChangeStatusRequest, TaskGateItem, TicketStatusMutationResponse,
};
use crate::state::AppState;
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::types;
use core_domain::policy::BoardOp;
use core_domain::status::TicketStatusCategory;
use core_domain::workflow;
use serde_json::Value;
use uuid::Uuid;

/// Applies a guarded ticket status transition as `principal`. The
/// `POST /api/tickets/{id}/status` route calls this; see the module note.
pub async fn transition_status(
    state: &AppState,
    principal: &Principal,
    ticket_id: Uuid,
    req: ChangeStatusRequest,
) -> ApiResult<MutationOutcome<TicketStatusMutationResponse>> {
    transition_status_with_pool(&state.pool, principal, ticket_id, req).await
}

/// Same transition, from a bare pool — the workflow node endpoint has no
/// `AppState`, only the connection it is already running inside.
pub async fn transition_status_with_pool(
    pool: &sqlx::PgPool,
    principal: &Principal,
    ticket_id: Uuid,
    req: ChangeStatusRequest,
) -> ApiResult<MutationOutcome<TicketStatusMutationResponse>> {
    // The route already checked this against the session principal; the
    // check is repeated here so the node endpoint — which builds its
    // principal from the run's author — cannot skip it.
    require_permission(principal, tickets(core_domain::policy::Action::Edit))?;
    let org_id = principal.user.org_id;
    let user_id = principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let ctx = org_tx::begin_pool(pool, org_id, user_id).await?;
    let (ctx, outcome) =
        apply_transition(ctx, principal, ticket_id, &req, key, version, fingerprint).await?;
    ctx.commit().await?;
    Ok(outcome)
}

/// The shared body, in the caller's transaction. Written against the route's
/// `id` path parameter and its `req` body, so both keep those names.
async fn apply_transition(
    mut ctx: org_tx::OrgTx<'static>,
    principal: &Principal,
    id: Uuid,
    req: &ChangeStatusRequest,
    key: Option<String>,
    version: Option<i64>,
    fingerprint: Value,
) -> ApiResult<(
    org_tx::OrgTx<'static>,
    MutationOutcome<TicketStatusMutationResponse>,
)> {
    let org_id = principal.user.org_id;
    let user_id = principal.user.id;
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

    // Per-column WIP cap (0089): the target column's occupancy
    // (excluding THIS ticket — a move within is self-neutral) is
    // enforced inside the same write transaction as the move, so a
    // concurrent move cannot squeeze past the cap either. Typed 422
    // naming the column + its cap; the board UI renders the count.
    if let Some(wip_limit) = target.wip_limit {
        let occupancy =
            core_db::boards::count_tickets_in_status(&mut *ctx.tx, req.status_id, id).await?;
        if occupancy >= i64::from(wip_limit) {
            return Err(ApiError::Validation(format!(
                    "status `{}` has reached its WIP limit ({}/{}) — move the ticket elsewhere or raise the cap",
                    target.name, occupancy, wip_limit
                )));
        }
    }

    // Board-level RBAC (deny wins): closing is a distinct board-scoped
    // operation from ordinary work, so the role matrix can forbid
    // closure while still allowing edits.
    let close_op = target.category == TicketStatusCategory::Closed;
    authorize_board_op(
        &mut ctx.tx,
        principal,
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
        core_db::checklists::instantiate_missing(&mut ctx.tx, org_id, before.board_id, id).await?;
    let checklist = core_db::tickets::list_checklist(&mut *ctx.tx, org_id, id).await?;
    // Only closures are gated; other categories never consult the list.
    if target.category.requires_checklist() {
        let active = checklist_enforcement(&mut ctx.tx, org_id, before.board_id).await?;
        if let Err(blocked) = core_domain::notes::enforce_pre_close(&checklist, active) {
            return Err(ApiError::ChecklistBlocked {
                missing: blocked.missing,
            });
        }
        // Wave 17 workbar close gate (§4): every task needs a terminal
        // decision (checked / N-A) — independent of the board's
        // require_checklist_close posture; empty checklists pass
        // vacuously. Typed 409 with the exact unassigned rows so raw
        // API callers get the same gate as any closing surface.
        if let Err(gate) = core_domain::notes::enforce_tasks_assigned(&checklist) {
            return Err(ApiError::TasksUnassigned {
                items: gate
                    .items
                    .iter()
                    .map(|(item_id, text, required)| TaskGateItem {
                        item_id: *item_id,
                        text: text.clone(),
                        required: *required,
                    })
                    .collect(),
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
        let calendar = match core_db::sla::get_policy_record(
            &mut *ctx.tx,
            org_id,
            sla_state.policy_id,
        )
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
}
