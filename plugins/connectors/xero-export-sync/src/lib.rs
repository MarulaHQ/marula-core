//! Xero export-sync adapter (the private build tracker Phase 3 adapters line).
//!
//! Purpose: push Marula invoices, payments and credit notes into a
//! customer-provided Xero tenant over the PUBLIC Xero accounting API
//! (`/api.xro/2.0/...`) — HTTP only, no vendor SDK. OAuth2 access tokens are
//! operator-supplied (BYOK: sealed at rest in the core's
//! `connector_connections.secrets_encrypted` document; Marula hosts no OAuth
//! app). Background sync scheduling is a future worker job — this crate is the
//! edge adapter only.
//!
//! Read leg (claims-gap G2): [`ExportSink::read_accounts`] and
//! [`ExportSink::read_tax_codes`] return the tenant's chart of accounts and tax
//! rates in a NEUTRAL shape — account kinds are the vendor-neutral buckets in
//! [`AccountKind`], not Xero's `CURRLIAB`/`OVERHEADS` strings. That is what
//! lets Core validate a tax mapping against the books instead of guessing
//! (G3), so the shape is defined here, in Core-facing terms.
//!
//! Credit notes (G1): [`ExportSink::push_credit_note`] posts a Xero
//! `CreditNote` (`ACCRECCREDIT`) and allocates it to the invoice exported
//! earlier, through `Allocations` (`Invoice.InvoiceID` = the vendor invoice id
//! from the stored [`ExportReceipt`]). An allocation is what reduces the
//! invoice; a credit note without one is credit the customer's books show
//! twice.
//!
//! Reconcile (the crash window): a process that dies between the vendor call
//! and the receipt write leaves no receipt, and neither Xero nor QBO takes an
//! idempotency key. [`ExportSink::find_document`] asks the tenant whether the
//! document number the first attempt sent exists, and confirms the
//! `[marula:<kind>:<uuid>]` marker this crate writes into `Reference`, so a
//! retry adopts the existing document instead of creating a twin.
//!
//! None mode: [`NoneExportSink`] is the shipped default — it performs zero
//! network I/O and returns [`ExportError::Disabled`], so an org with no
//! accounting credentials operates fully standalone (standalone-first
//! doctrine). Every call site must treat `Disabled` as "would have synced".
//!
//! Non-purpose: no OAuth token refresh (operators rotate the token in the
//! sealed store), no Xero contact/item catalog management, no reverse
//! (Xero→Marula) import.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

// --- domain types (deliberately vendor-neutral and self-contained) ----------

/// One invoice line Marula asks the accounting system to record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportLine {
    pub description: String,
    /// Quantity as a decimal string ("1", "2.5") — avoids float drift
    /// across vendors that use decimal strings (Xero) or numbers (QBO).
    pub quantity: String,
    /// Unit price in minor units (cents).
    pub unit_amount_minor: i64,
}

/// The invoice Marula exports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportInvoice {
    /// Marula invoice id (stable identity for idempotent re-pushes).
    pub invoice_id: Uuid,
    /// Human invoice number ("INV-000042") — surfaced as the vendor's
    /// external reference / invoice number where supported.
    pub number: String,
    /// The vendor's customer id (Xero `Contact.ContactID`). Export
    /// adapters never invent customers.
    pub customer_ref: String,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    /// ISO 4217 code ("USD") — Xero's `CurrencyCode`.
    pub currency: String,
    pub lines: Vec<ExportLine>,
    pub notes: Option<String>,
    /// The VENDOR tax code every line carries, already resolved by the caller
    /// through its mapping store (Xero's `TaxType`, e.g. `OUTPUT2`). `None`
    /// means the caller named none — this adapter never substitutes a
    /// default, because that is the guess claims-gap G3 exists to prevent.
    pub tax_code: Option<String>,
}

/// A payment received against a previously exported invoice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPayment {
    pub payment_id: Uuid,
    /// The invoice's VENDOR-side id (`ExportReceipt.external_id` from the
    /// earlier invoice push) — payments always attach to vendor records.
    pub invoice_external_id: String,
    /// Xero account code the payment is applied through (Xero requires
    /// `Account.Code` on Payments when no bank-transaction default exists).
    pub account_code: String,
    /// Payment date.
    pub applied_on: NaiveDate,
    /// Amount in minor units.
    pub amount_minor: i64,
    pub currency: String,
}

