//! Client-portal v2 HTTP surface (the private build tracker Phase 3 "Portal v2"):
//! invoice view + pay, KB self-service, and appointment requests.
//!
//! Purpose: extends the portal v1 surface (`routes::portal`) with the
//! three self-service blocks. Every route sits behind the SAME
//! `require_portal_auth` middleware (portal token family + fresh per-
//! request user load) and every read/write is scoped to the caller's
//! client_id IN the query (`core_db::invoices` client-scoped fns,
//! `core_db::kb` client-gated reads, `core_db::portal` request reads) on
//! top of org RLS — the v1 doctrine, unchanged.
//!
//! * Invoices — portal clients see exactly `issued`/`paid` documents of
//!   their own client (void/draft stay staff-internal); payment goes
//!   through the SAME `PaymentProvider` seam as the staff endpoint. With
//!   the built-in none-mode provider a payment is LEDGER-ONLY: no card
//!   processor exists, the receipt records that the client settled the
//!   amount. UI copy must present it that way.
//! * KB — only the effective client-facing gate (published AND article
//!   flag AND category flag, decided in the query layer) is readable.
//! * Appointments — a request is an INTENT against published
//!   availability; staff approve/decline elsewhere
//!   (`routes::appointment_requests`). Nothing here books a calendar.
//!
//! Instant-first contract (IF1–IF3): every NEW mutation (payment, request
//! create/cancel) is idempotency-keyed with read-your-write `mutation`
//! metadata and lands its write-trail row under the caller's portal
//! actor kind (via the IF9 `ProvenanceContext` seam); reads keep the
//! plain typed envelope.

use crate::auth::PortalAuthed;
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::invoices::render_invoice_html;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Utc};
use core_billing::ChargeRequest;
use core_domain::entities::{
    AppointmentKind, AppointmentRequest, AppointmentRequestStatus, Invoice, InvoicePayment,
    InvoiceStatus, PortalRole,
};
use core_domain::events::types;
use core_domain::policy::{check_portal, PortalAction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Requested spans cap at one working day; longer engagements are split
/// by the client into several requests.
const MAX_REQUEST_DURATION: chrono::Duration = chrono::Duration::hours(8);
/// Requests may look at most one quarter ahead.
const MAX_REQUEST_LEAD: chrono::Duration = chrono::Duration::days(90);
/// Slots shorter than this are not worth showing a client.
const MIN_SLOT_MINUTES: i64 = 30;
/// Portal slot-range cap: one month per query.
const MAX_SLOT_RANGE_DAYS: i64 = 31;

fn portal_authed(extensions: &Extensions) -> ApiResult<PortalAuthed> {
    extensions
        .get::<PortalAuthed>()
        .cloned()
        .ok_or_else(|| ApiError::Unauthorized("not authenticated".into()))
}

fn portal_actor(role: PortalRole, id: Uuid) -> core_domain::entities::Actor {
    match role {
        PortalRole::Contact => core_domain::entities::Actor::PortalContact(id),
        PortalRole::Approver => core_domain::entities::Actor::PortalApprover(id),
        PortalRole::ItManager => core_domain::entities::Actor::PortalItManager(id),
    }
}

// --- invoices -----------------------------------------------------------------

/// The client-facing invoice document view. Deliberately NOT the staff
/// entity: dunning internals and staff notes never reach the portal.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortalInvoice {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub number: String,
    /// `issued | paid` (void/draft documents are staff-internal).
    pub status: String,
    pub currency: String,
    #[schema(value_type = Option<String>, format = "date")]
    pub period_from: Option<chrono::NaiveDate>,
    #[schema(value_type = Option<String>, format = "date")]
    pub period_to: Option<chrono::NaiveDate>,
    #[schema(value_type = String, format = "date")]
    pub issued_at: chrono::NaiveDate,
    #[schema(value_type = String, format = "date")]
    pub due_on: chrono::NaiveDate,
    pub total_minor: i64,
    pub total_paid_minor: i64,
    /// `total_minor - total_paid_minor`, never negative.
    pub outstanding_minor: i64,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
}

impl From<Invoice> for PortalInvoice {
    fn from(i: Invoice) -> Self {
        Self {
            id: i.id,
            number: i.number,
            status: i.status.as_str().to_string(),
            currency: i.currency,
            period_from: i.period_from,
            period_to: i.period_to,
            issued_at: i.issued_at,
            due_on: i.due_on,
            total_minor: i.total_minor,
            total_paid_minor: i.total_paid_minor,
            outstanding_minor: (i.total_minor - i.total_paid_minor).max(0),
            created_at: i.created_at,
        }
    }
}

/// GET /api/portal/invoices — the caller's client's invoices (issued and
/// paid only), newest issued first.
#[utoipa::path(operation_id = "portal_list_invoices", get, path = "/api/portal/invoices", tag = "portal",
    security(("bearer" = [])),
    responses((status = 200, body = [PortalInvoice])))]
pub async fn list_invoices(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PortalInvoice>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewInvoices)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let invoices = core_db::invoices::list_client_invoices(
        &mut *tx,
        authed.user.org_id,
        authed.user.client_id,
        200,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(invoices.into_iter().map(Into::into).collect()))
}

