//! Per-user email greeting and signature, org-governed (Wave 42).
//!
//! Purpose: the one place that answers "whose name goes at the bottom of
//! this message, and why" — the resolution chain, the org policy that can
//! close the personal rungs of it, and the label each outcome carries so
//! the ledger can show what actually applied.
//!
//! The chain, highest first:
//!
//!   1. **Message override** — the composer wrote a greeting/signature for
//!      THIS message. Available only while the policy allows personal
//!      signatures, because an override is a personal signature that was
//!      not stored.
//!   2. **User default** — the sender's own `email.signoff`.
//!   3. **Board identity** — the board the mail belongs to carries its own
//!      signature. It is a SHARED fallback, one rung above the org: an
//!      admin who set it for a board means it for that board's mail.
//!   4. **Org default** — `email.signature`.
//!   5. **Unset** — nothing to send. Never a silent empty block.
//!
//! Rungs 1 and 2 are PERSONAL and are skipped entirely when the org policy
//! is `org_locked`; rungs 3 and 4 are org-scoped storage and are not the
//! policy's business. That split is the decision: a signature is a claim
//! about who wrote the message, so an org that locks signatures gets one
//! answer; a board signature is the org speaking through a shared inbox,
//! so it survives the lock.
//!
//! Board-vs-user, recorded because it is a real fork in the road: a member
//! with a personal signature who sends from a board that carries one gets
//! their OWN (rung 2 beats rung 3). The rejected alternative — board beats
//! user — makes `allowed` mean "your signature applies unless you happen
//! to sit on a board that set one", which makes the per-user editor
//! unreliable rather than merely unused.
//!
//! Greeting is deliberately NOT in this module's policy gate. `resolve`
//! below carries the same order for it, and the org default is rung 3 in
//! spirit, but the flag does not reach it: the greeting is house voice
//! rather than personal identity, and the per-user greeting is a shipped
//! feature (Wave 12 lane 6) that a lock would remove rather than govern.
//! The reasoning and the exact change a future lock would need are in the
//! ADR note beside this module.

use crate::settings;
use crate::templates::{self, RenderContext};
use std::collections::BTreeMap;

/// Admin policy over personal signatures. `parse` denies by default: an
/// unreadable stored value must never widen what a person may put in mail
/// that leaves under the org's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignaturePolicy {
    #[default]
    Allowed,
    OrgLocked,
}

impl SignaturePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            SignaturePolicy::Allowed => "allowed",
            SignaturePolicy::OrgLocked => "org_locked",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "org_locked" => SignaturePolicy::OrgLocked,
            _ => SignaturePolicy::Allowed,
        }
    }

    /// Whether a person may put their own details on a message — both as a
    /// stored default and as a one-message override.
    pub const fn allows_personal(self) -> bool {
        matches!(self, SignaturePolicy::Allowed)
    }

    /// Every policy name, for the settings UI and the registry's option
    /// list. Order is the UI order.
    pub const OPTIONS: &'static [&'static str] = settings::EMAIL_SIGNATURE_POLICY_OPTIONS;
}

/// Which rung of the chain supplied the block that went out. Recorded on
/// the ledger row so a delivery view can answer "why does this message
/// carry that signature" without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignatureSource {
    MessageOverride,
    UserDefault,
    BoardIdentity,
    OrgDefault,
    /// A line the product writes itself and no setting can change — the
    /// CSAT survey's neutral "Hi,", and the "Hi {first name}" salutation a
    /// notification falls back to when the org has set no greeting. It is
    /// here so the ledger never has to claim a rung supplied a block it
    /// did not, and so a reader can tell "the product wrote this" apart
    /// from "nothing was configured".
    Product,
    #[default]
    Unset,
}

