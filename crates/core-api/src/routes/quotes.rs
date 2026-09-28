//! Quotes (TASKLIST Phase 3, Wave 12 lane 2; migration 0060): the quote
//! builder — create, edit, lifecycle, and the convert-to-invoice seam —
//! on the IF1–IF3 instant-first envelope.
//!
//! Lifecycle (data, never side effects): `draft` → `sent` → `accepted` |
//! `declined` | `expired`. Only a DRAFT is editable (sending locks the
//! document; the DB guards re-check in SQL). Money rolls up through the
//! core-domain helpers: line amount = half-even(qty × unit price),
//! tax = half-even(taxable subtotal × tax_rate_bps), total = subtotal +
//! tax — integer minor units in ONE currency (ADR 0002; no FX).
//!
//! **Convert-to-invoice seam.** Native invoicing v1 consumes contracts
//! (invoices.contract_id NOT NULL; "billed once, ever" indexes key on the
//! contract) and has no manual-invoice API yet, so the seam materializes
//! an ACCEPTED quote into exactly those shapes: one fixed-fee CONTRACT
//! (one scheduled fee per quote line, plus a tax fee so the invoice's
//! line-sum total matches the quote total — invoicing v1 has no separate
//! tax concept) and a DRAFT invoice written through the SAME persistence
//! the invoice run uses (`core_db::invoices::insert_invoice` +
//! `insert_lines`, kind `fixed_fee`, `run_id = NULL`). One conversion per
//! quote; the validity window is enforced at convert time. The draft
//! invoice then rides the ordinary issue/void/pay/dunning surface.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    routing::{get, post},
    Json, Router,
};
use core_domain::entities::{
    line_amount_minor, rollup, Actor, AuditEvent, BillingLineKind, Contract, ContractFixedFee,
    ContractStatus, ContractType, DunningState, Invoice, InvoiceLine, InvoiceStatus, Quote,
    QuoteLine, QuoteStatus,
};
use core_domain::events::{
    types, InvoiceCreated, QuoteAccepted, QuoteConverted, QuoteCreated, QuoteDeclined,
    QuoteExpired, QuoteSent, QuoteUpdated,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

const ENTITY: &str = "quote";
const LIST_LIMIT: i64 = 200;
const MAX_LINES: usize = 200;
const CONVERT_DUE_IN_DAYS: i64 = 30;

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/quotes"), get(list).post(create))
        .route(&format!("{base}/quotes/{{id}}"), get(get_one).patch(update))
        .route(&format!("{base}/quotes/{{id}}/send"), post(send))
        .route(&format!("{base}/quotes/{{id}}/accept"), post(accept))
        .route(&format!("{base}/quotes/{{id}}/decline"), post(decline))
        .route(&format!("{base}/quotes/{{id}}/expire"), post(expire))
        .route(
            &format!("{base}/quotes/{{id}}/convert-to-invoice"),
            post(convert),
        )
}

// --- wire structs ----------------------------------------------------------

/// One requested line. Catalog-backed lines snapshot the item's
/// description/unit/price/taxability at write time; free lines name
/// their own.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct QuoteLineInput {
    pub catalog_item_id: Option<Uuid>,
    /// Required when no catalog item backs the line.
    pub description: Option<String>,
    /// Defaults to the catalog item's unit or `each`.
    pub unit: Option<String>,
    /// Quantity in thousandths of a unit (1.5 = 1500).
    pub quantity_milli: i64,
    /// Per-unit price, minor units; required when no catalog item backs
    /// the line.
    pub unit_amount_minor: Option<i64>,
    pub taxable: Option<bool>,
}

/// POST /api/quotes body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateQuoteRequest {
    pub client_id: Uuid,
    pub currency: String,
    /// Issuance date (defaults to today).
    pub issued_on: Option<chrono::NaiveDate>,
    pub valid_until: chrono::NaiveDate,
    pub notes: Option<String>,
    pub terms: Option<String>,
    /// Flat tax rate over the TAXABLE subtotal, basis points (800 = 8%).
    #[serde(default)]
    pub tax_rate_bps: i32,
    pub lines: Vec<QuoteLineInput>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// PATCH /api/quotes/{id} body — draft-only. Absent fields keep their
