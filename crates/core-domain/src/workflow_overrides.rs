//! Per-client workflow overrides (Wave 12 lane 5).
//!
//! Purpose: declare — EXACTLY ONCE — the closed set of org workflow defaults a
//! CLIENT may override, the typed value each override accepts, and the pure
//! precedence rule that turns an org default plus an optional client override
//! into one effective value. This is the "typed registry" the persistence,
//! HTTP, and manifest layers validate against, so a stored override is always
//! `customizations-as-data`: a known key with a validated, typed payload
//! (upgrade-compatibility-policy §4.5/§5), never a free-text bucket.
//!
//! ## Resolution precedence (documented, binding)
//!
//! For every construct, independently:
//!
//! 1. **Client override, `set`** — replaces the org default; the client value
//!    wins.
//! 2. **Client override, `disabled`** — an explicit deny/opt-out. It wins even
//!    when the org default is enabled (deny beats enable). Valid only where the
//!    declaration allows it ([`OverrideDeclaration::allow_disable`]); a
//!    reference construct that cannot be denied is cleared (row removed) to
//!    inherit instead.
//! 3. **No client override** — the org default applies.
//! 4. **Neither present** — the construct is `unset`.
//!
//! A row that is absent is distinct from a row that is `disabled`: absence
//! inherits, `disabled` denies. That distinction is the whole point of the
//! explicit-disable doctrine.
//!
//! ## Semantic-category binding
//!
//! Reference values are ids, never labels. `workflow.intake_status` resolves to
//! a board status whose [`crate::status::TicketStatusCategory`] travels with the
//! resolved view, so downstream behavior keys on the semantic category and a
//! status rename can never change behavior (defaults-first doctrine, policy §5).
//!
//! Non-purpose: persistence (core-db stores validated rows), HTTP shape
//! (core-api projects the resolved document), and *who* may write
//! (authorization lives in the policy module).

use crate::status::TicketStatusCategory;
use serde::{Deserialize, Serialize};

/// `workflow.intake_board` — the board new tickets for this client open on.
pub const WORKFLOW_INTAKE_BOARD: &str = "workflow.intake_board";
/// `workflow.intake_status` — the entry status new tickets open in; bound to a
/// semantic status category.
pub const WORKFLOW_INTAKE_STATUS: &str = "workflow.intake_status";
/// `sla.policy` — the SLA policy applied to this client's tickets.
pub const SLA_POLICY: &str = "sla.policy";
/// `notifications.enabled` — whether notifications fire for this client.
pub const NOTIFICATIONS_ENABLED: &str = "notifications.enabled";
/// `automations.enabled` — whether org automation rules run for this client.
pub const AUTOMATIONS_ENABLED: &str = "automations.enabled";

/// The value shapes a client override accepts. The wire value is the plain
/// JSON payload (uuid string / boolean); the kind exists so validation can
/// reject a mistyped payload before persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OverrideValueKind {
    /// A board id in the caller's org.
    BoardRef,
    /// A board-status id in the caller's org, belonging to the resolved intake
    /// board and admissible at creation (`is_entry`).
    StatusRef,
    /// An SLA policy id in the caller's org.
    SlaPolicyRef,
    /// A boolean toggle.
    Toggle,
}

impl OverrideValueKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BoardRef => "board_ref",
            Self::StatusRef => "status_ref",
            Self::SlaPolicyRef => "sla_policy_ref",
            Self::Toggle => "toggle",
        }
    }
}

/// One construct a client may override. `key` is immutable once shipped — it
/// is the storage key, the audit subject, and the manifest field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideDeclaration {
    pub key: &'static str,
    pub value_kind: OverrideValueKind,
    /// Whether an explicit `disabled` deny is a legal state for this
    /// construct. Reference constructs that must always resolve somewhere
    /// (intake board/status) are clear-to-inherit only.
    pub allow_disable: bool,
    /// Org-scope settings-registry key carrying the org default for a toggle
    /// construct; `None` for reference constructs, whose org default is the
    /// org's default board / entry status / SLA policy.
    pub org_default_setting: Option<&'static str>,
    pub label: &'static str,
    pub description: &'static str,
}

/// A parsed, typed override value. Persisted as its plain JSON projection
/// ([`OverrideValue::to_json`]); parsed back through the declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverrideValue {
    Board(uuid::Uuid),
    Status(uuid::Uuid),
    SlaPolicy(uuid::Uuid),
    Toggle(bool),
}

