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
    Email,
}

impl SettingSection {
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingSection::General => "general",
            SettingSection::Appearance => "appearance",
            SettingSection::Onboarding => "onboarding",
            SettingSection::Integrations => "integrations",
            SettingSection::Email => "email",
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
            if self.key == APPEARANCE_THEME || self.key == APPEARANCE_THEME_OVERRIDE {
                // The org theme AND the per-user override may name an
                // installed custom theme pack (`pack:<slug>`) — a dynamic
                // id validated at install/uninstall time by the theme-pack
                // seam; the bare shape (not free text) is checked here.
                if text.starts_with("pack:")
                    && crate::theme_pack::is_preset_or_pack_slug(&text[5..])
                {
                    return Ok(());
                }
            }
            let options: &[&str] = self.options().unwrap_or(&[]);
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
                APPEARANCE_THEME => THEME_PRESET_OPTIONS,
                APPEARANCE_THEME_OVERRIDE => THEME_OVERRIDE_OPTIONS,
                APPEARANCE_MODE => MODE_OPTIONS,
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
/// Per-user override over the org theme (Wave 24). 'org_default' sentinel.
pub const APPEARANCE_THEME_OVERRIDE: &str = "appearance.theme_override";
/// Per-user light/dark mode. Supersedes the old flat user-scope
/// `appearance.theme` key via migration 0097 (U6-safe supersede).
pub const APPEARANCE_MODE: &str = "appearance.mode";
/// The org-scope setting key pointing at the active custom theme pack.
pub const APPEARANCE_THEME_PACK: &str = "appearance.theme_pack";
pub const ONBOARDING_WIZARD_COMPLETED: &str = "onboarding.wizard_completed";
/// Org-wide master switch for the onboarding flow family (Settings →
/// Onboarding). True silences the wizard banner AND user-level onboarding
/// org-wide; recorded, never silent.
pub const ONBOARDING_FLOWS_DISABLED: &str = "onboarding.flows_disabled";
/// The owner dismissed the SETUP BANNER (a "later" that sticks across
/// reloads; the wizard + Settings stay reachable).
pub const ONBOARDING_ORG_BANNER_DISMISSED: &str = "onboarding.org_banner_dismissed";
/// The member dismissed their own first-run prompt.
pub const ONBOARDING_USER_BANNER_DISMISSED: &str = "onboarding.user_banner_dismissed";

/// Canonical org wizard step ids (route module validates enumeration
/// membership against this list).
pub const ONBOARDING_ORG_STEPS: &[&str] =
    &["clients", "board", "tickets", "invites", "feeder", "ai"];

/// User onboarding step ids (counted flow family, per-user scope).
pub const ONBOARDING_USER_STEPS: &[&str] = &["profile", "mfa", "orientation", "defaults"];

/// Assistant for `onboarding.org_<step>_done` / `..._dismissed` key lookup.
/// Each org wizard step has BOTH a done flag (server-gated completion) and
/// a dismissed flag ("never show this again"); skip-for-now records
/// nothing — the step simply stays pending and resumable.
pub fn onboarding_org_step_key(step: &str, suffix: &str) -> String {
    format!("onboarding.org_{step}_{suffix}")
}

pub fn onboarding_user_step_key(step: &str, suffix: &str) -> String {
    format!("onboarding.user_{step}_{suffix}")
}

/// True when the dotted key belongs to the org wizard step flags.
pub fn is_onboarding_org_step_key(key: &str) -> bool {
    ONBOARDING_ORG_STEPS.iter().any(|step| {
        key == onboarding_org_step_key(step, "done")
            || key == onboarding_org_step_key(step, "dismissed")
    })
}

/// True when the dotted key belongs to the user-level onboarding flags.
pub fn is_onboarding_user_step_key(key: &str) -> bool {
    ONBOARDING_USER_STEPS.iter().any(|step| {
        key == onboarding_user_step_key(step, "done")
            || key == onboarding_user_step_key(step, "dismissed")
    })
}
pub const AI_COMPLETION_PROVIDER: &str = "ai.completion_provider";
pub const AI_COMPLETION_MODEL: &str = "ai.completion_model";
/// Admin policy flag (ADR 0008): whether users may add personal
/// (per-user) sealed AI credentials at all. Default OFF — personal keys
/// are opt-in per org, org credentials always work.
pub const AI_PERSONAL_CREDENTIALS_ENABLED: &str = "ai.personal_credentials_enabled";

/// The org master switch (ADR 0010 §4): when OFF (the default) the org
/// sees exactly the pre-AI product — no decision calls, no completion
/// calls, no suggestion cards, no AI entry points. One switch for all
/// AI edges.
pub const AI_FEATURES_ENABLED: &str = "ai.features_enabled";
/// Email greeting/sign-off (Wave 12 lane 6): per-user declarations for the
/// client-email composers (close-flow summary email, reply composer).
/// Editable defaults live in the settings registry, so they persist as
/// real user settings instead of device-local panel state.
pub const EMAIL_GREETING: &str = "email.greeting";
pub const EMAIL_SIGNOFF: &str = "email.signoff";
/// Org default for firing notifications on client ticket activity. A client
/// may override it through the per-client workflow-override registry
/// (`core_domain::workflow_overrides`), including an explicit disable.
pub const NOTIFICATIONS_CLIENT_EVENTS: &str = "notifications.client_events";
/// Org default for running automation rules against client tickets. A client
/// may override it through the per-client workflow-override registry,
/// including an explicit disable.
pub const AUTOMATIONS_CLIENT_RULES: &str = "automations.client_rules";
/// Per-connector enable/disable toggles (Wave 15 lane 4): one org-scope
/// Boolean per shipped connector family, default ON.
pub const CONNECTORS_CI_FEEDERS: &str = "connectors.ci_feeders";
pub const CONNECTORS_IMMYBOT: &str = "connectors.immybot";
pub const CONNECTORS_M365_GRAPH: &str = "connectors.m365_graph";
pub const CONNECTORS_DMS_SYNC: &str = "connectors.dms_sync";

/// The feed-upsert source systems that ride the `connectors.ci_feeders`
/// toggle (the CI feeder-line sources the feed-upsert seam serves).
pub const CI_FEEDER_SOURCE_SYSTEMS: &[&str] = &["ninjaone", "it_glue", "auvix"];

/// Whether the named connector is enabled for an org, from that org's
/// stored org-scope overrides: DEFAULT ON — absent key, unknown source
/// system (no toggle for it), or any non-false value mean enabled.
/// `overrides` is the literal stored rows (key → JSON value) from
/// `core_db::settings::list_org_overrides`.
pub fn connector_enabled(source_system: &str, overrides: &[(String, serde_json::Value)]) -> bool {
    let key = match source_system {
        _ if CI_FEEDER_SOURCE_SYSTEMS.contains(&source_system) => CONNECTORS_CI_FEEDERS,
        "immybot" => CONNECTORS_IMMYBOT,
        "m365_graph" => CONNECTORS_M365_GRAPH,
        "dms_sync" => CONNECTORS_DMS_SYNC,
        _ => return true, // unrecognized connectors stay ungated
    };
    overrides
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_bool())
        .unwrap_or(true)
}

