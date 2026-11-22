//! Client loans + rentals API (Wave 32 lane 3, `p32-tools-loans`;
//! migration 0104).
//!
//! Issues devices to a client under an agreement. Stock-sourced devices
//! ride the lane-1 append-only ledger on both legs (`loan` out,
//! `loan_return` back) and instantiate a `DeviceAsset` in the client's
//! fleet with identity keys — the ADR 0003 dedupe path then converges a
//! later M365/RMM feed of the same phone onto that same row. Leases stamp
//! the 0061 rental fields on the asset from the loan's lease terms.
//!
//! Overdue is derived at read time (`returned_at IS NULL AND
//! expected_return_at < current_date`), never stored. Mutations ride the
//! IF1-IF3 envelope and emit `loan.*` events plus the per-leg
//! `stock.movement_recorded` events in the same commit.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::entities::{Actor, AuditEvent, ClientLoan, ClientLoanLine};
use core_domain::policy::{Action, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "client_loan";
const LIST_LIMIT: i64 = 500;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: core_domain::policy::Module::Commercial,
        action,
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/client-loans"),
            get(list_loans).post(issue_loan),
        )
        .route(&format!("{base}/client-loans/{{id}}"), get(get_loan))
        .route(
            &format!("{base}/client-loans/{{id}}/charges"),
            get(list_charges),
        )
        .route(
            &format!("{base}/client-loans/{{id}}/return"),
            post(return_loan),
        )
        .route(
            &format!("{base}/vendor-rentals"),
            get(list_vendor_rentals).post(create_vendor_rental),
        )
        .route(
            &format!("{base}/vendor-rentals/{{id}}"),
            get(get_vendor_rental),
        )
        .route(
            &format!("{base}/vendor-rentals/{{id}}/return"),
            post(return_vendor_rental),
        )
}

/// POST /api/client-loans.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct IssueLoanRequest {
    #[schema(value_type = uuid::Uuid)]
    pub client_id: Uuid,
    /// Agreement anchor where a contract row exists.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub contract_id: Option<Uuid>,
    /// Free-form contract/agreement reference when no contract row exists.
    #[serde(default)]
    pub agreement_ref: Option<String>,
    /// Defaults to now.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub out_at: Option<DateTime<Utc>>,
    #[schema(value_type = String)]
    pub expected_return_at: NaiveDate,
    /// When true the lease terms below are required and stamp the issued
    /// asset's 0061 rental fields.
    #[serde(default)]
    pub is_lease: bool,
    /// monthly | quarterly | annual.
    #[serde(default)]
    pub lease_cycle: Option<String>,
    #[serde(default)]
    pub lease_cost_minor: Option<i64>,
    #[serde(default)]
    pub lease_currency: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    pub items: Vec<LoanItemRequest>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// One device to issue: from org stock (item + optional serial) or an
/// existing fleet asset.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct LoanItemRequest {
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub device_asset_id: Option<Uuid>,
}

/// POST /api/client-loans/{id}/return.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReturnLoanRequest {
    /// Required when any returned device came from stock.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub to_location_id: Option<Uuid>,
    /// Lines to return; empty/absent = every outstanding device.
    #[serde(default)]
    pub lines: Vec<LoanReturnLineRequest>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct LoanReturnLineRequest {
    #[schema(value_type = uuid::Uuid)]
    pub line_id: Uuid,
    /// good | fair | damaged.
    #[serde(default)]
    pub return_condition: Option<String>,
    #[serde(default)]
    pub return_notes: Option<String>,
    /// Where the unit goes now: pool | sale_stock | refurbishment | ewaste.
    /// Absent = back to the class it was issued from.
    #[serde(default)]
    pub disposition: Option<String>,
    /// A damage charge raised with this return. Leases only: the charge bills
    /// through the next ordinary run under the agreement's contract.
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub damage_charge_minor: Option<i64>,
    #[serde(default)]
    pub damage_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClientLoanLineView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub serial_id: Option<Uuid>,
    /// The device's row in the client's fleet (instantiated on issue for
    /// stock-sourced devices; ADR 0003 identity keys converge later feeds).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub device_asset_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub movement_out_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub movement_in_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub returned_at: Option<DateTime<Utc>>,
    #[schema(nullable = true)]
    pub return_condition: Option<String>,
    #[schema(nullable = true)]
    pub return_notes: Option<String>,
    /// True when the device came from org stock (rode the ledger).
    pub from_stock: bool,
}

