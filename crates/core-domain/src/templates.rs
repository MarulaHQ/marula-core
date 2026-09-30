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
pub fn variables_for_kind(kind: TemplateKind) -> &'static [&'static str] {
    match kind {
        TemplateKind::Ticket
        | TemplateKind::Note
        | TemplateKind::Email
        | TemplateKind::Workflow => &[
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
}

/// The shipped default roster: one per kind.
pub const DEFAULT_TEMPLATES: &[DefaultTemplate] = &[
    DefaultTemplate {
        builtin: "standard_service_ticket",
        kind: TemplateKind::Ticket,
        slug: "standard_service_ticket",
        name: "Standard service ticket",
        description: "Standard intake skeleton for a managed-service request.",
        subject: None,
        body: "## Request\n{{client.name}} reported: {{ticket.summary}}\n\n## Triage\nPriority: {{ticket.priority}}\nStatus: {{ticket.status}}\n\n## Steps\n- [ ] Confirm scope with {{contact.name}}\n- [ ] Investigate\n- [ ] Document resolution\n",
    },
    DefaultTemplate {
        builtin: "ticket_resolution_note",
        kind: TemplateKind::Note,
        slug: "ticket_resolution_note",
        name: "Ticket resolution note",
        description: "Closing-note skeleton: what was wrong, what fixed it.",
        subject: None,
        body: "Root cause: \nResolution: \nVerified with {{contact.name}} on {{ticket.number}}.\n",
    },
    DefaultTemplate {
        builtin: "ticket_first_response",
        kind: TemplateKind::Email,
        slug: "ticket_first_response",
        name: "Ticket first response",
        description: "Warm acknowledgement email for a newly triaged ticket.",
        subject: Some("Re: {{ticket.summary}} ({{ticket.number}})"),
        body: "Hi {{contact.name}},\n\nThanks for reaching out — {{ticket.summary}} is logged as {{ticket.number}} and is currently {{ticket.priority}} priority.\n\nWe will keep you posted here. Reply to this email to add detail.\n\n{{user.name}}\n",
    },
    DefaultTemplate {
        builtin: "article_starter",
        kind: TemplateKind::Kb,
        slug: "article_starter",
        name: "KB article starter",
        description: "Problem/cause/fix skeleton for a staff KB article.",
        subject: None,
        body: "## Problem\n\n## Environment\n\n## Cause\n\n## Fix\n\nMaintained by {{user.name}}.\n",
    },
    DefaultTemplate {
        builtin: "standard_quote_cover",
        kind: TemplateKind::QuoteInvoice,
        slug: "standard_quote_cover",
        name: "Standard quote cover note",
        description: "Cover-email body for a sent quote from the quote catalog.",
        subject: Some("Quote {{quote.number}} from {{org.name}}"),
        body: "Hi {{contact.name}},\n\nPlease find our quote {{quote.number}}: {{quote.total}} {{quote.currency}}, valid as sent.\n\nHappy to walk through any line.\n\n{{user.name}}\n",
    },
    DefaultTemplate {
        builtin: "client_onboarding_scaffold",
        kind: TemplateKind::Project,
        slug: "client_onboarding_scaffold",
        name: "Client onboarding scaffold",
        description: "Phase/plan skeleton for onboarding a new client.",
        subject: None,
        body: "## Onboarding: {{client.name}}\n\n1. Kickoff with {{contact.name}}\n2. Asset inventory\n3. Access and credentials handover\n4. Monitoring baseline\n5. First review\n",
    },
    DefaultTemplate {
        builtin: "escalation_playbook",
        kind: TemplateKind::Workflow,
        slug: "escalation_playbook",
        name: "Escalation playbook",
        description: "Escalation runbook body for out-of-SLA tickets.",
        subject: None,
        body: "## Escalate {{ticket.number}}\n\nTrigger: SLA at risk for {{client.name}}.\n\n1. Notify {{assignee.name}}\n2. Engage {{contact.name}}\n3. Escalate internally if unresolved in 2h\n",
    },
];

/// Sanity gate at compile time of the test suite: every shipped default
/// only references variables inside its kind's closed vocabulary.
#[cfg(test)]
mod default_vocab_tests {
    use super::*;

    #[test]
    fn shipped_defaults_use_only_in_vocab_variables() {
        for d in DEFAULT_TEMPLATES {
            for text in [d.body, d.subject.unwrap_or_default()] {
                // Render against an empty context: only in-vocab keys pass
                // the vocabulary gate (missing values render empty).
                render(d.kind, text, &RenderContext::new())
                    .unwrap_or_else(|e| panic!("default `{}` invalid: {e}", d.builtin));
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
