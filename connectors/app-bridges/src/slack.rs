//! The Slack forwarder: incoming-webhook delivery of Meridian
//! notifications (the standard app "message to a channel" webhook).
//! The webhook URL is the ENTIRE configuration; sending NEVER logs it.
use crate::{BridgeError, NotificationPayload};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct SlackConnector {
    webhook_url: String,
    http: reqwest::Client,
}

/// URL validation at the write boundary: https required, EXCEPT explicit
/// loopback (the conformance mock binds 127.0.0.1; production config
/// never lands there and the route layer never passes loopback urls —
/// the mock calls this rule out explicitly).
pub fn validate_webhook_url(raw: &str, kind: &'static str) -> Result<String, BridgeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BridgeError::BadConfig(kind, "the webhook URL is required".into()));
    }
    let loopback = trimmed.starts_with("http://127.0.0.1") || trimmed.starts_with("http://localhost");
    if !trimmed.starts_with("https://") && !loopback {
        return Err(BridgeError::BadConfig(
            kind,
            "the webhook URL must be an https URL (loopback mocks excepted in tests)".into(),
        ));
    }
    Ok(trimmed.to_string())
}

impl SlackConnector {
    /// `webhook_url` comes sealed from the connector installation.
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Write-boundary validation of the stored URL.
    pub fn validate(&self) -> Result<(), BridgeError> {
        validate_webhook_url(&self.webhook_url, "slack")?;
        Ok(())
    }

    /// Delivery of one notification; a success body that REJECTS the
    /// payload shape (Slack answers 200 with an error text) is a typed
    /// refusal, not silent success.
    pub async fn send(&self, payload: &NotificationPayload) -> Result<(), BridgeError> {
        self.validate()?;
        let body = json!({
            "text": format!("*{}*\n\n{}", payload.title, payload.body)
        });
        let response = self
            .http
            .post(&self.webhook_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| BridgeError::Unreachable("slack", e.to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(BridgeError::Refused(
                "slack",
                format!("HTTP {}: {text}", status.as_u16()),
            ));
        }
        let lower = text.to_ascii_lowercase();
        if lower.contains("missing_type") || lower.contains("invalid_payload") {
            return Err(BridgeError::Refused("slack", text.clone()));
        }
        Ok(())
    }
}


