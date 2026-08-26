//! Outbound mail rendering and retry-backoff math.
//!
//! Purpose: pure functions turning ticket events into the two v1 outbound
//! kinds (ticket-created confirmation, comment notification) and computing
//! the exponential backoff delay for failed sends. Everything here is
//! deterministic so the CI harness can assert exact rendering.
//!
//! Non-purpose: no persistence, no transport calls, no policy about WHEN
//! to send (core-api decides; this module only shapes content).

use chrono::Duration;

/// The v1 outbound kinds Meridian generates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundKind {
    /// Auto-responder sent when a ticket is created from an inbound email.
    TicketCreatedConfirmation {
        ticket_number: String,
        ticket_title: String,
    },
    /// Requester notification that a client-visible reply landed.
    CommentNotification {
        ticket_number: String,
        ticket_title: String,
        excerpt: String,
    },
}

impl OutboundKind {
    /// Rendered subject line. Replies carry the bracketed ticket token so
    /// the requester's next reply threads via subject fallback too.
    #[must_use]
    pub fn subject(&self) -> String {
        match self {
            OutboundKind::TicketCreatedConfirmation {
                ticket_number,
                ticket_title,
            } => format!("[{ticket_number}] {ticket_title} — we received your request"),
            OutboundKind::CommentNotification {
                ticket_number,
                ticket_title,
                ..
            } => format!("Re: [{ticket_number}] {ticket_title}"),
        }
    }

    /// Rendered plain-text body. Sign-off names the product; org branding
    /// customization arrives with settings registry work.
    #[must_use]
    pub fn body(&self) -> String {
        match self {
            OutboundKind::TicketCreatedConfirmation {
                ticket_number,
                ticket_title,
            } => format!(
                "Your request \"{ticket_title}\" was received and registered as \
                 ticket {ticket_number}.\nReply to this email to add information — \
                 keep the [{ticket_number}] tag in the subject."
            ),
            OutboundKind::CommentNotification {
                ticket_number,
                excerpt,
                ..
            } => format!(
                "New reply on ticket {ticket_number}:\n\n{excerpt}\n\n\
                 Reply to this email to respond."
            ),
        }
    }
}

/// Backoff before attempt number `attempts` (1-based count of attempts
/// already made): 30s doubling per failure, capped at 1 hour. Pure so the
/// harness can simulate weeks of retries without sleeping.
#[must_use]
pub fn backoff_delay(attempts: i64) -> Duration {
    let step = attempts.clamp(1, 32);
    let secs = 30_i64.saturating_mul(1_i64 << (step - 1).min(7));
    Duration::seconds(secs.min(3_600))
}

/// Truncates a comment body for the notification excerpt on word-ish
/// boundaries (never mid-UTF-8: operates on chars).
#[must_use]
pub fn excerpt(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max_chars).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_subject_and_body_carry_the_ticket_token() {
        let k = OutboundKind::TicketCreatedConfirmation {
            ticket_number: "MER-42".into(),
            ticket_title: "Printer on fire".into(),
        };
        assert_eq!(
            k.subject(),
            "[MER-42] Printer on fire — we received your request"
        );
        assert!(k.body().contains("[MER-42]"));
    }

    #[test]
    fn notification_is_a_reply_shaped_subject() {
        let k = OutboundKind::CommentNotification {
            ticket_number: "MER-7".into(),
            ticket_title: "VPN".into(),
            excerpt: "fixed".into(),
        };
        assert_eq!(k.subject(), "Re: [MER-7] VPN");
        assert!(k.body().contains("fixed"));
    }

    #[test]
    fn backoff_doubles_then_caps_at_one_hour() {
        assert_eq!(backoff_delay(0), Duration::seconds(30));
        assert_eq!(backoff_delay(1), Duration::seconds(30));
        assert_eq!(backoff_delay(2), Duration::seconds(60));
        assert_eq!(backoff_delay(4), Duration::seconds(240));
        assert_eq!(backoff_delay(8), Duration::seconds(3_600));
        assert_eq!(backoff_delay(50), Duration::seconds(3_600));
    }

    #[test]
    fn excerpt_truncates_on_char_boundaries() {
        assert_eq!(excerpt("short", 10), "short");
        let long = "x".repeat(300);
        let out = excerpt(&long, 120);
        assert_eq!(out.chars().count(), 121);
        assert!(out.ends_with('…'));
        let emoji = "🦀".repeat(200);
        assert!(excerpt(&emoji, 10).ends_with('…'));
    }
}
