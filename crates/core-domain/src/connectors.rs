//! Connector vocabulary (ADR 0032, ADR 0033): the closed lists the staff UI
//! and the database both branch on, and the open list a connector declares.
//!
//! Purpose: one place that says what a record type is, what a connection's
//! status is, which record pairs may be linked, and which behaviours an action
//! may have. The migration's CHECK lists are the storage half of the same
//! vocabulary, and `tests::sql_check_lists_match_the_vocabulary` fails when the
//! two disagree — so a new record type cannot land in Rust and be silently
//! unstorable, or in SQL and be unrenderable.
//!
//! Why three different kinds of list:
//!
//! * **Closed and core-owned** — [`RecordType`], [`ConnectionStatus`],
//!   [`ActionBehaviour`]. The UI, the audit and the permission check each
//!   branch on these; a new one is a product change, not a config change.
//! * **Closed pairs** — [`LinkEndKind`] with [`LinkEndKind::pair_allowed`].
//!   A link the surface cannot draw is a link that rots, so the permitted
//!   pairs are stated once, here and in `record_links_pair_allowed`.
//! * **Open and connector-owned** — product kinds (`rmm`, `remote_access`,
//!   `inventory_control`, …). A vendor that fits no existing kind declares a
//!   new one and the UI renders it; the format is validated, the list is not.
//!
//! Non-purpose: no I/O, no manifest parsing, no SQL. The resolver that turns
//! these into chips lives in `core-db`; the manifest that declares them is the
//! plugin record.
//!
//! Permission note: two product kinds have a scope already in the vocabulary
//! (`notifications.forward`, `calls.ingest`) and the connector consent store
//! still carries a fixed per-kind scope list. It is removed when the manifest
//! becomes the source, per ADR 0032 — this module deliberately does not
//! duplicate it.

/// A Marula record a connector can reference, link or act on.
///
/// The set is closed because each variant needs a surface that can show it: a
/// list, a detail page, a permission gate. A connector that can only store a
/// reference to something the product cannot open has produced a dead link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    Configuration,
    Client,
    Site,
    Contact,
    Ticket,
    KbArticle,
    Invoice,
    Payment,
    CreditNote,
    Quote,
    PurchaseOrder,
    Domain,
}

impl RecordType {
    /// Every variant, in the migration's order.
    pub const ALL: &'static [RecordType] = &[
        RecordType::Configuration,
        RecordType::Client,
        RecordType::Site,
        RecordType::Contact,
        RecordType::Ticket,
        RecordType::KbArticle,
        RecordType::Invoice,
        RecordType::Payment,
        RecordType::CreditNote,
        RecordType::Quote,
        RecordType::PurchaseOrder,
        RecordType::Domain,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            RecordType::Configuration => "configuration",
            RecordType::Client => "client",
            RecordType::Site => "site",
            RecordType::Contact => "contact",
            RecordType::Ticket => "ticket",
            RecordType::KbArticle => "kb_article",
            RecordType::Invoice => "invoice",
            RecordType::Payment => "payment",
            RecordType::CreditNote => "credit_note",
            RecordType::Quote => "quote",
            RecordType::PurchaseOrder => "purchase_order",
            RecordType::Domain => "domain",
        }
    }

    /// Parses a stored value. `None` is the typed refusal the API returns with
    /// the allowed set in the message — never a 500 and never a silent default.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == trimmed)
    }
}

/// A connection's health, as the roster and the chip both read it.
///
/// `Degraded` and `AuthFailed` are deliberately distinct: the first still
/// answers (slowly, or with partial data) so the action stays available with a
/// warning; the second cannot act until someone fixes it, so the action
/// degrades to `open` or to `Fix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    NeedsAttachment,
    AuthFailed,
    Degraded,
    Disabled,
}

