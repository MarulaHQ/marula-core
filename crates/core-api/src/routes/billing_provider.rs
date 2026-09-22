//! The PSP provider RESOLVER (Wave 22): org-typed settings + an env
//! secret decide which [`PaymentProvider`] implementation a payment
//! mutation uses.
//!
//! Posture:
//! * `MERIDIAN_STRIPE_SECRET_KEY` (a RESTRICTED `rk_` key per Stripe's
//!   key guidance) is the ONLY way Stripe mode exists — the key is a
//!   secret like every other app credential; it is never stored, logged,
//!   or returned.
//! * an org opts in per-org with the typed org-scope settings key
//!   `billing.payment_provider = "stripe"`; without the opt-in the org
//!   keeps none-mode (ledger-only receipts) even when the env is set.
//! * re-keyed/malformed configuration fails CLOSED to none-mode — a
//!   broken PSP config can never silently start charging on a
//!   different provider.
//!
//! Non-purpose: no Checkout Session / hosted check-out surface yet — the
//! portal collects the confirmed payment method through its embedded
//! payment form and the adapter confirms a PaymentIntent synchronously
//! (`charge` runs inside the payment mutation's transaction, per the
//! seam design note).
use crate::state::AppState;

/// Resolves the provider for an org's payment mutation. Only the
/// Stripe adapter is org-configurable; everything else stays none-mode.
pub(crate) async fn resolve_payment_provider(
    state: &AppState,
    org_id: uuid::Uuid,
) -> Result<std::sync::Arc<dyn core_billing::PaymentProvider>, crate::error::ApiError> {
    let stripe_key = std::env::var("MERIDIAN_STRIPE_SECRET_KEY")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());
    if stripe_key.is_none() {
        return Ok(std::sync::Arc::new(core_billing::NonePaymentProvider));
    }
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(crate::error::ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let overrides = core_db::settings::list_org_overrides(&mut *tx, org_id)
        .await
        .map_err(crate::error::ApiError::from)?;
    tx.commit().await.map_err(crate::error::ApiError::from)?;
    let opted_in = overrides
        .into_iter()
        .any(|s| s.key == "billing.payment_provider" && s.value.as_str() == Some("stripe"));
    if opted_in {
        Ok(std::sync::Arc::new(
            core_billing::StripePaymentProvider::new(stripe_key.expect("checked above")),
        ))
    } else {
        Ok(std::sync::Arc::new(core_billing::NonePaymentProvider))
    }
}
