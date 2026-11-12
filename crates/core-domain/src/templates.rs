//! Typed template variables + render engine (Wave 13 lane 5, KB3, ADR 0004).
//!
//! Templates carry `{{path}}` placeholders drawn from a CLOSED per-kind
//! vocabulary. Resolution happens at render time from real entity context
//! (`RenderContext`), never raw string interpolation of user data:
//!
//!   * a placeholder outside the kind's vocabulary is a typed render error
//!     (it can never silently expand empty);
//!   * a resolved value is inserted as plain text — no SQL, no HTML, no
//!     re-interpretation downstream;
//!   * the vocabulary is one function (`variables_for_kind`), so the
//!     editor, the validator, and the renderer cannot drift.
//!
//! Pure domain: no I/O. Contexts arrive as string maps built by core-db
//! from the joined entity row (ticket + client + contact + assignee +
//! org, project + client, quote + client).

use crate::entities::templates::TemplateKind;
use std::collections::BTreeMap;

/// Render errors are typed so the API can surface `template_unknown_variable`
/// with the offending key instead of a generic 400.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RenderError {
    #[error("unknown template variable `{0}` for kind `{1}`")]
    UnknownVariable(String, String),
    #[error("malformed template placeholder near `{0}`")]
    MalformedPlaceholder(String),
}

/// The render context: every variable the templates may reference for one
/// entity, already resolved to display strings.
pub type RenderContext = BTreeMap<String, String>;

/// The closed variable vocabulary per kind (KB3). Common keys resolve for
/// every kind; kind-specific keys extend it.
///
/// The `email` kind carries the system-outbound keys in addition to the
/// common entity keys: the seeded outbound templates (migration 0182 —
/// invitation, ticket confirmation, comment notification, invoice
/// statement, and the auth-token links) and every fork of them can only
/// reference what the enqueue path actually feeds. A key outside this
/// list can never be written into an email template, so the enqueue
/// renderer can never meet an unknown placeholder it cannot resolve.
pub fn variables_for_kind(kind: TemplateKind) -> &'static [&'static str] {
    match kind {
        TemplateKind::Email => &[
            "org.name",
            "user.name",
            "client.name",
            "contact.name",
            "contact.email",
            "contact.phone",
            "ticket.number",
            "ticket.summary",
            "ticket.priority",
            "ticket.status",
            "assignee.name",
            // System-outbound keys. Each names a value the enqueue path
            // builds for exactly one outbound kind (`OutboundKind::
            // template_slug` + `render_context` are the feed side); a key
            // absent from that kind's context renders empty, never an
            // error — the variable was typed and permitted, the message
            // simply carries no value for it.
            "comment.excerpt",
            "invoice.currency",
            "invoice.total_outstanding",
            "invoice.lines",
            "invite.display_name",
            "invite.inviter_name",
            "invite.url",
            "invite.expires_at",
            "link.url",
        ],
        TemplateKind::Ticket | TemplateKind::Note | TemplateKind::Workflow => &[
            "org.name",
            "user.name",
            "client.name",
            "contact.name",
            "contact.email",
            "contact.phone",
            "ticket.number",
            "ticket.summary",
            "ticket.priority",
            "ticket.status",
            "assignee.name",
        ],
        TemplateKind::Kb => &["org.name", "user.name"],
        TemplateKind::QuoteInvoice => &[
            "org.name",
            "user.name",
            "client.name",
            "contact.name",
            "contact.email",
            "contact.phone",
            "quote.number",
            "quote.status",
            "quote.total",
            "quote.currency",
        ],
        TemplateKind::Project => &[
            "org.name",
            "user.name",
            "client.name",
            "contact.name",
            "contact.email",
            "contact.phone",
            "project.name",
            "project.status",
        ],
    }
}

/// Validates that every declared variable key is inside the kind's closed
/// vocabulary. Write-time gate (create/fork/patch); render re-checks anyway.
pub fn validate_variables(kind: TemplateKind, keys: &[String]) -> Result<(), RenderError> {
    let vocab = variables_for_kind(kind);
    for key in keys {
        if !vocab.contains(&key.as_str()) {
            return Err(RenderError::UnknownVariable(
                key.clone(),
                kind.as_str().to_string(),
            ));
        }
    }
    Ok(())
}

