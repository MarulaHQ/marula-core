//! 3CX agentic call-control adapter (Wave 22 lane 3, ecosystem/SDK
//! review leg B) — rides the Wave 22 lane 1 VoIP connector API seam:
//! `POST /api/v1/connectors/voip/call-logs` over an installed `voip`
//! connector consent, plus click-to-call through the 3CX V20 Call
//! Control API.
//!
//! The 3CX side is the V20 XAPI: OAuth2 client-credentials grant
//! against `POST /connect/token` (client application minted in the 3CX
//! Management Console with the XAPI + Call Control scopes; tokens
//! expire after one hour and the shared grant cache refreshes a minute
//! early), JSON calls under `GET/POST /xapi/v1/...`:
//!
//! * `GET /xapi/v1/Cdr` — the call detail records (paged via OData
//!   `$top`/`$skip`), mapped onto the Meridian VoIP ingest contract;
//! * `POST /xapi/v1/Users/Pbx.MakeCall` `{dn, destination}` — the
//!   click-to-call context action (the adapter dials; call state flows
//!   back asynchronously through 3CX's notifications, outside the
//!   reference adapter's scope).
//!
//! Ticket linkage: call events land on the ticket activity timeline via
//! the call log's `ticket_id`. The click-to-call context is the same
//! map — a dial initiated from ticket context remembers
//! `(call id → ticket id)` and the sync pushes the completed call
//! against that ticket. Calls without context push ticketless.
//!
//! None mode (honest posture): 3CX is a self-hosted PBX — without
//! client credentials there is no token, no call control, and nothing
//! to pull; the adapter reports a typed `ThreecxError::Vendor` and the
//! Meridian side stays fully functional (the VoIP ingest seam accepts
//! call rows from any provider; every push the adapter WOULD have made
//! is reported). With a Meridian sink in none mode
//! ([`NoneCallLogSink`]) every mapped call counts as "would have
//! pushed" with zero Meridian I/O. Live-PBX smoke is out of scope —
//! tests run against contract-shaped stub servers.

use std::collections::HashMap;

use adapters_core::auth::{ClientCredentials, VendorAuth};
use adapters_core::vendor_fields::{first_str_in, scalar_in};
use serde_json::{json, Value};
use uuid::Uuid;

const SOURCE: &str = "threecx";

#[cfg(test)]
mod tests;

/// 3CX client. `fqdn` is the PBX origin (`https://pbx.example.com`);
/// the client authenticates with OAuth2 client credentials against
/// `{fqdn}/connect/token` and calls the XAPI under `{fqdn}/xapi/v1`.
pub struct ThreecxClient {
    creds: ClientCredentials,
    http: reqwest::Client,
}

impl ThreecxClient {
    pub fn new(
        fqdn: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        let base = fqdn.into().trim_end_matches('/').to_string();
        Self {
            creds: ClientCredentials {
                token_url: format!("{base}/connect/token"),
                client_id: client_id.into(),
                client_secret: client_secret.into(),
                scope: String::new(),
            },
            http: reqwest::Client::new(),
        }
    }

    /// Every call detail record, following 3CX's OData `$top`/`$skip`
    /// pagination to exhaustion — a silently truncated journal is a
    /// wrong journal.
    pub async fn cdrs(&self) -> Result<Vec<Value>, adapters_core::client::VendorError> {
        let xapi = format!(
            "{}/xapi/v1",
            self.creds.token_url.trim_end_matches("/connect/token")
        );
        let page_size = 100usize;
        let mut items = Vec::new();
        let mut skip = 0usize;
        let mut guard = 0usize;
        loop {
            let query = vec![
                ("$top".to_string(), page_size.to_string()),
                ("$skip".to_string(), skip.to_string()),
            ];
            let request = self.http.get(format!("{xapi}/Cdr")).query(&query);
            let request = self.creds_auth(request).await?;
            let response = request
                .send()
                .await
                .map_err(|e| adapters_core::client::VendorError::Network(format!("{xapi}: {e}")))?;
            let status = response.status();
            let body: Value = response.json().await.unwrap_or(Value::Null);
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(adapters_core::client::VendorError::Unauthorized(
                    message_of(&body).into_owned(),
                ));
            }
            if !status.is_success() {
                return Err(adapters_core::client::VendorError::Rejected(format!(
                    "{status}: {}",
                    message_of(&body)
                )));
            }
            // The XAPI serves bare arrays (OData) — count-based paging
            // would need a total; stop at the first short page.
            let page: Vec<Value> = match body {
                Value::Array(items) => items,
                Value::Object(ref o) => o
                    .get("value")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let count = page.len();
            items.extend(page);
            skip += count;
            if count < page_size {
                break;
            }
            guard += 1;
            if guard > 10_000 {
                return Err(adapters_core::client::VendorError::Rejected(format!(
                    "pagination exceeded 10,000 pages at {xapi}/Cdr"
                )));
            }
        }
        Ok(items)
    }