/// A credit note Marula issues against an invoice it already exported.
///
/// `invoice_external_id` is the Xero `InvoiceID` from the stored receipt of
/// the parent invoice, and it lands in `Allocations` — the field Xero applies
/// the credit through. There is no Xero side effect that applies a credit
/// implicitly, so a note without it is two documents for the same money.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportCreditNote {
    pub credit_note_id: Uuid,
    /// Human credit-note number ("CRN-000012") — the vendor's
    /// `CreditNoteNumber`, which Xero keeps unique per tenant.
    pub number: String,
    /// Vendor customer id (`Contact.ContactID`).
    pub customer_ref: String,
    /// The parent invoice's VENDOR id (`ExportReceipt.external_id` from the
    /// invoice export, same connection).
    pub invoice_external_id: String,
    pub issued_on: NaiveDate,
    pub currency: String,
    pub lines: Vec<ExportLine>,
    pub tax_code: Option<String>,
    pub reason: Option<String>,
}

/// What a successful push produced on the vendor side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportReceipt {
    /// The vendor's id for the created/updated record (Xero `InvoiceID` /
    /// `PaymentID`). Callers store this to attach payments and to make
    /// re-pushes idempotent.
    pub external_id: String,
    /// Vendor document number when the vendor assigns/echoes one.
    pub external_number: Option<String>,
    /// True when the vendor created a NEW record.
    pub created: bool,
}

/// Which document a [`ExportLookup`] is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportLookupKind {
    Invoice,
    CreditNote,
}

/// A reconcile question: did this run already create a vendor document?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportLookup {
    pub kind: ExportLookupKind,
    /// The Marula id this crate stamps into the vendor's `Reference` at
    /// creation (`[marula:invoice:<uuid>]`). The marker is the proof of
    /// ownership: a document number is unique per tenant in Xero, but
    /// confirming the marker is what makes the answer true for either vendor.
    pub marula_id: Uuid,
    /// The document number the earlier attempt would have sent.
    pub number: String,
}

/// The neutral classification of a ledger account.
///
/// Xero's `Type` vocabulary (`CURRLIAB`, `OVERHEADS`, `TERMLIAB`, …) and QBO's
/// `AccountType` strings are both vendor-specific. Core branches on this set,
/// so a mapping rule is written once. `Unknown` is honest rather than lossy: a
/// vendor adds an account type, it lands here, and Core leaves it out of a
/// pick list that claims to be complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Bank,
    AccountsReceivable,
    OtherCurrentAsset,
    FixedAsset,
    OtherAsset,
    AccountsPayable,
    CreditCard,
    OtherCurrentLiability,
    LongTermLiability,
    Equity,
    Income,
    CostOfGoodsSold,
    Expense,
    OtherIncome,
    OtherExpense,
    Unknown,
}

/// One account in the tenant's chart of accounts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerAccount {
    /// The vendor's id (Xero `AccountID`).
    pub id: String,
    /// The operator-facing account code (Xero `Code`), which is what a Xero
    /// line item carries.
    pub code: Option<String>,
    pub name: String,
    pub kind: AccountKind,
    /// False for an archived account: it is still returned (a historical line
    /// may reference it) but a pick list must not offer it.
    pub active: bool,
}

/// One tax rate in the tenant's books.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaxCode {
    /// The value a vendor invoice line carries (`TaxType`, e.g. `OUTPUT2`).
    /// This is what a mapping stores on the vendor side.
    pub code: String,
    pub name: String,
    /// Effective rate in basis points when the vendor states one (8.25% =
    /// 825). `None` for a rate the vendor did not publish.
    pub rate_bps: Option<i32>,
    pub active: bool,
}

