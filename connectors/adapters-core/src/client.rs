//! The vendor HTTP client: one base URL, one auth flow, native
//! pagination flows, typed errors.

use serde_json::Value;

use crate::auth::VendorAuth;

/// How a vendor pages a list endpoint. A pull follows EVERY page — a
/// silently truncated inventory is a wrong inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageFlow {
    /// Continuation URLs in the body (`{"@odata.nextLink": "..."}` —
    /// M365 Graph). The first request targets `{base}{path}`; each
    /// subsequent one targets the returned URL verbatim.
    NextLink {
        /// Body key carrying the continuation URL (`@odata.nextLink`).
        key: &'static str,
        /// Body key carrying the item array (`value` for Graph).
        items_key: &'static str,
    },
    /// JSON:API style: `links.next.href` when more pages exist (IT
    /// Glue), with `page[number]` request parameters.
    JsonApi {
        items_key: &'static str,
        page_param: &'static str,
        page_size: usize,
    },
    /// Plain page/size parameters; the pull stops at the first page
    /// with fewer items than the page size (ImmyBot, Hudu).
    PageNumber {
        items_key: &'static str,
        page_param: &'static str,
        size_param: Option<&'static str>,
        page_size: usize,
    },
}

/// One fetched page: the item array plus the raw body (vendors tuck
/// metadata — totals, page counts — next to the items).
#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<Value>,
    pub body: Value,
}

/// The vendor HTTP client. Construct per vendor, clone freely — the
/// inner reqwest client pools connections.
#[derive(Clone, Debug)]
pub struct VendorClient {
    /// Vendor API base URL (no trailing slash).
    pub base_url: String,
    pub auth: VendorAuth,
    http: reqwest::Client,
}

