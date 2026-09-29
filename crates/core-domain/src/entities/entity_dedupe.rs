//! Cross-entity dedupe (ADR 0003 extension, Wave 22 lane 2): the
//! source-link + identity-key + merge-candidate pattern applied to
//! clients/contacts. Pure domain logic only — no I/O.
//!
//! Contacts carry the SAME tier mechanics as config items (CI5): observed
//! fields churn with their feed, curated fields are human-set and sticky.
//! Client candidates are ORG MAP RECONCILIATION: a vendor organization
//! that no operator mapping resolves gets deterministic label-match
//! proposals as reviewable candidates — never a silent auto-map, and
//! never a guessed client attribution (the adapters-core org_map
//! posture).

use crate::ids::{OrgId, UserId};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

pub type EntitySourceLinkId = Uuid;
pub type EntityMergeCandidateId = Uuid;
pub type EntityMergeId = Uuid;
pub type ClientOrgMapId = Uuid;

/// The entity family a dedupe record binds. Clients ride reconciliation
/// only (mapping decisions, not folds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DedupeEntityKind {
    Contact,
    Client,
}

impl DedupeEntityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DedupeEntityKind::Contact => "contact",
            DedupeEntityKind::Client => "client",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "contact" => DedupeEntityKind::Contact,
            "client" => DedupeEntityKind::Client,
            _ => return None,
        })
    }
}

/// Candidate lifecycle — mirrors `CiMergeStatus` (same queue mechanics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EntityMergeStatus {
    Pending,
    AutoMerged,
    Accepted,
    Rejected,
    Deferred,
}

impl EntityMergeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityMergeStatus::Pending => "pending",
            EntityMergeStatus::AutoMerged => "auto_merged",
            EntityMergeStatus::Accepted => "accepted",
            EntityMergeStatus::Rejected => "rejected",
            EntityMergeStatus::Deferred => "deferred",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => EntityMergeStatus::Pending,
            "auto_merged" => EntityMergeStatus::AutoMerged,
            "accepted" => EntityMergeStatus::Accepted,
            "rejected" => EntityMergeStatus::Rejected,
            "deferred" => EntityMergeStatus::Deferred,
            _ => return None,
        })
    }
}

/// A vendor record link for a contact (the CI2 link key generalized).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntitySourceLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: EntitySourceLinkId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub entity_kind: DedupeEntityKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub entity_id: Uuid,
    pub source_system: String,
    pub external_id: String,
    #[serde(default)]
    pub source_guid: Option<String>,
    #[serde(default)]
    pub payload_ref: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reviewable client/contact dedupe candidate (ADR 0003 visible
/// evidence). For contact candidates both ids are set; for client
/// reconciliation the candidate side is the VENDOR ORG
/// (`vendor_org_id`/`vendor_label`) and `candidate_entity_id` is NULL.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntityMergeCandidate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: EntityMergeCandidateId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub entity_kind: DedupeEntityKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_entity_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub candidate_entity_id: Option<Uuid>,
    #[serde(default)]
    pub source_system: Option<String>,
    #[serde(default)]
    pub vendor_org_id: Option<String>,
    #[serde(default)]
    pub vendor_label: Option<String>,
    pub confidence: f64,
    pub reason: String,
    #[serde(default)]
    pub evidence: Value,
    pub status: EntityMergeStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    #[serde(default)]
    pub decided_by: Option<UserId>,
    #[serde(default)]
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An audited, undoable contact merge (the `ci_merges` split substrate).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EntityMerge {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: EntityMergeId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub entity_kind: DedupeEntityKind,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_entity_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub merged_entity_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub candidate_id: Option<EntityMergeCandidateId>,
    pub confidence: Option<f64>,
    pub reason: String,
    #[serde(default)]
    pub evidence: Value,
    #[serde(default)]
    pub moved: Value,
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub merged_by: Option<UserId>,
    pub merged_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub undone_by: Option<UserId>,
    pub undone_at: Option<DateTime<Utc>>,
}

/// The durable operator mapping a reconciliation accept writes: one row
/// per (source system, vendor org id), pointing at one Marula client.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientOrgMap {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ClientOrgMapId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub source_system: String,
    pub vendor_org_id: String,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: Uuid,
    #[serde(default)]
    pub vendor_label: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---- deterministic contact identity ----------------------------------------

/// Lowercases and trims an email; returns `None` for empty input.
pub fn normalize_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_ascii_lowercase();
    if e.is_empty() || !e.contains('@') {
        return None;
    }
    Some(e)
}