impl ConnectionStatus {
    pub const ALL: &'static [ConnectionStatus] = &[
        ConnectionStatus::Connected,
        ConnectionStatus::NeedsAttachment,
        ConnectionStatus::AuthFailed,
        ConnectionStatus::Degraded,
        ConnectionStatus::Disabled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ConnectionStatus::Connected => "connected",
            ConnectionStatus::NeedsAttachment => "needs_attachment",
            ConnectionStatus::AuthFailed => "auth_failed",
            ConnectionStatus::Degraded => "degraded",
            ConnectionStatus::Disabled => "disabled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == trimmed)
    }

    /// Whether the connection may run an action without a warning. `Degraded`
    /// is usable — it has a last known good answer — and says so in the UI.
    pub const fn can_act(self) -> bool {
        matches!(
            self,
            ConnectionStatus::Connected | ConnectionStatus::Degraded
        )
    }

    /// Whether the roster's `Fix` action applies rather than `Manage`.
    pub const fn needs_repair(self) -> bool {
        matches!(
            self,
            ConnectionStatus::AuthFailed | ConnectionStatus::NeedsAttachment
        )
    }
}

/// One end of a `record_links` edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEndKind {
    Ticket,
    KbArticle,
    Configuration,
}

impl LinkEndKind {
    pub const ALL: &'static [LinkEndKind] = &[
        LinkEndKind::Ticket,
        LinkEndKind::KbArticle,
        LinkEndKind::Configuration,
    ];

    /// The kinds that may appear in `record_links.from_kind`.
    pub const FROM: &'static [LinkEndKind] = &[LinkEndKind::Ticket, LinkEndKind::KbArticle];

    /// The kinds that may appear in `record_links.to_kind`.
    pub const TO: &'static [LinkEndKind] = &[LinkEndKind::Configuration, LinkEndKind::KbArticle];

    pub const fn as_str(self) -> &'static str {
        match self {
            LinkEndKind::Ticket => "ticket",
            LinkEndKind::KbArticle => "kb_article",
            LinkEndKind::Configuration => "configuration",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == trimmed)
    }

    /// Whether an edge may point from `from` to `to`.
    ///
    /// The same statement as `record_links_pair_allowed` in migration 0185.
    /// Both exist because the router refuses early with a typed message and
    /// the database refuses regardless of which code path reached it.
    pub const fn pair_allowed(from: Self, to: Self) -> bool {
        matches!(
            (from, to),
            (LinkEndKind::Ticket, LinkEndKind::Configuration)
                | (LinkEndKind::Ticket, LinkEndKind::KbArticle)
                | (LinkEndKind::KbArticle, LinkEndKind::Configuration)
        )
    }
}

/// What an action does when a technician presses it.
///
/// Closed, and deliberately small. A connector declares *which* behaviour it
/// wants and never a new one: every behaviour has a permission check and an
/// audit shape in Core, so a new behaviour is an RFC, not a manifest field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionBehaviour {
    /// Build a URL from the connection and the external id. Spends no secret.
    Open,
    /// Start something: a deterministic URL template, or an adapter call that
    /// mints a one-time URL. Always audited.
    Launch,
    /// A `Launch` whose target is a phone number, dialled from the caller's own
    /// extension. Requires a `pbx_extension` identity for the caller.
    Dial,
    /// Post a Core-rendered card to a bound destination.
    Share,
    /// Move records in the background. Never a user press.
    Sync,
}

impl ActionBehaviour {
    pub const ALL: &'static [ActionBehaviour] = &[
        ActionBehaviour::Open,
        ActionBehaviour::Launch,
        ActionBehaviour::Dial,
        ActionBehaviour::Share,
        ActionBehaviour::Sync,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ActionBehaviour::Open => "open",
            ActionBehaviour::Launch => "launch",
            ActionBehaviour::Dial => "dial",
            ActionBehaviour::Share => "share",
            ActionBehaviour::Sync => "sync",
        }
    }

    /// Whether the behaviour is something a person presses. A chip row shows
    /// these and hides `Sync`, which runs on a schedule.
    pub const fn is_user_action(self) -> bool {
        !matches!(self, ActionBehaviour::Sync)
    }
}

/// A connector's product kind, as declared in its manifest.
///
/// Open on purpose: a vendor that fits no existing kind declares its own and
/// the UI renders it (see `docs/reference/connector-capabilities.md` §5). The
/// format is constrained so a kind is usable as a stable key — lowercase ASCII
/// snake case, 1–40 characters — and the known list below is documentation,
/// never an allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductKind(String);