/// A payment receipt as the portal sees it. `created_by` stays internal
/// (portal identities are not staff rows; the field is always NULL on
/// portal-recorded rows anyway).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PortalPaymentView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    /// `'none'` while no external PSP is connected — the receipt is a
    /// ledger entry, not a card charge.
    pub provider: String,
    #[schema(value_type = String, format = "date-time")]
    pub received_at: DateTime<Utc>,
}

impl From<&InvoicePayment> for PortalPaymentView {
    fn from(p: &InvoicePayment) -> Self {
        Self {
            id: p.id,
            amount_minor: p.amount_minor,
            currency: p.currency.clone(),
            provider: p.provider.clone(),
            received_at: p.received_at,
        }
    }
}

/// The client-facing invoice aggregate.
#[derive(Debug, Serialize, ToSchema)]
pub struct PortalInvoiceDetail {
    pub invoice: PortalInvoice,
    pub lines: Vec<core_domain::entities::InvoiceLine>,
    pub payments: Vec<PortalPaymentView>,
}

/// GET /api/portal/invoices/{id} — one of the caller's client's invoices
/// with lines and payment receipts. Another client's document (or a
/// voided one) is a 404, indistinguishable from absent.
#[utoipa::path(operation_id = "portal_get_invoice", get, path = "/api/portal/invoices/{id}", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = PortalInvoiceDetail), (status = 404)))]
pub async fn get_invoice(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PortalInvoiceDetail>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewInvoices)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let invoice = core_db::invoices::get_client_invoice(&mut *tx, org_id, client_id, id).await?;
    let lines = core_db::invoices::list_lines(&mut *tx, org_id, id).await?;
    let payments = core_db::invoices::list_payments(&mut *tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PortalInvoiceDetail {
        invoice: invoice.into(),
        lines,
        payments: payments.iter().map(Into::into).collect(),
    }))
}

/// GET /api/portal/invoices/{id}/document — the SAME print-ready HTML
/// document staff print (shared renderer), client-scoped: another
/// client's (or a void) invoice is a plain 404.
#[utoipa::path(operation_id = "portal_invoice_document", get, path = "/api/portal/invoices/{id}/document", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, content_type = "text/html", body = String), (status = 404)))]
pub async fn invoice_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::response::Html<String>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewInvoices)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let invoice = core_db::invoices::get_client_invoice(&mut *tx, org_id, client_id, id).await?;
    let lines = core_db::invoices::list_lines(&mut *tx, org_id, id).await?;
    let client = core_db::clients::get_client(&mut *tx, org_id, client_id).await?;
    let html = render_invoice_html(&mut tx, org_id, &invoice, &lines, &client.name).await;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(axum::response::Html(html))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalPayRequest {
    /// Amount to settle, minor units; must not exceed the outstanding
    /// balance. Currency is always the invoice's own.
    pub amount_minor: i64,
    pub notes: Option<String>,
    /// PSP payment-method surface (e.g. a confirmed Stripe `pm_*` id
    /// from the embedded payment form). None = ledger-only none-mode
    /// settlement; required when the org resolved provider is Stripe.
    pub payment_method: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key + If-Match
    /// `version` on the invoice.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2) for a portal payment: the
/// receipt, the refreshed client-facing invoice, and the additive
/// `mutation` metadata. Stored/replayed byte-identically under the key.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortalPayResponse {
    pub payment: InvoicePayment,
    pub invoice: PortalInvoice,
    pub mutation: instant::MutationMeta,
}

/// The stored payment payload (mutation metadata merges in on record).
#[derive(Debug, Serialize, Deserialize)]
struct PortalPayStoredPayload {
    payment: InvoicePayment,
    invoice: PortalInvoice,
}

/// POST /api/portal/invoices/{id}/pay — record a payment through the
/// `PaymentProvider` seam from the portal. v1 resolves the built-in
/// none-mode provider: NOTHING is charged externally; the receipt
/// ("ledger-only") records that the client settled the amount — the UI
/// presents it exactly that way. Provenance: the payment row keeps
/// `created_by = NULL` (portal identities are not staff rows), the audit
/// row and the write trail carry the caller's portal actor kind.
#[utoipa::path(operation_id = "portal_pay_invoice", post, path = "/api/portal/invoices/{id}/pay", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PortalPayRequest,
    responses((status = 200, body = PortalPayResponse),
              (status = 404, description = "invoice not visible to this client"),
              (status = 409, description = "invoice not payable, overpayment, version conflict, or idempotency key reused"),
              (status = 422, description = "validation failure")))]