/// The settings key that gates `source_system`, for consumers that want
/// to point AT the declaration (report surfaces, connector run manuals).
pub fn connector_key(source_system: &str) -> Option<&'static str> {
    match source_system {
        _ if CI_FEEDER_SOURCE_SYSTEMS.contains(&source_system) => Some(CONNECTORS_CI_FEEDERS),
        "immybot" => Some(CONNECTORS_IMMYBOT),
        "m365_graph" => Some(CONNECTORS_M365_GRAPH),
        "dms_sync" => Some(CONNECTORS_DMS_SYNC),
        _ => None,
    }
}

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

/// Shipped default theme presets (Wave 24 leg A). `tangerine` is the
/// pinned default-v1 of the theme-token schema.
pub const THEME_PRESET_OPTIONS: &[&str] = &["tangerine", "slate", "high_contrast", "copper"];
pub const MODE_OPTIONS: &[&str] = &["system", "light", "dark"];

pub const THEME_OPTIONS: &[&str] = THEME_PRESET_OPTIONS;
/// The "follow the org theme" sentinel value of appearance.theme_override.
pub const THEME_OVERRIDE_ORG_SENTINEL: &str = "org_default";
/// Per-user override enumerates the presets + the org-default sentinel.
pub const THEME_OVERRIDE_OPTIONS: &[&str] = &[
    THEME_OVERRIDE_ORG_SENTINEL,
    "tangerine",
    "slate",
    "high_contrast",
    "copper",
];