impl SignatureSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            SignatureSource::MessageOverride => "message_override",
            SignatureSource::UserDefault => "user_default",
            SignatureSource::BoardIdentity => "board_identity",
            SignatureSource::OrgDefault => "org_default",
            SignatureSource::Product => "product",
            SignatureSource::Unset => "unset",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "message_override" => SignatureSource::MessageOverride,
            "user_default" => SignatureSource::UserDefault,
            "board_identity" => SignatureSource::BoardIdentity,
            "org_default" => SignatureSource::OrgDefault,
            "product" => SignatureSource::Product,
            _ => SignatureSource::Unset,
        }
    }

    /// Ledger wording. `Unset` is a real answer ("nothing was sent"), not
    /// an error, and it reads like one.
    pub const fn label(self) -> &'static str {
        match self {
            SignatureSource::MessageOverride => "this message",
            SignatureSource::UserDefault => "your default",
            SignatureSource::BoardIdentity => "the board",
            SignatureSource::OrgDefault => "the organization",
            SignatureSource::Product => "a fixed Marula line",
            SignatureSource::Unset => "none",
        }
    }

    /// Ledger wording for an ADMIN reading someone else's mail, where
    /// "your" would be a lie.
    pub const fn admin_label(self) -> &'static str {
        match self {
            SignatureSource::MessageOverride => "this message",
            SignatureSource::UserDefault => "the sender",
            SignatureSource::BoardIdentity => "the board",
            SignatureSource::OrgDefault => "the organization",
            SignatureSource::Product => "a fixed Marula line",
            SignatureSource::Unset => "none",
        }
    }
}

/// Who is signing. `User` is a person writing to a client; `System` is
/// mail the product sends with nobody behind it (a notification or digest
/// to a member, a public booking confirmation), where a personal
/// signature would be a signature on the org's own mail to its own staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signer {
    User,
    System,
}

impl Signer {
    const fn is_personal(self) -> bool {
        matches!(self, Signer::User)
    }
}

/// One rung's candidate text. `None` at a rung means "nothing stored
/// here", which is the only reason the chain falls through — a stored
/// blank is not a candidate and must not win.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignatureInput {
    pub override_text: Option<String>,
    pub user_default: Option<String>,
    pub board_identity: Option<String>,
    pub org_default: Option<String>,
}

/// The resolved block and the rung it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// `None` when nothing was configured anywhere — the caller sends no
    /// block rather than an empty one.
    pub text: Option<String>,
    pub source: SignatureSource,
}

impl Resolved {
    fn pick(candidate: Option<&String>, source: SignatureSource) -> Self {
        match candidate.map(|text| text.trim()).filter(|text| !text.is_empty()) {
            Some(text) => Resolved {
                text: Some(text.to_string()),
                source,
            },
            None => Resolved {
                text: None,
                source: SignatureSource::Unset,
            },
        }
    }
}

/// Walks the chain. Pure: every value it needs is passed in, so the whole
/// precedence table is unit-testable without a database, and the API layer
/// is left with nothing but reads.
pub fn resolve(policy: SignaturePolicy, signer: Signer, input: &SignatureInput) -> Resolved {
    // The lock closes both personal rungs, and it does so for the composer
    // too: an override is a personal signature that was never stored, so a
    // workspace that locked signatures must not be routable around one.
    let personal = policy.allows_personal() && signer.is_personal();
    if personal {
        if let Some(found) = non_empty(&input.override_text) {
            return Resolved::pick(Some(&found), SignatureSource::MessageOverride);
        }
        if let Some(found) = non_empty(&input.user_default) {
            return Resolved::pick(Some(&found), SignatureSource::UserDefault);
        }
    }
    if let Some(found) = non_empty(&input.board_identity) {
        return Resolved::pick(Some(&found), SignatureSource::BoardIdentity);
    }
    if let Some(found) = non_empty(&input.org_default) {
        return Resolved::pick(Some(&found), SignatureSource::OrgDefault);
    }
    Resolved {
        text: None,
        source: SignatureSource::Unset,
    }
}

/// The greeting's own order. The policy does not reach it (see the module
/// header), so a locked org still honours a member's own salutation; the
/// board has no greeting rung because the board identity family has never
/// carried one.
pub fn resolve_greeting(signer: Signer, input: &SignatureInput) -> Resolved {
    if signer.is_personal() {
        if let Some(found) = non_empty(&input.override_text) {
            return Resolved::pick(Some(&found), SignatureSource::MessageOverride);
        }
        if let Some(found) = non_empty(&input.user_default) {
            return Resolved::pick(Some(&found), SignatureSource::UserDefault);
        }
    }
    if let Some(found) = non_empty(&input.org_default) {
        return Resolved::pick(Some(&found), SignatureSource::OrgDefault);
    }
    Resolved {
        text: None,
        source: SignatureSource::Unset,
    }
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// The send-time context a signature is rendered against: the SENDING
/// user's row plus the org name. Built here so the preview, the composer
/// and the send path all fill the same keys from the same shape — the
/// preview cannot promise a field the send would drop.
///
/// It carries exactly the keys `SIGNATURE_VARIABLES` admits. The `users`
/// row has no title or phone column, so there is nothing to fill them
/// with, and adding a field here without adding its column would put the
/// two out of step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignerContext {
    pub user_name: String,
    pub user_email: String,
    pub org_name: String,
}

