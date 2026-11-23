//! Accounting export: send an invoice or a credit note to a QuickBooks/Xero
//! connection, and read back the customer's books (claims-gaps G1–G3).
//!
//! Purpose. Nothing in Core called `ExportSink` before this module: the two
//! adapters were libraries with a trait. This is the orchestration, and it owns
//! the four decisions the adapters deliberately do not make:
//!
//! * **Which document goes to which company.** The caller names a connection;
//!   the vendor (QBO or Xero) is read from that connection's installed
//!   manifest, never from a request field, because ADR 0032 makes the manifest
//!   the definition of a connector.
//! * **The vendor customer id.** Read from `external_refs` (the client IS that
//!   customer on that connection). There is no client→vendor map here, because
//!   inventing a second one would be a second thing to keep true. No link is a
//!   typed 422, not a guess.
//! * **The tax code.** Core carries no tax on an invoice (the ADR 0036
//!   boundary), so the tax code is an explicit input of the export and is
//!   remembered on the run. It is resolved through a per-connection mapping and
//!   checked against the vendor's own list. Unmapped ⇒ the run is HELD with a
//!   typed reason; it is never exported with a substituted default.
//! * **The retry.** A claim is committed before the vendor call and the receipt
//!   is written in the same transaction as the terminal state. A run that comes
//!   back from `in_flight` with no receipt (a process died between the two)
//!   reconciles against the vendor by the marker the adapter stamps into the
//!   document before it sends anything.
//!
//! What this route returns. An export answers with the RUN, not with an empty
//! 204: `state` and `reason` are the product. A vendor refusal is a `failed`
//! run (200, `state: "failed"`), because the request itself was valid and the
//! report is the single place a person reads what happened. Everything that is
//! a refusal of the REQUEST — an unknown connection, an unsupported connector,
//! a missing credential, a second attempt racing a live one — is an HTTP error
//! with a code naming the fault.
//!
//! Held versus failed. `held` means nothing was sent and a person has a
//! decision: an unmapped tax code, a mapping the books no longer contain, or a
//! credit note whose parent invoice was never exported. `failed` means the
//! attempt happened and went wrong. The report filters on `state`, and retrying
//! a held run is a different act from retrying a failed one.
//!
//! Non-purpose: no scheduled/automatic export (a `credit_note.issued`
//! subscription belongs in the worker, which this lane does not own), no
//! payment export, no vendor-side document edit or void, and no credential
//! refresh — a rotated token is an operator act on the sealed store.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use core_db::accounting_export::{
    ClaimOutcome, ClaimSpec, ExportReceiptRow, ExportRun, ExportState, ExportVendor, RecordKind,
    RunFilters, TaxMapping,
};
use core_domain::connectors::RecordType;
use core_domain::entities::{Actor, AuditEvent, CreditNoteStatus, InvoiceStatus};
use core_domain::policy::{Action, Module, Permission};
// The two adapters' traits have to be in scope for method resolution. Both are
// named `ExportSink` and both are imported anonymously: this module never names
// the trait, it just calls the methods on the concrete client.
use qbo_export_sync::ExportSink as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use utoipa::ToSchema;
use uuid::Uuid;
use xero_export_sync::ExportSink as _;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

/// How long a claim is believed before another attempt may take the run.
///
/// Five minutes, and the number is only safe because every adapter request is
/// bounded well below it (30s wall clock, 10s connect). A retry inside the
/// window is refused with `export_in_flight` rather than racing a live call;
/// outside it the claiming process is gone, and the retry reconciles before it
/// sends anything.
const RECLAIM_AFTER_SECS: i64 = 300;

/// The report's page cap. A report that can return an unbounded set is a report
/// that pages by accident.
const MAX_RUNS: i64 = 200;

// --- wire structs -------------------------------------------------------------

/// POST /invoices/{id}/export and /credit-notes/{id}/export body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ExportRequest {
    /// The accounting connection to export through. It names the company (QBO
    /// realm, Xero tenant) and carries the sealed credential.
    pub connection_id: Uuid,
    /// The Marula tax code this document is for, when one applies.
    ///
    /// Core carries no tax on an invoice (the ADR 0036 boundary), so the export
    /// names it. It is matched against the connection's mapping store and then
    /// against the vendor's own list of codes; either miss holds the run instead
    /// of exporting a guess.
    pub tax_code: Option<String>,
}

/// One export run as the report renders it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExportRunView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    /// The operator's label for the connection ("Xero — Contoso"), so a row
    /// names a company instead of a uuid.
    pub connection_label: String,
    /// `qbo` | `xero`.
    pub vendor: String,
    /// `invoice` | `credit_note`.
    pub record_kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub record_id: Uuid,
    /// `in_flight` | `succeeded` | `failed` | `held`. Held is NOT failed: it is
    /// waiting on a human, and the UI must not offer "retry" as the only verb.
    pub state: String,
    /// Typed reason code — present for `held` and `failed`, absent otherwise.
    /// Held: `tax_code_unmapped`, `tax_code_unknown_to_vendor`,
    /// `parent_invoice_not_exported`. Failed: `auth_failed`, `rate_limited`,
    /// `vendor_rejected`, `vendor_unavailable`, `network_failed`,
    /// `export_disabled`, `invalid_export_request`.
    pub reason: Option<String>,
    /// The vendor's own words, verbatim, when the vendor said anything.
    pub reason_detail: Option<String>,
    /// The Marula tax code this run is for.
    pub tax_code: Option<String>,
    /// The vendor code the mapping resolved to, when it did — a report can name
    /// the code the books are missing.
    pub vendor_tax_code: Option<String>,
    pub attempts: i32,
    pub claimed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// The vendor record, present when the run succeeded. A credit note's link
    /// to its parent invoice is not here: it is the parent's receipt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ExportReceiptView>,
}

/// The vendor record an export produced.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExportReceiptView {
    /// The vendor's id for the document — QBO `Id`, Xero `InvoiceID` /
    /// `CreditNoteID`. Never the document number, and never `txnId`.
    pub external_id: String,
    pub external_number: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /accounting/export-runs query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct RunListQuery {
    pub connection_id: Option<Uuid>,
    /// `in_flight` | `succeeded` | `failed` | `held`.
    pub state: Option<String>,
    /// `invoice` | `credit_note`.
    pub record_kind: Option<String>,
    pub record_id: Option<Uuid>,
    /// Page size (1–200, default 50).
    pub limit: Option<i64>,
}

/// One account in the customer's chart of accounts, neutral-shaped (G2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LedgerAccountView {
    /// The vendor's account id.
    pub id: String,
    /// The operator-facing code when the vendor has one (Xero `Code`, QBO
    /// `AcctNum`). A Xero line item carries this, not the id.
    pub code: Option<String>,
    pub name: String,
    /// A vendor-neutral bucket: `bank`, `income`, `expense`,
    /// `cost_of_goods_sold`, `other_current_asset`, … or `unknown` for a
    /// vendor type nobody planned. Core branches on this, not on vendor text.
    pub kind: String,
    pub active: bool,
}

/// One tax code in the customer's books, neutral-shaped (G2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaxCodeView {
    /// The id a vendor document line carries (QBO `TaxCode.Id`, Xero
    /// `TaxType`). This is the value a mapping stores.
    pub code: String,
    pub name: String,
    /// Effective rate in basis points (8.25% = 825), absent when the vendor
    /// publishes no single rate.
    pub rate_bps: Option<i32>,
    pub active: bool,
}

/// GET /accounting/connections/{id}/ledger — the read leg's payload.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LedgerView {
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    pub vendor: String,
    pub accounts: Vec<LedgerAccountView>,
    pub tax_codes: Vec<TaxCodeView>,
}

/// One Marula→vendor tax code mapping.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaxMappingView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    pub marula_tax_code: String,
    pub vendor_tax_code: String,
    pub vendor_tax_code_name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// PUT /accounting/tax-mappings body.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpsertTaxMappingRequest {
    pub connection_id: Uuid,
    pub marula_tax_code: String,
    /// The vendor code (QBO `TaxCode.Id`, Xero `TaxType`). It is checked
    /// against the connection's own list before it is stored, so a typo is a
    /// refusal here rather than a held invoice later.
    pub vendor_tax_code: String,
    /// The vendor's name for the code, captured for the report. Optional: the
    /// vendor's own list supplies it when omitted.
    pub vendor_tax_code_name: Option<String>,
}

/// GET /accounting/tax-mappings query.
#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TaxMappingListQuery {
    pub connection_id: Option<Uuid>,
}

// --- the document being exported ---------------------------------------------