/// Typed error surfaced across the export seam. Vendor detail is
/// stringified into `reason`s — no vendor SDK types leak past this crate.
///
/// The variants are the three answers a caller must be able to tell apart
/// (claims-gap G3's "held vs failed" needs exactly this): the credential is
/// wrong ([`ExportError::Unauthorized`]), the vendor asked us to slow down
/// ([`ExportError::RateLimited`]), or the request itself is unacceptable
/// ([`ExportError::Vendor`] with a 4xx status). [`ExportError::retryable`] is
/// the one place that split is encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// None mode (no credentials configured). Zero network was performed.
    #[error("export sink disabled: no credentials configured (none mode)")]
    Disabled,
    /// The caller omitted data the vendor requires.
    #[error("invalid export request: {0}")]
    InvalidRequest(String),
    /// The vendor rejected the credential (401/403) — token expired or
    /// revoked; operator rotates it in the sealed store.
    #[error("export unauthorized: {0}")]
    Unauthorized(String),
    /// The vendor asked us to back off (429).
    #[error("export rate limited: {0}")]
    RateLimited(String),
    /// Vendor-side rejection or outage. 4xx is a refusal of THIS request
    /// (retrying re-sends the same refusal); 5xx is the vendor's own fault
    /// and is retryable. The split is [`ExportError::retryable`].
    #[error("export vendor error (status {status}): {message}")]
    Vendor { status: u16, message: String },
    /// Transport failure (DNS, TLS, timeout). The message never carries the
    /// request URL: a base URL can hold credentials.
    #[error("export transport failure: {0}")]
    Network(String),
}

impl ExportError {
    /// True when re-sending the same request could succeed later.
    pub fn retryable(&self) -> bool {
        match self {
            ExportError::Disabled
            | ExportError::InvalidRequest(_)
            | ExportError::Unauthorized(_) => false,
            ExportError::RateLimited(_) | ExportError::Network(_) => true,
            ExportError::Vendor { status, .. } => *status >= 500,
        }
    }
}

// --- the seam ----------------------------------------------------------------

/// The vendor-neutral export seam. Every QBO/Xero-style accounting sink
/// implements this; none mode is [`NoneExportSink`].
#[async_trait]
pub trait ExportSink: Send + Sync {
    fn name(&self) -> &str;

    async fn push_invoice(&self, invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError>;

    async fn push_payment(&self, payment: &ExportPayment) -> Result<ExportReceipt, ExportError>;

    /// Post a credit note that adjusts a previously exported invoice.
    async fn push_credit_note(&self, note: &ExportCreditNote)
        -> Result<ExportReceipt, ExportError>;

    /// The tenant's chart of accounts, neutral-shaped (read leg, G2).
    async fn read_accounts(&self) -> Result<Vec<LedgerAccount>, ExportError>;

    /// The tenant's tax rates, neutral-shaped (read leg, G2).
    async fn read_tax_codes(&self) -> Result<Vec<TaxCode>, ExportError>;

    /// Did an earlier attempt already create this document? `None` means the
    /// vendor has no record carrying this Marula id, so a re-push is safe.
    async fn find_document(
        &self,
        lookup: &ExportLookup,
    ) -> Result<Option<ExportReceipt>, ExportError>;
}

/// Built-in none-mode sink: disabled, zero network, zero dependencies.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoneExportSink;

#[async_trait]
impl ExportSink for NoneExportSink {
    fn name(&self) -> &str {
        "none"
    }

