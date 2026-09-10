//! Ticket costs & expenses (Wave 11 lane 9, migration 0056).
//!
//! Purpose: per-ticket money lines — cost lines (parts / flat / other)
//! and expense entries, each with a billable flag — plus the org-wide
//! invoicing-candidate feed (billable, not-yet-billed rows) the invoicing
//! run consumes through the `invoice_line_id` marker ("billed once,
//! ever").
//!
//! Mutations ride the IF1–IF3 instant-first envelope; every mutation
//! emits its contract event + audit row in the same commit. Billed lines
//! refuse every mutation with a typed 409.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::board_access::authorize_board_op;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::{get, patch},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent, CostLineKind, TicketCostLine, TicketExpense};
use core_domain::events::types;
use core_domain::policy::{BoardOp, Permission};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub fn routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/tickets/{{id}}/cost-lines"),
            get(list_cost_lines).post(create_cost_line),
        )
        .route(
            &format!("{base}/tickets/{{id}}/cost-lines/{{line_id}}"),
            patch(update_cost_line).delete(remove_cost_line),
        )
        .route(
            &format!("{base}/tickets/{{id}}/expenses"),
            get(list_expenses).post(create_expense),
        )
        .route(
            &format!("{base}/tickets/{{id}}/expenses/{{expense_id}}"),
            patch(update_expense).delete(remove_expense),
        )
        .route(&format!("{base}/invoice-candidates"), get(list_candidates))
}

fn view_financials() -> Permission {
    Permission::ViewFinancials
}

async fn ticket_for_board_check(
    tx: &mut sqlx::PgConnection,
    principal: &core_domain::entities::Principal,
    org_id: Uuid,
    ticket_id: Uuid,
    op: BoardOp,
) -> ApiResult<core_domain::entities::Ticket> {
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id).await?;
    authorize_board_op(tx, principal, ticket.board_id, op).await?;
    Ok(ticket)
}

// --- cost lines ------------------------------------------------------------------

/// GET /api/tickets/{id}/cost-lines
#[utoipa::path(operation_id = "tickets_list_cost_lines", get, path = "/api/tickets/{id}/cost-lines", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [TicketCostLine])))]
pub async fn list_cost_lines(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<TicketCostLine>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    ticket_for_board_check(&mut tx, &authed.principal, org_id, id, BoardOp::View).await?;
    let lines = core_db::costs::list_cost_lines(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(lines))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateCostLineRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub kind: CostLineKind,
    pub description: String,
    #[serde(default = "one")]
    pub quantity: f64,
    pub unit_cost_minor: i64,
    pub currency: String,
    #[serde(default)]
    pub billable: bool,
}

fn one() -> f64 {
    1.0
}

/// POST /api/tickets/{id}/cost-lines — add a money line (parts/flat/other).
#[utoipa::path(operation_id = "tickets_create_cost_line", post, path = "/api/tickets/{id}/cost-lines", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateCostLineRequest,
    responses((status = 200, body = TicketCostLine),
              (status = 404, description = "ticket not found in scope"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "invalid quantity/currency")))]
