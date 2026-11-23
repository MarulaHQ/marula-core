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
use crate::routes::fx::lock_snapshot;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    routing::{get, patch, post},
    Json, Router,
};
use core_domain::entities::{Actor, AuditEvent, CostLineKind, TicketCostLine, TicketExpense};
use core_domain::events::types;
use core_domain::policy::{Action, BoardOp, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
        .route(
            &format!("{base}/invoice-candidates/bill"),
            post(bill_candidates),
        )
}

fn view_financials() -> Permission {
    Permission::ViewFinancials
}

/// Loads a ticket and authorizes the caller's board access for `op`. Shared
/// with the purchasing recharge path, which writes a cost line on a ticket the
/// purchase serves and must obey the same board gate as this surface.
pub(crate) async fn ticket_for_board_check(
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

// --- recharge leg (purchasing) ---------------------------------------------------

/// What an operator is told when they try to edit the cost line an issued
/// recharge produced. A recharge is a fact, so its line is append-only; the
/// correction depends on how far it travelled, which the guard decides.
const RECHARGE_LINE_BILLED_REFUSAL: &str = "this line is the charge of an issued recharge that is already on an invoice; correct the invoice with a credit note";

/// An unbilled recharge has no invoice to credit, and its receipt cannot be
/// replayed to re-price it. Deleting the line is the whole reversal: the
/// recharge row and the removal event keep the record.
const RECHARGE_LINE_UNBILLED_REFUSAL: &str = "this line is the charge of an issued recharge and is append-only; it is not on an invoice, so delete it to reverse the charge";

/// One client recharge to post as a billable ticket cost line.
///
/// The purchasing receipt engine calls this from inside its own transaction, so
/// a receipt that fails later never leaves a charge behind. The `billable` flag
/// rides from the purchase line: the operator's statement about whether this
/// charge goes on the client's bill. Source identity (`purchase_receipt_line` +
/// the receipt line id) is stamped in the same commit, which is the citation the
/// billing join extends.
pub(crate) struct RechargeCostLine<'a> {
    pub ticket_id: Uuid,
    pub kind: CostLineKind,
    pub description: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub reporting_class: &'a str,
    /// Whether the charge goes on the client's bill. Comes from the purchase
    /// line, which is where the operator said so.
    pub billable: bool,
    /// The receipt line that caused the charge.
    pub receipt_line_id: Uuid,
}

impl RechargeCostLine<'_> {
    fn validate(&self) -> ApiResult<()> {
        if self.amount_minor < 0 {
            return Err(ApiError::Validation(
                "a recharge amount must not be negative".into(),
            ));
        }
        if !core_domain::entities::costs::is_valid_reporting_class(self.reporting_class) {
            return Err(ApiError::Validation(format!(
                "unknown reporting class: {}",
                self.reporting_class
            )));
        }
        Ok(())
    }
}

