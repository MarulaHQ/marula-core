//! QuickBooks Online export-sync adapter (the private build tracker Phase 3 adapters line).
//!
//! Purpose: push Marula invoices, payments and credit notes into a
//! customer-provided QBO company over the PUBLIC QBO REST API
//! (`/v3/company/{realmId}/...`) — HTTP only, no vendor SDK. OAuth2 access
//! tokens are operator-supplied (BYOK: sealed at rest in the core's
//! `connector_connections.secrets_encrypted` document; Marula hosts no OAuth
//! app). Background sync scheduling is a future worker job — this crate is the
//! edge adapter only.
//!
//! Read leg (claims-gap G2): [`ExportSink::read_accounts`] and
//! [`ExportSink::read_tax_codes`] return the customer's chart of accounts and
//! sales tax codes in a NEUTRAL shape — account kinds are the vendor-neutral
//! buckets in [`AccountKind`], not QBO's own strings. That is what lets Core
//! validate a tax mapping against the books instead of guessing (G3), so the
//! shape is defined here, in Core-facing terms, and never leaks vendor JSON.
//!
//! Credit notes (G1): [`ExportSink::push_credit_note`] posts a QBO
//! `CreditMemo` and links it to the invoice that was exported earlier, through
//! `LinkedTxn` (`TxnId` = the vendor invoice `Id` from the stored
//! [`ExportReceipt`]). A credit memo that is not linked is a second document
//! for the same money, so the caller must resolve the parent invoice's
//! vendor id first; this crate refuses to invent one.
//!
//! Reconcile (the crash window): a process that dies between the vendor call
//! and the receipt write leaves no receipt and no second attempt that can know
//! whether QBO created the document. [`ExportSink::find_document`] answers that
//! question by querying for the `DocNumber` and confirming the
//! `[marula:<kind>:<uuid>]` marker this crate writes into `PrivateNote`, so a
//! retry adopts the existing document instead of creating a twin.
//!
//! None mode: [`NoneExportSink`] is the shipped default — it performs zero
//! network I/O and returns [`ExportError::Disabled`], so an org with no
//! accounting credentials operates fully standalone (standalone-first
//! doctrine). Every call site must treat `Disabled` as "would have synced".
//!
//! Non-purpose: no OAuth token refresh (operators rotate the token in the
//! sealed store), no QBO customer/item catalog management, no reverse
//! (QBO→Marula) import, no `open_url` (QBO has no supported transaction deep
//! link: `txnId` is not the API `Id`, so a constructed link can open another
//! company's invoice — see `docs/reference/connector-capabilities.md`).

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
    /// The vendor's customer id (QBO `CustomerRef.value` / Xero
    /// `Contact.ContactID`). Export adapters never invent customers.
    pub customer_ref: String,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    /// ISO 4217 code ("USD").
    pub currency: String,
    pub lines: Vec<ExportLine>,
    pub notes: Option<String>,
    /// The VENDOR tax code every line carries, already resolved by the caller
    /// through its mapping store (QBO `TaxCode.Id`). `None` means the caller
    /// asked for no tax code — this adapter never substitutes a default,
    /// because a default someone has to reconcile later is exactly the guess
    /// claims-gap G3 exists to prevent.
    pub tax_code: Option<String>,
}

/// A payment received against a previously exported invoice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPayment {
    pub payment_id: Uuid,
    /// The invoice's VENDOR-side id (`ExportReceipt.external_id` from the
    /// earlier invoice push) — payments always attach to vendor records.
    pub invoice_external_id: String,
    /// Vendor customer id (same value as the invoice's `customer_ref`).
    pub customer_ref: String,
    /// Payment date.
    pub applied_on: NaiveDate,
    /// Amount in minor units.
    pub amount_minor: i64,
    pub currency: String,
}