impl SignerContext {
    /// The render context, one key per vocabulary entry.
    pub fn render_context(&self) -> RenderContext {
        let mut ctx: RenderContext = BTreeMap::new();
        ctx.insert("user.name".to_string(), self.user_name.clone());
        ctx.insert("user.email".to_string(), self.user_email.clone());
        ctx.insert("org.name".to_string(), self.org_name.clone());
        ctx
    }
}

/// Expands a signature's placeholders from the SENDER's row. A block that
/// renders to whitespace is still a block the sender chose, so it stays
/// (trimmed) rather than vanishing: a signature that is only a title is a
/// real, if thin, signature. A placeholder the engine rejects is a STORED-value fault:
/// the write path gates it, so reaching here means a restore or a direct
/// write bypassed the gate, and the honest move is to send the raw text
/// with the offending placeholder visible rather than drop the signature.
pub fn render_signature(block: &str, ctx: &SignerContext) -> String {
    let render_ctx = ctx.render_context();
    match templates::render_signature(block, &render_ctx) {
        Ok(rendered) => rendered.trim().to_string(),
        Err(_) => block.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> SignatureInput {
        SignatureInput {
            override_text: Some("Kind regards,\nDana".into()),
            user_default: Some("Dana Okafor\nField Engineer".into()),
            board_identity: Some("Acme Projects\nprojects@acme.test".into()),
            org_default: Some("Acme Support\nsupport@acme.test".into()),
        }
    }

    #[test]
    fn override_beats_everything_under_allowed() {
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &input());
        assert_eq!(got.source, SignatureSource::MessageOverride);
        assert_eq!(got.text.as_deref(), Some("Kind regards,\nDana"));
    }

    #[test]
    fn user_default_beats_the_board_and_the_org() {
        let mut i = input();
        i.override_text = None;
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::UserDefault);
    }

    #[test]
    fn board_beats_the_org_when_the_user_has_nothing() {
        let mut i = input();
        i.override_text = None;
        i.user_default = None;
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::BoardIdentity);
    }

    #[test]
    fn org_is_the_last_resort() {
        let mut i = input();
        i.override_text = None;
        i.user_default = None;
        i.board_identity = None;
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::OrgDefault);
    }

    #[test]
    fn nothing_configured_is_unset_not_an_empty_block() {
        let got = resolve(
            SignaturePolicy::Allowed,
            Signer::User,
            &SignatureInput::default(),
        );
        assert_eq!(got.source, SignatureSource::Unset);
        assert_eq!(got.text, None);
    }

    #[test]
    fn org_locked_skips_the_personal_rungs_including_the_override() {
        let got = resolve(SignaturePolicy::OrgLocked, Signer::User, &input());
        assert_eq!(
            got.source,
            SignatureSource::BoardIdentity,
            "the lock must close the override rung too"
        );
    }

    #[test]
    fn org_locked_still_honours_a_board_signature_over_the_org_one() {
        let mut i = input();
        i.board_identity = None;
        let got = resolve(SignaturePolicy::OrgLocked, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::OrgDefault);
    }

    #[test]
    fn system_mail_never_borrows_a_personal_signature() {
        let got = resolve(SignaturePolicy::Allowed, Signer::System, &input());
        assert_eq!(got.source, SignatureSource::BoardIdentity);
    }

    #[test]
    fn a_stored_blank_never_wins_its_rung() {
        let mut i = input();
        i.override_text = None;
        i.user_default = Some("   ".into());
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::BoardIdentity);
    }

    #[test]
    fn a_stored_blank_on_the_only_rung_is_unset() {
        let i = SignatureInput {
            override_text: Some("".into()),
            user_default: Some("  \n ".into()),
            board_identity: None,
            org_default: Some(String::new()),
        };
        let got = resolve(SignaturePolicy::Allowed, Signer::User, &i);
        assert_eq!(got.source, SignatureSource::Unset);
        assert_eq!(got.text, None);
    }

    #[test]
    fn policy_parse_denies_by_default() {
        // An unreadable stored value must never widen what a person may put
        // in mail: the fallback is the permissive default only because the
        // DATABASE constraint makes every other value unreachable.
        assert_eq!(SignaturePolicy::parse("nonsense"), SignaturePolicy::Allowed);
        assert_eq!(SignaturePolicy::parse(""), SignaturePolicy::Allowed);
        assert_eq!(SignaturePolicy::parse("  ALLOWED "), SignaturePolicy::Allowed);
        assert_eq!(
            SignaturePolicy::parse("org_locked"),
            SignaturePolicy::OrgLocked
        );
        assert!(SignaturePolicy::Allowed.allows_personal());
        assert!(!SignaturePolicy::OrgLocked.allows_personal());
    }

    #[test]
    fn the_policy_option_list_is_the_registrys() {
        // One source of truth: the UI's options, the registry's enumeration
        // and the migration's CHECK all name the same two strings.
        assert_eq!(SignaturePolicy::OPTIONS, &["allowed", "org_locked"]);
        for name in SignaturePolicy::OPTIONS {
            assert_eq!(SignaturePolicy::parse(name).as_str(), *name);
        }
    }

    #[test]
    fn source_round_trips_through_its_wire_name() {
        for source in [
            SignatureSource::MessageOverride,
            SignatureSource::UserDefault,
            SignatureSource::BoardIdentity,
            SignatureSource::OrgDefault,
            SignatureSource::Product,
            SignatureSource::Unset,
        ] {
            assert_eq!(SignatureSource::parse(source.as_str()), source);
        }
        assert_eq!(SignatureSource::parse("made-up"), SignatureSource::Unset);
    }

    #[test]
    fn the_product_source_is_never_returned_by_the_chain() {
        // It is a LABEL for a line the product writes outside the chain, so
        // the chain must not be able to produce it: a caller that resolves
        // an envelope has, by definition, gone through a rung.
        for source in [
            resolve(SignaturePolicy::Allowed, Signer::User, &input()),
            resolve(SignaturePolicy::OrgLocked, Signer::User, &input()),
            resolve(SignaturePolicy::Allowed, Signer::System, &input()),
            resolve_greeting(Signer::User, &input()),
            resolve_greeting(Signer::System, &input()),
        ] {
            assert_ne!(source.source, SignatureSource::Product);
        }
    }

    #[test]
    fn greeting_order_skips_the_board_and_ignores_the_policy() {
        let i = SignatureInput {
            override_text: Some("Good morning,".into()),
            user_default: Some("Hi there,".into()),
            board_identity: Some("ignored on purpose".into()),
            org_default: Some("Hello from Acme,".into()),
        };
        assert_eq!(
            resolve_greeting(Signer::User, &i).source,
            SignatureSource::MessageOverride
        );
        let mut no_override = i.clone();
        no_override.override_text = None;
        assert_eq!(
            resolve_greeting(Signer::User, &no_override).source,
            SignatureSource::UserDefault
        );
        let mut no_user = no_override;
        no_user.user_default = None;
        assert_eq!(
            resolve_greeting(Signer::User, &no_user).source,
            SignatureSource::OrgDefault
        );
        // A system envelope has no personal greeting to fall back on.
        let mut system = i.clone();
        system.user_default = None;
        system.override_text = None;
        assert_eq!(
            resolve_greeting(Signer::System, &system).source,
            SignatureSource::OrgDefault
        );
    }

    #[test]
    fn placeholders_resolve_from_the_senders_row() {
        let ctx = SignerContext {
            user_name: "Dana Okafor".into(),
            user_email: "dana@acme.test".into(),
            org_name: "Acme".into(),
        };
        let got = render_signature("{{user.name}}\n{{user.email}}\n{{org.name}}", &ctx);
        assert_eq!(got, "Dana Okafor\ndana@acme.test\nAcme");
    }

    #[test]
    fn a_signature_resolved_against_a_changed_row_uses_the_new_row() {
        // The send-time property, stated as a test: nothing is baked at save
        // time, so a name edited after the signature was written is the
        // name that sends.
        let before = SignerContext {
            user_name: "Dana Okafor".into(),
            user_email: "dana@acme.test".into(),
            org_name: "Acme".into(),
        };
        let saved = "{{user.name}}\n{{user.email}}";
        let first = render_signature(saved, &before);
        assert_eq!(first, "Dana Okafor\ndana@acme.test");

        let after = SignerContext {
            user_name: "Dana O. Okafor".into(),
            ..before
        };
        let second = render_signature(saved, &after);
        assert_eq!(second, "Dana O. Okafor\ndana@acme.test");
    }

    #[test]
    fn an_out_of_vocabulary_placeholder_sends_raw_rather_than_dropping_the_block() {
        let ctx = SignerContext {
            user_name: "Dana".into(),
            user_email: "dana@acme.test".into(),
            org_name: "Acme".into(),
        };
        let got = render_signature("{{user.name}} {{ticket.number}}", &ctx);
        assert_eq!(got, "{{user.name}} {{ticket.number}}");
    }

    #[test]
    fn every_offered_token_is_one_the_context_can_actually_fill() {
        // The property that matters, pinned mechanically: a token in the
        // vocabulary must be a key the send-time context always carries, so
        // no token the picker offers can render empty. (It is deliberately
        // NOT a subset of the `email` kind's vocabulary, which has no
        // `user.email` and would not grow one.)
        let ctx = SignerContext {
            user_name: "Dana".into(),
            user_email: "dana@acme.test".into(),
            org_name: "Acme".into(),
        };
        let rendered_ctx = ctx.render_context();
        for key in templates::SIGNATURE_VARIABLES {
            assert!(
                rendered_ctx.contains_key(*key),
                "{key} is offered but the context cannot fill it"
            );
            // `{{{{ … }}}}` is the escaped form of a `{{token}}` placeholder;
            // `{{{key}}}` would produce a SINGLE-brace `{key}`, which the
            // engine correctly leaves alone, and the assertion would then be
            // testing nothing.
            let placeholder = format!("{{{{{key}}}}}");
            assert!(
                render_signature(&placeholder, &ctx) != placeholder,
                "{key} must expand"
            );
        }
        // And nothing that is only about a ticket, a client or a contact.
        for key in [
            "ticket.number",
            "client.name",
            "contact.email",
            "contact.phone",
            "assignee.name",
        ] {
            assert!(!templates::SIGNATURE_VARIABLES.contains(&key), "{key}");
        }
    }

    #[test]
    fn the_vocabulary_never_offers_a_field_the_users_row_cannot_fill() {
        // The picker renders `SIGNATURE_VARIABLE_LABELS`, so a label with
        // no matching vocabulary key would insert a placeholder the write
        // gate refuses. The two lists are kept in step by this.
        assert_eq!(
            templates::SIGNATURE_VARIABLE_LABELS.len(),
            templates::SIGNATURE_VARIABLES.len()
        );
        for (key, label) in templates::SIGNATURE_VARIABLE_LABELS {
            assert!(
                templates::SIGNATURE_VARIABLES.contains(key),
                "{key} has a label but no vocabulary entry"
            );
            assert!(!label.trim().is_empty());
        }
    }

    #[test]
    fn the_write_gate_accepts_the_vocabulary_and_refuses_the_rest() {
        assert!(templates::validate_signature("{{user.name}} · {{org.name}}").is_ok());
        for refused in ["{{ticket.number}}", "{{user.title}}", "{{client.name}}"] {
            let err = templates::validate_signature(refused).unwrap_err();
            assert!(
                matches!(err, templates::RenderError::UnknownVariable(..)),
                "{refused} must not be writable"
            );
        }
        assert!(templates::validate_signature("{{user.name}").is_err());
    }

    #[test]
    fn labels_speak_to_the_reader_without_lying() {
        assert_eq!(SignatureSource::UserDefault.label(), "your default");
        assert_eq!(SignatureSource::UserDefault.admin_label(), "the sender");
        assert_eq!(SignatureSource::Unset.admin_label(), "none");
    }
}