/// Digits only, dropping a leading country `1` for NANP-style input, so
/// `(555) 010-1234` and `+1 555 010 1234` compare equal. Returns `None`
/// when fewer than 7 digits remain (not a matchable phone).
pub fn normalize_phone(raw: &str) -> Option<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 7 {
        return None;
    }
    let trimmed = if digits.len() > 7 && digits.starts_with('1') {
        digits[1..].to_string()
    } else {
        digits
    };
    Some(trimmed)
}

/// The deterministic identity signals a contact feed declares.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContactIdentityKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
}

impl ContactIdentityKeys {
    pub fn from_value(v: &Value) -> Self {
        let s = |k: &str| -> Option<String> {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
        };
        ContactIdentityKeys {
            email: s("email").and_then(|x| normalize_email(&x)),
            phone: s("phone")
                .or_else(|| s("phone_number"))
                .and_then(|x| normalize_phone(&x)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.phone.is_none()
    }
}

/// Deterministic contact matching (ADR 0003: deterministic first). Email
/// is the strong personal identity (0.90 — same mailbox, same person);
/// a phone alone is a household/business-shared signal and stays
/// reviewable (0.75 — never auto-merged at the shipped 0.85 threshold);
/// BOTH signals agreeing is near-decisive (0.95).
pub fn match_contact_identity(
    a: &ContactIdentityKeys,
    b: &ContactIdentityKeys,
) -> Option<crate::entities::config_identity::IdentityMatch> {
    use crate::entities::config_identity::IdentityMatch;

    let mut signals: Vec<(f64, String, Value)> = Vec::new();
    if let (Some(x), Some(y)) = (&a.email, &b.email) {
        if x == y {
            signals.push((0.90, format!("email match {x}"), json!({ "email": x })));
        }
    }
    if let (Some(x), Some(y)) = (&a.phone, &b.phone) {
        if x == y {
            signals.push((0.75, format!("phone match +{x}"), json!({ "phone": x })));
        }
    }
    if signals.is_empty() {
        return None;
    }
    signals.sort_by(|p, q| q.0.partial_cmp(&p.0).unwrap_or(std::cmp::Ordering::Equal));
    let (best, reason, _) = signals[0].clone();
    let combined = signals
        .get(1)
        .map(|(c, _, _)| (best + c).min(0.95))
        .unwrap_or(best);
    let all_evidence: Vec<Value> = signals.iter().map(|(_, _, e)| e.clone()).collect();
    Some(IdentityMatch {
        confidence: (combined * 1000.0).round() / 1000.0,
        reason,
        evidence: json!({ "signals": all_evidence }),
    })
}

// ---- the Jev intelligent-dedupe workflow (Wave 22 lane 2 scope addition) ----

/// What an automated pass DOES with its above-threshold suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DedupeRunMode {
    /// Every decision stays suggestion-grade; the workbench carries the
    /// answers inline and humans decide.
    ReportOnly,
    /// Suggestions at/above the workflow threshold fold automatically
    /// (audited + undoable exactly like a human accept); everything
    /// below stays queued for review.
    AutoMerge,
}

impl DedupeRunMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            DedupeRunMode::ReportOnly => "report_only",
            DedupeRunMode::AutoMerge => "auto_merge",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "report_only" => DedupeRunMode::ReportOnly,
            "auto_merge" => DedupeRunMode::AutoMerge,
            _ => return None,
        })
    }
}

/// The queue families a dedupe pass covers.
pub const DEDUPE_QUEUE_CONFIG_ITEMS: &str = "config_items";
pub const DEDUPE_QUEUE_CONTACTS: &str = "contacts";
pub const DEDUPE_QUEUE_CLIENTS: &str = "clients";

/// One org's Jev dedupe workflow (the `dedupe_workflows` row).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DedupeWorkflow {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub enabled: bool,
    /// The workflow's own confidence dial: auto_merge folds only at/above
    /// this; everything below stays queued for the workbench.
    pub confidence_threshold: f64,
    pub mode: DedupeRunMode,
    /// Scheduled cadence; None = manual runs only (the workbench trigger).
    pub interval_hours: Option<i32>,
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub notify_user_ids: Vec<UserId>,
    pub entity_kinds: Vec<String>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_result: Value,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub updated_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Why a workflow was refused (typed, never silent) — the API maps these
