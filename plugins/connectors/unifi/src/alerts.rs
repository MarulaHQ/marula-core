//! Signed UniFi-alert forwarding — the IN half of the connector.
//!
//! UniFi's controller webhooks carry NO signature of their own, so the
//! operator's forwarding hop (UniFi webhook → signing relay → this
//! connector, or UniFi → this connector directly) is the trust boundary.
//! The connector accepts both postures honestly:
//!
//! * when a hop signature is configured, deliveries are verified
//!   EXACTLY the way first-party receivers must — constant-time
//!   HMAC-SHA256 over `"{t}.{rawBody}"` with the `t=<ts>,v1=<hex>`
//!   header (the same scheme Marula's own webhook + alert-ingest
//!   senders use), plus the ±10-minute anti-replay window, verified
//!   over the RAW BYTES before any JSON parsing — signature covers
//!   bytes, not objects;
//! * without a hop secret the receiver states the unauthenticated
//!   posture loudly in its run report (the local LAN endpoint stays
//!   reachable; the alert source's OWN secret still seals the
//!   Marula-bound leg).
//!
//! The Marula-bound leg is always signed: the alert source's HMAC
//! secret (returned ONCE at source creation) signs every alert this
//! connector posts to `POST /api/v1/alerts/ingest/{source_ref}` —
//! Marula 401s unsigned ingests, so a signature-less hop never
//! reaches Marula with fabricated events.

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Same header name and tolerance as Marula's webhook receivers and
/// the NinjaOne/Hudu reference receivers — one scheme everywhere.
pub const SIGNATURE_HEADER: &str = "X-Marula-Signature";
/// Generous for queue delay, tight enough to kill replays (~10 min).
pub const REPLAY_TOLERANCE_SECS: i64 = 600;

pub type VerifyOutcome = Result<(), &'static str>;

fn hmac_hex(secret: &[u8], message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// Receiver-side verification of one forwarding hop's headers + raw
/// body. Raw bytes are verified BEFORE anything is parsed.
pub fn verify_delivery(
    secret: &[u8],
    raw_body: &[u8],
    signature_header: Option<&str>,
    now_unix: i64,
) -> VerifyOutcome {
    let Some(presented) = signature_header.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err("malformed-header");
    };
    let mut parts = presented.split(',');
    let (Some(t_part), Some(v_part), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err("malformed-header");
    };
    let Some(ts) = t_part
        .strip_prefix("t=")
        .and_then(|v| v.parse::<i64>().ok())
    else {
        return Err("malformed-header");
    };
    let Some(presented_mac) = v_part.strip_prefix("v1=") else {
        return Err("malformed-header");
    };
    if presented_mac.is_empty() || !presented_mac.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("malformed-header");
    }
    if (now_unix - ts).abs() > REPLAY_TOLERANCE_SECS {
        return Err("outside-window");
    }
    let expected = hmac_hex(secret, &timestamp_prefixed(ts, raw_body));
    // Length mismatch short-circuits; lengths are not secret. The
    // comparison itself is constant-time.
    if presented_mac.len() != expected.len() {
        return Err("bad-signature");
    }
    if presented_mac.as_bytes().ct_eq(expected.as_bytes()).into() {
        Ok(())
    } else {
        Err("bad-signature")
    }
}

/// Sender-side signature for one Marula-bound alert delivery —
/// `t=<unix_ts>,v1=<hex hmac>` over `{t}.{rawBody}`, the exact value
/// `X-Marula-Signature` must carry at the ingest endpoint.
pub fn sign_delivery(secret: &[u8], raw_body: &[u8], now_unix: i64) -> String {
    let mac = hmac_hex(secret, &timestamp_prefixed(now_unix, raw_body));
    format!("t={now_unix},v1={mac}")
}

fn timestamp_prefixed(unix_ts: i64, body: &[u8]) -> Vec<u8> {
    let mut buf = unix_ts.to_string().into_bytes();
    buf.push(b'.');
    buf.extend_from_slice(body);
    buf
}

/// The Marula ingest wire shape (the published `IngestAlertRequest`
/// contract: `dedup_key` + `title` + `severity` required).
#[derive(Debug, Clone, PartialEq)]
pub struct IngestAlert {
    pub dedup_key: String,
    pub title: String,
    pub severity: String,
    pub description: Option<String>,
    pub occurred_at: Option<String>,
    pub attributes: Option<Value>,
}