pub async fn create_cost_line(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateCostLineRequest>,
) -> ApiResult<Json<CostLineMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.description.trim().is_empty() {
        return Err(ApiError::Validation("description is required".into()));
    }
    if req.quantity <= 0.0 {
        return Err(ApiError::Validation("quantity must be positive".into()));
    }
    if req.unit_cost_minor < 0 {
        return Err(ApiError::Validation("unit_cost_minor must be >= 0".into()));
    }
    if req.currency.len() != 3 || !req.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ApiError::Validation(
            "currency must be a 3-letter ISO 4217 code".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_cost_line",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket =
            ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View)
                .await?;
        let line = TicketCostLine {
            id: uuid::Uuid::now_v7(),
            org_id,
            ticket_id: id,
            kind: req.kind,
            description: req.description.trim().to_string(),
            quantity: req.quantity,
            unit_cost_minor: req.unit_cost_minor,
            currency: req.currency.clone(),
            billable: req.billable,
            invoice_line_id: None,
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        core_db::costs::insert_cost_line(&mut *ctx.tx, &line).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.cost_line_added".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "kind": line.kind.as_str(),
                    "total_minor": line.total_minor(),
                    "currency": line.currency,
                    "billable": line.billable,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_COST_LINE_ADDED,
            "ticket_cost_line",
            Some(line.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCostLineAdded { line: line.clone() })
                .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_cost_line",
                entity_id: line.id,
                version: req.instant.version,
            },
        )
        .await?;
        let stored = CostLineMutationResponse {
            line: line.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_cost_line",
                entity_id: line.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&line).unwrap_or(serde_json::Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        let _ = ticket;
        Ok((
            ctx,
            MutationOutcome::Applied(CostLineMutationResponse { line, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct CostLineMutationResponse {
    pub line: TicketCostLine,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct UpdateCostLineRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub description: Option<String>,
    pub quantity: Option<f64>,
    pub unit_cost_minor: Option<i64>,
    pub billable: Option<bool>,
}

/// PATCH /api/tickets/{id}/cost-lines/{line_id} — edit an unbilled line.
#[utoipa::path(operation_id = "tickets_update_cost_line", patch, path = "/api/tickets/{id}/cost-lines/{line_id}", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("line_id" = Uuid, Path)),
    request_body = UpdateCostLineRequest,
    responses((status = 200, body = CostLineMutationResponse),
              (status = 404, description = "line not found in scope"),
              (status = 409, description = "already billed, version conflict, or idempotency key reused")))]
pub async fn update_cost_line(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, line_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<UpdateCostLineRequest>,
) -> ApiResult<Json<CostLineMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_cost_line",
                entity_id: Some(line_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View).await?;
        let before = core_db::costs::list_cost_lines(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|l| l.id == line_id)
            .ok_or(ApiError::NotFound)?;
        core_db::costs::update_cost_line(
            &mut ctx.tx,
            org_id,
            line_id,
            req.description.as_deref().map(str::trim),
            req.quantity,
            req.unit_cost_minor,
            req.billable,
        )
        .await?;
        let after = core_db::costs::list_cost_lines(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|l| l.id == line_id)
            .ok_or(ApiError::NotFound)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.cost_line_updated".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "line_id": line_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_COST_LINE_UPDATED,
            "ticket_cost_line",
            Some(line_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCostLineAdded {
                line: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_cost_line",
                entity_id: line_id,
                version: req.instant.version,
            },
        )
        .await?;
        let before_json = serde_json::to_value(&before).unwrap_or(serde_json::Value::Null);
        let stored = CostLineMutationResponse {
            line: after.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: Some(before_json.clone()),
                after: serde_json::Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_cost_line",
                entity_id: line_id,
                op: "update",
                seq: write_seq,
                before: Some(before_json),
                after: serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CostLineMutationResponse {
                line: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/tickets/{id}/cost-lines/{line_id} — remove an unbilled line.
#[utoipa::path(operation_id = "tickets_remove_cost_line", delete, path = "/api/tickets/{id}/cost-lines/{line_id}", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("line_id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "line not found in scope"),
              (status = 409, description = "already billed")))]
pub async fn remove_cost_line(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, line_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View).await?;
        let removed = core_db::costs::remove_cost_line(&mut ctx.tx, org_id, line_id).await?;
        if !removed {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.cost_line_removed".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "line_id": line_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_COST_LINE_REMOVED,
            "ticket_cost_line",
            Some(line_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketCostLineRemoved {
                line_id,
                ticket_id: id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "removed": true })))
    })
    .await
    .map(Json)
}

// --- expenses --------------------------------------------------------------------

/// GET /api/tickets/{id}/expenses — the ticket's expense entries.
#[utoipa::path(operation_id = "tickets_list_expenses", get, path = "/api/tickets/{id}/expenses", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [TicketExpense])))]
pub async fn list_expenses(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<TicketExpense>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    ticket_for_board_check(&mut tx, &authed.principal, org_id, id, BoardOp::View).await?;
    let expenses = core_db::costs::list_expenses(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(expenses))
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct CreateExpenseRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub description: String,
    pub amount_minor: i64,
    pub currency: String,
    #[serde(default)]
    pub billable: bool,
}

/// POST /api/tickets/{id}/expenses — record an expense entry.
#[utoipa::path(operation_id = "tickets_create_expense", post, path = "/api/tickets/{id}/expenses", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateExpenseRequest,
    responses((status = 200, body = ExpenseMutationResponse),
              (status = 404, description = "ticket not found in scope"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "invalid amount/currency")))]