/// One document as the export needs it, vendor-neutral.
///
/// Credit notes come through here too because the differences that matter are
/// two fields: a credit note has a parent invoice's vendor id and a reason, and
/// Core models it as one face value rather than lines (see [`doc_lines`]).
struct Doc {
    kind: RecordKind,
    id: Uuid,
    number: String,
    client_id: Uuid,
    issued_on: NaiveDate,
    /// The payment terms date. A credit note has none — it is due the day it is
    /// issued — so the loaders pass `issued_on` there.
    due_on: NaiveDate,
    currency: String,
    lines: Vec<DocLine>,
    notes: Option<String>,
    /// The vendor invoice id a credit note allocates against. `None` for an
    /// invoice, and never defaulted for a credit note — an unallocated credit is
    /// a second document for the same money.
    parent_invoice_external_id: Option<String>,
    /// The parent invoice's Marula id, for the "was it ever exported here?"
    /// question the run report has to answer.
    parent_invoice_id: Option<Uuid>,
    /// The Marula tax code the caller named.
    tax_code: Option<String>,
    reason: Option<String>,
}

struct DocLine {
    description: String,
    quantity: String,
    unit_amount_minor: i64,
}

/// Loads an invoice and refuses the states that must not be exported.
async fn load_invoice(
    conn: &mut PgConnection,
    org_id: Uuid,
    invoice_id: Uuid,
    tax_code: Option<String>,
) -> ApiResult<Doc> {
    let invoice = core_db::invoices::get_invoice(&mut *conn, org_id, invoice_id).await?;
    if invoice.status == InvoiceStatus::Void {
        return Err(ApiError::Conflict(
            "invoice_void: a voided invoice is not exported; the credit note that reverses it is"
                .into(),
        ));
    }
    let lines = core_db::invoices::list_lines(&mut *conn, org_id, invoice_id).await?;
    if lines.is_empty() {
        return Err(ApiError::Validation(format!(
            "invoice_has_no_lines: invoice {} has no lines, so there is nothing to export",
            invoice.number
        )));
    }
    Ok(Doc {
        kind: RecordKind::Invoice,
        id: invoice.id,
        number: invoice.number,
        client_id: invoice.client_id,
        issued_on: invoice.issued_at,
        due_on: invoice.due_on,
        currency: invoice.currency,
        lines: lines
            .into_iter()
            .map(|l| DocLine {
                description: l.description,
                // Core's billing lines carry labor minutes, not a decimal
                // quantity: the line is already priced at `amount_minor`, so the
                // exported quantity is one unit at the line amount. A quantity
                // reconstructed from minutes would be a number the invoice never
                // said.
                quantity: "1".to_string(),
                unit_amount_minor: l.amount_minor,
            })
            .collect(),
        notes: invoice.notes,
        parent_invoice_external_id: None,
        parent_invoice_id: None,
        tax_code,
        reason: None,
    })
}

/// Loads a credit note. A free-standing credit (no parent invoice) is refused
/// here rather than in the adapters: this lane's contract is "adjust an invoice
/// that was exported", and a standalone credit memo is a different product
/// decision.
async fn load_credit_note(
    conn: &mut PgConnection,
    org_id: Uuid,
    credit_note_id: Uuid,
    tax_code: Option<String>,
) -> ApiResult<Doc> {
    let note = core_db::credit_notes::get_credit_note(&mut *conn, org_id, credit_note_id).await?;
    if note.status == CreditNoteStatus::Void {
        return Err(ApiError::Conflict(
            "credit_note_void: a voided credit note is not exported".into(),
        ));
    }
    let parent_invoice_id = note.issued_against_invoice_id.ok_or_else(|| {
        ApiError::Validation(format!(
            "credit_note_has_no_invoice: credit note {} is a free-standing client credit; this \
             export adjusts an exported invoice and does not create a standalone vendor credit",
            note.number
        ))
    })?;
    let description = note
        .reason
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| format!("Credit note {}", note.number));
    Ok(Doc {
        kind: RecordKind::CreditNote,
        id: note.id,
        number: note.number,
        client_id: note.client_id,
        issued_on: note.created_at.date_naive(),
        due_on: note.created_at.date_naive(),
        currency: note.currency,
        // Core models a credit note as one face value, not lines. One line at
        // the total is the honest mapping: it is exactly the money the credit
        // moves, and the reason rides the description so the vendor side reads
        // as an adjustment rather than an unexplained amount.
        lines: vec![DocLine {
            description,
            quantity: "1".to_string(),
            unit_amount_minor: note.total_minor,
        }],
        notes: None,
        parent_invoice_external_id: None,
        parent_invoice_id: Some(parent_invoice_id),
        tax_code,
        reason: note.reason,
    })
}

// --- the vendor seam ----------------------------------------------------------

/// The two adapters, behind one match.
///
/// The crates are deliberately self-contained (an Apache-2.0 connector depends
/// on nothing in `crates/`), so they have distinct types with the same shape.
/// This enum is the one place that difference is paid for.
enum Sink {
    Qbo(qbo_export_sync::QboClient),
    Xero(xero_export_sync::XeroClient),
}

/// The result of a vendor write, flattened so the rest of this module does not
/// care which adapter produced it.
struct Pushed {
    external_id: String,
    external_number: Option<String>,
    /// False when the document was adopted from a previous attempt instead of
    /// created by this call.
    created: bool,
}

impl Sink {
    async fn push_invoice(&self, doc: &Doc, customer_ref: &str) -> Result<Pushed, AdapterError> {
        match self {
            Sink::Qbo(client) => client
                .push_invoice(&qbo_invoice(doc, customer_ref))
                .await
                .map(flatten_qbo)
                .map_err(adapter_error_qbo),
            Sink::Xero(client) => client
                .push_invoice(&xero_invoice(doc, customer_ref))
                .await
                .map(flatten_xero)
                .map_err(adapter_error_xero),
        }
    }

    async fn push_credit_note(
        &self,
        doc: &Doc,
        customer_ref: &str,
    ) -> Result<Pushed, AdapterError> {
        match self {
            Sink::Qbo(client) => client
                .push_credit_note(&qbo_credit_note(doc, customer_ref))
                .await
                .map(flatten_qbo)
                .map_err(adapter_error_qbo),
            Sink::Xero(client) => client
                .push_credit_note(&xero_credit_note(doc, customer_ref))
                .await
                .map(flatten_xero)
                .map_err(adapter_error_xero),
        }
    }

    /// Did an earlier attempt already create this document? Answered before a
    /// retry sends anything, which is the only defence either vendor offers —
    /// neither takes an idempotency key.
    async fn find_document(&self, doc: &Doc) -> Result<Option<Pushed>, AdapterError> {
        match self {
            Sink::Qbo(client) => client
                .find_document(&qbo_export_sync::ExportLookup {
                    kind: qbo_lookup_kind(doc.kind),
                    marula_id: doc.id,
                    number: doc.number.clone(),
                })
                .await
                .map(|found| found.map(flatten_qbo))
                .map_err(adapter_error_qbo),
            Sink::Xero(client) => client
                .find_document(&xero_export_sync::ExportLookup {
                    kind: xero_lookup_kind(doc.kind),
                    marula_id: doc.id,
                    number: doc.number.clone(),
                })
                .await
                .map(|found| found.map(flatten_xero))
                .map_err(adapter_error_xero),
        }
    }

    /// The customer's chart of accounts, neutral-shaped, with the vendor that
    /// produced it. Returns the parts rather than a `LedgerView` because the
    /// view carries a connection id this enum does not know.
    async fn read_accounts(&self) -> Result<(ExportVendor, Vec<LedgerAccountView>), AdapterError> {
        match self {
            Sink::Qbo(client) => {
                let accounts = client.read_accounts().await.map_err(adapter_error_qbo)?;
                Ok((
                    ExportVendor::Qbo,
                    accounts
                        .into_iter()
                        .map(|a| LedgerAccountView {
                            id: a.id,
                            code: a.code,
                            name: a.name,
                            kind: kind_name_qbo(a.kind),
                            active: a.active,
                        })
                        .collect(),
                ))
            }
            Sink::Xero(client) => {
                let accounts = client.read_accounts().await.map_err(adapter_error_xero)?;
                Ok((
                    ExportVendor::Xero,
                    accounts
                        .into_iter()
                        .map(|a| LedgerAccountView {
                            id: a.id,
                            code: a.code,
                            name: a.name,
                            kind: kind_name_xero(a.kind),
                            active: a.active,
                        })
                        .collect(),
                ))
            }
        }
    }

    /// The customer's tax codes, neutral-shaped.
    ///
    /// Its own method rather than half of [`Sink::read_ledger`] because the
    /// export path checks a tax code on every send: fetching the chart of
    /// accounts it does not read would be a second vendor round trip on the hot
    /// path.
    async fn read_tax_codes(&self) -> Result<Vec<TaxCodeView>, AdapterError> {
        match self {
            Sink::Qbo(client) => Ok(client
                .read_tax_codes()
                .await
                .map_err(adapter_error_qbo)?
                .into_iter()
                .map(|t| TaxCodeView {
                    code: t.code,
                    name: t.name,
                    rate_bps: t.rate_bps,
                    active: t.active,
                })
                .collect()),
            Sink::Xero(client) => Ok(client
                .read_tax_codes()
                .await
                .map_err(adapter_error_xero)?
                .into_iter()
                .map(|t| TaxCodeView {
                    code: t.code,
                    name: t.name,
                    rate_bps: t.rate_bps,
                    active: t.active,
                })
                .collect()),
        }
    }

