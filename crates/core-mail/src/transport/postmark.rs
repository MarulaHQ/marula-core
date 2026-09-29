//! Postmark adapter — the reference hosted MailTransport (§2 email row).
//!
//! Purpose: POST one JSON message to Postmark's `/email` endpoint carrying
//! Marula's threading headers; map HTTP/ErrorCode failures onto
//! TransportError so the ledger can retry.
//!
//! None-mode note: none — choose `EMAIL_TRANSPORT=none` to keep mail
//! queued without any provider dependency (standalone-first doctrine).

use crate::transport::{MailTransport, OutboundEmail, SendReceipt, TransportError};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct PostmarkTransport {
    pub server_token: String,
    /// Overridable so tests can point at a local capture server.
    pub base_url: String,
    pub client: reqwest::Client,
}

/// Request body per Postmark's send API (`Headers` carries the RFC 5322
/// threading fields Postmark has no first-class field for).
fn build_request(message: &OutboundEmail) -> serde_json::Value {
    let from = match &message.from_name {
        Some(name) => format!("{name} <{}>", message.from_address),
        None => message.from_address.clone(),
    };
    let mut headers: Vec<serde_json::Value> = Vec::new();
    if let Some(irt) = &message.in_reply_to {
        headers.push(serde_json::json!({
            "Name": "In-Reply-To",
            "Value": format!("<{}>", irt.trim().trim_start_matches('<').trim_end_matches('>'))
        }));
    }
    if !message.references.is_empty() {
        let refs = message
            .references
            .iter()
            .map(|r| {
                format!(
                    "<{}>",
                    r.trim().trim_start_matches('<').trim_end_matches('>')
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        headers.push(serde_json::json!({ "Name": "References", "Value": refs }));
    }
    serde_json::json!({
        "From": from,
        "To": message.to_addresses.join(", "),
        "ReplyTo": message.reply_to.clone().unwrap_or_default(),
        "Subject": message.subject,
        "TextBody": message.body_text,
        // Postmark stores its own id form — bare, no angle brackets.
        "MessageID": message
            .rfc_message_id
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>'),
        "Headers": headers,
    })
}

#[async_trait]
impl MailTransport for PostmarkTransport {
    fn name(&self) -> &'static str {
        "postmark"
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        let url = format!("{}/email", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("X-Postmark-Server-Token", &self.server_token)
            .header("Accept", "application/json")
            .json(&build_request(message))
            .send()
            .await
            .map_err(|e| TransportError::Unavailable(format!("postmark http: {e}")))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| TransportError::Unavailable(format!("postmark decode: {e}")))?;

        if !status.is_success() {
            let msg = body["Message"].as_str().unwrap_or("unknown error");
            return Err(TransportError::Rejected(format!(
                "postmark {}: {msg}",
                status.as_u16()
            )));
        }
        let code = body["ErrorCode"].as_i64().unwrap_or(0);
        if code != 0 {
            return Err(TransportError::Rejected(format!(
                "postmark error code {code}: {}",
                body["Message"].as_str().unwrap_or("")
            )));
        }
        Ok(SendReceipt {
            provider_message_id: body["MessageID"].as_str().map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_carries_from_to_threading_and_custom_message_id() {
        let m = OutboundEmail {
            from_address: "support@acme.test".into(),
            from_name: Some("Acme".into()),
            reply_to: None,
            to_addresses: vec!["a@x.test".into(), "b@x.test".into()],
            subject: "S".into(),
            body_text: "B".into(),
            rfc_message_id: "m1@acme.test".into(),
            in_reply_to: Some("p@x.test".into()),
            references: vec!["p@x.test".to_string(), "q@x.test".to_string()],
        };
        let v = build_request(&m);
        assert_eq!(v["From"], "Acme <support@acme.test>");
        assert_eq!(v["To"], "a@x.test, b@x.test");
        assert_eq!(v["MessageID"], "m1@acme.test");
        // Unset Reply-To rides as an empty string, which Postmark treats
        // as absent (its API omits empty fields).
        assert_eq!(v["ReplyTo"], "");
        let headers = v["Headers"].as_array().expect("headers");
        assert!(headers
            .iter()
            .any(|h| h["Name"] == "In-Reply-To" && h["Value"] == "<p@x.test>"));
        assert!(headers
            .iter()
            .any(|h| h["Name"] == "References" && h["Value"] == "<p@x.test> <q@x.test>"));
    }

    #[tokio::test]
    async fn sends_against_local_capture_server_and_reads_receipt() {
        use axum::routing::post;
        use axum::Json;
        async fn capture(
            headers: axum::http::HeaderMap,
            Json(body): Json<serde_json::Value>,
        ) -> Json<serde_json::Value> {
            assert_eq!(
                headers
                    .get("X-Postmark-Server-Token")
                    .and_then(|v| v.to_str().ok()),
                Some("token-123")
            );
            assert_eq!(body["TextBody"], "hello");
            Json(serde_json::json!({ "MessageID": "pm-9", "ErrorCode": 0 }))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let app = axum::Router::new().route("/email", post(capture));
            axum::serve(listener, app).await.unwrap();
        });

        let t = PostmarkTransport {
            server_token: "token-123".into(),
            base_url: format!("http://{addr}"),
            client: reqwest::Client::new(),
        };
        let receipt = t
            .send(&OutboundEmail {
                from_address: "f@a.test".into(),
                from_name: None,
                reply_to: None,
                to_addresses: vec!["t@a.test".into()],
                subject: "s".into(),
                body_text: "hello".into(),
                rfc_message_id: "<m@a.test>".into(),
                in_reply_to: None,
                references: vec![],
            })
            .await
            .expect("send ok");
        assert_eq!(receipt.provider_message_id.as_deref(), Some("pm-9"));
    }

    #[tokio::test]
    async fn provider_error_maps_to_rejected() {
        use axum::routing::post;
        use axum::Json;
        async fn fail() -> (axum::http::StatusCode, Json<serde_json::Value>) {
            (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "Message": "bad sender", "ErrorCode": 401 })),
            )
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let app = axum::Router::new().route("/email", post(fail));
            axum::serve(listener, app).await.unwrap();
        });
        let t = PostmarkTransport {
            server_token: "t".into(),
            base_url: format!("http://{addr}"),
            client: reqwest::Client::new(),
        };
        let err = t
            .send(&OutboundEmail {
                from_address: "f@a.test".into(),
                from_name: None,
                reply_to: None,
                to_addresses: vec!["t@a.test".into()],
                subject: "s".into(),
                body_text: "b".into(),
                rfc_message_id: "<m@a.test>".into(),
                in_reply_to: None,
                references: vec![],
            })
            .await
            .expect_err("must reject");
        assert!(matches!(err, TransportError::Rejected(msg) if msg.contains("bad sender")));
    }
}