impl OverrideValue {
    pub fn kind(self) -> OverrideValueKind {
        match self {
            Self::Board(_) => OverrideValueKind::BoardRef,
            Self::Status(_) => OverrideValueKind::StatusRef,
            Self::SlaPolicy(_) => OverrideValueKind::SlaPolicyRef,
            Self::Toggle(_) => OverrideValueKind::Toggle,
        }
    }

    /// The storage/API projection: a bare uuid string or boolean.
    pub fn to_json(self) -> serde_json::Value {
        match self {
            Self::Board(id) | Self::Status(id) | Self::SlaPolicy(id) => {
                serde_json::Value::String(id.to_string())
            }
            Self::Toggle(b) => serde_json::Value::Bool(b),
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        match self {
            Self::Toggle(b) => Some(b),
            _ => None,
        }
    }
}

impl OverrideDeclaration {
    /// Parses + validates a candidate payload against this declaration's value
    /// kind. Returns a human-readable reason on mismatch (surfaced as a typed
    /// 422).
    pub fn parse_value(&self, raw: &serde_json::Value) -> Result<OverrideValue, String> {
        match (self.value_kind, raw) {
            (OverrideValueKind::BoardRef, serde_json::Value::String(s)) => {
                parse_uuid(s).map(OverrideValue::Board)
            }
            (OverrideValueKind::StatusRef, serde_json::Value::String(s)) => {
                parse_uuid(s).map(OverrideValue::Status)
            }
            (OverrideValueKind::SlaPolicyRef, serde_json::Value::String(s)) => {
                parse_uuid(s).map(OverrideValue::SlaPolicy)
            }
            (OverrideValueKind::Toggle, serde_json::Value::Bool(b)) => {
                Ok(OverrideValue::Toggle(*b))
            }
            (kind, other) => Err(format!(
                "{} expects a {} value, got {other}",
                self.key,
                kind.as_str()
            )),
        }
    }
}

fn parse_uuid(s: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(s).map_err(|_| format!("'{s}' is not a valid id"))
}

/// A stored client-override row, parsed against its declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverrideState {
    /// A concrete override value is in force.
    Set(OverrideValue),
    /// Explicit deny/opt-out.
    Disabled,
}

/// The effective state of one construct after resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Resolution {
    /// A concrete effective value is in force.
    Set {
        #[cfg_attr(feature = "openapi", schema(value_type = Object))]
        value: serde_json::Value,
    },
    /// An explicit deny/opt-out is in force.
    Disabled,
    /// Neither an org default nor a client override exists.
    Unset,
}

/// Which layer supplied the effective value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResolutionSource {
    ClientOverride,
    OrgDefault,
    Unset,
}

/// The pure precedence rule (see module docs). Kept free of I/O so the
/// precedence contract is unit-tested directly.
pub fn resolve(
    client_override: Option<OverrideState>,
    org_default: Option<OverrideValue>,
) -> (Resolution, ResolutionSource) {
    match client_override {
        Some(OverrideState::Set(value)) => (
            Resolution::Set {
                value: value.to_json(),
            },
            ResolutionSource::ClientOverride,
        ),
        Some(OverrideState::Disabled) => (Resolution::Disabled, ResolutionSource::ClientOverride),
        None => match org_default {
            Some(value) => (
                Resolution::Set {
                    value: value.to_json(),
                },
                ResolutionSource::OrgDefault,
            ),
            None => (Resolution::Unset, ResolutionSource::Unset),
        },
    }
}

/// THE registry. Every client-override construct appears exactly once here.
pub static OVERRIDES: &[OverrideDeclaration] = &[
    OverrideDeclaration {
        key: WORKFLOW_INTAKE_BOARD,
        value_kind: OverrideValueKind::BoardRef,
        allow_disable: false,
        org_default_setting: None,
        label: "Intake board",
        description: "Board new tickets for this client open on. Overrides the org's \
                      default board; clear to inherit.",
    },
    OverrideDeclaration {
        key: WORKFLOW_INTAKE_STATUS,
        value_kind: OverrideValueKind::StatusRef,
        allow_disable: false,
        org_default_setting: None,
        label: "Intake status",
        description: "Entry status new tickets open in on the resolved intake board. \
                      Behavior keys on the status's semantic category, not its name; \
                      clear to inherit the board's default entry status.",
    },
    OverrideDeclaration {
        key: SLA_POLICY,
        value_kind: OverrideValueKind::SlaPolicyRef,
        allow_disable: true,
        org_default_setting: None,
        label: "SLA policy",
        description: "SLA policy applied to this client's tickets. Overrides the org's \
                      default policy; explicitly disable to opt the client out of SLA.",
    },
    OverrideDeclaration {
        key: NOTIFICATIONS_ENABLED,
        value_kind: OverrideValueKind::Toggle,
        allow_disable: true,
        org_default_setting: Some(crate::settings::NOTIFICATIONS_CLIENT_EVENTS),
        label: "Notifications",
        description: "Whether notifications fire for this client. Overrides the org \
                      default; explicitly disable to silence.",
    },
    OverrideDeclaration {
        key: AUTOMATIONS_ENABLED,
        value_kind: OverrideValueKind::Toggle,
        allow_disable: true,
        org_default_setting: Some(crate::settings::AUTOMATIONS_CLIENT_RULES),
        label: "Automations",
        description: "Whether org automation rules run for this client's tickets. \
                      Overrides the org default; explicitly disable to exempt the client.",
    },
];

