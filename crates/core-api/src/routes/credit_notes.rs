//! Credit notes (the private build tracker Money+docs backend gap, recorded 2026-09-10):
//! issue client credits, allocate them against invoices, refund them
//! back out, and void untouched notes — PSA-standard semantics riding
//! the instant-first write path (IF1–IF3).
//!
//! Purpose:
//! - `POST /api/credit-notes` — issue a credit for a client, optionally
//!   riding one invoice as the issuing context. Money is minor units in
//!   ONE currency (ADR 0002: never converted — the credit's currency is
//!   the named invoice's, or the request's for free-standing credits).
//! - `GET /api/credit-notes` (+ `/{id}` with its application ledger) —
//!   the allocation state is data: applied/refunded counters against
//!   the face value, `remaining` derived.
//! - `POST /api/credit-notes/{id}/apply` — move part of the remaining
//!   credit onto an ISSUED invoice of the same client + currency. The
//!   money move IS a real invoice payment receipt (provider
//!   `credit_note`) through the same `apply_payment_total` path as
//!   card/cash payments, so paid-state transitions and overpayment
//!   guards stay in one place. Co-emits `invoice.payment_recorded`.
//! - `POST /api/credit-notes/{id}/refund` — pay part of the remaining
//!   credit back to the client. The none-mode `PaymentProvider` seam
//!   has no refund method in v1, so the refund approves locally
//!   (exactly like charges); an external PSP refund rides the same
//!   endpoint when the seam grows one.
//! - `POST /api/credit-notes/{id}/void` — withdraw an untouched note
//!   (terminal; refused once anything applied or refunded).
//!
//! Non-purpose: no negative invoices, no FX conversion, no automatic
//! dunning interaction (applying a credit can close an invoice through
//! the ordinary paid transition; dunning states stay data).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
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
    CreditNote, CreditNoteApplication, CreditNoteStatus, Invoice, InvoicePayment, InvoiceStatus,
};
use core_domain::events::{
    types, CreditNoteApplied, CreditNoteIssued, CreditNoteRefunded, InvoicePaymentRecorded,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

const ENTITY: &str = "credit_note";

/// The invoice-payment provider name credit applications ride.
const CREDIT_PROVIDER: &str = "credit_note";

// --- wire structs ----------------------------------------------------------

/// POST /api/credit-notes body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct IssueCreditNoteRequest {
    pub client_id: Uuid,
    /// The invoice the credit is raised against (issuing context). When
    /// set, the credit's currency is the invoice's.
    pub invoice_id: Option<Uuid>,
    /// Credit face value, minor units (> 0).
    pub total_minor: i64,
    /// ISO 4217 code — required for free-standing credits; when
    /// `invoice_id` is set it must match the invoice's currency.
    pub currency: Option<String>,
    pub reason: Option<String>,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// The credit document plus its allocation ledger (GET /{id}).
#[derive(Debug, Serialize, ToSchema)]
pub struct CreditNoteDetail {
    #[serde(flatten)]
    pub credit_note: CreditNote,
    /// Still-available credit, minor units.
    pub remaining_minor: i64,
    pub applications: Vec<CreditNoteApplication>,
}

/// GET /api/credit-notes query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct CreditNoteListQuery {
    pub client_id: Option<Uuid>,
    /// Exact status: `issued` | `void`.
    pub status: Option<String>,
}

/// POST /api/credit-notes/{id}/apply body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApplyCreditNoteRequest {
    /// The invoice receiving the allocation (same client + currency).
    pub invoice_id: Uuid,
    /// Allocation amount, minor units; must not exceed the credit's
    /// remaining value.
    pub amount_minor: i64,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// POST /api/credit-notes/{id}/refund body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RefundCreditNoteRequest {
    /// Refund amount, minor units; must not exceed the credit's
    /// remaining value.
    pub amount_minor: i64,
    pub notes: Option<String>,
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// POST /api/credit-notes/{id}/void body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct VoidCreditNoteRequest {
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// Instant-first mutation response (IF2) for credit-note mutations.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreditNoteMutationResponse {
    #[serde(flatten)]
    pub credit_note: CreditNote,
    pub mutation: instant::MutationMeta,
}

