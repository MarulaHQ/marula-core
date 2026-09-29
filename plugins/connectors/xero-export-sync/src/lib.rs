//! Xero export-sync adapter (TASKLIST Phase 3 adapters line).
//!
//! Purpose: push Marula invoices and payments into a customer-provided
//! Xero tenant over the PUBLIC Xero accounting API (`/api.xro/2.0/...`) —
//! HTTP only, no vendor SDK. OAuth2 access tokens are operator-supplied
//! (BYOK: sealed at rest in the core's `byok_secrets` store under provider
//! id `xero`; Marula hosts no OAuth app). Background sync scheduling is
//! a future worker job — this crate is the edge adapter only.
//!
//! None mode: [`NoneExportSink`] is the shipped default — it performs zero
//! network I/O and returns [`ExportError::Disabled`], so an org with no
//! accounting credentials operates fully standalone (standalone-first
//! doctrine). Every call site must treat `Disabled` as "would have synced".
//!
//! Non-purpose: no OAuth token refresh (operators rotate the token in the
//! BYOK store), no Xero contact/Account catalog management, no reverse
//! (Xero→Marula) import.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
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

/// Typed error surfaced across the export seam. Vendor detail is
/// stringified into `reason`s — no vendor SDK types leak past this crate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// None mode (no credentials configured). Zero network was performed.
    #[error("export sink disabled: no credentials configured (none mode)")]
    Disabled,
    /// The caller omitted data the vendor requires.
    #[error("invalid export request: {0}")]
    InvalidRequest(String),
    /// The vendor rejected the credential (401/403) — token expired or
    /// revoked; operator rotates it in the BYOK store.
    #[error("export unauthorized: {0}")]
    Unauthorized(String),
    /// The vendor asked us to back off (429).
    #[error("export rate limited: {0}")]
    RateLimited(String),
    /// Vendor-side rejection or outage (other 4xx/5xx).
    #[error("export vendor error (status {status}): {message}")]
    Vendor { status: u16, message: String },
    /// Transport failure (DNS, TLS, timeout).
    #[error("export transport failure: {0}")]
    Network(String),
}

// --- the seam ----------------------------------------------------------------

/// The vendor-neutral export seam. Every QBO/Xero-style accounting sink
/// implements this; none mode is [`NoneExportSink`].
#[async_trait]
pub trait ExportSink: Send + Sync {
    fn name(&self) -> &str;

    async fn push_invoice(&self, invoice: &ExportInvoice) -> Result<ExportReceipt, ExportError>;

    async fn push_payment(&self, payment: &ExportPayment) -> Result<ExportReceipt, ExportError>;
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
}

// --- Xero reference implementation ---------------------------------------------

/// Credentials + endpoint configuration for [`XeroClient`]. Values come
/// from the BYOK sealed store (decrypted just-in-time by the caller, never
/// logged, never serialized).
#[derive(Debug, Clone)]
pub struct XeroConfig {
    /// Xero OAuth2 access token with `accounting.transactions` scope.
    pub access_token: String,
    /// The target tenant's id (`Xero-tenant-id` header).
    pub tenant_id: String,
    /// API base — overridable for tests; production default is the public
    /// API host.
    pub base_url: String,
}

impl XeroConfig {
    pub fn new(access_token: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            tenant_id: tenant_id.into(),
            base_url: "https://api.xero.com".to_string(),
        }
    }
}

/// Reference HTTP implementation of [`ExportSink`] against the public Xero
/// accounting API (`POST /api.xro/2.0/Invoices`, `POST /api.xro/2.0/Payments`).
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

/// Minimal slice of the Xero Invoices/Payments response JSON: Xero wraps
/// created entities in a `{ "...": [ { ... } ] }` array.
#[derive(Debug, Deserialize)]
struct XeroResponse {
    #[serde(rename = "Invoices", default)]
    invoices: Vec<XeroEntity>,
    #[serde(rename = "Payments", default)]
    payments: Vec<XeroEntity>,
}

#[derive(Debug, Deserialize)]
struct XeroEntity {
    #[serde(rename = "InvoiceID", default)]
    invoice_id: String,
    #[serde(rename = "PaymentID", default)]
    payment_id: String,
    #[serde(rename = "InvoiceNumber", default)]
    invoice_number: String,
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

fn auth_error(status: reqwest::StatusCode, body: &str) -> ExportError {
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ExportError::Unauthorized(format!("Xero returned {status}"))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            ExportError::RateLimited(format!("Xero returned 429: {body}"))
        }
        s => ExportError::Vendor {
            status: s.as_u16(),
            message: body.chars().take(500).collect(),
        },
    }
}

impl XeroClient {
    pub fn new(config: XeroConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
    }

    fn endpoint(&self, resource: &str) -> String {
        format!(
            "{}/api.xro/2.0/{}",
            self.config.base_url.trim_end_matches('/'),
            resource
        )
    }

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
            .map_err(|e| ExportError::Network(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ExportError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(auth_error(status, &text));
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

        let mut xero_lines = Vec::with_capacity(invoice.lines.len());
        for line in &invoice.lines {
            xero_lines.push(XeroLineItem {
                description: line.description.clone(),
                quantity: qty_to_number(&line.quantity)?,
                unit_amount: minor_to_number(line.unit_amount_minor),
            });
        }

        let body = XeroInvoice {
            invoice_type: "ACCREC",
            contact: XeroContact {
                contact_id: invoice.customer_ref.clone(),
            },
            line_items: xero_lines,
            date: invoice.issue_date.to_string(),
            due_date: invoice.due_date.to_string(),
            invoice_number: invoice.number.clone(),
            reference: format!("[marula:invoice:{}]", invoice.invoice_id),
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
}

#[cfg(test)]
mod tests;