pub async fn pay_invoice(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<PortalPayRequest>,
) -> ApiResult<Json<PortalPayResponse>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::PayInvoice)
        .expect()
        .map_err(ApiError::Forbidden)?;
    if req.amount_minor <= 0 {
        return Err(ApiError::Validation("amount_minor must be positive".into()));
    }
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let actor_kind = authed.user.role.actor_kind();

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let provider =
        crate::routes::billing_provider::resolve_payment_provider(&state, org_id).await?;
    let outcome = org_tx::with_org_tx(&state, org_id, authed.user.id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "invoice_payment",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Client-scoped read INSIDE the mutation: a foreign invoice is an
        // rls_sparse, never an existence hint.
        let invoice = core_db::invoices::get_client_invoice(&mut *ctx.tx, org_id, client_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        match invoice.status {
            InvoiceStatus::Issued => {}
            InvoiceStatus::Paid => {
                return Err(ApiError::Conflict("invoice is already fully paid".into()))
            }
            // Draft/void are invisible to the portal, so reaching here
            // means the document flipped under us.
            other => {
                return Err(ApiError::Conflict(format!(
                    "invoice is not payable in state {}",
                    other.as_str()
                )))
            }
        }
        let outstanding = invoice.total_minor.saturating_sub(invoice.total_paid_minor);
        if req.amount_minor > outstanding {
            return Err(ApiError::Conflict(format!(
                "payment exceeds the outstanding balance ({outstanding} minor units)"
            )));
        }

        // The seam hands the charge to the ORG'S RESOLVED PROVIDER
        // (Wave 22: none-mode by default; Stripe once the org opts in
        // via billing.payment_provider AND the instance key exists —
        // resolve fails closed to none-mode). Declines are TYPED on the
        // wire (payment_declined + code extra), never PSP prose.
        let receipt = provider
            .charge(&ChargeRequest {
                org_id,
                invoice_id: id,
                invoice_number: invoice.number.clone(),
                amount_minor: req.amount_minor,
                currency: invoice.currency.clone(),
                payment_method: req.payment_method.clone(),
            })
            .await
            .map_err(|e| match e {
                core_billing::PaymentError::Declined { reason, code } => {
                    ApiError::PaymentDeclined {
                        code: code.as_str().into(),
                        provider_reason: reason,
                    }
                }
                core_billing::PaymentError::Unavailable(m) => ApiError::Unavailable(m),
            })?;

        let snap = crate::routes::fx::lock_snapshot(
            &mut ctx.tx,
            org_id,
            req.amount_minor,
            &invoice.currency,
            Utc::now().date_naive(),
        )
        .await?;
        let payment = InvoicePayment {
            id: Uuid::now_v7(),
            org_id,
            invoice_id: id,
            amount_minor: req.amount_minor,
            currency: invoice.currency.clone(),
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
            provider: receipt.provider,
            provider_ref: receipt.provider_ref,
            notes: req.notes.clone(),
            received_at: Utc::now(),
            // Portal identities are not staff rows — provenance lives in
            // the audit row + write trail, never in this FK.
            created_by: None,
        };
        core_db::invoices::insert_payment(&mut *ctx.tx, &payment).await?;
        // Same basis as the staff path: the portal read a payable invoice
        // and validated the amount against its total, so the total is part
        // of the predicate and a portal payer racing an edit loses with a
        // typed 409 rather than settling on a stale total.
        let after = core_db::invoices::apply_payment_total_expecting(
            &mut ctx.tx,
            org_id,
            id,
            req.amount_minor,
            invoice.total_minor,
        )
        .await?;

        ctx.emit(
            types::INVOICE_PAYMENT_RECORDED,
            "invoice_payment",
            Some(payment.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::InvoicePaymentRecorded {
                payment: payment.clone(),
                invoice_status: after.status.as_str().to_string(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        // Append-only provenance, committed with the write (v1 reply
        // pattern): WHO KIND settled the invoice.
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: portal_actor(authed.user.role, authed.user.id),
                action: "portal.invoice_payment".into(),
                entity_type: Some("invoice_payment".into()),
                entity_id: Some(payment.id.to_string()),
                detail: serde_json::json!({
                    "invoice_id": id,
                    "invoice_number": invoice.number,
                    "amount_minor": req.amount_minor,
                    "currency": invoice.currency,
                    "provider": "none",
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "invoice",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "invoice",
                entity_id: id,
                op: "portal_payment_record",
                seq: write_seq,
                before: Some(serde_json::to_value(&invoice).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &PortalPayStoredPayload {
                    payment: payment.clone(),
                    invoice: PortalInvoice::from(after.clone()),
                },
                provenance: instant::ProvenanceContext {
                    // Portal write: land the trail row under the caller's
                    // portal identity kind (IF9 provenance seam, 0028
                    // vocabulary).
                    actor_kind: Some(actor_kind),
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PortalPayResponse {
                payment,
                invoice: PortalInvoice::from(after),
                mutation: meta,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- KB self-service ------------------------------------------------------------

/// GET /api/portal/kb/categories — the client-visible category tree.
#[utoipa::path(operation_id = "portal_kb_categories", get, path = "/api/portal/kb/categories", tag = "portal",
    security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::KbCategory])))]
pub async fn kb_categories(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<core_domain::entities::KbCategory>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewKb)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let categories = core_db::kb::list_client_categories(&mut *tx, authed.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(categories))
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct PortalArticleListQuery {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub category_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /api/portal/kb/articles — published client-facing articles
/// (optionally narrowed to one visible category), gated by the effective
/// client-facing rule in the query.
#[utoipa::path(operation_id = "portal_list_kb_articles", get, path = "/api/portal/kb/articles", tag = "portal",
    security(("bearer" = [])), params(PortalArticleListQuery),
    responses((status = 200, body = [core_domain::entities::KbArticle])))]
pub async fn kb_articles(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PortalArticleListQuery>,
) -> ApiResult<Json<Vec<core_domain::entities::KbArticle>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewKb)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let articles = core_db::kb::list_client_articles(
        &mut *tx,
        authed.user.org_id,
        q.category_id,
        q.limit.clamp(1, 100),
        q.offset.max(0),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(articles))
}

/// GET /api/portal/kb/articles/{id} — one client-gated article; a draft,
/// archived, unflagged, or foreign article is a plain 404.
#[utoipa::path(operation_id = "portal_get_kb_article", get, path = "/api/portal/kb/articles/{id}", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = core_domain::entities::KbArticle), (status = 404)))]
pub async fn kb_article(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<core_domain::entities::KbArticle>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewKb)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let article = core_db::kb::get_client_article(&mut *tx, authed.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(article))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PortalKbSearchHit {
    pub article: core_domain::entities::KbArticle,
    /// FTS rank blended with trigram similarity — ordering signal only.
    pub score: f64,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct PortalKbSearchQuery {
    /// Websearch query (plain words, quoted phrases, `OR`, `-`
    /// exclusions — Postgres websearch syntax).
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

/// GET /api/portal/kb/search — FTS over client-visible published
/// articles only (websearch query + trigram fallback, ranked the same
/// way the unified search ranks).
#[utoipa::path(operation_id = "portal_kb_search", get, path = "/api/portal/kb/search", tag = "portal",
    security(("bearer" = [])), params(PortalKbSearchQuery),
    responses((status = 200, body = [PortalKbSearchHit]),
              (status = 422, description = "empty query")))]
pub async fn kb_search(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PortalKbSearchQuery>,
) -> ApiResult<Json<Vec<PortalKbSearchHit>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewKb)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let term = q.q.trim();
    if term.is_empty() {
        return Err(ApiError::Validation("q is required".into()));
    }
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let hits = core_db::kb::search_client_articles(
        &mut *tx,
        authed.user.org_id,
        term,
        q.limit.clamp(1, 50),
        q.offset.max(0),
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        hits.into_iter()
            .map(|hit| PortalKbSearchHit {
                article: hit.article,
                score: hit.score,
            })
            .collect(),
    ))
}

