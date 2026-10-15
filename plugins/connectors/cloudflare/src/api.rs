//! Cloudflare REST API v4 transport shared by the DNS and edge-posture
//! halves of the connector.
//!
//! The transport rides `adapters-core`'s `VendorClient` (bearer-token auth,
//! typed status mapping) and adds the Cloudflare envelope: every response is
//! `{"success": bool, "errors": [...], "messages": [...], "result": ...}`.
//! A `success: false` body is an error even when the HTTP status is 200, and
//! Cloudflare's error arrays are unwrapped into an operator-readable message.

use adapters_core::auth::VendorAuth;
use adapters_core::client::{VendorClient, VendorError};
use adapters_core::dns::{candidate_zone_names, DnsZone};
use serde_json::Value;

/// Cloudflare API v4 base URL.
pub const CLOUDFLARE_API_V4: &str = "https://api.cloudflare.com/client/v4";

/// Operator-declared Cloudflare credentials and endpoint.
///
/// This connector is **not a hosted OAuth app**: the credential is an API
/// token the operator creates at My Profile → API Tokens, scoped to
/// `Zone:DNS:Edit` (plus the optional read scopes the edge-posture helpers
/// use), stored in Core's existing org-settings / BYOK secret surface.
#[derive(Clone)]
pub struct CloudflareConfig {
    /// Operator-created Cloudflare API token. Never logged, never placed
    /// in a URL, redacted in `Debug` (see the manual `Debug` impl below).
    pub api_token: String,
    /// API base URL (no trailing slash). Defaults to `CLOUDFLARE_API_V4`.
    pub base_url: String,
}

impl CloudflareConfig {
    pub fn new(api_token: impl Into<String>) -> Self {
        Self {
            api_token: api_token.into(),
            base_url: CLOUDFLARE_API_V4.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }
}

impl std::fmt::Debug for CloudflareConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareConfig")
            .field("api_token", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// Typed Cloudflare errors. Every variant carries enough operator-facing
/// context to be actionable without digging through vendor payloads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudflareError {
    #[error("invalid domain name `{0}`")]
    InvalidDomain(String),
    #[error("invalid DNS record parameter: {0}")]
    InvalidRecord(String),
    #[error("no active Cloudflare zone matches `{domain}` (tried {})", tried.join(", "))]
    ZoneNotFound { domain: String, tried: Vec<String> },
    /// 401/403 — the token was refused or lacks the scope this call needs.
    /// `detail` names the scope so the operator can fix the token.
    #[error(
        "cloudflare refused the token for {operation}: {detail} — the operator token needs \
         `Zone:DNS:Edit`, plus `Zone:Zone Settings:Read` and \
         `Zone:SSL and Certificates:Read` for the edge reads"
    )]
    Unauthorized { operation: String, detail: String },
    #[error("cloudflare rate limited {operation}: {detail}")]
    RateLimited { operation: String, detail: String },
    #[error("cloudflare rejected {operation}: {detail}")]
    Rejected { operation: String, detail: String },
    #[error("cloudflare transport failure: {detail}")]
    Transport { detail: String },
}

impl From<CloudflareError> for adapters_core::dns::DnsProviderError {
    fn from(err: CloudflareError) -> Self {
        use adapters_core::dns::DnsProviderError as Dns;
        match err {
            CloudflareError::InvalidDomain(domain) => Dns::InvalidDomain(domain),
            CloudflareError::InvalidRecord(detail) => Dns::InvalidRecord(detail),
            CloudflareError::ZoneNotFound { domain, tried } => Dns::ZoneNotFound { domain, tried },
            CloudflareError::Unauthorized { operation, detail } => {
                Dns::UnauthorizedScope { operation, detail }
            }
            CloudflareError::RateLimited { detail, .. } => Dns::RateLimited(detail),
            CloudflareError::Rejected { operation, detail } => Dns::Rejected { operation, detail },
            CloudflareError::Transport { detail } => Dns::Transport(detail),
        }
    }
}

