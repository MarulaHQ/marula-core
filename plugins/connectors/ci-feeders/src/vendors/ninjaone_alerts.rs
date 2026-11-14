//! NinjaOne alert payloads onto Marula's signed alert-ingest contract.
//!
//! NinjaOne's documented alert object (`GET /v2/alerts`, and the same shape
//! its webhook deliveries carry) is a flat record:
//!
//! ```json
//! {
//!   "uid": "0a1b…",
//!   "deviceId": 4242,
//!   "deviceName": "ACME-MAIL-01",
//!   "severity": "CRITICAL",
//!   "condition": "DISK_SPACE",
//!   "message": "Drive C: is 96% full",
//!   "status": "STARTED",
//!   "createTime": "2026-11-14T09:12:00Z",
//!   "lastSeen": "2026-11-14T09:40:00Z",
//!   "source": "CONDITION",
//!   "ticketNumber": 0,
//!   "data": { … }
//! }
//! ```
//!
//! Three properties matter more than the field names, because each one is a
//! behaviour the ingest depends on:
//!
//! * **A stable dedupe key.** `uid` is the alert's own identity and repeats
//!   across deliveries, so a webhook retry is a `duplicate` at the ingest and
//!   writes nothing. When `uid` is absent, the key is derived from the facts
//!   that do not change between deliveries — device, condition and the
//!   condition's start time — never from receipt time.
//! * **A storm key.** Every alert of one condition on one device shares
//!   `correlation_key`, so the ingest folds distinct alerts into the ONE open
//!   incident ticket and increments its `alert_count`. That is the storm
//!   counting: one event (one incident), a count, not a row per repeat.
//! * **A severity on Marula's three-value vocabulary.** NinjaOne grades
//!   `CRITICAL | MAJOR | MINOR | NONE`; the mapping is stated once below and
//!   tested per grade, because "NONE" arriving as a warning is how a cleared
//!   check opens a P2 ticket.
//!
//! The mapper performs no I/O and signs nothing. The body it returns is exactly
//! the `POST /api/v1/alerts/ingest/{source_ref}` document, so the caller uses
//! the same signed transport every other alert source uses (see
//! `unifi::alerts` for the sender half).
//!
//! Vendor documentation for the alert object was not reachable from the build
//! environment that wrote this module; the shape above is the one recorded in
//! `tests/fixtures/ninjaone_alert.json`, and every field is read leniently so a
//! delivery that carries a subset still maps rather than being dropped.

use serde_json::{json, Map, Value};

use crate::vendors::{attributes_of, first_str, NormalizeError};

/// The Marula source system id this mapper speaks for.
pub const SOURCE: &str = "ninjaone";

/// Marula's closed severity vocabulary, as the ingest accepts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            AlertSeverity::Info => "info",
            AlertSeverity::Warning => "warning",
            AlertSeverity::Critical => "critical",
        }
    }
}

/// One NinjaOne alert, normalized onto the ingest document.
#[derive(Debug, Clone, PartialEq)]
pub struct NinjaOneAlert {
    /// The replay gate: identical across deliveries of the same alert.
    pub dedup_key: String,
    /// The storm identity: device + condition, identical across the alerts a
    /// storm produces.
    pub correlation_key: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: AlertSeverity,
    pub occurred_at: Option<String>,
    pub attributes: Value,
}

impl NinjaOneAlert {
    /// The exact `IngestAlertRequest` body. `correlation_key` is always sent:
    /// without it the ingest would default it to the dedup key, and every
    /// alert of a storm would anchor its own ticket.
    pub fn ingest_body(&self) -> Value {
        let mut body = json!({
            "dedup_key": self.dedup_key,
            "correlation_key": self.correlation_key,
            "title": self.title,
            "severity": self.severity.as_str(),
        });
        let object = body.as_object_mut().expect("literal json object");
        if let Some(description) = &self.description {
            object.insert("description".into(), json!(description));
        }
        if let Some(occurred_at) = &self.occurred_at {
            object.insert("occurred_at".into(), json!(occurred_at));
        }
        if self.attributes.as_object().is_some_and(|m| !m.is_empty()) {
            object.insert("attributes".into(), self.attributes.clone());
        }
        body
    }
}

/// What one NinjaOne delivery means for Marula.
#[derive(Debug, Clone, PartialEq)]
pub enum NinjaOneEvent {
    /// An active condition: ingest it and let the storm rules run.
    Alert(NinjaOneAlert),
    /// A resolved or cleared condition. The ingest has no "close an alert"
    /// verb, and posting this as a fresh alert would open a ticket for a
    /// problem that just went away, so the caller records it instead.
    Resolved {
        dedup_key: String,
        correlation_key: String,
    },
}