impl VendorClient {
    pub fn new(base_url: impl Into<String>, auth: VendorAuth) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            auth,
            http: reqwest::Client::new(),
        }
    }

    /// GET `{base_url}{path}` with `query` applied, returning the raw
    /// JSON body.
    pub async fn get_json(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Value, VendorError> {
        let url = format!("{}{}", self.base_url, path);
        let request = self.http.get(&url).query(query);
        let request = self.auth.apply(request, &self.http).await?;
        let response = request
            .send()
            .await
            .map_err(|e| VendorError::Network(format!("{url}: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else {
            Err(VendorError::from_status(status, &body))
        }
    }

    fn items_of(body: &Value, items_key: &str) -> Vec<Value> {
        match body.get(items_key) {
            Some(Value::Array(items)) => items.clone(),
            // Bare-array bodies (some vendor endpoints) are their own list.
            Some(_) => Vec::new(),
            None if body.is_array() => body.as_array().cloned().unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Follows the page flow to exhaustion, returning every item
    /// across every page in vendor order.
    pub async fn fetch_all(&self, path: &str, flow: &PageFlow) -> Result<Vec<Value>, VendorError> {
        let mut items = Vec::new();
        match flow {
            PageFlow::NextLink { key, items_key } => {
                let mut next: Option<String> = Some(path.to_string());
                let mut guard = 0usize;
                while let Some(target) = next {
                    // Relative continuations resolve against the base;
                    // absolute ones (Graph ships full URLs) are used as-is.
                    let body = if target.starts_with("http") {
                        self.get_absolute(&target).await?
                    } else {
                        self.get_json(&target, &[]).await?
                    };
                    items.extend(Self::items_of(&body, items_key));
                    next = body.get(*key).and_then(Value::as_str).map(str::to_string);
                    guard += 1;
                    // A hard stop, never hit by honest vendors: a
                    // self-perpetuating nextLink must not spin forever.
                    if next.is_some() && guard > 10_000 {
                        return Err(VendorError::Rejected(format!(
                            "pagination exceeded 10,000 pages at {target}"
                        )));
                    }
                }
            }
            PageFlow::JsonApi {
                items_key,
                page_param,
                page_size,
            } => {
                // `links.next` is the VENDOR'S signal — our page size
                // never overrides it (the vendor's actual page size may
                // differ from ours).
                let mut page = 1usize;
                loop {
                    let query = vec![((*page_param).to_string(), page.to_string())];
                    let body = self.get_json(path, &query).await?;
                    let page_items = Self::items_of(&body, items_key);
                    let reached_end = Self::jsonapi_next_href(&body).is_none();
                    items.extend(page_items);
                    if reached_end {
                        break;
                    }
                    page += 1;
                    if page > 10_000 {
                        return Err(VendorError::Rejected(format!(
                            "pagination exceeded 10,000 pages at {path}"
                        )));
                    }
                    let _ = page_size;
                }
            }
            PageFlow::PageNumber {
                items_key,
                page_param,
                size_param,
                page_size,
            } => {
                let mut page = 1usize;
                loop {
                    let mut query = vec![((*page_param).to_string(), page.to_string())];
                    if let Some(size_param) = size_param {
                        query.push(((*size_param).to_string(), page_size.to_string()));
                    }
                    let body = self.get_json(path, &query).await?;
                    let page_items = Self::items_of(&body, items_key);
                    let reached_end = page_items.len() < *page_size;
                    items.extend(page_items);
                    if reached_end {
                        break;
                    }
                    page += 1;
                    // Same hard stop as the other flows: a vendor that
                    // ignores the page parameter would re-serve page 1
                    // forever.
                    if page > 10_000 {
                        return Err(VendorError::Rejected(format!(
                            "pagination exceeded 10,000 pages at {path}"
                        )));
                    }
                }
            }
        }
        Ok(items)
    }

    /// GET an absolute continuation URL, applying auth. Continuation
    /// URLs come from the vendor's response BODY — a compromised or
    /// manipulated response must not be able to redirect the credential
    /// to a third-party host, so the URL is validated against the
    /// configured base (same scheme + host) before anything is sent.
    async fn get_absolute(&self, url: &str) -> Result<Value, VendorError> {
        let parsed = url
            .parse::<reqwest::Url>()
            .map_err(|_| VendorError::Rejected(format!("malformed continuation URL: {url}")))?;
        let base = self
            .base_url
            .parse::<reqwest::Url>()
            .map_err(|_| VendorError::Rejected(format!("malformed base URL: {}", self.base_url)))?;
        if parsed.scheme() != base.scheme()
            || parsed.host_str() != base.host_str()
            || parsed.port_or_known_default() != base.port_or_known_default()
        {
            return Err(VendorError::Rejected(format!(
                "continuation URL {url} leaves the vendor base {}",
                self.base_url
            )));
        }
        let request = self.http.get(parsed);
        let request = self.auth.apply(request, &self.http).await?;
        let response = request
            .send()
            .await
            .map_err(|e| VendorError::Network(format!("{url}: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else {
            Err(VendorError::from_status(status, &body))
        }
    }

    /// IT Glue JSON:API: `{"links": {"next": {"href": "..."}}}` — a
    /// present `links.next` (href present or not) means another page.
    fn jsonapi_next_href(body: &Value) -> Option<&Value> {
        body.get("links").and_then(|links| links.get("next"))
    }
}

/// What a vendor API did to a pull. Mirrors the lane-8 sink error
/// vocabulary so connector run reports read uniformly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VendorError {
    /// The vendor refused the credential (401/403) — the operator
    /// rotates the key/token.
    #[error("vendor unauthorized: {0}")]
    Unauthorized(String),
    #[error("vendor rate limited: {0}")]
    RateLimited(String),
    /// The vendor answered but the request/payload was refused.
    #[error("vendor rejected the request: {0}")]
    Rejected(String),
    #[error("vendor transport failure: {0}")]
    Network(String),
}

impl VendorError {
    /// Maps an HTTP status + response body onto the typed surface.
    pub fn from_status(status: reqwest::StatusCode, body: &serde_json::Value) -> Self {
        let message = body
            .get("message")
            .or_else(|| body.get("error"))
            .or_else(|| body.get("error_description"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                // Graph nests errors: {"error": {"code": ..., "message": ...}}
                body.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
            })
            .unwrap_or("unspecified")
            .to_string();
        match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                VendorError::Unauthorized(message)
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS => VendorError::RateLimited(message),
            code => VendorError::Rejected(format!("{code}: {message}")),
        }
    }
}