// --- appointment requests -------------------------------------------------------

/// Free intervals for one staff member over the query range.
#[derive(Debug, Serialize, ToSchema)]
pub struct PortalSlotInterval {
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
}

/// One staff member's published free slots over the query range.
#[derive(Debug, Serialize, ToSchema)]
pub struct PortalStaffSlots {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    /// Sorted, internally disjoint, contiguous FREE stretches. A client
    /// requests a slice of one; the server re-validates at request time
    /// and again at approval.
    pub slots: Vec<PortalSlotInterval>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct PortalSlotsQuery {
    /// Range start (inclusive), ISO date.
    #[schema(value_type = String, format = "date")]
    pub from: chrono::NaiveDate,
    /// Range end (inclusive), ISO date.
    #[schema(value_type = String, format = "date")]
    pub to: chrono::NaiveDate,
    /// Optional: narrow to one staff member.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
}

/// GET /api/portal/schedule/slots — published availability as FREE
/// intervals per staff member: weekly windows minus time-off minus
/// already-booked time (appointments + on-call). Slots that already
/// started are dropped; anything shorter than 30 minutes is noise.
#[utoipa::path(operation_id = "portal_schedule_slots", get, path = "/api/portal/schedule/slots", tag = "portal",
    security(("bearer" = [])), params(PortalSlotsQuery),
    responses((status = 200, body = [PortalStaffSlots]),
              (status = 422, description = "range exceeds 31 days")))]
