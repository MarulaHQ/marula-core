//! Stripe WEBHOOK receiver (Wave 22): `POST /api/webhooks/stripe`.
//!
//! Posture (stripe-best-practices binding rules):
//! * the webhook is NEVER optional decoration — it reconciles
//!   out-of-band PSP state (`payment_intent.succeeded` → a payment row
//!   recorded exactly once; `payment_intent.payment_failed` → an
//!   audit-only typed decline record);
//! * signature verification is DENY-BY-DEFAULT: the `Stripe-Signature`
//!   v1 scheme (`t=…,v1=<hmac-sha256(secret, "t.payload")>`) is checked
//!   against `MARULA_STRIPE_WEBHOOK_SECRET`, timestamp skew ≤ 5 min,
//!   and a request without/with a bad signature is a typed 403, never a
//!   processed event;
//! * idempotence: one provider reference settles money once per invoice, and
//!   the DATABASE says so — `invoice_payments` carries a unique index on
//!   `(org_id, invoice_id, provider_ref)` (migration 0165), and the receipt
//!   insert below uses `ON CONFLICT DO NOTHING`. A redelivered event, including
//!   one that races the first delivery, inserts nothing and answers 200 with
//!   `acted: false`. The lookup before the insert is a fast path, not the
//!   decision;
//! * settlement is decided by the statement that writes it: the total the
//!   event was priced on rides in the UPDATE's predicate next to the payable
//!   status and the outstanding balance, so a total that moved between the
//!   read and the write is a typed 409 rather than a silent settlement. A
//!   retry of that delivery re-reads the invoice inside its own transaction
//!   and settles once against the total that is actually there;
//! * the raw event payload is never persisted (only typed extracts);
//!   webhook secrets are config, never logged.
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{Extensions, HeaderMap},
    Json,
};
use serde_json::Value;
use uuid::Uuid;

const SKEW_SECONDS: i64 = 300;

/// How many `v1` candidates one request may present.
///
/// Stripe signs with every active signing secret, which is two during a
/// rotation (the new secret and the old one being retired); the docs tell
/// an endpoint to accept a signature that matches ANY of them. Four is
/// headroom over that rotation shape and a hard ceiling on the work an
/// unauthenticated caller can buy: without it the cost of one request is
/// one full-body HMAC per comma-separated part the header carries, bounded
/// only by the ingress header limit.
const MAX_V1_CANDIDATES: usize = 4;

/// Splits a `Stripe-Signature` header into its timestamp and its `v1`
/// candidates, refusing a header that carries more candidates than a
/// rotation can explain. Pure — unit-tested without a request.
fn parse_signature_header(raw: &str) -> Result<(i64, Vec<String>), ApiError> {
    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.trim() {
            "t" => timestamp = v.trim().parse().ok(),
            "v1" => {
                if signatures.len() >= MAX_V1_CANDIDATES {
                    // A real Stripe sends one candidate, or two while a secret
                    // rotation is live. Reaching the cap means either an attack
                    // or a rotation with more secrets live than the bound
                    // explains, and the two look identical in the response, so
                    // the log is where an operator can tell them apart.
                    tracing::warn!(
                        cap = MAX_V1_CANDIDATES,
                        "stripe signature header exceeds the v1 candidate cap: refusing the webhook. A live rotation with more than two secrets would look the same"
                    );
                    return Err(ApiError::StripeWebhookRejected(
                        "the signature header carries more v1 candidates than a secret rotation explains",
                    ));
                }
                signatures.push(v.trim().to_string());
            }
            _ => {}
        }
    }
    let Some(t) = timestamp else {
        return Err(ApiError::StripeWebhookRejected(
            "the signature timestamp is missing",
        ));
    };
    Ok((t, signatures))
}

/// Verifies the Stripe-Signature header against the instance's webhook
/// secret (Stripe's v1 scheme: `t=…,v1=<hmac-sha256(secret, "t.body")>`,
/// computed with the SAME `core_mail::hmac` helpers as every other
/// machine surface — one hmac method, one constant-time posture).
pub(crate) fn verify_stripe_signature(
    headers: &HeaderMap,
    payload: &[u8],
    secret: &str,
) -> Result<(), ApiError> {
    let raw = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::StripeWebhookRejected("a Stripe-Signature header is required"))?;
    let (t, signatures) = parse_signature_header(raw)?;
    // Timestamp skew: fresh events only (replay of old captures dies here).
    let age = (chrono::Utc::now().timestamp() - t).abs();
    if age > SKEW_SECONDS {
        return Err(ApiError::StripeWebhookRejected(
            "the signature timestamp is outside the acceptance window",
        ));
    }
    if signatures.is_empty() {
        return Err(ApiError::StripeWebhookRejected("no v1 signature present"));
    }
    let signed = format!("{t}.{}", String::from_utf8_lossy(payload));
    let verified = signatures
        .iter()
        .any(|s| core_mail::hmac::verify(secret.as_bytes(), signed.as_bytes(), s));
    if !verified {
        return Err(ApiError::StripeWebhookRejected(
            "the v1 signature does not verify",
        ));
    }
    Ok(())
}