/// A credit note Marula issues against an invoice it already exported.
///
/// The vendor link is the whole point: `invoice_external_id` is the QBO
/// invoice `Id` read from the stored receipt of the parent invoice. A credit
/// memo carrying it is an adjustment to that invoice; one without it is a
/// second document for the same money, which is why this field is required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportCreditNote {
    pub credit_note_id: Uuid,
    /// Human credit-note number ("CRN-000012") — the vendor's `DocNumber`.
    pub number: String,
    /// Vendor customer id; QBO requires it on a credit memo.
    pub customer_ref: String,
    /// The parent invoice's VENDOR id (`ExportReceipt.external_id` from the
    /// invoice export, same connection). Required: an orphan credit is refused
    /// upstream, never exported unapplied.
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
    /// The vendor's id for the created/updated record (QBO `Id`, Xero
    /// `InvoiceID`/`PaymentID`). Callers store this to attach payments and
    /// to make re-pushes idempotent.
    pub external_id: String,
    /// Vendor document number when the vendor assigns/echoes one.
    pub external_number: Option<String>,
    /// True when the vendor created a NEW record; false for an update of
    /// an existing one (QBO SyncToken update).
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
    /// The Marula id, which this crate stamps into the vendor's private note
    /// at creation (`[marula:invoice:<uuid>]`). The marker is the proof of
    /// ownership: a `DocNumber` alone is not unique in QBO.
    pub marula_id: Uuid,
    /// The document number the earlier attempt would have sent.
    pub number: String,
}

/// The neutral classification of a ledger account.
///
/// QBO's own `AccountType` vocabulary is the wider one; Xero uses different
/// strings again. Core branches on this set instead, so a mapping rule is
/// written once. `Unknown` is honest rather than lossy: a vendor adds an
/// account type, it lands here, and Core leaves it out of a pick list that
/// claims to be complete.
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

/// One account in the customer's chart of accounts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerAccount {
    /// The vendor's id (QBO `Account.Id`) — what a vendor document references.
    pub id: String,
    /// The operator-facing account code (QBO `AcctNum`) when the vendor has
    /// one. QBO leaves it empty on many accounts, which is why `id` exists
    /// beside it rather than instead of it.
    pub code: Option<String>,
    pub name: String,
    pub kind: AccountKind,
    /// False for an archived/inactive account: it is still returned (a
    /// historical line may reference it) but a pick list must not offer it.
    pub active: bool,
}

/// One tax code in the customer's books.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaxCode {
    /// The id a vendor invoice line carries (QBO `TaxCode.Id`, Xero
    /// `TaxType`). This is the value a mapping stores on the vendor side.
    pub code: String,
    pub name: String,
    /// Effective rate in basis points when the vendor states one (8.25% =
    /// 825). `None` for a rate the vendor did not publish — QBO tax groups
    /// report rates per component, and a group with no resolvable component
    /// is reported as none rather than summed into a number nobody set.
    pub rate_bps: Option<i32>,
    pub active: bool,
}

/// Typed error surfaced across the export seam. Vendor detail is
/// stringified into `reason`s — no vendor SDK types leak past this crate.
///
/// The variants are the three answers a caller must be able to tell apart
/// (claims-gap G3's "held vs failed" needs exactly this): the credential is
/// wrong ([`ExportError::Unauthorized`], an operator fixes it), the vendor
/// asked us to slow down ([`ExportError::RateLimited`], retry later), or the
/// request itself is unacceptable ([`ExportError::Vendor`] with a 4xx status
/// — retrying re-sends the same refusal). [`ExportError::retryable`] is the
/// one place that split is encoded.
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
    /// request URL: a base URL can hold credentials a vendor's sandbox
    /// documents as `https://user:pass@host`.
    #[error("export transport failure: {0}")]
    Network(String),
}

impl ExportError {
    /// True when re-sending the same request could succeed later.
    ///
    /// A 4xx is a statement about the request, so a retry buys nothing; the
    /// run report shows it as failed with `vendor_rejected` rather than
    /// leaving a person pressing retry. A transport failure, a 429 and a 5xx
    /// are all "not now".
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
#[async_trait::async_trait]
pub trait ExportSink: Send + Sync {
    fn name(&self) -> &str;

