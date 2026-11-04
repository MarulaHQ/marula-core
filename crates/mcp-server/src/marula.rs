//! The MCP service: the catalog, the capability filter, and the proxy.
//!
//! Built on `rmcp`'s `ServerHandler`, so framing, version negotiation and
//! transport semantics belong to the SDK. What stays here is Marula's:
//! which tools exist (derived from the committed OpenAPI snapshot, filtered by
//! the org's grant registry) and who may call them (the caller's own session,
//! forwarded to core-api, which authorizes every read).

use crate::api::CoreApi;
use crate::catalog::{self, Tool as CatalogTool};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ContentBlock, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool as McpTool,
    },
    service::RequestContext,
    ErrorData as McpError, RoleServer, ServerHandler,
};
use std::collections::HashSet;
use std::sync::Arc;

/// Requests carry the caller's bearer either in the HTTP request parts or, for
/// stdio, in the environment the adapter was started with.
#[derive(Clone)]
pub struct MarulaServer {
    api: CoreApi,
    tools: Arc<Vec<CatalogTool>>,
    stdio_token: Option<String>,
}

impl MarulaServer {
    pub fn new(
        base_url: &str,
        spec: &serde_json::Value,
        catalog_spec: &serde_json::Value,
        stdio_token: Option<String>,
    ) -> Self {
        Self {
            api: CoreApi::new(base_url),
            tools: Arc::new(catalog::build_catalog(spec, catalog_spec)),
            stdio_token,
        }
    }

    /// The full derived catalog, for tests and diagnostics.
    pub fn catalog(&self) -> &[CatalogTool] {
        &self.tools
    }

    fn bearer(&self, context: &RequestContext<RoleServer>) -> Option<String> {
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            if let Some(value) = parts
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
            {
                if let Some(token) = value.strip_prefix("Bearer ") {
                    let token = token.trim();
                    if !token.is_empty() {
                        return Some(token.to_string());
                    }
                }
            }
        }
        self.stdio_token.clone()
    }

    /// Granted capability namespaces for this session. An unreadable context is
    /// an empty set, so deny-by-default holds on every failure mode.
    async fn granted(&self, token: Option<&str>) -> HashSet<String> {
        let Some(token) = token else {
            return HashSet::new();
        };
        let Some(session) = self.api.session(token).await else {
            return HashSet::new();
        };
        session
            .mcp_capabilities
            .into_iter()
            .filter_map(|capability| {
                capability
                    .strip_suffix(":read")
                    .map(|namespace| namespace.to_string())
            })
            .collect()
    }

    fn visible(&self, granted: &HashSet<String>) -> Vec<&CatalogTool> {
        self.tools
            .iter()
            .filter(|tool| {
                tool.capability
                    .strip_suffix(":read")
                    .is_some_and(|namespace| granted.contains(namespace))
            })
            .collect()
    }

    fn as_mcp_tool(tool: &CatalogTool) -> McpTool {
        let schema = tool.input_schema.as_object().cloned().unwrap_or_else(|| {
            let mut fallback = serde_json::Map::new();
            fallback.insert("type".into(), serde_json::json!("object"));
            fallback.insert("properties".into(), serde_json::json!({}));
            fallback
        });
        let mut mcp = McpTool::new(
            tool.name.clone(),
            tool.description.clone(),
            Arc::new(schema),
        );
        mcp.annotations = None;
        mcp
    }
}

impl ServerHandler for MarulaServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("marula-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Read tools derived from the Marula Core API. A tool exists only when \
                 this organization has granted its capability, and every call runs as \
                 the caller's own session.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let token = self.bearer(&context);
        let granted = self.granted(token.as_deref()).await;
        let tools = self
            .visible(&granted)
            .into_iter()
            .map(Self::as_mcp_tool)
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        let Some(token) = self.bearer(&context) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "this request carries no session token; the adapter holds no credential of its own",
            )])
            .into());
        };
        let Some(tool) = self
            .tools
            .iter()
            .find(|tool| tool.name == request.name.as_ref())
        else {
            return Err(McpError::invalid_params(
                format!("unknown tool: {}", request.name),
                None,
            ));
        };
        let granted = self.granted(Some(&token)).await;
        let namespace = tool
            .capability
            .strip_suffix(":read")
            .unwrap_or_default()
            .to_string();
        if !granted.contains(&namespace) {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "capability {} is not granted for this organization (deny-by-default registry)",
                tool.capability
            ))])
            .into());
        }
        let arguments = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or_else(|| serde_json::json!({}));
        let path = match catalog::resolve_path(tool, &arguments) {
            Ok(path) => path,
            Err(message) => return Err(McpError::invalid_params(message, None)),
        };
        let query = catalog::query_string(tool, &arguments);
        match self.api.get(&path, &query, &token).await {
            Ok(value) => {
                let text = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
                Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into())
            }
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "core-api answered {}: {}",
                error.status, error.body
            ))])
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> MarulaServer {
        let spec = serde_json::json!({ "paths": {
            "/api/clients": { "get": {
                "operationId": "clients_list",
                "summary": "List clients",
                "responses": { "200": { "description": "ok" } }
            }},
            "/api/tickets": { "get": {
                "operationId": "tickets_list",
                "summary": "List tickets",
                "responses": { "200": { "description": "ok" } }
            }}
        }});
        let catalog = serde_json::json!({ "version": 1, "families": [
            { "namespace": "clients", "capability": "clients:read", "tools": [
                { "name": "clients_list", "path": "/api/clients", "capability": "clients:read" }
            ]},
            { "namespace": "tickets", "capability": "tickets:read", "tools": [
                { "name": "tickets_list", "path": "/api/tickets", "capability": "tickets:read" }
            ]}
        ]});
        MarulaServer::new("http://127.0.0.1:9", &spec, &catalog, None)
    }

    #[test]
    fn the_catalog_is_derived_only_from_granted_families() {
        let server = server();
        assert_eq!(server.catalog().len(), 2);
        let granted: HashSet<String> = ["clients".to_string()].into_iter().collect();
        let visible: Vec<&str> = server
            .visible(&granted)
            .into_iter()
            .map(|tool| tool.name.as_str())
            .collect();
        assert_eq!(visible, vec!["clients_list"]);
    }

    #[test]
    fn no_grants_means_no_tools() {
        let server = server();
        assert!(server.visible(&HashSet::new()).is_empty());
    }

    #[test]
    fn the_advertised_protocol_is_the_sdk_default_not_an_echo() {
        let info = server().get_info();
        assert_eq!(info.protocol_version, rmcp::model::ProtocolVersion::LATEST);
        assert!(info.capabilities.tools.is_some());
    }
}
