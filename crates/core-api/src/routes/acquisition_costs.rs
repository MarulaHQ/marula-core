//! Acquisition cost layers API (ADR 0035 decision 3, plan §6; migration 0258).
//!
//! A receipt is the cost fact; freight, duty, brokerage, insurance and
//! handling arrive later as documents. This surface records one such document
//! as a layer over a receipt, splits it over the receipt's lines, and answers
//! two reads: what a receipt is now worth, and what the organisation spent
//! under each reporting class.
//!
//! Three rules the handlers keep rather than delegate:
//!
//!   * Nothing here edits cost. The only correction is
//!     `POST /api/acquisition-costs/{id}/reverse`, which records a new layer
//!     naming the one it reverses. The database refuses an update to either
//!     table, so a route that tried would fail rather than be reviewed.
//!   * The organisation's mode is read once, when the layer is recorded, and
//!     stored on the layer with the decision it produced. A later switch
//!     changes the next layer, never this one.
//!   * The FX snapshot is locked here, at the document's own date, through the
//!     shared `lock_snapshot` path every other money fact uses. The reads never
//!     convert again; they sum stored base amounts.
//!
//! In `invoice_values` mode (the default) a charge that does not capitalise
//! posts as a ticket expense under its reporting class, so the ticket's money
//! reflects it. A reversal posts the negated expense against the same ticket,
//! because a correction is a document, not an edit. A purchase order with no
//! ticket has no ticket ledger to post to: the layer itself is the classed
//! record, attributed to the project, and the class rollup and the accountant
//! handoff read it from there.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::fx::lock_snapshot;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use core_db::acquisition_costs::{
    self, AcquisitionCost, AcquisitionCostAllocation, AcquisitionCostDetail, CostClassTotal,
    ReceiptValuation, RollupFilter, SplitError,
};
use core_domain::entities::{Actor, AuditEvent, LandedCostMode, TicketExpense};
use core_domain::policy::{Action, BoardOp, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "acquisition_cost";
const DEFAULT_ALLOCATION_METHOD: &str = "extended_value";

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

/// A money read needs the financial permission as well as commercial view: the
/// layer is a purchase document, and its amounts are money. Writes need it too,
/// so a caller who cannot see money cannot create a charge document either —
/// the same posture the ticket expense surface takes.
fn require_money(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    require_permission(principal, commercial(Action::View))?;
    require_permission(principal, Permission::ViewFinancials)
}

fn require_money_write(principal: &core_domain::entities::Principal) -> ApiResult<()> {
    require_permission(principal, commercial(Action::Edit))?;
    require_permission(principal, Permission::ViewFinancials)
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/purchase-receipts/{{id}}/acquisition-costs"),
            get(list_for_receipt).post(create),
        )
        .route(
            &format!("{base}/purchase-receipts/{{id}}/valuation"),
            get(receipt_valuation),
        )
        .route(
            &format!("{base}/acquisition-costs/summary"),
            get(class_rollup),
        )
        .route(&format!("{base}/acquisition-costs/{{id}}"), get(get_one))
        .route(
            &format!("{base}/acquisition-costs/{{id}}/reverse"),
            post(reverse),
        )
}

// ---- wire types ------------------------------------------------------------

/// POST /api/purchase-receipts/{id}/acquisition-costs.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateAcquisitionCostRequest {
    /// freight | duty | brokerage | insurance | handling.
    pub kind: String,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub vendor_id: Option<Uuid>,
    /// The vendor's own invoice number.
    #[serde(default)]
    pub vendor_reference: Option<String>,
    /// Our reference for the document. Required: it is the stable name the
    /// accountant handoff cites.
    pub reference: String,
    /// The document's own date, and the date the FX rate is locked at.
    #[schema(value_type = String)]
    pub document_date: NaiveDate,
    /// ISO 4217, uppercase.
    pub currency: String,
    /// Integer minor units; must be positive.
    pub amount_minor: i64,
    /// extended_value (default) | quantity | weight.
    #[serde(default)]
    pub allocation_method: Option<String>,
    /// Whether this charge should enter inventory cost. Default true; it only
    /// takes effect in `landed_costs` mode.
    #[serde(default)]
    pub capitalise: Option<bool>,
    /// The class the charge reports under when it does not capitalise. Defaults
    /// from the kind; `inventory` is refused because it is the capitalised
    /// class.
    #[serde(default)]
    pub reporting_class: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/acquisition-costs/{id}/reverse.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReverseAcquisitionCostRequest {
    /// Our reference for the correcting document (the vendor's credit note).
    pub reference: String,
    #[schema(value_type = String)]
    pub document_date: NaiveDate,
    #[serde(default)]
    pub vendor_reference: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/acquisition-costs/summary.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CostRollupQuery {
    #[serde(default)]
    pub purchase_order_id: Option<Uuid>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub ticket_id: Option<Uuid>,
    #[serde(default)]
    pub from: Option<NaiveDate>,
    #[serde(default)]
    pub to: Option<NaiveDate>,
}

/// A mutation answers with the whole layer and the instant-first metadata, so
/// the caller's next write can carry the version it just read.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AcquisitionCostMutationResponse {
    #[serde(flatten)]
    pub detail: AcquisitionCostDetail,
    pub mutation: instant::MutationMeta,
}