    async fn push_invoice(&self, invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError>;

    async fn push_payment(&self, payment: &ExportPayment) -> Result<ExportReceipt, ExportError>;

    /// Post a credit note that adjusts a previously exported invoice. The
    /// caller supplies the parent invoice's vendor id; this crate never
    /// resolves or invents one.
    async fn push_credit_note(&self, note: &ExportCreditNote)
        -> Result<ExportReceipt, ExportError>;

    /// The customer's chart of accounts, neutral-shaped (read leg, G2).
    async fn read_accounts(&self) -> Result<Vec<LedgerAccount>, ExportError>;

    /// The customer's sales tax codes, neutral-shaped (read leg, G2).
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

// --- QBO reference implementation ---------------------------------------------

/// Credentials + endpoint configuration for [`QboClient`]. Values come from
/// the sealed store (decrypted just-in-time by the caller, never logged,
/// never serialized).
#[derive(Clone)]
pub struct QboConfig {
    /// QBO OAuth2 access token with `com.intuit.quickbooks.accounting`
    /// scope.
    pub access_token: String,
    /// The target company's realm id (QBO paths are per-company).
    pub realm_id: String,
    /// API base — overridable for tests; production default is the public
    /// sandboxless production host.
    pub base_url: String,
    /// `minorversion` query parameter pinned per release (docs list it as
    /// required for deterministic behavior).
    pub minor_version: String,
    /// Wall-clock budget for ONE vendor request, connect included.
    ///
    /// Bounded because an export runs inside a request handler: an unbounded
    /// dial is a handler that outlives its client. The caller's claim on the
    /// run row expires later than this (see the core's export store), so a
    /// timeout can never be mistaken for a dead process.
    pub timeout: Duration,
}

/// Debug prints the realm and the host, never the token. The derive would
/// have made every `{:?}` on a config — a log line, a panic message, a test
/// failure — a credential leak.
impl std::fmt::Debug for QboConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QboConfig")
            .field("access_token", &"<redacted>")
            .field("realm_id", &self.realm_id)
            .field("base_url", &self.base_url)
            .field("minor_version", &self.minor_version)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Connect timeout, deliberately shorter than [`QboConfig::timeout`]: a TCP
/// handshake that has not completed in ten seconds is a dead host, and
/// spending the whole budget on it only delays the typed failure.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Rows per QBO query page. QBO caps `MAXRESULTS` at 1000.
const QUERY_PAGE: usize = 1000;

/// Pages one read leg walks. A book past 5000 accounts or tax codes is not a
/// small-business book; stopping is honest, and the read is bounded so a
/// malformed paging response cannot loop forever.
const QUERY_MAX_PAGES: usize = 5;

impl QboConfig {
    pub fn new(access_token: impl Into<String>, realm_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            realm_id: realm_id.into(),
            base_url: "https://quickbooks.api.intuit.com".to_string(),
            minor_version: "75".to_string(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Reference HTTP implementation of [`ExportSink`] against the public QBO
/// REST API (`/v3/company/{realmId}/invoice`, `/payment`, `/creditmemo`,
/// `/query`).
pub struct QboClient {
    http: reqwest::Client,
    config: QboConfig,
}

#[derive(Debug, Serialize)]
struct QboLine {
    #[serde(rename = "DetailType")]
    detail_type: &'static str,
    #[serde(rename = "Amount")]
    amount: serde_json::Number,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "SalesItemLineDetail")]
    detail: QboLineDetail,
}

#[derive(Debug, Serialize)]
struct QboLineDetail {
    #[serde(rename = "Qty")]
    qty: serde_json::Number,
    #[serde(rename = "UnitPrice")]
    unit_price: serde_json::Number,
    /// The vendor's own tax code for the line. Omitted when the caller named
    /// none, so QBO applies the company default — which is the caller's
    /// decision to make, not this adapter's.
    #[serde(rename = "TaxCodeRef", skip_serializing_if = "Option::is_none")]
    tax_code_ref: Option<QboRef>,
}

#[derive(Debug, Serialize)]
struct QboInvoice {
    #[serde(rename = "CustomerRef")]
    customer_ref: QboRef,
    #[serde(rename = "TxnDate")]
    txn_date: String,
    #[serde(rename = "DueDate")]
    due_date: String,
    #[serde(rename = "DocNumber")]
    doc_number: String,
    #[serde(rename = "CurrencyRef", skip_serializing_if = "Option::is_none")]
    currency_ref: Option<QboRef>,
    #[serde(rename = "PrivateNote", skip_serializing_if = "Option::is_none")]
    private_note: Option<String>,
    #[serde(rename = "Line")]
    line: Vec<QboLine>,
}

/// A QBO `CreditMemo`. `LinkedTxn` is the documented link to the invoice it
/// credits; without it the memo is an unapplied credit and the invoice keeps
/// its full balance.
#[derive(Debug, Serialize)]
struct QboCreditMemo {
    #[serde(rename = "CustomerRef")]
    customer_ref: QboRef,
    #[serde(rename = "TxnDate")]
    txn_date: String,
    #[serde(rename = "DocNumber")]
    doc_number: String,
    #[serde(rename = "CurrencyRef", skip_serializing_if = "Option::is_none")]
    currency_ref: Option<QboRef>,
    #[serde(rename = "PrivateNote", skip_serializing_if = "Option::is_none")]
    private_note: Option<String>,
    #[serde(rename = "LinkedTxn")]
    linked_txn: Vec<QboLinkedTxn>,
    #[serde(rename = "Line")]
    line: Vec<QboLine>,
}

#[derive(Debug, Serialize)]
struct QboRef {
    value: String,
}

#[derive(Debug, Serialize)]
struct QboPayment {
    #[serde(rename = "CustomerRef")]
    customer_ref: QboRef,
    #[serde(rename = "TotalAmt")]
    total_amt: serde_json::Number,
    #[serde(rename = "TxnDate")]
    txn_date: String,
    #[serde(rename = "Line")]
    line: Vec<QboPaymentLine>,
}

#[derive(Debug, Serialize)]
struct QboPaymentLine {
    #[serde(rename = "Amount")]
    amount: serde_json::Number,
    #[serde(rename = "LinkedTxn")]
    linked_txn: Vec<QboLinkedTxn>,
}

#[derive(Debug, Serialize)]
struct QboLinkedTxn {
    #[serde(rename = "TxnId")]
    txn_id: String,
    #[serde(rename = "TxnType")]
    txn_type: &'static str,
}

/// Minimal slice of the QBO Invoice/Payment/CreditMemo response JSON.
#[derive(Debug, Deserialize)]
struct QboResponse {
    #[serde(rename = "Invoice", default)]
    invoice: Option<QboEntity>,
    #[serde(rename = "Payment", default)]
    payment: Option<QboEntity>,
    #[serde(rename = "CreditMemo", default)]
    credit_memo: Option<QboEntity>,
}

#[derive(Debug, Deserialize)]
struct QboEntity {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "DocNumber", default)]
    doc_number: String,
}

/// `GET /query` answers `{ "QueryResponse": { "<Entity>": [ … ] } }`.
#[derive(Debug, Deserialize)]
struct QboQueryResponse {
    #[serde(rename = "QueryResponse", default)]
    query_response: Option<QboQueryBody>,
}

#[derive(Debug, Deserialize, Default)]
struct QboQueryBody {
    #[serde(rename = "Invoice", default)]
    invoices: Vec<QboQueriedEntity>,
    #[serde(rename = "CreditMemo", default)]
    credit_memos: Vec<QboQueriedEntity>,
    #[serde(rename = "Account", default)]
    accounts: Vec<QboAccount>,
    #[serde(rename = "TaxCode", default)]
    tax_codes: Vec<QboTaxCode>,
    #[serde(rename = "TaxRate", default)]
    tax_rates: Vec<QboTaxRate>,
}

/// A queried transaction row. `PrivateNote` is read because it carries the
/// `[marula:…]` marker that proves the document is ours.
#[derive(Debug, Deserialize)]
struct QboQueriedEntity {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "DocNumber", default)]
    doc_number: String,
    #[serde(rename = "PrivateNote", default)]
    private_note: String,
}

