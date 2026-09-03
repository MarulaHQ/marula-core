//! Typed settings registry (docs/10-agent-platform.md §2).
//!
//! Purpose: every user-visible setting is declared EXACTLY ONCE here as
//! `key / kind / default / scope / validation`. The UI renders settings
//! pages FROM this registry and all code reads settings THROUGH it — no
//! ad-hoc env/config reads for behavior a user can change. Defaults-first:
//! an org that touches nothing gets every default below.
//!
//! Non-purpose: persistence (core-db stores opaque validated JSONB values),
//! HTTP surface shape (core-api projects declarations into DTOs), and any
//! notion of *who* may write — authorization lives in the policy module.

use serde::{Deserialize, Serialize};

/// Whom a setting belongs to. Org-scope values are shared by the whole
/// tenant and edited by admins; user-scope values are private per account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SettingScope {
    Org,
    User,
}

impl SettingScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingScope::Org => "org",
            SettingScope::User => "user",
        }
    }
}

/// The value shapes the registry understands. A stored value must satisfy
/// its declaration's kind or the write is rejected (validation is applied
/// before persistence, never after).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SettingKind {
    Boolean,
    /// Inclusive integer range.
    Integer {
        min: i64,
        max: i64,
    },
    /// Free text with a hard length cap.
    Text {
        max_len: usize,
    },
    /// Closed choice set; the wire value is the option string itself.
    Enumeration,
}

/// A concrete setting value. Only three payload types exist by design —
/// richer structures are entities, not settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Text(String),
}

impl SettingValue {
    /// Validates this value against a declared kind. Returns a human-readable
    /// reason on mismatch — surfaced verbatim as HTTP 422 detail.
    pub fn validate(&self, kind: &SettingKind) -> Result<(), String> {
        match (self, kind) {
            (SettingValue::Boolean(_), SettingKind::Boolean) => Ok(()),
            (SettingValue::Integer(v), SettingKind::Integer { min, max }) => {
                if *v >= *min && *v <= *max {
                    Ok(())
                } else {
                    Err(format!("integer out of range [{min}, {max}]: {v}"))
                }
            }
            (SettingValue::Text(s), SettingKind::Text { max_len }) => {
                if s.chars().count() <= *max_len {
                    Ok(())
                } else {
                    Err(format!("text longer than {max_len} characters"))
                }
            }
            // Enumerations are validated by `SettingDeclaration::validate`,
            // which owns the option list; a bare value cannot prove
            // membership and is rejected here.
            (other, _) => Err(format!(
                "value {:?} does not match expected kind {}",
                other,
                kind.label()
            )),
        }
    }

    /// Parses an untyped JSON payload (as received over HTTP) into a value.
    /// Numbers must be integral; anything else is rejected before validation
    /// against the kind.
    pub fn from_json(raw: &serde_json::Value) -> Result<Self, String> {
        match raw {
            serde_json::Value::Bool(b) => Ok(SettingValue::Boolean(*b)),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(SettingValue::Integer)
                .ok_or_else(|| "number must be an integer".to_string()),
            serde_json::Value::String(s) => Ok(SettingValue::Text(s.clone())),
            other => Err(format!("unsupported JSON value: {other}")),
        }
    }

    /// Renders as JSON for storage and API responses.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl SettingKind {
    /// Human label used in validation messages and generated UI.
    pub fn label(&self) -> &'static str {
        match self {
            SettingKind::Boolean => "boolean",
            SettingKind::Integer { .. } => "integer",
            SettingKind::Text { .. } => "text",
            SettingKind::Enumeration => "enumeration",
        }
    }
}

/// Where a declaration shows up in the settings hub IA. Sections whose
/// backends land in later waves still carry their future keys today so the
/// registry stays the single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SettingSection {
    General,
    Appearance,
    Onboarding,
    Integrations,
}

impl SettingSection {
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingSection::General => "general",
            SettingSection::Appearance => "appearance",
            SettingSection::Onboarding => "onboarding",
            SettingSection::Integrations => "integrations",
        }
    }
}

/// Whether a declaration renders as an editable row in the settings hub.
/// `Hidden` settings are real, validated, readable through the registry —
/// they are state flags rather than preferences (e.g. wizard completion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SettingVisibility {
    Ui,
    Hidden,
}

/// One setting, declared once. This struct IS the contract between the
/// backend consumers, the REST surface, and the generated settings UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SettingDeclaration {
    /// Dotted key, namespaced by area (`general.eod_hour`). Immutable once
    /// shipped — it is the storage key and audit subject.
    pub key: &'static str,
    pub kind: SettingKind,
    pub default: SettingValue,
    pub scope: SettingScope,
    pub section: SettingSection,
    pub visibility: SettingVisibility,
    /// Short human label rendered by the generated UI.
    pub label: &'static str,
    /// One-or-two sentence explanation of what the setting governs and who
    /// consumes it downstream.
    pub description: &'static str,
}