/// Extracts every `{{path}}` placeholder in `text` (malformed placeholders
/// are errors — the same rule `render` enforces).
pub fn extract_placeholders(text: &str) -> Result<Vec<String>, RenderError> {
    let mut keys = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let start = i + 2;
            let Some(rel_end) = text[start..].find("}}") else {
                return Err(RenderError::MalformedPlaceholder(
                    text[i..].chars().take(24).collect(),
                ));
            };
            let key = text[start..start + rel_end].trim();
            if key.is_empty() || key.chars().any(|c| c == '{' || c == '}') {
                return Err(RenderError::MalformedPlaceholder(
                    text[i..(start + rel_end + 2).min(text.len())]
                        .chars()
                        .take(24)
                        .collect(),
                ));
            }
            if !keys.iter().any(|k: &String| k == key) {
                keys.push(key.to_string());
            }
            i = start + rel_end + 2;
        } else {
            let ch_len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            i += ch_len;
        }
    }
    Ok(keys)
}

/// Write-time content gate (KB3): every placeholder in subject + body must
/// live inside the kind's closed vocabulary, malformed ones reject — an
/// out-of-vocabulary placeholder can never be written, so render can never
/// silently expand it empty.
pub fn validate_content(
    kind: TemplateKind,
    subject: Option<&str>,
    body: &str,
) -> Result<(), RenderError> {
    for text in [body, subject.unwrap_or_default()] {
        let keys = extract_placeholders(text)?;
        validate_variables(kind, &keys)?;
    }
    Ok(())
}

/// Renders `{{path}}` placeholders in `text` from `ctx`. Unknown or
/// out-of-vocabulary variables are hard errors — never silent empties
/// (ADR 0004: typed variables, resolved from entity context).
pub fn render(kind: TemplateKind, text: &str, ctx: &RenderContext) -> Result<String, RenderError> {
    render_with(text, variables_for_kind(kind), kind.as_str(), ctx)
}

/// The SIGNATURE vocabulary (Wave 42). A signature identifies the person
/// who wrote the message, so it gets the sender's own row and nothing
/// else: no ticket, no client, no contact.
///
/// It is exactly the set the `users` row can fill today: `display_name`,
/// `email`, and the org's name. A `users.title` or `users.phone` column
/// would add its entry here in the same commit that adds the column.
/// Offering a token that can only ever render empty is the one thing this
/// vocabulary must not do, because the editor would promise a field the
/// send path cannot deliver — so the closed set is also the offerable set.
pub const SIGNATURE_VARIABLES: &[&str] = &["org.name", "user.name", "user.email"];

/// Human labels for the signature placeholders — the token picker's rows
/// come from this, so the picker and the vocabulary cannot drift.
pub const SIGNATURE_VARIABLE_LABELS: &[(&str, &str)] = &[
    ("user.name", "Your name"),
    ("user.email", "Your email address"),
    ("org.name", "Workspace name"),
];

/// Write-time gate for a signature (org default or personal): every
/// placeholder must be in the signature vocabulary, malformed ones reject.
/// Same contract as `validate_content`, over the signature vocabulary.
pub fn validate_signature(text: &str) -> Result<(), RenderError> {
    for key in extract_placeholders(text)? {
        if !SIGNATURE_VARIABLES.contains(&key.as_str()) {
            return Err(RenderError::UnknownVariable(key, "signature".to_string()));
        }
    }
    Ok(())
}

/// Renders a signature through the SAME engine and the SAME error type as
/// every other template — this is a vocabulary, not a second dialect.
pub fn render_signature(text: &str, ctx: &RenderContext) -> Result<String, RenderError> {
    render_with(text, SIGNATURE_VARIABLES, "signature", ctx)
}

