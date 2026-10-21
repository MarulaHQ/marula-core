//! `MailTransport` — the trait boundary every outbound email provider sits
//! behind (§4.7). No vendor SDK type may leak past this file's types.
//!
//! Non-purpose: no retry/backoff state (that is the core-api/worker
//! ledger's job) and no rendering — callers pass fully rendered text.

pub mod postmark;
pub mod recording;
pub mod ses;
pub mod smtp;

use async_trait::async_trait;

/// A fully rendered outbound message. `rfc_message_id` is the stable
/// threading anchor Marula assigns; adapters should transmit it as the
/// actual RFC 5322 Message-ID so replies thread back to us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEmail {
    pub from_address: String,
    pub from_name: Option<String>,
    /// Sent as a `Reply-To` header when set (board identities route client
    /// replies to the board's inbound address while sending from a
    /// different outbound address).
    pub reply_to: Option<String>,
    pub to_addresses: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub rfc_message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// Provider acknowledgement for one accepted message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendReceipt {
    /// Provider-assigned id when it returns one (Postmark, SES). None
    /// otherwise — SMTP has no server-assigned id to record.
    pub provider_message_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Could not reach the provider — retry later with backoff.
    #[error("transport unavailable: {0}")]
    Unavailable(String),
    /// The provider answered, and told us to come back later: a rate limit, a
    /// temporary refusal, a greylist, a mail queue that is full. Distinct from
    /// [`Self::Unavailable`] because the provider is alive and the message is
    /// acceptable — it is the moment that is wrong.
    #[error("transport throttled: {0}")]
    Throttled(String),
    /// The provider refused THIS message on its merits — an unknown recipient,
    /// an inactive address, a sender it will not accept. Retrying identical
    /// content cannot succeed, and the ledger dead-letters it on this report
    /// rather than spending the rest of its budget.
    #[error("transport rejected message: {0}")]
    Rejected(String),
}

impl TransportError {
    /// Whether retrying the identical message could ever succeed.
    ///
    /// The single classification the ledger reads, so "is this worth another
    /// attempt" is answered once, at the adapter boundary that actually saw the
    /// provider's answer, rather than by a status list re-derived downstream.
    /// A permanent refusal is not a delivery failure for the destination
    /// either — the endpoint is working, it is the message it will not take —
    /// which is why the same flag resets a webhook-style health counter.
    #[must_use]
    pub fn permanent(&self) -> bool {
        matches!(self, Self::Rejected(_))
    }
}

/// Maps an HTTP answer from a provider's API onto the transport class.
///
/// One place, so the Postmark and SES adapters cannot disagree about whether a
/// 429 is worth retrying. The buckets are
/// [`core_domain::retry::classify_http`]'s: transient statuses (408, 429, 5xx)
/// retry, and the rest are the request's fault. Within "retry", a 5xx is the
/// provider failing over to its own capacity and takes the plain unavailable
/// path, while 408/429 is the provider pushing back on us and keeps its own
/// class so an operator reading logs can tell throttle from outage.
#[must_use]
pub fn http_status_error(provider: &str, status: u16, detail: &str) -> TransportError {
    use core_domain::retry::{classify_http, ErrorClass};
    let text = format!("{provider} {status}: {detail}");
    match classify_http(status) {
        ErrorClass::Transient if status >= 500 => TransportError::Unavailable(text),
        ErrorClass::Transient => TransportError::Throttled(text),
        ErrorClass::Client | ErrorClass::Permanent => TransportError::Rejected(text),
    }
}

/// The single seam for sending email. Implementations: Postmark API,
/// Amazon SES v2, SMTP (dev/mailpit + self-host relays), recording/noop
/// test doubles.
#[async_trait]
pub trait MailTransport: Send + Sync {
    /// Adapter name recorded on the message ledger (`postmark`, `smtp`, …).
    fn name(&self) -> &'static str;

    /// Sends one rendered message. Must not mutate shared state other than
    /// its own adapter config; idempotency across retries is owned by the
    /// caller via `rfc_message_id`.
    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one question the ledger asks, answered by the class rather than by
    /// the text of a provider's sentence.
    #[test]
    fn only_a_rejection_is_permanent() {
        assert!(TransportError::Rejected("no such mailbox".into()).permanent());
        assert!(!TransportError::Throttled("slow down".into()).permanent());
        assert!(!TransportError::Unavailable("connection reset".into()).permanent());
    }

    /// The provider-status mapping is the HTTP taxonomy's, plus the
    /// throttle/outage split an operator needs to read a log line correctly.
    #[test]
    fn provider_statuses_land_in_the_right_class() {
        // 4xx the request's fault: permanent, no retry.
        for status in [400, 401, 403, 404, 406, 413, 422] {
            assert!(
                http_status_error("postmark", status, "detail").permanent(),
                "{status} must be permanent"
            );
        }
        // 408 and 429 are "ask again".
        for status in [408, 429] {
            assert!(matches!(
                http_status_error("ses", status, "detail"),
                TransportError::Throttled(_)
            ));
        }
        // 5xx is the provider failing, not us being throttled.
        for status in [500, 502, 503, 504] {
            assert!(matches!(
                http_status_error("postmark", status, "detail"),
                TransportError::Unavailable(_)
            ));
        }
        // The provider's name and status survive into the text an operator
        // greps for.
        let err = http_status_error("postmark", 422, "inactive recipient");
        assert!(err.to_string().contains("postmark 422"));
        assert!(err.to_string().contains("inactive recipient"));
    }
}