    /// Both read legs, for the ledger route.
    async fn read_ledger(
        &self,
    ) -> Result<(ExportVendor, Vec<LedgerAccountView>, Vec<TaxCodeView>), AdapterError> {
        let (vendor, accounts) = self.read_accounts().await?;
        let tax_codes = self.read_tax_codes().await?;
        Ok((vendor, accounts, tax_codes))
    }
}

/// One error type over the two adapters' identical error enums.
#[derive(Debug)]
struct AdapterError {
    reason: &'static str,
    detail: Option<String>,
    /// The adapter's own verdict on retrying, which decides whether the run is
    /// worth another attempt at all.
    retryable: bool,
}

fn adapter_error_qbo(e: qbo_export_sync::ExportError) -> AdapterError {
    let reason = qbo_reason(&e);
    AdapterError {
        reason,
        detail: adapter_detail_qbo(&e),
        retryable: e.retryable(),
    }
}

fn adapter_error_xero(e: xero_export_sync::ExportError) -> AdapterError {
    let reason = xero_reason(&e);
    AdapterError {
        reason,
        detail: adapter_detail_xero(&e),
        retryable: e.retryable(),
    }
}

/// The typed reason code a failed run carries. One vocabulary across both
/// vendors, because a report that said `qbo_401` in one row and `xero_auth` in
/// the next would be two reports.
fn qbo_reason(e: &qbo_export_sync::ExportError) -> &'static str {
    match e {
        qbo_export_sync::ExportError::Disabled => "export_disabled",
        qbo_export_sync::ExportError::InvalidRequest(_) => "invalid_export_request",
        qbo_export_sync::ExportError::Unauthorized(_) => "auth_failed",
        qbo_export_sync::ExportError::RateLimited(_) => "rate_limited",
        qbo_export_sync::ExportError::Vendor { status, .. } if *status < 500 => "vendor_rejected",
        qbo_export_sync::ExportError::Vendor { .. } => "vendor_unavailable",
        qbo_export_sync::ExportError::Network(_) => "network_failed",
    }
}

fn xero_reason(e: &xero_export_sync::ExportError) -> &'static str {
    match e {
        xero_export_sync::ExportError::Disabled => "export_disabled",
        xero_export_sync::ExportError::InvalidRequest(_) => "invalid_export_request",
        xero_export_sync::ExportError::Unauthorized(_) => "auth_failed",
        xero_export_sync::ExportError::RateLimited(_) => "rate_limited",
        xero_export_sync::ExportError::Vendor { status, .. } if *status < 500 => "vendor_rejected",
        xero_export_sync::ExportError::Vendor { .. } => "vendor_unavailable",
        xero_export_sync::ExportError::Network(_) => "network_failed",
    }
}

/// The vendor's own words, kept verbatim. `Display` is what the adapters
/// guarantee to be free of the token; it is bounded at the adapter's edge.
fn adapter_detail_qbo(e: &qbo_export_sync::ExportError) -> Option<String> {
    match e {
        qbo_export_sync::ExportError::Disabled => None,
        _ => Some(e.to_string()),
    }
}

fn adapter_detail_xero(e: &xero_export_sync::ExportError) -> Option<String> {
    match e {
        xero_export_sync::ExportError::Disabled => None,
        _ => Some(e.to_string()),
    }
}

fn flatten_qbo(r: qbo_export_sync::ExportReceipt) -> Pushed {
    Pushed {
        external_id: r.external_id,
        external_number: r.external_number,
        created: r.created,
    }
}

fn flatten_xero(r: xero_export_sync::ExportReceipt) -> Pushed {
    Pushed {
        external_id: r.external_id,
        external_number: r.external_number,
        created: r.created,
    }
}

fn qbo_lookup_kind(kind: RecordKind) -> qbo_export_sync::ExportLookupKind {
    match kind {
        RecordKind::CreditNote => qbo_export_sync::ExportLookupKind::CreditNote,
        // Payments never reach a reconcile in this lane.
        RecordKind::Invoice | RecordKind::Payment => qbo_export_sync::ExportLookupKind::Invoice,
    }
}

fn xero_lookup_kind(kind: RecordKind) -> xero_export_sync::ExportLookupKind {
    match kind {
        RecordKind::CreditNote => xero_export_sync::ExportLookupKind::CreditNote,
        RecordKind::Invoice | RecordKind::Payment => xero_export_sync::ExportLookupKind::Invoice,
    }
}

fn kind_name_qbo(kind: qbo_export_sync::AccountKind) -> String {
    use qbo_export_sync::AccountKind::*;
    match kind {
        Bank => "bank",
        AccountsReceivable => "accounts_receivable",
        OtherCurrentAsset => "other_current_asset",
        FixedAsset => "fixed_asset",
        OtherAsset => "other_asset",
        AccountsPayable => "accounts_payable",
        CreditCard => "credit_card",
        OtherCurrentLiability => "other_current_liability",
        LongTermLiability => "long_term_liability",
        Equity => "equity",
        Income => "income",
        CostOfGoodsSold => "cost_of_goods_sold",
        Expense => "expense",
        OtherIncome => "other_income",
        OtherExpense => "other_expense",
        Unknown => "unknown",
    }
    .to_string()
}

fn kind_name_xero(kind: xero_export_sync::AccountKind) -> String {
    use xero_export_sync::AccountKind::*;
    match kind {
        Bank => "bank",
        AccountsReceivable => "accounts_receivable",
        OtherCurrentAsset => "other_current_asset",
        FixedAsset => "fixed_asset",
        OtherAsset => "other_asset",
        AccountsPayable => "accounts_payable",
        CreditCard => "credit_card",
        OtherCurrentLiability => "other_current_liability",
        LongTermLiability => "long_term_liability",
        Equity => "equity",
        Income => "income",
        CostOfGoodsSold => "cost_of_goods_sold",
        Expense => "expense",
        OtherIncome => "other_income",
        OtherExpense => "other_expense",
        Unknown => "unknown",
    }
    .to_string()
}

fn qbo_lines(doc: &Doc) -> Vec<qbo_export_sync::ExportLine> {
    doc.lines
        .iter()
        .map(|l| qbo_export_sync::ExportLine {
            description: l.description.clone(),
            quantity: l.quantity.clone(),
            unit_amount_minor: l.unit_amount_minor,
        })
        .collect()
}

fn xero_lines(doc: &Doc) -> Vec<xero_export_sync::ExportLine> {
    doc.lines
        .iter()
        .map(|l| xero_export_sync::ExportLine {
            description: l.description.clone(),
            quantity: l.quantity.clone(),
            unit_amount_minor: l.unit_amount_minor,
        })
        .collect()
}

fn qbo_invoice(doc: &Doc, customer_ref: &str) -> qbo_export_sync::ExportInvoice {
    qbo_export_sync::ExportInvoice {
        invoice_id: doc.id,
        number: doc.number.clone(),
        customer_ref: customer_ref.to_string(),
        issue_date: doc.issued_on,
        due_date: doc.due_on,
        currency: doc.currency.clone(),
        lines: qbo_lines(doc),
        notes: doc.notes.clone(),
        tax_code: doc.tax_code.clone(),
    }
}

fn xero_invoice(doc: &Doc, customer_ref: &str) -> xero_export_sync::ExportInvoice {
    xero_export_sync::ExportInvoice {
        invoice_id: doc.id,
        number: doc.number.clone(),
        customer_ref: customer_ref.to_string(),
        issue_date: doc.issued_on,
        due_date: doc.due_on,
        currency: doc.currency.clone(),
        lines: xero_lines(doc),
        notes: doc.notes.clone(),
        tax_code: doc.tax_code.clone(),
    }
}

fn qbo_credit_note(doc: &Doc, customer_ref: &str) -> qbo_export_sync::ExportCreditNote {
    qbo_export_sync::ExportCreditNote {
        credit_note_id: doc.id,
        number: doc.number.clone(),
        customer_ref: customer_ref.to_string(),
        invoice_external_id: doc.parent_invoice_external_id.clone().unwrap_or_default(),
        issued_on: doc.issued_on,
        currency: doc.currency.clone(),
        lines: qbo_lines(doc),
        tax_code: doc.tax_code.clone(),
        reason: doc.reason.clone(),
    }
}

