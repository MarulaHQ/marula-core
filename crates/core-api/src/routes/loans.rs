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
            &format!("{base}/client-loans/{{id}}/return"),
            post(return_loan),
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
        let before = core_db::loans::get_loan(&mut ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?
            .0;
        let targets: Vec<core_db::loans::LoanReturnTarget> = req
            .lines
            .iter()
            .map(|l| core_db::loans::LoanReturnTarget {
                line_id: l.line_id,
                return_condition: l.return_condition.clone(),
                return_notes: l.return_notes.clone(),
            })
            .collect();
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
