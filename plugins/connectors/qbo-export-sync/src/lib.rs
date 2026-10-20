//! QuickBooks Online export-sync adapter (the private build tracker Phase 3 adapters line).
//!
//! Purpose: push Marula invoices and payments into a customer-provided
//! QBO company over the PUBLIC QBO REST API (`/v3/company/{realmId}/...`)
//! — HTTP only, no vendor SDK. OAuth2 access tokens are operator-supplied
//! (BYOK: sealed at rest in the core's `byok_secrets` store under provider
//! id `qbo`; Marula hosts no OAuth app). Background sync scheduling is a
//! future worker job — this crate is the edge adapter only.
//!
//! None mode: [`NoneExportSink`] is the shipped default — it performs zero
//! network I/O and returns [`ExportError::Disabled`], so an org with no
//! accounting credentials operates fully standalone (standalone-first
//! doctrine). Every call site must treat `Disabled` as "would have synced".
//!
//! Non-purpose: no OAuth token refresh (operators rotate the token in the
//! BYOK store), no QBO customer/item catalog management, no reverse
//! (QBO→Marula) import.

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
    /// The vendor's customer id (QBO `CustomerRef.value` / Xero
    /// `Contact.ContactID`). Export adapters never invent customers.
    pub customer_ref: String,
    pub issue_date: NaiveDate,
    pub due_date: NaiveDate,
    /// ISO 4217 code ("USD").
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
    /// Vendor customer id (same value as the invoice's `customer_ref`).
    pub customer_ref: String,
    /// Payment date.
    pub applied_on: NaiveDate,
    /// Amount in minor units.
    pub amount_minor: i64,
    pub currency: String,
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
#[async_trait::async_trait]
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

// --- QBO reference implementation ---------------------------------------------

/// Credentials + endpoint configuration for [`QboClient`]. Values come from
/// the BYOK sealed store (decrypted just-in-time by the caller, never
/// logged, never serialized).
#[derive(Debug, Clone)]
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
}

impl QboConfig {
    pub fn new(access_token: impl Into<String>, realm_id: impl Into<String>) -> Self {
        Self {
            access_token: access_token.into(),
            realm_id: realm_id.into(),
            base_url: "https://quickbooks.api.intuit.com".to_string(),
            minor_version: "75".to_string(),
        }
    }
}

/// Reference HTTP implementation of [`ExportSink`] against the public QBO
/// REST API (`/v3/company/{realmId}/invoice` and `/payment`).
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

/// Minimal slice of the QBO Invoice/Payment response JSON.
#[derive(Debug, Deserialize)]
struct QboResponse {
    #[serde(rename = "Invoice", default)]
    invoice: Option<QboEntity>,
    #[serde(rename = "Payment", default)]
    payment: Option<QboEntity>,
}

#[derive(Debug, Deserialize)]
struct QboEntity {
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "DocNumber", default)]
    doc_number: String,
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

fn auth_error(status: reqwest::StatusCode, body: &str) -> ExportError {
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ExportError::Unauthorized(format!("QBO returned {status}"))
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            ExportError::RateLimited(format!("QBO returned 429: {body}"))
        }
        s => ExportError::Vendor {
            status: s.as_u16(),
            message: body.chars().take(500).collect(),
        },
    }
}

impl QboClient {
    pub fn new(config: QboConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
        }
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
            message: format!("unparseable QBO response: {e}"),
        })
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

        let mut qbo_lines = Vec::with_capacity(invoice.lines.len());
        for line in &invoice.lines {
            let qty = qty_to_number(&line.quantity)?;
            let amount_minor =
                (line.unit_amount_minor as f64 * qty.as_f64().unwrap_or(0.0)).round() as i64;
            qbo_lines.push(QboLine {
                detail_type: "SalesItemLine",
                amount: minor_to_number(amount_minor),
                description: line.description.clone(),
                detail: QboLineDetail {
                    qty,
                    unit_price: minor_to_number(line.unit_amount_minor),
                },
            });
        }

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
                "{}[marula:invoice:{}]",
                invoice
                    .notes
                    .clone()
                    .map(|n| format!("{n} "))
                    .unwrap_or_default(),
                invoice.invoice_id
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
}

#[cfg(test)]
mod tests;
