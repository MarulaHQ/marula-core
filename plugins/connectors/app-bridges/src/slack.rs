//! The Slack forwarder: incoming-webhook delivery of Marula
//! notifications (the standard app "message to a channel" webhook).
//! The webhook URL is the ENTIRE configuration; sending NEVER logs it.
use crate::{BridgeError, NotificationPayload};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct SlackConnector {
    webhook_url: String,
    /// Admit a plain-http loopback ADDRESS (`127.0.0.0/8`, `::1`) as this
    /// connector's destination. Off by default: a configured connector is a
    /// destination an admin typed, and the plain-http allowance is for the
    /// conformance mock (and a process running under the dev-gated loopback
    /// policy), never for a shipped deployment.
    allow_loopback: bool,
    http: reqwest::Client,
}

/// Whether `raw` names a loopback ADDRESS exactly.
///
/// Parsed with the WHATWG parser rather than by string surgery. A hand-rolled
/// split reads `http://127.0.0.1:80@evil.test/` as host `127.0.0.1` — the real
/// host is `evil.test` and the rest is userinfo — so the exemption would have
/// admitted a public host. Only `http` on a loopback IP qualifies, and userinfo
/// is refused outright: a name is judged by the guard upstream in every policy,
/// so there is no name spelling to except here.
fn is_loopback_url(raw: &str) -> bool {
    use url::Host;

    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "http" || !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(_)) | None => false,
    }
}

/// URL validation at the write boundary: https required. A plain-http
/// loopback URL is admitted only when `allow_loopback` says so — the
/// conformance mock sets it, and the route layer sets it only under the
/// dev-gated egress policy. The route layer screens the address class with
/// `core_domain::egress` before this runs and again before it sends, so this
/// is the scheme half of the rule, not the only one.
pub fn validate_webhook_url(
    raw: &str,
    kind: &'static str,
    allow_loopback: bool,
) -> Result<String, BridgeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BridgeError::BadConfig(
            kind,
            "the webhook URL is required".into(),
        ));
    }
    let allowed = trimmed.starts_with("https://") || (allow_loopback && is_loopback_url(trimmed));
    if !allowed {
        return Err(BridgeError::BadConfig(
            kind,
            "the webhook URL must be an https URL".into(),
        ));
    }
    Ok(trimmed.to_string())
}

impl SlackConnector {
    /// `webhook_url` comes sealed from the connector installation.
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self::with_http_client(webhook_url, crate::bridge_client())
    }

    /// Dial through a client the caller already built — the route layer passes
    /// the guard's pinned, redirect-free client, so the address that was
    /// checked is the address this connector connects to.
    pub fn with_http_client(webhook_url: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            webhook_url: webhook_url.into(),
            allow_loopback: false,
            http,
        }
    }

    /// Admit a plain-http loopback destination for this one connector.
    ///
    /// The conformance tests bind their mock on `127.0.0.1`, and a process
    /// running the dev-gated egress policy may talk to a local mock. No other
    /// caller has a reason to ask for it.
    #[must_use]
    pub fn allow_loopback(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// Write-boundary validation of the stored URL.
    pub fn validate(&self) -> Result<(), BridgeError> {
        validate_webhook_url(&self.webhook_url, "slack", self.allow_loopback)?;
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
        if status.is_redirection() {
            return Err(crate::refuse_redirect("slack", status));
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is the shipped rule: an https destination and nothing else.
    /// The loopback exemption is an explicit, per-connector opt-in, so a stored
    /// row that names the host's own loopback cannot be dialled by accident.
    #[test]
    fn https_is_the_default_and_loopback_needs_the_opt_in() {
        assert!(
            validate_webhook_url("https://hooks.slack.com/services/T/B/X", "slack", false).is_ok()
        );
        for refusal in [
            "http://127.0.0.1:9099/hook",
            "http://[::1]:9099/hook",
            "http://localhost:9099/hook",
            "http://localhost.attacker.test/hook",
            "ftp://hooks.slack.com/x",
        ] {
            assert!(
                validate_webhook_url(refusal, "slack", false).is_err(),
                "{refusal} must be refused without the opt-in"
            );
        }
    }

    /// The opt-in admits the loopback address class and nothing else: a public
    /// name that merely spells `localhost` somewhere in its host is still a
    /// public destination, and plain http to a public host is still refused.
    #[test]
    fn the_loopback_opt_in_is_an_address_class_not_a_prefix() {
        for admitted in [
            "http://127.0.0.1:9099/hook",
            "http://127.9.9.9/hook",
            "http://[::1]:9099/hook",
        ] {
            assert!(
                validate_webhook_url(admitted, "slack", true).is_ok(),
                "{admitted} is a loopback address"
            );
        }
        for refused in [
            "http://localhost.attacker.test/hook",
            "http://127.0.0.1.attacker.test/hook",
            // A real host with loopback dressed as userinfo. A hand-rolled
            // authority split reads the left side as the host and admits it.
            "http://127.0.0.1:80@evil.test/hook",
            "http://example.com/hook",
        ] {
            assert!(
                validate_webhook_url(refused, "slack", true).is_err(),
                "{refused} is not a loopback address"
            );
        }
    }
}