/// Maps one NinjaOne alert payload.
///
/// Returns [`NinjaOneEvent::Resolved`] for the terminal statuses, and a typed
/// refusal when the payload carries no identity at all — never a fabricated
/// alert for an object this mapper does not understand.
pub fn map_alert(payload: &Value) -> Result<NinjaOneEvent, NormalizeError> {
    let fields = attributes_of(payload, SOURCE)?;

    let device_id = first_str(fields, &["deviceId", "device_id", "nodeId"])
        .or_else(|| numeric(fields, &["deviceId", "device_id", "nodeId"]));
    let condition = first_str(fields, &["condition", "alertType", "type", "key"]);
    let uid = first_str(fields, &["uid", "alertUid", "alertId", "guid", "id"])
        .or_else(|| numeric(fields, &["id", "alertId"]));

    let created = first_str(fields, &["createTime", "createdAt", "created", "timestamp"]);
    let correlation_key = storm_key(device_id.as_deref(), condition.as_deref(), uid.as_deref());
    let dedup_key = match (&uid, &device_id, &condition) {
        (Some(uid), _, _) => cap_key(uid.clone()),
        // No alert identity: the condition's own identity is the stable key.
        // Receipt time is deliberately not part of it, or a retry would look
        // like a new alert.
        (None, Some(device), Some(condition)) => cap_key(format!(
            "{device}:{condition}:{}",
            created.as_deref().unwrap_or("open")
        )),
        _ => {
            return Err(NormalizeError::Unrecognised {
                vendor: SOURCE,
                reason: "neither an alert uid nor a device+condition pair".into(),
            })
        }
    };

    let status = first_str(fields, &["status", "state"])
        .unwrap_or_else(|| "STARTED".into())
        .to_ascii_uppercase();
    if matches!(status.as_str(), "RESOLVED" | "CLEARED" | "CLOSED" | "ENDED") {
        return Ok(NinjaOneEvent::Resolved {
            dedup_key,
            correlation_key,
        });
    }

    let device_name = first_str(fields, &["deviceName", "displayName", "hostname"]);
    let raw_severity = first_str(fields, &["severity", "priority", "severityLevel"]);
    let severity = severity_of(raw_severity.as_deref());

    let message = first_str(fields, &["message", "subject", "title"]);
    let title = match (&message, &device_name, &condition) {
        (Some(message), Some(device), _) => format!("{message} ({device})"),
        (Some(message), None, _) => message.clone(),
        (None, Some(device), Some(condition)) => format!("{condition} on {device}"),
        (None, None, Some(condition)) => condition.clone(),
        _ => format!("NinjaOne alert {dedup_key}"),
    };

    let description = first_str(fields, &["description", "detail", "details"]);

    let mut attributes = Map::new();
    for (key, value) in [
        ("device_id", device_id.clone()),
        ("device_name", device_name.clone()),
        ("condition", condition.clone()),
        ("status", Some(status)),
        ("source", first_str(fields, &["source"])),
        ("ticket_number", first_str(fields, &["ticketNumber"])),
        ("vendor_severity", raw_severity),
    ] {
        if let Some(value) = value {
            attributes.insert(key.to_string(), Value::String(value));
        }
    }
    // NinjaOne nests condition detail under `data`; carry it verbatim so a
    // rule or a technician sees the vendor's own numbers, not a paraphrase.
    if let Some(data) = fields.get("data").filter(|v| v.is_object()) {
        attributes.insert("data".into(), data.clone());
    }

    Ok(NinjaOneEvent::Alert(NinjaOneAlert {
        dedup_key,
        correlation_key,
        title,
        description,
        severity,
        occurred_at: first_str(
            fields,
            &["createTime", "createdAt", "timestamp", "lastSeen"],
        ),
        attributes: Value::Object(attributes),
    }))
}

/// The storm identity: one condition on one device. Falls back to the alert's
/// own uid when the payload carries no device/condition pair, which makes the
/// alert its own storm — the same default the ingest applies.
fn storm_key(device_id: Option<&str>, condition: Option<&str>, uid: Option<&str>) -> String {
    match (device_id, condition) {
        (Some(device), Some(condition)) => cap_key(format!("{device}:{condition}")),
        _ => uid
            .map(|uid| cap_key(uid.to_string()))
            .unwrap_or_else(|| "ninjaone:unknown".into()),
    }
}