impl From<ClientLoanLine> for ClientLoanLineView {
    fn from(l: ClientLoanLine) -> Self {
        Self {
            id: l.id,
            stock_item_id: l.stock_item_id,
            serial_id: l.serial_id,
            device_asset_id: l.device_asset_id,
            movement_out_id: l.movement_out_id,
            movement_in_id: l.movement_in_id,
            returned_at: l.returned_at,
            return_condition: l.return_condition,
            return_notes: l.return_notes,
            from_stock: l.stock_item_id.is_some(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClientLoanView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub client_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub contract_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub agreement_ref: Option<String>,
    #[schema(value_type = String)]
    pub out_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub expected_return_at: NaiveDate,
    #[schema(value_type = Option<String>)]
    pub returned_at: Option<DateTime<Utc>>,
    pub status: String,
    pub is_lease: bool,
    pub lease_cycle: Option<String>,
    #[schema(value_type = Option<i64>)]
    pub lease_cost_minor: Option<i64>,
    pub lease_currency: Option<String>,
    #[schema(nullable = true)]
    pub notes: Option<String>,
    #[schema(nullable = true)]
    pub return_notes: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub issued_by: Option<Uuid>,
    /// Derived: open past the expected return date.
    pub overdue: bool,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl ClientLoanView {
    fn view(l: ClientLoan) -> Self {
        let overdue = l.is_overdue(Utc::now().date_naive());
        Self {
            id: l.id,
            org_id: l.org_id,
            client_id: l.client_id,
            contract_id: l.contract_id,
            agreement_ref: l.agreement_ref,
            out_at: l.out_at,
            expected_return_at: l.expected_return_at,
            returned_at: l.returned_at,
            status: l.status,
            is_lease: l.is_lease,
            lease_cycle: l.lease_cycle,
            lease_cost_minor: l.lease_cost_minor,
            lease_currency: l.lease_currency,
            notes: l.notes,
            return_notes: l.return_notes,
            issued_by: l.issued_by,
            overdue,
            created_at: l.created_at,
            updated_at: l.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ClientLoanMutationResponse {
    pub loan: ClientLoanView,
    pub lines: Vec<ClientLoanLineView>,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ClientLoanDetailResponse {
    pub loan: ClientLoanView,
    pub lines: Vec<ClientLoanLineView>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct LoanListQuery {
    /// open | returned (absent = both).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub client_id: Option<Uuid>,
    /// Only open loans past their expected return.
    #[serde(default)]
    pub overdue_only: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// POST /api/client-loans — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "client_loans_issue",
    post,
    path = "/api/client-loans",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = IssueLoanRequest,
    responses(
        (status = 200, body = ClientLoanMutationResponse),
        (status = 404, description = "client / contract / device not visible in scope"),
        (status = 409, description = "device not in stock / already on an open loan / incomplete lease terms")
    )
)]
pub async fn issue_loan(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<IssueLoanRequest>,
) -> ApiResult<Json<ClientLoanMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let core_req = core_db::loans::LoanIssueRequest {
        client_id: req.client_id,
        contract_id: req.contract_id,
        agreement_ref: req.agreement_ref.clone(),
        out_at: req.out_at,
        expected_return_at: req.expected_return_at,
        is_lease: req.is_lease,
        lease_cycle: req.lease_cycle.clone(),
        lease_cost_minor: req.lease_cost_minor,
        lease_currency: req.lease_currency.clone(),
        notes: req.notes.clone(),
        items: req
            .items
            .iter()
            .map(|i| core_db::loans::LoanIssueItem {
                stock_item_id: i.stock_item_id,
                serial_id: i.serial_id,
                device_asset_id: i.device_asset_id,
            })
            .collect(),
    };
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
        let result = core_db::loans::issue_loan(&mut ctx.tx, org_id, user_id, &core_req)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        for leg in &result.legs {
            crate::routes::stock::emit_movement_side_effects(
                &mut ctx,
                org_id,
                &leg.movement,
                leg.low.as_ref(),
            )
            .await?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: result.loan.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::LOAN_ISSUED,
            ENTITY,
            Some(result.loan.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::LoanIssued {
                loan: result.loan.clone(),
                lines: result.lines.clone(),
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
                action: "loan.issued".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(result.loan.id.to_string()),
                detail: serde_json::json!({
                    "client_id": result.loan.client_id,
                    "is_lease": result.loan.is_lease,
                    "devices": result.lines.len(),
                    "expected_return_at": result.loan.expected_return_at,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ClientLoanView::view(result.loan.clone());
        let lines: Vec<ClientLoanLineView> = result
            .lines
            .into_iter()
            .map(ClientLoanLineView::from)
            .collect();
        let payload = serde_json::json!({ "loan": &view, "lines": &lines });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: view.id,
                op: "issue",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ClientLoanMutationResponse {
                loan: view,
                lines,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/client-loans/{id}/return — instant-first (IF1/IF2).
#[utoipa::path(
    operation_id = "client_loans_return",
    post,
    path = "/api/client-loans/{id}/return",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReturnLoanRequest,
    responses(
        (status = 200, body = ClientLoanMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "already returned / missing return location for stock devices")
    )
)]
pub async fn return_loan(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<ReturnLoanRequest>,
) -> ApiResult<Json<ClientLoanMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // The path id is part of what this key was used for: a key replayed against
    // another agreement is a different request, not the same one.
    let fingerprint = serde_json::json!({ "id": id, "request": &req });
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
        let before = core_db::loans::get_loan(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?
            .0;
        let today = Utc::now().date_naive();
        // A damage charge bills under the lease's contract, in the contract's
        // currency. A lease written before the currency rule existed can still
        // carry another one, and the run would abort on the charge; refuse it
        // here, before any charge is written.
        let damage_contract = match (before.contract_id, before.is_lease) {
            (Some(contract_id), true) => Some(
                core_db::contracts::get_contract(&mut *ctx.tx, org_id, contract_id)
                    .await
                    .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?,
            ),
            _ => None,
        };
        let mut targets: Vec<core_db::loans::LoanReturnTarget> =
            Vec::with_capacity(req.lines.len());
        for line in &req.lines {
            // The note is stored inside the charge's description, so a note
            // with no charge would be accepted and silently lost.
            if line
                .damage_note
                .as_deref()
                .is_some_and(|note| !note.trim().is_empty())
                && line.damage_charge_minor.is_none()
            {
                return Err(ApiError::Validation(
                    "a damage note needs a damage charge: the note is recorded with the charge"
                        .into(),
                ));
            }
            let damage_charge = match line.damage_charge_minor {
                None => None,
                Some(amount) => {
                    if amount < 0 {
                        return Err(ApiError::Validation(
                            "a damage charge cannot be negative".into(),
                        ));
                    }
                    if !before.is_lease {
                        return Err(ApiError::Validation(
                            "damage charges belong to a lease agreement".into(),
                        ));
                    }
                    let Some(contract_id) = before.contract_id else {
                        return Err(ApiError::Conflict(
                            "this lease names no contract, so a damage charge has nothing to bill under"
                                .into(),
                        ));
                    };
                    let currency = before.lease_currency.clone().ok_or_else(|| {
                        ApiError::Conflict("this lease carries no currency to charge in".into())
                    })?;
                    if let Some(contract) = &damage_contract {
                        if !currency.eq_ignore_ascii_case(&contract.currency) {
                            return Err(ApiError::Conflict(format!(
                                "lease bills in {currency} but its contract {} bills in {}; \
                                 record the damage in the contract currency",
                                contract.number, contract.currency
                            )));
                        }
                    }
                    let snapshot = crate::routes::fx::lock_snapshot(
                        &mut ctx.tx, org_id, amount, &currency, today,
                    )
                    .await?;
                    Some(core_db::loans::NewRentalCharge {
                        id: Uuid::now_v7(),
                        loan_id: id,
                        contract_id,
                        client_id: before.client_id,
                        kind: "damage".into(),
                        source_line_id: Some(line.line_id),
                        period_start: today,
                        period_end: today + chrono::Duration::days(1),
                        days_billed: 1,
                        period_days: 1,
                        unit_amount_minor: amount,
                        amount_minor: amount,
                        currency,
                        description: match line.damage_note.as_deref() {
                            Some(note) => format!("Rental damage — {note} [{}]", line.line_id),
                            None => format!("Rental damage on return [{}]", line.line_id),
                        },
                        fx_rate_used: snapshot.as_ref().map(|s| s.rate_used.clone()),
                        fx_rate_date: snapshot.as_ref().map(|s| s.rate_date),
                        fx_source: snapshot.as_ref().map(|s| s.source.clone()),
                        base_amount_minor: snapshot.as_ref().map(|s| s.base_amount_minor),
                    })
                }
            };
            targets.push(core_db::loans::LoanReturnTarget {
                line_id: line.line_id,
                return_condition: line.return_condition.clone(),
                return_notes: line.return_notes.clone(),
                disposition: line.disposition.clone(),
                damage_charge,
            });
        }
        let result = core_db::loans::return_loan(
            &mut ctx.tx,
            org_id,
            user_id,
            id,
            req.to_location_id,
            &targets,
            req.notes.as_deref(),
        )
        .await
        .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        for leg in &result.legs {
            crate::routes::stock::emit_movement_side_effects(
                &mut ctx,
                org_id,
                &leg.movement,
                leg.low.as_ref(),
            )
            .await?;
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
            core_domain::events::types::LOAN_RETURNED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::LoanReturned {
                loan: result.loan.clone(),
                lines: result.lines.clone(),
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
                action: "loan.returned".into(),
                entity_type: Some(ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "devices_returned": result.legs.len(),
                    "status": result.loan.status,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = ClientLoanView::view(result.loan.clone());
        let lines: Vec<ClientLoanLineView> = result
            .lines
            .into_iter()
            .map(ClientLoanLineView::from)
            .collect();
        let payload = serde_json::json!({ "loan": &view, "lines": &lines });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: id,
                op: "return",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ClientLoanMutationResponse {
                loan: view,
                lines,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/client-loans.
#[utoipa::path(
    operation_id = "client_loans_list",
    get,
    path = "/api/client-loans",
    tag = "commercial",
    security(("bearer" = [])),
    params(LoanListQuery),
    responses((status = 200, body = [ClientLoanDetailResponse]))
)]
pub async fn list_loans(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<LoanListQuery>,
) -> ApiResult<Json<Vec<ClientLoanDetailResponse>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    if let Some(status) = q.status.as_deref() {
        if status != "open" && status != "returned" {
            return Err(ApiError::Validation(
                "status must be open or returned".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let filter = core_db::loans::LoanFilter {
        status: q.status.as_deref(),
        client_id: q.client_id,
        overdue_only: q.overdue_only == Some(true),
        limit: q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    };
    let loans = core_db::loans::list_loans(&mut tx, org_id, &filter).await?;
    let mut out = Vec::with_capacity(loans.len());
    for loan in loans {
        let lines = core_db::loans::list_lines(&mut tx, org_id, loan.id).await?;
        out.push(ClientLoanDetailResponse {
            loan: ClientLoanView::view(loan),
            lines: lines.into_iter().map(ClientLoanLineView::from).collect(),
        });
    }
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(out))
}

/// GET /api/client-loans/{id}.
#[utoipa::path(
    operation_id = "client_loans_get",
    get,
    path = "/api/client-loans/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ClientLoanDetailResponse),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_loan(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<ClientLoanDetailResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let (loan, lines) = core_db::loans::get_loan(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ClientLoanDetailResponse {
        loan: ClientLoanView::view(loan),
        lines: lines.into_iter().map(ClientLoanLineView::from).collect(),
    }))
}

// --- lease charges ------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RentalChargeView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub loan_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub contract_id: Uuid,
    /// rent | damage | loss
    pub kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub source_line_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub period_start: NaiveDate,
    #[schema(value_type = String)]
    pub period_end: NaiveDate,
    pub days_billed: i64,
    pub period_days: i64,
    #[schema(value_type = i64)]
    pub unit_amount_minor: i64,
    #[schema(value_type = i64)]
    pub amount_minor: i64,
    pub currency: String,
    pub description: String,
    /// pending | billed
    pub status: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub invoice_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub invoice_line_id: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub billed_at: Option<DateTime<Utc>>,
}

impl From<core_db::loans::RentalCharge> for RentalChargeView {
    fn from(c: core_db::loans::RentalCharge) -> Self {
        Self {
            id: c.id,
            loan_id: c.loan_id,
            contract_id: c.contract_id,
            kind: c.kind,
            source_line_id: c.source_line_id,
            period_start: c.period_start,
            period_end: c.period_end,
            days_billed: c.days_billed,
            period_days: c.period_days,
            unit_amount_minor: c.unit_amount_minor,
            amount_minor: c.amount_minor,
            currency: c.currency,
            description: c.description,
            status: c.status,
            invoice_id: c.invoice_id,
            invoice_line_id: c.invoice_line_id,
            billed_at: c.billed_at,
        }
    }
}

/// GET /api/client-loans/{id}/charges — every charge the agreement has
/// produced, billed or waiting for the next run.
#[utoipa::path(
    operation_id = "client_loans_charges",
    get,
    path = "/api/client-loans/{id}/charges",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = [RentalChargeView]),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn list_charges(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<Vec<RentalChargeView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    // The agreement must exist in scope before its charges do.
    core_db::loans::get_loan(&mut tx, org_id, id).await?;
    let charges = core_db::loans::list_rental_charges(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        charges.into_iter().map(RentalChargeView::from).collect(),
    ))
}

// --- vendor rentals (inbound) -------------------------------------------------
//
// The unit is the vendor's. It lands as a `device_assets` row with
// `ownership = rented` and class `rental_in`, so it never depreciates and never
// enters owned valuation. This surface records the agreement around it: terms,
// the expense, the deposit, and the return with its condition, damage, or loss.

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct VendorRentalAssetRequest {
    pub tag: String,
    pub name: String,
    /// device | server | ... (free text, as on every asset).
    #[serde(default = "VendorRentalAssetRequest::default_kind")]
    pub kind: String,
}

impl VendorRentalAssetRequest {
    fn default_kind() -> String {
        "device".into()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct CreateVendorRentalRequest {
    /// An asset the procurement receipt already created as `rental_in`.
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub device_asset_id: Option<Uuid>,
    /// Or a unit to create in the vendor's class.
    #[serde(default)]
    pub asset: Option<VendorRentalAssetRequest>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub vendor_id: Option<Uuid>,
    /// The vendor's name when they are not in the directory.
    #[serde(default)]
    pub vendor_name: Option<String>,
    #[serde(default)]
    pub agreement_ref: Option<String>,
    #[schema(value_type = String)]
    pub start_on: NaiveDate,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub end_on: Option<NaiveDate>,
    /// The rental expense per cycle, integer minor units.
    #[schema(value_type = i64)]
    pub cost_minor: i64,
    pub currency: String,
    /// monthly | quarterly | annual.
    pub cycle: String,
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub deposit_minor: Option<i64>,
    #[serde(default)]
    pub deposit_currency: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VendorRentalView {
    #[schema(value_type = Uuid)]
    pub id: Uuid,
    #[schema(value_type = Uuid)]
    pub device_asset_id: Uuid,
    #[schema(value_type = Option<Uuid>)]
    pub vendor_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub vendor_name: Option<String>,
    #[schema(nullable = true)]
    pub agreement_ref: Option<String>,
    #[schema(value_type = String)]
    pub start_on: NaiveDate,
    #[schema(value_type = Option<String>)]
    pub end_on: Option<NaiveDate>,
    #[schema(value_type = i64)]
    pub cost_minor: i64,
    pub currency: String,
    pub cycle: String,
    #[schema(value_type = Option<i64>)]
    pub deposit_minor: Option<i64>,
    #[schema(nullable = true)]
    pub deposit_currency: Option<String>,
    /// The cycle cost in the organisation base currency, locked when it was
    /// written. Absent when the cost is already in the base currency.
    #[schema(value_type = Option<i64>)]
    pub base_cost_minor: Option<i64>,
    /// open | returned
    pub status: String,
    #[schema(value_type = Option<String>)]
    pub returned_at: Option<DateTime<Utc>>,
    #[schema(nullable = true)]
    pub return_condition: Option<String>,
    #[schema(value_type = Option<i64>)]
    pub damage_minor: Option<i64>,
    pub lost: bool,
    #[schema(value_type = Option<i64>)]
    pub loss_minor: Option<i64>,
    #[schema(nullable = true)]
    pub notes: Option<String>,
    #[schema(value_type = String)]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub updated_at: DateTime<Utc>,
}

impl From<core_db::loans::VendorRental> for VendorRentalView {
    fn from(v: core_db::loans::VendorRental) -> Self {
        Self {
            id: v.id,
            device_asset_id: v.device_asset_id,
            vendor_id: v.vendor_id,
            vendor_name: v.vendor_name,
            agreement_ref: v.agreement_ref,
            start_on: v.start_on,
            end_on: v.end_on,
            cost_minor: v.cost_minor,
            currency: v.currency,
            cycle: v.cycle,
            deposit_minor: v.deposit_minor,
            deposit_currency: v.deposit_currency,
            base_cost_minor: v.base_cost_minor,
            status: v.status,
            returned_at: v.returned_at,
            return_condition: v.return_condition,
            damage_minor: v.damage_minor,
            lost: v.lost,
            loss_minor: v.loss_minor,
            notes: v.notes,
            created_at: v.created_at,
            updated_at: v.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct VendorRentalMutationResponse {
    pub rental: VendorRentalView,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct VendorRentalListQuery {
    /// open | returned (absent = both).
    #[serde(default)]
    pub status: Option<String>,
    /// Page size. Must be at least 1; absent uses the module limit and larger
    /// values are capped. A value below 1 is a 422.
    #[serde(default)]
    #[schema(minimum = 1)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct VendorRentalReturnRequest {
    /// good | fair | damaged — required unless the unit was lost.
    #[serde(default)]
    pub return_condition: Option<String>,
    #[serde(default)]
    pub lost: bool,
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub damage_minor: Option<i64>,
    #[serde(default)]
    #[schema(value_type = Option<i64>)]
    pub loss_minor: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

const VENDOR_ENTITY: &str = "vendor_rental";

/// POST /api/vendor-rentals — record what the organisation is renting in.
#[utoipa::path(
    operation_id = "vendor_rentals_create",
    post,
    path = "/api/vendor-rentals",
    tag = "commercial",
    security(("bearer" = [])),
    request_body = CreateVendorRentalRequest,
    responses(
        (status = 200, body = VendorRentalMutationResponse),
        (status = 404, description = "vendor or asset not visible in scope"),
        (status = 409, description = "the unit is not rental_in gear, or is already on an open rental"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn create_vendor_rental(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<CreateVendorRentalRequest>,
) -> ApiResult<Json<VendorRentalMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    // Currency and amount are validated here, before the transaction: a bad
    // code is a caller mistake, not a conflict.
    core_domain::money::CurrencyCode::new(&req.currency)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    if let Some(code) = &req.deposit_currency {
        core_domain::money::CurrencyCode::new(code)
            .map_err(|e| ApiError::Validation(e.to_string()))?;
    }
    match (req.device_asset_id, req.asset.as_ref()) {
        (None, None) => {
            return Err(ApiError::Validation(
                "name the rented asset or describe one to create".into(),
            ))
        }
        (Some(_), Some(_)) => {
            return Err(ApiError::Validation(
                "name an existing asset or describe one to create, not both".into(),
            ))
        }
        _ => {}
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: VENDOR_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // ADR 0002: the cycle cost is priced once, so its FX snapshot locks
        // here and never re-converts.
        let snapshot = crate::routes::fx::lock_snapshot(
            &mut ctx.tx,
            org_id,
            req.cost_minor,
            &req.currency,
            req.start_on,
        )
        .await?;
        let core_req = core_db::loans::VendorRentalRequest {
            device_asset_id: req.device_asset_id,
            new_asset: req.asset.as_ref().map(|a| core_db::loans::NewRentalAsset {
                tag: a.tag.clone(),
                name: a.name.clone(),
                kind: a.kind.clone(),
            }),
            vendor_id: req.vendor_id,
            vendor_name: req.vendor_name.clone(),
            agreement_ref: req.agreement_ref.clone(),
            start_on: req.start_on,
            end_on: req.end_on,
            cost_minor: req.cost_minor,
            currency: req.currency.clone(),
            cycle: req.cycle.clone(),
            deposit_minor: req.deposit_minor,
            deposit_currency: req.deposit_currency.clone(),
            fx_rate_used: snapshot.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snapshot.as_ref().map(|s| s.rate_date),
            fx_source: snapshot.as_ref().map(|s| s.source.clone()),
            base_cost_minor: snapshot.as_ref().map(|s| s.base_amount_minor),
            notes: req.notes.clone(),
        };
        let rental = core_db::loans::create_vendor_rental(&mut ctx.tx, org_id, user_id, &core_req)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: VENDOR_ENTITY,
                entity_id: rental.id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "vendor_rental.created".into(),
                entity_type: Some(VENDOR_ENTITY.into()),
                entity_id: Some(rental.id.to_string()),
                detail: serde_json::json!({
                    "device_asset_id": rental.device_asset_id,
                    "cost_minor": rental.cost_minor,
                    "currency": rental.currency,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = VendorRentalView::from(rental);
        // The replay payload is the response body without `mutation` (the
        // framework injects that), so a same-key retry deserializes back into
        // `VendorRentalMutationResponse` instead of failing on a missing
        // `rental` field.
        let payload = serde_json::json!({ "rental": &view });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: VENDOR_ENTITY,
                entity_id: view.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(VendorRentalMutationResponse {
                rental: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/vendor-rentals.
#[utoipa::path(
    operation_id = "vendor_rentals_list",
    get,
    path = "/api/vendor-rentals",
    tag = "commercial",
    security(("bearer" = [])),
    params(VendorRentalListQuery),
    responses(
        (status = 200, body = [VendorRentalView]),
        (status = 422, description = "validation failure (limit below 1)")
    )
)]
pub async fn list_vendor_rentals(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<VendorRentalListQuery>,
) -> ApiResult<Json<Vec<VendorRentalView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    // The row carries the rental expense, the deposit, damage and loss.
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    if let Some(status) = q.status.as_deref() {
        if status != "open" && status != "returned" {
            return Err(ApiError::Validation(
                "status must be open or returned".into(),
            ));
        }
    }
    if q.limit.is_some_and(|limit| limit < 1) {
        return Err(ApiError::Validation("limit must be at least 1".into()));
    }
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::loans::list_vendor_rentals(
        &mut tx,
        org_id,
        q.status.as_deref(),
        q.limit.unwrap_or(LIST_LIMIT).min(LIST_LIMIT),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows.into_iter().map(VendorRentalView::from).collect()))
}

/// GET /api/vendor-rentals/{id}.
#[utoipa::path(
    operation_id = "vendor_rentals_get",
    get,
    path = "/api/vendor-rentals/{id}",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = VendorRentalView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_vendor_rental(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> ApiResult<Json<VendorRentalView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rental = core_db::loans::get_vendor_rental(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(VendorRentalView::from(rental)))
}

/// POST /api/vendor-rentals/{id}/return — close the agreement with what
/// happened to the unit.
#[utoipa::path(
    operation_id = "vendor_rentals_return",
    post,
    path = "/api/vendor-rentals/{id}/return",
    tag = "commercial",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = VendorRentalReturnRequest,
    responses(
        (status = 200, body = VendorRentalMutationResponse),
        (status = 404, description = "not visible in scope"),
        (status = 409, description = "already closed, or an incomplete return")
    )
)]
pub async fn return_vendor_rental(
    axum::extract::State(state): axum::extract::State<AppState>,
    extensions: Extensions,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<VendorRentalReturnRequest>,
) -> ApiResult<Json<VendorRentalMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    // The response is the same row the reads return, expense and deposit
    // included, so the same financial gate applies to the write.
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // Same rule as the client-loan return: the path id is part of the request.
    let fingerprint = serde_json::json!({ "id": id, "request": &req });
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: VENDOR_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::loans::get_vendor_rental(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let rental = core_db::loans::return_vendor_rental(
            &mut ctx.tx,
            org_id,
            user_id,
            id,
            &core_db::loans::VendorRentalReturn {
                return_condition: req.return_condition.clone(),
                lost: req.lost,
                damage_minor: req.damage_minor,
                loss_minor: req.loss_minor,
                notes: req.notes.clone(),
            },
        )
        .await
        .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: VENDOR_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "vendor_rental.returned".into(),
                entity_type: Some(VENDOR_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "return_condition": rental.return_condition,
                    "lost": rental.lost,
                    "damage_minor": rental.damage_minor,
                    "loss_minor": rental.loss_minor,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let view = VendorRentalView::from(rental);
        let payload = serde_json::json!({ "rental": &view });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: VENDOR_ENTITY,
                entity_id: id,
                op: "return",
                seq: write_seq,
                before: Some(
                    serde_json::to_value(VendorRentalView::from(before)).unwrap_or(Value::Null),
                ),
                after: serde_json::to_value(&view).unwrap_or(Value::Null),
                updated_at: view.updated_at,
                key: key.as_deref(),
                payload: &payload,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(VendorRentalMutationResponse {
                rental: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}
