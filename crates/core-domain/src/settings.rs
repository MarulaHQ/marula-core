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
    Surveys,
}

impl SettingSection {
    pub const fn as_str(self) -> &'static str {
        match self {
            SettingSection::General => "general",
            SettingSection::Appearance => "appearance",
            SettingSection::Onboarding => "onboarding",
            SettingSection::Integrations => "integrations",
            SettingSection::Email => "email",
            SettingSection::Surveys => "surveys",
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
                APPEARANCE_DENSITY => DENSITY_OPTIONS,
                AI_COMPLETION_PROVIDER => AI_PROVIDER_OPTIONS,
                SHORTCUTS_PROFILE => SHORTCUT_PROFILE_OPTIONS,
                TRAVEL_MILEAGE_RATE_UNIT => MILEAGE_RATE_UNIT_OPTIONS,
                EMAIL_TRANSPORT => EMAIL_TRANSPORT_OPTIONS,
                EMAIL_DIGEST_POLICY => EMAIL_DIGEST_POLICY_OPTIONS,
                EMAIL_SIGNATURE_POLICY => EMAIL_SIGNATURE_POLICY_OPTIONS,
                EMAIL_DIGEST => EMAIL_DIGEST_OPTIONS,
                EMAIL_INBOUND_SENDER_BINDING => EMAIL_INBOUND_SENDER_BINDING_OPTIONS,
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
/// Per-user switch for the second clock (Wave 50 lane T2). On: a surface that
/// knows a record's zone draws it beside the reader's. Off: the reader's zone
/// is the only reading.
pub const GENERAL_SHOW_RECORD_TIME: &str = "general.show_record_time";
pub const APPEARANCE_THEME: &str = "appearance.theme";
/// Per-user override over the org theme (Wave 24). 'org_default' sentinel.
pub const APPEARANCE_THEME_OVERRIDE: &str = "appearance.theme_override";
/// Per-user light/dark mode. Supersedes the old flat user-scope
/// `appearance.theme` key via migration 0097 (U6-safe supersede).
pub const APPEARANCE_MODE: &str = "appearance.mode";

/// Wave 41 lane 2b, decision D7: the density step, user-selectable, three
/// steps. `comfortable` is the default and matches Marula; `dense` reproduces
/// the pre-Marula values so the data-heavy fleet, stock and billing tables do
/// not regress for a self-hoster who needs them not to.
pub const APPEARANCE_DENSITY: &str = "appearance.density";

/// Wave 41 lane 2b, decision D12: the four data-surface controls (views,
/// columns, filters, export) for one user, as ONE document keyed by surface.
/// The settings table's value column carries a CHECK that admits only a
/// string, a number or a boolean, so the document is stored as a JSON string
/// and typed by `crate::data_surface` rather than by the registry's scalar
/// `SettingValue` union — richer structures are entities, not settings.
pub const APPEARANCE_DATA_SURFACES: &str = "appearance.data_surfaces";
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
///
/// Wave 42: `email.signoff` is the PERSONAL SIGNATURE. Its meaning is
/// unchanged (the block that closes mail a person writes) and the key is
/// kept so the composers that already read it keep working, but it now
/// carries the template dialect's placeholders, resolves at SEND time from
/// the sending user's row, and is gated by `EMAIL_SIGNATURE_POLICY`.
/// `email.greeting` stays a plain salutation line with no placeholders —
/// the recipient's name is the COMPOSER's business (it knows the ticket's
/// contact), not the settings store's.
pub const EMAIL_GREETING: &str = "email.greeting";
pub const EMAIL_SIGNOFF: &str = "email.signoff";
/// Org default signature (the pre-Wave-42 `email.signature`, unchanged key —
/// Wave 42 re-scoped the ROLES around it, not the storage): the block that
/// closes mail the org sends, and the fallback for a member with no
/// signature of their own. Holds the same placeholders as a personal
/// signature, resolved at send time against the SENDING user.
pub const EMAIL_SIGNATURE: &str = "email.signature";
/// Org default greeting (Wave 42). The house voice: opens mail the org
/// sends to its own staff (notification and digest envelopes), and is the
/// fallback for a person with no greeting of their own.
pub const EMAIL_ORG_GREETING: &str = "email.org_greeting";
/// Admin policy over personal signatures (Wave 42): `allowed` (a member
/// may keep their own signature; the org default is the fallback) or
/// `org_locked` (every signature is the org's — the personal editor and
/// the per-message override are both refused). Deny wins, the same shape
/// as `EMAIL_DIGEST_POLICY`.
pub const EMAIL_SIGNATURE_POLICY: &str = "email.signature_policy";

// --- Wave 34 lane 8 (`p34-email-config-closeout`): mail configuration ----
//
// Org-scope mail configuration lives here as typed settings (the transport
// SELECTION / identity fields — all non-secret). The transport CREDENTIALS
// (Postmark server token, SMTP password) never ride settings: they are
// sealed into the generic sealed-key store (`byok_secrets`, provider
// `mail.postmark` / `mail.smtp`) through the admin API and are never
// returned or logged. Per-BOARD outbound identities are org-scope settings
// under board-qualified keys (`email.board.<board_id>.<field>`) written
// only by the board-identity admin routes; they are intentionally NOT
// declared here because the key set is per-board (unbounded), exactly like
// the `onboarding.org_<step>_done` family is addressed by helper rather
// than declaration.

/// Outbound transport selection: `none` (queued, honest not-configured
/// state), `postmark` (API token in the sealed slot), or `smtp` (host +
/// username in settings, password in the sealed slot).
pub const EMAIL_TRANSPORT: &str = "email.transport";
/// Org-level From address for Marula-generated mail. Boards override it
/// with their own identity; empty keeps the operator/process default.
pub const EMAIL_FROM_ADDRESS: &str = "email.from_address";
pub const EMAIL_FROM_NAME: &str = "email.from_name";
/// Org-level Reply-To. Empty sends no Reply-To header (the From address is
/// the reply path). Boards override it with their own identity.
pub const EMAIL_REPLY_TO: &str = "email.reply_to";
/// Wordmark shown in the branded notification/digest wrapper header.
/// Empty falls back to the org name.
pub const EMAIL_BRAND_NAME: &str = "email.brand_name";
/// Optional https URL of a logo rendered in the branded wrapper header.
/// Empty renders the wordmark alone.
pub const EMAIL_LOGO_URL: &str = "email.logo_url";
pub const EMAIL_SMTP_HOST: &str = "email.smtp_host";
pub const EMAIL_SMTP_PORT: &str = "email.smtp_port";
pub const EMAIL_SMTP_USERNAME: &str = "email.smtp_username";
pub const EMAIL_POSTMARK_BASE_URL: &str = "email.postmark_base_url";
/// opt in) or `disabled` (no digest sends org-wide). Deny wins.
pub const EMAIL_DIGEST_POLICY: &str = "email.digest_policy";
/// Per-user daily digest opt-in: `off` | `daily`. The digest is ONE email
/// summarizing unread notifications, sent at `email.digest_hour` in the
/// user's own timezone; because it is scheduled (not interruptive) quiet
/// hours do not suppress it — the chosen hour IS the quiet-hours answer.
pub const EMAIL_DIGEST: &str = "email.digest";
/// Local hour (0-23) the daily digest sends at, in the user's quiet-hours
/// timezone (falling back to UTC).
pub const EMAIL_DIGEST_HOUR: &str = "email.digest_hour";
/// Hidden idempotency marker: the local date (`YYYY-MM-DD`) of the last
/// digest send, so at-least-once loop ticks cannot double-send a day.
pub const EMAIL_DIGEST_LAST_SENT: &str = "email.digest_last_sent";
/// The AWS region whose SES endpoint this org sends through (e.g.
/// `eu-west-1`). Not a preference: SES endpoints are region-scoped, so a
/// wrong region sends to the wrong data centre or fails to resolve. Only
/// the key pair is sealed, because that is the secret. The region is plain
/// config and stays readable so the admin UI can display it.
pub const EMAIL_SES_REGION: &str = "email.ses_region";
/// Optional SES endpoint override, for a VPC interface endpoint or a
/// private-link deployment. Empty means the public regional endpoint.
pub const EMAIL_SES_ENDPOINT: &str = "email.ses_endpoint";

/// Transport selections the settings UI and worker resolver accept.
/// `ses` is the third hosted option (Wave 34 claims close-out, claim 10);
/// the ordering is the order the settings UI renders them in, so the two
/// token-based providers lead and SES follows the relay-style option.
pub const EMAIL_TRANSPORT_OPTIONS: &[&str] = &["none", "postmark", "smtp", "ses"];

/// CSAT surveys on resolution: master toggle. Off (or transport `none`)
/// means no surveys are ever queued — the honest not-configured state.
pub const CSAT_SEND_ON_RESOLVE: &str = "csat.send_on_resolve";
/// Per-client minimum interval in hours between survey sends; a client that
/// received a survey more recently is skipped (recorded as `min_interval`).
pub const CSAT_MIN_INTERVAL_HOURS: &str = "csat.min_interval_hours";
/// Days a tokenized response link stays valid after send; expired links are
/// rejected at the portal route, tokens stay single-use regardless.
pub const CSAT_TOKEN_TTL_DAYS: &str = "csat.token_ttl_days";
/// Org quiet-hours window (local `HH:MM`, org timezone) surveys defer to.
/// Empty means no quiet window — surveys send as soon as they are due.
pub const CSAT_QUIET_FROM: &str = "csat.quiet_from";
pub const CSAT_QUIET_UNTIL: &str = "csat.quiet_until";
/// Digest policy options (org scope).
pub const EMAIL_DIGEST_POLICY_OPTIONS: &[&str] = &["allowed", "disabled"];
/// Signature policy options (org scope, Wave 42).
pub const EMAIL_SIGNATURE_POLICY_OPTIONS: &[&str] = &["allowed", "org_locked"];
/// Per-user digest options.
pub const EMAIL_DIGEST_OPTIONS: &[&str] = &["off", "daily"];

// --- Wave 55 (`p55-inbound-sender-binding`): who may email a ticket -------
//
// Record 4 of the Wave 54 security decisions. The webhook HMAC proves a
// message came through the mail provider and says nothing about who wrote
// it, so a reply may only join an existing ticket when its `From` is already
// a participant on that ticket. A message from anyone else waits.
//
// The level is the org's grant of authority to the decision engine, and the
// three answers are levels of triage rather than one rule: `human_only`
// (default) holds every non-participant message for a person, `shadow`
// records what the engine would have done beside the held message while a
// person still decides, and `autonomous` applies the engine's decision with
// the audit row a person's decision leaves. `core-api/src/email.rs` reads it
// before the inbound ledger row is written.

/// Authority the org grants the engine over a message whose sender is not a
/// participant on the ticket it names.
pub const EMAIL_INBOUND_SENDER_BINDING: &str = "email.inbound_sender_binding";
/// The three levels, in the order the settings UI renders them: the default
/// first, then the two that are deliberate acts.
pub const EMAIL_INBOUND_SENDER_BINDING_OPTIONS: &[&str] = &["human_only", "shadow", "autonomous"];

/// True when the key belongs to the per-board outbound identity family
/// (`email.board.<uuid>.<field>`), addressed dynamically per board.
pub fn is_board_identity_key(key: &str) -> bool {
    board_identity_parts(key).is_some()
}

/// Parses `email.board.<uuid>.<from_address|from_name|reply_to|signature>`
/// into its coordinates; `None` for everything else (including unknown
/// fields, so callers can reject typos instead of silently storing them).
pub fn board_identity_parts(key: &str) -> Option<(uuid::Uuid, BoardIdentityField)> {
    let rest = key.strip_prefix("email.board.")?;
    let (id, field) = rest.split_once('.')?;
    let board_id = uuid::Uuid::parse_str(id).ok()?;
    let field = match field {
        "from_address" => BoardIdentityField::FromAddress,
        "from_name" => BoardIdentityField::FromName,
        "reply_to" => BoardIdentityField::ReplyTo,
        "signature" => BoardIdentityField::Signature,
        _ => return None,
    };
    Some((board_id, field))
}

/// The four per-board outbound identity fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardIdentityField {
    FromAddress,
    FromName,
    ReplyTo,
    Signature,
}

impl BoardIdentityField {
    pub const ALL: [BoardIdentityField; 4] = [
        BoardIdentityField::FromAddress,
        BoardIdentityField::FromName,
        BoardIdentityField::ReplyTo,
        BoardIdentityField::Signature,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            BoardIdentityField::FromAddress => "from_address",
            BoardIdentityField::FromName => "from_name",
            BoardIdentityField::ReplyTo => "reply_to",
            BoardIdentityField::Signature => "signature",
        }
    }