    async fn push_invoice(&self, _invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError> {
        Err(ExportError::Disabled)
    }

    async fn push_payment(&self, _payment: &ExportPayment) -> Result<ExportReceipt, ExportError> {
        Err(ExportError::Disabled)
    }

    async fn push_credit_note(
        &self,
        _note: &ExportCreditNote,
    ) -> Result<ExportReceipt, ExportError> {
        Err(ExportError::Disabled)
    }

    async fn read_accounts(&self) -> Result<Vec<LedgerAccount>, ExportError> {
        Err(ExportError::Disabled)
    }

    async fn read_tax_codes(&self) -> Result<Vec<TaxCode>, ExportError> {
        Err(ExportError::Disabled)
    }

    async fn find_document(
        &self,
        _lookup: &ExportLookup,
    ) -> Result<Option<ExportReceipt>, ExportError> {
        // None mode never reached a vendor, so it can never have created a
        // document. `Disabled` rather than `Ok(None)`: a caller must not read
        // "nothing found" as permission to push.
        Err(ExportError::Disabled)
    }
}

// --- Xero reference implementation ---------------------------------------------

/// Credentials + endpoint configuration for [`XeroClient`]. Values come
/// from the sealed store (decrypted just-in-time by the caller, never
/// logged, never serialized).
#[derive(Clone)]
pub struct XeroConfig {
    /// Xero OAuth2 access token with `accounting.transactions` scope.
    pub access_token: String,
    /// The target tenant's id (`Xero-tenant-id` header).
    pub tenant_id: String,
    /// API base — overridable for tests; production default is the public
    /// API host.
    pub base_url: String,
    /// Wall-clock budget for ONE vendor request, connect included. Bounded
    /// because an export runs inside a request handler.
    pub timeout: Duration,
}

/// Debug prints the tenant and the host, never the token.
impl std::fmt::Debug for XeroConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XeroConfig")
            .field("access_token", &"<redacted>")
            .field("tenant_id", &self.tenant_id)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Connect timeout, deliberately shorter than [`XeroConfig::timeout`].
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

impl XeroConfig {
    pub fn new(access_token: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            tenant_id: tenant_id.into(),
            base_url: "https://api.xero.com".to_string(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Reference HTTP implementation of [`ExportSink`] against the public Xero
/// accounting API (`POST /api.xro/2.0/Invoices`, `/Payments`, `/CreditNotes`;
/// `GET /Accounts`, `/TaxRates`).
pub struct XeroClient {
    http: reqwest::Client,
    config: XeroConfig,
}

#[derive(Debug, Serialize)]
struct XeroInvoice {
    #[serde(rename = "Type")]
    invoice_type: &'static str,
    #[serde(rename = "Contact")]
    contact: XeroContact,
    #[serde(rename = "LineItems")]
    line_items: Vec<XeroLineItem>,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "DueDate")]
    due_date: String,
    #[serde(rename = "InvoiceNumber")]
    invoice_number: String,
    #[serde(rename = "Reference")]
    reference: String,
    #[serde(rename = "CurrencyCode")]
    currency_code: String,
}

#[derive(Debug, Serialize)]
struct XeroContact {
    #[serde(rename = "ContactID")]
    contact_id: String,
}

#[derive(Debug, Serialize)]
struct XeroLineItem {
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Quantity")]
    quantity: serde_json::Number,
    #[serde(rename = "UnitAmount")]
    unit_amount: serde_json::Number,
    /// Xero's per-line `TaxType`. Omitted when the caller named none, so the
    /// tenant default applies — the caller's decision, not this adapter's.
    #[serde(rename = "TaxType", skip_serializing_if = "Option::is_none")]
    tax_type: Option<String>,
}

/// A Xero `CreditNote` (`ACCRECCREDIT`) with the allocation that applies it
/// to the invoice it credits.
#[derive(Debug, Serialize)]
struct XeroCreditNote {
    #[serde(rename = "Type")]
    credit_note_type: &'static str,
    #[serde(rename = "Contact")]
    contact: XeroContact,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "LineItems")]
    line_items: Vec<XeroLineItem>,
    #[serde(rename = "CreditNoteNumber")]
    credit_note_number: String,
    #[serde(rename = "Reference")]
    reference: String,
    #[serde(rename = "CurrencyCode")]
    currency_code: String,
    #[serde(rename = "Allocations")]
    allocations: Vec<XeroAllocation>,
}

#[derive(Debug, Serialize)]
struct XeroAllocation {
    #[serde(rename = "Invoice")]
    invoice: XeroPaymentInvoice,
    #[serde(rename = "Amount")]
    amount: serde_json::Number,
    #[serde(rename = "Date")]
    date: String,
}

#[derive(Debug, Serialize)]
struct XeroPayment {
    #[serde(rename = "Invoice")]
    invoice: XeroPaymentInvoice,
    #[serde(rename = "Account")]
    account: XeroPaymentAccount,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Amount")]
    amount: serde_json::Number,
}

#[derive(Debug, Serialize)]
struct XeroPaymentInvoice {
    #[serde(rename = "InvoiceID")]
    invoice_id: String,
}

#[derive(Debug, Serialize)]
struct XeroPaymentAccount {
    #[serde(rename = "Code")]
    code: String,
}

/// Minimal slice of the Xero response JSON: Xero wraps created entities in a
/// `{ "...": [ { ... } ] }` array.
#[derive(Debug, Deserialize)]
struct XeroResponse {
    #[serde(rename = "Invoices", default)]
    invoices: Vec<XeroEntity>,
    #[serde(rename = "Payments", default)]
    payments: Vec<XeroEntity>,
    #[serde(rename = "CreditNotes", default)]
    credit_notes: Vec<XeroEntity>,
}

#[derive(Debug, Deserialize)]
struct XeroEntity {
    #[serde(rename = "InvoiceID", default)]
    invoice_id: String,
    #[serde(rename = "PaymentID", default)]
    payment_id: String,
    #[serde(rename = "CreditNoteID", default)]
    credit_note_id: String,
    #[serde(rename = "InvoiceNumber", default)]
    invoice_number: String,
    #[serde(rename = "CreditNoteNumber", default)]
    credit_note_number: String,
    /// Carries the `[marula:…]` marker on the write response and on a
    /// reconcile read — the proof a document is ours.
    #[serde(rename = "Reference", default)]
    reference: String,
}

#[derive(Debug, Deserialize)]
struct XeroAccountsResponse {
    #[serde(rename = "Accounts", default)]
    accounts: Vec<XeroAccount>,
}

#[derive(Debug, Deserialize)]
struct XeroAccount {
    #[serde(rename = "AccountID", default)]
    account_id: String,
    #[serde(rename = "Code", default)]
    code: Option<String>,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Type", default)]
    account_type: String,
    #[serde(rename = "Status", default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct XeroTaxRatesResponse {
    #[serde(rename = "TaxRates", default)]
    tax_rates: Vec<XeroTaxRate>,
}

#[derive(Debug, Deserialize)]
struct XeroTaxRate {
    #[serde(rename = "TaxType", default)]
    tax_type: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Status", default)]
    status: String,
    /// Xero's decimal rate (10.0000). `Option` because the field is absent on
    /// some rate kinds.
    #[serde(rename = "EffectiveRate", default)]
    effective_rate: Option<f64>,
}

/// Maps a Xero account `Type` onto the neutral bucket. Unmapped values are
/// [`AccountKind::Unknown`], never a guess.
fn account_kind(xero_type: &str) -> AccountKind {
    match xero_type {
        "BANK" => AccountKind::Bank,
        "CURRENT" | "PREPAYMENT" | "INVENTORY" => AccountKind::OtherCurrentAsset,
        "FIXED" => AccountKind::FixedAsset,
        "NONCURRENT" => AccountKind::OtherAsset,
        "CURRLIAB" | "LIABILITY" => AccountKind::OtherCurrentLiability,
        "TERMLIAB" => AccountKind::LongTermLiability,
        "EQUITY" => AccountKind::Equity,
        "REVENUE" | "SALES" => AccountKind::Income,
        "DIRECTCOSTS" => AccountKind::CostOfGoodsSold,
        "EXPENSE" | "OVERHEADS" => AccountKind::Expense,
        "DEPRECIATN" => AccountKind::OtherExpense,
        "OTHERINCOME" => AccountKind::OtherIncome,
        _ => AccountKind::Unknown,
    }
}

/// Renders minor units (cents) as a JSON number. f64 round-trips two
/// decimal-place amounts within invoice magnitudes and Xero re-parses the
/// shortest serialization, so no cents are lost in practice.
fn minor_to_number(minor: i64) -> serde_json::Number {
    serde_json::Number::from_f64(minor as f64 / 100.0)
        .unwrap_or_else(|| serde_json::Number::from(0))
}

fn qty_to_number(qty: &str) -> Result<serde_json::Number, ExportError> {
    let parsed: f64 = qty
        .parse()
        .map_err(|_| ExportError::InvalidRequest(format!("quantity is not a number: {qty}")))?;
    if !parsed.is_finite() {
        return Err(ExportError::InvalidRequest(format!(
            "quantity is not finite: {qty}"
        )));
    }
    Ok(serde_json::Number::from_f64(parsed).unwrap_or_else(|| serde_json::Number::from(0)))
}

/// The `[marula:<kind>:<uuid>]` marker this crate stamps onto a document so a
/// later reconcile can prove the vendor record is ours.
fn marker(kind: &str, marula_id: Uuid) -> String {
    format!("[marula:{kind}:{marula_id}]")
}

/// Vendor prose is kept, but bounded: a 2 MB HTML error page is not a reason.
fn truncate(body: &str) -> String {
    body.chars().take(500).collect()
}

fn vendor_error(status: reqwest::StatusCode, body: &str) -> ExportError {
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ExportError::Unauthorized(format!("Xero returned {status}"))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            ExportError::RateLimited(format!("Xero returned 429: {}", truncate(body)))
        }
        s => ExportError::Vendor {
            status: s.as_u16(),
            message: truncate(body),
        },
    }
}

