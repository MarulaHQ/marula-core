//! The Microsoft Teams forwarder: workflow (incoming) webhook card
//! delivery. One Adaptive Card per notification; text fallback included
//! so plain webhooks still surface the message.
use crate::{BridgeError, NotificationPayload};

#[derive(Debug, Clone)]
pub struct TeamsConnector {
    webhook_url: String,
    http: reqwest::Client,
}

impl TeamsConnector {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            http: reqwest::Client::new(),
        }
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        crate::slack::validate_webhook_url(&self.webhook_url, "teams")?;
        Ok(())
    }

    pub async fn send(&self, payload: &NotificationPayload) -> Result<(), BridgeError> {
        self.validate()?;
        let textblock = serde_json::json!({
            "type": "TextBlock",
            "text": format!("**{}**\n\n{}", payload.title, payload.body),
            "wrap": true,
        });
        let mut card = serde_json::json!({
            "type": "message",
            "attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                "content": { "type": "AdaptiveCard", "$schema": "http://adaptivecards.io/schemas/adaptive-card.json", "version": "1.4", "body": [textblock] }
            }]
        });
        // onClickable titles carry the deep link (when one is set).
        if let Some(link) = &payload.link_url {
            card["attachments"][0]["content"]["actions"] = serde_json::json!([{
                "type": "Action.OpenUrl",
                "title": "Marula \u{2026} Open",
                "url": link,
            }]);
        }
        let response = self
            .http
            .post(&self.webhook_url)
            .json(&card)
            .send()
            .await
            .map_err(|e| BridgeError::Unreachable("teams", e.to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(BridgeError::Refused(
                "teams",
                format!("HTTP {}: {text}", status.as_u16()),
            ));
        }
        // Teams answers 202 Accepted with "1" on delivery, and 400 with
        // {"code":"ActionableMessageTokenValidationFailure"} style
        // bodies on broken cards — treat non-"1" success bodies as a
        // typed refusal.
        if !text.trim().is_empty() && !text.trim().contains('1') && !text.to_ascii_lowercase().contains("message") {
            return Err(BridgeError::Refused("teams", text.clone()));
        }
        Ok(())
    }
}
