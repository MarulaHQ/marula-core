//! Wire DTOs exchanged between the worker (which performs transport
//! calls) and core-api (the only database writer, §4.1).
//!
//! Purpose: single-sourced request/response shapes for the claim/report
//! endpoints so worker and core-api cannot drift. The worker claims due
//! outbound rows from core-api, sends them through its configured
//! `MailTransport`, and reports per-message results back; core-api applies
//! sent/dead/retry state transitions.
//!
//! Non-purpose: no transport behavior and no backoff math here — those
//! live in `outbound` and the ledger queries.

use serde::{Deserialize, Serialize};

/// POST /api/internal/email/outbound/claim — ask core-api for due sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRequest {
    /// Rows in `sending` state older than this are considered abandoned by
    /// a dead worker and re-claimed (lease recovery).
    pub lease_secs: i64,
    pub limit: i32,
}

/// One claimed outbound message with everything needed to render and send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedOutbound {
    /// `email_outbound.id` — echoed back in the result report.
    pub id: uuid::Uuid,
    pub org_id: uuid::Uuid,
    pub rfc_message_id: String,
    pub from_address: String,
    #[serde(default)]
    pub from_name: Option<String>,
    /// Reply-To resolved at claim time from the org/board identity.
    #[serde(default)]
    pub reply_to: Option<String>,
    pub to_addresses: Vec<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_text: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    /// Attempts already made (for logging/backoff on the api side).
    pub attempts: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResponse {
    pub messages: Vec<ClaimedOutbound>,
}

impl ClaimedOutbound {
    /// Converts the wire shape into a transport-ready message.
    #[must_use]
    pub fn into_outbound(self) -> crate::transport::OutboundEmail {
        crate::transport::OutboundEmail {
            from_address: self.from_address,
            from_name: self.from_name,
            reply_to: self.reply_to,
            to_addresses: self.to_addresses,
            subject: self.subject.unwrap_or_default(),
            body_text: self.body_text.unwrap_or_default(),
            rfc_message_id: self.rfc_message_id,
            in_reply_to: self.in_reply_to,
            references: self.references,
        }
    }
}

/// One per-message outcome reported by the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResult {
    pub id: uuid::Uuid,
    pub ok: bool,
    /// Attempts the worker made for this claim batch (drives backoff).
    pub attempts: i32,
    #[serde(default)]
    pub provider_message_id: Option<String>,
    /// Error detail when `ok` is false — persisted as last_error.
    #[serde(default)]
    pub error: Option<String>,
}

/// POST /api/internal/email/outbound/results — batched result reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultsRequest {
    pub results: Vec<SendResult>,
}

/// POST /api/internal/email/notification — the worker decided the email leg
/// of one notification delivers; core-api renders the branded message and
/// enqueues it on the outbound ledger (single-writer preserved).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEmailRequest {
    pub org_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    /// Notification kind slug (`mention`, `sla_breach`, …).
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub ticket_id: Option<uuid::Uuid>,
}

/// One digest row as the worker reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestItemDto {
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// POST /api/internal/email/digest — the worker determined the recipient's
/// local digest hour arrived; core-api renders, enforces the org policy and
/// the once-per-local-day marker, then enqueues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestEmailRequest {
    pub org_id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    /// Recipient-local date label (`YYYY-MM-DD`) — the marker anchor.
    pub day: String,
    pub items: Vec<DigestItemDto>,
}

/// POST /api/internal/email/csat — the worker ran the survey policy for a
/// resolved ticket; core-api renders the branded tokenized survey mail and
/// enqueues it. The raw token rides this request once and lives on only in
/// the mail link; the survey row stores its SHA-256 hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsatEmailRequest {
    pub org_id: uuid::Uuid,
    pub survey_id: uuid::Uuid,
    pub ticket_id: uuid::Uuid,
    pub contact_email: String,
    pub token: String,
}

/// Outcome of an internal email enqueue. `outcome` is one of `queued`,
/// `skipped_disabled`, `skipped_already_sent`, `skipped_empty`,
/// `skipped_no_recipient`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueEmailResponse {
    pub outcome: String,
    #[serde(default)]
    pub message_id: Option<uuid::Uuid>,
}

impl EnqueueEmailResponse {
    #[must_use]
    pub fn skipped(outcome: &str) -> Self {
        Self {
            outcome: outcome.to_string(),
            message_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claimed_message_converts_into_transport_shape() {
        let claimed = ClaimedOutbound {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            rfc_message_id: "<m@a>".into(),
            from_address: "f@a".into(),
            from_name: Some("N".into()),
            reply_to: None,
            to_addresses: vec!["t@a".into()],
            subject: Some("s".into()),
            body_text: None,
            in_reply_to: None,
            references: vec![],
            attempts: 1,
        };
        let m = claimed.clone().into_outbound();
        assert_eq!(m.subject, "s");
        assert_eq!(m.body_text, "");
        // Round-trips through serde for the wire.
        let json = serde_json::to_string(&claimed).unwrap();
        let back: ClaimedOutbound = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, claimed.id);
        let res = ResultsRequest {
            results: vec![SendResult {
                id: claimed.id,
                ok: false,
                attempts: 1,
                provider_message_id: None,
                error: Some("smtp down".into()),
            }],
        };
        assert!(
            serde_json::from_str::<ResultsRequest>(&serde_json::to_string(&res).unwrap()).is_ok()
        );
    }
}