pub async fn schedule_slots(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PortalSlotsQuery>,
) -> ApiResult<Json<Vec<PortalStaffSlots>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::RequestAppointment)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    if (q.to - q.from).num_days() > MAX_SLOT_RANGE_DAYS {
        return Err(ApiError::Validation(
            "slot range exceeds 31 days; query month by month".into(),
        ));
    }
    // Slots the client could still book: clamp the range start to now.
    let range_start = DateTime::<Utc>::from_naive_utc_and_offset(
        q.from.and_hms_opt(0, 0, 0).unwrap_or_default(),
        Utc,
    )
    .max(Utc::now());
    let range_end = DateTime::<Utc>::from_naive_utc_and_offset(
        q.to.succ_opt()
            .unwrap_or(q.to)
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default(),
        Utc,
    );

    // RLS-scoped read (pool carries no app.org_id GUC): one tx around
    // the whole multi-query body.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let staff = core_db::users::list_users(&mut *tx, org_id).await?;
    let staff: Vec<_> = staff
        .into_iter()
        .filter(|u| u.is_active && q.user_id.is_none_or(|one| u.id == one))
        .collect();
    if staff.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let staff_ids: Vec<Uuid> = staff.iter().map(|u| u.id).collect();

    let windows = core_db::scheduling::list_all_availability_windows(&mut *tx, org_id).await?;
    let appointments = core_db::scheduling::list_appointments_between(
        &mut *tx,
        org_id,
        &staff_ids,
        range_start,
        range_end,
    )
    .await?;
    let on_call = core_db::scheduling::list_on_call_blocks_between(
        &mut *tx,
        org_id,
        &staff_ids,
        range_start,
        range_end,
    )
    .await?;
    let exclusions = core_db::scheduling::list_calendar_exclusions_for_users_between(
        &mut *tx,
        org_id,
        &staff_ids,
        range_start,
        range_end,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let mut view = Vec::new();
    for user in &staff {
        let windows_of: Vec<_> = windows
            .iter()
            .filter(|w| w.user_id == user.id)
            .cloned()
            .collect();
        if windows_of.is_empty() {
            continue;
        }
        let busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = appointments
            .iter()
            .filter(|a| a.user_id == user.id)
            .map(|a| (a.starts_at, a.ends_at))
            .chain(
                on_call
                    .iter()
                    .filter(|b| b.user_id == user.id)
                    .map(|b| (b.starts_at, b.ends_at)),
            )
            .collect();
        let exclusions_of: Vec<_> = exclusions
            .iter()
            .filter(|x| x.user_id == user.id)
            .cloned()
            .collect();
        let slots: Vec<PortalSlotInterval> = core_domain::scheduling::free_intervals(
            &windows_of,
            &exclusions_of,
            &busy,
            range_start,
            range_end,
        )
        .into_iter()
        .filter(|(s, e)| (*e - *s).num_minutes() >= MIN_SLOT_MINUTES)
        .map(|(s, e)| PortalSlotInterval {
            starts_at: s,
            ends_at: e,
        })
        .collect();
        if slots.is_empty() {
            continue;
        }
        view.push(PortalStaffSlots {
            user_id: user.id,
            display_name: user.display_name.clone(),
            slots,
        });
    }
    Ok(Json(view))
}

/// GET /api/portal/appointment-requests — the caller's OWN requests,
/// newest first.
#[utoipa::path(operation_id = "portal_list_appointment_requests", get, path = "/api/portal/appointment-requests", tag = "portal",
    security(("bearer" = [])),
    responses((status = 200, body = [core_domain::entities::AppointmentRequest])))]
pub async fn list_appointment_requests(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<AppointmentRequest>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::RequestAppointment)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::portal::list_own_appointment_requests(
        &mut *tx,
        authed.user.org_id,
        authed.user.client_id,
        authed.user.id,
        100,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalCreateAppointmentRequest {
    /// The staff member whose calendar the client asked for.
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    /// `on_site | remote` (internal/personal are staff-only kinds).
    pub kind: Option<AppointmentKind>,
    pub notes: Option<String>,
    /// Instant-first autosave parameters (IF1): idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2) for appointment-request writes.
/// The request fields sit at the TOP LEVEL (flattened) so the stored
/// replay payload (`request` serialized + merged mutation metadata)
/// round-trips byte-identically.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortalAppointmentRequestMutationResponse {
    #[serde(flatten)]
    pub request: core_domain::entities::AppointmentRequest,
    pub mutation: instant::MutationMeta,
}

/// POST /api/portal/appointment-requests — ask for a slot on a staff
/// member's calendar. The slot must be FULLY inside one free interval of
/// that staff member's PUBLISHED availability (windows minus time-off
/// minus bookings) — requests are intents, so several clients may hold
/// overlapping pending requests; the staff approval is the single point
/// that books the calendar (and re-validates everything).
#[utoipa::path(operation_id = "portal_create_appointment_request", post, path = "/api/portal/appointment-requests", tag = "portal",
    security(("bearer" = [])), request_body = PortalCreateAppointmentRequest,
    responses((status = 200, body = PortalAppointmentRequestMutationResponse),
              (status = 404, description = "requested staff member not found"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "validation failure (span, lead time, or slot outside published availability)")))]