// ---- validation ------------------------------------------------------------

/// Trims optional free text and treats an all-whitespace value as absent, so an
/// empty string cannot reach a column whose CHECK requires a trimmed value.
fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn require_max_len(value: &str, max: usize, field: &str) -> ApiResult<()> {
    if value.chars().count() > max {
        return Err(ApiError::Validation(format!(
            "{field} must be at most {max} characters"
        )));
    }
    Ok(())
}

fn require_currency(currency: &str) -> ApiResult<String> {
    let code = core_domain::money::CurrencyCode::new(currency)
        .map_err(|_| ApiError::Validation("currency must be a 3-letter ISO 4217 code".into()))?;
    if code.as_str() != currency {
        return Err(ApiError::Validation(
            "currency must be an uppercase ISO 4217 code".into(),
        ));
    }
    Ok(code.as_str().to_string())
}

fn split_error(e: SplitError) -> ApiError {
    match e {
        // The caller asked for a basis the receipt cannot provide: a 422 that
        // names the line to fix.
        SplitError::EmptyBasis(_) | SplitError::MissingWeight { .. } | SplitError::NoLines => {
            ApiError::Validation(e.to_string())
        }
        // Arithmetic that did not reconcile is our bug, not the caller's.
        SplitError::Unbalanced => ApiError::Internal(anyhow::anyhow!(e.to_string())),
    }
}

/// The class a layer reports under, given what it is and whether it capitalises.
/// A capitalised charge is inventory; anything else keeps its own class, which
/// the operator may name and which otherwise follows the document's kind.
fn resolve_reporting_class(
    kind: &str,
    capitalised: bool,
    named: Option<&str>,
) -> ApiResult<String> {
    let named = named.map(str::trim).filter(|s| !s.is_empty());
    if capitalised {
        return match named {
            None | Some("inventory") => Ok("inventory".into()),
            Some(other) => Err(ApiError::Validation(format!(
                "a capitalised charge reports as inventory; \"{other}\" would be an expense, so \
                 turn capitalise off first"
            ))),
        };
    }
    match named {
        None => Ok(acquisition_costs::default_reporting_class(kind).to_string()),
        Some("inventory") => Err(ApiError::Validation(
            "inventory is the capitalised class; a charge that does not capitalise reports under \
             its own class"
                .into(),
        )),
        Some(class) => {
            if !core_domain::entities::costs::is_valid_reporting_class(class) {
                return Err(ApiError::Validation(format!(
                    "unknown reporting class: {class}"
                )));
            }
            Ok(class.to_string())
        }
    }
}

/// What a layer carries into the ticket expense it posts: the amount in the
/// layer's currency, the locked snapshot it was converted at, its base amount,
/// and the description the expense shows.
struct LayerCharge<'a> {
    amount_minor: i64,
    currency: &'a str,
    snapshot: Option<&'a crate::routes::fx::LockedSnapshot>,
    base_amount_minor: Option<i64>,
    description: &'a str,
}