fn xero_credit_note(doc: &Doc, customer_ref: &str) -> xero_export_sync::ExportCreditNote {
    xero_export_sync::ExportCreditNote {
        credit_note_id: doc.id,
        number: doc.number.clone(),
        customer_ref: customer_ref.to_string(),
        invoice_external_id: doc.parent_invoice_external_id.clone().unwrap_or_default(),
        issued_on: doc.issued_on,
        currency: doc.currency.clone(),
        lines: xero_lines(doc),
        tax_code: doc.tax_code.clone(),
        reason: doc.reason.clone(),
    }
}

// --- connection resolution ----------------------------------------------------

/// The accounting vendor a connection's manifest declares.
///
/// Read from the manifest name and the catalog slug, because ADR 0032 makes the
/// manifest the definition of a connector: a second "this is QBO" column would
/// be a second thing to keep in step with it.
fn vendor_for_manifest(name: &str, slug: &str) -> Option<ExportVendor> {
    for candidate in [name, slug] {
        let lower = candidate.to_ascii_lowercase();
        if lower.contains("quickbooks") || lower.contains("qbo") {
            return Some(ExportVendor::Qbo);
        }
        if lower.contains("xero") {
            return Some(ExportVendor::Xero);
        }
    }
    None
}

/// What a connection carries, decrypted just enough to build a client.
///
/// The sealed document is the adapter's contract, not a vendor-wide one:
///
/// * QBO — `{"access_token": "…", "realm_id": "…"}`
/// * Xero — `{"access_token": "…", "tenant_id": "…"}`
///
/// `base_url` is read from the connection's non-secret SETTINGS, never the
/// sealed blob, because a sandbox host is not a secret and the roster should be
/// able to show where a connection points.
async fn build_sink(
    state: &AppState,
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    connection: &core_db::connector_connections::Connection,
    vendor: ExportVendor,
) -> ApiResult<Sink> {
    let sealed = core_db::accounting_export::get_connection_secret(conn, org_id, connection_id)
        .await?
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "connection_has_no_credentials: connection {} has no sealed credential",
                connection.label
            ))
        })?;
    let plaintext = crate::crypto::open(&state.config.secret_key, &sealed).map_err(|_| {
        ApiError::Unavailable(format!(
            "connection_credentials_unreadable: connection {} could not be unsealed",
            connection.label
        ))
    })?;
    let creds: Value = serde_json::from_slice(&plaintext).map_err(|_| {
        ApiError::Validation(format!(
            "connection_credentials_malformed: connection {} does not carry a JSON credential",
            connection.label
        ))
    })?;

    let field = |name: &str| -> ApiResult<String> {
        creds
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ApiError::Validation(format!(
                    "connection_credentials_incomplete: connection {} is missing `{name}`",
                    connection.label
                ))
            })
    };

    let base_url = connection
        .settings
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    match vendor {
        ExportVendor::Qbo => {
            let mut config =
                qbo_export_sync::QboConfig::new(field("access_token")?, field("realm_id")?);
            if let Some(base) = base_url {
                config.base_url = base;
            }
            let client = qbo_export_sync::QboClient::new(config).map_err(|e| {
                ApiError::Unavailable(format!(
                    "export_client_unavailable: {}",
                    adapter_detail_qbo(&e).unwrap_or_default()
                ))
            })?;
            Ok(Sink::Qbo(client))
        }
        ExportVendor::Xero => {
            let mut config =
                xero_export_sync::XeroConfig::new(field("access_token")?, field("tenant_id")?);
            if let Some(base) = base_url {
                config.base_url = base;
            }
            let client = xero_export_sync::XeroClient::new(config).map_err(|e| {
                ApiError::Unavailable(format!(
                    "export_client_unavailable: {}",
                    adapter_detail_xero(&e).unwrap_or_default()
                ))
            })?;
            Ok(Sink::Xero(client))
        }
    }
}

/// The vendor customer id for a client on a connection.
///
/// `external_refs` (migration 0185) is where "this Marula client IS that vendor
/// customer" lives. A missing link is a 422 naming the fix, never an invented
/// id: the adapters have no create-customer call by design.
async fn customer_ref(
    conn: &mut PgConnection,
    org_id: Uuid,
    connection_id: Uuid,
    client_id: Uuid,
) -> ApiResult<String> {
    let refs = core_db::connector_connections::list_refs_for_record(
        conn,
        org_id,
        RecordType::Client,
        client_id,
    )
    .await?;
    // Prefer a reference that belongs to THIS connection, then any reference
    // whose kind reads as a customer. A reference to another company's customer
    // id would be the wrong id in the wrong books.
    refs.iter()
        .find(|r| {
            r.connection_id == Some(connection_id)
                && matches!(r.external_kind.as_str(), "customer" | "contact")
        })
        .or_else(|| refs.iter().find(|r| r.connection_id == Some(connection_id)))
        .map(|r| r.external_id.clone())
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "client_not_linked_to_connection: client {client_id} has no vendor customer id on \
                 this connection; link the client to its accounting customer first"
            ))
        })
}

// --- the export flow ----------------------------------------------------------

/// Shared body of both export routes: claim, resolve, send, settle.
async fn run_export(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    connection_id: Uuid,
    mut doc: Doc,
) -> ApiResult<ExportRunView> {
    // 1. Connection + vendor, in the caller's org scope.
    let (connection, vendor) = {
        let mut tx = org_tx::begin(state, org_id, user_id).await?;
        let connection =
            core_db::connector_connections::get_connection(&mut tx.tx, org_id, connection_id)
                .await?
                .ok_or(ApiError::NotFound)?;
        if connection.status == core_domain::connectors::ConnectionStatus::Disabled {
            return Err(ApiError::Conflict(format!(
                "connection_disabled: connection {} is disabled",
                connection.label
            )));
        }
        let (name, slug) = core_db::accounting_export::get_installation_identity(
            &mut tx.tx,
            org_id,
            connection.installation_id,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "connection_installation_missing: connection {} has no installed manifest",
                connection.label
            ))
        })?;
        let vendor = vendor_for_manifest(&name, &slug).ok_or_else(|| {
            ApiError::Validation(format!(
                "unsupported_connector_for_export: connection {} is `{name}`, which is not a \
                 QuickBooks or Xero accounting connector",
                connection.label
            ))
        })?;
        tx.commit().await?;
        (connection, vendor)
    };

    // 2. Claim the run and COMMIT before the vendor call. A claim that rolls
    //    back with the request would let two retries push the same invoice.
    let claim = {
        let mut tx = org_tx::begin(state, org_id, user_id).await?;
        let outcome = core_db::accounting_export::claim_run(
            &mut tx.tx,
            org_id,
            &ClaimSpec {
                connection_id,
                vendor,
                record_kind: doc.kind,
                record_id: doc.id,
                tax_code: doc.tax_code.as_deref(),
                requested_by: user_id,
            },
            RECLAIM_AFTER_SECS,
        )
        .await?;
        tx.commit().await?;
        outcome
    };
    let mut run = match claim {
        ClaimOutcome::AlreadySucceeded(run, receipt) => {
            return Ok(run_view(*run, Some(*receipt)));
        }
        ClaimOutcome::InFlight(run) => {
            return Err(ApiError::Conflict(format!(
                "export_in_flight: run {} is already sending this document (attempt {}, claimed \
                 {}); wait for it or retry after {} seconds",
                run.id,
                run.attempts,
                run.claimed_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                RECLAIM_AFTER_SECS
            )));
        }
        ClaimOutcome::Claimed(run) => *run,
    };

    // 3. A credit note must name its parent invoice's vendor id. No receipt on
    //    this connection means the invoice was never exported here — a real
    //    state, held rather than exported as an orphan.
    if doc.kind == RecordKind::CreditNote {
        let parent_id = doc.parent_invoice_id.ok_or_else(|| {
            ApiError::Validation("credit_note_has_no_invoice: no parent invoice".into())
        })?;
        let parent = {
            let mut tx = org_tx::begin(state, org_id, user_id).await?;
            let found = core_db::accounting_export::get_receipt(
                &mut tx.tx,
                org_id,
                connection_id,
                RecordKind::Invoice,
                parent_id,
            )
            .await?;
            tx.commit().await?;
            found
        };
        match parent {
            Some(receipt) => doc.parent_invoice_external_id = Some(receipt.external_id),
            None => {
                let held = settle_held(
                    state,
                    org_id,
                    user_id,
                    run.id,
                    "parent_invoice_not_exported",
                    Some(
                        "the invoice this credit note references has no vendor record on this \
                         connection; export the invoice first",
                    ),
                    None,
                )
                .await?;
                return Ok(held);
            }
        }
    }

    // 4. Tax resolution: mapping, then the vendor's own list. Neither miss is a
    //    failure — both are a decision a person has to make.
    if let Some(marula_code) = doc.tax_code.clone() {
        let mapping = {
            let mut tx = org_tx::begin(state, org_id, user_id).await?;
            let found = core_db::accounting_export::find_tax_mapping(
                &mut tx.tx,
                org_id,
                connection_id,
                &marula_code,
            )
            .await?;
            tx.commit().await?;
            found
        };
        let Some(mapping) = mapping else {
            let held = settle_held(
                state,
                org_id,
                user_id,
                run.id,
                "tax_code_unmapped",
                Some(&format!(
                    "tax code `{marula_code}` has no vendor mapping on connection {}; add one and \
                     retry — nothing was sent and the invoice is not re-issued",
                    connection.label
                )),
                None,
            )
            .await?;
            return Ok(held);
        };
        run = settled_tax_code(state, org_id, user_id, run.id, &mapping).await?;
        doc.tax_code = Some(mapping.vendor_tax_code.clone());
    }

    // 5. Build the client and, when a tax code is in play, confirm the books
    //    still carry it.
    let sink = {
        let mut tx = org_tx::begin(state, org_id, user_id).await?;
        let built = build_sink(
            state,
            &mut tx.tx,
            org_id,
            connection_id,
            &connection,
            vendor,
        )
        .await?;
        tx.commit().await?;
        built
    };

    if let Some(vendor_code) = doc.tax_code.clone() {
        match sink.read_tax_codes().await {
            Ok(codes) => {
                let known = codes.iter().any(|c| c.code == vendor_code && c.active);
                if !known {
                    let held = settle_held(
                        state,
                        org_id,
                        user_id,
                        run.id,
                        "tax_code_unknown_to_vendor",
                        Some(&format!(
                            "the mapping names `{vendor_code}`, which connection {} does not list \
                             as an active tax code; fix the mapping (or the books) and retry",
                            connection.label
                        )),
                        Some(&vendor_code),
                    )
                    .await?;
                    return Ok(held);
                }
            }
            Err(e) => {
                return settle_failed(state, org_id, user_id, run.id, e, Some(connection_id)).await;
            }
        }
    }

    // 6. A retry asks the vendor whether a previous attempt landed before it
    //    sends anything. Neither vendor takes an idempotency key, so this is the
    //    only way a crash between the call and the receipt can be resolved
    //    without a second document.
    if run.attempts > 1 {
        match sink.find_document(&doc).await {
            Ok(Some(found)) => {
                return settle_succeeded(
                    state,
                    org_id,
                    user_id,
                    run.id,
                    connection_id,
                    doc.kind,
                    doc.id,
                    &found,
                )
                .await;
            }
            Ok(None) => {}
            Err(e) => {
                return settle_failed(state, org_id, user_id, run.id, e, Some(connection_id)).await;
            }
        }
    }

    // 7. The vendor customer id, then the call.
    let customer = {
        let mut tx = org_tx::begin(state, org_id, user_id).await?;
        let found = customer_ref(&mut tx.tx, org_id, connection_id, doc.client_id).await?;
        tx.commit().await?;
        found
    };

    let pushed = match doc.kind {
        RecordKind::CreditNote => sink.push_credit_note(&doc, &customer).await,
        RecordKind::Invoice | RecordKind::Payment => sink.push_invoice(&doc, &customer).await,
    };
    match pushed {
        Ok(pushed) => {
            settle_succeeded(
                state,
                org_id,
                user_id,
                run.id,
                connection_id,
                doc.kind,
                doc.id,
                &pushed,
            )
            .await
        }
        Err(e) => settle_failed(state, org_id, user_id, run.id, e, Some(connection_id)).await,
    }
}

