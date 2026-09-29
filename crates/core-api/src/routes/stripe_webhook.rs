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
//! * idempotence: the provider_ref (the Stripe intent id from the
//!   metadata) is looked up BEFORE inserting — a redelivered event finds
//!   the existing payment row and answers 200 with `processed: false`;
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
    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.trim() {
            "t" => timestamp = v.trim().parse().ok(),
            "v1" => signatures.push(v.trim().to_string()),
            _ => {}
        }
    }
    let Some(t) = timestamp else {
        return Err(ApiError::StripeWebhookRejected(
            "the signature timestamp is missing",
        ));
    };
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
    responses((status = 200, body = Value), (status = 400), (status = 403)))]
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

/// Records an out-of-band settled payment EXACTLY ONCE (the provider_ref
/// idempotence lock), through the same org tx + FX behavior as the API
/// flow. Returns whether a NEW row was inserted.
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
    core_db::invoices::insert_payment(&mut *tx, &payment).await?;
    core_db::invoices::apply_payment_total(&mut *tx, org_id, invoice_id, amount_minor)
        .await?
        .ok_or_else(|| {
            ApiError::Conflict("invoice left its payable state (webhook settlement)".into())
        })?;
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