/// Maps a vendor-agnostic DNS-provider error (the contract's own validation
/// errors) onto the Cloudflare surface so `?` composes in both directions.
impl From<adapters_core::dns::DnsProviderError> for CloudflareError {
    fn from(err: adapters_core::dns::DnsProviderError) -> Self {
        use adapters_core::dns::DnsProviderError as Dns;
        match err {
            Dns::InvalidDomain(domain) => CloudflareError::InvalidDomain(domain),
            Dns::InvalidRecord(detail) => CloudflareError::InvalidRecord(detail),
            Dns::ZoneNotFound { domain, tried } => CloudflareError::ZoneNotFound { domain, tried },
            Dns::UnauthorizedScope { operation, detail } => {
                CloudflareError::Unauthorized { operation, detail }
            }
            Dns::RateLimited(detail) => CloudflareError::RateLimited {
                operation: "cloudflare".to_string(),
                detail,
            },
            Dns::Rejected { operation, detail } => CloudflareError::Rejected { operation, detail },
            Dns::Transport(detail) => CloudflareError::Transport { detail },
        }
    }
}

/// Maps an `adapters-core` transport error onto the Cloudflare surface,
/// enriching the unauthorized case with the scope the call needed.
fn from_vendor_error(operation: &str, err: VendorError) -> CloudflareError {
    match err {
        VendorError::Unauthorized(detail) => CloudflareError::Unauthorized {
            operation: operation.to_string(),
            detail,
        },
        VendorError::RateLimited(detail) => CloudflareError::RateLimited {
            operation: operation.to_string(),
            detail,
        },
        VendorError::Rejected(detail) => CloudflareError::Rejected {
            operation: operation.to_string(),
            detail,
        },
        VendorError::Network(detail) => CloudflareError::Transport { detail },
    }
}

/// Percent-encodes a path or query component (Cloudflare record names,
/// hostnames, and zone ids are safe ASCII, but this keeps a hostile or
/// generated name from breaking the request URL).
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Flattens Cloudflare's `errors` / `messages` arrays into one line.
fn describe_failure(body: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    for key in ["errors", "messages"] {
        if let Some(entries) = body.get(key).and_then(Value::as_array) {
            for entry in entries {
                let code = entry.get("code").and_then(Value::as_i64);
                let text = entry
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified");
                match code {
                    Some(code) => lines.push(format!("{code}: {text}")),
                    None => lines.push(text.to_string()),
                }
            }
        }
    }
    if lines.is_empty() {
        "cloudflare reported a failure without a message".to_string()
    } else {
        lines.join("; ")
    }
}

/// Unwraps the `result` field of a Cloudflare envelope, treating
/// `success: false` (any HTTP status) as a typed error.
fn unwrap_result(operation: &str, body: &Value) -> Result<Value, CloudflareError> {
    if body.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(CloudflareError::Rejected {
            operation: operation.to_string(),
            detail: describe_failure(body),
        });
    }
    Ok(body.get("result").cloned().unwrap_or(Value::Null))
}

/// Parses one Cloudflare zone object into the vendor-agnostic [`DnsZone`].
pub(crate) fn parse_zone(raw: &Value) -> Result<DnsZone, CloudflareError> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| CloudflareError::Rejected {
            operation: "zone parse".to_string(),
            detail: "zone response entry is missing `id`".to_string(),
        })?
        .to_string();
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(DnsZone {
        id,
        name,
        status: raw
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active")
            .to_string(),
    })
}

/// Strips surrounding presentation quotes from a TXT content value.
pub(crate) fn strip_txt_quotes(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .to_string()
}

/// The Cloudflare REST transport: bearer-token `VendorClient` plus
/// envelope-aware request helpers.
#[derive(Clone)]
pub struct CloudflareApi {
    client: VendorClient,
}

