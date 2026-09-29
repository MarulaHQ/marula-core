//! Branded message wrapper for Marula-generated notification and digest
//! email (Wave 34 lane 8).
//!
//! Purpose: one place that shapes the human-facing chrome around a
//! notification or digest — header wordmark (or logo), greeting line,
//! body block, sign-off, footer — rendered as BOTH plain text and simple
//! inline-styled HTML (email clients do not read the app's CSS variables,
//! so the HTML uses literal, neutral values: dark text on white with a
//! single thin rule; no invented brand colors).
//!
//! Non-purpose: no ticket mail (the outbound ledger renders those kinds in
//! `outbound.rs`), no transport, no persistence, no scheduling — callers
//! resolve the brand/greeting/sign-off values from settings first.

/// Brand values resolved by the caller from settings (org/board identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brand<'a> {
    /// Header wordmark; callers fall back to the organization name.
    pub brand_name: &'a str,
    /// Optional https logo URL. Callers validate the scheme; this module
    /// only escapes it into the HTML attribute.
    pub logo_url: Option<&'a str>,
    /// Greeting line already expanded (for example `Hi Dana,`). Empty
    /// renders no greeting.
    pub greeting: Option<&'a str>,
    /// Sign-off block (may be multi-line). Empty renders no sign-off.
    pub signoff: Option<&'a str>,
}

/// One digest row: a notification title plus an optional one-line detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestItem {
    pub title: String,
    pub detail: Option<String>,
}

/// A rendered message ready for the outbound ledger or transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandedEmail {
    pub subject: String,
    pub text_body: String,
    pub html_body: String,
}

/// Renders one notification email: greeting, title, optional body line,
/// sign-off, footer.
#[must_use]
pub fn render_notification(brand: &Brand<'_>, title: &str, body: Option<&str>) -> BrandedEmail {
    let mut text = String::new();
    if let Some(greeting) = non_empty(brand.greeting) {
        text.push_str(greeting.trim());
        text.push_str("\n\n");
    }
    text.push_str(title.trim());
    text.push('\n');
    if let Some(body) = non_empty(body) {
        text.push('\n');
        text.push_str(body.trim());
        text.push('\n');
    }
    if let Some(signoff) = non_empty(brand.signoff) {
        text.push('\n');
        text.push_str(signoff.trim());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&footer_text(brand));

    let mut html = String::new();
    html.push_str(&open_document(brand));
    if let Some(greeting) = non_empty(brand.greeting) {
        html.push_str(&format!(
            "<p style=\"margin:0 0 16px;font-size:15px;color:#111827\">{}</p>",
            escape_html(greeting.trim())
        ));
    }
    html.push_str(&format!(
        "<p style=\"margin:0 0 8px;font-size:15px;font-weight:600;color:#111827\">{}</p>",
        escape_html(title.trim())
    ));
    if let Some(body) = non_empty(body) {
        html.push_str(&format!(
            "<p style=\"margin:0 0 16px;font-size:14px;line-height:1.5;color:#374151\">{}</p>",
            escape_html(body.trim())
        ));
    }
    html.push_str(&close_document(brand));
    BrandedEmail {
        subject: title.trim().to_string(),
        text_body: text,
        html_body: html,
    }
}