/// The one placeholder scanner. Every renderer is a vocabulary over it, so
/// a malformed placeholder, an empty `{{}}`, or a nested brace behaves
/// identically wherever it is met.
fn render_with(
    text: &str,
    vocab: &[&str],
    kind_label: &str,
    ctx: &RenderContext,
) -> Result<String, RenderError> {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let start = i + 2;
            let Some(rel_end) = text[start..].find("}}") else {
                return Err(RenderError::MalformedPlaceholder(
                    text[i..].chars().take(24).collect(),
                ));
            };
            let key = text[start..start + rel_end].trim();
            if key.is_empty() || key.chars().any(|c| c == '{' || c == '}') {
                return Err(RenderError::MalformedPlaceholder(
                    text[i..(start + rel_end + 2).min(text.len())]
                        .chars()
                        .take(24)
                        .collect(),
                ));
            }
            if !vocab.contains(&key) {
                return Err(RenderError::UnknownVariable(
                    key.to_string(),
                    kind_label.to_string(),
                ));
            }
            // A vocabulary key missing from the context (e.g. no contact on
            // the ticket) renders empty — the variable was typed and
            // permitted, the entity simply has no value for it.
            out.push_str(ctx.get(key).map(String::as_str).unwrap_or_default());
            i = start + rel_end + 2;
        } else {
            let ch_len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
            out.push_str(&text[i..i + ch_len]);
            i += ch_len;
        }
    }
    Ok(out)
}

/// Renders subject + body in one call. `subject` may be `None` for
/// body-only kinds.
pub fn render_template(
    kind: TemplateKind,
    subject: Option<&str>,
    body: &str,
    ctx: &RenderContext,
) -> Result<(Option<String>, String), RenderError> {
    Ok((
        subject.map(|s| render(kind, s, ctx)).transpose()?,
        render(kind, body, ctx)?,
    ))
}

/// One shipped default template (KB4). The canonical copy the orgs see as
/// system rows (seeded by migration 0070) and restore resets to. The
/// migration carries its own copy of these definitions — a core-db test
/// pins the rows byte-identical to this list (0071's drift guard).
#[derive(Debug, Clone, Copy)]
pub struct DefaultTemplate {
    pub builtin: &'static str,
    pub kind: TemplateKind,
    pub slug: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub subject: Option<&'static str>,
    pub body: &'static str,
    /// The optional branded HTML part, stored beside the text body the
    /// same way `email_messages.body_html` rides beside `body_text`
    /// (migration 0182). `None` for every text-only template: the column
    /// exists so a branded email can carry its styled twin, not so every
    /// template grows one.
    pub html: Option<&'static str>,
}

/// The shipped default roster: the seven entity-kind scaffolds (one per
/// kind), seeded as immutable system rows. The system-outbound email
/// set lives in [`OUTBOUND_EMAIL_TEMPLATES`] and seeds as editable org
/// rows. Both migrations carry their own copies of these definitions —
/// core-db tests pin the rows byte-identical to these lists (0071's
/// drift guard).
pub const DEFAULT_TEMPLATES: &[DefaultTemplate] = &[
    DefaultTemplate {
        builtin: "standard_service_ticket",
        kind: TemplateKind::Ticket,
        slug: "standard_service_ticket",
        name: "Standard service ticket",
        description: "Standard intake skeleton for a managed-service request.",
        subject: None,
        body: "## Request\n{{client.name}} reported: {{ticket.summary}}\n\n## Triage\nPriority: {{ticket.priority}}\nStatus: {{ticket.status}}\n\n## Steps\n- [ ] Confirm scope with {{contact.name}}\n- [ ] Investigate\n- [ ] Document resolution\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "ticket_resolution_note",
        kind: TemplateKind::Note,
        slug: "ticket_resolution_note",
        name: "Ticket resolution note",
        description: "Closing-note skeleton: what was wrong, what fixed it.",
        subject: None,
        body: "Root cause: \nResolution: \nVerified with {{contact.name}} on {{ticket.number}}.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "ticket_first_response",
        kind: TemplateKind::Email,
        slug: "ticket_first_response",
        name: "Ticket first response",
        description: "Warm acknowledgement email for a newly triaged ticket.",
        subject: Some("Re: {{ticket.summary}} ({{ticket.number}})"),
        body: "Hi {{contact.name}},\n\nThanks for reaching out — {{ticket.summary}} is logged as {{ticket.number}} and is currently {{ticket.priority}} priority.\n\nWe will keep you posted here. Reply to this email to add detail.\n\n{{user.name}}\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "article_starter",
        kind: TemplateKind::Kb,
        slug: "article_starter",
        name: "KB article starter",
        description: "Problem/cause/fix skeleton for a staff KB article.",
        subject: None,
        body: "## Problem\n\n## Environment\n\n## Cause\n\n## Fix\n\nMaintained by {{user.name}}.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "standard_quote_cover",
        kind: TemplateKind::QuoteInvoice,
        slug: "standard_quote_cover",
        name: "Standard quote cover note",
        description: "Cover-email body for a sent quote from the quote catalog.",
        subject: Some("Quote {{quote.number}} from {{org.name}}"),
        body: "Hi {{contact.name}},\n\nPlease find our quote {{quote.number}}: {{quote.total}} {{quote.currency}}, valid as sent.\n\nHappy to walk through any line.\n\n{{user.name}}\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "client_onboarding_scaffold",
        kind: TemplateKind::Project,
        slug: "client_onboarding_scaffold",
        name: "Client onboarding scaffold",
        description: "Phase/plan skeleton for onboarding a new client.",
        subject: None,
        body: "## Onboarding: {{client.name}}\n\n1. Kickoff with {{contact.name}}\n2. Asset inventory\n3. Access and credentials handover\n4. Monitoring baseline\n5. First review\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "escalation_playbook",
        kind: TemplateKind::Workflow,
        slug: "escalation_playbook",
        name: "Escalation playbook",
        description: "Escalation runbook body for out-of-SLA tickets.",
        subject: None,
        body: "## Escalate {{ticket.number}}\n\nTrigger: SLA at risk for {{client.name}}.\n\n1. Notify {{assignee.name}}\n2. Engage {{contact.name}}\n3. Escalate internally if unresolved in 2h\n",
        html: None,
    },
];