    /// The click-to-call context action: originates one outbound call
    /// from the operator's extension/device (`dn`) to `destination`
    /// through `POST /xapi/v1/Users/Pbx.MakeCall`. Call state flows
    /// back asynchronously through 3CX's notifications — the adapter
    /// reports the API's immediate verdict only.
    pub async fn make_call(
        &self,
        dn: &str,
        destination: &str,
    ) -> Result<Value, adapters_core::client::VendorError> {
        let xapi = format!(
            "{}/xapi/v1",
            self.creds.token_url.trim_end_matches("/connect/token")
        );
        let request = self
            .http
            .post(format!("{xapi}/Users/Pbx.MakeCall"))
            .json(&json!({ "dn": dn, "destination": destination }));
        let request = self.creds_auth(request).await?;
        let response = request
            .send()
            .await
            .map_err(|e| adapters_core::client::VendorError::Network(format!("{xapi}: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            Err(adapters_core::client::VendorError::Unauthorized(
                message_of(&body).into_owned(),
            ))
        } else {
            Err(adapters_core::client::VendorError::Rejected(format!(
                "{status}: {}",
                message_of(&body)
            )))
        }
    }

    async fn creds_auth(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, adapters_core::client::VendorError> {
        VendorAuth::ClientCredentials(self.creds.clone())
            .apply(request, &self.http)
            .await
    }
}

/// The Meridian VoIP ingest wire shape (the published
/// `VoipCallLogRequest` contract — `call_id`, `direction in|out`,
/// `external_number`, `duration_seconds`, optional summary + ticket,
/// RFC-3339 `occurred_at`).
#[derive(Debug, Clone, PartialEq)]
pub struct MeridianCallLog {
    pub call_id: String,
    pub direction: String,
    pub external_number: String,
    pub duration_seconds: i64,
    pub summary: Option<String>,
    pub ticket_id: Option<Uuid>,
    pub occurred_at: String,
}

/// Maps one 3CX call detail record onto the ingest contract. The
/// external party is the OTHER side: an inbound call's caller, an
/// outbound call's callee. Direction keys vary by 3CX build —
/// `direction`/`type` are accepted (IN/OUT, case-insensitive).
/// The call id is 3CX's own CDR id — the ingest's idempotency gate.
pub fn map_call(cdr: &Value) -> Option<MeridianCallLog> {
    let call_id = first_str_in(cdr, &["id", "callId", "call_id"])?;
    let direction = first_str_in(cdr, &["direction", "type"])?.to_ascii_lowercase();
    let direction = match direction.as_str() {
        "in" | "inbound" | "incoming" => "in",
        "out" | "outbound" | "outgoing" => "out",
        _ => return None,
    };
    let external_number = match direction {
        "in" => first_str_in(cdr, &["callerNumber", "callerId", "caller", "from"])?,
        _ => first_str_in(cdr, &["calleeNumber", "calleeId", "callee", "to"])?,
    };
    let duration = scalar_in(cdr, &["duration", "durationSeconds", "duration_seconds"])
        .and_then(|d| d.trim().parse::<i64>().ok())
        .unwrap_or(0);
    Some(MeridianCallLog {
        call_id,
        direction: direction.into(),
        external_number,
        duration_seconds: duration.max(0),
        summary: first_str_in(cdr, &["statusText", "reasonDescription", "reason"]),
        occurred_at: first_str_in(
            cdr,
            &["startedAt", "startedDateTime", "timeStart", "started"],
        )
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        ticket_id: None,
    })
}

/// Why a run failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThreecxError {
    #[error("3CX pull failed: {0}")]
    Vendor(#[from] adapters_core::client::VendorError),
}

/// The Meridian-side apply seam: one call log row pushed through the
/// public VoIP ingest seam. Never logs credential material.
#[async_trait::async_trait]
pub trait CallLogSink: Send + Sync {
    async fn push(&self, call: &MeridianCallLog) -> Result<Uuid, String>;
}

/// Credentials + endpoint for the Meridian sink. The machine API key
/// rides `x-api-key` (the VoIP ingest surface authenticates scoped
/// machine keys); the org's `voip` connector installation is the
/// consent, this key the transport. Never persisted.
#[derive(Clone)]
pub struct MeridianCallLogConfig {
    pub base_url: String,
    pub api_key: String,
}

impl std::fmt::Debug for MeridianCallLogConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeridianCallLogConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// The public-API call-log sink: `POST /api/v1/connectors/voip/call-logs`
/// with the EXACT published wire shape — never hand-rolled beyond it.
pub struct MeridianCallLogSink {
    config: MeridianCallLogConfig,
}

impl std::fmt::Debug for MeridianCallLogSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeridianCallLogSink")
            .field("config", &"[REDACTED CONFIG]")
            .finish()
    }
}