/// The ingest caps both keys at 200 characters; truncating here keeps the
/// request valid instead of earning a 422 for one long uid.
fn cap_key(mut key: String) -> String {
    const MAX: usize = 200;
    if key.len() > MAX {
        key.truncate(MAX);
    }
    key
}

fn numeric(fields: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match fields.get(*key) {
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    })
}

/// NinjaOne grades a condition, not a ticket. The mapping is deliberately
/// total: an unknown grade is a warning (P2), never critical by accident and
/// never quiet enough to hide a real fault.
pub fn severity_of(raw: Option<&str>) -> AlertSeverity {
    let Some(raw) = raw else {
        return AlertSeverity::Warning;
    };
    let grade = raw.trim().to_ascii_uppercase();
    match grade.as_str() {
        "CRITICAL" | "SEVERE" | "P1" | "1" => AlertSeverity::Critical,
        "MAJOR" | "WARNING" | "WARN" | "MODERATE" | "P2" | "2" => AlertSeverity::Warning,
        "MINOR" | "LOW" | "INFO" | "INFORMATIONAL" | "NONE" | "P3" | "3" => AlertSeverity::Info,
        _ => AlertSeverity::Warning,
    }
}

/// What the coalescer decided for one delivery.
#[derive(Debug, Clone, PartialEq)]
pub enum StormDecision {
    /// Post this body; it is the first delivery of that alert.
    Send(NinjaOneAlert),
    /// The same alert was already sent inside the window. Do not post it;
    /// the ingest would answer `duplicate` and write nothing anyway. The
    /// count is the local evidence that the repeat happened.
    Repeat {
        dedup_key: String,
        correlation_key: String,
        repeats: u32,
    },
}

/// In-memory guard against repeat deliveries of the SAME alert.
///
/// It is not the storm counter: Marula's ingest owns that, folding distinct
/// alerts that share a `correlation_key` into one incident and incrementing
/// that incident's `alert_count`. This guard only stops a duplicated delivery
/// from costing a round trip, and gives the connector's own report the repeat
/// count it needs.
///
/// One instance per forwarding process. The map is bounded by the number of
/// alerts seen inside the window, and expired entries are swept on insert.
#[derive(Debug)]
pub struct StormCoalescer {
    window_secs: i64,
    seen: std::collections::HashMap<String, (i64, u32)>,
}

impl StormCoalescer {
    /// `window_secs` should match the alert source's correlation window; a
    /// shorter one re-sends repeats the ingest will drop as duplicates, which
    /// is wasteful but never wrong.
    pub fn new(window_secs: i64) -> Self {
        Self {
            window_secs: window_secs.max(1),
            seen: std::collections::HashMap::new(),
        }
    }

    /// Admits one mapped alert at `now_unix`.
    pub fn admit(&mut self, alert: NinjaOneAlert, now_unix: i64) -> StormDecision {
        self.seen
            .retain(|_, (first, _)| now_unix.saturating_sub(*first) <= self.window_secs);
        match self.seen.get_mut(&alert.dedup_key) {
            Some((_, repeats)) => {
                *repeats += 1;
                StormDecision::Repeat {
                    dedup_key: alert.dedup_key,
                    correlation_key: alert.correlation_key,
                    repeats: *repeats,
                }
            }
            None => {
                self.seen.insert(alert.dedup_key.clone(), (now_unix, 1));
                StormDecision::Send(alert)
            }
        }
    }
}

/// The reason a whole batch could not be mapped, with the payload index that
/// refused it. The caller reports the index and keeps the rest — one bad
/// delivery must not drop the storm behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRefusal {
    pub index: usize,
    pub reason: String,
}

/// Maps a delivery batch, coalescing exact repeats and dropping resolved
/// conditions. Returns `(decisions, refusals)`: refusals are reported, never
/// silently skipped.
pub fn map_batch(
    payloads: &[Value],
    coalescer: &mut StormCoalescer,
    now_unix: i64,
) -> (Vec<StormDecision>, Vec<BatchRefusal>) {
    let mut decisions = Vec::new();
    let mut refusals = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        match map_alert(payload) {
            Ok(NinjaOneEvent::Alert(alert)) => {
                decisions.push(coalescer.admit(alert, now_unix));
            }
            Ok(NinjaOneEvent::Resolved { .. }) => {}
            Err(error) => refusals.push(BatchRefusal {
                index,
                reason: error.to_string(),
            }),
        }
    }
    (decisions, refusals)
}