/// Apply/refund response: the credit plus the affected invoice (and the
/// receipt for applies) so the client reconciles both sides in one shot.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreditNoteAllocationResponse {
    #[serde(flatten)]
    pub credit_note: CreditNote,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invoice: Option<Invoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<InvoicePayment>,
    pub mutation: instant::MutationMeta,
}

// --- router -------------------------------------------------------------------

/// The credit-notes slice of the `/api` surface, built per generation so
/// both mounts register identical routes through one merge line.
pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/credit-notes"), get(list).post(issue))
        .route(&format!("{base}/credit-notes/{{id}}"), get(get_one))
        .route(&format!("{base}/credit-notes/{{id}}/apply"), post(apply))
        .route(&format!("{base}/credit-notes/{{id}}/refund"), post(refund))
        .route(&format!("{base}/credit-notes/{{id}}/void"), post(void))
}

// --- handlers -----------------------------------------------------------------

/// POST /api/credit-notes — issue a client credit through the
/// instant-first write path.
#[utoipa::path(
    operation_id = "credit_notes_issue",
    post,
    path = "/api/credit-notes",
    tag = "billing",
    security(("bearer" = [])),
    request_body = IssueCreditNoteRequest,
    responses(
        (status = 200, body = CreditNoteMutationResponse),
        (status = 404, description = "client or invoice not visible in scope"),
        (status = 409, description = "idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn issue(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<IssueCreditNoteRequest>,
) -> ApiResult<Json<CreditNoteMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    if req.total_minor <= 0 {
        return Err(ApiError::Validation("total_minor must be positive".into()));
    }
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

        // Currency doctrine (ADR 0002): the credit never converts. A
        // credit riding an invoice adopts the invoice's currency; a
        // free-standing credit names its own.
        let currency = match req.invoice_id {
            Some(invoice_id) => {
                let invoice = core_db::invoices::get_invoice(&mut *ctx.tx, ctx.org_id, invoice_id)
                    .await
                    .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
                if invoice.client_id != req.client_id {
                    return Err(ApiError::Validation(
                        "invoice does not belong to the credit's client".into(),
                    ));
                }
                if let Some(named) = req.currency.as_deref() {
                    if !named.eq_ignore_ascii_case(&invoice.currency) {
                        return Err(ApiError::Validation(format!(
                            "currency {named} does not match the invoice's currency {}",
                            invoice.currency
                        )));
                    }
                }
                invoice.currency
            }
            None => req.currency.clone().ok_or_else(|| {
                ApiError::Validation(
                    "currency is required for a credit note without an invoice".into(),
                )
            })?,
        };

        let id = Uuid::now_v7();
        let number = core_db::invoices::next_number(&mut *ctx.tx, ctx.org_id, "CRN-").await?;
        let note = CreditNote {
            id,
            org_id: ctx.org_id,
            number,
            client_id: req.client_id,
            issued_against_invoice_id: req.invoice_id,
            status: CreditNoteStatus::Issued,
            currency,
            total_minor: req.total_minor,
            applied_minor: 0,
            refunded_minor: 0,
            reason: req.reason.clone(),
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        note.validate().map_err(ApiError::from)?;
        core_db::credit_notes::insert_credit_note(&mut *ctx.tx, &note).await?;
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
            types::CREDIT_NOTE_ISSUED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(CreditNoteIssued {
                credit_note_id: id,
                number: note.number.clone(),
                client_id: note.client_id,
                invoice_id: note.issued_against_invoice_id,
                currency: note.currency.clone(),
                total_minor: note.total_minor,
                reason: note.reason.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "billing.credit_note_issued".to_string(),
                entity_type: Some(ENTITY.to_string()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": note.number,
                    "client_id": note.client_id,
                    "total_minor": note.total_minor,
                    "currency": note.currency,
                }),
                ip: None,
                occurred_at: note.created_at,
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
                after: serde_json::to_value(&note).unwrap_or(Value::Null),
                updated_at: note.updated_at,
                key: key.as_deref(),
                payload: &note,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CreditNoteMutationResponse {
                credit_note: note,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// GET /api/credit-notes — newest first, client/status filters.
#[utoipa::path(
    operation_id = "credit_notes_list",
    get,
    path = "/api/credit-notes",
    tag = "billing",
    security(("bearer" = [])),
    params(CreditNoteListQuery),
    responses((status = 200, body = [CreditNote]))
)]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<CreditNoteListQuery>,
) -> ApiResult<Json<Vec<CreditNote>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let filters = core_db::credit_notes::CreditNoteFilters {
        client_id: q.client_id,
        status: q.status,
    };
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let notes = core_db::credit_notes::list_credit_notes(
        &mut *tx,
        authed.principal.user.org_id,
        &filters,
        200,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(notes))
}

