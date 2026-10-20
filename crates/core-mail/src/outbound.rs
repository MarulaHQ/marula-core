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

/// The v1 outbound kinds Marula generates.
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
    /// Account statement on the dunning path (TASKLIST backend-gaps
    /// lane): the client's open invoices with outstanding balances.
    /// Body rides the outbound ledger; none-mode stays queued like
    /// every other leg.
    InvoiceStatement {
        client_name: String,
        currency: String,
        total_outstanding_minor: i64,
        /// One `"INV-… · due YYYY-MM-DD · total · outstanding"` line per
        /// open invoice (pre-rendered by core-api, which owns the money
        /// formatting).
        invoice_lines: Vec<String>,
    },
    /// Free-form message fully rendered by the caller (Wave 12 lane 6:
    /// the scheduling-link template auto-email, whose subject/body are
    /// org-edited template text with the placeholders already expanded).
    /// Rendering happens at the composition site because only it knows
    /// the template semantics; this module carries it verbatim.
    Templated { subject: String, body: String },
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
            OutboundKind::InvoiceStatement { client_name, .. } => {
                format!("Account statement — {client_name}")
            }
            OutboundKind::Templated { subject, .. } => subject.clone(),
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
            OutboundKind::InvoiceStatement {
                currency,
                total_outstanding_minor,
                invoice_lines,
                ..
            } => {
                let total = format_minor(*total_outstanding_minor);
                let mut body = String::from("Your current account statement:\n\n");
                for line in invoice_lines {
                    body.push_str(line);
                    body.push('\n');
                }
                body.push_str(&format!(
                    "\nTotal outstanding: {total} {currency}\n\n\
                     If you have already sent payment, please disregard this notice."
                ));
                body
            }
            OutboundKind::Templated { body, .. } => body.clone(),
        }
    }
}

/// Minor units rendered with an assumed two-decimal exponent (the
/// currency code is rendered alongside by the caller's lines and total).
fn format_minor(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Backoff before attempt number `attempts` (1-based count of attempts
/// already made), jittered, from the email
/// [`RetryPolicy`](core_domain::retry::RetryPolicy). Pure apart from the
/// sample, so the harness can simulate weeks of retries without sleeping.
#[must_use]
pub fn backoff_delay(attempts: i64) -> Duration {
    backoff_delay_with(attempts, core_domain::retry::sample())
}

/// [`backoff_delay`] with the jitter sample supplied. `sample = 1.0` is the
/// unjittered curve, which is what a monotonicity property can pin.
#[must_use]
pub fn backoff_delay_with(attempts: i64, sample: f64) -> Duration {
    let made = u32::try_from(attempts.max(1)).unwrap_or(1);
    let secs =
        core_domain::retry::policy(core_domain::retry::Subsystem::Email).delay_secs(made, sample);
    Duration::seconds(secs)
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
            ticket_number: "MAR-42".into(),
            ticket_title: "Printer on fire".into(),
        };
        assert_eq!(
            k.subject(),
            "[MAR-42] Printer on fire — we received your request"
        );
        assert!(k.body().contains("[MAR-42]"));
    }

    #[test]
    fn notification_is_a_reply_shaped_subject() {
        let k = OutboundKind::CommentNotification {
            ticket_number: "MAR-7".into(),
            ticket_title: "VPN".into(),
            excerpt: "fixed".into(),
        };
        assert_eq!(k.subject(), "Re: [MAR-7] VPN");
        assert!(k.body().contains("fixed"));
    }

    #[test]
    fn backoff_doubles_then_caps() {
        // `sample = 1.0` is the unjittered curve; the live value is jittered
        // inside [0, curve]. The numbers belong to the email RetryPolicy.
        let cap = core_domain::retry::policy(core_domain::retry::Subsystem::Email).cap;
        assert_eq!(backoff_delay_with(0, 1.0), Duration::seconds(30));
        assert_eq!(backoff_delay_with(1, 1.0), Duration::seconds(30));
        assert_eq!(backoff_delay_with(2, 1.0), Duration::seconds(60));
        assert_eq!(backoff_delay_with(4, 1.0), Duration::seconds(240));
        assert_eq!(
            backoff_delay_with(8, 1.0),
            Duration::seconds(i64::try_from(cap.as_secs()).expect("cap fits"))
        );
        assert_eq!(
            backoff_delay_with(50, 1.0),
            Duration::seconds(i64::try_from(cap.as_secs()).expect("cap fits"))
        );
        // A sampled delay never waits more than the curve, and never zero.
        for attempts in 1..40 {
            let sampled = backoff_delay(attempts);
            assert!(sampled >= Duration::seconds(1));
            assert!(sampled <= backoff_delay_with(attempts, 1.0));
        }
    }

    #[test]
    fn templated_carries_the_caller_rendering_verbatim() {
        let k = OutboundKind::Templated {
            subject: "Confirmed: {date}".replace("{date}", "2026-09-15 09:00"),
            body: "Hi Dana,\n\nYour booking is confirmed.".into(),
        };
        assert_eq!(k.subject(), "Confirmed: 2026-09-15 09:00");
        assert!(k.body().contains("Your booking is confirmed"));
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