/// Writes the receipt and the terminal state in one transaction, plus the audit
/// row and the connection's health. The audit row is inside the same
/// transaction on purpose: an export that a client can see must be one an
/// auditor can see.
#[allow(clippy::too_many_arguments)]
async fn settle_succeeded(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    run_id: Uuid,
    connection_id: Uuid,
    kind: RecordKind,
    record_id: Uuid,
    pushed: &Pushed,
) -> ApiResult<ExportRunView> {
    let mut tx = org_tx::begin(state, org_id, user_id).await?;
    let (run, receipt) = core_db::accounting_export::finish_succeeded(
        &mut tx.tx,
        org_id,
        run_id,
        &core_db::accounting_export::NewReceipt {
            connection_id,
            record_kind: kind,
            record_id,
            external_id: &pushed.external_id,
            external_number: pushed.external_number.as_deref(),
            created_by: Some(user_id),
        },
    )
    .await?;
    core_db::connector_connections::record_connection_outcome(
        &mut tx.tx,
        org_id,
        connection_id,
        core_domain::connectors::ConnectionStatus::Connected,
        None,
        None,
        true,
    )
    .await?;
    let audit = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "accounting.export_succeeded".to_string(),
        entity_type: Some(kind.as_str().to_string()),
        entity_id: Some(record_id.to_string()),
        detail: serde_json::json!({
            "run_id": run_id,
            "connection_id": connection_id,
            "vendor": run.vendor.as_str(),
            "external_id": pushed.external_id,
            "adopted": !pushed.created,
        }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    core_db::audit::insert(&mut *tx.tx, &audit).await?;
    tx.commit().await?;
    Ok(run_view(run, Some(receipt)))
}

/// Ends the run in `failed` and records the connection's health when the fault
/// is the credential's (a token an operator has to rotate) rather than the
/// network's (which is nobody's decision).
async fn settle_failed(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    run_id: Uuid,
    error: AdapterError,
    connection_id: Option<Uuid>,
) -> ApiResult<ExportRunView> {
    let mut tx = org_tx::begin(state, org_id, user_id).await?;
    let run = core_db::accounting_export::finish_failed(
        &mut tx.tx,
        org_id,
        run_id,
        error.reason,
        error.detail.as_deref(),
    )
    .await?;
    if error.reason == "auth_failed" {
        if let Some(connection_id) = connection_id {
            core_db::connector_connections::record_connection_outcome(
                &mut tx.tx,
                org_id,
                connection_id,
                core_domain::connectors::ConnectionStatus::AuthFailed,
                Some("auth_failed"),
                error.detail.as_deref(),
                false,
            )
            .await?;
        }
    }
    let audit = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "accounting.export_failed".to_string(),
        entity_type: Some(run.record_kind.as_str().to_string()),
        entity_id: Some(run.record_id.to_string()),
        detail: serde_json::json!({
            "run_id": run_id,
            "connection_id": run.connection_id,
            "reason": error.reason,
            "retryable": error.retryable,
        }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    core_db::audit::insert(&mut *tx.tx, &audit).await?;
    tx.commit().await?;
    Ok(run_view(run, None))
}

/// Ends the run in `held`: nothing was sent, a person has a decision to make.
async fn settle_held(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    run_id: Uuid,
    reason: &str,
    detail: Option<&str>,
    vendor_tax_code: Option<&str>,
) -> ApiResult<ExportRunView> {
    let mut tx = org_tx::begin(state, org_id, user_id).await?;
    let run = core_db::accounting_export::mark_held(
        &mut tx.tx,
        org_id,
        run_id,
        reason,
        detail,
        vendor_tax_code,
    )
    .await?;
    let audit = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "accounting.export_held".to_string(),
        entity_type: Some(run.record_kind.as_str().to_string()),
        entity_id: Some(run.record_id.to_string()),
        detail: serde_json::json!({
            "run_id": run_id,
            "connection_id": run.connection_id,
            "reason": reason,
        }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    core_db::audit::insert(&mut *tx.tx, &audit).await?;
    tx.commit().await?;
    Ok(run_view(run, None))
}

/// Records the vendor code the mapping resolved to on the run, so the report
/// names it even if the send then fails.
async fn settled_tax_code(
    state: &AppState,
    org_id: Uuid,
    user_id: Uuid,
    run_id: Uuid,
    mapping: &TaxMapping,
) -> ApiResult<ExportRun> {
    let mut tx = org_tx::begin(state, org_id, user_id).await?;
    let run = core_db::accounting_export::set_run_vendor_tax_code(
        &mut tx.tx,
        org_id,
        run_id,
        &mapping.vendor_tax_code,
    )
    .await?;
    tx.commit().await?;
    Ok(run)
}

fn run_view(run: ExportRun, receipt: Option<ExportReceiptRow>) -> ExportRunView {
    ExportRunView {
        id: run.id,
        connection_id: run.connection_id,
        connection_label: run.connection_label,
        vendor: run.vendor.as_str().to_string(),
        record_kind: run.record_kind.as_str().to_string(),
        record_id: run.record_id,
        state: run.state.as_str().to_string(),
        reason: run.reason,
        reason_detail: run.reason_detail,
        tax_code: run.tax_code,
        vendor_tax_code: run.vendor_tax_code,
        attempts: run.attempts,
        claimed_at: run.claimed_at,
        completed_at: run.completed_at,
        created_at: run.created_at,
        updated_at: run.updated_at,
        receipt: receipt.map(|r| ExportReceiptView {
            external_id: r.external_id,
            external_number: r.external_number,
            created_at: r.created_at,
        }),
    }
}

// --- handlers -----------------------------------------------------------------

/// POST /api/invoices/{id}/export — send one invoice to one connection.
///
/// Re-posting is how a held or failed run is retried: the run is unique per
/// (connection, invoice), so the second call advances the first run instead of
/// issuing a second vendor document.
#[utoipa::path(
    operation_id = "invoices_export",
    post,
    path = "/api/invoices/{id}/export",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ExportRequest,
    responses(
        (status = 200, body = ExportRunView),
        (status = 404, description = "invoice or connection not visible in scope"),
        (status = 409, description = "another attempt is in flight, or the connection is disabled"),
        (status = 422, description = "unsupported connector, missing credential, or unlinked client")
    )
)]
pub async fn export_invoice(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ExportRequest>,
) -> ApiResult<Json<ExportRunView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let doc = {
        let mut tx = org_tx::begin(&state, org_id, user_id).await?;
        let doc = load_invoice(&mut tx.tx, org_id, id, req.tax_code).await?;
        tx.commit().await?;
        doc
    };
    let view = run_export(&state, org_id, user_id, req.connection_id, doc).await?;
    Ok(Json(view))
}