#[derive(Debug, Deserialize)]
struct QboAccount {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "AcctNum", default)]
    acct_num: Option<String>,
    #[serde(rename = "AccountType", default)]
    account_type: String,
    #[serde(rename = "Active", default = "default_true")]
    active: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct QboTaxCode {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Active", default = "default_true")]
    active: bool,
    #[serde(rename = "SalesTaxRateList", default)]
    sales_tax_rate_list: Option<QboTaxRateList>,
}

#[derive(Debug, Deserialize, Default)]
struct QboTaxRateList {
    #[serde(rename = "TaxRateDetail", default)]
    tax_rate_detail: Vec<QboTaxRateDetail>,
}

#[derive(Debug, Deserialize, Default)]
struct QboTaxRateDetail {
    #[serde(rename = "TaxRateRef", default)]
    tax_rate_ref: Option<QboRefOut>,
}

#[derive(Debug, Deserialize, Default)]
struct QboRefOut {
    #[serde(rename = "value", default)]
    value: String,
}

#[derive(Debug, Deserialize)]
struct QboTaxRate {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "RateValue", default)]
    rate_value: Option<f64>,
    #[serde(rename = "Active", default = "default_true")]
    active: bool,
}

/// Maps a QBO `AccountType` string onto the neutral bucket. Unmapped values
/// are [`AccountKind::Unknown`], never a guess.
fn account_kind(qbo_type: &str) -> AccountKind {
    match qbo_type {
        "Bank" => AccountKind::Bank,
        "Accounts Receivable" => AccountKind::AccountsReceivable,
        "Other Current Asset" => AccountKind::OtherCurrentAsset,
        "Fixed Asset" => AccountKind::FixedAsset,
        "Other Asset" => AccountKind::OtherAsset,
        "Accounts Payable" => AccountKind::AccountsPayable,
        "Credit Card" => AccountKind::CreditCard,
        "Other Current Liability" => AccountKind::OtherCurrentLiability,
        "Long Term Liability" => AccountKind::LongTermLiability,
        "Equity" => AccountKind::Equity,
        "Income" => AccountKind::Income,
        "Cost of Goods Sold" => AccountKind::CostOfGoodsSold,
        "Expense" => AccountKind::Expense,
        "Other Income" => AccountKind::OtherIncome,
        "Other Expense" => AccountKind::OtherExpense,
        _ => AccountKind::Unknown,
    }
}