/// value; `lines` present replaces the whole line list.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateQuoteRequest {
    pub valid_until: Option<chrono::NaiveDate>,
    pub notes: Option<String>,
    pub terms: Option<String>,
    pub tax_rate_bps: Option<i32>,
    pub lines: Option<Vec<QuoteLineInput>>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// A lifecycle leg body (send/accept/decline/expire): envelope params
/// only; the target status is the URL.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct QuoteLifecycleRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/quotes/{id}/convert-to-invoice body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConvertQuoteRequest {
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/quotes query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct QuoteListQuery {
    pub client_id: Option<Uuid>,
    /// Exact status: `draft|sent|accepted|declined|expired`.
    pub status: Option<String>,
}

/// The quote aggregate: document + lines.
#[derive(Debug, Serialize, ToSchema)]
pub struct QuoteDetail {
    #[serde(flatten)]
    pub quote: Quote,
    pub lines: Vec<QuoteLine>,
}

/// Instant-first mutation response (IF2) for quote mutations.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct QuoteMutationResponse {
    #[serde(flatten)]
    pub quote: Quote,
    pub mutation: instant::MutationMeta,
}

/// Convert-to-invoice response: the quote with its seam targets.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConvertQuoteResponse {
    pub quote: Quote,
    pub contract: Contract,
    pub invoice: Invoice,
    pub mutation: instant::MutationMeta,
}

// --- shared helpers ---------------------------------------------------------

/// Resolves requested line inputs into persisted [`QuoteLine`]s with
/// server-computed amounts.
async fn resolve_lines(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    quote_id: Uuid,
    currency: &str,
    inputs: &[QuoteLineInput],
) -> ApiResult<Vec<QuoteLine>> {
    if inputs.is_empty() {
        return Err(ApiError::Validation(
            "a quote needs at least one line".into(),
        ));
    }
    if inputs.len() > MAX_LINES {
        return Err(ApiError::Validation(format!(
            "a quote carries at most {MAX_LINES} lines"
        )));
    }
    let mut lines = Vec::with_capacity(inputs.len());
    for (position, input) in inputs.iter().enumerate() {
        let mut description = input.description.clone();
        let mut unit = input.unit.clone();
        let mut unit_amount_minor = input.unit_amount_minor;
        let mut taxable = input.taxable;
        let catalog_item_id = match input.catalog_item_id {
            Some(item_id) => {
                let item = core_db::catalog::get_catalog_item(&mut *conn, org_id, item_id)
                    .await
                    .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
                if !item.currency.eq_ignore_ascii_case(currency) {
                    return Err(ApiError::Validation(format!(
                        "catalog item {} is priced in {}, not the quote's currency {currency}",
                        item.id, item.currency
                    )));
                }
                description = description.or_else(|| Some(item.name.clone()));
                unit = unit.or_else(|| Some(item.unit.clone()));
                unit_amount_minor = unit_amount_minor.or(Some(item.unit_price_minor));
                taxable = taxable.or(Some(item.taxable));
                Some(item.id)
            }
            None => None,
        };
        let description = description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                ApiError::Validation(
                    "line description is required when no catalog item backs the line".into(),
                )
            })?;
        let unit_amount_minor = unit_amount_minor.ok_or_else(|| {
            ApiError::Validation(
                "line unit_amount_minor is required when no catalog item backs the line".into(),
            )
        })?;
        if unit_amount_minor < 0 {
            return Err(ApiError::Validation(
                "line unit amount must not be negative".into(),
            ));
        }
        if input.quantity_milli <= 0 {
            return Err(ApiError::Validation(
                "line quantity must be positive".into(),
            ));
        }
        let amount_minor =
            line_amount_minor(input.quantity_milli, unit_amount_minor).map_err(ApiError::from)?;
        lines.push(QuoteLine {
            id: Uuid::now_v7(),
            org_id,
            quote_id,
            catalog_item_id,
            description,
            unit: taxable_unit(unit),
            quantity_milli: input.quantity_milli,
            taxable: taxable.unwrap_or(true),
            unit_amount_minor,
            amount_minor,
            position: position as i32,
        });
    }
    Ok(lines)
}

