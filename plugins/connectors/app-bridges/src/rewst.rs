//! The Rewst bridge: triggers a Rewst WORKFLOW run over its org webhook
//! URL. Auth is the org-scoped shared secret, HMAC-signed over the
//! request BODY with the SAME core hmac method as every other machine
//! surface (one algorithm, one constant-time verification posture).
//! The payload carries the requested notification → workflow contract:
//! `{ org, event, invoice_id?, ticket_id?, text, link_url? }` etc.
use crate::BridgeError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RewstBridge {
    /// The org webhook endpoint that starts the workflow
    /// (`https://<rewst>.eng.rewst.io/webhooks/…`).
    webhook_url: String,
    /// The Rewst shared secret, HMAC-SHA256 signed over the body —
    /// never logged or returned.
    secret: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RewstTriggerPayload {
    /// Freetext org key the workflow filters on (never a Marula id
    /// unless the org chooses to put one in).
    pub org: String,
    pub event: String,
    /// Untrusted-to-Rewst payload — what the workflow asked for.
    pub notification: crate::NotificationPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
}

impl RewstTriggerPayload {
    pub fn to_json(&self) -> Result<String, BridgeError> {
        serde_json::to_string(self).map_err(|e| BridgeError::BadConfig("rewst", e.to_string()))
    }
}

impl RewstBridge {
    pub fn new(webhook_url: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            secret: secret.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        crate::slack::validate_webhook_url(&self.webhook_url, "rewst")?;
        if self.secret.trim().is_empty() {
            return Err(BridgeError::BadConfig(
                "rewst",
                "the bridge secret is required (HMAC-signed triggers)".into(),
            ));
        }
        Ok(())
    }

    /// Triggers a workflow with an HMAC-signed body (Rewst verifies the
    /// Marula signature header before trusting the envelope).
    pub async fn trigger(&self, payload: &RewstTriggerPayload) -> Result<(), BridgeError> {
        self.validate()?;
        let body = payload.to_json()?;
        let signed = core_signature(&body, &self.secret);
        let response = self
            .http
            .post(&self.webhook_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-marula-signature", &signed)
            .body(body)
            .send()
            .await
            .map_err(|e| BridgeError::Unreachable("rewst", e.to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(BridgeError::Refused(
                "rewst",
                format!("HTTP {}: {text}", status.as_u16()),
            ));
        }
        Ok(())
    }
}

/// HMAC-SHA256 over the raw body with the bridge secret; the wired
/// verification comparison lives on the Rewst side (their engine).
fn core_signature(payload: &str, secret: &str) -> String {
    use hmac::Mac;
    let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