#[utoipa::path(operation_id = "stripe_webhook_receive", post, path = "/api/webhooks/stripe", tag = "billing",
    request_body(content = Vec<u8>, content_type = "application/json"),
    responses((status = 200, body = Value), (status = 400), (status = 403),
              (status = 409, description = "the invoice refused the settlement: it left its payable state, its total moved since the event was priced, or the amount exceeds the balance")))]
pub async fn receive(
    State(state): State<AppState>,
    extensions: Extensions,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let _ = extensions;
    let secret = std::env::var("MARULA_STRIPE_WEBHOOK_SECRET")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::StripeWebhookRejected(
                "no webhook secret configured for this instance; refusing",
            )
        })?;
    verify_stripe_signature(&headers, &body, &secret)?;
    let event: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::Validation("the webhook body is not an event document".into()))?;
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let intent = event
        .pointer("/data/object")
        .cloned()
        .unwrap_or(Value::Null);
    let intent_id = intent
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let org_id = intent
        .pointer("/metadata/marula_org_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let invoice_id = intent
        .pointer("/metadata/marula_invoice_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let amount_minor = intent.get("amount_received").and_then(|v| v.as_i64());
    let currency = intent
        .get("currency")
        .and_then(|v| v.as_str())
        .map(str::to_uppercase)
        .unwrap_or_default();
    let mut recorded = false;
    match event_type {
        "payment_intent.succeeded" => {
            let (Some(org), Some(invoice), Some(amount)) = (org_id, invoice_id, amount_minor)
            else {
                // Not a Marula-tagged event (foreign webhook traffic) —
                // accepted (200) but not acted on: webhooks MUST answer
                // 200 even for foreign/unknown events so Stripe retries
                // don't hammer this endpoint.
                return Ok(Json(
                    serde_json::json!({ "received": true, "acted": false }),
                ));
            };
            recorded = record_payment(&state, org, invoice, amount, &currency, &intent_id).await?;
            return Ok(Json(
                serde_json::json!({ "received": true, "acted": recorded }),
            ));
        }
        "payment_intent.payment_failed" => {
            // Typed decline: audit-only, no payment row, no PSP prose in
            // the wire — the typed code derives from the decline_code.
            let decline_code = intent
                .pointer("/last_payment_error/decline_code")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let provider_reason = intent
                .pointer("/last_payment_error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            tracing::warn!(
                intent = intent.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                code = decline_code,
                "stripe webhook reports a declined payment"
            );
            if let (Some(org), Some(invoice)) = (org_id, invoice_id) {
                record_decline_audit(&state, org, invoice, decline_code, provider_reason).await?;
            }
        }
        _ => {}
    }
    Ok(Json(
        serde_json::json!({ "received": true, "acted": recorded }),
    ))
}

/// Records an out-of-band settled payment EXACTLY ONCE, through the same org
/// tx + FX behavior as the API flow. "Once" is a database fact, not a check
/// here: the receipt insert runs with `ON CONFLICT DO NOTHING` against the
/// per-invoice provider-reference index (0165), so a delivery that raced
/// another one inserts nothing and answers `false` — the same answer a later
/// redelivery gets. The settlement also carries the total it read into the
/// apply statement's predicate, so a total that moved is a typed 409 rather
/// than a settlement.
async fn record_payment(
    state: &AppState,
    org_id: Uuid,
    invoice_id: Uuid,
    amount_minor: i64,
    currency: &str,
    provider_ref: &str,
) -> ApiResult<bool> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let invoice = core_db::invoices::get_invoice(&mut *tx, org_id, invoice_id).await?;
    // The total this event is priced on, read from the invoice in THIS
    // transaction — the one that validates the event (its provider_ref
    // idempotence and the invoice's payable state). Carrying it into the
    // apply statement below is what makes the value the settlement was
    // validated against the value it writes against.
    let expected_total_minor = invoice.total_minor;
    // Advisory fast path, and nothing more: the read takes no lock, so two
    // deliveries can both see "absent" here. It is kept because a redelivery is
    // the common case and this answers it before paying FX resolution. The
    // decision is the receipt insert below, which the database refuses twice.
    let exists =
        core_db::invoices::payment_exists_for_ref(&mut *tx, org_id, invoice_id, provider_ref)
            .await?;
    if exists {
        tx.commit().await.map_err(ApiError::from)?;
        return Ok(false);
    }
    // Out-of-band settlements ride the SAME FX locks as recorded
    // payments (the amount is the invoice's own currency; the snapshot
    // captures the base-ledger conversion state at settle time).
    let snap = crate::routes::fx::lock_snapshot(
        &mut tx,
        org_id,
        amount_minor,
        &invoice.currency,
        chrono::Utc::now().date_naive(),
    )
    .await
    .ok()
    .flatten();
    let payment = core_domain::entities::InvoicePayment {
        id: Uuid::now_v7(),
        org_id,
        invoice_id,
        amount_minor,
        currency: invoice.currency.clone(),
        fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
        fx_rate_date: snap.as_ref().map(|s| s.rate_date),
        fx_source: snap.as_ref().map(|s| s.source.clone()),
        base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
        provider: "stripe".into(),
        provider_ref: Some(provider_ref.to_string()),
        notes: None,
        received_at: chrono::Utc::now(),
        created_by: None,
    };
    // The fence: the unique index on (org_id, invoice_id, provider_ref) decides
    // whether this delivery is the first one. A false answer means another
    // delivery of this event committed the receipt while this one was reading —
    // including one that passed the advisory check above — so nothing this
    // transaction did is kept, and the answer is the same one a later
    // redelivery gets. Until migration 0165 this insert was the second half of
    // a check-then-act pair and two deliveries could both land it.
    if !core_db::invoices::insert_payment_once(&mut *tx, &payment).await? {
        tx.commit().await.map_err(ApiError::from)?;
        return Ok(false);
    }
    // The settlement decides against the basis it read, not against whatever
    // the invoice holds by the time the UPDATE runs: payable status, the
    // total it was priced on, and the outstanding balance are all in the
    // predicate. A total that moved between the read above and this write
    // loses with a typed `DbError::Conflict` (409 at the wire) naming what
    // moved, and the receipt row inserted a line earlier rolls back with the
    // transaction — nothing is committed, so a Stripe retry finds no
    // provider_ref and settles exactly once against the total now on the
    // invoice. This path was the one window Wave 54's S4a left open: it
    // called the no-expectation form, so an out-of-band settlement could
    // apply against a total that had moved, and could pass the total outright.
    core_db::invoices::apply_payment_total_expecting(
        &mut tx,
        org_id,
        invoice_id,
        amount_minor,
        expected_total_minor,
    )
    .await?;
    core_db::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: core_domain::entities::Actor::System,
            action: "invoice.payment_recorded_webhook".into(),
            entity_type: Some("invoice".into()),
            entity_id: Some(invoice_id.to_string()),
            detail: serde_json::json!({
                "amount_minor": amount_minor,
                "currency": currency,
                "provider_ref": provider_ref,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(true)
}

async fn record_decline_audit(
    state: &AppState,
    org_id: Uuid,
    invoice_id: Uuid,
    decline_code: &str,
    provider_reason: &str,
) -> ApiResult<()> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    core_db::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: core_domain::entities::Actor::System,
            action: "invoice.payment_declined_webhook".into(),
            entity_type: Some("invoice".into()),
            entity_id: Some(invoice_id.to_string()),
            detail: serde_json::json!({
                "code": decline_code,
                "provider_reason": provider_reason,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn header(raw: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "stripe-signature",
            HeaderValue::from_str(raw).expect("visible ascii header"),
        );
        headers
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn candidates(count: usize) -> String {
        (0..count)
            .map(|i| format!("v1={i:064x}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn a_rotation_pair_verifies_on_either_candidate() {
        // Stripe signs with every active secret during a rotation; a
        // signature matching the SECOND candidate must still be accepted.
        let payload = b"{\"id\":\"evt_1\"}";
        let t = now();
        let signed = format!("{t}.{}", String::from_utf8_lossy(payload));
        let good = core_mail::hmac::sign(b"whsec_test", signed.as_bytes());
        let raw = format!("t={t},v1={},v1={good}", candidates(1));
        assert!(verify_stripe_signature(&header(&raw), payload, "whsec_test").is_ok());
    }

    #[test]
    fn the_candidate_set_is_capped_regardless_of_header_size() {
        // Worst case: the caller fills the header with parts. The parse
        // stops at the cap, so the number of full-body HMACs one request
        // can force is a constant, not a function of the header size.
        let t = now();
        let at_cap = format!("t={t},{}", candidates(MAX_V1_CANDIDATES));
        let (parsed_t, parsed) = parse_signature_header(&at_cap).expect("a rotation fits");
        assert_eq!(parsed_t, t);
        assert_eq!(parsed.len(), MAX_V1_CANDIDATES);

        let over_cap = format!("t={t},{}", candidates(1_000));
        let refused = parse_signature_header(&over_cap);
        assert!(
            refused.is_err(),
            "a header past the candidate cap must be refused before verification"
        );
    }

    #[test]
    fn a_flood_of_candidates_is_refused_as_a_webhook_rejection() {
        let t = now();
        let raw = format!("t={t},{}", candidates(10_000));
        let refused = verify_stripe_signature(&header(&raw), b"{}", "whsec_test");
        match refused {
            Err(ApiError::StripeWebhookRejected(reason)) => {
                assert!(reason.contains("v1 candidates"), "typed reason: {reason}");
            }
            other => panic!("expected a typed webhook rejection, got {other:?}"),
        }
    }

    #[test]
    fn stale_timestamps_are_rejected_before_any_hmac() {
        let stale = now() - SKEW_SECONDS - 1;
        let raw = format!("t={stale},{}", candidates(1));
        assert!(verify_stripe_signature(&header(&raw), b"{}", "whsec_test").is_err());
    }
}