pub async fn create_expense(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateExpenseRequest>,
) -> ApiResult<Json<ExpenseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if req.description.trim().is_empty() {
        return Err(ApiError::Validation("description is required".into()));
    }
    if req.amount_minor < 0 {
        return Err(ApiError::Validation("amount_minor must be >= 0".into()));
    }
    if req.currency.len() != 3 || !req.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(ApiError::Validation(
            "currency must be a 3-letter ISO 4217 code".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_expense",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let ticket =
            ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View)
                .await?;
        let expense = TicketExpense {
            id: uuid::Uuid::now_v7(),
            org_id,
            ticket_id: id,
            description: req.description.trim().to_string(),
            amount_minor: req.amount_minor,
            currency: req.currency.clone(),
            billable: req.billable,
            invoice_line_id: None,
            incurred_by: Some(user_id),
            incurred_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        core_db::costs::insert_expense(&mut *ctx.tx, &expense).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.expense_recorded".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "amount_minor": expense.amount_minor,
                    "currency": expense.currency,
                    "billable": expense.billable,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_EXPENSE_RECORDED,
            "ticket_expense",
            Some(expense.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketExpenseRecorded {
                expense: expense.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_expense",
                entity_id: expense.id,
                version: req.instant.version,
            },
        )
        .await?;
        let stored = ExpenseMutationResponse {
            expense: expense.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: None,
                after: serde_json::Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_expense",
                entity_id: expense.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&expense).unwrap_or(serde_json::Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        let _ = ticket;
        Ok((
            ctx,
            MutationOutcome::Applied(ExpenseMutationResponse { expense, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Serialize, ToSchema, serde::Deserialize)]
pub struct ExpenseMutationResponse {
    pub expense: TicketExpense,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct UpdateExpenseRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    pub description: Option<String>,
    pub amount_minor: Option<i64>,
    pub billable: Option<bool>,
}

/// PATCH /api/tickets/{id}/expenses/{expense_id} — edit an unbilled expense.
#[utoipa::path(operation_id = "tickets_update_expense", patch, path = "/api/tickets/{id}/expenses/{expense_id}", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("expense_id" = Uuid, Path)),
    request_body = UpdateExpenseRequest,
    responses((status = 200, body = ExpenseMutationResponse),
              (status = 404, description = "expense not found in scope"),
              (status = 409, description = "already billed, version conflict, or idempotency key reused")))]
pub async fn update_expense(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, expense_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<UpdateExpenseRequest>,
) -> ApiResult<Json<ExpenseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "ticket_expense",
                entity_id: Some(expense_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View).await?;
        let before = core_db::costs::list_expenses(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|e| e.id == expense_id)
            .ok_or(ApiError::NotFound)?;
        core_db::costs::update_expense(
            &mut ctx.tx,
            org_id,
            expense_id,
            req.description.as_deref().map(str::trim),
            req.amount_minor,
            req.billable,
        )
        .await?;
        let after = core_db::costs::list_expenses(&mut *ctx.tx, org_id, id)
            .await?
            .into_iter()
            .find(|e| e.id == expense_id)
            .ok_or(ApiError::NotFound)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.expense_updated".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "expense_id": expense_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_EXPENSE_UPDATED,
            "ticket_expense",
            Some(expense_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketExpenseRecorded {
                expense: after.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_expense",
                entity_id: expense_id,
                version: req.instant.version,
            },
        )
        .await?;
        let before_json = serde_json::to_value(&before).unwrap_or(serde_json::Value::Null);
        let stored = ExpenseMutationResponse {
            expense: after.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: chrono::Utc::now(),
                before: Some(before_json.clone()),
                after: serde_json::Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_expense",
                entity_id: expense_id,
                op: "update",
                seq: write_seq,
                before: Some(before_json),
                after: serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
                updated_at: chrono::Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ExpenseMutationResponse {
                expense: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/tickets/{id}/expenses/{expense_id} — remove an unbilled expense.
#[utoipa::path(operation_id = "tickets_remove_expense", delete, path = "/api/tickets/{id}/expenses/{expense_id}", tag = "ticket-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path), ("expense_id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "expense not found in scope"),
              (status = 409, description = "already billed")))]
pub async fn remove_expense(
    State(state): State<AppState>,
    extensions: Extensions,
    Path((id, expense_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        ticket_for_board_check(&mut ctx.tx, &authed.principal, org_id, id, BoardOp::View).await?;
        let removed = core_db::costs::remove_expense(&mut ctx.tx, org_id, expense_id).await?;
        if !removed {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: uuid::Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "ticket.expense_removed".into(),
                entity_type: Some("ticket".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "expense_id": expense_id }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            types::TICKET_EXPENSE_REMOVED,
            "ticket_expense",
            Some(expense_id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketExpenseRemoved {
                expense_id,
                ticket_id: id,
            })
            .unwrap_or_default(),
        )
        .await?;
        Ok((ctx, serde_json::json!({ "removed": true })))
    })
    .await
    .map(Json)
}

// --- invoicing candidate feed ------------------------------------------------------

/// GET /api/invoice-candidates — every billable, not-yet-billed cost line
/// and expense in the org, oldest first. The invoicing run consumes rows
/// by writing `invoice_line_id` through the costs module's marker.
#[utoipa::path(operation_id = "invoice_candidates_list", get, path = "/api/invoice-candidates", tag = "ticket-costs", security(("bearer" = [])),
    responses((status = 200, body = [InvoiceCandidateView])))]
pub async fn list_candidates(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<InvoiceCandidateView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::costs::list_invoice_candidates(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter().map(InvoiceCandidateView::from).collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceCandidateView {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: uuid::Uuid,
    #[schema(value_type = Option<uuid::Uuid>, nullable = true)]
    pub client_id: Option<uuid::Uuid>,
    /// "cost" | "expense"
    pub source: String,
    #[schema(value_type = uuid::Uuid)]
    pub line_id: uuid::Uuid,
    pub description: String,
    pub amount_minor: i64,
    pub currency: String,
}

impl From<core_db::costs::InvoiceCandidateLine> for InvoiceCandidateView {
    fn from(c: core_db::costs::InvoiceCandidateLine) -> Self {
        Self {
            ticket_id: c.ticket_id,
            client_id: c.client_id,
            source: c.source.to_string(),
            line_id: c.line_id,
            description: c.description,
            amount_minor: c.amount_minor,
            currency: c.currency,
        }
    }
}