/// Transport failures keep the reason and drop the URL. `reqwest`'s Display
/// prints the request URL, and a sandbox base URL is often written with
/// credentials in it, so the URL is not a safe thing to put in a log.
fn transport_error(e: reqwest::Error) -> ExportError {
    if e.is_timeout() {
        ExportError::Network("request timed out".into())
    } else {
        ExportError::Network(e.without_url().to_string())
    }
}

/// One line of a document, vendor-shaped. Shared by invoices and credit notes
/// because Xero uses the same `LineItem` for both.
fn xero_lines(
    lines: &[ExportLine],
    tax_code: Option<&str>,
) -> Result<Vec<XeroLineItem>, ExportError> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        out.push(XeroLineItem {
            description: line.description.clone(),
            quantity: qty_to_number(&line.quantity)?,
            unit_amount: minor_to_number(line.unit_amount_minor),
            tax_type: tax_code.map(|v| v.to_string()),
        });
    }
    Ok(out)
}

/// The document total in minor units, rounded the same way the lines price.
fn lines_total_minor(lines: &[ExportLine]) -> Result<i64, ExportError> {
    let mut total: i64 = 0;
    for line in lines {
        let qty = qty_to_number(&line.quantity)?;
        let amount = (line.unit_amount_minor as f64 * qty.as_f64().unwrap_or(0.0)).round() as i64;
        total = total.saturating_add(amount);
    }
    Ok(total)
}