    /// Max stored length per field — mirrors the settings validation caps.
    pub const fn max_len(self) -> usize {
        match self {
            BoardIdentityField::FromAddress | BoardIdentityField::ReplyTo => 254,
            BoardIdentityField::FromName => 120,
            BoardIdentityField::Signature => 1000,
        }
    }
}

/// Builds the settings key for one board identity field.
pub fn board_identity_key(board_id: uuid::Uuid, field: BoardIdentityField) -> String {
    format!("email.board.{board_id}.{}", field.as_str())
}

/// Wave 26 lane 2 (`p26-contextual-help`): per-user "Was this helpful?"
/// feedback on docs surfaced by the in-app help panel. Text carrying a
/// capped JSON log (`[{doc, up, at}]`, newest last, max 50) — append-only
/// in spirit, persisted through the ordinary settings audit trail.
pub const HELP_DOC_FEEDBACK: &str = "help.doc_feedback";
/// Wave 26 lane 2: the user-level "don't show me tours" switch. OFF by
/// default; flipping it stops every auto-started feature tour for this
/// account (manual tours from the help panel still run).
pub const TOURS_DISABLED: &str = "tours.disabled";
/// Wave 26 lane 2: which feature tours this account has completed or
/// skipped, as a capped JSON array of tour ids (max 100). Hidden — state,
/// not a preference.
pub const TOURS_COMPLETED: &str = "tours.completed";
/// Wave 26 lane 2: the org-wide kill switch for feature tours. ON when an
/// admin decides tours distract the org; individual opt-outs ride
/// `tours.disabled` independently.
pub const TOURS_ORG_DISABLED: &str = "tours.org_disabled";
/// Wave 29 lane 1 (`p29-keyboard-shortcuts`): per-user keyboard-shortcut
/// profile. 'default' runs the standard Alt-command registry; 'all_off'
/// disables chords + the hold-Alt overlay for this account (the ?-cheatsheet
/// stays reachable as a read-only reference). Per-command remapping lands
/// later and extends this enumeration — the key itself is immutable.
pub const SHORTCUTS_PROFILE: &str = "shortcuts.profile";
/// Org default for firing notifications on client ticket activity. A client
/// may override it through the per-client workflow-override registry
/// (`core_domain::workflow_overrides`), including an explicit disable.
pub const NOTIFICATIONS_CLIENT_EVENTS: &str = "notifications.client_events";
/// Org default for running automation rules against client tickets. A client
/// may override it through the per-client workflow-override registry,
/// including an explicit disable.
pub const AUTOMATIONS_CLIENT_RULES: &str = "automations.client_rules";
/// Wave 43 lane 14 (`p43-workflow-run-ledger`): how many days a FINISHED
/// workflow run stays in `workflow_runs` before the retention sweep drops
/// it. Runs are the highest-volume rows the workflow engine produces, so
/// the window is a setting rather than an unbounded table: 30 days by
/// default, bounded by `workflow_runs::RETENTION_MIN_DAYS` /
/// `RETENTION_MAX_DAYS` (an out-of-range stored value falls back to the
/// default at read time instead of silently extending the table).
pub const WORKFLOW_RUN_RETENTION_DAYS: &str = "workflow.run_retention_days";
/// Per-connector enable/disable toggles (Wave 15 lane 4): one org-scope
/// Boolean per shipped connector family, default ON.
pub const CONNECTORS_CI_FEEDERS: &str = "connectors.ci_feeders";
pub const CONNECTORS_IMMYBOT: &str = "connectors.immybot";
pub const CONNECTORS_M365_GRAPH: &str = "connectors.m365_graph";
pub const CONNECTORS_DMS_SYNC: &str = "connectors.dms_sync";

