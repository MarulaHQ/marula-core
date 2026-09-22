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
    /// The provider processed the attempt and refused it; the typed
    /// component is a [`PaymentDeclinedCode`] so the 409 envelope is
    /// actionable (`payment_declined` + `code` extra), with the raw
    /// provider reason preserved as the non-primary detail.
    #[error("provider declined the charge ({code:?}): {reason}")]
    Declined {
        reason: String,
        code: PaymentDeclinedCode,
    },
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
    /// Provider-side payment-method surface (PSP token, e.g. a Stripe
    /// `pm_*` id confirmed by the embedded form). `None` on the none
    /// provider (ledger-only settlement).
    #[allow(dead_code)]
    pub payment_method: Option<String>,
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
                code: PaymentDeclinedCode::Generic,
            });
        }
        if request.currency.trim().is_empty() {
            return Err(PaymentError::Declined {
                reason: "currency is required".into(),
                code: PaymentDeclinedCode::CurrencyNotSupported,
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
            payment_method: None,
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

// ---------------------------------------------------------------------------
// Stripe adapter (Wave 22): PaymentIntents confirm + typed decline codes.
// ---------------------------------------------------------------------------

/// Typed decline component of [`PaymentError::Declined`] — the
/// `payment_declined` envelope reason carries the code + keeps the raw
/// provider text in the extra field (never third-party prose as the
/// primary message per the error-envelope pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentDeclinedCode {
    /// The card (or instrument) refused the charge without a more
    /// specific reason we can distinguish.
    CardDeclined,
    InsufficientFunds,
    ExpiredCard,
    IncorrectCvc,
    /// Issuer rule (e.g. do_not_honor) — retry rarely helps, contact
    /// the instrument holder instead.
    DoNotHonor,
    /// A currency or account constraint prevents the charge.
    CurrencyNotSupported,
    /// Provider-side malfunction — retry may succeed later.
    ProcessingError,
    /// Everything else, still declined and typed.
    Generic,
}

impl PaymentDeclinedCode {
    /// Canonical wire code (the `payment_declined` envelope's `code`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CardDeclined => "card_declined",
            Self::InsufficientFunds => "insufficient_funds",
            Self::ExpiredCard => "expired_card",
            Self::IncorrectCvc => "incorrect_cvc",
            Self::DoNotHonor => "do_not_honor",
            Self::CurrencyNotSupported => "currency_not_supported",
            Self::ProcessingError => "processing_error",
            Self::Generic => "generic",
        }
    }

    /// Maps Stripe decline codes (and processor codes) to the typed set.
    /// Unknown decline codes NEVER silently upgrade to success — they
    /// stay declined, typed `generic`, with the raw text preserved.
    pub fn from_stripe_decline_code(raw: &str) -> Self {
        match raw {
            "card_declined" | "generic_decline" | "try_again_later" => Self::CardDeclined,
            "insufficient_funds" => Self::InsufficientFunds,
            "expired_card" => Self::ExpiredCard,
            "incorrect_cvc" | "incorrect_cvc_format" => Self::IncorrectCvc,
            "do_not_honor" => Self::DoNotHonor,
            "currency_not_supported" => Self::CurrencyNotSupported,
            "processing_error" | "temporary_processing_error" => Self::ProcessingError,
            "authentication_required" => Self::DoNotHonor,
            _ => Self::Generic,
        }
    }
}

/// Maps an amount in minor units to the zero-decimal minor units Stripe
/// accepts for the given currency (zero-decimal: JPY; everything else
/// keeps minor units verbatim).
fn stripe_amount(amount_minor: i64, currency: &str) -> i64 {
    match currency.to_ascii_lowercase().as_str() {
        "jpy" | "krw" | "vnd" | "clp" | "xaf" | "xof" | "xpf" | "ugx" | "rwf" => amount_minor,
        _ => amount_minor,
    }
}