impl ProductKind {
    /// The kinds the core connectors ship, and the ones the UI has a rendered
    /// label for. Others still work; they get the connector's own label.
    pub const KNOWN: &'static [&'static str] = &[
        "rmm",
        "remote_access",
        "telephony",
        "accounting",
        "dms",
        "dns",
        "identity",
        "network",
        "inventory",
        "notifications",
        "crm",
    ];

    pub const MAX_LEN: usize = 40;

    /// Validates a declared kind. A malformed kind is refused at manifest read
    /// time rather than becoming an unaddressable chip at render time.
    pub fn parse(raw: &str) -> Result<Self, ProductKindError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ProductKindError::Empty);
        }
        if trimmed.len() > Self::MAX_LEN {
            return Err(ProductKindError::TooLong {
                len: trimmed.len(),
                max: Self::MAX_LEN,
            });
        }
        let mut chars = trimmed.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        if !first_ok {
            return Err(ProductKindError::NotSnakeCase(trimmed.to_string()));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(ProductKindError::NotSnakeCase(trimmed.to_string()));
        }
        Ok(ProductKind(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the UI has a rendered label for this kind. Not a gate: an
    /// unknown kind renders with the connector's own label instead.
    pub fn is_known(&self) -> bool {
        Self::KNOWN.contains(&self.0.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductKindError {
    Empty,
    TooLong { len: usize, max: usize },
    NotSnakeCase(String),
}

impl std::fmt::Display for ProductKindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProductKindError::Empty => write!(f, "a product kind is required"),
            ProductKindError::TooLong { len, max } => {
                write!(f, "product kind is {len} characters (max {max})")
            }
            ProductKindError::NotSnakeCase(value) => write!(
                f,
                "product kind `{value}` must be lowercase ascii, digits and underscores, starting with a letter"
            ),
        }
    }
}

impl std::error::Error for ProductKindError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The migration that owns the storage half of this vocabulary. Included at
    /// compile time so a rename or deletion is a build failure, not a test that
    /// silently stops checking anything.
    const MIGRATION: &str =
        include_str!("../../core-db/migrations/0185_connector_connections_and_refs.sql");

    /// Reads the quoted literals out of one `CHECK (col IN ('a','b',…))`.
    ///
    /// Deliberately a small hand parser: a regex crate for one test is a
    /// dependency the workspace does not need, and this fails loudly when the
    /// SQL shape changes rather than quietly returning an empty list.
    fn check_list(marker: &str) -> Vec<String> {
        let start = MIGRATION
            .find(marker)
            .unwrap_or_else(|| panic!("migration has no `{marker}`"));
        let rest = &MIGRATION[start + marker.len()..];
        let end = rest
            .find("))")
            .unwrap_or_else(|| panic!("`{marker}` list is unterminated"));
        let body = &rest[..end];
        let mut out = Vec::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            if c != '\'' {
                continue;
            }
            let mut value = String::new();
            for inner in chars.by_ref() {
                if inner == '\'' {
                    break;
                }
                value.push(inner);
            }
            out.push(value);
        }
        out
    }

    #[test]
    fn sql_check_lists_match_the_vocabulary() {
        let sql_record_types = check_list("CHECK (record_type IN (");
        let ours: Vec<String> = RecordType::ALL
            .iter()
            .map(|r| r.as_str().to_string())
            .collect();
        assert_eq!(
            sql_record_types, ours,
            "external_refs.record_type and RecordType have drifted"
        );

        let sql_status = check_list("CHECK (status IN (");
        let ours: Vec<String> = ConnectionStatus::ALL
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_eq!(
            sql_status, ours,
            "connector_connections.status and ConnectionStatus have drifted"
        );

        let sql_from = check_list("CHECK (from_kind IN (");
        let ours: Vec<String> = LinkEndKind::FROM
            .iter()
            .map(|k| k.as_str().to_string())
            .collect();
        assert_eq!(sql_from, ours, "record_links.from_kind has drifted");

        let sql_to = check_list("CHECK (to_kind IN (");
        let ours: Vec<String> = LinkEndKind::TO
            .iter()
            .map(|k| k.as_str().to_string())
            .collect();
        assert_eq!(sql_to, ours, "record_links.to_kind has drifted");

        let sql_binding = check_list("CHECK (binding_kind IN (");
        assert_eq!(
            sql_binding,
            vec![
                "slack_channel".to_string(),
                "teams_conversation".to_string(),
                "rewst_workflow".to_string(),
                "voip_route".to_string(),
            ],
            "connector_bindings.binding_kind changed without a vocabulary update"
        );

        let sql_identity = check_list("CHECK (identity_kind IN (");
        assert_eq!(
            sql_identity,
            vec![
                "slack_user".to_string(),
                "teams_user".to_string(),
                "pbx_extension".to_string(),
                "rmm_technician".to_string(),
            ],
            "user_external_identities.identity_kind changed without a vocabulary update"
        );
    }

    #[test]
    fn every_linked_record_type_is_an_owned_table_with_a_prune_trigger() {
        for kind in LinkEndKind::ALL {
            let needle = format!("EXECUTE FUNCTION record_links_prune('{}')", kind.as_str());
            assert!(
                MIGRATION.contains(&needle),
                "no prune trigger for `{}` — deleting that record would orphan its links",
                kind.as_str()
            );
        }
    }

    #[test]
    fn record_type_round_trips_and_refuses_the_unknown() {
        for kind in RecordType::ALL {
            assert_eq!(RecordType::parse(kind.as_str()), Some(*kind));
        }
        assert_eq!(RecordType::parse("vehicles"), None);
        assert_eq!(RecordType::parse(""), None);
        // Stored values are lowercase; surrounding whitespace is tolerated.
        assert_eq!(
            RecordType::parse(" configuration "),
            Some(RecordType::Configuration)
        );
        assert_eq!(RecordType::parse("Configuration"), None);
    }

    #[test]
    fn link_pairs_match_the_database_constraint() {
        for from in LinkEndKind::ALL {
            for to in LinkEndKind::ALL {
                let allowed = LinkEndKind::pair_allowed(*from, *to);
                let expected = match (from, to) {
                    (LinkEndKind::Ticket, LinkEndKind::Configuration) => true,
                    (LinkEndKind::Ticket, LinkEndKind::KbArticle) => true,
                    (LinkEndKind::KbArticle, LinkEndKind::Configuration) => true,
                    _ => false,
                };
                assert_eq!(
                    allowed,
                    expected,
                    "pair {}→{} disagrees with record_links_pair_allowed",
                    from.as_str(),
                    to.as_str()
                );
            }
        }
        // The case two independent CHECKs would have allowed and no surface draws.
        assert!(!LinkEndKind::pair_allowed(
            LinkEndKind::KbArticle,
            LinkEndKind::KbArticle
        ));
        assert!(MIGRATION.contains("record_links_pair_allowed"));
    }

    #[test]
    fn connection_status_answers_the_two_questions_the_roster_asks() {
        assert!(ConnectionStatus::Connected.can_act());
        assert!(ConnectionStatus::Degraded.can_act());
        assert!(!ConnectionStatus::AuthFailed.can_act());
        assert!(!ConnectionStatus::NeedsAttachment.can_act());
        assert!(!ConnectionStatus::Disabled.can_act());

        assert!(ConnectionStatus::AuthFailed.needs_repair());
        assert!(ConnectionStatus::NeedsAttachment.needs_repair());
        assert!(!ConnectionStatus::Degraded.needs_repair());
        assert!(!ConnectionStatus::Connected.needs_repair());
    }

    #[test]
    fn product_kinds_are_open_but_well_formed() {
        assert_eq!(ProductKind::parse("rmm").unwrap().as_str(), "rmm");
        assert!(ProductKind::parse("rmm").unwrap().is_known());

        // The whole point: a kind nobody planned still parses and renders.
        let invented = ProductKind::parse("inventory_control").unwrap();
        assert_eq!(invented.as_str(), "inventory_control");
        assert!(!invented.is_known());

        assert!(ProductKind::parse("").is_err());
        assert!(ProductKind::parse("RMM").is_err());
        assert!(ProductKind::parse("remote-access").is_err());
        assert!(ProductKind::parse("_leading").is_err());
        assert!(ProductKind::parse(&"a".repeat(41)).is_err());
    }
}