/// Renders the daily digest: one subject, one list of unread items.
/// `day` is the recipient-local date label (`YYYY-MM-DD`).
#[must_use]
pub fn render_digest(brand: &Brand<'_>, day: &str, items: &[DigestItem]) -> BrandedEmail {
    let count = items.len();
    let subject = if count == 1 {
        "Your daily digest: 1 unread notification".to_string()
    } else {
        format!("Your daily digest: {count} unread notifications")
    };

    let mut text = String::new();
    if let Some(greeting) = non_empty(brand.greeting) {
        text.push_str(greeting.trim());
        text.push_str("\n\n");
    }
    text.push_str(&format!(
        "You have {count} unread notification{} on {day}:\n\n",
        if count == 1 { "" } else { "s" }
    ));
    for item in items {
        text.push_str("- ");
        text.push_str(item.title.trim());
        if let Some(detail) = non_empty(item.detail.as_deref()) {
            text.push_str(" — ");
            text.push_str(detail.trim());
        }
        text.push('\n');
    }
    if let Some(signoff) = non_empty(brand.signoff) {
        text.push('\n');
        text.push_str(signoff.trim());
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&footer_text(brand));

    let mut html = String::new();
    html.push_str(&open_document(brand));
    if let Some(greeting) = non_empty(brand.greeting) {
        html.push_str(&format!(
            "<p style=\"margin:0 0 16px;font-size:15px;color:#111827\">{}</p>",
            escape_html(greeting.trim())
        ));
    }
    html.push_str(&format!(
        "<p style=\"margin:0 0 12px;font-size:15px;color:#111827\">You have <strong>{count}</strong> unread notification{} on {}.</p>",
        if count == 1 { "" } else { "s" },
        escape_html(day)
    ));
    html.push_str("<ul style=\"margin:0 0 16px;padding-left:20px\">");
    for item in items {
        html.push_str(&format!(
            "<li style=\"margin:0 0 6px;font-size:14px;line-height:1.5;color:#374151\">{}{}</li>",
            escape_html(item.title.trim()),
            non_empty(item.detail.as_deref())
                .map(|d| format!(
                    "<span style=\"color:#6b7280\"> — {}</span>",
                    escape_html(d.trim())
                ))
                .unwrap_or_default()
        ));
    }
    html.push_str("</ul>");
    html.push_str(&close_document(brand));
    BrandedEmail {
        subject,
        text_body: text,
        html_body: html,
    }
}