fn taxable_unit(unit: Option<String>) -> String {
    match unit {
        Some(u) if !u.trim().is_empty() => u,
        _ => "each".into(),
    }
}

async fn load_detail(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> ApiResult<QuoteDetail> {
    let quote = core_db::quotes::get_quote(&mut *conn, org_id, id)
        .await
        .map_err(ApiError::from)?;
    let lines = core_db::quotes::list_lines(&mut *conn, org_id, id).await?;
    Ok(QuoteDetail { quote, lines })
}

fn map_unique_conflict(e: core_db::DbError) -> ApiError {
    match e {
        core_db::DbError::Sql(ref sql_err) if core_db::catalog::is_unique_conflict(sql_err) => {
            ApiError::Conflict("quote number collision; retry".into())
        }
        other => other.into(),
    }
}

// --- reads ------------------------------------------------------------------

/// GET /api/quotes — newest first, client/status filters.
#[utoipa::path(
    operation_id = "quotes_list",
    get,
    path = "/api/quotes",
    tag = "commercial",
    security(("bearer" = [])),
    params(QuoteListQuery),
    responses((status = 200, body = [Quote]))
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<QuoteListQuery>,
) -> ApiResult<Json<Vec<Quote>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let filters = core_db::quotes::QuoteFilters {
        client_id: q.client_id,
        status: q.status,
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let quotes =
        core_db::quotes::list_quotes(&mut *tx, authed.principal.user.org_id, &filters, LIST_LIMIT)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(quotes))
}

/// GET /api/quotes/{id} — the quote plus its lines.
#[utoipa::path(
    operation_id = "quotes_get",
    get,
    path = "/api/quotes/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = QuoteDetail),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<QuoteDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let detail = load_detail(&mut tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(detail))
}

// --- create / edit ----------------------------------------------------------