/// Renders minor units (cents) as a JSON number. f64 round-trips two
/// decimal-place amounts within invoice magnitudes and QBO re-parses the
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

fn vendor_error(status: reqwest::StatusCode, body: &str) -> ExportError {
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ExportError::Unauthorized(format!("QBO returned {status}"))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            ExportError::RateLimited(format!("QBO returned 429: {}", truncate(body)))
        }
        s => ExportError::Vendor {
            status: s.as_u16(),
            message: truncate(body),
        },
    }
}

/// Vendor prose is kept, but bounded: a 2 MB HTML error page is not a reason.
fn truncate(body: &str) -> String {
    body.chars().take(500).collect()
}

/// One line of a document, vendor-shaped. Shared by invoices and credit
/// memos because QBO uses the same `SalesItemLine` for both.
fn qbo_lines(lines: &[ExportLine], tax_code: Option<&str>) -> Result<Vec<QboLine>, ExportError> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let qty = qty_to_number(&line.quantity)?;
        let amount_minor =
            (line.unit_amount_minor as f64 * qty.as_f64().unwrap_or(0.0)).round() as i64;
        out.push(QboLine {
            detail_type: "SalesItemLine",
            amount: minor_to_number(amount_minor),
            description: line.description.clone(),
            detail: QboLineDetail {
                qty,
                unit_price: minor_to_number(line.unit_amount_minor),
                tax_code_ref: tax_code.map(|v| QboRef {
                    value: v.to_string(),
                }),
            },
        });
    }
    Ok(out)
}