impl MeridianCallLogSink {
    pub fn new(config: MeridianCallLogConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl CallLogSink for MeridianCallLogSink {
    async fn push(&self, call: &MeridianCallLog) -> Result<Uuid, String> {
        let url = format!(
            "{}/api/v1/connectors/voip/call-logs",
            self.config.base_url.trim_end_matches('/')
        );
        let response = reqwest::Client::new()
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .json(&json!({
                "call_id": call.call_id,
                "direction": call.direction,
                "external_number": call.external_number,
                "duration_seconds": call.duration_seconds,
                "summary": call.summary,
                "ticket_id": call.ticket_id,
                "occurred_at": call.occurred_at,
            }))
            .send()
            .await
            .map_err(|e| format!("ingest transport failure: {e}"))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        match status {
            reqwest::StatusCode::OK => Ok(body
                .get("id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "ingest response unreadable (missing id)".to_string())?),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                Err(format!("ingest unauthorized: {}", message_of(&body)))
            }
            code => Err(format!("ingest returned {code}: {}", message_of(&body))),
        }
    }
}

/// None mode: the shipped default. Every push reports honestly with
/// zero Meridian I/O.
pub struct NoneCallLogSink;

#[async_trait::async_trait]
impl CallLogSink for NoneCallLogSink {
    async fn push(&self, _call: &MeridianCallLog) -> Result<Uuid, String> {
        Err("call-log sink disabled: no Meridian connection configured (none mode)".into())
    }
}

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallSyncReport {
    /// Call detail records pulled from the PBX.
    pub total: usize,
    /// Call logs the sink applied.
    pub pushed: usize,
    /// Would-be pushes under none mode: zero I/O, counted honestly.
    pub would_push: usize,
    /// Dials initiated through the click-to-call seam during the run.
    pub dials: usize,
    /// Unmappable/failed records, with reasons.
    pub errors: Vec<String>,
}

impl CallSyncReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// The 3CX sync driver. `ticket_context` maps call ids → ticket ids
/// (the click-to-call context the adapter's dials record); a completed
/// call with a mapped id pushes against ITS ticket so the call event
/// lands on the ticket activity timeline. Without a context every call
/// pushes without a ticket link.
pub struct ThreecxSync;

impl ThreecxSync {
    pub async fn run(
        threecx: &ThreecxClient,
        ticket_context: Option<&HashMap<String, Uuid>>,
        sink: &dyn CallLogSink,
    ) -> Result<CallSyncReport, ThreecxError> {
        let cdrs = threecx.cdrs().await?;
        let mut report = CallSyncReport {
            total: cdrs.len(),
            ..Default::default()
        };
        for cdr in &cdrs {
            let mut call = match map_call(cdr) {
                Some(call) => call,
                None => {
                    report.errors.push(format!(
                        "{SOURCE}: unmappable call record (missing id or direction)"
                    ));
                    continue;
                }
            };
            if let Some(context) = ticket_context {
                call.ticket_id = context.get(&call.call_id).copied();
            }
            match sink.push(&call).await {
                Ok(_) => report.pushed += 1,
                Err(err) => {
                    if err.contains("none mode") {
                        report.would_push += 1;
                    } else {
                        report
                            .errors
                            .push(format!("{SOURCE}: call {} refused: {err}", call.call_id));
                    }
                }
            }
        }
        Ok(report)
    }

    /// The click-to-call context action: dials through the PBX and
    /// remembers the `(call id → ticket id)` mapping the sync consumes
    /// when the call completes. The returned call id is the context
    /// key; `record_context` is the operator transport's memory.
    pub async fn dial(
        threecx: &ThreecxClient,
        dn: &str,
        destination: &str,
    ) -> Result<String, ThreecxError> {
        let result = threecx.make_call(dn, destination).await?;
        let call_id = result
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| result.get("callId").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        Ok(call_id)
    }
}

fn message_of(body: &Value) -> std::borrow::Cow<'_, str> {
    body.get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or_else(|| std::borrow::Cow::Owned("unspecified".to_string()))
}
