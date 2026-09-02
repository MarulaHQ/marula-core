//! The `PaymentProvider` seam (TASKLIST Phase 3, native invoicing v1):
//! how recorded payments reach the outside world.
//!
//! v1 ships exactly one implementation, [`NonePaymentProvider`]: the
//! built-in none-mode provider that approves every charge locally with a
//! deterministic reference and makes NO external PSP calls. The trait
//! exists so a PSP adapter can land later without touching the invoice
//! flow — the API layer resolves the provider once and hands it a
//! [`ChargeRequest`]; everything provider-specific stays behind
//! [`PaymentProvider::charge`].
//!
//! Design note: `charge` is called INSIDE the payment mutation's
//! database transaction (after the idempotency claim). Holding row locks
//! across a PSP round-trip is the price of keeping receipt, payment row,
//! and invoice status atomic; the none-mode provider is instantaneous,
//! and a real adapter that prefers out-of-band settlement should fail
//! the write and reconcile via its own webhook instead.
//!
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PaymentError {
    /// The provider processed the attempt and refused it.
    #[error("provider declined the charge: {reason}")]
    Declined { reason: String },
    /// The provider could not be reached or is misconfigured; the
    /// payment must not be recorded.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
}

/// One charge attempt against an issued invoice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeRequest {
    #[allow(dead_code)]
    pub org_id: Uuid,
    #[allow(dead_code)]
    pub invoice_id: Uuid,
    /// The invoice's human number, for provider-side references.
    pub invoice_number: String,
    /// Amount to collect, minor units. Always positive; overpayments
    /// are refused by the API layer before the provider sees them.
    pub amount_minor: i64,
    /// ISO 4217 code; always the invoice's own currency.
    pub currency: String,
}

/// What a successful charge hands back; persisted verbatim on the
/// payment row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChargeReceipt {
    /// Provider name recorded on the receipt (`"none"` in v1).
    pub provider: String,
    /// Provider-side reference (`None` when the provider mints none).
    pub provider_ref: Option<String>,
}

/// The payment seam. Implementations must be non-panicking and
/// cancel-safe (no unbounded waits); every outcome is expressed as
/// `Ok(receipt)` or a typed [`PaymentError`].
#[async_trait::async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Human provider name recorded on receipts.
    fn name(&self) -> &str;

    /// Attempts to collect the requested amount. Returns a receipt on
    /// success or a typed error — never panics.
    async fn charge(&self, request: &ChargeRequest) -> Result<ChargeReceipt, PaymentError>;
}

/// Built-in none-mode provider: no external PSP calls, every well-formed
/// charge is approved locally with the deterministic reference
/// `local-{invoice_id}` (so replays of the same charge id are
/// reconcilable without provider state).
pub struct NonePaymentProvider;

#[async_trait::async_trait]
impl PaymentProvider for NonePaymentProvider {
    fn name(&self) -> &str {
        "none"
    }

    async fn charge(&self, request: &ChargeRequest) -> Result<ChargeReceipt, PaymentError> {
        if request.amount_minor <= 0 {
            return Err(PaymentError::Declined {
                reason: "amount_minor must be positive".into(),
            });
        }
        if request.currency.trim().is_empty() {
            return Err(PaymentError::Declined {
                reason: "currency is required".into(),
            });
        }
        Ok(ChargeReceipt {
            provider: "none".into(),
            provider_ref: Some(format!("local-{}", request.invoice_id)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(amount: i64) -> ChargeRequest {
        ChargeRequest {
            org_id: Uuid::now_v7(),
            invoice_id: Uuid::now_v7(),
            invoice_number: "INV-000001".into(),
            amount_minor: amount,
            currency: "USD".into(),
        }
    }

    #[tokio::test]
    async fn none_provider_approves_well_formed_charges() {
        let req = request(5_000);
        let receipt = NonePaymentProvider.charge(&req).await.unwrap();
        assert_eq!(receipt.provider, "none");
        assert_eq!(
            receipt.provider_ref,
            Some(format!("local-{}", req.invoice_id))
        );
    }

    #[tokio::test]
    async fn none_provider_declines_non_positive_amounts() {
        let err = NonePaymentProvider.charge(&request(0)).await.unwrap_err();
        assert!(matches!(err, PaymentError::Declined { .. }));
        assert!(NonePaymentProvider.charge(&request(-1)).await.is_err());
    }

    #[tokio::test]
    async fn none_provider_declines_missing_currency() {
        let mut req = request(1_000);
        req.currency = "  ".into();
        let err = NonePaymentProvider.charge(&req).await.unwrap_err();
        assert!(matches!(err, PaymentError::Declined { .. }));
    }
}
