//! `MailTransport` — the trait boundary every outbound email provider sits
//! behind (§4.7). No vendor SDK type may leak past this file's types.
//!
//! Non-purpose: no retry/backoff state (that is the core-api/worker
//! ledger's job) and no rendering — callers pass fully rendered text.

pub mod postmark;
pub mod recording;
pub mod smtp;

use async_trait::async_trait;

/// A fully rendered outbound message. `rfc_message_id` is the stable
/// threading anchor Meridian assigns; adapters should transmit it as the
/// actual RFC 5322 Message-ID so replies thread back to us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEmail {
    pub from_address: String,
    pub from_name: Option<String>,
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
    /// Provider-assigned id when it returns one (Postmark); None otherwise.
    pub provider_message_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Could not reach the provider — retry later with backoff.
    #[error("transport unavailable: {0}")]
    Unavailable(String),
    /// The provider refused THIS message — retrying identical content is
    /// futile unless noted; the ledger still applies its backoff policy.
    #[error("transport rejected message: {0}")]
    Rejected(String),
}

/// The single seam for sending email. Implementations: Postmark API,
/// SMTP (dev/mailpit + self-host relays), recording/noop test doubles.
#[async_trait]
pub trait MailTransport: Send + Sync {
    /// Adapter name recorded on the message ledger (`postmark`, `smtp`, …).
    fn name(&self) -> &'static str;

    /// Sends one rendered message. Must not mutate shared state other than
    /// its own adapter config; idempotency across retries is owned by the
    /// caller via `rfc_message_id`.
    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError>;
}