pub async fn create_appointment_request(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Json(req): axum::extract::Json<PortalCreateAppointmentRequest>,
) -> ApiResult<Json<PortalAppointmentRequestMutationResponse>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::RequestAppointment,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let portal_user_id = authed.user.id;

    let request = AppointmentRequest {
        id: Uuid::now_v7(),
        org_id,
        client_id,
        portal_user_id,
        user_id: req.user_id,
        title: req.title.trim().to_string(),
        starts_at: req.starts_at,
        ends_at: req.ends_at,
        kind: req.kind.unwrap_or(AppointmentKind::Remote),
        notes: req.notes.clone().filter(|n| !n.trim().is_empty()),
        status: AppointmentRequestStatus::Pending,
        decision_note: None,
        decided_by: None,
        decided_at: None,
        appointment_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    request
        .validate_new(Utc::now(), MAX_REQUEST_DURATION, MAX_REQUEST_LEAD)
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    if !matches!(
        request.kind,
        core_domain::entities::AppointmentKind::OnSite
            | core_domain::entities::AppointmentKind::Remote
    ) {
        return Err(ApiError::Validation(
            "portal requests may only ask for on_site or remote appointments".into(),
        ));
    }
    // The staff member must exist in this org (RLS-scoped read; pool
    // carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    core_db::users::get_user(&mut *tx, org_id, req.user_id)
        .await
        .map_err(|_| ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let actor_kind = authed.user.role.actor_kind();

    let outcome = org_tx::with_org_tx(&state, org_id, portal_user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "appointment_request",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Availability + conflict gate at request time: the slot must sit
        // fully inside ONE free interval of published availability. The
        // approval path re-validates against the live calendar, so a
        // stale listing cannot wedge a booking in.
        ensure_slot_available(
            &mut ctx,
            org_id,
            request.user_id,
            request.starts_at,
            request.ends_at,
        )
        .await?;
        core_db::portal::create_appointment_request(&mut *ctx.tx, &request).await?;
        ctx.emit(
            types::APPOINTMENT_REQUEST_CREATED,
            "appointment_request",
            Some(request.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::AppointmentRequestCreated {
                request: request.clone(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        // Append-only provenance, committed with the write.
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: portal_actor(authed.user.role, authed.user.id),
                action: "portal.appointment_requested".into(),
                entity_type: Some("appointment_request".into()),
                entity_id: Some(request.id.to_string()),
                detail: serde_json::json!({
                    "staff_user_id": request.user_id,
                    "starts_at": request.starts_at,
                    "ends_at": request.ends_at,
                    "client_id": request.client_id,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "appointment_request",
                entity_id: request.id,
                version: req.instant.version,
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "appointment_request",
                entity_id: request.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&request).unwrap_or(Value::Null),
                updated_at: request.created_at,
                key: key.as_deref(),
                payload: &request,
                provenance: instant::ProvenanceContext {
                    // Portal write: land the trail row under the caller's
                    // portal identity kind (IF9 provenance seam, 0028
                    // vocabulary).
                    actor_kind: Some(actor_kind),
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PortalAppointmentRequestMutationResponse {
                request,
                mutation: meta,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// Body for the portal cancel (idempotency parameters only).
#[derive(Debug, Deserialize, Serialize, Default, ToSchema)]
pub struct PortalCancelRequest {
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// POST /api/portal/appointment-requests/{id}/cancel — the requester
/// withdraws their own still-pending request. Terminal; decided requests
/// cannot be cancelled here (staff owns the calendar once decided).
#[utoipa::path(operation_id = "portal_cancel_appointment_request", post, path = "/api/portal/appointment-requests/{id}/cancel", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PortalCancelRequest,
    responses((status = 200, body = PortalAppointmentRequestMutationResponse),
              (status = 404, description = "request not owned by this portal identity"),
              (status = 409, description = "request is not pending, version conflict, or idempotency key reused")))]
pub async fn cancel_appointment_request(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<PortalCancelRequest>,
) -> ApiResult<Json<PortalAppointmentRequestMutationResponse>> {
    let authed = portal_authed(&extensions)?;
    check_portal(
        authed.user.role,
        core_domain::policy::PortalAction::RequestAppointment,
    )
    .expect()
    .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let portal_user_id = authed.user.id;
    let actor_kind = authed.user.role.actor_kind();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, portal_user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "appointment_request",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::portal::get_appointment_request(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Ownership + client scope double-check (defense in depth).
        if before.client_id != client_id || before.portal_user_id != portal_user_id {
            return Err(ApiError::NotFound);
        }
        if before.status != AppointmentRequestStatus::Pending {
            return Err(ApiError::Conflict(format!(
                "request is {} and can no longer be cancelled",
                before.status.as_str()
            )));
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "appointment_request",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        let after = core_db::portal::cancel_own_appointment_request(
            &mut *ctx.tx,
            org_id,
            id,
            client_id,
            portal_user_id,
        )
        .await?
        .ok_or_else(|| ApiError::Conflict("request left its pending state; retry".into()))?;
        ctx.emit(
            types::APPOINTMENT_REQUEST_CANCELLED,
            "appointment_request",
            Some(id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::AppointmentRequestCancelled {
                request_id: id,
                client_id,
                cancelled_by_portal_user_id: portal_user_id,
                cancelled_at: Utc::now(),
            })
            .unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: portal_actor(authed.user.role, authed.user.id),
                action: "portal.appointment_request_cancelled".into(),
                entity_type: Some("appointment_request".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "client_id": client_id,
                    "staff_user_id": before.user_id,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "appointment_request",
                entity_id: id,
                op: "portal_cancel",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: instant::ProvenanceContext {
                    // Portal write: land the trail row under the caller's
                    // portal identity kind (IF9 provenance seam, 0028
                    // vocabulary).
                    actor_kind: Some(actor_kind),
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PortalAppointmentRequestMutationResponse {
                request: after,
                mutation: meta,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- shared helpers ---------------------------------------------------------

/// Validates that [starts_at, ends_at) sits FULLY inside one free
/// interval of the staff member's published availability: coverage
/// (windows minus time-off) minus live bookings (appointments + on-call)
/// must contain the whole requested span. Shared by the portal create
/// path and the staff approval path.
pub(crate) async fn ensure_slot_available(
    ctx: &mut org_tx::OrgTx<'_>,
    org_id: Uuid,
    user_id: Uuid,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
) -> ApiResult<()> {
    let windows =
        core_db::scheduling::list_availability_windows(&mut *ctx.tx, org_id, user_id).await?;
    if windows.is_empty() {
        return Err(ApiError::Validation(
            "requested staff member has no published availability".into(),
        ));
    }
    let staff_ids = [user_id];
    let busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = core_db::scheduling::list_appointments_between(
        &mut *ctx.tx,
        org_id,
        &staff_ids,
        starts_at,
        ends_at,
    )
    .await?
    .into_iter()
    .map(|a| (a.starts_at, a.ends_at))
    .chain(
        core_db::scheduling::list_on_call_blocks_between(
            &mut *ctx.tx,
            org_id,
            &staff_ids,
            starts_at,
            ends_at,
        )
        .await?
        .into_iter()
        .map(|b| (b.starts_at, b.ends_at)),
    )
    .collect();
    let exclusions = core_db::scheduling::list_calendar_exclusions_for_users_between(
        &mut *ctx.tx,
        org_id,
        &staff_ids,
        starts_at,
        ends_at,
    )
    .await?;
    let free =
        core_domain::scheduling::free_intervals(&windows, &exclusions, &busy, starts_at, ends_at);
    if !free.iter().any(|(s, e)| *s <= starts_at && ends_at <= *e) {
        return Err(ApiError::Validation(
            "requested slot is outside the staff member's published availability".into(),
        ));
    }
    Ok(())
}

// --- quotes (Wave 34 lane 1: portal quote approval) ----------------------------

/// The client-facing quote view. Deliberately NOT the staff entity: the
/// creator provenance and conversion seam ids stay staff-internal — the
/// portal sees the document, its validity, and its lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortalQuote {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub number: String,
    /// `sent | accepted | declined | expired` (drafts are staff-internal).
    pub status: String,
    pub currency: String,
    #[schema(value_type = String, format = "date")]
    pub issued_on: chrono::NaiveDate,
    /// Last day the quote can be accepted; shown honestly on the portal.
    #[schema(value_type = String, format = "date")]
    pub valid_until: chrono::NaiveDate,
    pub notes: Option<String>,
    pub terms: Option<String>,
    pub subtotal_minor: i64,
    pub tax_rate_bps: i32,
    pub tax_minor: i64,
    pub total_minor: i64,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub sent_at: Option<DateTime<Utc>>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub decided_at: Option<DateTime<Utc>>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
}

impl From<core_domain::entities::Quote> for PortalQuote {
    fn from(q: core_domain::entities::Quote) -> Self {
        Self {
            id: q.id,
            number: q.number,
            status: q.status.as_str().to_string(),
            currency: q.currency,
            issued_on: q.issued_on,
            valid_until: q.valid_until,
            notes: q.notes,
            terms: q.terms,
            subtotal_minor: q.subtotal_minor,
            tax_rate_bps: q.tax_rate_bps,
            tax_minor: q.tax_minor,
            total_minor: q.total_minor,
            sent_at: q.sent_at,
            decided_at: q.decided_at,
            created_at: q.created_at,
        }
    }
}

/// A client-facing quote line: description/qty/price/amount only.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortalQuoteLine {
    pub description: String,
    pub unit: String,
    /// Quantity in thousandths of a unit (1.5 = 1500).
    pub quantity_milli: i64,
    pub unit_amount_minor: i64,
    pub taxable: bool,
    pub amount_minor: i64,
    pub position: i32,
}

impl From<core_domain::entities::QuoteLine> for PortalQuoteLine {
    fn from(l: core_domain::entities::QuoteLine) -> Self {
        Self {
            description: l.description,
            unit: l.unit,
            quantity_milli: l.quantity_milli,
            unit_amount_minor: l.unit_amount_minor,
            taxable: l.taxable,
            amount_minor: l.amount_minor,
            position: l.position,
        }
    }
}

/// The quote aggregate as the portal sees it: document + lines.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortalQuoteDetail {
    #[serde(flatten)]
    pub quote: PortalQuote,
    pub lines: Vec<PortalQuoteLine>,
}

/// A quote decision body (approve/decline): envelope params only; the
/// target status is the URL. The SAME instant-first envelope the staff
/// lifecycle legs use.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalQuoteDecisionRequest {
    /// Optional reason, recorded in the audit trail (decline-with-reason).
    pub reason: Option<String>,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2) for a portal quote decision.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PortalQuoteDecisionResponse {
    #[serde(flatten)]
    pub quote: PortalQuote,
    pub mutation: instant::MutationMeta,
}

// The stored decision payload is the FLATTENED PortalQuote (the
// response minus mutation metadata): the mutation metadata merges into
// the same JSON object on record, so a replay deserializes straight
// into the flattened response type.

/// GET /api/portal/quotes — the caller's client's quotes (non-draft
/// only), newest first.
#[utoipa::path(operation_id = "portal_list_quotes", get, path = "/api/portal/quotes", tag = "portal",
    security(("bearer" = [])),
    responses((status = 200, body = [PortalQuote])))]
pub async fn list_quotes(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PortalQuote>>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewQuotes)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let quotes = core_db::quotes::list_client_quotes(
        &mut *tx,
        authed.user.org_id,
        authed.user.client_id,
        200,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(quotes.into_iter().map(Into::into).collect()))
}

/// GET /api/portal/quotes/{id} — one quote of the caller's client with
/// its lines. A foreign quote or a draft is a bare 404 (no existence
/// leak).
#[utoipa::path(operation_id = "portal_get_quote", get, path = "/api/portal/quotes/{id}", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = PortalQuoteDetail),
              (status = 404, description = "quote not visible to this client")))]
pub async fn get_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PortalQuoteDetail>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::ViewQuotes)
        .expect()
        .map_err(ApiError::Forbidden)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, authed.user.org_id, authed.user.id)
        .await
        .map_err(ApiError::from)?;
    let quote =
        core_db::quotes::get_client_quote(&mut *tx, authed.user.org_id, authed.user.client_id, id)
            .await?;
    let lines = core_db::quotes::list_lines(&mut *tx, authed.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(PortalQuoteDetail {
        quote: quote.into(),
        lines: lines.into_iter().map(Into::into).collect(),
    }))
}

/// The shared portal decision engine. Binds the SAME lifecycle legs as
/// the staff quote flow (`routes::quotes::lifecycle`): the SAME domain
/// transition guard (`QuoteStatus::transition`), the SAME guarded SQL
/// move (`core_db::quotes::transition_status`), the SAME typed events,
/// instant-first envelope — the only differences are the portal
/// principal, the client-scoped load, and the portal actor provenance.
/// The conversion-to-invoice seam stays a staff action; an accepted
/// quote simply becomes convertible by the provider.
#[allow(clippy::too_many_arguments)]
async fn portal_quote_decision(
    state: AppState,
    extensions: Extensions,
    id: Uuid,
    req: PortalQuoteDecisionRequest,
    op: &'static str,
    to: core_domain::entities::QuoteStatus,
) -> ApiResult<Json<PortalQuoteDecisionResponse>> {
    let authed = portal_authed(&extensions)?;
    check_portal(authed.user.role, PortalAction::DecideQuotes)
        .expect()
        .map_err(ApiError::Forbidden)?;
    let org_id = authed.user.org_id;
    let client_id = authed.user.client_id;
    let actor_kind = authed.user.role.actor_kind();
    let action_label = format!("portal.quote_{op}");
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, authed.user.id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "quote",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Client-scoped read INSIDE the mutation: a foreign or draft
        // quote is an rls_sparse, never an existence hint.
        let before = core_db::quotes::get_client_quote(&mut *ctx.tx, org_id, client_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // The SAME domain guard the staff legs run.
        core_domain::entities::QuoteStatus::transition(
            core_domain::entities::QuoteStatus::Sent,
            to,
        )
        .map_err(ApiError::from)?;
        let after = core_db::quotes::transition_status(
            &mut *ctx.tx,
            org_id,
            id,
            core_domain::entities::QuoteStatus::Sent.as_str(),
            to.as_str(),
            false,
            true,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "quote left its {} state; retry",
                core_domain::entities::QuoteStatus::Sent.as_str()
            ))
        })?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "quote",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        // The SAME typed event emission the staff legs run.
        crate::routes::quotes::emit_lifecycle_event(&mut ctx, to, &after).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: portal_actor(authed.user.role, authed.user.id),
                action: action_label,
                entity_type: Some("quote".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "number": after.number,
                    "status": after.status.as_str(),
                    "reason": req.reason,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let meta = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "quote",
                entity_id: id,
                op,
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &PortalQuote::from(after.clone()),
                provenance: instant::ProvenanceContext {
                    // Portal write: land the trail row under the caller's
                    // portal identity kind (IF9 provenance seam).
                    actor_kind: Some(actor_kind),
                    ..Default::default()
                },
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PortalQuoteDecisionResponse {
                quote: after.into(),
                mutation: meta,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// POST /api/portal/quotes/{id}/approve — the client accepts a sent
/// quote. Same `sent → accepted` leg as the staff flow; the provider
/// can then run the conversion-to-invoice seam.
#[utoipa::path(operation_id = "portal_approve_quote", post, path = "/api/portal/quotes/{id}/approve", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PortalQuoteDecisionRequest,
    responses((status = 200, body = PortalQuoteDecisionResponse),
              (status = 404, description = "quote not visible to this client"),
              (status = 409, description = "quote not decidable, version conflict, or idempotency key reused")))]
pub async fn approve_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<PortalQuoteDecisionRequest>,
) -> ApiResult<Json<PortalQuoteDecisionResponse>> {
    portal_quote_decision(
        state,
        extensions,
        id,
        req,
        "approve",
        core_domain::entities::QuoteStatus::Accepted,
    )
    .await
}

/// POST /api/portal/quotes/{id}/decline — the client declines a sent
/// quote. Same `sent → declined` leg as the staff flow; the optional
/// reason is audit-only, never shown back as validation.
#[utoipa::path(operation_id = "portal_decline_quote", post, path = "/api/portal/quotes/{id}/decline", tag = "portal",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = PortalQuoteDecisionRequest,
    responses((status = 200, body = PortalQuoteDecisionResponse),
              (status = 404, description = "quote not visible to this client"),
              (status = 409, description = "quote not decidable, version conflict, or idempotency key reused")))]
pub async fn decline_quote(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Json(req): axum::extract::Json<PortalQuoteDecisionRequest>,
) -> ApiResult<Json<PortalQuoteDecisionResponse>> {
    portal_quote_decision(
        state,
        extensions,
        id,
        req,
        "decline",
        core_domain::entities::QuoteStatus::Declined,
    )
    .await
}
