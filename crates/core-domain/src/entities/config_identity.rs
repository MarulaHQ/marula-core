//! Configuration identity entities + deterministic matching (ADR 0003,
//! CI1/CI3/CI4/CI5/CI6). Pure domain logic only — no I/O.
//!
//! The canonical config item IS a `device_assets` row (no parallel instance
//! registry); the CI facets and lifecycle columns are declared on
//! `DeviceAsset` in `client.rs`. This module owns:
//!   * the source-link / interface / merge-queue / precedence shapes,
//!   * CI6 semantic classes bound to `document_layouts`,
//!   * CI3 deterministic identity keys + match confidence,
//!   * CI5 curated-vs-observed overlay + shipped precedence defaults.

use crate::error::DomainError;
use crate::ids::{
    CiFieldPrecedenceId, CiInterfaceId, CiMergeCandidateId, CiMergeId, ConfigSourceLinkId,
    DeviceAssetId, DocumentLayoutId, OrgId, UserId,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// CI6: a semantic class is the behavior-bearing identity of a CI type.
/// Layouts (types) are org-configurable and renameable; the class is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CiSemanticClass {
    Workstation,
    Server,
    Vm,
    NetworkAppliance,
    Printer,
    Iot,
}

impl CiSemanticClass {
    pub const ALL: [CiSemanticClass; 6] = [
        CiSemanticClass::Workstation,
        CiSemanticClass::Server,
        CiSemanticClass::Vm,
        CiSemanticClass::NetworkAppliance,
        CiSemanticClass::Printer,
        CiSemanticClass::Iot,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            CiSemanticClass::Workstation => "workstation",
            CiSemanticClass::Server => "server",
            CiSemanticClass::Vm => "vm",
            CiSemanticClass::NetworkAppliance => "network_appliance",
            CiSemanticClass::Printer => "printer",
            CiSemanticClass::Iot => "iot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "workstation" => CiSemanticClass::Workstation,
            "server" => CiSemanticClass::Server,
            "vm" => CiSemanticClass::Vm,
            "network_appliance" => CiSemanticClass::NetworkAppliance,
            "printer" => CiSemanticClass::Printer,
            "iot" => CiSemanticClass::Iot,
            _ => return None,
        })
    }
}

impl std::str::FromStr for CiSemanticClass {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        CiSemanticClass::parse(s)
            .ok_or_else(|| DomainError::Validation(format!("unknown semantic class '{s}'")))
    }
}

/// CI5: which tier a field value belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CiFieldTier {
    Observed,
    Curated,
}

impl CiFieldTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            CiFieldTier::Observed => "observed",
            CiFieldTier::Curated => "curated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "observed" => CiFieldTier::Observed,
            "curated" => CiFieldTier::Curated,
            _ => return None,
        })
    }
}

/// CI4: lifecycle of a reviewable merge candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CiMergeStatus {
    Pending,
    AutoMerged,
    Accepted,
    Rejected,
}

impl CiMergeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            CiMergeStatus::Pending => "pending",
            CiMergeStatus::AutoMerged => "auto_merged",
            CiMergeStatus::Accepted => "accepted",
            CiMergeStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => CiMergeStatus::Pending,
            "auto_merged" => CiMergeStatus::AutoMerged,
            "accepted" => CiMergeStatus::Accepted,
            "rejected" => CiMergeStatus::Rejected,
            _ => return None,
        })
    }
}

/// A feeder's per-source record link (ADR 0003). 0..N per config item; the
/// unique key is `(org, source_system, external_id)`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConfigSourceLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: ConfigSourceLinkId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub config_item_id: DeviceAssetId,
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

impl Default for ConfigSourceLink {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            config_item_id: Uuid::nil(),
            source_system: String::new(),
            external_id: String::new(),
            source_guid: None,
            payload_ref: None,
            last_seen_at: now,
            created_at: now,
            updated_at: now,
        }
    }
}

/// A NIC attribute of a config item (ADR 0003 lifecycle rule).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CiInterface {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: CiInterfaceId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub config_item_id: DeviceAssetId,
    #[serde(default)]
    pub name: Option<String>,
    /// Normalized lower colon form, e.g. `aa:bb:cc:dd:ee:ff`.
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default)]
    pub ips: Vec<String>,
    #[serde(default)]
    pub vlans: Vec<String>,
    #[serde(default)]
    pub is_primary: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    #[serde(default)]
    pub source_link_id: Option<ConfigSourceLinkId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reviewable ambiguous match (ADR 0003): visible evidence, one-click