impl SettingDeclaration {
    /// Validates a candidate value against this declaration's kind, plus
    /// enumeration membership (options live on the declaration).
    pub fn validate(&self, value: &SettingValue) -> Result<(), String> {
        if let SettingKind::Enumeration = self.kind {
            let text = match value {
                SettingValue::Text(s) => s.as_str(),
                other => {
                    return Err(format!(
                        "enumeration requires a string, got {}",
                        other.kind_name()
                    ))
                }
            };
            let options = self.options().unwrap_or(&[]);
            if !options.contains(&text) {
                return Err(format!("'{text}' is not one of: {}", options.join(", ")));
            }
            return Ok(());
        }
        value.validate(&self.kind)
    }

    /// Option list for enumerations; `None` for non-enumeration kinds.
    pub fn options(&self) -> Option<&'static [&'static str]> {
        match &self.kind {
            SettingKind::Enumeration => Some(match self.key {
                GENERAL_DEFAULT_TIMEZONE => TIMEZONE_OPTIONS,
                GENERAL_WEEK_START => WEEK_START_OPTIONS,
                APPEARANCE_THEME => THEME_OPTIONS,
                AI_COMPLETION_PROVIDER => AI_PROVIDER_OPTIONS,
                _ => EMPTY_OPTIONS,
            }),
            _ => None,
        }
    }
}

impl SettingValue {
    fn kind_name(&self) -> &'static str {
        match self {
            SettingValue::Boolean(_) => "boolean",
            SettingValue::Integer(_) => "integer",
            SettingValue::Text(_) => "string",
        }
    }
}

// --- canonical option sets ---------------------------------------------------

pub const GENERAL_DEFAULT_TIMEZONE: &str = "general.default_timezone";
pub const GENERAL_WEEK_START: &str = "general.week_start";
pub const GENERAL_EOD_HOUR: &str = "general.eod_hour";
pub const APPEARANCE_THEME: &str = "appearance.theme";
pub const ONBOARDING_WIZARD_COMPLETED: &str = "onboarding.wizard_completed";
pub const AI_COMPLETION_PROVIDER: &str = "ai.completion_provider";
pub const AI_COMPLETION_MODEL: &str = "ai.completion_model";

pub const TIMEZONE_OPTIONS: &[&str] = &[
    "UTC",
    "America/Los_Angeles",
    "America/Denver",
    "America/Chicago",
    "America/New_York",
    "America/Sao_Paulo",
    "Europe/London",
    "Europe/Berlin",
    "Europe/Paris",
    "Europe/Warsaw",
    "Europe/Athens",
    "Africa/Cairo",
    "Africa/Johannesburg",
    "Asia/Dubai",
    "Asia/Karachi",
    "Asia/Kolkata",
    "Asia/Bangkok",
    "Asia/Singapore",
    "Asia/Shanghai",
    "Asia/Tokyo",
    "Australia/Perth",
    "Australia/Sydney",
    "Pacific/Auckland",
];

pub const WEEK_START_OPTIONS: &[&str] = &["monday", "sunday"];

pub const THEME_OPTIONS: &[&str] = &["light", "dark"];

pub const AI_PROVIDER_OPTIONS: &[&str] = &["none", "openai", "anthropic", "ollama", "custom"];

/// Provider identifiers the generic sealed-key store (BYOK) accepts for
/// accounting/export-sync connectors (Wave 9 adapters lane). Their OAuth
/// access/refresh tokens ride the same per-org sealed storage as AI keys —
/// one store, one audit trail, no plaintext anywhere.
pub const CONNECTOR_PROVIDER_OPTIONS: &[&str] = &["qbo", "xero"];

/// Every provider id acceptable to the BYOK seal/unseal surface.
pub fn byok_provider_options() -> impl Iterator<Item = &'static str> {
    AI_PROVIDER_OPTIONS
        .iter()
        .copied()
        .chain(CONNECTOR_PROVIDER_OPTIONS.iter().copied())
}

const EMPTY_OPTIONS: &[&str] = &[];

// --- the registry ------------------------------------------------------------