/// onto envelope codes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DedupeWorkflowRefusal {
    /// The org's decision model is not configured — the honest
    /// unconfigured posture (ADR 0011 §3): nothing turns on quietly.
    DecisionNotConfigured,
    InvalidDedupeEntityKinds,
    ThresholdOutOfRange,
    ScheduleOutOfRange,
}

impl DedupeWorkflowRefusal {
    pub const fn as_str(self) -> &'static str {
        match self {
            DedupeWorkflowRefusal::DecisionNotConfigured => "dedupe_decision_not_configured",
            DedupeWorkflowRefusal::InvalidDedupeEntityKinds => "dedupe_invalid_entity_kinds",
            DedupeWorkflowRefusal::ThresholdOutOfRange => "dedupe_threshold_out_of_range",
            DedupeWorkflowRefusal::ScheduleOutOfRange => "dedupe_schedule_out_of_range",
        }
    }
}

/// Validates a candidate workflow payload. The decision-model gate is a
/// parameter because only the API layer can query the live provider
/// config; everything else is pure domain law.
pub fn validate_dedupe_workflow(
    confidence_threshold: f64,
    entity_kinds: &[String],
    interval_hours: Option<i32>,
    decision_configured: bool,
) -> Result<(), DedupeWorkflowRefusal> {
    if !(0.0..=1.0).contains(&confidence_threshold) {
        return Err(DedupeWorkflowRefusal::ThresholdOutOfRange);
    }
    if entity_kinds.is_empty()
        || entity_kinds.iter().any(|k| {
            !matches!(
                k.as_str(),
                DEDUPE_QUEUE_CONFIG_ITEMS | DEDUPE_QUEUE_CONTACTS | DEDUPE_QUEUE_CLIENTS
            )
        })
    {
        return Err(DedupeWorkflowRefusal::InvalidDedupeEntityKinds);
    }
    if let Some(h) = interval_hours {
        if !(1..=720).contains(&h) {
            return Err(DedupeWorkflowRefusal::ScheduleOutOfRange);
        }
    }
    if !decision_configured {
        return Err(DedupeWorkflowRefusal::DecisionNotConfigured);
    }
    Ok(())
}

/// One pass's outcome — the run report (persisted on the workflow row and
/// delivered to the designated users as a `dedupe_report` notification).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DedupeRunReport {
    pub queues_scanned: Vec<String>,
    pub scanned: usize,
    pub asked: usize,
    /// Suggestions whose answer sat at/above the workflow threshold.
    pub above_threshold: usize,
    pub auto_merged: usize,
    /// The asks the pass would have fired next run (the resource-efficiency
    /// cap applies per run; a truncated pass is VISIBLE, never silent).
    pub truncated: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub started_at: chrono::DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub finished_at: DateTime<Utc>,
    /// Per-candidate outcomes for the report/notification body.
    pub outcomes: Vec<DedupeRunOutcome>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DedupeRunOutcome {
    /// "config_items" | "contacts" | "clients"
    pub queue: String,
    pub candidate_id: Uuid,
    /// Human label pair ("A" ← "B") for the report body.
    pub label: String,
    /// The recorded answer's probability/noul number, when one came back.
    pub confidence: Option<f64>,
    /// "auto_merged" | "suggested" | "below_threshold" | "quiet"
    pub disposition: String,
}

// ---- client org_map reconciliation proposals -------------------------------

/// One deterministic proposal linking a vendor org label to an existing
/// Marula client (reviewable, never auto-applied).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OrgReconcileProposal {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: Uuid,
    pub client_name: String,
    pub confidence: f64,
    pub reason: String,
}

/// Folds a label to its comparable form: lowercase, trimmed, whitespace
/// collapsed, common corporate suffixes dropped.
pub fn normalize_client_label(raw: &str) -> String {
    let mut out = String::new();
    let mut last_space = false;
    let lowered = raw.trim().to_ascii_lowercase();
    for ch in lowered.chars() {
        if ch.is_whitespace() || ch == '-' || ch == '_' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else if ch.is_alphanumeric() || ch == '.' {
            out.push(ch);
            last_space = false;
        }
    }
    let suffixes = [
        " inc", " llc", " ltd", " corp", " co", " llp", " pty", " gmbh", " plc",
    ];
    let mut s = out.trim().trim_end_matches(['.', ',']).to_string();
    for suf in suffixes {
        if s.ends_with(suf) {
            s = s[..s.len() - suf.len()]
                .trim()
                .trim_end_matches(['.', ','])
                .to_string();
            break;
        }
    }
    s
}