/// accept/reject, auto-merge only above the org threshold.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CiMergeCandidate {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: CiMergeCandidateId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub candidate_config_item_id: DeviceAssetId,
    pub confidence: f64,
    pub reason: String,
    #[serde(default)]
    pub evidence: Value,
    pub status: CiMergeStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    #[serde(default)]
    pub decided_by: Option<UserId>,
    #[serde(default)]
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An audited, undoable merge (the split substrate).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CiMerge {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: CiMergeId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub primary_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub merged_config_item_id: DeviceAssetId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub candidate_id: Option<CiMergeCandidateId>,
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

/// A per-type/per-source precedence override (CI5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CiFieldPrecedence {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: CiFieldPrecedenceId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub layout_id: Option<DocumentLayoutId>,
    pub semantic_class: Option<CiSemanticClass>,
    pub field_key: String,
    pub source_system: String,
    pub priority: i16,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---- CI3: deterministic identity keys --------------------------------------

/// The deterministic identity signals a feeder can declare. Normalized on
/// construction so comparisons are stable.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IdentityKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

/// Uppercases and strips all non-alphanumerics (serial numbers are printed
/// with dashes/spaces that vary between feeds).
pub fn normalize_serial(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// Normalizes a MAC to lower colon form; returns `None` for non-MAC input.
pub fn normalize_mac(raw: &str) -> Option<String> {
    let hex: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if hex.len() != 12 {
        return None;
    }
    Some(
        hex.as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// Lowercases, trims, and strips a trailing dot from a hostname.
pub fn normalize_hostname(raw: &str) -> String {
    raw.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Lowercases/trims a domain.
pub fn normalize_domain(raw: &str) -> String {
    raw.trim().trim_matches('.').to_ascii_lowercase()
}

impl IdentityKeys {
    /// Builds keys from arbitrary feed input, normalizing every signal.
    pub fn from_value(v: &Value) -> Self {
        let obj = v.as_object();
        let s = |k: &str| -> Option<String> {
            obj.and_then(|o| o.get(k))
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
        };
        let mut macs: Vec<String> = Vec::new();
        if let Some(arr) = obj.and_then(|o| o.get("macs")).and_then(|x| x.as_array()) {
            for m in arr {
                if let Some(m) = m.as_str().and_then(normalize_mac) {
                    if !macs.contains(&m) {
                        macs.push(m);
                    }
                }
            }
        }
        if let Some(m) = s("mac").and_then(|m| normalize_mac(&m)) {
            if !macs.contains(&m) {
                macs.push(m);
            }
        }
        macs.sort();
        IdentityKeys {
            source_guid: s("source_guid"),
            serial: s("serial")
                .map(|x| normalize_serial(&x))
                .filter(|x| !x.is_empty()),
            service_tag: s("service_tag")
                .map(|x| normalize_serial(&x))
                .filter(|x| !x.is_empty()),
            macs,
            hostname: s("hostname")
                .map(|x| normalize_hostname(&x))
                .filter(|x| !x.is_empty()),
            domain: s("domain")
                .map(|x| normalize_domain(&x))
                .filter(|x| !x.is_empty()),
        }
    }

    /// Projects the fields object into identity keys (feed convenience):
    /// explicit `identity_keys` take precedence over layout fields.
    pub fn from_fields(fields: &Value, explicit: Option<&Value>) -> Self {
        let mut merged = fields.clone();
        if let Some(Value::Object(explicit)) = explicit {
            if let Value::Object(base) = &mut merged {
                for (k, v) in explicit {
                    base.insert(k.clone(), v.clone());
                }
            } else {
                merged = Value::Object(explicit.clone());
            }
        }
        let mut keys = IdentityKeys::from_value(&merged);
        // Field aliases used by the seeded layouts.
        if keys.serial.is_none() {
            keys.serial = merged
                .get("serial_number")
                .and_then(|v| v.as_str())
                .map(normalize_serial)
                .filter(|s| !s.is_empty());
        }
        keys
    }

    pub fn is_empty(&self) -> bool {
        self.source_guid.is_none()
            && self.serial.is_none()
            && self.service_tag.is_none()
            && self.macs.is_empty()
            && self.hostname.is_none()
            && self.domain.is_none()
    }
}

/// CI3: one deterministic match with human-verifiable evidence.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IdentityMatch {
    pub confidence: f64,
    pub reason: String,
    pub evidence: Value,
}

/// Deterministic matching first (ADR 0003). Scores are capped by the
/// strongest signal; ties combine the best two. A source GUID is decisive
/// (1.0), an exact serial/service tag is near-decisive, an exact MAC set or
/// hostname+domain is a strong-but-reviewable signal.
pub fn match_identity(a: &IdentityKeys, b: &IdentityKeys) -> Option<IdentityMatch> {
    let mut signals: Vec<(f64, String, Value)> = Vec::new();

    if let (Some(x), Some(y)) = (&a.source_guid, &b.source_guid) {
        if x.eq_ignore_ascii_case(y) {
            signals.push((1.0, "source GUID match".into(), json!({ "source_guid": x })));
        }
    }
    if let (Some(x), Some(y)) = (&a.serial, &b.serial) {
        if x == y {
            signals.push((0.95, format!("serial match {x}"), json!({ "serial": x })));
        }
    }
    if let (Some(x), Some(y)) = (&a.service_tag, &b.service_tag) {
        if x == y {
            signals.push((
                0.9,
                format!("service tag match {x}"),
                json!({ "service_tag": x }),
            ));
        }
    }
    if !a.macs.is_empty() && !b.macs.is_empty() {
        let common: Vec<&String> = a.macs.iter().filter(|m| b.macs.contains(m)).collect();
        if !common.is_empty() {
            // Exact set = 0.9; a partial overlap scales down and is always
            // reviewable (never auto-merged at the shipped 0.85 threshold
            // unless the overlap is the whole smaller set).
            let exact = a.macs.len() == b.macs.len() && common.len() == a.macs.len();
            let conf = if exact { 0.9 } else { 0.6 };
            signals.push((
                conf,
                format!(
                    "MAC overlap {}/{}",
                    common.len(),
                    a.macs.len().max(b.macs.len())
                ),
                json!({ "macs_common": common }),
            ));
        }
    }
    if let (Some(hx), Some(hy)) = (&a.hostname, &b.hostname) {
        if hx == hy {
            match (&a.domain, &b.domain) {
                (Some(dx), Some(dy)) if dx == dy => signals.push((
                    0.88,
                    format!("hostname+domain match {hx}.{dx}"),
                    json!({ "hostname": hx, "domain": dx }),
                )),
                (None, None) => signals.push((
                    0.75,
                    format!("hostname match {hx}"),
                    json!({ "hostname": hx }),
                )),
                _ => {}
            }
        }
    }

    if signals.is_empty() {
        return None;
    }
    signals.sort_by(|p, q| q.0.partial_cmp(&p.0).unwrap_or(std::cmp::Ordering::Equal));
    let (best, reason, _) = signals[0].clone();
    // A second independent signal raises confidence (capped at 0.99).
    let combined = signals
        .get(1)
        .map(|(c, _, _)| (best + c * 0.1).min(0.99))
        .unwrap_or(best);
    let all_evidence: Vec<Value> = signals.iter().map(|(_, _, e)| e.clone()).collect();
    Some(IdentityMatch {
        confidence: (combined * 1000.0).round() / 1000.0,
        reason,
        evidence: json!({ "signals": all_evidence }),
    })
}

// ---- CI5: curated vs observed ---------------------------------------------

/// The ADR-shipped source kind for a known vendor; unknown vendors are
/// treated as generic documentation sources.
pub fn shipped_source_kind(source_system: &str) -> &'static str {
    match source_system {
        "ninjaone" => "rmm",
        "auvix" => "nms",
        "it_glue" | "itglue" | "hudu" => "docs",
        _ => "docs",
    }
}

/// Default precedence for a field, by source kind. Higher wins. Curated
/// (human) values always outrank every feed, independent of this table.
pub fn shipped_priority(field_key: &str, source_kind: &str) -> i16 {
    let key = field_key.to_ascii_lowercase();
    let rmm_field = matches!(
        key.as_str(),
        "serial" | "serial_number" | "service_tag" | "make_model" | "os" | "hostname"
    );
    let nms_field = matches!(
        key.as_str(),
        "ip_address" | "wan_ip" | "lan_ip" | "mac" | "mac_address" | "last_seen"
    );
    let human_field = matches!(
        key.as_str(),
        "owner" | "assigned_user" | "location" | "site" | "description"
    );
    match (source_kind, rmm_field, nms_field, human_field) {
        ("rmm", true, _, _) => 90,
        ("nms", _, true, _) => 90,
        ("human", _, _, _) => 100,
        (_, _, _, true) => 30,
        ("rmm", _, _, _) => 40,
        ("nms", _, _, _) => 40,
        _ => 50,
    }
}

/// CI5: the effective value of a field — curated wins over observed. The
/// `fields` column is projected from this so existing asset consumers keep
/// working unchanged.
pub fn effective_fields(observed: &Value, curated: &Value) -> Value {
    let mut out = Map::new();
    if let Some(o) = observed.as_object() {
        for (k, v) in o {
            if !v.is_null() {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(c) = curated.as_object() {
        for (k, v) in c {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// CI5: does an incoming observed value contradict a curated one?
pub fn contradicts_curated(curated: &Value, field_key: &str, incoming: &Value) -> bool {
    match curated.get(field_key) {
        Some(existing) => existing != incoming,
        None => false,
    }
}

/// The book value of an asset under straight-line depreciation, in minor
/// units, floored at salvage. `as_of` is a date; returns `None` when the
/// inputs are incomplete or the method is `none`.
pub fn straight_line_book_value_minor(
    purchase_cost_minor: Option<i64>,
    salvage_value_minor: Option<i64>,
    purchase_date: Option<NaiveDate>,
    useful_life_months: Option<i32>,
    as_of: NaiveDate,
) -> Option<i64> {
    let cost = purchase_cost_minor?;
    let life = useful_life_months?;
    if life <= 0 {
        return None;
    }
    let salvage = salvage_value_minor.unwrap_or(0).max(0);
    let months = (as_of.year() - purchase_date?.year()) as i64 * 12
        + (as_of.month() as i64 - purchase_date?.month() as i64);
    let months = months.max(0) as f64;
    let depreciable = (cost - salvage).max(0) as f64;
    let remaining = (depreciable * (life as f64 - months) / life as f64).round() as i64;
    Some((salvage + remaining.max(0)).min(cost))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(json: Value) -> IdentityKeys {
        IdentityKeys::from_value(&json)
    }

    #[test]
    fn normalizers_are_deterministic() {
        assert_eq!(normalize_serial(" sn-12 34_A "), "SN1234A");
        assert_eq!(
            normalize_mac("AA-BB-CC-DD-EE-FF").unwrap(),
            "aa:bb:cc:dd:ee:ff"
        );
        assert_eq!(normalize_mac("not-a-mac"), None);
        assert_eq!(
            normalize_hostname(" Host.Example.COM. "),
            "host.example.com"
        );
    }

    #[test]
    fn guid_match_is_decisive() {
        let a = keys(json!({ "source_guid": "G-1", "serial": "SN1" }));
        let b = keys(json!({ "source_guid": "g-1" }));
        let m = match_identity(&a, &b).unwrap();
        assert_eq!(m.confidence, 1.0);
    }

    #[test]
    fn serial_beats_hostname_only() {
        let a = keys(json!({ "serial": "sn-99", "hostname": "host" }));
        let b = keys(json!({ "serial": "SN99", "hostname": "other" }));
        let m = match_identity(&a, &b).unwrap();
        assert!(m.confidence >= 0.95);
        let c = keys(json!({ "hostname": "HOST." }));
        let m2 = match_identity(&a, &c).unwrap();
        assert!(m2.confidence < 0.8);
    }

    #[test]
    fn mac_partial_overlap_is_reviewable() {
        let a = keys(json!({ "macs": ["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:02"] }));
        let b = keys(json!({ "macs": ["aa:bb:cc:dd:ee:02"] }));
        let m = match_identity(&a, &b).unwrap();
        assert!(m.confidence < 0.85);
        let c = keys(json!({ "macs": ["AA:BB:CC:DD:EE:01", "AA:BB:CC:DD:EE:02"] }));
        assert!(match_identity(&a, &c).unwrap().confidence >= 0.9);
    }

    #[test]
    fn no_signal_is_no_match() {
        assert!(match_identity(
            &keys(json!({ "serial": "A" })),
            &keys(json!({ "hostname": "b" }))
        )
        .is_none());
    }

    #[test]
    fn curated_overrides_observed_in_effective_view() {
        let observed = json!({ "serial": "OBS", "ip_address": "10.0.0.1" });
        let curated = json!({ "serial": "CUR" });
        let eff = effective_fields(&observed, &curated);
        assert_eq!(eff["serial"], "CUR");
        assert_eq!(eff["ip_address"], "10.0.0.1");
        assert!(contradicts_curated(&curated, "serial", &json!("NEW")));
        assert!(!contradicts_curated(&curated, "serial", &json!("CUR")));
    }

    #[test]
    fn shipped_precedence_matches_adr_defaults() {
        assert!(shipped_priority("serial", "rmm") > shipped_priority("serial", "nms"));
        assert!(shipped_priority("ip_address", "nms") > shipped_priority("ip_address", "rmm"));
        assert!(shipped_priority("owner", "docs") < shipped_priority("owner", "human"));
    }
}