// --- Wave 32 lane 7 (`p32-dispatch-routing`): routing/travel policy --------
//
// The routing PROVIDER (and its credentials) is operator infrastructure
// config — env-parsed like the email transport, never org settings (a
// token is not a preference). What lives here is the org POLICY the
// NONE-mode estimate and mileage reconciliation consume: the per-mile
// heuristic table and the reimbursement rates.

/// Minutes per mile the built-in (NONE-mode) travel estimate assumes,
/// including typical door-to-door overhead. 2 = a 30 mph effective pace.
pub const ROUTING_HEURISTIC_MINUTES_PER_MILE: &str = "routing.heuristic_minutes_per_mile";
/// Fixed minutes added to every heuristic leg (parking, walk-in).
pub const ROUTING_HEURISTIC_OVERHEAD_MINUTES: &str = "routing.heuristic_overhead_minutes";
/// Road factor percent: great-circle distance × this / 100 approximates
/// road distance. 100 would assume straight roads (dishonest); default 125.
pub const ROUTING_HEURISTIC_ROAD_FACTOR_PERCENT: &str = "routing.heuristic_road_factor_percent";
/// Reimbursement rate in MINOR currency units per `travel.mileage_rate_unit`.
pub const TRAVEL_MILEAGE_RATE_MINOR: &str = "travel.mileage_rate_minor";
/// The distance unit the mileage rate is quoted per: `mi` | `km`.
pub const TRAVEL_MILEAGE_RATE_UNIT: &str = "travel.mileage_rate_unit";
/// Whether recorded (submitted) travel mileage rolls into a
/// reimbursement-to-technician rollup. Off keeps mileage informational.
pub const TRAVEL_REIMBURSE_TECHS: &str = "travel.reimburse_techs";
/// Unit options for [`TRAVEL_MILEAGE_RATE_UNIT`].
pub const MILEAGE_RATE_UNIT_OPTIONS: &[&str] = &["mi", "km"];

/// The org's workflow node-kind switch (Wave 56 lane `p56-node-authority`):
/// a JSON array of node type ids the org has switched off (a comma-separated
/// list is accepted too). Empty means every executable kind is enabled, which
/// is defaults-first and keeps an existing org's documents runnable. Read by
/// the workflow publish boundary and by the executor's authority re-check
/// through `workflow_document::OrgNodePolicy`.
pub const WORKFLOW_DISABLED_NODE_KINDS: &str = "workflow.disabled_node_kinds";

/// The folded routing/travel policy an org operates under, read from its
/// stored org-scope overrides. Defaults ARE the shipped table (see the
/// constants above); an unknown or absent key falls back to the default,
/// exactly like every other typed setting read.
#[derive(Debug, Clone, PartialEq)]
pub struct TravelPolicy {
    pub heuristic_minutes_per_mile: i64,
    pub heuristic_overhead_minutes: i64,
    pub heuristic_road_factor_percent: i64,
    pub mileage_rate_minor: i64,
    pub mileage_rate_unit: String,
    pub reimburse_techs: bool,
}

impl Default for TravelPolicy {
    fn default() -> Self {
        Self {
            heuristic_minutes_per_mile: 2,
            heuristic_overhead_minutes: 5,
            heuristic_road_factor_percent: 125,
            mileage_rate_minor: 67,
            mileage_rate_unit: "mi".into(),
            reimburse_techs: true,
        }
    }
}

/// Folds stored overrides onto the default policy. Keys are validated at
/// write time by the registry; a wrong-typed stored value is ignored here
/// (defensive read — the default is always better than a panic).
pub fn travel_policy(overrides: &[(String, serde_json::Value)]) -> TravelPolicy {
    fn int(overrides: &[(String, serde_json::Value)], key: &str) -> Option<i64> {
        overrides
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_i64())
    }
    fn text(overrides: &[(String, serde_json::Value)], key: &str) -> Option<String> {
        overrides
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_str().map(str::to_string))
    }
    fn boolean(overrides: &[(String, serde_json::Value)], key: &str) -> Option<bool> {
        overrides
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_bool())
    }
    let defaults = TravelPolicy::default();
    TravelPolicy {
        heuristic_minutes_per_mile: int(overrides, ROUTING_HEURISTIC_MINUTES_PER_MILE)
            .filter(|v| *v > 0)
            .unwrap_or(defaults.heuristic_minutes_per_mile),
        heuristic_overhead_minutes: int(overrides, ROUTING_HEURISTIC_OVERHEAD_MINUTES)
            .filter(|v| *v >= 0)
            .unwrap_or(defaults.heuristic_overhead_minutes),
        heuristic_road_factor_percent: int(overrides, ROUTING_HEURISTIC_ROAD_FACTOR_PERCENT)
            .filter(|v| (100..=300).contains(v))
            .unwrap_or(defaults.heuristic_road_factor_percent),
        mileage_rate_minor: int(overrides, TRAVEL_MILEAGE_RATE_MINOR)
            .filter(|v| *v >= 0)
            .unwrap_or(defaults.mileage_rate_minor),
        mileage_rate_unit: text(overrides, TRAVEL_MILEAGE_RATE_UNIT)
            .filter(|v| MILEAGE_RATE_UNIT_OPTIONS.contains(&v.as_str()))
            .unwrap_or(defaults.mileage_rate_unit),
        reimburse_techs: boolean(overrides, TRAVEL_REIMBURSE_TECHS)
            .unwrap_or(defaults.reimburse_techs),
    }
}

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

/// Shipped default theme presets (Wave 24 leg A; repointed to Marula in
/// wave41 lane 2). `marula` is the pinned default-v1 of the theme-token
/// schema and the default for web and desktop. `marula-warm` is the cream
/// column of the same pack and the default on mobile (wave41 D2).
///
/// `slate` and `copper` are DROPPED (wave41 D8): both were second-brand
/// neutrals that never had authority in the brand spec, and the axis they
/// wanted to move is now owned by `marula` vs `marula-warm`.
pub const THEME_PRESET_OPTIONS: &[&str] = &["marula", "marula-warm", "high_contrast"];
pub const MODE_OPTIONS: &[&str] = &["system", "light", "dark"];

/// The three density steps of D7, in the order a chooser renders them. The
/// order is the ergonomic order (roomiest first), not the numeric one.
pub const DENSITY_OPTIONS: &[&str] = &["comfortable", "compact", "dense"];