/// Stripe PaymentIntents adapter (Wave 22). Async travel through
/// `confirm=true` + a caller-supplied `payment_method`. Confirmed,
/// succeeded intents settle synchronously — exactly the seam contract —
/// and every decline rides a typed [`PaymentDeclinedCode`]; `webhook`
/// reconciliation covers late-arriving async states.
pub struct StripePaymentProvider {
    secret_key: String,
    /// Immutable base URL (the env-overridable endpoint is how the
    /// conformance suite pins the adapter against a local mock PSP).
    api_base: String,
    client: reqwest::Client,
}

impl StripePaymentProvider {
    /// The live provider: Stripe's documented restricted-key advice
    /// applies — this takes the ALREADY-configured key string; the
    /// secret is never logged or returned.
    pub fn new(secret_key: String) -> Self {
        Self {
            secret_key,
            api_base: "https://api.stripe.com".into(),
            client: reqwest::Client::new(),
        }
    }

    /// Test seam only: pins `api_base` at a mock PSP endpoint.
    pub fn with_api_base(mut self, api_base: String) -> Self {
        self.api_base = api_base;
        self
    }

    fn form_error(&self, response: &str) -> PaymentError {
        // Stripe error envelopes: { error: { message, code, decline_code } }
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(response);
        match parsed.ok().as_ref().and_then(|v| v.get("error")) {
            Some(err_obj) => {
                let decline_code = err_obj
                    .get("decline_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let message = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("the payment processor refused the request");
                let kind = err_obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if kind.contains("card_error") || !decline_code.is_empty() {
                    PaymentError::Declined {
                        reason: message.to_string(),
                        code: PaymentDeclinedCode::from_stripe_decline_code(decline_code),
                    }
                } else {
                    PaymentError::Unavailable(format!("stripe request refused: {message}"))
                }
            }
            None => PaymentError::Unavailable(format!(
                "stripe request rejected (HTTP {response_len})",
                response_len = response.len()
            )),
        }
    }
}

#[async_trait::async_trait]
impl PaymentProvider for StripePaymentProvider {
    fn name(&self) -> &str {
        "stripe"
    }

    async fn charge(&self, request: &ChargeRequest) -> Result<ChargeReceipt, PaymentError> {
        if request.amount_minor <= 0 {
            return Err(PaymentError::Declined {
                reason: "amount_minor must be positive".into(),
                code: PaymentDeclinedCode::Generic,
            });
        }
        let currency = request.currency.trim().to_lowercase();
        if currency.is_empty() {
            return Err(PaymentError::Declined {
                reason: "currency is required".into(),
                code: PaymentDeclinedCode::CurrencyNotSupported,
            });
        }
        let payment_method = request
            .payment_method
            .as_deref()
            .map(str::trim)
            .unwrap_or("");
        if payment_method.is_empty() {
            return Err(PaymentError::Unavailable(
                "stripe charges need a confirmed payment method (the portal collects it via the embedded payment form)".into(),
            ));
        }
        let amount = stripe_amount(request.amount_minor, &currency);
        let idempotency_key = format!("meridian-charge-{}", request.invoice_id);
        let params: Vec<(&str, String)> = vec![
            ("amount", amount.to_string()),
            ("currency", currency.to_string()),
            ("confirm", "true".into()),
            ("payment_method", payment_method.to_string()),
            (
                "description",
                format!("Meridian invoice {}", request.invoice_number),
            ),
            (
                "metadata[meridian_invoice_id]",
                request.invoice_id.to_string(),
            ),
            ("metadata[meridian_org_id]", request.org_id.to_string()),
        ];
        let response = self
            .client
            .post(format!("{}/v1/payment_intents", self.api_base))
            .bearer_auth(&self.secret_key)
            .header("Stripe-Idempotency-Key", idempotency_key)
            .header("Stripe-Version", "2026-08-26.dahlia")
            .form(&params)
            .send()
            .await
            .map_err(|e| PaymentError::Unavailable(format!("the PSP is unreachable: {e}")))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(self.form_error(&body));
        }
        let intent: serde_json::Value = serde_json::from_str(&body)
            .map_err(|_| PaymentError::Unavailable("PSP returned unreadable JSON".into()))?;
        if intent.get("succeeded").and_then(|v| v.as_bool()) != Some(true) {
            let decline_code = intent
                .pointer("/last_payment_error/decline_code")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let message = intent
                .pointer("/last_payment_error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("the payment was not completed");
            return Err(PaymentError::Declined {
                reason: message.to_string(),
                code: if decline_code.is_empty() {
                    PaymentDeclinedCode::ProcessingError
                } else {
                    PaymentDeclinedCode::from_stripe_decline_code(decline_code)
                },
            });
        }
        let pi_id = intent
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                PaymentError::Unavailable("PSP response has no payment intent id".into())
            })?;
        Ok(ChargeReceipt {
            provider: "stripe".into(),
            provider_ref: Some(pi_id),
        })
    }
}

