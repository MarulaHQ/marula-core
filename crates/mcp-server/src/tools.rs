//! Tool execution: proxy each derived read tool onto core-api with the
//! caller's credentials. The adapter holds NO database connection — the
//! request carries the `x-api-key` (or bearer) it was authenticated with,
//! and core-api re-checks scopes and RLS (defense in depth).

use crate::catalog::{query_string, resolve_path, Tool};
use serde_json::Value;

/// The core-api base URL the adapter proxies onto.
pub const DEFAULT_API_BASE: &str = "http://127.0.0.1:8080";

/// A tool-call failure mapped to a JSON-RPC error.
#[derive(Debug)]
pub struct ToolError {
    pub code: i64,
    pub message: String,
}

impl ToolError {
    pub fn capability_denied(capability: &str) -> Self {
        Self {
            code: crate::protocol::CODE_CAPABILITY_DENIED,
            message: format!(
                "capability {capability} is not granted for this org (deny-by-default registry)"
            ),
        }
    }

    pub fn upstream(status: u16, body: String) -> Self {
        Self {
            code: crate::protocol::CODE_UPSTREAM,
            message: format!("core-api returned {status}: {body}"),
        }
    }
}

/// Async-trait-free backend abstraction (one implementor per transport
/// wiring; sealed by usage in [`crate::protocol::Server`]).
pub trait Backend: Send + Sync + 'static {
    fn call_tool<'a>(
        &'a self,
        tool: &'a Tool,
        args: &'a Value,
    ) -> impl std::future::Future<Output = Result<String, ToolError>> + Send + 'a;
}

/// Proxy backend over the core-api HTTP surface.
pub struct CoreApiBackend {
    pub base_url: String,
    pub client: reqwest::Client,
}

impl CoreApiBackend {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Resolves the org's granted capabilities from the registry via the
    /// admin surface (the adapter's service key), for `tools/list`
    /// filtering. An unreachable registry degrades to NO capabilities —
    /// deny-by-default holds.
    pub async fn granted_capabilities(&self, service_key: &str) -> Vec<String> {
        let Ok(resp) = self
            .client
            .get(format!("{}/api/v1/mcp/capabilities", self.base_url))
            .header("x-api-key", service_key)
            .send()
            .await
        else {
            return Vec::new();
        };
        let Ok(body) = resp.json::<Value>().await else {
            return Vec::new();
        };
        body.get("grants")
            .and_then(Value::as_array)
            .map(|grants| {
                grants
                    .iter()
                    .filter_map(|g| g.get("capability").and_then(Value::as_str))
                    .filter_map(|c| c.strip_suffix(":read").map(|ns| ns.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Backend for CoreApiBackend {
    async fn call_tool(&self, tool: &Tool, args: &Value) -> Result<String, ToolError> {
        let path = resolve_path(tool, args).map_err(|m| ToolError {
            code: crate::protocol::CODE_INVALID_PARAMS,
            message: m,
        })?;
        let query = query_string(tool, args);
        let url = format!("{}{}{}", self.base_url, path, query);
        let resp = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| ToolError::upstream(0, e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| ToolError::upstream(status, e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(ToolError::upstream(status, body));
        }
        Ok(body)
    }
}

/// A backend whose catalog is pre-filtered to granted capabilities;
/// denies anything not in the allowlist BEFORE proxying (the registry
/// decision), then delegates to the HTTP backend.
pub struct ScopedBackend {
    pub inner: CoreApiBackend,
    /// Granted capability NAMESPACES (read side). Empty = deny all.
    pub granted: std::collections::HashSet<String>,
}

impl ScopedBackend {
    pub fn new(
        inner: CoreApiBackend,
        granted_namespaces: std::collections::HashSet<String>,
    ) -> Self {
        Self {
            inner,
            granted: granted_namespaces,
        }
    }
}

impl Backend for ScopedBackend {
    async fn call_tool(&self, tool: &Tool, args: &Value) -> Result<String, ToolError> {
        let namespace = tool
            .capability
            .strip_suffix(":read")
            .unwrap_or_default()
            .to_string();
        if !self.granted.contains(&namespace) {
            return Err(ToolError::capability_denied(&tool.capability));
        }
        self.inner.call_tool(tool, args).await
    }
}