pub const THEME_OPTIONS: &[&str] = THEME_PRESET_OPTIONS;
/// Keyboard-shortcut profiles (Wave 29 lane 1). 'custom' per-command
/// remapping is reserved but not shipped: adding it later is an additive
/// options extension, never a key change.
pub const SHORTCUT_PROFILE_OPTIONS: &[&str] = &["default", "all_off"];
/// The "follow the org theme" sentinel value of appearance.theme_override.
pub const THEME_OVERRIDE_ORG_SENTINEL: &str = "org_default";
/// Per-user override enumerates the presets + the org-default sentinel.
pub const THEME_OVERRIDE_OPTIONS: &[&str] = &[
    THEME_OVERRIDE_ORG_SENTINEL,
    "marula",
    "marula-warm",
    "high_contrast",
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
                key: GENERAL_SHOW_RECORD_TIME,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(true),
                scope: SettingScope::User,
                section: SettingSection::General,
                visibility: SettingVisibility::Ui,
                label: "Show each record's time zone",
                description: "Draw a record's own clock beside yours when the two differ — \
                              the site's zone on a ticket, the member's on a time entry. When \
                              the zones agree nothing changes, so only readers working across \
                              zones see a second reading.",
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
                default: SettingValue::Text("marula".into()),
                scope: SettingScope::Org,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Org theme",
                description: "The theme preset applied org-wide (Wave 24 theming system): \
                             Marula is the default; admins may pick any shipped built-in \
                             preset or an installed theme pack. Per-user overrides ride the \
                             appearance.theme_override setting. A preset may move the \
                             neutral, chrome and accent-tint axes only — the primary and its \
                             ink-to-gold mode flip, the destructive colour, the status pairs \
                             and the assistant inversion are the brand's and stay fixed.",
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
                key: APPEARANCE_DENSITY,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("comfortable".into()),
                scope: SettingScope::User,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Ui,
                label: "Density",
                description: "How much room the interface takes: comfortable (44px controls, \
                             48px rows, 24px card padding, the Marula default), compact \
                             (36/40/16) or dense (32/32/12, the pre-Marula values the \
                             data-heavy fleet, stock and billing tables were built at). One \
                             attribute on the document root moves every control that reads \
                             the token, so this is a per-user preference rather than a \
                             per-surface setting. 44px stays the floor for tap targets on \
                             every step; only pointer targets sit at 32px under dense.",
            },
            SettingDeclaration {
                key: APPEARANCE_DATA_SURFACES,
                kind: SettingKind::Text {
                    max_len: crate::data_surface::MAX_DOCUMENT_CHARS,
                },
                default: SettingValue::Text("{}".into()),
                scope: SettingScope::User,
                section: SettingSection::Appearance,
                visibility: SettingVisibility::Hidden,
                label: "Data surface preferences",
                description: "Per-user state for the data-surface controls: the named view, \
                             the visible columns and their order and widths, the active \
                             filter clauses and the sort, for each surface that has been \
                             changed. A JSON document stored as a string (the settings value \
                             column admits no object) and parsed by \
                             core_domain::data_surface, which owns the closed vocabularies \
                             and rejects anything malformed. An empty document means every \
                             surface is on its own defaults; export reads the same document, \
                             so an export always matches what the table is showing. \
                             Hidden from the generated UI: the four controls write it.",
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
                key: WORKFLOW_RUN_RETENTION_DAYS,
                kind: SettingKind::Integer {
                    min: crate::workflow_runs::RETENTION_MIN_DAYS,
                    max: crate::workflow_runs::RETENTION_MAX_DAYS,
                },
                default: SettingValue::Integer(crate::workflow_runs::RETENTION_DEFAULT_DAYS),
                scope: SettingScope::Org,
                section: SettingSection::Integrations,
                visibility: SettingVisibility::Ui,
                label: "Keep workflow runs for (days)",
                description: "How long a finished workflow run stays in the run ledger \
                             before the retention sweep removes it. The run ledger is the \
                             record an operator reads to see what a workflow actually did, \
                             and it is also the highest-volume table the engine writes, so \
                             the window is a setting rather than an unbounded table. Runs \
                             still parked or still running are never swept.",
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
                kind: SettingKind::Text { max_len: 1000 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Your email signature",
                description: "The block that closes email you write to a client. \
                             Placeholders like {{user.name}} and {{org.name}} fill \
                             in when the message sends, never when you save. Empty \
                             falls back to the workspace signature — and while the \
                             workspace signature is locked to the organization, this \
                             field is refused rather than ignored.",
            },
            // Wave 42: the org's own greeting + the policy over personal
            // signatures. Both are org scope; the personal signature is the
            // user-scope `email.signoff` above.
            SettingDeclaration {
                key: EMAIL_ORG_GREETING,
                kind: SettingKind::Text { max_len: 500 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Org email greeting",
                description: "Opens mail the workspace sends to its own staff — \
                             notifications and the daily digest. It is also the \
                             fallback for a member with no greeting of their own. \
                             Empty opens the message with the recipient's name.",
            },
            SettingDeclaration {
                key: EMAIL_SIGNATURE_POLICY,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("allowed".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Personal signatures",
                description: "Whether members may sign mail with their own details. \
                             Allowed keeps a personal signature with the workspace \
                             one as its fallback; locked to the organization sends \
                             the workspace signature on every message and refuses \
                             personal ones.",
            },
            // Wave 55 (`p55-inbound-sender-binding`): who may email a ticket.
            SettingDeclaration {
                key: EMAIL_INBOUND_SENDER_BINDING,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("human_only".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Replies from unknown senders",
                description: "What happens when someone who is not already on a \
                             ticket emails that ticket. Human only holds the message \
                             for a person and nothing reaches the ticket until they \
                             decide. Shadow also records what the decision engine \
                             would have done, so its agreement can be measured while \
                             a person still decides. Autonomous applies the engine's \
                             decision and records it as one.",
            },
            // Wave 34 lane 8 (`p34-email-config-closeout`): outbound mail
            // configuration. Selection/identity only — credentials live in
            // the sealed slot, never here.
            SettingDeclaration {
                key: EMAIL_TRANSPORT,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("none".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Outbound transport",
                description: "How Marula sends mail: none keeps messages queued \
                             (honest not-configured state), postmark sends through \
                             the Postmark API, smtp relays through your own SMTP \
                             host. Credentials go through the sealed slot on the \
                             email settings page — never environment-only.",
            },
            SettingDeclaration {
                key: EMAIL_FROM_ADDRESS,
                kind: SettingKind::Text { max_len: 254 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "From address",
                description: "The address Marula mail is sent from (for example \
                             support@yourdomain.com). Boards with their own \
                             identity override it. Empty keeps the operator \
                             process default.",
            },
            SettingDeclaration {
                key: EMAIL_FROM_NAME,
                kind: SettingKind::Text { max_len: 120 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "From display name",
                description: "Display name shown beside the From address (for \
                             example \"Acme Support\"). Boards with their own \
                             identity override it.",
            },
            SettingDeclaration {
                key: EMAIL_REPLY_TO,
                kind: SettingKind::Text { max_len: 254 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Reply-To address",
                description: "Where client replies go when it differs from the From \
                             address — usually the board's inbound address. Empty \
                             sends no Reply-To header. Boards override it.",
            },
            SettingDeclaration {
                key: EMAIL_SIGNATURE,
                kind: SettingKind::Text { max_len: 1000 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Org email signature",
                description: "The workspace signature: it closes every outbound \
                             message, and it is the fallback for a member with no \
                             signature of their own. Placeholders like \
                             {{org.name}} and {{user.name}} fill in when the message \
                             sends. A board with its own signature takes precedence \
                             for that board's mail.",
            },
            SettingDeclaration {
                key: EMAIL_BRAND_NAME,
                kind: SettingKind::Text { max_len: 120 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Email brand name",
                description: "Wordmark shown in the header of notification and \
                             digest emails. Empty uses the organization name.",
            },
            SettingDeclaration {
                key: EMAIL_LOGO_URL,
                kind: SettingKind::Text { max_len: 500 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Email logo URL",
                description: "Optional https URL of a logo image shown in the \
                             branded email header. Empty renders the wordmark \
                             alone.",
            },
            SettingDeclaration {
                key: EMAIL_SMTP_HOST,
                kind: SettingKind::Text { max_len: 255 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "SMTP host",
                description: "Hostname of your SMTP relay (for example \
                             smtp.yourdomain.com). The SMTP password goes through \
                             the sealed slot, never this field.",
            },
            SettingDeclaration {
                key: EMAIL_SMTP_PORT,
                kind: SettingKind::Integer { min: 1, max: 65535 },
                default: SettingValue::Integer(587),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "SMTP port",
                description: "SMTP port: 587 for submission with STARTTLS, 25 for \
                             a plain relay, 465 where implicit TLS is required.",
            },
            SettingDeclaration {
                key: EMAIL_SMTP_USERNAME,
                kind: SettingKind::Text { max_len: 255 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "SMTP username",
                description: "Username for SMTP AUTH. Empty sends no authentication \
                             (relay must trust this instance by network). The \
                             password is sealed separately.",
            },
            SettingDeclaration {
                key: EMAIL_POSTMARK_BASE_URL,
                kind: SettingKind::Text { max_len: 255 },
                default: SettingValue::Text("https://api.postmarkapp.com".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Postmark API base URL",
                description: "Postmark API endpoint — keep the default unless you \
                             run a regional endpoint or a compatible proxy. The \
                             server token is sealed separately.",
            },
            SettingDeclaration {
                key: EMAIL_SES_REGION,
                kind: SettingKind::Text { max_len: 64 },
                default: SettingValue::Text("eu-west-1".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "AWS SES region",
                description: "The AWS region whose SES endpoint this organization \
                             sends through, e.g. eu-west-1. Not a preference: SES \
                             endpoints are region-scoped, so a wrong region sends to \
                             the wrong data centre. The access key pair is sealed \
                             separately and never readable.",
            },
            SettingDeclaration {
                key: EMAIL_SES_ENDPOINT,
                kind: SettingKind::Text { max_len: 255 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "SES endpoint override",
                description: "Leave empty to use the public regional SES endpoint. \
                             Set it only for a VPC interface endpoint or a \
                             private-link deployment.",
            },
            SettingDeclaration {
                key: EMAIL_DIGEST_POLICY,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("allowed".into()),
                scope: SettingScope::Org,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Daily digest policy",
                description: "Admin policy over the per-user daily digest: allowed \
                             lets members opt in, disabled stops digest email for \
                             the whole organization (deny wins over personal \
                             preferences).",
            },
            SettingDeclaration {
                key: EMAIL_DIGEST,
                kind: SettingKind::Enumeration,
                default: SettingValue::Text("off".into()),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Daily digest email",
                description: "One email a day summarizing your unread \
                             notifications, sent at your chosen hour in your own \
                             timezone. Scheduled — quiet hours do not suppress it.",
            },
            SettingDeclaration {
                key: EMAIL_DIGEST_HOUR,
                kind: SettingKind::Integer { min: 0, max: 23 },
                default: SettingValue::Integer(7),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Ui,
                label: "Digest send hour",
                description: "Local hour (0-23) the daily digest arrives, read in \
                             your notification timezone. Pick one outside quiet \
                             hours.",
            },
            SettingDeclaration {
                key: EMAIL_DIGEST_LAST_SENT,
                kind: SettingKind::Text { max_len: 40 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::User,
                section: SettingSection::Email,
                visibility: SettingVisibility::Hidden,
                label: "Digest last-sent marker",
                description: "Internal idempotency marker: local date of your last \
                             digest send. Written by the digest loop; not a \
                             preference.",
            },
            SettingDeclaration {
                key: CSAT_SEND_ON_RESOLVE,
                kind: SettingKind::Boolean,
                default: SettingValue::Boolean(false),
                scope: SettingScope::Org,
                section: SettingSection::Surveys,
                visibility: SettingVisibility::Ui,
                label: "CSAT survey on resolution",
                description: "Queue a satisfaction survey whenever a ticket \
                             resolves. Requires a configured mail transport — \
                             with transport set to none no surveys are sent \
                             and the scorecard section stays honestly empty.",
            },
            SettingDeclaration {
                key: CSAT_MIN_INTERVAL_HOURS,
                kind: SettingKind::Integer { min: 0, max: 8_760 },
                default: SettingValue::Integer(168),
                scope: SettingScope::Org,
                section: SettingSection::Surveys,
                visibility: SettingVisibility::Ui,
                label: "Minimum interval per client (hours)",
                description: "Skip a survey when this client already received \
                             one within the window. Default 168 = at most one \
                             survey per client per week.",
            },
            SettingDeclaration {
                key: CSAT_TOKEN_TTL_DAYS,
                kind: SettingKind::Integer { min: 1, max: 90 },
                default: SettingValue::Integer(14),
                scope: SettingScope::Org,
                section: SettingSection::Surveys,
                visibility: SettingVisibility::Ui,
                label: "Response link validity (days)",
                description: "How long a tokenized response link works after \
                             the survey email is sent. Links are single-use \
                             regardless of validity.",
            },
            SettingDeclaration {
                key: CSAT_QUIET_FROM,
                kind: SettingKind::Text { max_len: 5 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Surveys,
                visibility: SettingVisibility::Ui,
                label: "Quiet hours start",
                description: "Local HH:MM in the org timezone surveys defer \
                             to; empty means no quiet window.",
            },
            SettingDeclaration {
                key: CSAT_QUIET_UNTIL,
                kind: SettingKind::Text { max_len: 5 },
                default: SettingValue::Text(String::new()),
                scope: SettingScope::Org,
                section: SettingSection::Surveys,
                visibility: SettingVisibility::Ui,
                label: "Quiet hours end",
                description: "Local HH:MM in the org timezone surveys defer \
                             to; empty means no quiet window.",
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
                label: "DMS sync connector (IT Glue / Hudu ↔ Marula docs)",
                description: "Operator-side toggle for the documentation-system sync: a \
                              sync run that starts while this is off reports disabled \
                               before touching a single Marula record.",
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
        // --- Wave 26 lane 2 (`p26-contextual-help`): the help + tours
        // seam. Additive under the help.* / tours.* namespaces — the
        // in-flight onboarding lane owns onboarding.* exclusively.
        decls.push(SettingDeclaration {
            key: HELP_DOC_FEEDBACK,
            kind: SettingKind::Text { max_len: 4096 },
            default: SettingValue::Text(String::new()),
            scope: SettingScope::User,
            section: SettingSection::Onboarding,
            visibility: SettingVisibility::Hidden,
            label: "Docs feedback",
            description: "Was-this-helpful answers recorded from the in-app help \
                         panel, stored as a capped JSON log. Consumed by docs curation; \
                         written through the ordinary audited settings path.",
        });
        decls.push(SettingDeclaration {
            key: TOURS_DISABLED,
            kind: SettingKind::Boolean,
            default: SettingValue::Boolean(false),
            scope: SettingScope::User,
            section: SettingSection::Onboarding,
            visibility: SettingVisibility::Ui,
            label: "Don't show me tours",
            description: "Turns off every auto-started feature tour for this account. \
                         Help-panel guides and the docs site stay available.",
        });
        decls.push(SettingDeclaration {
            key: TOURS_COMPLETED,
            kind: SettingKind::Text { max_len: 4096 },
            default: SettingValue::Text(String::new()),
            scope: SettingScope::User,
            section: SettingSection::Onboarding,
            visibility: SettingVisibility::Hidden,
            label: "Completed tours",
            description: "Feature tours this account finished or skipped, stored as a \
                         capped JSON array of tour ids so a tour shows once per person.",
        });
        decls.push(SettingDeclaration {
            key: TOURS_ORG_DISABLED,
            kind: SettingKind::Boolean,
            default: SettingValue::Boolean(false),
            scope: SettingScope::Org,
            section: SettingSection::Onboarding,
            visibility: SettingVisibility::Ui,
            label: "Disable feature tours (org-wide)",
            description: "Admin switch that suppresses auto-started feature tours for \
                         the whole org; individual opt-outs ride the user-level \
                         don't-show-me-tours setting independently.",
        });

        // --- Wave 29 lane 1 (`p29-keyboard-shortcuts`): the keyboard
        // seam rides a NEW shortcuts.* namespace — it never touches
        // onboarding.* or any other lane's keys.
        decls.push(SettingDeclaration {
            key: SHORTCUTS_PROFILE,
            kind: SettingKind::Enumeration,
            default: SettingValue::Text("default".into()),
            scope: SettingScope::User,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Keyboard shortcuts",
            description: "How keyboard shortcuts behave for this account: the \
                         default profile keeps the Alt-command registry on \
                         (hold-Alt overlay + chords), 'all_off' disables chords \
                         and the hold-Alt overlay for readers and presenters. \
                         The ? cheat-sheet stays reachable either way.",
        });

        // --- Wave 32 lane 7 (`p32-dispatch-routing`): routing NONE-mode
        // table + travel reimbursement policy. The provider itself is
        // operator config (env), never a setting; these are org policy.
        decls.push(SettingDeclaration {
            key: ROUTING_HEURISTIC_MINUTES_PER_MILE,
            kind: SettingKind::Integer { min: 1, max: 60 },
            default: SettingValue::Integer(2),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Travel estimate: minutes per mile",
            description: "Pace the built-in travel estimate assumes when no routing \
                         provider is configured. Every estimate from this table is \
                         labeled heuristic; measured providers replace it.",
        });
        decls.push(SettingDeclaration {
            key: ROUTING_HEURISTIC_OVERHEAD_MINUTES,
            kind: SettingKind::Integer { min: 0, max: 120 },
            default: SettingValue::Integer(5),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Travel estimate: fixed overhead",
            description: "Minutes added to every built-in travel leg for parking and \
                         walk-in time, so blocks do not under-book short trips.",
        });
        decls.push(SettingDeclaration {
            key: ROUTING_HEURISTIC_ROAD_FACTOR_PERCENT,
            kind: SettingKind::Integer { min: 100, max: 300 },
            default: SettingValue::Integer(125),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Travel estimate: road factor %",
            description: "Percent applied to straight-line distance to approximate road \
                         distance in the built-in estimate. 100 assumes straight roads.",
        });
        decls.push(SettingDeclaration {
            key: TRAVEL_MILEAGE_RATE_MINOR,
            kind: SettingKind::Integer {
                min: 0,
                max: 100_000,
            },
            default: SettingValue::Integer(67),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Mileage reimbursement rate (minor units)",
            description: "Amount in the org's base currency (minor units) paid per \
                         mileage unit when travel drafts are reconciled and \
                         reimbursements are rolled up. 67 = 0.67 per mile.",
        });
        decls.push(SettingDeclaration {
            key: TRAVEL_MILEAGE_RATE_UNIT,
            kind: SettingKind::Enumeration,
            default: SettingValue::Text("mi".into()),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Mileage rate unit",
            description: "The distance unit the mileage reimbursement rate is quoted \
                         per. Logged distances in the other unit convert for the \
                         rollup (1 mi = 1.609344 km).",
        });
        decls.push(SettingDeclaration {
            key: TRAVEL_REIMBURSE_TECHS,
            kind: SettingKind::Boolean,
            default: SettingValue::Boolean(true),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Ui,
            label: "Reimburse technician mileage",
            description: "When on, submitted travel mileage accrues a reimbursement \
                         rollup owed to the technician; when off, mileage is \
                         informational and only billable amounts matter.",
        });
        decls.push(SettingDeclaration {
            key: WORKFLOW_DISABLED_NODE_KINDS,
            kind: SettingKind::Text { max_len: 4000 },
            default: SettingValue::Text(String::new()),
            scope: SettingScope::Org,
            section: SettingSection::General,
            visibility: SettingVisibility::Hidden,
            label: "Disabled workflow node kinds",
            description: "Workflow node kinds this org switched off, as a JSON array \
                         of node type ids (a comma-separated list is accepted too). \
                         Empty means every executable kind is available. Read by the \
                         workflow publish boundary and by the executor on every start \
                         and resume: a document that places a switched-off kind is \
                         refused, naming the node.",
        });

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
    fn shortcuts_seam_is_additive_and_well_scoped() {
        // Wave 29 lane 1 owns shortcuts.* exclusively; other namespaces
        // stay untouched.
        let d =
            declaration(SHORTCUTS_PROFILE).unwrap_or_else(|| panic!("shortcuts.profile declared"));
        assert!(SHORTCUTS_PROFILE.starts_with("shortcuts."));
        assert_eq!(d.scope, SettingScope::User);
        assert_eq!(d.visibility, SettingVisibility::Ui);
        assert_eq!(d.kind, SettingKind::Enumeration);
        // Default is valid per its own declaration.
        d.validate(&d.default.clone())
            .unwrap_or_else(|e| panic!("default invalid: {e}"));
        // Enumeration membership is enforced at the write gate.
        for good in ["default", "all_off"] {
            assert!(d.validate(&SettingValue::Text(good.into())).is_ok());
        }
        for bad in ["on", "custom", "", "All_Off"] {
            assert!(d.validate(&SettingValue::Text(bad.into())).is_err());
        }
        // The lane owns no other keys; nothing under shortcuts.* has been
        // reserved here twice (duplicates fail the registry test above).
        assert!(
            declarations()
                .iter()
                .filter(|x| x.key.starts_with("shortcuts."))
                .count()
                == 1
        );
    }

    #[test]
    fn the_record_clock_is_a_user_switch_and_defaults_on() {
        // Wave 50 lane T2: the second clock is a choice, but the choice starts
        // made — `DualTime` says nothing when the zones agree, so a reader in
        // one zone sees no change and a reader across zones sees the record's
        // clock on the day the project started asking for it.
        let d = declaration(GENERAL_SHOW_RECORD_TIME)
            .unwrap_or_else(|| panic!("{GENERAL_SHOW_RECORD_TIME} declared"));
        assert_eq!(d.scope, SettingScope::User);
        assert_eq!(d.kind, SettingKind::Boolean);
        assert_eq!(d.default, SettingValue::Boolean(true));
        assert_eq!(d.visibility, SettingVisibility::Ui);
        d.validate(&SettingValue::Boolean(false))
            .unwrap_or_else(|e| panic!("false must be storable: {e}"));
        // It is not org-wide: one member turning it off must not decide for
        // the rest of the team.
        assert_eq!(
            declarations()
                .iter()
                .filter(|x| x.key == GENERAL_SHOW_RECORD_TIME)
                .count(),
            1
        );
    }

    #[test]
    fn the_run_retention_window_is_a_bounded_org_setting() {
        // Wave 43 lane 14: runs are the highest-volume table the workflow
        // engine produces, so their window is a setting — bounded, so an
        // org cannot turn the ledger into an unbounded archive by typo.
        let d = declaration(WORKFLOW_RUN_RETENTION_DAYS)
            .unwrap_or_else(|| panic!("{WORKFLOW_RUN_RETENTION_DAYS} declared"));
        assert_eq!(d.scope, SettingScope::Org);
        assert_eq!(d.visibility, SettingVisibility::Ui);
        assert_eq!(
            d.default,
            SettingValue::Integer(crate::workflow_runs::RETENTION_DEFAULT_DAYS)
        );
        // The bounds are the same constants the run-ledger module clamps
        // with: the declaration and the reader cannot drift.
        let (min, max) = match &d.kind {
            SettingKind::Integer { min, max } => (*min, *max),
            other => panic!("retention must be an integer, got {}", other.label()),
        };
        assert_eq!(min, crate::workflow_runs::RETENTION_MIN_DAYS);
        assert_eq!(max, crate::workflow_runs::RETENTION_MAX_DAYS);
        assert!(d.validate(&SettingValue::Integer(30)).is_ok());
        assert!(d.validate(&SettingValue::Integer(1)).is_err());
        assert!(d.validate(&SettingValue::Integer(99_999)).is_err());
    }

    #[test]
    fn lookups_resolve() {
        assert!(declaration(GENERAL_EOD_HOUR).is_some());
        assert!(declaration("nope.nope").is_none());
        assert_eq!(
            default_value(APPEARANCE_THEME),
            Some(SettingValue::Text("marula".into()))
        );
        // Wave 24: per-user override + per-user mode (appearance.theme
        // is org-scope now): greeting, signoff, override, mode = 4.
        // Wave 26 lane 1: user onboarding adds four steps × done/dismissed = 8.
        // Wave 26 lane 2 added the help/tours seam: doc feedback,
        // tours.disabled, tours.completed = 16.
        // Wave 29 lane 1 added shortcuts.profile (user scope) = 17.
        // Wave 34 lane 8 added the digest trio (opt-in, hour, marker) = 20.
        // Wave 41 lane 2b added the two appearance seams (density, and the
        // data-surface document) = 22.
        // Wave 50 lane T2 added general.show_record_time (user scope) = 23.
        assert_eq!(declarations_for(SettingScope::User).len(), 23);
        // Tripwire count: bump deliberately when an org-scope setting is
        // added (Wave 26 lane 1: flows_disabled + six org steps ×
        // done/dismissed = 13, so 15 → 28; Wave 26 lane 2 added
        // tours.org_disabled — 28 → 29... plus the user/org banner
        // dismissed pair re-counted at 29 → 30. Wave 32 lane 7 added the
        // routing/travel policy block (three heuristic keys + mileage
        // rate, unit, and reimbursement toggle): 30 → 36. Wave 34 lane 8
        // added the twelve mail-config keys (transport, from/reply-to,
        // signature, brand, logo, smtp ×3, postmark base, digest policy):
        // 36 → 48. Wave 34 lane 2 added the five CSAT survey keys
        // (send-on-resolve, min interval, link TTL, quiet from/until):
        // 48 → 53. The Wave 34 claims close-out added the two SES
        // deployment keys (region, endpoint override) for the third
        // `MailTransport` choice: 53 → 55. Neither is a secret — the key
        // pair is sealed in the byok store — which is why they are ordinary
        // org settings rather than sealed credentials.
        // 55 → 57. Wave 42 added the org greeting and the org policy over
        // personal signatures, the two settings that make the resolution
        // chain governable (the personal signature itself is user scope).
        // 57 → 58. Wave 43 lane 14 added the workflow run-retention window:
        // runs are the highest-volume rows the workflow engine produces, so
        // their window is an org setting rather than an unbounded table.
        // 58 → 59. Wave 55 lane V7 added the inbound sender-binding level:
        // who may email a ticket, and how much authority the engine has over
        // a message from someone who is not on it.
        // 59 → 60. Wave 56 `p56-node-authority` added the workflow
        // node-kind switch: the kinds an org has taken out of service, read by
        // the publish boundary and by the executor's run-time re-check.
        assert_eq!(declarations_for(SettingScope::Org).len(), 60);
    }

    #[test]
    fn mail_config_seam_is_typed_and_secret_free() {
        // Transport selection is an enumeration; credentials never ride
        // settings (they live in the sealed slot).
        let d = declaration(EMAIL_TRANSPORT).unwrap();
        assert_eq!(d.scope, SettingScope::Org);
        assert_eq!(d.section, SettingSection::Email);
        for good in EMAIL_TRANSPORT_OPTIONS {
            assert!(d.validate(&SettingValue::Text((*good).into())).is_ok());
        }
        for bad in ["postfix", "Postmark", "", "smtp+starttls"] {
            assert!(d.validate(&SettingValue::Text(bad.into())).is_err());
        }
        // Digest knobs: org policy denies; per-user opt-in + hour.
        for (key, scope) in [
            (EMAIL_DIGEST_POLICY, SettingScope::Org),
            (EMAIL_DIGEST, SettingScope::User),
            (EMAIL_DIGEST_HOUR, SettingScope::User),
        ] {
            assert_eq!(declaration(key).unwrap().scope, scope, "{key}");
        }
        assert!(declaration(EMAIL_DIGEST_HOUR)
            .unwrap()
            .validate(&SettingValue::Integer(23))
            .is_ok());
        assert!(declaration(EMAIL_DIGEST_HOUR)
            .unwrap()
            .validate(&SettingValue::Integer(24))
            .is_err());
        // The last-sent marker is hidden state, not a preference.
        assert_eq!(
            declaration(EMAIL_DIGEST_LAST_SENT).unwrap().visibility,
            SettingVisibility::Hidden
        );
        // From/reply-to fields are text with the mailbox-length cap.
        for key in [EMAIL_FROM_ADDRESS, EMAIL_REPLY_TO] {
            let d = declaration(key).unwrap();
            assert!(d.validate(&SettingValue::Text("a@b.co".into())).is_ok());
            assert!(d.validate(&SettingValue::Text("x".repeat(255))).is_err());
        }
    }

    #[test]
    fn board_identity_keys_parse_only_known_fields() {
        let board = uuid::Uuid::now_v7();
        for field in BoardIdentityField::ALL {
            let key = board_identity_key(board, field);
            assert_eq!(board_identity_parts(&key), Some((board, field)), "{key}");
        }
        assert!(is_board_identity_key(&board_identity_key(
            board,
            BoardIdentityField::FromAddress
        )));
        // Unknown fields, missing parts, and non-uuid ids all miss.
        assert!(board_identity_parts(&format!("email.board.{board}.cc")).is_none());
        assert!(board_identity_parts("email.board.not-a-uuid.from_address").is_none());
        assert!(board_identity_parts("email.board.").is_none());
        assert!(!is_board_identity_key("email.from_address"));
    }

    #[test]
    fn help_and_tours_seam_is_additive_and_well_scoped() {
        // Wave 26 lane 2 owns help.* / tours.* exclusively; onboarding.*
        // stays lane 1's (p26-onboarding-flows) — no key overlap.
        for key in [
            HELP_DOC_FEEDBACK,
            TOURS_DISABLED,
            TOURS_COMPLETED,
            TOURS_ORG_DISABLED,
        ] {
            let d = declaration(key).unwrap_or_else(|| panic!("{key} declared"));
            assert!(key.starts_with("help.") || key.starts_with("tours."));
            assert!(!key.starts_with("onboarding."));
            assert_eq!(d.section, SettingSection::Onboarding);
            assert!(d.validate(&d.default.clone()).is_ok());
        }
        // The feedback and completion stores are hidden state (Text,
        // user scope); the two suppression switches are visible toggles.
        for key in [HELP_DOC_FEEDBACK, TOURS_COMPLETED] {
            let d = declaration(key).unwrap();
            assert_eq!(d.scope, SettingScope::User);
            assert_eq!(d.visibility, SettingVisibility::Hidden);
        }
        for key in [TOURS_DISABLED, TOURS_ORG_DISABLED] {
            let d = declaration(key).unwrap();
            assert_eq!(d.kind, SettingKind::Boolean);
            assert_eq!(d.visibility, SettingVisibility::Ui);
        }
        assert_eq!(
            declaration(TOURS_DISABLED).unwrap().scope,
            SettingScope::User
        );
        assert_eq!(
            declaration(TOURS_ORG_DISABLED).unwrap().scope,
            SettingScope::Org
        );
    }

    /// The three levels of triage are a per-org setting with the strictest
    /// one as its default (Wave 55, decision 4). The vocabularies the code
    /// accepts and the ruling names have to be the same three strings.
    #[test]
    fn inbound_sender_binding_defaults_to_human_only_and_names_three_levels() {
        let decl = declaration(EMAIL_INBOUND_SENDER_BINDING).expect("declared");
        assert_eq!(decl.scope, SettingScope::Org);
        assert_eq!(decl.section, SettingSection::Email);
        assert!(matches!(decl.kind, SettingKind::Enumeration));
        assert_eq!(
            decl.default,
            SettingValue::Text("human_only".into()),
            "a value nobody chose is not authority for the engine"
        );
        assert_eq!(
            decl.options(),
            Some(EMAIL_INBOUND_SENDER_BINDING_OPTIONS),
            "the only levels the code accepts are the three the ruling names"
        );
        for level in ["human_only", "shadow", "autonomous"] {
            assert!(
                decl.validate(&SettingValue::Text(level.into())).is_ok(),
                "{level} must be accepted"
            );
        }
        // Anything else is refused rather than read as a grant: the pipeline
        // falls back to the default for a value it cannot name, and this
        // declaration is what keeps an unknown value from being stored.
        assert!(decl
            .validate(&SettingValue::Text("automatic".into()))
            .is_err());
        assert!(decl.validate(&SettingValue::Boolean(true)).is_err());
    }

    #[test]
    fn email_greeting_and_signoff_are_user_scope_text_settings() {
        for key in [EMAIL_GREETING, EMAIL_SIGNOFF] {
            let d = declaration(key).unwrap_or_else(|| panic!("{key} declared"));
            assert_eq!(d.scope, SettingScope::User);
            assert_eq!(d.section, SettingSection::Email);
            assert!(d.validate(&SettingValue::Text("Hi {name},".into())).is_ok());
            assert!(d.validate(&SettingValue::Integer(1)).is_err());
            let max_len = match &d.kind {
                SettingKind::Text { max_len } => *max_len,
                other => panic!("{key} is not text: {other:?}"),
            };
            assert!(d.validate(&SettingValue::Text("x".repeat(max_len))).is_ok());
            assert!(d
                .validate(&SettingValue::Text("x".repeat(max_len + 1)))
                .is_err());
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
    fn density_seam_is_user_scope_and_defaults_to_comfortable() {
        let decl = declaration(APPEARANCE_DENSITY).expect("density is declared");
        assert_eq!(decl.scope, SettingScope::User);
        assert_eq!(decl.section, SettingSection::Appearance);
        assert_eq!(decl.visibility, SettingVisibility::Ui);
        assert_eq!(decl.default, SettingValue::Text("comfortable".into()));
        assert_eq!(decl.options(), Some(DENSITY_OPTIONS));
        for step in ["comfortable", "compact", "dense"] {
            assert!(
                decl.validate(&SettingValue::Text(step.into())).is_ok(),
                "{step}"
            );
        }
        for rejected in ["cosy", "Comfortable", "tight", ""] {
            assert!(
                decl.validate(&SettingValue::Text(rejected.into())).is_err(),
                "{rejected}"
            );
        }
        assert!(
            decl.validate(&SettingValue::Integer(1)).is_err(),
            "density is a name, not a number"
        );
        // The Marula default is the roomiest step, not the densest: the whole
        // point of D7 is that comfortable is what a new account lands on.
        assert_eq!(
            default_value(APPEARANCE_DENSITY),
            Some(SettingValue::Text("comfortable".into()))
        );
    }

    #[test]
    fn data_surface_document_is_user_scope_bounded_and_not_ui_listed() {
        let decl = declaration(APPEARANCE_DATA_SURFACES).expect("the D12 seam is declared");
        assert_eq!(decl.scope, SettingScope::User);
        assert_eq!(decl.section, SettingSection::Appearance);
        // Hidden: the four controls own it, so it must not appear in the
        // generated settings hub as a field a user edits by hand.
        assert_eq!(decl.visibility, SettingVisibility::Hidden);
        assert_eq!(decl.default, SettingValue::Text("{}".into()));
        assert_eq!(
            decl.kind,
            SettingKind::Text {
                max_len: crate::data_surface::MAX_DOCUMENT_CHARS
            }
        );
        // The declared cap and the parser's cap are one number, or a document
        // could be accepted on write and refused on read.
        let parsed = crate::data_surface::DataSurfacePrefs::parse("{}").unwrap();
        assert_eq!(parsed.to_compact_string().unwrap(), "{}");
        let over = "x".repeat(crate::data_surface::MAX_DOCUMENT_CHARS + 1);
        assert!(decl.validate(&SettingValue::Text(over.clone())).is_err());
        assert!(crate::data_surface::DataSurfacePrefs::parse(&over).is_err());
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

    /// Wave 42: the greeting/signature family and the policy over personal
    /// signatures. The scopes and the option list are pinned here because
    /// the resolver's two personal rungs are gated on the policy — a key
    /// that drifted to the wrong scope, or an option list that grew a third
    /// name the migration's CHECK does not know, would fail open.
    #[test]
    fn wave42_signature_family_declares_the_scopes_the_resolver_assumes() {
        let greeting = declaration(EMAIL_ORG_GREETING).unwrap();
        assert_eq!(greeting.scope, SettingScope::Org);
        assert_eq!(greeting.section, SettingSection::Email);
        assert_eq!(greeting.visibility, SettingVisibility::Ui);
        assert!(greeting
            .validate(&SettingValue::Text("Hello from Acme,".into()))
            .is_ok());

        let policy = declaration(EMAIL_SIGNATURE_POLICY).unwrap();
        assert_eq!(policy.scope, SettingScope::Org);
        assert_eq!(policy.kind, SettingKind::Enumeration);
        assert_eq!(policy.default, SettingValue::Text("allowed".into()));
        for allowed in EMAIL_SIGNATURE_POLICY_OPTIONS {
            assert!(policy
                .validate(&SettingValue::Text((*allowed).into()))
                .is_ok());
        }
        assert!(policy
            .validate(&SettingValue::Text("org_locked_or_not".into()))
            .is_err());
        // Exactly the two names the migration 0118 CHECK lists — one list,
        // quoted in three places, and this test is what makes the drift
        // loud rather than silent.
        assert_eq!(EMAIL_SIGNATURE_POLICY_OPTIONS, &["allowed", "org_locked"]);

        // The personal signature is the pre-existing per-user key (Wave 12
        // lane 6): same key, wider cap, now a signature rather than a plain
        // sign-off. Keeping the key is what stops the composers that already
        // read it from orphaning a stored value.
        let personal = declaration(EMAIL_SIGNOFF).unwrap();
        assert_eq!(personal.scope, SettingScope::User);
        assert_eq!(personal.kind, SettingKind::Text { max_len: 1000 });
        assert!(personal
            .validate(&SettingValue::Text("x".repeat(1000)))
            .is_ok());
        assert!(personal
            .validate(&SettingValue::Text("x".repeat(1001)))
            .is_err());

        // The org signature keeps its key and its cap; only its role changed
        // (it is now the fallback for a member with none of their own).
        let org = declaration(EMAIL_SIGNATURE).unwrap();
        assert_eq!(org.scope, SettingScope::Org);
        assert_eq!(org.kind, SettingKind::Text { max_len: 1000 });
    }

    #[test]
    fn hidden_settings_exist_for_state_flags() {
        let d = declaration(ONBOARDING_WIZARD_COMPLETED).unwrap();
        assert_eq!(d.visibility, SettingVisibility::Hidden);
        assert_eq!(d.scope, SettingScope::Org);
        assert!(d.validate(&SettingValue::Boolean(true)).is_ok());
    }
}