impl QboClient {
    /// Builds a client. The constructor is fallible because a client without
    /// its timeout is exactly the fault this adapter must not have: the
    /// builder only fails on an invalid TLS configuration, and pretending
    /// otherwise would hand the caller an unbounded client that looks
    /// identical.
    pub fn new(config: QboConfig) -> Result<Self, ExportError> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| ExportError::Network(format!("HTTP client: {}", e.without_url())))?;
        Ok(Self { http, config })
    }

    fn endpoint(&self, resource: &str) -> String {
        format!(
            "{}/v3/company/{}/{}?minorversion={}",
            self.config.base_url.trim_end_matches('/'),
            self.config.realm_id,
            resource,
            self.config.minor_version
        )
    }

    fn query_url(&self) -> String {
        format!(
            "{}/v3/company/{}/query",
            self.config.base_url.trim_end_matches('/'),
            self.config.realm_id
        )
    }

    /// Sends a POST and parses the JSON body, mapping every failure onto the
    /// typed error set. Shared by all write paths so the timeout, the auth
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
            message: format!("unparseable QBO response: {e}"),
        })
    }

    /// Sends a GET and parses the JSON body.
    async fn get(
        &self,
        url: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value, ExportError> {
        let response = self
            .http
            .get(url)
            .query(&[("minorversion", self.config.minor_version.clone())])
            .query(query)
            .bearer_auth(&self.config.access_token)
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
            message: format!("unparseable QBO response: {e}"),
        })
    }

    /// Runs one QBO query and returns the `QueryResponse` body.
    async fn query(&self, query: &str) -> Result<QboQueryBody, ExportError> {
        let raw = self
            .get(&self.query_url(), &[("query", query.to_string())])
            .await?;
        let parsed: QboQueryResponse =
            serde_json::from_value(raw).map_err(|e| ExportError::Vendor {
                status: 200,
                message: format!("missing QueryResponse in QBO reply: {e}"),
            })?;
        Ok(parsed.query_response.unwrap_or_default())
    }

    /// Walks one entity's query pages until a short page arrives.
    ///
    /// QBO's query language pages with `STARTPOSITION` / `MAXRESULTS` and
    /// reports no total, so "fewer rows than I asked for" is the only end
    /// signal available. The page cap bounds a malformed reply.
    async fn query_all<T, F>(&self, entity: &str, mut pick: F) -> Result<Vec<T>, ExportError>
    where
        F: FnMut(QboQueryBody) -> Vec<T>,
    {
        let mut out = Vec::new();
        for page in 0..QUERY_MAX_PAGES {
            let start = page * QUERY_PAGE + 1;
            let q = format!("select * from {entity} STARTPOSITION {start} MAXRESULTS {QUERY_PAGE}");
            let body = self.query(&q).await?;
            let mut rows = pick(body);
            let short = rows.len() < QUERY_PAGE;
            out.append(&mut rows);
            if short {
                break;
            }
        }
        Ok(out)
    }
}

/// Transport failures keep the reason and drop the URL. `reqwest`'s Display
/// prints the request URL, and a sandbox base URL is often written with
/// credentials in it (`https://key:secret@host`), so the URL is not a safe
/// thing to put in a log or an error string.
fn transport_error(e: reqwest::Error) -> ExportError {
    if e.is_timeout() {
        ExportError::Network("request timed out".into())
    } else {
        ExportError::Network(e.without_url().to_string())
    }
}

#[async_trait]
impl ExportSink for QboClient {
    fn name(&self) -> &str {
        "qbo"
    }