impl CloudflareApi {
    pub fn new(config: &CloudflareConfig) -> Self {
        Self {
            client: VendorClient::new(
                config.base_url.clone(),
                VendorAuth::Bearer(config.api_token.clone()),
            ),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.client.base_url
    }

    /// GET with a query, returning the envelope's `result`.
    pub async fn get(
        &self,
        operation: &str,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Value, CloudflareError> {
        let body = self
            .client
            .get_json(path, query)
            .await
            .map_err(|e| from_vendor_error(operation, e))?;
        unwrap_result(operation, &body)
    }

    pub async fn post(
        &self,
        operation: &str,
        path: &str,
        payload: &Value,
    ) -> Result<Value, CloudflareError> {
        let body = self
            .client
            .post_json(path, payload)
            .await
            .map_err(|e| from_vendor_error(operation, e))?;
        unwrap_result(operation, &body)
    }

    pub async fn put(
        &self,
        operation: &str,
        path: &str,
        payload: &Value,
    ) -> Result<Value, CloudflareError> {
        let body = self
            .client
            .put_json(path, payload)
            .await
            .map_err(|e| from_vendor_error(operation, e))?;
        unwrap_result(operation, &body)
    }

    pub async fn delete(&self, operation: &str, path: &str) -> Result<Value, CloudflareError> {
        let body = self
            .client
            .delete_json(path)
            .await
            .map_err(|e| from_vendor_error(operation, e))?;
        unwrap_result(operation, &body)
    }

    /// `GET /zones?name=<candidate>` for one candidate parent label.
    async fn zones_for(&self, candidate: &str) -> Result<Vec<Value>, CloudflareError> {
        let query = vec![
            ("name".to_string(), candidate.to_string()),
            ("per_page".to_string(), "50".to_string()),
        ];
        let result = self.get("zone discovery", "/zones", &query).await?;
        Ok(result.as_array().cloned().unwrap_or_default())
    }

    /// Discovers the authoritative Cloudflare zone for `domain` by walking
    /// parent labels from the hostname down to the two-label apex
    /// (`support.msp.example.com` → `msp.example.com` → `example.com`).
    ///
    /// A candidate only matches a zone whose `name` equals the candidate
    /// exactly (case-insensitive); the `active` status is preferred and a
    /// non-active match is kept for a readable error rather than thrown away.
    pub async fn discover_zone(&self, domain: &str) -> Result<DnsZone, CloudflareError> {
        let candidates = candidate_zone_names(domain)?;
        let mut tried = Vec::with_capacity(candidates.len());
        let mut inactive: Option<DnsZone> = None;

        for candidate in &candidates {
            tried.push(candidate.clone());
            for raw in self.zones_for(candidate).await? {
                let zone = parse_zone(&raw)?;
                if zone.name != *candidate {
                    continue;
                }
                if zone.status.eq_ignore_ascii_case("active") || zone.status.is_empty() {
                    return Ok(zone);
                }
                if inactive.is_none() {
                    inactive = Some(zone);
                }
            }
        }

        match inactive {
            Some(zone) => Err(CloudflareError::Rejected {
                operation: "zone discovery".to_string(),
                detail: format!(
                    "zone `{}` matches `{domain}` but is `{}` rather than `active`",
                    zone.name, zone.status
                ),
            }),
            None => Err(CloudflareError::ZoneNotFound {
                domain: domain.to_string(),
                tried,
            }),
        }
    }

    /// `GET /zones/{zone_id}/dns_records` with the given filters, returning
    /// the parsed record list.
    pub(crate) async fn dns_records(
        &self,
        zone_id: &str,
        filters: &[(String, String)],
    ) -> Result<Vec<Value>, CloudflareError> {
        let path = format!("/zones/{}/dns_records", urlencode(zone_id));
        let mut query = filters.to_vec();
        query.push(("per_page".to_string(), "100".to_string()));
        let result = self.get("dns record list", &path, &query).await?;
        Ok(result.as_array().cloned().unwrap_or_default())
    }

    /// URL-encodes a record id for a DELETE/PUT path segment.
    pub(crate) fn record_path(zone_id: &str, record_id: &str) -> String {
        format!(
            "/zones/{}/dns_records/{}",
            urlencode(zone_id),
            urlencode(record_id)
        )
    }

    /// Encodes a record content value for a `name=` query filter.
    pub(crate) fn encode_query_value(raw: &str) -> String {
        urlencode(raw)
    }
}