/// Maps one UniFi alert event (whatever the forwarding hop passes
/// through from the controller's alert stream) onto the ingest
/// contract. UniFi alert events carry `_id`/`key` + `msg`/`message` +
/// `datetime`; the mapper keys on what the payload actually shows and
/// never invents fields.
///
/// Severity: UniFi's alert stream does not classify severity. An
/// explicit `severity`/`level` key wins (critical|error|down →
/// `critical`, warning|warn → `warning`, else `info`); with none the
/// alert forwards as `warning` — UniFi alerts signal operational
/// faults (device down, WAN transition), and the incident workflow's
/// P2 default matches that posture.
pub fn map_alert(payload: &Value) -> Option<IngestAlert> {
    let dedup_key = first_of(payload, &["_id", "id"]).or_else(|| {
        let key = first_of(payload, &["key"])?;
        let stamp = first_of(payload, &["datetime", "time", "timestamp"]).unwrap_or_default();
        Some(format!("{key}:{stamp}"))
    })?;
    if dedup_key.len() > 200 {
        return None;
    }
    let title = first_of(payload, &["msg", "message", "alert", "title"])
        .unwrap_or_else(|| format!("UniFi alert {}", dedup_key));

    let severity = match first_of(payload, &["severity", "level"]).map(|s| s.to_ascii_lowercase()) {
        Some(level)
            if level.contains("critical") || level.contains("error") || level.contains("down") =>
        {
            "critical".to_string()
        }
        Some(level) if level.contains("warning") || level.contains("warn") => "warning".to_string(),
        Some(_) => "info".to_string(),
        None => "warning".to_string(),
    };

    let attributes = payload
        .get("meta")
        .or_else(|| payload.get("attributes"))
        .filter(|v| v.is_object())
        .cloned();

    Some(IngestAlert {
        dedup_key,
        title,
        severity,
        description: first_of(payload, &["description", "detail"]),
        occurred_at: first_of(payload, &["datetime", "time", "timestamp", "occurredAt"]),
        attributes,
    })
}

fn first_of(payload: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match payload.get(*key) {
            Some(Value::String(s)) if !s.trim().is_empty() => return Some(s.trim().to_string()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => continue,
        }
    }
    None
}

/// Why one forward failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForwardError {
    #[error("ingest unauthorized: {0}")]
    Unauthorized(String),
    #[error("ingest refused: {0}")]
    Rejected(String),
    #[error("ingest transport failure: {0}")]
    Network(String),
}

/// What one forward did (the ingest endpoint's own verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardOutcome {
    /// `incident_created` | `collapsed` | `duplicate`.
    pub status: String,
    /// The alert id at Marula.
    pub alert_id: String,
}

/// The Marula-bound alert forwarder. Talks to Marula ONLY through
/// the published ingest seam; the alert-source secret signs every
/// delivery and is never logged or persisted by the crate.
pub struct AlertForwarder {
    base_url: String,
    source_ref: String,
    secret: Vec<u8>,
}

impl AlertForwarder {
    pub fn new(
        base_url: impl Into<String>,
        source_ref: impl Into<String>,
        secret: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            source_ref: source_ref.into(),
            secret: secret.into(),
        }
    }

    /// Signs and posts ONE alert's canonical body to
    /// `POST /api/v1/alerts/ingest/{source_ref}`. The signature covers
    /// the exact bytes sent — the body is serialized once and those
    /// bytes are what gets HMAC'd.
    pub async fn forward(
        &self,
        alert: &IngestAlert,
        now_unix: i64,
    ) -> Result<ForwardOutcome, ForwardError> {
        let mut body = json!({
            "dedup_key": alert.dedup_key,
            "title": alert.title,
            "severity": alert.severity,
        });
        let object = body.as_object_mut().expect("literal json object");
        if let Some(description) = &alert.description {
            object.insert("description".into(), json!(description));
        }
        if let Some(occurred_at) = &alert.occurred_at {
            object.insert("occurred_at".into(), json!(occurred_at));
        }
        if let Some(attributes) = &alert.attributes {
            object.insert("attributes".into(), attributes.clone());
        }
        let serialized = serde_json::to_vec(&body)
            .map_err(|e| ForwardError::Rejected(format!("alert body unreadable: {e}")))?;

        let url = format!(
            "{}/api/v1/alerts/ingest/{}",
            self.base_url.trim_end_matches('/'),
            self.source_ref.trim()
        );
        let response = reqwest::Client::new()
            .post(url)
            .header(
                SIGNATURE_HEADER,
                sign_delivery(&self.secret, &serialized, now_unix),
            )
            .header("content-type", "application/json")
            .body(serialized)
            .send()
            .await
            .map_err(|e| ForwardError::Network(e.to_string()))?;

        let status = response.status();
        let payload: Value = response.json().await.unwrap_or(Value::Null);
        match status {
            reqwest::StatusCode::OK => {
                let alert_id = payload
                    .get("alert")
                    .and_then(|a| a.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(ForwardOutcome {
                    status: payload
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    alert_id,
                })
            }
            reqwest::StatusCode::UNAUTHORIZED => Err(ForwardError::Unauthorized(
                message_of(&payload).into_owned(),
            )),
            code => Err(ForwardError::Rejected(format!(
                "ingest returned {code}: {}",
                message_of(&payload)
            ))),
        }
    }
}

fn message_of(body: &Value) -> std::borrow::Cow<'_, str> {
    body.get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or_else(|| std::borrow::Cow::Owned("unspecified".to_string()))
}