    async fn push_invoice(&self, invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError> {
        if invoice.customer_ref.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "customer_ref (QBO CustomerRef.value) is required to create an invoice".into(),
            ));
        }
        if invoice.lines.is_empty() {
            return Err(ExportError::InvalidRequest("invoice has no lines".into()));
        }

        let qbo_lines = qbo_lines(&invoice.lines, invoice.tax_code.as_deref())?;

        let body = QboInvoice {
            customer_ref: QboRef {
                value: invoice.customer_ref.clone(),
            },
            txn_date: invoice.issue_date.to_string(),
            due_date: invoice.due_date.to_string(),
            doc_number: invoice.number.clone(),
            currency_ref: Some(QboRef {
                value: invoice.currency.clone(),
            }),
            private_note: Some(format!(
                "{} {}",
                invoice
                    .notes
                    .clone()
                    .map(|n| format!("{n} "))
                    .unwrap_or_default(),
                marker("invoice", invoice.invoice_id)
            )),
            line: qbo_lines,
        };

        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize invoice: {e}")))?;

        let response: QboResponse = serde_json::from_value(
            self.post(&self.endpoint("invoice"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing Invoice entity in response: {e}"),
        })?;
        let entity = response.invoice.ok_or_else(|| ExportError::Vendor {
            status: 200,
            message: "QBO response carried no Invoice".into(),
        })?;
        Ok(ExportReceipt {
            external_id: entity.id,
            external_number: (!entity.doc_number.is_empty()).then_some(entity.doc_number),
            created: true,
        })
    }

    async fn push_payment(&self, payment: &ExportPayment) -> Result<ExportReceipt, ExportError> {
        if payment.invoice_external_id.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "invoice_external_id (the QBO invoice Id) is required to link a payment".into(),
            ));
        }
        let body = QboPayment {
            customer_ref: QboRef {
                value: payment.customer_ref.clone(),
            },
            total_amt: minor_to_number(payment.amount_minor),
            txn_date: payment.applied_on.to_string(),
            line: vec![QboPaymentLine {
                amount: minor_to_number(payment.amount_minor),
                linked_txn: vec![QboLinkedTxn {
                    txn_id: payment.invoice_external_id.clone(),
                    txn_type: "Invoice",
                }],
            }],
        };
        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize payment: {e}")))?;
        let response: QboResponse = serde_json::from_value(
            self.post(&self.endpoint("payment"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing Payment entity in response: {e}"),
        })?;
        let entity = response.payment.ok_or_else(|| ExportError::Vendor {
            status: 200,
            message: "QBO response carried no Payment".into(),
        })?;
        Ok(ExportReceipt {
            external_id: entity.id,
            external_number: (!entity.doc_number.is_empty()).then_some(entity.doc_number),
            created: true,
        })
    }

    async fn push_credit_note(
        &self,
        note: &ExportCreditNote,
    ) -> Result<ExportReceipt, ExportError> {
        if note.customer_ref.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "customer_ref (QBO CustomerRef.value) is required to create a credit memo".into(),
            ));
        }
        if note.invoice_external_id.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "invoice_external_id (the QBO invoice Id) is required: an unlinked credit memo \
                 is a second document for the same money"
                    .into(),
            ));
        }
        if note.lines.is_empty() {
            return Err(ExportError::InvalidRequest(
                "credit note has no lines".into(),
            ));
        }

        let body = QboCreditMemo {
            customer_ref: QboRef {
                value: note.customer_ref.clone(),
            },
            txn_date: note.issued_on.to_string(),
            doc_number: note.number.clone(),
            currency_ref: Some(QboRef {
                value: note.currency.clone(),
            }),
            private_note: Some(format!(
                "{} {}",
                note.reason
                    .clone()
                    .map(|r| format!("{r} "))
                    .unwrap_or_default(),
                marker("credit_note", note.credit_note_id)
            )),
            linked_txn: vec![QboLinkedTxn {
                txn_id: note.invoice_external_id.clone(),
                txn_type: "Invoice",
            }],
            line: qbo_lines(&note.lines, note.tax_code.as_deref())?,
        };

        let wire = serde_json::to_value(&body)
            .map_err(|e| ExportError::InvalidRequest(format!("serialize credit memo: {e}")))?;
        let response: QboResponse = serde_json::from_value(
            self.post(&self.endpoint("creditmemo"), wire).await?,
        )
        .map_err(|e| ExportError::Vendor {
            status: 200,
            message: format!("missing CreditMemo entity in response: {e}"),
        })?;
        let entity = response.credit_memo.ok_or_else(|| ExportError::Vendor {
            status: 200,
            message: "QBO response carried no CreditMemo".into(),
        })?;
        Ok(ExportReceipt {
            external_id: entity.id,
            external_number: (!entity.doc_number.is_empty()).then_some(entity.doc_number),
            created: true,
        })
    }

    async fn read_accounts(&self) -> Result<Vec<LedgerAccount>, ExportError> {
        let accounts = self.query_all("Account", |body| body.accounts).await?;
        Ok(accounts
            .into_iter()
            .map(|a| LedgerAccount {
                code: a
                    .acct_num
                    .map(|c| c.trim().to_string())
                    .filter(|c| !c.is_empty()),
                id: a.id,
                name: a.name,
                kind: account_kind(&a.account_type),
                active: a.active,
            })
            .collect())
    }

    async fn read_tax_codes(&self) -> Result<Vec<TaxCode>, ExportError> {
        // A tax code's rate lives on the TaxRate entities its list points at,
        // and a group points at several. Rates are fetched once and indexed so
        // each code resolves without a per-code request.
        let rates = self.query_all("TaxRate", |body| body.tax_rates).await?;
        let rate_by_id: std::collections::HashMap<String, Option<f64>> = rates
            .into_iter()
            .map(|r| (r.id, if r.active { r.rate_value } else { None }))
            .collect();

        let codes = self.query_all("TaxCode", |body| body.tax_codes).await?;
        Ok(codes
            .into_iter()
            .map(|c| {
                let components: Vec<f64> = c
                    .sales_tax_rate_list
                    .unwrap_or_default()
                    .tax_rate_detail
                    .into_iter()
                    .filter_map(|d| {
                        let id = d.tax_rate_ref.unwrap_or_default().value;
                        rate_by_id.get(&id).copied().flatten()
                    })
                    .collect();
                let rate_bps = if components.is_empty() {
                    None
                } else {
                    let sum: f64 = components.iter().sum();
                    Some((sum * 100.0).round() as i32)
                };
                TaxCode {
                    code: c.id,
                    name: c.name,
                    rate_bps,
                    active: c.active,
                }
            })
            .collect())
    }

    async fn find_document(
        &self,
        lookup: &ExportLookup,
    ) -> Result<Option<ExportReceipt>, ExportError> {
        let (entity, kind_marker) = match lookup.kind {
            ExportLookupKind::Invoice => ("Invoice", "invoice"),
            ExportLookupKind::CreditNote => ("CreditMemo", "credit_note"),
        };
        if lookup.number.trim().is_empty() {
            return Err(ExportError::InvalidRequest(
                "a reconcile lookup needs the document number the first attempt sent".into(),
            ));
        }
        // Numbers are ours, but a single quote would end the query literal.
        let number = lookup.number.replace('\'', "''");
        let needle = marker(kind_marker, lookup.marula_id);
        let body = self
            .query(&format!(
                "select * from {entity} where DocNumber = '{number}'"
            ))
            .await?;

        let candidates = match lookup.kind {
            ExportLookupKind::Invoice => body.invoices,
            ExportLookupKind::CreditNote => body.credit_memos,
        };
        for candidate in candidates {
            if candidate.private_note.contains(&needle) {
                return Ok(Some(ExportReceipt {
                    external_id: candidate.id,
                    external_number: (!candidate.doc_number.is_empty())
                        .then_some(candidate.doc_number),
                    created: false,
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