/// All declarations, registry order preserved.
pub fn declarations() -> &'static [OverrideDeclaration] {
    OVERRIDES
}

/// Looks up a declaration by key.
pub fn declaration(key: &str) -> Option<&'static OverrideDeclaration> {
    OVERRIDES.iter().find(|d| d.key == key)
}

/// The whole registry key set, for validation.
pub fn is_registered_key(key: &str) -> bool {
    declaration(key).is_some()
}

/// Parses a stored client row's `state`/`value` pair against the declaration.
/// `Disabled` must carry no value; `Set` must parse.
pub fn parse_state(
    decl: &OverrideDeclaration,
    state: &str,
    value: Option<&serde_json::Value>,
) -> Result<OverrideState, String> {
    match state {
        "disabled" => {
            if value.is_some() {
                return Err(format!(
                    "{}: a disabled override carries no value",
                    decl.key
                ));
            }
            if !decl.allow_disable {
                return Err(format!("{}: this override cannot be disabled", decl.key));
            }
            Ok(OverrideState::Disabled)
        }
        "set" => {
            let raw =
                value.ok_or_else(|| format!("{}: a set override requires a value", decl.key))?;
            decl.parse_value(raw).map(OverrideState::Set)
        }
        other => Err(format!(
            "{}: unknown override state '{other}' (expected set|disabled)",
            decl.key
        )),
    }
}

/// Convenience: the semantic category a status override binds to, when known.
/// Exposed so the HTTP layer can document the category contract without
/// re-deriving it.
pub fn status_category_label(category: TicketStatusCategory) -> &'static str {
    category.as_str()
}

// --- resolved view types (serialized by the API layer) ------------------------

/// Render-ready details of a resolved reference value: names and the semantic
/// category travel WITH the resolution so consumers never branch on labels and
/// never need a second lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedDetail {
    Board {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        board_id: uuid::Uuid,
        name: String,
    },
    Status {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        status_id: uuid::Uuid,
        name: String,
        /// The semantic category the behavior layer keys on.
        category: TicketStatusCategory,
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        board_id: uuid::Uuid,
    },
    SlaPolicy {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        policy_id: uuid::Uuid,
        name: String,
    },
    Toggle {
        enabled: bool,
    },
}

/// The stored client row, as provenance on a resolved entry (who last set it
/// and when — the audit subject; the append-only trail keeps the full history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientOverrideRow {
    /// "set" | "disabled".
    pub state: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Object, nullable = true))]
    pub value: Option<serde_json::Value>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>, nullable = true))]
    pub changed_by: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// One resolved override entry: the declaration it validated against, the
/// resolution outcome, which layer supplied it, the raw org default and client
/// row, the renderable detail, and any resolution issues (e.g. a stale
/// cross-board status override that fell back to the board's entry status).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OverrideEntry {
    pub key: String,
    pub label: String,
    pub description: String,
    pub value_kind: OverrideValueKind,
    pub allow_disable: bool,
    pub resolution: Resolution,
    pub source: ResolutionSource,
    /// The org default this resolution inherited from, when one existed.
    #[cfg_attr(feature = "openapi", schema(value_type = Object, nullable = true))]
    pub org_default: Option<serde_json::Value>,
    pub client_override: Option<ClientOverrideRow>,
    pub detail: Option<ResolvedDetail>,
    /// Typed drift/validation notes (never empty on a fallback).
    pub issues: Vec<String>,
}