/// POST /api/quotes — create a numbered draft. Lines snapshot catalog
/// context; roll-ups are computed server-side (never trusted from the
/// client).
#[utoipa::path(
    operation_id = "quotes_create",
    post,
    path = "/api/quotes",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateQuoteRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "client or catalog item not visible in scope"),
        (status = 409, description = "idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateQuoteRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

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
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Client must exist in scope.
        core_db::clients::get_client(&mut *ctx.tx, ctx.org_id, req.client_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;

        let id = Uuid::now_v7();
        let lines = resolve_lines(&mut ctx.tx, ctx.org_id, id, &req.currency, &req.lines).await?;
        let roll = rollup(&lines, req.tax_rate_bps).map_err(ApiError::from)?;
        let number = core_db::quotes::next_quote_number(&mut *ctx.tx, ctx.org_id).await?;
        let quote = Quote {
            id,
            org_id: ctx.org_id,
            number,
            client_id: req.client_id,
            status: QuoteStatus::Draft,
            currency: req.currency.clone(),
            issued_on: req
                .issued_on
                .unwrap_or_else(|| chrono::Utc::now().date_naive()),
            valid_until: req.valid_until,
            notes: req.notes.clone(),
            terms: req.terms.clone(),
            subtotal_minor: roll.subtotal_minor,
            tax_rate_bps: req.tax_rate_bps,
            tax_minor: roll.tax_minor,
            total_minor: roll.total_minor,
            sent_at: None,
            decided_at: None,
            converted_contract_id: None,
            converted_invoice_id: None,
            converted_at: None,
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        quote.validate().map_err(ApiError::from)?;
        core_db::quotes::insert_quote(&mut *ctx.tx, &quote)
            .await
            .map_err(map_unique_conflict)?;
        core_db::quotes::insert_lines(&mut *ctx.tx, &lines).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::QUOTE_CREATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(QuoteCreated {
                quote: quote.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: Actor::Human(user_id),
                action: "quote.created".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": quote.number,
                    "client_id": quote.client_id,
                    "currency": quote.currency,
                    "total_minor": quote.total_minor,
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
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&quote).unwrap_or(Value::Null),
                updated_at: quote.updated_at,
                key: key.as_deref(),
                payload: &quote,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(QuoteMutationResponse { quote, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/quotes/{id} — edit a DRAFT (header + optionally the whole
/// line list). Sent+ quotes are immutable: typed 409.
#[utoipa::path(
    operation_id = "quotes_update",
    patch,
    path = "/api/quotes/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateQuoteRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "quote or catalog item not visible in scope"),
        (status = 409, description = "quote is not a draft, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn update(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateQuoteRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

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
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::quotes::get_quote(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if !before.status.is_editable() {
            return Err(ApiError::Conflict(
                "only draft quotes are editable; sent quotes are immutable".into(),
            ));
        }
        let mut quote = before.clone();
        if let Some(valid_until) = req.valid_until {
            quote.valid_until = valid_until;
        }
        if let Some(notes) = &req.notes {
            quote.notes = Some(notes.clone());
        }
        if let Some(terms) = &req.terms {
            quote.terms = Some(terms.clone());
        }
        if let Some(tax_rate_bps) = req.tax_rate_bps {
            quote.tax_rate_bps = tax_rate_bps;
        }
        let lines = match &req.lines {
            Some(inputs) => resolve_lines(&mut ctx.tx, org_id, id, &quote.currency, inputs).await?,
            None => core_db::quotes::list_lines(&mut *ctx.tx, org_id, id).await?,
        };
        let roll = rollup(&lines, quote.tax_rate_bps).map_err(ApiError::from)?;
        quote.subtotal_minor = roll.subtotal_minor;
        quote.tax_minor = roll.tax_minor;
        quote.total_minor = roll.total_minor;
        quote.updated_at = chrono::Utc::now();
        quote.validate().map_err(ApiError::from)?;
        let after = core_db::quotes::update_draft_quote(&mut *ctx.tx, &quote)
            .await
            .map_err(map_unique_conflict)?
            .ok_or_else(|| ApiError::Conflict("quote is no longer a draft; retry".into()))?;
        if req.lines.is_some() {
            core_db::quotes::replace_lines(&mut ctx.tx, org_id, id, &lines).await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::QUOTE_UPDATED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(QuoteUpdated {
                quote: after.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "quote.updated".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after.number,
                    "total_minor": after.total_minor,
                    "tax_rate_bps": after.tax_rate_bps,
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
                entity_id: id,
                op: "update",
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
            MutationOutcome::Applied(QuoteMutationResponse {
                quote: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- lifecycle legs ---------------------------------------------------------

/// One lifecycle leg: shared shape for send/accept/decline (expire has a
/// POST /api/quotes/{id}/send — draft → sent. Locks the document.
#[utoipa::path(
    operation_id = "quotes_send",
    post,
    path = "/api/quotes/{id}/send",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = QuoteLifecycleRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "quote not visible in scope"),
        (status = 409, description = "illegal transition, version conflict, or idempotency key reused")
    )
)]
pub async fn send(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<QuoteLifecycleRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    lifecycle(
        state,
        extensions,
        id,
        req,
        "send",
        QuoteStatus::Draft,
        QuoteStatus::Sent,
        true,
        false,
    )
    .await
}

/// POST /api/quotes/{id}/accept — sent → accepted (terminal).
#[utoipa::path(
    operation_id = "quotes_accept",
    post,
    path = "/api/quotes/{id}/accept",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = QuoteLifecycleRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "quote not visible in scope"),
        (status = 409, description = "illegal transition, version conflict, or idempotency key reused")
    )
)]
pub async fn accept(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<QuoteLifecycleRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    lifecycle(
        state,
        extensions,
        id,
        req,
        "accept",
        QuoteStatus::Sent,
        QuoteStatus::Accepted,
        false,
        true,
    )
    .await
}

/// POST /api/quotes/{id}/decline — sent → declined. Terminal.
#[utoipa::path(
    operation_id = "quotes_decline",
    post,
    path = "/api/quotes/{id}/decline",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = QuoteLifecycleRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "quote not visible in scope"),
        (status = 409, description = "illegal transition, version conflict, or idempotency key reused")
    )
)]
pub async fn decline(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<QuoteLifecycleRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    lifecycle(
        state,
        extensions,
        id,
        req,
        "decline",
        QuoteStatus::Sent,
        QuoteStatus::Declined,
        false,
        true,
    )
    .await
}

/// The shared lifecycle engine: claim → load → transition check → guarded
/// SQL move → event → audit → instant record.
#[allow(clippy::too_many_arguments)]
async fn lifecycle(
    state: AppState,
    extensions: Extensions,
    id: Uuid,
    req: QuoteLifecycleRequest,
    op: &'static str,
    from: QuoteStatus,
    to: QuoteStatus,
    stamp_sent: bool,
    stamp_decided: bool,
) -> ApiResult<Json<QuoteMutationResponse>> {
    let action_label = format!("quote.{op}");
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

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
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::quotes::get_quote(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        QuoteStatus::transition(from, to).map_err(ApiError::from)?;
        let after = core_db::quotes::transition_status(
            &mut *ctx.tx,
            org_id,
            id,
            from.as_str(),
            to.as_str(),
            stamp_sent,
            stamp_decided,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!("quote left its {} state; retry", from.as_str()))
        })?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        emit_lifecycle_event(&mut ctx, to, &after).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: action_label,
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after.number,
                    "status": after.status.as_str(),
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
                entity_id: id,
                op: to.as_str(),
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
            MutationOutcome::Applied(QuoteMutationResponse {
                quote: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// Emits the typed lifecycle event for the reached state.
pub(crate) async fn emit_lifecycle_event<'a>(
    ctx: &mut crate::org_tx::OrgTx<'a>,
    to: QuoteStatus,
    after: &Quote,
) -> ApiResult<()> {
    let id = after.id;
    let (event_type, payload) = match to {
        QuoteStatus::Sent => (
            types::QUOTE_SENT,
            serde_json::to_value(QuoteSent {
                quote_id: id,
                number: after.number.clone(),
                client_id: after.client_id,
                currency: after.currency.clone(),
                total_minor: after.total_minor,
                valid_until: after.valid_until,
                sent_at: after.sent_at.unwrap_or_else(chrono::Utc::now),
            }),
        ),
        QuoteStatus::Accepted => (
            types::QUOTE_ACCEPTED,
            serde_json::to_value(QuoteAccepted {
                quote_id: id,
                number: after.number.clone(),
                client_id: after.client_id,
                currency: after.currency.clone(),
                total_minor: after.total_minor,
                decided_at: after.decided_at.unwrap_or_else(chrono::Utc::now),
            }),
        ),
        QuoteStatus::Declined => (
            types::QUOTE_DECLINED,
            serde_json::to_value(QuoteDeclined {
                quote_id: id,
                number: after.number.clone(),
                client_id: after.client_id,
                decided_at: after.decided_at.unwrap_or_else(chrono::Utc::now),
            }),
        ),
        _ => unreachable!("only sent/accepted/declined legs run through here"),
    };
    let payload = payload.unwrap_or_else(|_| serde_json::json!({}));
    ctx.emit(event_type, ENTITY, Some(id.to_string().as_str()), &payload)
        .await
}

/// POST /api/quotes/{id}/expire — explicitly expire a quote PAST its
/// validity window (decline is the in-window exit).
#[utoipa::path(
    operation_id = "quotes_expire",
    post,
    path = "/api/quotes/{id}/expire",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = QuoteLifecycleRequest,
    responses(
        (status = 200, body = QuoteMutationResponse),
        (status = 404, description = "quote not visible in scope"),
        (status = 409, description = "still inside validity window, illegal transition, version conflict, or idempotency key reused")
    )
)]
pub async fn expire(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<QuoteLifecycleRequest>,
) -> ApiResult<Json<QuoteMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let today = chrono::Utc::now().date_naive();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

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
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::quotes::get_quote(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        QuoteStatus::transition(before.status, QuoteStatus::Expired).map_err(ApiError::from)?;
        if today <= before.valid_until {
            return Err(ApiError::Conflict(
                "the quote is still inside its validity window; decline it instead".into(),
            ));
        }
        let after = core_db::quotes::transition_status(
            &mut *ctx.tx,
            org_id,
            id,
            before.status.as_str(),
            QuoteStatus::Expired.as_str(),
            false,
            true,
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("quote left its expirable state; retry".into()))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            types::QUOTE_EXPIRED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(QuoteExpired {
                quote_id: id,
                number: after.number.clone(),
                client_id: after.client_id,
                decided_at: after.decided_at.unwrap_or_else(chrono::Utc::now),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "quote.expired".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "number": after.number }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "expire",
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
            MutationOutcome::Applied(QuoteMutationResponse {
                quote: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- convert-to-invoice seam ------------------------------------------------

/// POST /api/quotes/{id}/convert-to-invoice — materialize an ACCEPTED
/// quote into a fixed-fee contract + DRAFT invoice through the existing
/// invoicing persistence (the seam's rationale is in the module docs).
#[utoipa::path(
    operation_id = "quotes_convert_to_invoice",
    post,
    path = "/api/quotes/{id}/convert-to-invoice",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ConvertQuoteRequest,
    responses(
        (status = 200, body = ConvertQuoteResponse),
        (status = 404, description = "quote not visible in scope"),
        (status = 409, description = "quote not accepted / outside validity / already converted, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn convert(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ConvertQuoteRequest>,
) -> ApiResult<Json<ConvertQuoteResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "quote_conversion",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let quote = core_db::quotes::get_quote(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let today = chrono::Utc::now().date_naive();
        if !quote.convertible_on(today) {
            return Err(ApiError::Conflict(format!(
                "quote is not convertible: status must be accepted, the validity window \
                 (until {}) must hold, and it must not be converted already",
                quote.valid_until
            )));
        }
        let lines = core_db::quotes::list_lines(&mut *ctx.tx, org_id, id).await?;
        if lines.is_empty() {
            return Err(ApiError::Validation(
                "a quote without lines cannot convert".into(),
            ));
        }

        // 1. The contract: the shape native invoicing consumes.
        let contract_id = Uuid::now_v7();
        let contract_number = core_db::invoices::next_number(&mut *ctx.tx, org_id, "C-").await?;
        let contract = Contract {
            id: contract_id,
            org_id,
            client_id: quote.client_id,
            number: contract_number,
            name: format!("Quote {}", quote.number),
            contract_type: ContractType::FixedFee,
            status: ContractStatus::Active,
            currency: quote.currency.clone(),
            effective_from: today,
            effective_until: None,
            billing_frequency: None,
            billing_anchor: None,
            amount_minor: None,
            hourly_rate_minor: None,
            included_minutes: None,
            overage_rate_minor: None,
            unit_price_minor: None,
            unit_basis: None,
            incident_price_minor: None,
            incident_priority: None,
            po_number: None,
            notes: Some(format!("Materialized from quote {}", quote.number)),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        contract.validate().map_err(ApiError::from)?;
        core_db::contracts::create_contract(&mut *ctx.tx, &contract).await?;

        // 2. The fee schedule: one fee per quote line (+ tax as a fee so
        //    the invoice's line-sum total matches the quote total).
        let mut labels_used: HashSet<String> = HashSet::new();
        let mut fees: Vec<ContractFixedFee> = Vec::with_capacity(lines.len() + 1);
        for line in &lines {
            let label = unique_label(&mut labels_used, &line.description);
            fees.push(ContractFixedFee {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                label,
                amount_minor: line.amount_minor,
                due_on: today,
                notes: Some(format!(
                    "quantity {} of {} (from quote {})",
                    line.quantity_milli, line.unit, quote.number
                )),
                created_at: chrono::Utc::now(),
            });
        }
        if quote.tax_minor > 0 {
            let label = unique_label(&mut labels_used, "Tax");
            fees.push(ContractFixedFee {
                id: Uuid::now_v7(),
                org_id,
                contract_id,
                label,
                amount_minor: quote.tax_minor,
                due_on: today,
                notes: Some(format!("from quote {}", quote.number)),
                created_at: chrono::Utc::now(),
            });
        }
        core_db::contracts::replace_fees(&mut ctx.tx, org_id, contract_id, &fees).await?;

        // 3. The draft invoice: the SAME persistence the invoice run uses.
        let invoice_id = Uuid::now_v7();
        let invoice_number = core_db::invoices::next_number(&mut *ctx.tx, org_id, "INV-").await?;
        let due_on = today + chrono::Duration::days(CONVERT_DUE_IN_DAYS);
        let invoice = Invoice {
            id: invoice_id,
            org_id,
            run_id: None,
            client_id: quote.client_id,
            contract_id,
            number: invoice_number,
            status: InvoiceStatus::Draft,
            currency: quote.currency.clone(),
            period_from: None,
            period_to: None,
            issued_at: today,
            due_on,
            total_minor: quote.total_minor,
            total_paid_minor: 0,
            // Draft document: no snapshot yet. The issue leg locks it
            // (ADR 0002); there is no issue endpoint yet (manual-invoice
            // API follow-up), so cross-currency quotes convert with the
            // snapshot pending until that leg exists.
            fx_rate_used: None,
            fx_rate_date: None,
            fx_source: None,
            base_total_minor: None,
            dunning_state: DunningState::None,
            dunning_updated_at: None,
            dunning_note: None,
            po_number: None,
            notes: Some(format!("From quote {}", quote.number)),
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        invoice.validate().map_err(ApiError::from)?;
        core_db::invoices::insert_invoice(&mut *ctx.tx, &invoice).await?;
        let invoice_lines: Vec<InvoiceLine> = fees
            .iter()
            .enumerate()
            .map(|(position, fee)| InvoiceLine {
                id: Uuid::now_v7(),
                org_id,
                invoice_id,
                contract_id,
                kind: BillingLineKind::FixedFee,
                description: fee.label.clone(),
                period_start: today,
                period_end: today,
                quantity_minutes: 0,
                unit_amount_minor: fee.amount_minor,
                amount_minor: fee.amount_minor,
                fx_rate_used: None,
                fx_rate_date: None,
                fx_source: None,
                base_amount_minor: None,
                position: position as i32,
            })
            .collect();
        core_db::invoices::insert_lines(&mut *ctx.tx, org_id, invoice_id, &invoice_lines).await?;

        // 4. Record the seam on the quote (guarded: accepted + unconverted).
        let after_quote =
            core_db::quotes::mark_converted(&mut *ctx.tx, org_id, id, contract_id, invoice_id)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict("quote left its convertible state; retry".into())
                })?;

        // 5. Events + audit.
        ctx.emit(
            types::INVOICE_CREATED,
            "invoice",
            Some(invoice_id.to_string().as_str()),
            &serde_json::to_value(InvoiceCreated {
                invoice: invoice.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        ctx.emit(
            types::QUOTE_CONVERTED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(QuoteConverted {
                quote_id: id,
                quote_number: after_quote.number.clone(),
                client_id: after_quote.client_id,
                contract_id,
                invoice_id,
                currency: after_quote.currency.clone(),
                total_minor: after_quote.total_minor,
                converted_at: after_quote.converted_at.unwrap_or_else(chrono::Utc::now),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "quote.converted".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after_quote.number,
                    "contract_id": contract_id,
                    "invoice_id": invoice_id,
                    "total_minor": after_quote.total_minor,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "convert_to_invoice",
                seq: write_seq,
                before: Some(serde_json::to_value(&quote).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after_quote).unwrap_or(Value::Null),
                updated_at: after_quote.updated_at,
                key: key.as_deref(),
                payload: &ConvertStored {
                    quote: &after_quote,
                    contract: &contract,
                    invoice: &invoice,
                },
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ConvertQuoteResponse {
                quote: after_quote,
                contract,
                invoice,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// The stored convert payload (metadata merges in on record) so a replay
/// deserializes into [`ConvertQuoteResponse`] verbatim.
#[derive(Debug, Serialize)]
struct ConvertStored<'a> {
    quote: &'a Quote,
    contract: &'a Contract,
    invoice: &'a Invoice,
}

/// Deduplicates labels for invoicing's fixed-fee uniqueness
/// (contract, due date, description): a second occurrence of the same
/// description gets a positional suffix.
fn unique_label(used: &mut HashSet<String>, wanted: &str) -> String {
    let mut label = wanted.to_string();
    let mut n = 2;
    while !used.insert(label.clone()) {
        label = format!("{label} ({n})");
        n += 1;
    }
    label
}
