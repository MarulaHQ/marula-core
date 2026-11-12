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
use core_domain::templates::RenderContext;

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
    /// Account statement on the dunning path (the private build tracker backend-gaps
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
    /// Single-use auth-token link mail (password reset, email
    /// verification, magic-link sign-in). The caller pre-renders the
    /// fallback subject/body exactly as it did before the stored-template
    /// path existed — those strings ARE the fallback — and carries the
    /// link separately so the stored template can receive it through
    /// `{{link.url}}` instead of finding it already baked into text.
    TokenMail {
        /// Which seeded template (kind `email`) the enqueue lookup reads.
        slug: &'static str,
        /// The absolute single-use link, fed to `{{link.url}}` when a
        /// stored template applies.
        link: String,
        /// Caller-rendered fallback subject.
        subject: String,
        /// Caller-rendered fallback body.
        body: String,
    },
    /// Workspace invitation (Wave p51 user-onboarding): a person with
    /// `manage_users` created a user and the product mails them the
    /// accept link. Branded: this is the first email the invitee sees
    /// from the workspace they are joining, so it renders an HTML part
    /// ([`OutboundKind::html`]) beside the text body.
    UserInvited {
        /// The invitee's name as the inviter typed it.
        display_name: String,
        /// The workspace name the mail greets them into.
        org_name: String,
        /// Who invited them (display name).
        inviter_name: String,
        /// Absolute `{web_base_url}/accept-invite?token=…` — built by the
        /// caller, which owns the config.
        invite_url: String,
        /// RFC 3339 expiry of the setup token.
        expires_at: String,
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
            OutboundKind::InvoiceStatement { client_name, .. } => {
                format!("Account statement — {client_name}")
            }
            OutboundKind::Templated { subject, .. } => subject.clone(),
            OutboundKind::TokenMail { subject, .. } => subject.clone(),
            OutboundKind::UserInvited { org_name, .. } => {
                format!("You've been invited to join {org_name}")
            }
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
            OutboundKind::TokenMail { body, .. } => body.clone(),
            OutboundKind::UserInvited {
                display_name,
                org_name,
                inviter_name,
                invite_url,
                expires_at,
            } => format!(
                "Hi {display_name},\n\n\
                 {inviter_name} invited you to join {org_name} on Marula, the workspace \
                 your team runs service work in.\n\n\
                 Accept your invitation:\n{invite_url}\n\n\
                 The link expires on {expires_at}. If you weren't expecting this \
                 invitation you can ignore this email — your address won't be added \
                 to the workspace until you accept."
            ),
        }
    }

    /// The styled HTML part, when this kind renders one. The HTML is
    /// self-contained (own greeting and sign-off; it never rides the
    /// personal-signature envelope) because an invitation is the product
    /// writing to someone who has never signed in.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn html(&self) -> Option<String> {
        let OutboundKind::UserInvited {
            display_name,
            org_name,
            inviter_name,
            invite_url,
            expires_at,
        } = self
        else {
            return None;
        };
        // Email clients style with inline attributes and tables, not the
        // app's token files. These are the §4.3 light values from
        // docs/theme.md, restated here because email has no CSS cascade.
        const INK: &str = "#2B2A33";
        const MUTED: &str = "#6F685D";
        const BORDER: &str = "#E2E0DA";
        const CANVAS: &str = "#F8F7F4";
        const GOLD_TEXT: &str = "#9A5B0F";
        // Org and people names are user data; an email renders without a
        // DOM or a sanitizer, so they are escaped here at the only place
        // they meet markup.
        let (display_name, org_name, inviter_name, invite_url) = (
            escape_html(display_name),
            escape_html(org_name),
            escape_html(inviter_name),
            escape_html(invite_url),
        );
        Some(format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"></head>
<body style="margin:0;padding:0;background:{CANVAS};">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:{CANVAS};padding:32px 16px;">
<tr><td align="center">
<table role="presentation" width="560" cellpadding="0" cellspacing="0" style="max-width:560px;width:100%;background:#FFFFFF;border:1px solid {BORDER};border-radius:16px;font-family:-apple-system,'Segoe UI',Helvetica,Arial,sans-serif;">
  <tr><td style="padding:32px 36px 0 36px;">
    <p style="margin:0 0 4px 0;font-family:Georgia,'Times New Roman',serif;font-size:26px;line-height:32px;color:{INK};font-weight:500;">
      Hi {display_name}, you've been invited</p>
    <p style="margin:0;font-size:15px;line-height:24px;color:{MUTED};">
      {inviter_name} invited you to join <strong style="color:{INK};">{org_name}</strong> on Marula —
      the workspace your team runs service work in.</p>
  </td></tr>
  <tr><td style="padding:28px 36px 0 36px;">
    <table role="presentation" cellpadding="0" cellspacing="0" width="100%"><tr><td align="center">
      <a href="{invite_url}" style="display:inline-block;background:{INK};color:#E8B83A;font-size:15px;font-weight:600;line-height:20px;padding:12px 28px;border-radius:10px;text-decoration:none;">Accept your invitation</a>
    </td></tr></table>
    <p style="margin:14px 0 0 0;text-align:center;font-size:13px;line-height:20px;color:{MUTED};">
      If the button doesn't work, copy this link into your browser:<br>
      <a href="{invite_url}" style="color:{GOLD_TEXT};word-break:break-all;">{invite_url}</a></p>
  </td></tr>
  <tr><td style="padding:28px 36px 32px 36px;">
    <p style="margin:0;padding-top:16px;border-top:1px solid {BORDER};font-size:13px;line-height:20px;color:{MUTED};">
      This link expires on {expires_at}. If you weren't expecting this invitation,
      you can ignore this email — your address won't be added to the workspace
      until you accept.</p>
  </td></tr>
</table>
</td></tr>
</table>
</body>
</html>"#
        ))
    }

    /// Which seeded template (kind `email`) the enqueue lookup reads for
    /// this kind, or `None` when the kind carries no system-template
    /// slug (`Templated` is fully caller-rendered). One slug per variant;
    /// the mapping is the enqueue path's OutboundKind → templates-row
    /// join key and nothing else reads it.
    #[must_use]
    pub fn template_slug(&self) -> Option<&'static str> {
        match self {
            OutboundKind::TicketCreatedConfirmation { .. } => Some("ticket_created_confirmation"),
            OutboundKind::CommentNotification { .. } => Some("comment_notification"),
            OutboundKind::InvoiceStatement { .. } => Some("invoice_statement"),
            OutboundKind::UserInvited { .. } => Some("user_invited"),
            OutboundKind::TokenMail { slug, .. } => Some(slug),
            OutboundKind::Templated { .. } => None,
        }
    }

    /// The render context a stored template resolves against: the kind's
    /// own data, keyed by the email-kind vocabulary the seeded templates
    /// (`core_domain::templates::DEFAULT_TEMPLATES`) are written in. This
    /// is the feed side of the closed vocabulary — a key listed in
    /// [`variables_for_kind`](core_domain::templates::variables_for_kind)
    /// but absent here renders empty by design; a key present here must
    /// be in the vocabulary, which is what keeps the seeded bodies
    /// renderable.
    #[must_use]
    pub fn render_context(&self) -> RenderContext {
        let mut ctx = RenderContext::new();
        match self {
            OutboundKind::TicketCreatedConfirmation {
                ticket_number,
                ticket_title,
            } => {
                ctx.insert("ticket.number".into(), ticket_number.clone());
                ctx.insert("ticket.summary".into(), ticket_title.clone());
            }
            OutboundKind::CommentNotification {
                ticket_number,
                ticket_title,
                excerpt,
            } => {
                ctx.insert("ticket.number".into(), ticket_number.clone());
                ctx.insert("ticket.summary".into(), ticket_title.clone());
                ctx.insert("comment.excerpt".into(), excerpt.clone());
            }
            OutboundKind::InvoiceStatement {
                client_name,
                currency,
                total_outstanding_minor,
                invoice_lines,
            } => {
                ctx.insert("client.name".into(), client_name.clone());
                ctx.insert("invoice.currency".into(), currency.clone());
                ctx.insert(
                    "invoice.total_outstanding".into(),
                    format_minor(*total_outstanding_minor),
                );
                // The caller renders one pre-formatted line per open
                // invoice; the template carries a single placeholder whose
                // value is those lines joined the way the fallback joins
                // them.
                ctx.insert("invoice.lines".into(), invoice_lines.join("\n"));
            }
            OutboundKind::UserInvited {
                display_name,
                org_name,
                inviter_name,
                invite_url,
                expires_at,
            } => {
                ctx.insert("org.name".into(), org_name.clone());
                ctx.insert("invite.display_name".into(), display_name.clone());
                ctx.insert("invite.inviter_name".into(), inviter_name.clone());
                ctx.insert("invite.url".into(), invite_url.clone());
                ctx.insert("invite.expires_at".into(), expires_at.clone());
            }
            OutboundKind::TokenMail { link, .. } => {
                ctx.insert("link.url".into(), link.clone());
            }
            OutboundKind::Templated { .. } => {}
        }
        ctx
    }
}