pub const AI_PROVIDER_OPTIONS: &[&str] = &[
    "none",
    "openai",
    "anthropic",
    "ollama",
    "custom",
    "typesafe",
];

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
pub static REGISTRY: std::sync::LazyLock<Vec<SettingDeclaration>> = std::sync::LazyLock::new(
    || {
        let mut decls = vec![
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
                default: SettingValue::Text("tangerine".into()),
                scope: SettingScope::Org,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Org theme",
                description: "The theme preset applied org-wide (Wave 24 theming system): \
                             Tangerine is the default; admins may pick any shipped built-in \
                             preset or an installed theme pack. Per-user overrides ride the \
                             appearance.theme_override setting.",
            },
            SettingDeclaration {
                key: APPEARANCE_THEME_OVERRIDE,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("org_default".into()),
                scope: SettingScope::User,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Theme override",
                description: "Per-user theme override. 'org_default' follows the org theme \
                             story; any other value pins one of the shipped presets for this \
                             account (user→org precedence, like the sticky AI model picks).",
            },
            SettingDeclaration {
                key: APPEARANCE_MODE,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("system".into()),
                scope: SettingScope::User,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Mode",
                description: "Light or dark appearance for this account. 'system' follows the \
                             device preference; the device-level instant toggle syncs here. \
                             Supersedes the old flat user-scope appearance.theme key \
                             (U6-safe migration 0097).",
            },
            SettingDeclaration {
                key: ONBOARDING_WIZARD_COMPLETED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Hidden,
                label: "First-run wizard completed",
                description: "LEGACY (Wave 26): superseded by the per-step onboarding.org_* \
                              flags. Still read as a U6-safe fallback: when no step flags \
                              exist yet, true means every org wizard step is done. Skipping \
                              changes nothing else — defaults already operate the org.",
            },
            SettingDeclaration {
                key: AI_COMPLETION_PROVIDER,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("none".into()),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "AI completion provider (legacy)",
                description: "DEPRECATED (ADR 0008): superseded by the AI availability \
                             matrix (settings/ai). Still readable and honored as an \
                             overridable org default when the catalog contains the \
                             customized (provider, model) pair; 'none' disables all \
                             outgoing model calls (none mode).",
            },
            SettingDeclaration {
                key: AI_COMPLETION_MODEL,
                kind: SettingKind::Text { max_len: 128 },
                default: SettingValue::Text("none".into()),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "AI completion model (legacy)",
                description: "DEPRECATED (ADR 0008): superseded by the AI availability \
                             matrix (settings/ai). Still readable and honored as an \
                             overridable org default when the catalog contains the \
                             customized (provider, model) pair.",
            },
            SettingDeclaration {
                key: AI_PERSONAL_CREDENTIALS_ENABLED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "Allow personal AI credentials",
                description: "Admin policy flag (ADR 0008): whether users may add \
                             personal (per-user) sealed AI credentials at all. Off \
                             (default) restricts AI to org credentials; on, a user's \
                             personal key serves only their own calls in surfaces \
                             they are granted.",
            },
            SettingDeclaration {
                key: AI_FEATURES_ENABLED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "AI features enabled",
                description: "The org master switch (ADR 0010 §4). Off (default) is \
                             the old-school posture: no AI calls of any edge fire — \
                             no decisions, no completion, no suggestions; the org \
                             sees exactly the pre-AI product. Per-surface enabled \
                             flags and deny-by-default grants apply on top.",
            },
            SettingDeclaration {
                key: NOTIFICATIONS_CLIENT_EVENTS,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "Notify on client ticket activity",
                description: "Org default for firing notifications on client ticket activity. \
                             A client may override this through its workflow overrides, \
                             including an explicit disable.",
            },
            SettingDeclaration {
                key: AUTOMATIONS_CLIENT_RULES,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "Run automations for client tickets",
                description: "Org default for running automation rules against a client's \
                             tickets. A client may override this through its workflow \
                             overrides, including an explicit disable.",
            },
            SettingDeclaration {
                key: EMAIL_GREETING,
                kind: SettingKind::Text { max_len: 500 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Email greeting",
                description: "Default greeting line for client emails you compose. \
                             The `{name}` placeholder expands to the recipient's name; \
                             empty falls back to the composer default (\"Hi {name},\").",
            },
            SettingDeclaration {
                key: EMAIL_SIGNOFF,
                kind: SettingKind::Text { max_len: 500 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Email sign-off",
                description: "Default sign-off block appended to client emails you \
                             compose (e.g. your name and title). Empty appends none.",
            },
            // Connector enable/disable toggles (Wave 15 lane 4
            // `p15-docs-closeout`): one org-scope Boolean per shipped
            // connector family, all DEFAULT ON (disabling is the
            // deliberate operator action). `connectors.ci_feeders` is
            // enforced server-side: the `/api/config-items/feed-upsert`
            // seam checks it for its three source systems (ninjaone,
            // it_glue, auvix) and rejects with a typed 403 when the org
            // turned the connector off. The standalone connectors
            // (immybot, m365_graph, dms_sync) run operator-side outside
            // the core, so their keys are the operator-facing surface the
            // connector run manual consults (and any future in-core
            // wiring enforces the same way).
            SettingDeclaration {
                key: CONNECTORS_CI_FEEDERS,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "CI feeders connector (NinjaOne / IT Glue / Auvix)",
                description: "Accept device/configuration feed upserts from the RMM / \
                             docs / NMS feeder sources (`clients:write` machine keys). \
                             Turning this off rejects their writes with a typed 403 \
                             without touching already-linked records.",
            },
            SettingDeclaration {
                key: CONNECTORS_IMMYBOT,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "ImmyBot connector",
                description: "Operator-side toggle for the ImmyBot patch-state inventory \
                             connector: a run that starts while this is off skips the \
                             push and reports every record as skipped-not-synced.",
            },
            SettingDeclaration {
                key: CONNECTORS_M365_GRAPH,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "M365 Graph connector",
                description: "Operator-side toggle for the Microsoft Intune/Entra device \
                             inventory connector: a run that starts while this is off \
                             reports the pull as disabled rather than half-applying.",
            },
            SettingDeclaration {
                key: CONNECTORS_DMS_SYNC,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "DMS sync connector (IT Glue / Hudu ↔ Meridian docs)",
                description: "Operator-side toggle for the documentation-system sync: a \
                              sync run that starts while this is off reports disabled \
                               before touching a single Meridian record.",
            },
            SettingDeclaration {
                key: ONBOARDING_ORG_BANNER_DISMISSED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Hidden,
                label: "Setup banner dismissed",
                description: "The owner explicitly dismissed the setup banner after at least                               looking at it ('later' is real: dismissal persists across                               reloads; the wizard itself and Settings → Onboarding both                               still reach the wizard). Cleared by reset/re-run.",
            },
            SettingDeclaration {
                key: ONBOARDING_USER_BANNER_DISMISSED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::User,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Hidden,
                label: "First-run prompt dismissed",
                description: "I have dismissed my own first-run prompt for now — persists                               across reloads (the flow itself still re-runs from Settings,                               and the profile menu keeps an entry point).",
            },
                        SettingDeclaration {
                key: ONBOARDING_FLOWS_DISABLED,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Ui,
                label: "Disable onboarding flows org-wide",
                description: "Silences the org setup wizard and every member's first-run \
                              onboarding org-wide. State is kept, so re-enabling resumes \
                              exactly where flows were left.",
            },
        ];

        // --- Wave 26 onboarding: the flow family's typed flags. Every step        // is INDEPENDENTLY skippable — `done` records server-gated
        // completion, `dismissed` records "never show this again" (both
        // persisted choices, never a silent nag-loop). Dynamic step keys
        // are leaked once at init so the registry stays &'static.
        let flag_decl =
            |key: String, scope: SettingScope, label: &str, descr: &str| SettingDeclaration {
                key: Box::leak(key.into_boxed_str()),
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope,
                section: SettingSection::Onboarding,
                visibility: SettingVisibility::Hidden,
                label: Box::leak(label.to_string().into_boxed_str()),
                description: Box::leak(descr.to_string().into_boxed_str()),
            };
        // (step, done-label, dismissed-label, done-description,
        //  dismissed-description)
        let org_steps: &[(&str, &str, &str, &str, &str)] = &[
            ("clients",
                "Clients added",
                "Clients step dismissed",
                "Set when the org has at least one client recorded (server-verified), or manually by the owner via the wizard's done action when satisfied.",
                "Set when the owner pressed 'never show this again' on the clients step — recorded, not silent."),
            ("board",
                "First board configured",
                "Board step dismissed",
                "Set when the org has at least one ticket board (server-verified).",
                "Set when the owner pressed 'never show this again' on the board step."),
            ("tickets",
                "First tickets raised",
                "Tickets step dismissed",
                "Set when the org has at least one ticket (server-verified).",
                "Set when the owner pressed 'never show this again' on the tickets step."),
            ("invites",
                "Team invited",
                "Invites step dismissed",
                "Set when at least one member invite has been issued (server-verified).",
                "Set when the owner pressed 'never show this again' on the invites step."),
            ("feeder",
                "Feeder connector connected",
                "Feeder step dismissed",
                "Set when a feeder source has actually written configuration records into the org (server-verified).",
                "Set when the owner pressed 'never show this again' on the feeder step."),
            ("ai",
                "AI surfaces enabled",
                "AI step dismissed",
                "Set when the org has turned on its AI surfaces (server-verified against the AI settings master switch).",
                "Set when the owner pressed 'never show this again' on the AI step."),
        ];
        for (step, done_label, dismissed_label, done_descr, dismissed_descr) in org_steps {
            decls.push(flag_decl(
                onboarding_org_step_key(step, "done"),
                SettingScope::Org,
                done_label,
                done_descr,
            ));
            decls.push(flag_decl(
                onboarding_org_step_key(step, "dismissed"),
                SettingScope::Org,
                dismissed_label,
                dismissed_descr,
            ));
        }
        let user_steps: &[(&str, &str, &str, &str, &str)] = &[
            ("profile",
                "Profile completed",
                "Profile step dismissed",
                "Set when your profile detail is complete (server-verified).",
                "Set when 'never show this again' was pressed on the profile step."),
            ("mfa",
                "Second factor configured",
                "Second-factor step dismissed",
                "Set when a second factor has been enrolled (server-verified).",
                "Set when 'never show this again' was pressed on the second-factor step."),
            ("orientation",
                "Orientation done",
                "Orientation dismissed",
                "Set after the 60-second product orientation is completed.",
                "Set when 'never show this again' was pressed on the orientation."),
            ("defaults",
                "Personal defaults set",
                "Personal-defaults step dismissed",
                "Set after personal defaults (theme/mode/notifications) were saved or explicitly skipped.",
                "Set when 'never show this again' was pressed on the personal-defaults step."),
        ];
        for (step, done_label, dismissed_label, done_descr, dismissed_descr) in user_steps {
            decls.push(flag_decl(
                onboarding_user_step_key(step, "done"),
                SettingScope::User,
                done_label,
                done_descr,
            ));
            decls.push(flag_decl(
                onboarding_user_step_key(step, "dismissed"),
                SettingScope::User,
                dismissed_label,
                dismissed_descr,
            ));
        }
        decls
    },
);

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
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_'),
                "key {} must be lowercase dotted snake_case (digits allowed)",
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
            Some(SettingValue::Text("tangerine".into()))
        );
        // Wave 24: per-user override + per-user mode (appearance.theme
        // is org-scope now): greeting, signoff, override, mode = 4.
        // Wave 26: user onboarding adds four steps × done/dismissed = 8.
        assert_eq!(declarations_for(SettingScope::User).len(), 13);
        // Tripwire count: bump deliberately when an org-scope setting is
        // added (Wave 24 lane 1 repurposed appearance.theme as org scope
        // — 14 → 15; Wave 26 adds flows_disabled + six steps ×
        // done/dismissed = 13, so 15 → 28).
        assert_eq!(declarations_for(SettingScope::Org).len(), 29);
    }

    #[test]
    fn email_greeting_and_signoff_are_user_scope_text_settings() {
        for key in [EMAIL_GREETING, EMAIL_SIGNOFF] {
            let d = declaration(key).unwrap_or_else(|| panic!("{key} declared"));
            assert_eq!(d.scope, SettingScope::User);
            assert_eq!(d.section, SettingSection::Email);
            assert!(d.validate(&SettingValue::Text("Hi {name},".into())).is_ok());
            assert!(d.validate(&SettingValue::Integer(1)).is_err());
            let too_long = "x".repeat(501);
            assert!(d.validate(&SettingValue::Text(too_long)).is_err());
        }
        // Defaults are empty: untouched users keep the composer defaults.
        assert_eq!(
            default_value(EMAIL_GREETING),
            Some(SettingValue::Text(String::new()))
        );
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
    fn connector_toggles_default_on_and_gate_known_sources() {
        let none: Vec<(String, serde_json::Value)> = Vec::new();
        for source in [
            "ninjaone",
            "it_glue",
            "auvix",
            "immybot",
            "m365_graph",
            "dms_sync",
        ] {
            assert!(
                connector_enabled(source, &none),
                "{source} enabled with no overrides"
            );
            assert!(connector_key(source).is_some(), "{source} has a key");
        }
        // Unknown source systems stay ungated (the feed seam accepts open
        // source systems; only shipped connectors have toggles).
        assert!(connector_enabled("some_new_rmm", &none));
        assert!(connector_key("some_new_rmm").is_none());

        let off = vec![(CONNECTORS_CI_FEEDERS.to_string(), serde_json::json!(false))];
        // The whole feeder family toggles together (one connector, three
        // source systems).
        assert!(!connector_enabled("ninjaone", &off));
        assert!(!connector_enabled("it_glue", &off));
        assert!(!connector_enabled("auvix", &off));
        // Other connectors are NOT disabled by another connector's key.
        assert!(connector_enabled("immybot", &off));
        assert_eq!(connector_key("ninjaone"), Some(CONNECTORS_CI_FEEDERS));
        assert_eq!(connector_key("dms_sync"), Some(CONNECTORS_DMS_SYNC));
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