/// Deterministic org→client proposals: exact case-insensitive label match
/// (0.90) and normalized-name match after suffix folding (0.85). Both are
/// reviewable by construction — reconciliation is NEVER auto-applied
/// (an operator's mapping decision is an audited accept on the queue).
/// At most one proposal per candidate pair; the caller queues the best.
pub fn propose_client_matches(
    vendor_label: &str,
    clients: &[(Uuid, String)],
) -> Vec<OrgReconcileProposal> {
    let norm = normalize_client_label(vendor_label);
    if norm.is_empty() {
        return Vec::new();
    }
    let mut proposals: Vec<OrgReconcileProposal> = Vec::new();
    for (id, name) in clients {
        if name.trim().eq_ignore_ascii_case(vendor_label.trim()) {
            proposals.push(OrgReconcileProposal {
                client_id: *id,
                client_name: name.clone(),
                confidence: 0.90,
                reason: format!("vendor org label matches client name `{name}`"),
            });
        } else if normalize_client_label(name) == norm && !norm.is_empty() {
            proposals.push(OrgReconcileProposal {
                client_id: *id,
                client_name: name.clone(),
                confidence: 0.85,
                reason: format!("normalized name match `{name}`"),
            });
        }
    }
    proposals.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    proposals.truncate(1);
    proposals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization_is_deterministic() {
        assert_eq!(
            normalize_email("  Jane.Doe@Example.COM "),
            Some("jane.doe@example.com".to_string())
        );
        assert_eq!(normalize_email("not-an-email"), None);
        assert_eq!(normalize_email("   "), None);
    }

    #[test]
    fn phone_normalization_folds_formats() {
        assert_eq!(
            normalize_phone("(555) 010-1234"),
            normalize_phone("+1 555 010 1234")
        );
        assert_eq!(normalize_phone("555-0101"), Some("5550101".to_string()));
        assert_eq!(normalize_phone("no digits"), None);
    }

    #[test]
    fn email_plus_phone_is_near_decisive_phone_alone_stays_reviewable() {
        let a = ContactIdentityKeys::from_value(
            &json!({ "email": "A@B.co", "phone": "(555) 010-1234" }),
        );
        let b =
            ContactIdentityKeys::from_value(&json!({ "email": "a@b.co", "phone": "+15550101234" }));
        let m = match_contact_identity(&a, &b).unwrap();
        assert!(m.confidence >= 0.95);

        let c = ContactIdentityKeys::from_value(&json!({ "phone": "555.010.1234" }));
        let m2 = match_contact_identity(&a, &c).unwrap();
        assert!(m2.confidence < 0.85, "phone alone must never auto-merge");
    }

    #[test]
    fn disjoint_keys_are_no_match() {
        let a = ContactIdentityKeys::from_value(&json!({ "email": "a@b.co" }));
        let b = ContactIdentityKeys::from_value(&json!({ "email": "c@d.co", "phone": "5550101" }));
        assert!(match_contact_identity(&a, &b).is_none());
    }

    #[test]
    fn proposals_prefer_exact_and_never_multi() {
        let clients = vec![
            (Uuid::from_u128(1), "Acme Corp".to_string()),
            (Uuid::from_u128(2), "Acme Corporation, Inc.".to_string()),
        ];
        let exact = propose_client_matches("Acme Corp", &clients);
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].client_id, Uuid::from_u128(1));
        assert!(exact[0].confidence >= 0.90);

        let folded = propose_client_matches("ACME CORPORATION LLC", &clients);
        assert_eq!(folded.len(), 1);
        assert!(folded[0].confidence >= 0.85);

        assert!(propose_client_matches("Totally Different", &clients).is_empty());
        assert!(propose_client_matches("   ", &clients).is_empty());
    }

    #[test]
    fn corporate_suffixes_fold_to_the_same_label() {
        assert_eq!(
            normalize_client_label("Acme Corporation, Inc."),
            normalize_client_label("acme corporation")
        );
        assert_eq!(
            normalize_client_label("Northwind Traders Ltd."),
            "northwind traders"
        );
    }

    #[test]
    fn entity_kinds_round_trip() {
        assert_eq!(
            DedupeEntityKind::parse("contact"),
            Some(DedupeEntityKind::Contact)
        );
        assert_eq!(
            DedupeEntityKind::parse("client"),
            Some(DedupeEntityKind::Client)
        );
        assert_eq!(DedupeEntityKind::parse("device"), None);
        assert_eq!(
            EntityMergeStatus::parse("deferred"),
            Some(EntityMergeStatus::Deferred)
        );
    }
}