/// The ticket expense a non-capitalised layer posts, carrying the layer's own
/// locked snapshot so one document has one rate. `None` when there is no ticket
/// to post against.
fn expense_for_layer(
    org_id: Uuid,
    user_id: Uuid,
    ticket_id: Option<Uuid>,
    charge: LayerCharge<'_>,
    now: DateTime<Utc>,
) -> Option<TicketExpense> {
    let ticket_id = ticket_id?;
    let LayerCharge {
        amount_minor,
        currency,
        snapshot,
        base_amount_minor,
        description,
    } = charge;
    Some(TicketExpense {
        id: Uuid::now_v7(),
        org_id,
        ticket_id,
        description: description.to_string(),
        amount_minor,
        currency: currency.to_string(),
        // A cost we incurred is not a recharge the client agreed to pay; a
        // recharge is a separate, priced document (plan §5).
        billable: false,
        fx_rate_used: snapshot.map(|s| s.rate_used.clone()),
        fx_rate_date: snapshot.map(|s| s.rate_date),
        fx_source: snapshot.map(|s| s.source.clone()),
        base_amount_minor,
        invoice_line_id: None,
        incurred_by: Some(user_id),
        incurred_at: now,
        created_at: now,
        updated_at: now,
    })
}

async fn insert_layer(
    ctx: &mut org_tx::OrgTx<'_>,
    cost: &AcquisitionCost,
    allocations: &[AcquisitionCostAllocation],
) -> ApiResult<()> {
    acquisition_costs::insert_cost(&mut ctx.tx, cost)
        .await
        .map_err(ApiError::from)?;
    acquisition_costs::insert_allocations(&mut ctx.tx, cost.org_id, cost.id, allocations)
        .await
        .map_err(ApiError::from)?;
    Ok(())
}

/// A restricted client's freight money is hidden from a caller the client does
/// not name, on every path that answers it.
async fn require_layer_client_visible(
    conn: &mut sqlx::PgConnection,
    principal: &core_domain::entities::Principal,
    org_id: Uuid,
    client_id: Option<Uuid>,
) -> ApiResult<()> {
    if let Some(client_id) = client_id {
        crate::routes::clients::require_client_visible(conn, org_id, client_id, principal)
            .await
            .map(|_client| ())?;
    }
    Ok(())
}

/// A stored replay is still a read of money on a ticket. Board access can have
/// narrowed since the write, so the replay branch runs the same check the fresh
/// path runs before it answers. A layer with no ticket needs no board check; a
/// stored value that is not this module's own shape fails closed.
async fn require_replay_visible(
    conn: &mut sqlx::PgConnection,
    principal: &core_domain::entities::Principal,
    org_id: Uuid,
    stored: &Value,
) -> ApiResult<()> {
    let ticket_id = stored
        .get("cost")
        .and_then(|cost| cost.get("ticket_id"))
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok());
    if let Some(ticket_id) = ticket_id {
        crate::routes::costs::ticket_for_board_check(
            conn,
            principal,
            org_id,
            ticket_id,
            BoardOp::WorkTicket,
        )
        .await?;
    }
    Ok(())
}

// ---- handlers --------------------------------------------------------------

/// GET /api/purchase-receipts/{id}/acquisition-costs.
#[utoipa::path(operation_id = "acquisition_costs_list_for_receipt", get, path = "/api/purchase-receipts/{id}/acquisition-costs", tag = "acquisition-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = [AcquisitionCostDetail]),
              (status = 404, description = "receipt not visible in scope")))]
pub async fn list_for_receipt(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(receipt_id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<AcquisitionCostDetail>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The receipt read is the scope proof: a missing receipt and a receipt in
    // another org answer the same 404.
    let context = acquisition_costs::receipt_context(&mut tx, org_id, receipt_id).await?;
    require_layer_client_visible(&mut tx, &authed.principal, org_id, context.client_id).await?;
    let layers = acquisition_costs::details_for_receipt(&mut tx, org_id, receipt_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(layers))
}

/// GET /api/acquisition-costs/{id}.
#[utoipa::path(operation_id = "acquisition_costs_get", get, path = "/api/acquisition-costs/{id}", tag = "acquisition-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AcquisitionCostDetail),
              (status = 404, description = "layer not visible in scope")))]
pub async fn get_one(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<AcquisitionCostDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let detail = acquisition_costs::detail(&mut tx, org_id, id).await?;
    require_layer_client_visible(&mut tx, &authed.principal, org_id, detail.cost.client_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

/// GET /api/purchase-receipts/{id}/valuation — receipt cost plus capitalised
/// layers, per line and in total. Every amount comes from a stored snapshot, so
/// two runs over the same receipt answer the same money.
#[utoipa::path(operation_id = "acquisition_costs_receipt_valuation", get, path = "/api/purchase-receipts/{id}/valuation", tag = "acquisition-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ReceiptValuation),
              (status = 404, description = "receipt not visible in scope")))]