/// Posts the recharge line and returns it with the FX snapshot that was locked
/// for it, so the purchase stays the one writer of the charge record.
pub(crate) async fn write_recharge_cost_line(
    ctx: &mut crate::org_tx::OrgTx<'_>,
    principal: &core_domain::entities::Principal,
    input: RechargeCostLine<'_>,
) -> ApiResult<(TicketCostLine, Option<crate::routes::fx::LockedSnapshot>)> {
    input.validate()?;
    // The direct cost-line route refuses a caller without ViewFinancials, so the
    // receipt path must too: receiving a client purchase must not be a second
    // door onto a financial ticket write.
    require_permission(principal, view_financials())?;
    let org_id = ctx.org_id;
    let user_id = ctx.user_id;
    ticket_for_board_check(
        &mut ctx.tx,
        principal,
        org_id,
        input.ticket_id,
        BoardOp::WorkTicket,
    )
    .await?;
    let now = chrono::Utc::now();
    let snapshot = lock_snapshot(
        &mut ctx.tx,
        org_id,
        input.amount_minor,
        input.currency,
        now.date_naive(),
    )
    .await?;
    let line = TicketCostLine {
        id: Uuid::now_v7(),
        org_id,
        ticket_id: input.ticket_id,
        kind: input.kind,
        description: input.description.to_string(),
        quantity: 1.0,
        unit_cost_minor: input.amount_minor,
        currency: input.currency.to_string(),
        billable: input.billable,
        fx_rate_used: snapshot.as_ref().map(|s| s.rate_used.clone()),
        fx_rate_date: snapshot.as_ref().map(|s| s.rate_date),
        fx_source: snapshot.as_ref().map(|s| s.source.clone()),
        base_amount_minor: snapshot.as_ref().map(|s| s.base_amount_minor),
        invoice_line_id: None,
        created_by: Some(user_id),
        created_at: now,
        updated_at: now,
    };
    core_db::costs::insert_cost_line(&mut *ctx.tx, &line).await?;
    core_db::purchasing::stamp_cost_line_source(
        &mut *ctx.tx,
        org_id,
        line.id,
        input.receipt_line_id,
        input.reporting_class,
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
    Ok((line, snapshot))
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
    responses((status = 200, body = CostLineMutationResponse),
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
        // ADR 0002 lock moment (expense/purchase entry): a fact entered in
        // a non-base currency locks its conversion snapshot at entry. The
        // fact amount is computed in integer space with the documented
        // half-even rule (§5 — no float multiplies near money) so the
        // reconciliation read model reproduces it bit-for-bit.
        let today = chrono::Utc::now().date_naive();
        let fact_amount = core_domain::fx::fact_amount_minor(req.quantity, req.unit_cost_minor)
            .ok_or_else(|| ApiError::Validation("quantity × unit cost out of range".into()))?;
        let snap = lock_snapshot(&mut ctx.tx, org_id, fact_amount, &req.currency, today).await?;
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
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
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
        if let Some(billed) =
            core_db::purchasing::recharge_cost_line_state(&mut *ctx.tx, org_id, line_id).await?
        {
            let message = if billed {
                RECHARGE_LINE_BILLED_REFUSAL
            } else {
                RECHARGE_LINE_UNBILLED_REFUSAL
            };
            return Err(ApiError::Conflict(message.into()));
        }
        // A reversed consumption put its money back and left the negative line
        // as the record. Re-arming the original charge would subtract it twice,
        // so a stock-sourced line whose movement was reversed cannot be made
        // billable again.
        if req.billable == Some(true) {
            let reversed = sqlx::query_scalar!(
                r#"SELECT EXISTS(
                     SELECT 1 FROM stock_movements m
                       JOIN stock_movements r
                         ON r.org_id = m.org_id AND r.reverses_movement_id = m.id
                      WHERE m.org_id = $1 AND m.ticket_cost_line_id = $2)"#,
                org_id,
                line_id,
            )
            .fetch_one(&mut *ctx.tx)
            .await?;
            if reversed.unwrap_or(false) {
                return Err(ApiError::Conflict(
                    "this consumption was reversed; its charge cannot be made billable again"
                        .into(),
                ));
            }
        }
        // Snapshot doctrine (ADR 0002): a fact that already locked an FX
        // snapshot cannot have its amount edited in place — the locked
        // `base_amount_minor` would silently stop matching the row's own
        // quantity × unit cost. Amount corrections are new facts: remove
        // and re-enter. Description/billable edits stay allowed.
        if before.fx_rate_used.is_some()
            && (req.quantity.is_some_and(|q| {
                core_domain::fx::quantize_quantity(q)
                    != core_domain::fx::quantize_quantity(before.quantity)
            }) || req
                .unit_cost_minor
                .is_some_and(|u| u != before.unit_cost_minor))
        {
            return Err(ApiError::Conflict(
                "cost line carries an FX snapshot; amount corrections are new facts, \
                 not edits — remove and re-enter"
                    .into(),
            ));
        }
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
        // A3: the route names the ticket, but the delete is by line id alone.
        // Resolve the line through the ticket's own collection first, so ticket
        // A's URL can never delete ticket B's line (a cost line's ticket is
        // immutable, so this stays true for the rest of the transaction).
        let belongs = core_db::costs::list_cost_lines(&mut *ctx.tx, org_id, id)
            .await?
            .iter()
            .any(|line| line.id == line_id);
        if !belongs {
            return Err(ApiError::NotFound);
        }
        // An unbilled recharge line has no invoice to credit, so deleting it is
        // the reversal; a billed one is corrected by a credit note. The delete
        // itself re-checks `invoice_line_id`, so this is a message choice, not
        // the guard against losing a billed line.
        if let Some(true) =
            core_db::purchasing::recharge_cost_line_state(&mut *ctx.tx, org_id, line_id).await?
        {
            return Err(ApiError::Conflict(RECHARGE_LINE_BILLED_REFUSAL.into()));
        }
        // A stock-sourced line is the other half of a movement that names it,
        // and that row is append-only. Deleting the line would leave the
        // movement citing nothing; the reversal path is the undo.
        let source_kind = sqlx::query_scalar!(
            r#"SELECT source_kind FROM ticket_cost_lines WHERE org_id = $1 AND id = $2"#,
            org_id,
            line_id,
        )
        .fetch_one(&mut *ctx.tx)
        .await?;
        if source_kind.as_deref() == Some("stock_movement") {
            return Err(ApiError::Conflict(
                "this line came from a stock consumption; reverse the consumption instead of deleting it"
                    .into(),
            ));
        }
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
        let today = chrono::Utc::now().date_naive();
        let snap =
            lock_snapshot(&mut ctx.tx, org_id, req.amount_minor, &req.currency, today).await?;
        let expense = TicketExpense {
            id: uuid::Uuid::now_v7(),
            org_id,
            ticket_id: id,
            description: req.description.trim().to_string(),
            amount_minor: req.amount_minor,
            currency: req.currency.clone(),
            billable: req.billable,
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
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
        if let Some(layer) =
            core_db::acquisition_costs::layer_for_expense(&mut ctx.tx, org_id, expense_id).await?
        {
            return Err(ApiError::Conflict(format!(
                "this expense was posted by acquisition cost layer {layer}; reverse the layer \
                 instead of editing the expense"
            )));
        }
        // Snapshot doctrine (ADR 0002): same rule as cost lines — an
        // amount edit on a snapshotted expense would desynchronize the
        // locked base amount. Corrections are new facts.
        if before.fx_rate_used.is_some()
            && req.amount_minor.is_some_and(|a| a != before.amount_minor)
        {
            return Err(ApiError::Conflict(
                "expense carries an FX snapshot; amount corrections are new facts, \
                 not edits — remove and re-enter"
                    .into(),
            ));
        }
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
        // A3: same binding as the cost-line delete — the expense must be one of
        // THIS ticket's expenses before it is removed by id.
        let belongs = core_db::costs::list_expenses(&mut *ctx.tx, org_id, id)
            .await?
            .iter()
            .any(|expense| expense.id == expense_id);
        if !belongs {
            return Err(ApiError::NotFound);
        }
        if let Some(layer) =
            core_db::acquisition_costs::layer_for_expense(&mut ctx.tx, org_id, expense_id).await?
        {
            return Err(ApiError::Conflict(format!(
                "this expense was posted by acquisition cost layer {layer}; reverse the layer \
                 instead of removing the expense"
            )));
        }
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

/// The tickets whose lines the request names, so board access can be checked
/// before anything is marked billed.
async fn candidate_ticket_ids(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    cost_line_ids: &[Uuid],
    expense_ids: &[Uuid],
) -> ApiResult<Vec<Uuid>> {
    let mut tickets: Vec<Uuid> = Vec::new();
    if !cost_line_ids.is_empty() {
        let rows = sqlx::query_scalar!(
            r#"SELECT DISTINCT ticket_id FROM ticket_cost_lines
                WHERE org_id = $1 AND id = ANY($2)"#,
            org_id,
            cost_line_ids,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(ApiError::from)?;
        tickets.extend(rows);
    }
    if !expense_ids.is_empty() {
        let rows = sqlx::query_scalar!(
            r#"SELECT DISTINCT ticket_id FROM ticket_expenses
                WHERE org_id = $1 AND id = ANY($2)"#,
            org_id,
            expense_ids,
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(ApiError::from)?;
        tickets.extend(rows);
    }
    tickets.sort_unstable();
    tickets.dedup();
    Ok(tickets)
}

/// What to bill, and under which contract.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BillCandidatesRequest {
    #[schema(value_type = uuid::Uuid)]
    pub contract_id: uuid::Uuid,
    /// Ticket cost lines to bill.
    #[serde(default)]
    #[schema(value_type = Vec<uuid::Uuid>)]
    pub cost_line_ids: Vec<uuid::Uuid>,
    /// Ticket expenses to bill.
    #[serde(default)]
    #[schema(value_type = Vec<uuid::Uuid>)]
    pub expense_ids: Vec<uuid::Uuid>,
    pub period_from: Option<chrono::NaiveDate>,
    pub period_to: Option<chrono::NaiveDate>,
    pub due_on: chrono::NaiveDate,
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct BilledCandidatesResponse {
    pub invoice: core_domain::entities::Invoice,
    pub lines: Vec<core_domain::entities::InvoiceLine>,
    pub mutation: instant::MutationMeta,
}

/// POST /api/invoice-candidates/bill — issue one invoice for the named
/// candidates, under one contract. The lines keep their source, the candidates
/// are marked billed in the same transaction, and an already-billed candidate
/// refuses the request. Totals move only by this issued document.
#[utoipa::path(operation_id = "invoice_candidates_bill", post, path = "/api/invoice-candidates/bill", tag = "ticket-costs", security(("bearer" = [])),
    request_body = BillCandidatesRequest,
    responses((status = 200, body = BilledCandidatesResponse),
              (status = 404, description = "contract or candidate not visible in scope"),
              (status = 409, description = "a candidate is already billed, or the currencies disagree"),
              (status = 422, description = "validation failure")))]
pub async fn bill_candidates(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<BillCandidatesRequest>,
) -> ApiResult<Json<BilledCandidatesResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: core_domain::policy::Module::Tickets,
            action: Action::Create,
        },
    )?;
    require_permission(&authed.principal, view_financials())?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_candidate_bill",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            // The stored answer must be this endpoint's own shape; a foreign
            // payload fails closed rather than being served.
            let _stored_answer: BilledCandidatesResponse = serde_json::from_value(stored.clone())
                .map_err(|_| {
                ApiError::Internal(anyhow::anyhow!(
                    "a stored candidate bill is not this endpoint's shape"
                ))
            })?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let contract = core_db::contracts::get_contract(&mut *ctx.tx, org_id, req.contract_id)
            .await
            .map_err(ApiError::from)?;
        crate::routes::clients::require_client_visible(
            &mut ctx.tx,
            org_id,
            contract.client_id,
            &authed.principal,
        )
        .await?;
        // Board access is the same gate the ticket-cost reads and mutations
        // apply. A caller who cannot see a candidate's board cannot bill its
        // line: the invoice would reveal the description and mark it billed.
        for ticket_id in
            candidate_ticket_ids(&mut ctx.tx, org_id, &req.cost_line_ids, &req.expense_ids).await?
        {
            ticket_for_board_check(
                &mut ctx.tx,
                &authed.principal,
                org_id,
                ticket_id,
                BoardOp::WorkTicket,
            )
            .await?;
        }
        let today = chrono::Utc::now().date_naive();
        let (_base, resolved) =
            crate::routes::fx::resolve_lock_rate(&mut ctx.tx, org_id, &contract.currency, today)
                .await?;
        let fx =
            resolved.map(
                |(rate, rate_used, rate_date, source)| core_db::invoices::CandidateFx {
                    rate,
                    rate_used,
                    rate_date,
                    source,
                },
            );

        let (invoice, lines) = core_db::invoices::bill_candidates(
            &mut ctx.tx,
            org_id,
            &core_db::invoices::BillCandidatesRequest {
                contract_id: req.contract_id,
                fx,
                cost_line_ids: req.cost_line_ids.clone(),
                expense_ids: req.expense_ids.clone(),
                period_from: req.period_from,
                period_to: req.period_to,
                due_on: req.due_on,
                notes: req.notes.clone(),
                created_by: user_id,
            },
        )
        .await?;
        let payload = serde_json::json!({
            "invoice": invoice.clone(),
            "lines": lines.clone(),
        });
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice",
                entity_id: invoice.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::INVOICE_CREATED,
            "invoice",
            Some(invoice.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::InvoiceCreated {
                invoice: invoice.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice_candidate_bill",
                entity_id: invoice.id,
                op: "bill",
                seq: write_seq,
                before: None,
                after: payload.clone(),
                updated_at: invoice.created_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(BilledCandidatesResponse {
                invoice,
                lines,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvoiceCandidateView {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: uuid::Uuid,
    /// The ticket's human number and title. Present so the ledger can name it.
    pub ticket_number: String,
    pub ticket_title: String,
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
            ticket_number: c.ticket_number,
            ticket_title: c.ticket_title,
            client_id: c.client_id,
            source: c.source.to_string(),
            line_id: c.line_id,
            description: c.description,
            amount_minor: c.amount_minor,
            currency: c.currency,
        }
    }
}
