//! Core-api client used by the adapter. It holds no credential: every call
//! carries the token of the caller whose MCP request is being served.

use serde::Deserialize;
use serde_json::Value;

/// The org context `GET /api/v1/me` answers with. Only the fields the
/// adapter needs are typed; the rest stays on the wire.
#[derive(Debug, Deserialize)]
pub struct SessionContext {
    pub org_id: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub mcp_capabilities: Vec<String>,
}

#[derive(Debug)]
pub struct UpstreamError {
    pub status: u16,
    pub body: String,
}

pub struct CoreApi {
    pub base_url: String,
    pub client: reqwest::Client,
}

impl Clone for CoreApi {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            client: self.client.clone(),
        }
    }
}

impl CoreApi {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// The caller's session context, or `None` when the token is refused or the
    /// api is unreachable. A caller with no context sees no tools.
    pub async fn session(&self, token: &str) -> Option<SessionContext> {
        let response = self
            .client
            .get(format!("{}/api/v1/me", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<SessionContext>().await.ok()
    }

    /// Proxies one read. `path` already carries the resolved parameters and
    /// `query` the encoded filters.
    pub async fn get(&self, path: &str, query: &str, token: &str) -> Result<Value, UpstreamError> {
        let response = self
            .client
            .get(format!("{}{}{}", self.base_url, path, query))
            .bearer_auth(token)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|error| UpstreamError {
                status: 0,
                body: error.to_string(),
            })?;
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(UpstreamError { status, body });
        }
        serde_json::from_str(&body).map_err(|error| UpstreamError {
            status,
            body: format!("response is not JSON: {error}"),
        })
    }
}