/// Escapes user-supplied text for safe insertion into an email HTML
/// part. An email renders without a DOM or a sanitizer, so org and
/// people names must be escaped at the only place they meet markup.
/// The builtin HTML rendering calls this directly; the stored-template
/// path in core-api calls it on every context value before rendering a
/// stored HTML part, so a forked template is exactly as injection-proof
/// as the builtin one.
#[must_use]
pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
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

    /// The invitation kind: subject, text body, and the branded HTML part
    /// all carry the accept link, and the other kinds render no HTML.
    #[test]
    fn invitation_renders_link_in_every_part() {
        let k = OutboundKind::UserInvited {
            display_name: "Ada".into(),
            org_name: "Northwind IT".into(),
            inviter_name: "Grace".into(),
            invite_url: "http://localhost:3000/accept-invite?token=t".into(),
            expires_at: "19 November 2026".into(),
        };
        assert_eq!(k.subject(), "You've been invited to join Northwind IT");
        let text = k.body();
        assert!(text.contains("Ada"));
        assert!(text.contains("Grace"));
        assert!(text.contains("Northwind IT"));
        assert!(text.contains("/accept-invite?token=t"));
        let html = k.html().expect("invitation renders an HTML part");
        for needle in [
            "Ada",
            "Northwind IT",
            "Grace",
            "href=\"http://localhost:3000/accept-invite?token=t\"",
            "expires on 19 November 2026",
        ] {
            assert!(html.contains(needle), "html part missing {needle}");
        }
        // The button is the light-mode primary: ink fill with gold text,
        // one paired style — never bare gold text on the light card.
        assert!(html.contains("background:#2B2A33;color:#E8B83A"));
        // User-supplied names cannot inject markup into the mail.
        let hostile = OutboundKind::UserInvited {
            display_name: "<script>x</script>".into(),
            org_name: "A & B \"Quotes\"".into(),
            inviter_name: "Grace".into(),
            invite_url: "http://localhost:3000/accept-invite?token=t".into(),
            expires_at: "19 November 2026".into(),
        };
        let safe = hostile.html().expect("html part");
        assert!(!safe.contains("<script>"));
        assert!(safe.contains("&lt;script&gt;"));
        assert!(safe.contains("A &amp; B &quot;Quotes&quot;"));
    }

    #[test]
    fn other_kinds_render_no_html_part() {
        let k = OutboundKind::Templated {
            subject: "s".into(),
            body: "b".into(),
        };
        assert!(k.html().is_none());
    }

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

    /// Every system-sent kind carries its templates-registry slug;
    /// caller-rendered kinds carry none.
    #[test]
    fn template_slug_maps_one_row_per_system_kind() {
        assert_eq!(
            OutboundKind::TicketCreatedConfirmation {
                ticket_number: "MAR-1".into(),
                ticket_title: "t".into(),
            }
            .template_slug(),
            Some("ticket_created_confirmation")
        );
        assert_eq!(
            OutboundKind::CommentNotification {
                ticket_number: "MAR-1".into(),
                ticket_title: "t".into(),
                excerpt: "e".into(),
            }
            .template_slug(),
            Some("comment_notification")
        );
        assert_eq!(
            OutboundKind::InvoiceStatement {
                client_name: "c".into(),
                currency: "USD".into(),
                total_outstanding_minor: 0,
                invoice_lines: vec![],
            }
            .template_slug(),
            Some("invoice_statement")
        );
        assert_eq!(
            OutboundKind::UserInvited {
                display_name: "d".into(),
                org_name: "o".into(),
                inviter_name: "i".into(),
                invite_url: "u".into(),
                expires_at: "e".into(),
            }
            .template_slug(),
            Some("user_invited")
        );
        assert_eq!(
            OutboundKind::TokenMail {
                slug: "magic_link",
                link: "l".into(),
                subject: "s".into(),
                body: "b".into(),
            }
            .template_slug(),
            Some("magic_link")
        );
        assert_eq!(
            OutboundKind::Templated {
                subject: "s".into(),
                body: "b".into(),
            }
            .template_slug(),
            None,
            "caller-rendered mail has no system slug"
        );
    }

    /// The render context feeds exactly the keys the seeded templates
    /// place (the closed email-kind vocabulary): the invite renders from
    /// its five values, the statement from the pre-formatted money and
    /// joined lines, the token mail from the link alone.
    #[test]
    fn render_context_feeds_the_seeded_placeholders() {
        let invited = OutboundKind::UserInvited {
            display_name: "Ada".into(),
            org_name: "Northwind IT".into(),
            inviter_name: "Grace".into(),
            invite_url: "http://localhost:3000/accept-invite?token=t".into(),
            expires_at: "19 November 2026".into(),
        };
        let ctx = invited.render_context();
        for (key, value) in [
            ("org.name", "Northwind IT"),
            ("invite.display_name", "Ada"),
            ("invite.inviter_name", "Grace"),
            ("invite.url", "http://localhost:3000/accept-invite?token=t"),
            ("invite.expires_at", "19 November 2026"),
        ] {
            assert_eq!(ctx.get(key).map(String::as_str), Some(value), "key {key}");
        }

        let statement = OutboundKind::InvoiceStatement {
            client_name: "Contoso".into(),
            currency: "USD".into(),
            total_outstanding_minor: 12_345,
            invoice_lines: vec!["INV-1 · due 2026-11-01".into(), "INV-2".into()],
        };
        let ctx = statement.render_context();
        assert_eq!(
            ctx.get("invoice.total_outstanding").map(String::as_str),
            Some("123.45")
        );
        assert_eq!(
            ctx.get("invoice.lines").map(String::as_str),
            Some("INV-1 · due 2026-11-01\nINV-2")
        );

        let token = OutboundKind::TokenMail {
            slug: "password_reset",
            link: "http://localhost:3000/password-reset?token=r".into(),
            subject: "Reset your Marula password".into(),
            body: "fallback".into(),
        };
        let ctx = token.render_context();
        assert_eq!(
            ctx.get("link.url").map(String::as_str),
            Some("http://localhost:3000/password-reset?token=r")
        );
        assert_eq!(ctx.len(), 1, "the token mail feeds the link only");
    }

    /// The stored-template path renders the seeded bodies through the
    /// same engine with the kind's context — so a fresh org's mail is
    /// unchanged whether the stored row or the fallback applies.
    #[test]
    fn seeded_bodies_render_from_the_kind_context() {
        use core_domain::templates::{self, DEFAULT_TEMPLATES};
        let invited = OutboundKind::UserInvited {
            display_name: "Ada".into(),
            org_name: "Northwind IT".into(),
            inviter_name: "Grace".into(),
            invite_url: "http://localhost:3000/accept-invite?token=t".into(),
            expires_at: "19 November 2026".into(),
        };
        let d = DEFAULT_TEMPLATES
            .iter()
            .chain(core_domain::templates::OUTBOUND_EMAIL_TEMPLATES)
            .find(|d| d.builtin == "user_invited")
            .expect("user_invited seed");
        let subject = templates::render(
            d.kind,
            d.subject.expect("invite seed carries a subject"),
            &invited.render_context(),
        )
        .expect("seeded subject renders");
        assert_eq!(subject, invited.subject());
        let body =
            templates::render(d.kind, d.body, &invited.render_context()).expect("seeded body");
        assert_eq!(body.trim_end(), invited.body());
        // The stored HTML renders to the builtin HTML byte-for-byte: the
        // context values are escaped before insertion, exactly like the
        // builtin renderer escapes them.
        let html = templates::render(
            d.kind,
            d.html.expect("invite seed carries the branded HTML"),
            &invited.render_context(),
        )
        .expect("seeded html");
        assert_eq!(html, invited.html().expect("builtin html"));
    }

    /// The seeded HTML part survives hostile context values: escaping
    /// happens at insertion, so a forked template is as injection-proof
    /// as the builtin rendering.
    #[test]
    fn escape_html_is_the_stored_path_gate() {
        assert_eq!(
            escape_html("<script>x</script>"),
            "&lt;script&gt;x&lt;/script&gt;"
        );
        assert_eq!(
            escape_html("A & B \"q\" 'p'"),
            "A &amp; B &quot;q&quot; &#39;p&#39;"
        );
    }
}