/// GET /api/credit-notes/{id} — the credit plus its application ledger.
#[utoipa::path(
    operation_id = "credit_notes_get",
    get,
    path = "/api/credit-notes/{id}",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = CreditNoteDetail),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_one(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<CreditNoteDetail>> {
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
    let note =
        core_db::credit_notes::get_credit_note(&mut *tx, authed.principal.user.org_id, id).await?;
    let applications =
        core_db::credit_notes::list_applications(&mut *tx, authed.principal.user.org_id, id)
            .await?;
    tx.commit().await.map_err(ApiError::from)?;
    let remaining_minor = note.remaining_minor();
    Ok(Json(CreditNoteDetail {
        credit_note: note,
        remaining_minor,
        applications,
    }))
}

/// POST /api/credit-notes/{id}/apply — allocate part of the remaining
/// credit onto an issued invoice. The allocation inserts a real payment
/// receipt (provider `credit_note`) through the same apply path as
/// ordinary payments, then moves the credit's counter in the SAME
/// transaction (both compare-and-set: over-allocation is impossible).
#[utoipa::path(
    operation_id = "credit_notes_apply",
    post,
    path = "/api/credit-notes/{id}/apply",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ApplyCreditNoteRequest,
    responses(
        (status = 200, body = CreditNoteAllocationResponse),
        (status = 404, description = "credit note or invoice not visible in scope"),
        (status = 409, description = "not enough remaining credit, invoice not payable, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn apply(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ApplyCreditNoteRequest>,
) -> ApiResult<Json<CreditNoteAllocationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation("amount_minor must be positive".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "credit_note_allocation",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let note = core_db::credit_notes::get_credit_note(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if note.status != CreditNoteStatus::Issued {
            return Err(ApiError::Conflict(
                "credit note is void; no credit remains to apply".into(),
            ));
        }
        let invoice = core_db::invoices::get_invoice(&mut *ctx.tx, org_id, req.invoice_id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if invoice.client_id != note.client_id {
            return Err(ApiError::Validation(
                "invoice does not belong to the credit note's client".into(),
            ));
        }
        if !invoice.currency.eq_ignore_ascii_case(&note.currency) {
            return Err(ApiError::Validation(format!(
                "invoice currency {} does not match the credit note's currency {}",
                invoice.currency, note.currency
            )));
        }
        if invoice.status != InvoiceStatus::Issued {
            return Err(ApiError::Conflict(format!(
                "credit can only apply to issued invoices; this one is {}",
                invoice.status.as_str()
            )));
        }
        let outstanding = invoice.total_minor.saturating_sub(invoice.total_paid_minor);
        if req.amount_minor > outstanding {
            return Err(ApiError::Conflict(format!(
                "allocation exceeds the invoice's outstanding balance ({outstanding} minor units)"
            )));
        }
        if req.amount_minor > note.remaining_minor() {
            return Err(ApiError::Conflict(format!(
                "allocation exceeds the credit's remaining value ({} minor units)",
                note.remaining_minor()
            )));
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

        // The money move: a real receipt through the ordinary payment
        // path (paid-state transition + overpayment guards included).
        let snap = crate::routes::fx::lock_snapshot(
            &mut ctx.tx,
            org_id,
            req.amount_minor,
            &note.currency,
            chrono::Utc::now().date_naive(),
        )
        .await?;
        let payment = InvoicePayment {
            id: Uuid::now_v7(),
            org_id,
            invoice_id: req.invoice_id,
            amount_minor: req.amount_minor,
            currency: note.currency.clone(),
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
            provider: CREDIT_PROVIDER.to_string(),
            provider_ref: Some(note.number.clone()),
            notes: None,
            received_at: chrono::Utc::now(),
            created_by: Some(user_id),
        };
        core_db::invoices::insert_payment(&mut *ctx.tx, &payment).await?;
        let after_invoice = core_db::invoices::apply_payment_total(
            &mut *ctx.tx,
            org_id,
            req.invoice_id,
            req.amount_minor,
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("invoice left its payable state; retry".into()))?;

        // The credit-side counter: compare-and-set on remaining value.
        let after_note =
            core_db::credit_notes::apply_credit(&mut *ctx.tx, org_id, id, req.amount_minor)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict(
                        "credit note no longer has enough remaining value; retry".into(),
                    )
                })?;

        // The credit-side ledger row.
        let application = CreditNoteApplication {
            id: Uuid::now_v7(),
            org_id,
            credit_note_id: id,
            invoice_id: req.invoice_id,
            amount_minor: req.amount_minor,
            payment_id: Some(payment.id),
            created_by: Some(user_id),
            created_at: chrono::Utc::now(),
        };
        core_db::credit_notes::insert_application(&mut *ctx.tx, &application).await?;

        // Co-emissions: the invoice receipt (same payload shape as the
        // payment path) + the credit-side allocation.
        ctx.emit(
            types::INVOICE_PAYMENT_RECORDED,
            "invoice_payment",
            Some(payment.id.to_string().as_str()),
            &serde_json::to_value(InvoicePaymentRecorded {
                payment: payment.clone(),
                invoice_status: after_invoice.status.as_str().to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        ctx.emit(
            types::CREDIT_NOTE_APPLIED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(CreditNoteApplied {
                credit_note_id: id,
                credit_note_number: after_note.number.clone(),
                invoice_id: req.invoice_id,
                amount_minor: req.amount_minor,
                currency: after_note.currency.clone(),
                applied_minor: after_note.applied_minor,
                remaining_minor: after_note.remaining_minor(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "billing.credit_note_applied".to_string(),
                entity_type: Some(ENTITY.to_string()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after_note.number,
                    "invoice_id": req.invoice_id,
                    "amount_minor": req.amount_minor,
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
                op: "credit_apply",
                seq: write_seq,
                before: Some(serde_json::to_value(&note).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after_note).unwrap_or(Value::Null),
                updated_at: after_note.updated_at,
                key: key.as_deref(),
                payload: &AllocationPayload {
                    credit_note: &after_note,
                    invoice: &after_invoice,
                    payment: &payment,
                },
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CreditNoteAllocationResponse {
                credit_note: after_note,
                invoice: Some(after_invoice),
                payment: Some(payment),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// The stored allocation payload (metadata merges in on record) so a
/// replay deserializes into [`CreditNoteAllocationResponse`] verbatim.
#[derive(Debug, Serialize)]
struct AllocationPayload<'a> {
    credit_note: &'a CreditNote,
    invoice: &'a Invoice,
    payment: &'a InvoicePayment,
}

/// POST /api/credit-notes/{id}/refund — pay part of the remaining credit
/// back to the client. v1 approves the refund locally (none-mode
/// provider, exactly like charges); the amount may never exceed the
/// remaining credit.
#[utoipa::path(
    operation_id = "credit_notes_refund",
    post,
    path = "/api/credit-notes/{id}/refund",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = RefundCreditNoteRequest,
    responses(
        (status = 200, body = CreditNoteAllocationResponse),
        (status = 404, description = "credit note not visible in scope"),
        (status = 409, description = "not enough remaining credit, version conflict, or idempotency key reused"),
        (status = 422, description = "validation failure")
    )
)]
pub async fn refund(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<RefundCreditNoteRequest>,
) -> ApiResult<Json<CreditNoteAllocationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation("amount_minor must be positive".into()));
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "credit_note_allocation",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let note = core_db::credit_notes::get_credit_note(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if note.status != CreditNoteStatus::Issued {
            return Err(ApiError::Conflict(
                "credit note is void; no credit remains to refund".into(),
            ));
        }
        if req.amount_minor > note.remaining_minor() {
            return Err(ApiError::Conflict(format!(
                "refund exceeds the credit's remaining value ({} minor units)",
                note.remaining_minor()
            )));
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
        let after =
            core_db::credit_notes::refund_credit(&mut *ctx.tx, org_id, id, req.amount_minor)
                .await?
                .ok_or_else(|| {
                    ApiError::Conflict(
                        "credit note no longer has enough remaining value; retry".into(),
                    )
                })?;
        ctx.emit(
            types::CREDIT_NOTE_REFUNDED,
            ENTITY,
            Some(id.to_string().as_str()),
            &serde_json::to_value(CreditNoteRefunded {
                credit_note_id: id,
                credit_note_number: after.number.clone(),
                amount_minor: req.amount_minor,
                currency: after.currency.clone(),
                refunded_minor: after.refunded_minor,
                remaining_minor: after.remaining_minor(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "billing.credit_note_refunded".to_string(),
                entity_type: Some(ENTITY.to_string()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after.number,
                    "amount_minor": req.amount_minor,
                    "notes": req.notes,
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
                op: "credit_refund",
                seq: write_seq,
                before: Some(serde_json::to_value(&note).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &RefundPayload {
                    credit_note: &after,
                },
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CreditNoteAllocationResponse {
                credit_note: after,
                invoice: None,
                payment: None,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// The stored refund payload (metadata merges in on record).
#[derive(Debug, Serialize)]
struct RefundPayload<'a> {
    credit_note: &'a CreditNote,
}

/// POST /api/credit-notes/{id}/void — withdraw a note that has moved
/// nothing. Terminal.
#[utoipa::path(
    operation_id = "credit_notes_void",
    post,
    path = "/api/credit-notes/{id}/void",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = VoidCreditNoteRequest,
    responses(
        (status = 200, body = CreditNoteMutationResponse),
        (status = 404, description = "credit note not visible in scope"),
        (status = 409, description = "already void, has allocations, version conflict, or idempotency key reused")
    )
)]
pub async fn void(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<VoidCreditNoteRequest>,
) -> ApiResult<Json<CreditNoteMutationResponse>> {
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
        let note = core_db::credit_notes::get_credit_note(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(ApiError::from(e)))?;
        if note.status != CreditNoteStatus::Issued {
            return Err(ApiError::Conflict("credit note is already void".into()));
        }
        if note.applied_minor > 0 || note.refunded_minor > 0 {
            return Err(ApiError::Conflict(
                "a credit note with allocations cannot be voided; issue a reversal credit instead"
                    .into(),
            ));
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
        let after = core_db::credit_notes::void_credit_note(&mut *ctx.tx, org_id, id)
            .await?
            .ok_or_else(|| {
                ApiError::Conflict("credit note left its voidable state; retry".into())
            })?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "billing.credit_note_voided".to_string(),
                entity_type: Some(ENTITY.to_string()),
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
                op: "void",
                seq: write_seq,
                before: Some(serde_json::to_value(&note).unwrap_or(Value::Null)),
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
            MutationOutcome::Applied(CreditNoteMutationResponse {
                credit_note: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}