/// Renders the admin test-send probe: a real branded message stating what
/// was verified, so nobody mistakes it for a client-facing mail.
#[must_use]
pub fn render_test(brand: &Brand<'_>, transport_label: &str, from_address: &str) -> BrandedEmail {
    let title = "Marula test email";
    let body = format!(
        "This message confirms the configured mail transport can render and send. \
         Transport: {transport_label}. From: {from_address}."
    );
    render_notification(brand, title, Some(&body))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn footer_text(brand: &Brand<'_>) -> String {
    let name = brand.brand_name.trim();
    if name.is_empty() {
        "Sent by Marula.".to_string()
    } else {
        format!("Sent by {name}.")
    }
}

fn open_document(brand: &Brand<'_>) -> String {
    let header = match non_empty(brand.logo_url) {
        Some(url) => format!(
            "<img src=\"{}\" alt=\"{}\" height=\"28\" style=\"display:block;border:0\" />",
            escape_html(url),
            escape_html(brand.brand_name.trim())
        ),
        None => format!(
            "<div style=\"font-size:16px;font-weight:600;letter-spacing:-0.01em;color:#111827\">{}</div>",
            escape_html(brand.brand_name.trim())
        ),
    };
    format!(
        "<!doctype html><html><body style=\"margin:0;padding:0;background:#ffffff\">\
         <div style=\"max-width:560px;margin:0 auto;padding:32px 24px;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif\">\
         {header}\
         <div style=\"border-top:1px solid #e5e7eb;margin:16px 0 24px\"></div>"
    )
}

fn close_document(brand: &Brand<'_>) -> String {
    let signoff = non_empty(brand.signoff)
        .map(|s| {
            format!(
                "<p style=\"margin:0 0 24px;font-size:14px;line-height:1.5;color:#374151;white-space:pre-line\">{}</p>",
                escape_html(s.trim())
            )
        })
        .unwrap_or_default();
    format!(
        "{signoff}\
         <div style=\"border-top:1px solid #e5e7eb;margin:0 0 12px\"></div>\
         <p style=\"margin:0;font-size:12px;color:#6b7280\">{}</p>\
         </div></body></html>",
        escape_html(&footer_text(brand))
    )
}

/// Escapes the five HTML-significant characters. Every dynamic value in
/// this module crosses this function exactly once.
#[must_use]
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brand() -> Brand<'static> {
        Brand {
            brand_name: "Acme MSP",
            logo_url: None,
            greeting: Some("Hi Dana,"),
            signoff: Some("The Acme team\n+1 555 0100"),
        }
    }

    #[test]
    fn notification_renders_both_parts_with_greeting_and_signoff() {
        let mail = render_notification(&brand(), "Ticket MER-42 was assigned to you", Some("Due today."));
        assert_eq!(mail.subject, "Ticket MER-42 was assigned to you");
        assert!(mail.text_body.starts_with("Hi Dana,"));
        assert!(mail.text_body.contains("Due today."));
        assert!(mail.text_body.contains("The Acme team"));
        assert!(mail.text_body.contains("Sent by Acme MSP."));
        assert!(mail.html_body.contains("<!doctype html>"));
        assert!(mail.html_body.contains("The Acme team"));
        assert!(mail.html_body.contains("Sent by Acme MSP."));
    }

    #[test]
    fn dynamic_values_are_html_escaped_once() {
        let evil = Brand {
            brand_name: "Acme <script>",
            logo_url: Some("https://x.test/logo.png?a=1&b=2"),
            greeting: None,
            signoff: None,
        };
        let mail = render_notification(&evil, "5 < 6 & \"quoted\"", Some("<b>bold</b>"));
        assert!(!mail.html_body.contains("<script>"));
        assert!(mail.html_body.contains("Acme &lt;script&gt;"));
        assert!(mail.html_body.contains("5 &lt; 6 &amp; &quot;quoted&quot;"));
        assert!(mail.html_body.contains("&lt;b&gt;bold&lt;/b&gt;"));
        assert!(mail.html_body.contains("a=1&amp;b=2"));
        // Plain text stays literal — escaping is an HTML concern only.
        assert!(mail.text_body.contains("<b>bold</b>"));
    }

    #[test]
    fn empty_greeting_and_signoff_render_clean() {
        let bare = Brand {
            brand_name: "Acme",
            logo_url: None,
            greeting: Some("   "),
            signoff: None,
        };
        let mail = render_notification(&bare, "Subject line", None);
        assert!(!mail.text_body.contains("Hi"));
        assert!(mail.text_body.starts_with("Subject line"));
        assert!(!mail.html_body.contains("white-space:pre-line"));
    }

    #[test]
    fn digest_subject_and_count_agree() {
        let items = vec![DigestItem {
            title: "SLA breach on MER-7".into(),
            detail: Some("P1 overdue".into()),
        }];
        let one = render_digest(&brand(), "2026-09-26", &items);
        assert_eq!(one.subject, "Your daily digest: 1 unread notification");
        assert!(one.text_body.contains("2026-09-26"));
        assert!(one.text_body.contains("SLA breach on MER-7 — P1 overdue"));

        let many = vec![
            DigestItem {
                title: "A".into(),
                detail: None,
            },
            DigestItem {
                title: "B".into(),
                detail: None,
            },
        ];
        let digest = render_digest(&brand(), "2026-09-26", &many);
        assert_eq!(digest.subject, "Your daily digest: 2 unread notifications");
        assert!(digest.html_body.contains("<strong>2</strong>"));
        assert!(digest.text_body.contains("- A\n- B\n"));
    }

    #[test]
    fn empty_brand_name_falls_back_to_product_footer() {
        let anon = Brand {
            brand_name: "",
            logo_url: None,
            greeting: None,
            signoff: None,
        };
        let mail = render_notification(&anon, "x", None);
        assert!(mail.text_body.contains("Sent by Marula."));
        assert!(mail.html_body.contains("Sent by Marula."));
    }

    #[test]
    fn logo_renders_as_image_when_url_present() {
        let with_logo = Brand {
            brand_name: "Acme",
            logo_url: Some("https://cdn.acme.test/logo.png"),
            greeting: None,
            signoff: None,
        };
        let mail = render_notification(&with_logo, "x", None);
        assert!(mail.html_body.contains("<img src=\"https://cdn.acme.test/logo.png\""));
        assert!(!mail.html_body.contains("letter-spacing:-0.01em"));
    }

    #[test]
    fn test_probe_names_transport_and_from() {
        let mail = render_test(&brand(), "smtp", "support@acme.test");
        assert_eq!(mail.subject, "Marula test email");
        assert!(mail.text_body.contains("Transport: smtp"));
        assert!(mail.text_body.contains("From: support@acme.test"));
    }
}