/// POST /api/credit-notes/{id}/export — send one credit note to the connection
/// that already holds its parent invoice.
///
/// The parent invoice's vendor id comes from the stored receipt; a credit note
/// whose invoice was never exported is HELD with `parent_invoice_not_exported`
/// rather than exported as a standalone credit.
#[utoipa::path(
    operation_id = "credit_notes_export",
    post,
    path = "/api/credit-notes/{id}/export",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ExportRequest,
    responses(
        (status = 200, body = ExportRunView),
        (status = 404, description = "credit note or connection not visible in scope"),
        (status = 409, description = "another attempt is in flight, or the credit note is void"),
        (status = 422, description = "free-standing credit, unsupported connector, missing credential")
    )
)]
pub async fn export_credit_note(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<ExportRequest>,
) -> ApiResult<Json<ExportRunView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let doc = {
        let mut tx = org_tx::begin(&state, org_id, user_id).await?;
        let doc = load_credit_note(&mut tx.tx, org_id, id, req.tax_code).await?;
        tx.commit().await?;
        doc
    };
    let view = run_export(&state, org_id, user_id, req.connection_id, doc).await?;
    Ok(Json(view))
}

/// GET /api/accounting/export-runs — the run report.
///
/// `state=held` and `state=failed` are different queues on purpose: a held run
/// is waiting on a person (a mapping), a failed run is waiting on a decision
/// (retry or not). The filter exists so a UI never has to guess which.
#[utoipa::path(
    operation_id = "accounting_export_runs_list",
    get,
    path = "/api/accounting/export-runs",
    tag = "billing",
    security(("bearer" = [])),
    params(RunListQuery),
    responses((status = 200, body = [ExportRunView]))
)]
pub async fn list_runs(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<RunListQuery>,
) -> ApiResult<Json<Vec<ExportRunView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let state_filter = match q.state.as_deref() {
        None => None,
        Some(raw) => Some(ExportState::parse(raw).ok_or_else(|| {
            ApiError::Validation(format!(
                "invalid state `{raw}`; expected in_flight, succeeded, failed or held"
            ))
        })?),
    };
    let kind_filter = match q.record_kind.as_deref() {
        None => None,
        Some(raw) => Some(RecordKind::parse(raw).ok_or_else(|| {
            ApiError::Validation(format!(
                "invalid record_kind `{raw}`; expected invoice or credit_note"
            ))
        })?),
    };

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let runs = core_db::accounting_export::list_runs(
        &mut tx.tx,
        org_id,
        &RunFilters {
            connection_id: q.connection_id,
            state: state_filter,
            record_kind: kind_filter,
            record_id: q.record_id,
        },
        q.limit.unwrap_or(50).clamp(1, MAX_RUNS),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(runs.into_iter().map(|r| run_view(r, None)).collect()))
}

/// GET /api/accounting/export-runs/{id} — one run, with its receipt.
#[utoipa::path(
    operation_id = "accounting_export_runs_get",
    get,
    path = "/api/accounting/export-runs/{id}",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = ExportRunView),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn get_run(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ExportRunView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let run = core_db::accounting_export::get_run(&mut tx.tx, org_id, id).await?;
    let receipt = core_db::accounting_export::get_receipt(
        &mut tx.tx,
        org_id,
        run.connection_id,
        run.record_kind,
        run.record_id,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(run_view(run, receipt)))
}

/// GET /api/accounting/connections/{id}/ledger — the customer's chart of
/// accounts and tax codes (claims-gap G2).
///
/// This is the read leg that makes a mapping a choice instead of a guess: the
/// UI builds its pick list from `accounts`, and `tax_codes` is the same list the
/// export validates a mapping against.
#[utoipa::path(
    operation_id = "accounting_ledger_read",
    get,
    path = "/api/accounting/connections/{id}/ledger",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = LedgerView),
        (status = 404, description = "connection not visible in scope"),
        (status = 503, description = "the vendor could not be read")
    )
)]
pub async fn read_ledger(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LedgerView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let (connection, vendor, sink) = {
        let mut tx = org_tx::begin(&state, org_id, user_id).await?;
        let connection = core_db::connector_connections::get_connection(&mut tx.tx, org_id, id)
            .await?
            .ok_or(ApiError::NotFound)?;
        let (name, slug) = core_db::accounting_export::get_installation_identity(
            &mut tx.tx,
            org_id,
            connection.installation_id,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "connection_installation_missing: connection {} has no installed manifest",
                connection.label
            ))
        })?;
        let vendor = vendor_for_manifest(&name, &slug).ok_or_else(|| {
            ApiError::Validation(format!(
                "unsupported_connector_for_export: connection {} is `{name}`, which is not a \
                 QuickBooks or Xero accounting connector",
                connection.label
            ))
        })?;
        let sink = build_sink(&state, &mut tx.tx, org_id, id, &connection, vendor).await?;
        tx.commit().await?;
        (connection, vendor, sink)
    };

    let (_, accounts, tax_codes) = sink.read_ledger().await.map_err(|e| {
        ApiError::Unavailable(format!(
            "{}: the books could not be read from connection {}",
            e.reason, connection.label
        ))
    })?;
    Ok(Json(LedgerView {
        connection_id: id,
        vendor: vendor.as_str().to_string(),
        accounts,
        tax_codes,
    }))
}

/// GET /api/accounting/tax-mappings — the connection's tax mappings.
#[utoipa::path(
    operation_id = "accounting_tax_mappings_list",
    get,
    path = "/api/accounting/tax-mappings",
    tag = "billing",
    security(("bearer" = [])),
    params(TaxMappingListQuery),
    responses((status = 200, body = [TaxMappingView]))
)]
pub async fn list_tax_mappings(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TaxMappingListQuery>,
) -> ApiResult<Json<Vec<TaxMappingView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let mappings =
        core_db::accounting_export::list_tax_mappings(&mut tx.tx, org_id, q.connection_id).await?;
    tx.commit().await?;
    Ok(Json(mappings.into_iter().map(mapping_view).collect()))
}