pub async fn receipt_valuation(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(receipt_id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<ReceiptValuation>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let context = acquisition_costs::receipt_context(&mut tx, org_id, receipt_id).await?;
    require_layer_client_visible(&mut tx, &authed.principal, org_id, context.client_id).await?;
    let valuation = acquisition_costs::receipt_valuation(&mut tx, org_id, receipt_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(valuation))
}

/// GET /api/acquisition-costs/summary — base-currency totals per reporting
/// class, optionally narrowed to an order, project, ticket or date window.
#[utoipa::path(operation_id = "acquisition_costs_class_rollup", get, path = "/api/acquisition-costs/summary", tag = "acquisition-costs", security(("bearer" = [])),
    params(CostRollupQuery),
    responses((status = 200, body = [CostClassTotal]),
              (status = 422, description = "the date window is inverted")))]
pub async fn class_rollup(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<CostRollupQuery>,
) -> ApiResult<Json<Vec<CostClassTotal>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money(&authed.principal)?;
    if let (Some(from), Some(to)) = (q.from, q.to) {
        if from > to {
            return Err(ApiError::Validation(
                "the window's `from` must not be after its `to`".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let filter = RollupFilter {
        purchase_order_id: q.purchase_order_id,
        project_id: q.project_id,
        ticket_id: q.ticket_id,
        from: q.from,
        to: q.to,
        viewer_id: authed.principal.user.id,
        viewer_is_admin: authed.principal.is_owner_or_admin(),
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = acquisition_costs::class_rollup(&mut tx, org_id, &filter).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

/// POST /api/purchase-receipts/{id}/acquisition-costs — record one charge
/// document against a receipt and split it.
#[utoipa::path(operation_id = "acquisition_costs_create", post, path = "/api/purchase-receipts/{id}/acquisition-costs", tag = "acquisition-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateAcquisitionCostRequest,
    responses((status = 200, body = AcquisitionCostMutationResponse),
              (status = 404, description = "receipt or vendor not visible in scope"),
              (status = 409, description = "idempotency key reused or order not receivable"),
              (status = 422, description = "the charge or its split was refused")))]
pub async fn create(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(receipt_id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<CreateAcquisitionCostRequest>,
) -> ApiResult<Json<AcquisitionCostMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money_write(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let kind = req.kind.trim().to_lowercase();
    if !acquisition_costs::is_valid_kind(&kind) {
        return Err(ApiError::Validation(format!(
            "kind must be one of {}",
            acquisition_costs::COST_KINDS.join(", ")
        )));
    }
    let reference = req.reference.trim().to_string();
    if reference.is_empty() {
        return Err(ApiError::Validation("reference is required".into()));
    }
    require_max_len(&reference, 100, "reference")?;
    if let Some(vendor_reference) = req.vendor_reference.as_deref() {
        require_max_len(vendor_reference.trim(), 100, "vendor_reference")?;
    }
    let currency = require_currency(req.currency.trim())?;
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation(
            "amount_minor must be a positive number of minor units".into(),
        ));
    }
    let method = req
        .allocation_method
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_ALLOCATION_METHOD)
        .to_string();
    if !acquisition_costs::is_valid_allocation_method(&method) {
        return Err(ApiError::Validation(format!(
            "allocation_method must be one of {}",
            acquisition_costs::ALLOCATION_METHODS.join(", ")
        )));
    }
    let cost_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let vendor_reference = clean_text(req.vendor_reference.as_deref());
    let named_class = req.reporting_class.clone();
    let capitalise = req.capitalise.unwrap_or(true);

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, &authed.principal, org_id, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let context = acquisition_costs::receipt_context(&mut ctx.tx, org_id, receipt_id).await?;
        if let Some(vendor_id) = req.vendor_id {
            if !acquisition_costs::vendor_exists(&mut ctx.tx, org_id, vendor_id)
                .await
                .map_err(ApiError::from)?
            {
                return Err(ApiError::NotFound);
            }
        }
        require_layer_client_visible(&mut ctx.tx, &authed.principal, org_id, context.client_id)
            .await?;
        let policy = core_db::purchasing::get_inventory_policy(&mut *ctx.tx, org_id)
            .await
            .map_err(ApiError::from)?;
        // The mode is read once, here, and stored on the layer with the decision
        // it produced. A later switch changes the next layer, never this one.
        let capitalised = policy.landed_cost_mode == LandedCostMode::LandedCosts && capitalise;
        let reporting_class = resolve_reporting_class(&kind, capitalised, named_class.as_deref())?;

        let snapshot = lock_snapshot(
            &mut ctx.tx,
            org_id,
            req.amount_minor,
            &currency,
            req.document_date,
        )
        .await?;
        let base_amount_minor = snapshot.as_ref().map(|s| s.base_amount_minor);
        let base_total = base_amount_minor.unwrap_or(req.amount_minor);

        let base_currency = core_db::fx::base_currency(&mut *ctx.tx, org_id)
            .await
            .map_err(ApiError::from)?;
        let allocations = acquisition_costs::split_layer(
            cost_id,
            &currency,
            req.amount_minor,
            base_total,
            &base_currency,
            &context.lines,
            &method,
        )
        .map_err(split_error)?;

        // A charge that does not capitalise posts once, under its class, so the
        // ticket's money reflects it. It carries the layer's own locked rate: one
        // document, one rate.
        let expense = if capitalised {
            None
        } else {
            expense_for_layer(
                org_id,
                user_id,
                context.ticket_id,
                LayerCharge {
                    amount_minor: req.amount_minor,
                    currency: &currency,
                    snapshot: snapshot.as_ref(),
                    base_amount_minor,
                    description: &format!("{kind} {reference}"),
                },
                chrono::Utc::now(),
            )
        };
        if let Some(ticket_id) = expense.as_ref().map(|e| e.ticket_id) {
            crate::routes::costs::ticket_for_board_check(
                &mut ctx.tx,
                &authed.principal,
                org_id,
                ticket_id,
                BoardOp::WorkTicket,
            )
            .await?;
        }

        let cost = AcquisitionCost {
            id: cost_id,
            org_id,
            purchase_order_id: context.purchase_order_id,
            receipt_id,
            kind,
            vendor_id: req.vendor_id,
            vendor_reference,
            reference,
            document_date: req.document_date,
            currency,
            amount_minor: req.amount_minor,
            allocation_method: method,
            allocation_rule: "largest_remainder".into(),
            policy_mode: policy.landed_cost_mode.as_str().to_string(),
            capitalised,
            reporting_class,
            project_id: context.project_id,
            ticket_id: context.ticket_id,
            client_id: context.client_id,
            posted_ticket_expense_id: expense.as_ref().map(|e| e.id),
            reverses_acquisition_cost_id: None,
            fx_rate_used: snapshot.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snapshot.as_ref().map(|s| s.rate_date),
            fx_source: snapshot.as_ref().map(|s| s.source.clone()),
            base_amount_minor,
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
        };
        insert_layer(&mut ctx, &cost, &allocations).await?;
        if let Some(expense) = &expense {
            acquisition_costs::insert_classed_expense(&mut ctx.tx, expense, &cost.reporting_class)
                .await
                .map_err(ApiError::from)?;
        }

        let detail = AcquisitionCostDetail {
            cost: cost.clone(),
            allocations: allocations.clone(),
        };
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: cost_id,
                // A create has no prior version to match.
                version: None,
            },
        )
        .await?;
        ctx.emit(
            "acquisition_cost.recorded",
            ENTITY,
            Some(cost_id.to_string().as_str()),
            &serde_json::json!({
                "acquisition_cost_id": cost_id,
                "purchase_order_id": cost.purchase_order_id,
                "receipt_id": receipt_id,
                "kind": cost.kind,
                "capitalised": cost.capitalised,
                "reporting_class": cost.reporting_class,
                "amount_minor": cost.amount_minor,
                "base_amount_minor": cost.base_amount_minor,
                "line_count": allocations.len(),
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.acquisition_cost_recorded".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(cost_id.to_string()),
                detail: serde_json::json!({
                    "receipt_id": receipt_id,
                    "kind": cost.kind,
                    "capitalised": cost.capitalised,
                    "reporting_class": cost.reporting_class,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: cost_id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: cost.created_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AcquisitionCostMutationResponse { detail, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/acquisition-costs/{id}/reverse — record the correcting document.
///
/// The reversal copies the split it reverses, negated, and reuses the rate the
/// reversed layer locked, so the pair nets to zero in the base currency. It
/// does not re-split: a reversal that apportioned again could land a cent
/// somewhere the original did not, and the pair would no longer cancel.
#[utoipa::path(operation_id = "acquisition_costs_reverse", post, path = "/api/acquisition-costs/{id}/reverse", tag = "acquisition-costs", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReverseAcquisitionCostRequest,
    responses((status = 200, body = AcquisitionCostMutationResponse),
              (status = 404, description = "layer not visible in scope"),
              (status = 409, description = "layer already reversed, or is itself a reversal"),
              (status = 422, description = "the correcting document was refused")))]
pub async fn reverse(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<ReverseAcquisitionCostRequest>,
) -> ApiResult<Json<AcquisitionCostMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_money_write(&authed.principal)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let reference = req.reference.trim().to_string();
    if reference.is_empty() {
        return Err(ApiError::Validation("reference is required".into()));
    }
    require_max_len(&reference, 100, "reference")?;
    if let Some(vendor_reference) = req.vendor_reference.as_deref() {
        require_max_len(vendor_reference.trim(), 100, "vendor_reference")?;
    }
    let cost_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let vendor_reference = clean_text(req.vendor_reference.as_deref());

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            require_replay_visible(&mut ctx.tx, &authed.principal, org_id, &stored).await?;
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let original = acquisition_costs::get_cost(&mut ctx.tx, org_id, id).await?;
        if original.reverses_acquisition_cost_id.is_some() {
            return Err(ApiError::Conflict(
                "this layer is itself a reversal; a correction is a new charge, not a reversal of \
                 a reversal"
                    .into(),
            ));
        }
        if acquisition_costs::has_been_reversed(&mut ctx.tx, org_id, id).await? {
            return Err(ApiError::Conflict(format!(
                "layer {id} has already been reversed"
            )));
        }
        let original_allocations =
            acquisition_costs::list_allocations(&mut ctx.tx, org_id, id).await?;
        if original_allocations.is_empty() {
            return Err(ApiError::Conflict(
                "this layer has no split to reverse".into(),
            ));
        }
        // The reversal is expressed at the rate the original locked, so the pair
        // cancels exactly instead of leaving an FX difference on a mistake.
        let snapshot = match (
            original.fx_rate_used.as_ref(),
            original.fx_rate_date,
            original.fx_source.as_ref(),
        ) {
            (None, None, None) => None,
            (Some(rate_used), Some(rate_date), Some(source)) => {
                Some(crate::routes::fx::LockedSnapshot {
                    rate_used: rate_used.clone(),
                    rate_date,
                    source: source.clone(),
                    base_amount_minor: -original.base_amount(),
                })
            }
            _ => {
                return Err(ApiError::Internal(anyhow::anyhow!(
                    "layer {id} carries a partial fx snapshot"
                )))
            }
        };
        let base_amount_minor = original.base_amount_minor.map(|amount| -amount);
        let allocations: Vec<AcquisitionCostAllocation> = original_allocations
            .iter()
            .map(|allocation| AcquisitionCostAllocation {
                id: Uuid::now_v7(),
                acquisition_cost_id: cost_id,
                receipt_line_id: allocation.receipt_line_id,
                allocated_minor: -allocation.allocated_minor,
                allocated_base_minor: allocation.allocated_base_minor.map(|amount| -amount),
                basis_value: allocation.basis_value,
            })
            .collect();

        if original.posted_ticket_expense_id.is_some() && original.ticket_id.is_none() {
            return Err(ApiError::Internal(anyhow::anyhow!(
                "layer {id} posted an expense but names no ticket"
            )));
        }
        let expense = if original.posted_ticket_expense_id.is_some() {
            expense_for_layer(
                org_id,
                user_id,
                original.ticket_id,
                LayerCharge {
                    amount_minor: -original.amount_minor,
                    currency: &original.currency,
                    snapshot: snapshot.as_ref(),
                    base_amount_minor,
                    description: &format!("{} {} (reversal)", original.kind, reference),
                },
                chrono::Utc::now(),
            )
        } else {
            None
        };
        if let Some(ticket_id) = expense.as_ref().map(|e| e.ticket_id) {
            crate::routes::costs::ticket_for_board_check(
                &mut ctx.tx,
                &authed.principal,
                org_id,
                ticket_id,
                BoardOp::WorkTicket,
            )
            .await?;
        }

        let cost = AcquisitionCost {
            id: cost_id,
            org_id,
            purchase_order_id: original.purchase_order_id,
            receipt_id: original.receipt_id,
            kind: original.kind,
            vendor_id: original.vendor_id,
            vendor_reference,
            reference,
            document_date: req.document_date,
            currency: original.currency,
            amount_minor: -original.amount_minor,
            allocation_method: original.allocation_method,
            allocation_rule: "reversal_copy".into(),
            // The correction follows the document it corrects, so a mode switch
            // between the two never moves money between cost and expense.
            policy_mode: original.policy_mode,
            capitalised: original.capitalised,
            reporting_class: original.reporting_class,
            project_id: original.project_id,
            ticket_id: original.ticket_id,
            client_id: original.client_id,
            posted_ticket_expense_id: expense.as_ref().map(|e| e.id),
            reverses_acquisition_cost_id: Some(original.id),
            fx_rate_used: snapshot.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snapshot.as_ref().map(|s| s.rate_date),
            fx_source: snapshot.as_ref().map(|s| s.source.clone()),
            base_amount_minor,
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
        };
        insert_layer(&mut ctx, &cost, &allocations).await?;
        if let Some(expense) = &expense {
            acquisition_costs::insert_classed_expense(&mut ctx.tx, expense, &cost.reporting_class)
                .await
                .map_err(ApiError::from)?;
        }

        let detail = AcquisitionCostDetail {
            cost: cost.clone(),
            allocations: allocations.clone(),
        };
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: cost_id,
                // A create has no prior version to match.
                version: None,
            },
        )
        .await?;
        ctx.emit(
            "acquisition_cost.reversed",
            ENTITY,
            Some(cost_id.to_string().as_str()),
            &serde_json::json!({
                "acquisition_cost_id": cost_id,
                "reverses_acquisition_cost_id": original.id,
                "receipt_id": original.receipt_id,
                "amount_minor": cost.amount_minor,
            }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "purchase.acquisition_cost_reversed".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(cost_id.to_string()),
                detail: serde_json::json!({
                    "reverses_acquisition_cost_id": original.id,
                    "receipt_id": original.receipt_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: cost_id,
                op: "reverse",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&detail).unwrap_or(Value::Null),
                updated_at: cost.created_at,
                key: key.as_deref(),
                payload: &detail,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AcquisitionCostMutationResponse { detail, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capitalised_charge_reports_as_inventory() {
        assert_eq!(
            resolve_reporting_class("freight", true, None).expect("class"),
            "inventory"
        );
        assert_eq!(
            resolve_reporting_class("freight", true, Some("inventory")).expect("class"),
            "inventory"
        );
        assert!(resolve_reporting_class("freight", true, Some("freight_in")).is_err());
    }

    #[test]
    fn an_expensed_charge_defaults_from_its_kind() {
        assert_eq!(
            resolve_reporting_class("freight", false, None).expect("class"),
            "freight_in"
        );
        assert_eq!(
            resolve_reporting_class("duty", false, None).expect("class"),
            "duty"
        );
        assert_eq!(
            resolve_reporting_class("brokerage", false, None).expect("class"),
            "service_cost"
        );
        assert_eq!(
            resolve_reporting_class("insurance", false, Some("overhead")).expect("class"),
            "overhead"
        );
    }

    #[test]
    fn inventory_is_refused_for_a_charge_that_does_not_capitalise() {
        assert!(resolve_reporting_class("freight", false, Some("inventory")).is_err());
        assert!(resolve_reporting_class("freight", false, Some("capex")).is_err());
    }

    #[test]
    fn an_expensed_layer_without_a_ticket_posts_nothing() {
        assert!(expense_for_layer(
            Uuid::now_v7(),
            Uuid::now_v7(),
            None,
            LayerCharge {
                amount_minor: 100,
                currency: "USD",
                snapshot: None,
                base_amount_minor: None,
                description: "freight FRT-1",
            },
            Utc::now(),
        )
        .is_none());
    }
}