/// The system-outbound email templates (migration 0182): one per
/// `core_mail::outbound::OutboundKind` variant that renders a system-sent
/// email. Each mirrors the kind's hardcoded fallback rendering
/// byte-for-byte (subject + text body; the branded HTML for the
/// invitation), with the fallback's format arguments expressed as the
/// email-kind placeholders — so a fresh org's mail is unchanged whether
/// the stored row or the fallback applies, and an org can edit the copy
/// without touching the binary.
///
/// These seed as EDITABLE ORG ROWS (`is_system = false`), not shipped
/// defaults — deliberately off the defaults-first doctrine, for a reason
/// the enqueue path dictates: the lookup reads the org's row by the
/// kind's FIXED slug, so forking cannot rewire it, and an immutable
/// system row would make the stored copy uneditable and the builtin
/// fallback unreachable. Orgs own these rows exactly like any other
/// template: edit them, delete them (the fallback then applies and
/// `email_messages.template_source` records which copy went out), and
/// the auth-token templates carry friendlier TTL wording than the
/// fallback's raw seconds ("valid 900" → "valid 15 minutes"); the
/// fallbacks stay byte-identical to what shipped before this lane.
pub const OUTBOUND_EMAIL_TEMPLATES: &[DefaultTemplate] = &[
    DefaultTemplate {
        builtin: "user_invited",
        kind: TemplateKind::Email,
        slug: "user_invited",
        name: "Workspace invitation",
        description: "The invitation email a new member receives when an admin adds them to the workspace.",
        subject: Some("You've been invited to join {{org.name}}"),
        body: "Hi {{invite.display_name}},\n\n{{invite.inviter_name}} invited you to join {{org.name}} on Marula, the workspace your team runs service work in.\n\nAccept your invitation:\n{{invite.url}}\n\nThe link expires on {{invite.expires_at}}. If you weren't expecting this invitation you can ignore this email — your address won't be added to the workspace until you accept.\n",
        html: Some(r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"></head>
<body style="margin:0;padding:0;background:#F8F7F4;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#F8F7F4;padding:32px 16px;">
<tr><td align="center">
<table role="presentation" width="560" cellpadding="0" cellspacing="0" style="max-width:560px;width:100%;background:#FFFFFF;border:1px solid #E2E0DA;border-radius:16px;font-family:-apple-system,'Segoe UI',Helvetica,Arial,sans-serif;">
  <tr><td style="padding:32px 36px 0 36px;">
    <p style="margin:0 0 4px 0;font-family:Georgia,'Times New Roman',serif;font-size:26px;line-height:32px;color:#2B2A33;font-weight:500;">
      Hi {{invite.display_name}}, you've been invited</p>
    <p style="margin:0;font-size:15px;line-height:24px;color:#6F685D;">
      {{invite.inviter_name}} invited you to join <strong style="color:#2B2A33;">{{org.name}}</strong> on Marula —
      the workspace your team runs service work in.</p>
  </td></tr>
  <tr><td style="padding:28px 36px 0 36px;">
    <table role="presentation" cellpadding="0" cellspacing="0" width="100%"><tr><td align="center">
      <a href="{{invite.url}}" style="display:inline-block;background:#2B2A33;color:#E8B83A;font-size:15px;font-weight:600;line-height:20px;padding:12px 28px;border-radius:10px;text-decoration:none;">Accept your invitation</a>
    </td></tr></table>
    <p style="margin:14px 0 0 0;text-align:center;font-size:13px;line-height:20px;color:#6F685D;">
      If the button doesn't work, copy this link into your browser:<br>
      <a href="{{invite.url}}" style="color:#9A5B0F;word-break:break-all;">{{invite.url}}</a></p>
  </td></tr>
  <tr><td style="padding:28px 36px 32px 36px;">
    <p style="margin:0;padding-top:16px;border-top:1px solid #E2E0DA;font-size:13px;line-height:20px;color:#6F685D;">
      This link expires on {{invite.expires_at}}. If you weren't expecting this invitation,
      you can ignore this email — your address won't be added to the workspace
      until you accept.</p>
  </td></tr>
</table>
</td></tr>
</table>
</body>
</html>"#),
    },
    DefaultTemplate {
        builtin: "ticket_created_confirmation",
        kind: TemplateKind::Email,
        slug: "ticket_created_confirmation",
        name: "Ticket created confirmation",
        description: "Auto-responder acknowledging an email that opened a new ticket.",
        subject: Some("[{{ticket.number}}] {{ticket.summary}} — we received your request"),
        body: "Your request \"{{ticket.summary}}\" was received and registered as ticket {{ticket.number}}.\nReply to this email to add information — keep the [{{ticket.number}}] tag in the subject.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "comment_notification",
        kind: TemplateKind::Email,
        slug: "comment_notification",
        name: "Comment notification",
        description: "Requester notification that a client-visible reply landed on their ticket.",
        subject: Some("Re: [{{ticket.number}}] {{ticket.summary}}"),
        body: "New reply on ticket {{ticket.number}}:\n\n{{comment.excerpt}}\n\nReply to this email to respond.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "invoice_statement",
        kind: TemplateKind::Email,
        slug: "invoice_statement",
        name: "Invoice statement",
        description: "Account statement on the dunning path: the client's open invoices with outstanding balances.",
        subject: Some("Account statement — {{client.name}}"),
        body: "Your current account statement:\n\n{{invoice.lines}}\n\nTotal outstanding: {{invoice.total_outstanding}} {{invoice.currency}}\n\nIf you have already sent payment, please disregard this notice.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "magic_link",
        kind: TemplateKind::Email,
        slug: "magic_link",
        name: "Magic-link sign-in",
        description: "The single-use sign-in link email for orgs with magic-link sign-in enabled.",
        subject: Some("Your Marula sign-in link"),
        body: "Open this single-use link to sign in (valid 15 minutes):\n{{link.url}}\n\nIf it wasn't you, ignore this mail.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "password_reset",
        kind: TemplateKind::Email,
        slug: "password_reset",
        name: "Password reset",
        description: "The single-use password reset link email.",
        subject: Some("Reset your Marula password"),
        body: "A password reset was requested for your account.\n\nOpen this single-use link (valid 30 minutes):\n{{link.url}}\n\nIf it wasn't you, ignore this mail.\n",
        html: None,
    },
    DefaultTemplate {
        builtin: "email_verification",
        kind: TemplateKind::Email,
        slug: "email_verification",
        name: "Email verification",
        description: "The single-use email-verification link email.",
        subject: Some("Verify your Marula email"),
        body: "Confirm your Marula account email.\n\nOpen this single-use link (valid 24 hours):\n{{link.url}}\n",
        html: None,
    },
];

/// Sanity gate at compile time of the test suite: every shipped default
/// only references variables inside its kind's closed vocabulary.
#[cfg(test)]
mod default_vocab_tests {
    use super::*;

    #[test]
    fn shipped_defaults_use_only_in_vocab_variables() {
        for d in DEFAULT_TEMPLATES.iter().chain(OUTBOUND_EMAIL_TEMPLATES) {
            for text in [d.body, d.subject.unwrap_or_default()] {
                // Render against an empty context: only in-vocab keys pass
                // the vocabulary gate (missing values render empty).
                render(d.kind, text, &RenderContext::new())
                    .unwrap_or_else(|e| panic!("default `{}` invalid: {e}", d.builtin));
            }
            // The HTML part is a template text like any other: its
            // placeholders must live in the same closed vocabulary, so the
            // enqueue renderer can never meet an unknown key in stored
            // markup.
            if let Some(html) = d.html {
                render(d.kind, html, &RenderContext::new())
                    .unwrap_or_else(|e| panic!("default `{}` html invalid: {e}", d.builtin));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RenderContext {
        BTreeMap::from([
            ("org.name".to_string(), "Acme MSP".to_string()),
            ("client.name".to_string(), "Contoso Ltd".to_string()),
            ("contact.name".to_string(), "J. Doe".to_string()),
            ("ticket.number".to_string(), "T-1042".to_string()),
            (
                "ticket.summary".to_string(),
                "Laptop won't boot".to_string(),
            ),
            ("user.name".to_string(), "A. Tech".to_string()),
        ])
    }

    #[test]
    fn renders_known_variables() {
        let (subject, body) = render_template(
            TemplateKind::Email,
            Some("Re: {{ticket.number}}"),
            "Hi {{contact.name}}, re {{ticket.summary}}.",
            &ctx(),
        )
        .unwrap();
        assert_eq!(subject.as_deref(), Some("Re: T-1042"));
        assert_eq!(body, "Hi J. Doe, re Laptop won't boot.");
    }

    #[test]
    fn unknown_variable_is_a_typed_error() {
        let err = render(TemplateKind::Kb, "{{ticket.number}}", &ctx()).unwrap_err();
        assert_eq!(
            err,
            RenderError::UnknownVariable("ticket.number".into(), "kb".into())
        );
    }

    #[test]
    fn out_of_context_variable_renders_empty_not_error() {
        // ticket.kind allows assignee.name; the ticket simply has no assignee.
        assert_eq!(
            render(TemplateKind::Ticket, "{{assignee.name}}", &ctx()).unwrap(),
            ""
        );
    }

    #[test]
    fn malformed_placeholders_reject() {
        assert!(render(TemplateKind::Ticket, "{{oops", &ctx()).is_err());
        assert!(render(TemplateKind::Ticket, "{{}}", &ctx()).is_err());
        assert!(render(TemplateKind::Ticket, "{{a{b}}}", &ctx()).is_err());
    }

    #[test]
    fn validate_variables_rejects_out_of_vocab_keys() {
        assert!(validate_variables(TemplateKind::Project, &["project.name".into()]).is_ok());
        assert!(validate_variables(TemplateKind::Project, &["ticket.number".into()]).is_err());
    }

    #[test]
    fn write_time_content_gate_rejects_out_of_vocab_placeholders() {
        assert!(validate_content(TemplateKind::Kb, None, "ok {{org.name}}").is_ok());
        assert!(validate_content(TemplateKind::Kb, None, "{{ticket.number}}").is_err());
        assert!(validate_content(TemplateKind::Email, Some("Re: {{ticket.number}}"), "x").is_ok());
        assert!(validate_content(TemplateKind::Email, None, "{{oops").is_err());
        assert_eq!(
            extract_placeholders("{{a}} {{b}} {{a}}").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn braces_without_placeholders_pass_through() {
        assert_eq!(
            render(TemplateKind::Note, "json {\"a\": 1}", &ctx()).unwrap(),
            "json {\"a\": 1}"
        );
    }
}