#[cfg(test)]
mod stripe_tests {
    use super::*;

    fn request_with_method() -> ChargeRequest {
        ChargeRequest {
            org_id: Uuid::now_v7(),
            invoice_id: Uuid::now_v7(),
            invoice_number: "INV-000001".into(),
            amount_minor: 5_000,
            currency: "USD".into(),
            payment_method: Some("pm_card_visa".into()),
        }
    }

    #[test]
    fn stripe_decline_codes_map_to_the_typed_set() {
        assert_eq!(
            PaymentDeclinedCode::from_stripe_decline_code("insufficient_funds"),
            PaymentDeclinedCode::InsufficientFunds
        );
        assert_eq!(
            PaymentDeclinedCode::from_stripe_decline_code("do_not_honor"),
            PaymentDeclinedCode::DoNotHonor
        );
        // Unknown decline codes NEVER land in success — typed generic.
        assert_eq!(
            PaymentDeclinedCode::from_stripe_decline_code("some_future_code"),
            PaymentDeclinedCode::Generic
        );
    }

    #[test]
    fn zero_decimal_currencies_keep_minors() {
        assert_eq!(stripe_amount(1_990, "USD"), 1_990);
        assert_eq!(stripe_amount(1_990, "jpy"), 1_990);
    }

    #[tokio::test]
    async fn stripe_adapter_maps_card_declines_to_typed_codes() {
        // In-process mock PSP (bound loopback listener): serves ONE
        // pinned decline envelope at the charge route; the adapter must
        // map it to the typed code. Loopback only, no secrets.
        let base_url = mock_psp().await;
        let provider = StripePaymentProvider::new("rk_test_x".into()).with_api_base(base_url);
        let err = provider.charge(&request_with_method()).await.unwrap_err();
        assert!(matches!(
            err,
            PaymentError::Declined {
                code: PaymentDeclinedCode::InsufficientFunds,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn stripe_adapter_rejects_blank_currency_as_unsupported() {
        let provider = StripePaymentProvider::new("rk_test_x".into());
        let mut req = request_with_method();
        req.currency = "  ".into();
        let err = provider.charge(&req).await.unwrap_err();
        assert!(matches!(
            err,
            PaymentError::Declined {
                code: PaymentDeclinedCode::CurrencyNotSupported,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn stripe_adapter_refuses_charges_without_a_payment_method() {
        let base_url = mock_psp().await;
        let provider = StripePaymentProvider::new("rk_test_x".into()).with_api_base(base_url);
        let mut req = request_with_method();
        req.payment_method = None;
        assert!(matches!(
            provider.charge(&req).await,
            Err(PaymentError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn none_declines_keep_typed_codes() {
        let mut req = request_with_method();
        req.amount_minor = 0;
        let err = NonePaymentProvider.charge(&req).await.unwrap_err();
        assert!(matches!(err, PaymentError::Declined { .. }));
    }

    /// Stub HTTP app bound on loopback serving ONE pinned decline
    /// envelope at POST /v1/payment_intents. No real network leaves the
    /// process.
    async fn mock_psp() -> String {
        use axum::response::IntoResponse;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback for the mock PSP");
        let addr = listener.local_addr().expect("local addr");
        let router = axum::Router::new().route(
            "/v1/payment_intents",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::PAYMENT_REQUIRED,
                    axum::Json(serde_json::json!({
                        "error": {
                            "type": "card_error",
                            "message": "Your card has insufficient funds.",
                            "decline_code": "insufficient_funds"
                        }
                    })),
                )
                    .into_response()
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        format!("http://{addr}")
    }
}