/// PUT /api/accounting/tax-mappings — write (or rewrite) one mapping.
///
/// The vendor code is checked against the connection's own list first, so a
/// typo is a refusal now rather than a held invoice later. That check is the
/// read leg (G2) doing its job; a mapping that names a code the books do not
/// have is exactly the guess G3 forbids.
#[utoipa::path(
    operation_id = "accounting_tax_mappings_upsert",
    put,
    path = "/api/accounting/tax-mappings",
    tag = "billing",
    security(("bearer" = [])),
    request_body = UpsertTaxMappingRequest,
    responses(
        (status = 200, body = TaxMappingView),
        (status = 404, description = "connection not visible in scope"),
        (status = 422, description = "the vendor code is not in the connection's books"),
        (status = 503, description = "the vendor could not be read to check the code")
    )
)]
pub async fn upsert_tax_mapping(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<UpsertTaxMappingRequest>,
) -> ApiResult<Json<TaxMappingView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let marula_code = req.marula_tax_code.trim();
    let vendor_code = req.vendor_tax_code.trim();
    if marula_code.is_empty() || vendor_code.is_empty() {
        return Err(ApiError::Validation(
            "marula_tax_code and vendor_tax_code are both required".into(),
        ));
    }

    let (connection, vendor, sink) = {
        let mut tx = org_tx::begin(&state, org_id, user_id).await?;
        let connection =
            core_db::connector_connections::get_connection(&mut tx.tx, org_id, req.connection_id)
                .await?
                .ok_or(ApiError::NotFound)?;
        let (name, slug) = core_db::accounting_export::get_installation_identity(
            &mut tx.tx,
            org_id,
            connection.installation_id,
        )
        .await?
        .ok_or_else(|| {
            ApiError::Conflict(format!(
                "connection_installation_missing: connection {} has no installed manifest",
                connection.label
            ))
        })?;
        let vendor = vendor_for_manifest(&name, &slug).ok_or_else(|| {
            ApiError::Validation(format!(
                "unsupported_connector_for_export: connection {} is `{name}`, which is not a \
                 QuickBooks or Xero accounting connector",
                connection.label
            ))
        })?;
        let sink = build_sink(
            &state,
            &mut tx.tx,
            org_id,
            req.connection_id,
            &connection,
            vendor,
        )
        .await?;
        tx.commit().await?;
        (connection, vendor, sink)
    };

    let tax_codes = sink.read_tax_codes().await.map_err(|e| {
        ApiError::Unavailable(format!(
            "{}: the books could not be read from connection {}",
            e.reason, connection.label
        ))
    })?;
    let matched = tax_codes
        .iter()
        .find(|c| c.code == vendor_code)
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "vendor_tax_code_not_in_books: connection {} lists no tax code `{vendor_code}`",
                connection.label
            ))
        })?;
    if !matched.active {
        return Err(ApiError::Validation(format!(
            "vendor_tax_code_inactive: `{vendor_code}` is not active on connection {}",
            connection.label
        )));
    }
    let name = req
        .vendor_tax_code_name
        .clone()
        .or_else(|| Some(matched.name.clone()));

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let mapping = core_db::accounting_export::upsert_tax_mapping(
        &mut tx.tx,
        org_id,
        req.connection_id,
        marula_code,
        vendor_code,
        name.as_deref(),
        Some(user_id),
    )
    .await?;
    let audit = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "accounting.tax_mapping_written".to_string(),
        entity_type: Some("accounting_tax_mapping".to_string()),
        entity_id: Some(mapping.id.to_string()),
        detail: serde_json::json!({
            "connection_id": req.connection_id,
            "vendor": vendor.as_str(),
            "marula_tax_code": mapping.marula_tax_code,
            "vendor_tax_code": mapping.vendor_tax_code,
        }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    core_db::audit::insert(&mut *tx.tx, &audit).await?;
    tx.commit().await?;
    Ok(Json(mapping_view(mapping)))
}

/// DELETE /api/accounting/tax-mappings/{id} — remove a mapping.
///
/// Removing one holds the next export that needs it, which is correct: an
/// export with no mapping must not silently fall back to a vendor default.
#[utoipa::path(
    operation_id = "accounting_tax_mappings_delete",
    delete,
    path = "/api/accounting/tax-mappings/{id}",
    tag = "billing",
    security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "removed"),
        (status = 404, description = "not visible in scope")
    )
)]
pub async fn delete_tax_mapping(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<axum::http::StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = org_tx::begin(&state, org_id, user_id).await?;
    let removed = core_db::accounting_export::delete_tax_mapping(&mut tx.tx, org_id, id).await?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    let audit = AuditEvent {
        id: Uuid::now_v7(),
        org_id,
        actor: Actor::Human(user_id),
        action: "accounting.tax_mapping_deleted".to_string(),
        entity_type: Some("accounting_tax_mapping".to_string()),
        entity_id: Some(id.to_string()),
        detail: serde_json::json!({}),
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    core_db::audit::insert(&mut *tx.tx, &audit).await?;
    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn mapping_view(m: TaxMapping) -> TaxMappingView {
    TaxMappingView {
        id: m.id,
        connection_id: m.connection_id,
        marula_tax_code: m.marula_tax_code,
        vendor_tax_code: m.vendor_tax_code,
        vendor_tax_code_name: m.vendor_tax_code_name,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0032: the vendor comes from the manifest, and a connector that is
    /// not an accounting connector is refused rather than guessed at.
    #[test]
    fn vendor_is_read_from_the_manifest_and_unknown_is_none() {
        assert_eq!(
            vendor_for_manifest("QuickBooks Online", "qbo"),
            Some(ExportVendor::Qbo)
        );
        assert_eq!(
            vendor_for_manifest("Xero", "xero-export-sync"),
            Some(ExportVendor::Xero)
        );
        assert_eq!(
            vendor_for_manifest("QBO Export Sync", ""),
            Some(ExportVendor::Qbo)
        );
        assert_eq!(vendor_for_manifest("UniFi", "unifi"), None);
        assert_eq!(vendor_for_manifest("", ""), None);
    }

    /// The reason vocabulary is one list across both vendors: a report that said
    /// `qbo_401` in one row and `xero_auth` in the next would be two reports.
    #[test]
    fn both_adapters_map_onto_one_reason_vocabulary() {
        use qbo_export_sync::ExportError as Q;
        use xero_export_sync::ExportError as X;
        assert_eq!(qbo_reason(&Q::Disabled), xero_reason(&X::Disabled));
        assert_eq!(
            qbo_reason(&Q::Unauthorized("x".into())),
            xero_reason(&X::Unauthorized("x".into()))
        );
        assert_eq!(
            qbo_reason(&Q::RateLimited("x".into())),
            xero_reason(&X::RateLimited("x".into()))
        );
        assert_eq!(
            qbo_reason(&Q::Vendor {
                status: 400,
                message: "m".into()
            }),
            "vendor_rejected"
        );
        assert_eq!(
            qbo_reason(&Q::Vendor {
                status: 502,
                message: "m".into()
            }),
            "vendor_unavailable",
            "a 5xx is the vendor's outage, not our request's fault"
        );
        assert_eq!(
            qbo_reason(&Q::Network("dns".into())),
            xero_reason(&X::Network("dns".into()))
        );
    }
}

// --- accountant package ------------------------------------------------------

/// One class and the account it posts to.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClassAccountView {
    pub reporting_class: String,
    pub account_code: String,
    pub account_name: String,
}

/// PUT /accounting/class-accounts body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetClassAccountRequest {
    pub reporting_class: String,
    pub account_code: String,
    pub account_name: String,
}

/// One package line, with the account it resolves to.
#[derive(Debug, Serialize, ToSchema)]
pub struct PackageRecordView {
    /// invoice | credit_note | acquisition_cost | rental_charge | vendor_bill |
    /// stock_item | ewaste_disposition
    pub kind: String,
    /// `kind:id` — the trail from an exported row back to one Marula record.
    pub stable_ref: String,
    pub document_ref: Option<String>,
    #[schema(value_type = String)]
    pub occurred_on: NaiveDate,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    pub currency: String,
    pub amount_minor: i64,
    pub base_amount_minor: Option<i64>,
    pub reporting_class: Option<String>,
    pub account_code: String,
    pub memo: Option<String>,
}

/// A total per account, in the org base currency where a row has one.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountTotalView {
    pub account_code: String,
    pub account_name: String,
    pub base_total_minor: i64,
}

/// A total per record kind.
#[derive(Debug, Serialize, ToSchema)]
pub struct KindTotalView {
    pub kind: String,
    pub rows: i64,
    pub base_total_minor: i64,
}

/// The accountant package: one period, every money row, every class resolved to
/// an account, and the stable reference that names each row.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountingPackageView {
    pub generated_at: DateTime<Utc>,
    #[schema(value_type = String)]
    pub period_from: NaiveDate,
    #[schema(value_type = String)]
    pub period_to: NaiveDate,
    pub base_currency: String,
    pub class_accounts: Vec<ClassAccountView>,
    pub records: Vec<PackageRecordView>,
    /// Rows whose amount is not in the base currency and carries no locked
    /// snapshot. They are listed in `records` and left out of the totals, so a
    /// total is never a sum of mixed currencies.
    pub unconverted_rows: i64,
    pub totals_by_account: Vec<AccountTotalView>,
    pub totals_by_kind: Vec<KindTotalView>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PackageQuery {
    #[param(value_type = String)]
    pub from: NaiveDate,
    #[param(value_type = String)]
    pub to: NaiveDate,
}

/// GET /accounting/class-accounts — every class the org posts to, defaults
/// filled in for the ones it has not chosen.
#[utoipa::path(operation_id = "accounting_class_accounts_list", get, path = "/api/accounting/class-accounts", tag = "accounting", security(("bearer" = [])),
    responses((status = 200, body = [ClassAccountView])))]
pub async fn list_class_accounts(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<ClassAccountView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::View,
        },
    )?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        core_db::accounting_export::class_accounts(&mut tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ClassAccountView {
                reporting_class: r.reporting_class,
                account_code: r.account_code,
                account_name: r.account_name,
            })
            .collect(),
    ))
}

/// PUT /accounting/class-accounts — choose the account one class posts to.
#[utoipa::path(operation_id = "accounting_class_accounts_set", put, path = "/api/accounting/class-accounts", tag = "accounting", security(("bearer" = [])),
    request_body = SetClassAccountRequest,
    responses((status = 200, body = ClassAccountView),
              (status = 422, description = "class or account is empty")))]