/// The client's full resolved workflow: one entry per registry declaration,
/// registry order preserved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientWorkflowResolution {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: uuid::Uuid,
    pub entries: Vec<OverrideEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_unique_well_formed_and_carry_kinds() {
        let mut seen = std::collections::HashSet::new();
        for d in declarations() {
            assert!(!d.key.is_empty(), "empty key");
            assert!(
                d.key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "key {} must be lowercase dotted snake_case",
                d.key
            );
            assert!(seen.insert(d.key), "duplicate key {}", d.key);
            assert!(!d.label.is_empty());
            assert!(!d.description.is_empty());
            if !d.allow_disable {
                assert!(
                    d.org_default_setting.is_none(),
                    "{}: clear-only reference constructs have no toggle default",
                    d.key
                );
            }
            if d.value_kind == OverrideValueKind::Toggle {
                assert!(
                    d.allow_disable,
                    "{}: toggles must support explicit disable",
                    d.key
                );
                assert!(
                    d.org_default_setting.is_some(),
                    "{}: toggle needs an org default setting",
                    d.key
                );
            } else {
                assert!(
                    d.org_default_setting.is_none(),
                    "{}: reference constructs resolve from org constructs",
                    d.key
                );
            }
        }
        assert_eq!(declarations().len(), 5);
        assert!(is_registered_key(WORKFLOW_INTAKE_BOARD));
        assert!(!is_registered_key("nope.nope"));
    }

    #[test]
    fn typed_parsing_accepts_only_the_declared_shape() {
        let board = declaration(WORKFLOW_INTAKE_BOARD).unwrap();
        let id = uuid::Uuid::now_v7();
        let parsed = board
            .parse_value(&serde_json::json!(id))
            .expect("board id parses");
        assert_eq!(parsed, OverrideValue::Board(id));
        assert!(board.parse_value(&serde_json::json!(true)).is_err());
        assert!(board.parse_value(&serde_json::json!("not-a-uuid")).is_err());

        let toggle = declaration(NOTIFICATIONS_ENABLED).unwrap();
        assert_eq!(
            toggle.parse_value(&serde_json::json!(false)).unwrap(),
            OverrideValue::Toggle(false)
        );
        assert!(toggle.parse_value(&serde_json::json!("false")).is_err());
    }

    #[test]
    fn disable_is_only_legal_where_declared_and_carries_no_value() {
        let board = declaration(WORKFLOW_INTAKE_BOARD).unwrap();
        assert!(parse_state(board, "disabled", None).is_err());
        let sla = declaration(SLA_POLICY).unwrap();
        assert_eq!(
            parse_state(sla, "disabled", None).unwrap(),
            OverrideState::Disabled
        );
        assert!(parse_state(sla, "disabled", Some(&serde_json::json!("x"))).is_err());
        assert!(parse_state(sla, "bogus", None).is_err());
        assert!(parse_state(sla, "set", None).is_err());
    }

    #[test]
    fn precedence_client_set_wins_over_org_default() {
        let ov = OverrideValue::Board(uuid::Uuid::from_u128(9));
        let org = OverrideValue::Board(uuid::Uuid::from_u128(1));
        let (res, src) = resolve(Some(OverrideState::Set(ov)), Some(org));
        assert_eq!(src, ResolutionSource::ClientOverride);
        assert_eq!(
            res,
            Resolution::Set {
                value: ov.to_json()
            }
        );
    }

    #[test]
    fn precedence_explicit_disable_denies_an_org_enabled_default() {
        let (res, src) = resolve(
            Some(OverrideState::Disabled),
            Some(OverrideValue::Toggle(true)),
        );
        assert_eq!(res, Resolution::Disabled);
        assert_eq!(src, ResolutionSource::ClientOverride);
    }

    #[test]
    fn precedence_absence_inherits_org_default_or_unset() {
        let (res, src) = resolve(None, Some(OverrideValue::Toggle(true)));
        assert_eq!(
            res,
            Resolution::Set {
                value: serde_json::json!(true)
            }
        );
        assert_eq!(src, ResolutionSource::OrgDefault);

        let (res, src) = resolve(None, None);
        assert_eq!(res, Resolution::Unset);
        assert_eq!(src, ResolutionSource::Unset);
    }

    #[test]
    fn disabled_row_is_not_the_same_as_absent() {
        // The doctrine distinction: absent inherits, disabled denies.
        let (absent, _) = resolve(None, Some(OverrideValue::Toggle(true)));
        let (denied, _) = resolve(
            Some(OverrideState::Disabled),
            Some(OverrideValue::Toggle(true)),
        );
        assert_ne!(absent, denied);
        assert_eq!(denied, Resolution::Disabled);
    }
}