/// THE registry. Every user-visible setting appears exactly once here;
/// append new declarations at the end and add a uniqueness test.
/// `LazyLock` because defaults carry allocated strings (non-const).
pub static REGISTRY: std::sync::LazyLock<Vec<SettingDeclaration>> =
    std::sync::LazyLock::new(|| {
        vec![
            SettingDeclaration {
                key: GENERAL_EOD_HOUR,
                kind: SettingKind::Integer { min: 0, max: 23 },
                default: SettingValue::Integer(18),
                scope: SettingScope::Org,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "End-of-day hour",
                description: "Hour of day (0–23) after which unfinished billable work is \
                             flagged. Consumed by missing-time detection and EOD nudges.",
            },
            SettingDeclaration {
                key: GENERAL_DEFAULT_TIMEZONE,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("UTC".into()),
                scope: SettingScope::Org,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "Default timezone",
                description: "Baseline IANA timezone for business-hours calendars and SLA \
                             clocks; individual users may override in their own preferences \
                             later.",
            },
            SettingDeclaration {
                key: GENERAL_WEEK_START,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("monday".into()),
                scope: SettingScope::Org,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "First day of week",
                description: "Anchor day for weekly calendars, capacity views, and reports.",
            },
            SettingDeclaration {
                key: APPEARANCE_THEME,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("light".into()),
                scope: SettingScope::User,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Theme",
                description: "Workbench appearance for this account. Until a choice is saved \
                             the app follows the system preference.",
            },
            SettingDeclaration {
                key: ONBOARDING_WIZARD_COMPLETED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Hidden,
                label: "First-run wizard completed",
                description: "Set when the Tier-1 org wizard is finished or skipped. Skipping \
                             changes nothing else — defaults already operate the org.",
            },
            SettingDeclaration {
                key: AI_COMPLETION_PROVIDER,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("none".into()),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "AI completion provider",
                description: "Active LLM completion provider for BYOK intelligence. \
                             'none' disables all outgoing model calls (none mode).",
            },
            SettingDeclaration {
                key: AI_COMPLETION_MODEL,
                kind: SettingKind::Text { max_len: 128 },
                default: SettingValue::Text("none".into()),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "AI completion model",
                description: "Model identifier to invoke on the active completion provider.",
            },
        ]
    });

/// All declarations, registry order preserved.
pub fn declarations() -> &'static [SettingDeclaration] {
    &REGISTRY
}

/// All declarations for one scope, registry order preserved.
pub fn declarations_for(scope: SettingScope) -> Vec<&'static SettingDeclaration> {
    REGISTRY.iter().filter(|d| d.scope == scope).collect()
}

/// Looks up a declaration by key.
pub fn declaration(key: &str) -> Option<&'static SettingDeclaration> {
    REGISTRY.iter().find(|d| d.key == key)
}

/// The default value for a key (defaults-first doctrine: resolution starts
/// here, then overlays stored overrides).
pub fn default_value(key: &str) -> Option<SettingValue> {
    declaration(key).map(|d| d.default.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_are_unique_and_well_formed() {
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
            // Every default must satisfy its own declaration.
            d.validate(&d.default)
                .unwrap_or_else(|e| panic!("default for {} invalid: {e}", d.key));
        }
    }

    #[test]
    fn lookups_resolve() {
        assert!(declaration(GENERAL_EOD_HOUR).is_some());
        assert!(declaration("nope.nope").is_none());
        assert_eq!(
            default_value(APPEARANCE_THEME),
            Some(SettingValue::Text("light".into()))
        );
        assert_eq!(declarations_for(SettingScope::User).len(), 1);
        assert_eq!(declarations_for(SettingScope::Org).len(), 6);
    }

    #[test]
    fn integer_bounds_enforced() {
        let d = declaration(GENERAL_EOD_HOUR).unwrap();
        assert!(d.validate(&SettingValue::Integer(0)).is_ok());
        assert!(d.validate(&SettingValue::Integer(23)).is_ok());
        assert!(d.validate(&SettingValue::Integer(24)).is_err());
        assert!(d.validate(&SettingValue::Integer(-1)).is_err());
        assert!(d.validate(&SettingValue::Text("18".into())).is_err());
    }

    #[test]
    fn enumeration_membership_enforced() {
        let d = declaration(GENERAL_WEEK_START).unwrap();
        assert!(d.validate(&SettingValue::Text("monday".into())).is_ok());
        assert!(d.validate(&SettingValue::Text("sunday".into())).is_ok());
        assert!(d.validate(&SettingValue::Text("tuesday".into())).is_err());
        assert!(d.validate(&SettingValue::Integer(1)).is_err());
    }

    #[test]
    fn json_round_trip_and_rejection() {
        assert_eq!(
            SettingValue::from_json(&serde_json::json!(18)).unwrap(),
            SettingValue::Integer(18)
        );
        assert_eq!(
            SettingValue::from_json(&serde_json::json!("UTC")).unwrap(),
            SettingValue::Text("UTC".into())
        );
        assert!(SettingValue::from_json(&serde_json::json!(1.5)).is_err());
        assert!(SettingValue::from_json(&serde_json::json!({"a": 1})).is_err());
        assert_eq!(
            SettingValue::Boolean(true).to_json(),
            serde_json::json!(true)
        );
    }

    #[test]
    fn hidden_settings_exist_for_state_flags() {
        let d = declaration(ONBOARDING_WIZARD_COMPLETED).unwrap();
        assert_eq!(d.visibility, SettingVisibility::Hidden);
        assert_eq!(d.scope, SettingScope::Org);
        assert!(d.validate(&SettingValue::Boolean(true)).is_ok());
    }
}