pub async fn set_class_account(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SetClassAccountRequest>,
) -> ApiResult<Json<ClassAccountView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::Edit,
        },
    )?;
    let class = req.reporting_class.trim().to_lowercase();
    let code = req.account_code.trim().to_string();
    let name = req.account_name.trim().to_string();
    if class.is_empty() || code.is_empty() || name.is_empty() {
        return Err(ApiError::Validation(
            "a class, an account code and an account name are all required".into(),
        ));
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let saved = core_db::accounting_export::upsert_class_account(
        &mut tx,
        authed.principal.user.org_id,
        &class,
        &code,
        &name,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ClassAccountView {
        reporting_class: saved.reporting_class,
        account_code: saved.account_code,
        account_name: saved.account_name,
    }))
}

/// GET /accounting/package — the period's package, ready to hand an accountant.
#[utoipa::path(operation_id = "accounting_package", get, path = "/api/accounting/package", tag = "accounting", security(("bearer" = [])),
    params(PackageQuery),
    responses((status = 200, body = AccountingPackageView),
              (status = 409, description = "a reporting class in the period has no account mapped"),
              (status = 422, description = "the period is inverted")))]
pub async fn accounting_package(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PackageQuery>,
) -> ApiResult<Json<AccountingPackageView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    Ok(Json(build_package(&state, &authed.principal, &q).await?))
}

/// GET /accounting/package.csv — the same period and the same rows, as CSV.
#[utoipa::path(operation_id = "accounting_package_csv", get, path = "/api/accounting/package.csv", tag = "accounting", security(("bearer" = [])),
    params(PackageQuery),
    responses((status = 200, body = String, content_type = "text/csv"),
              (status = 409, description = "a reporting class in the period has no account mapped"),
              (status = 422, description = "the period is inverted")))]
pub async fn accounting_package_csv(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<PackageQuery>,
) -> ApiResult<([(&'static str, &'static str); 1], String)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let package = build_package(&state, &authed.principal, &q).await?;
    Ok((
        [("content-type", "text/csv; charset=utf-8")],
        package_csv(&package),
    ))
}

/// The period's package from the database, shared by the JSON and CSV forms so
/// the hold and the totals cannot drift between them.
async fn build_package(
    state: &AppState,
    principal: &core_domain::entities::Principal,
    q: &PackageQuery,
) -> ApiResult<AccountingPackageView> {
    require_permission(
        principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::View,
        },
    )?;
    // The rows carry bill amounts and stock values, which the source surfaces
    // put behind financial read. The package asks for the same authority, so
    // commercial view alone cannot reach money it could not read elsewhere.
    require_permission(principal, Permission::ViewFinancials)?;
    if q.to < q.from {
        return Err(ApiError::Validation(
            "the period ends before it starts".into(),
        ));
    }
    let org_id = principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let accounts = core_db::accounting_export::class_accounts(&mut tx, org_id).await?;
    let records =
        core_db::accounting_export::package_records(&mut tx, org_id, q.from, q.to).await?;
    let base_currency = core_db::fx::base_currency(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let account_of = |class: Option<&str>| -> Option<(String, String)> {
        let wanted = class.unwrap_or("other");
        accounts
            .iter()
            .find(|a| a.reporting_class == wanted)
            .map(|a| (a.account_code.clone(), a.account_name.clone()))
    };

    // Holding is the point. A package with an unmapped class used to fall back
    // to `other`/5900, which put a real period on an account nobody chose. The
    // rows stay in the database; the handoff names every unmapped class and
    // refuses until the org maps it.
    let mut missing: Vec<String> = records
        .iter()
        .map(|r| r.reporting_class.as_deref().unwrap_or("other"))
        .filter(|class| account_of(Some(class)).is_none())
        .map(str::to_string)
        .collect();
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        return Err(ApiError::AccountMappingMissing { classes: missing });
    }

    let mut views: Vec<PackageRecordView> = Vec::with_capacity(records.len());
    let mut by_account: std::collections::BTreeMap<String, (String, i64)> = Default::default();
    let mut by_kind: std::collections::BTreeMap<String, (i64, i64)> = Default::default();
    let mut unconverted_rows: i64 = 0;
    for r in &records {
        let class = r.reporting_class.as_deref().unwrap_or("other");
        // The hold above proves every class resolves. If a later change broke
        // that, answer with the same typed refusal rather than panicking or
        // dropping the row.
        let Some((account_code, account_name)) = account_of(Some(class)) else {
            return Err(ApiError::AccountMappingMissing {
                classes: vec![class.to_string()],
            });
        };

        // A total is only meaningful in one currency: take the locked base
        // amount, or the amount itself when the row is already in base. A
        // foreign row with no snapshot stays in `records` and is counted, not
        // added.
        let money = match (r.base_amount_minor, r.currency == base_currency) {
            (Some(base), _) => Some(base),
            (None, true) => Some(r.amount_minor),
            (None, false) => {
                unconverted_rows += 1;
                None
            }
        };
        if let Some(money) = money {
            let entry = by_account
                .entry(account_code.clone())
                .or_insert((account_name, 0));
            entry.1 = entry.1.saturating_add(money);
            let kind = by_kind.entry(r.kind.clone()).or_insert((0, 0));
            kind.0 += 1;
            kind.1 = kind.1.saturating_add(money);
        }

        views.push(PackageRecordView {
            kind: r.kind.clone(),
            stable_ref: r.stable_ref.clone(),
            document_ref: r.document_ref.clone(),
            occurred_on: r.occurred_on,
            client_id: r.client_id,
            currency: r.currency.clone(),
            amount_minor: r.amount_minor,
            base_amount_minor: r.base_amount_minor,
            reporting_class: r.reporting_class.clone(),
            account_code,
            memo: r.memo.clone(),
        });
    }

    Ok(AccountingPackageView {
        generated_at: Utc::now(),
        period_from: q.from,
        period_to: q.to,
        base_currency,
        class_accounts: accounts
            .into_iter()
            .map(|a| ClassAccountView {
                reporting_class: a.reporting_class,
                account_code: a.account_code,
                account_name: a.account_name,
            })
            .collect(),
        records: views,
        unconverted_rows,
        totals_by_account: by_account
            .into_iter()
            .map(
                |(account_code, (account_name, base_total_minor))| AccountTotalView {
                    account_code,
                    account_name,
                    base_total_minor,
                },
            )
            .collect(),
        totals_by_kind: by_kind
            .into_iter()
            .map(|(kind, (rows, base_total_minor))| KindTotalView {
                kind,
                rows,
                base_total_minor,
            })
            .collect(),
    })
}

/// The same package as CSV: one header row, one line per record, with exactly
/// the fields the JSON form carries. Values stay in minor units and keep their
/// field names, so two exports of one period agree.
fn package_csv(package: &AccountingPackageView) -> String {
    let mut out = String::from(
        "kind,stable_ref,document_ref,occurred_on,client_id,currency,amount_minor,base_amount_minor,reporting_class,account_code,memo\n",
    );
    for r in &package.records {
        let fields = [
            r.kind.clone(),
            r.stable_ref.clone(),
            r.document_ref.clone().unwrap_or_default(),
            r.occurred_on.to_string(),
            r.client_id.map(|id| id.to_string()).unwrap_or_default(),
            r.currency.clone(),
            r.amount_minor.to_string(),
            r.base_amount_minor
                .map(|v| v.to_string())
                .unwrap_or_default(),
            r.reporting_class.clone().unwrap_or_default(),
            r.account_code.clone(),
            r.memo.clone().unwrap_or_default(),
        ];
        out.push_str(&fields.map(|f| csv_field(&f)).join(","));
        out.push('\n');
    }
    out
}

/// A CSV field, quoted only when it has to be. `memo` carries vendor prose and
/// can hold a comma, so quoting is not optional. A cell a spreadsheet would
/// evaluate is prefixed so it stays text; a negative number is not, because
/// the amount columns are meant to be numbers.
fn csv_field(value: &str) -> String {
    let dangerous = value.starts_with('=')
        || value.starts_with('+')
        || value.starts_with('@')
        || value.starts_with('\t')
        || value.starts_with('\r')
        || (value.starts_with('-') && value.parse::<f64>().is_err());
    if dangerous {
        format!("\"'{}\"", value.replace('"', "\"\""))
    } else if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod package_csv_tests {
    use super::csv_field;

    #[test]
    fn a_cell_a_spreadsheet_would_evaluate_stays_text() {
        assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_field("+1+1"), "\"'+1+1\"");
        assert_eq!(csv_field("@SUM(A1)"), "\"'@SUM(A1)\"");
        assert_eq!(csv_field("\tcmd"), "\"'\tcmd\"");
        assert_eq!(csv_field("-cmd"), "\"'-cmd\"");
    }

    #[test]
    fn numbers_and_plain_text_keep_their_shape() {
        assert_eq!(csv_field("-15000"), "-15000");
        assert_eq!(csv_field("1200"), "1200");
        assert_eq!(csv_field("Recycle Co"), "Recycle Co");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