impl XeroClient {
    /// Builds a client. Fallible because a client without its timeout is the
    /// fault this adapter must not have.
    pub fn new(config: XeroConfig) -> Result<Self, ExportError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| ExportError::Network(format!("HTTP client: {}", e.without_url())))?;
        Ok(Self { http, config })
    }

    fn endpoint(&self, resource: &str) -> String {
        format!(
            "{}/api.xro/2.0/{}",
            self.config.base_url.trim_end_matches('/'),
            resource
        )
    }

    /// Sends a POST and parses the JSON body, mapping every failure onto the
    /// typed error set. Shared by all write paths so the timeout, the tenant
    /// header and the error split cannot drift between them.
    async fn post(
        &self,
        url: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ExportError> {
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.access_token)
            .header("Xero-tenant-id", &self.config.tenant_id)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(transport_error)?;

        let status = response.status();
        let text = response.text().await.map_err(transport_error)?;
        if !status.is_success() {
            return Err(vendor_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|e| ExportError::Vendor {
            status: status.as_u16(),
            message: format!("unparseable Xero response: {e}"),
        })
    }

    /// Sends a GET with optional query parameters.
    async fn get(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, ExportError> {
        let response = self
            .http
            .get(url)
            .query(query)
            .bearer_auth(&self.config.access_token)
            .header("Xero-tenant-id", &self.config.tenant_id)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(transport_error)?;

        let status = response.status();
        let text = response.text().await.map_err(transport_error)?;
        if !status.is_success() {
            return Err(vendor_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|e| ExportError::Vendor {
            status: status.as_u16(),
            message: format!("unparseable Xero response: {e}"),
        })
    }
}

#[async_trait]
impl ExportSink for XeroClient {
    fn name(&self) -> &str {
        "xero"
    }

    async fn push_invoice(&self, invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError> {
        if invoice.customer_ref.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "customer_ref (Xero ContactID) is required to create an invoice".into(),
            ));
        }
        if invoice.lines.is_empty() {
            return Err(ExportError::InvalidRequest("invoice has no lines".into()));
        }

        let body = XeroInvoice {
            invoice_type: "ACCREC",
            contact: XeroContact {
                contact_id: invoice.customer_ref.clone(),
            },
            line_items: xero_lines(&invoice.lines, invoice.tax_code.as_deref())?,
            date: invoice.issue_date.to_string(),
            due_date: invoice.due_date.to_string(),
            invoice_number: invoice.number.clone(),
            reference: format!(
                "{} {}",
                marker("invoice", invoice.invoice_id),
                invoice.notes.clone().unwrap_or_default()
            )
            .trim_end()
            .to_string(),
            currency_code: invoice.currency.clone(),
        };
        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize invoice: {e}")))?;

        let response: XeroResponse = serde_json::from_value(
            self.post(&self.endpoint("Invoices"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing Invoices array in response: {e}"),
        })?;
        let entity = response
            .invoices
            .into_iter()
            .next()
            .ok_or_else(|| ExportError::Vendor {
                status: 200,
                message: "Xero response carried no Invoice".into(),
            })?;
        Ok(ExportReceipt {
            external_id: entity.invoice_id,
            external_number: (!entity.invoice_number.is_empty()).then_some(entity.invoice_number),
            created: true,
        })
    }

    async fn push_payment(&self, payment: &ExportPayment) -> Result<ExportReceipt, ExportError> {
        if payment.invoice_external_id.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "invoice_external_id (the Xero InvoiceID) is required to link a payment".into(),
            ));
        }
        if payment.account_code.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "account_code (Xero Account.Code) is required on Payments".into(),
            ));
        }
        let body = XeroPayment {
            invoice: XeroPaymentInvoice {
                invoice_id: payment.invoice_external_id.clone(),
            },
            account: XeroPaymentAccount {
                code: payment.account_code.clone(),
            },
            date: payment.applied_on.to_string(),
            amount: minor_to_number(payment.amount_minor),
        };
        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize payment: {e}")))?;

        let response: XeroResponse = serde_json::from_value(
            self.post(&self.endpoint("Payments"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing Payments array in response: {e}"),
        })?;
        let entity = response
            .payments
            .into_iter()
            .next()
            .ok_or_else(|| ExportError::Vendor {
                status: 200,
                message: "Xero response carried no Payment".into(),
            })?;
        Ok(ExportReceipt {
            external_id: entity.payment_id,
            external_number: None,
            created: true,
        })
    }

    async fn push_credit_note(
        &self,
        note: &ExportCreditNote,
    ) -> Result<ExportReceipt, ExportError> {
        if note.customer_ref.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "customer_ref (Xero ContactID) is required to create a credit note".into(),
            ));
        }
        if note.invoice_external_id.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "invoice_external_id (the Xero InvoiceID) is required: an unallocated credit \
                 note leaves the invoice at its full balance"
                    .into(),
            ));
        }
        if note.lines.is_empty() {
            return Err(ExportError::InvalidRequest(
                "credit note has no lines".into(),
            ));
        }
        let amount_minor = lines_total_minor(&note.lines)?;

        let body = XeroCreditNote {
            credit_note_type: "ACCRECCREDIT",
            contact: XeroContact {
                contact_id: note.customer_ref.clone(),
            },
            date: note.issued_on.to_string(),
            line_items: xero_lines(&note.lines, note.tax_code.as_deref())?,
            credit_note_number: note.number.clone(),
            reference: format!(
                "{} {}",
                marker("credit_note", note.credit_note_id),
                note.reason.clone().unwrap_or_default()
            )
            .trim_end()
            .to_string(),
            currency_code: note.currency.clone(),
            allocations: vec![XeroAllocation {
                invoice: XeroPaymentInvoice {
                    invoice_id: note.invoice_external_id.clone(),
                },
                amount: minor_to_number(amount_minor),
                date: note.issued_on.to_string(),
            }],
        };

        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize credit note: {e}")))?;
        let response: XeroResponse = serde_json::from_value(
            self.post(&self.endpoint("CreditNotes"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing CreditNotes array in response: {e}"),
        })?;
        let entity =
            response
                .credit_notes
                .into_iter()
                .next()
                .ok_or_else(|| ExportError::Vendor {
                    status: 200,
                    message: "Xero response carried no CreditNote".into(),
                })?;
        Ok(ExportReceipt {
            external_id: entity.credit_note_id,
            external_number: (!entity.credit_note_number.is_empty())
                .then_some(entity.credit_note_number),
            created: true,
        })
    }

    async fn read_accounts(&self) -> Result<Vec<LedgerAccount>, ExportError> {
        let raw = self.get(&self.endpoint("Accounts"), &[]).await?;
        let parsed: XeroAccountsResponse =
            serde_json::from_value(raw).map_err(|e| ExportError::Vendor {
                status: 200,
                message: format!("missing Accounts array in Xero reply: {e}"),
            })?;
        Ok(parsed
            .accounts
            .into_iter()
            .map(|a| LedgerAccount {
                code: a
                    .code
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty()),
                id: a.account_id,
                name: a.name,
                kind: account_kind(&a.account_type),
                active: a.status.eq_ignore_ascii_case("ACTIVE"),
            })
            .collect())
    }

    async fn read_tax_codes(&self) -> Result<Vec<TaxCode>, ExportError> {
        let raw = self.get(&self.endpoint("TaxRates"), &[]).await?;
        let parsed: XeroTaxRatesResponse =
            serde_json::from_value(raw).map_err(|e| ExportError::Vendor {
                status: 200,
                message: format!("missing TaxRates array in Xero reply: {e}"),
            })?;
        Ok(parsed
            .tax_rates
            .into_iter()
            .map(|r| TaxCode {
                code: r.tax_type,
                name: r.name,
                // Xero reports `10.0000`; basis points are 1000. Rounding to
                // the nearest point is the only safe reduction — a rate with
                // more precision than that has no exact bps answer.
                rate_bps: r.effective_rate.map(|v| (v * 100.0).round() as i32),
                active: r.status.eq_ignore_ascii_case("ACTIVE"),
            })
            .collect())
    }

    async fn find_document(
        &self,
        lookup: &ExportLookup,
    ) -> Result<Option<ExportReceipt>, ExportError> {
        if lookup.number.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "a reconcile lookup needs the document number the first attempt sent".into(),
            ));
        }
        // Xero's `where` filter takes a double-quoted literal; a quote inside
        // the value would end it. Document numbers are ours, but the lookup is
        // public API, so escape rather than trust.
        let number = lookup.number.replace('"', "");
        let (resource, field, kind_marker) = match lookup.kind {
            ExportLookupKind::Invoice => ("Invoices", "InvoiceNumber", "invoice"),
            ExportLookupKind::CreditNote => ("CreditNotes", "CreditNoteNumber", "credit_note"),
        };
        let raw = self
            .get(
                &self.endpoint(resource),
                &[("where", format!("{field}==\"{number}\""))],
            )
            .await?;
        let parsed: XeroResponse =
            serde_json::from_value(raw).map_err(|e| ExportError::Vendor {
                status: 200,
                message: format!("missing {resource} array in Xero reply: {e}"),
            })?;

        let needle = marker(kind_marker, lookup.marula_id);
        let candidates = match lookup.kind {
            ExportLookupKind::Invoice => parsed.invoices,
            ExportLookupKind::CreditNote => parsed.credit_notes,
        };
        for candidate in candidates {
            if candidate.reference.contains(&needle) {
                let (external_id, external_number) = match lookup.kind {
                    ExportLookupKind::Invoice => (candidate.invoice_id, candidate.invoice_number),
                    ExportLookupKind::CreditNote => {
                        (candidate.credit_note_id, candidate.credit_note_number)
                    }
                };
                return Ok(Some(ExportReceipt {
                    external_id,
                    external_number: (!external_number.is_empty()).then_some(external_number),
                    created: false,
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
