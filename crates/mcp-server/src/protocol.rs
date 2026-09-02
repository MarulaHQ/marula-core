//! MCP JSON-RPC protocol envelope (2025-03-26 streamable flavor, v1
//! subset): initialize, tools/list, tools/call, ping. Transport-agnostic:
//! the stdio and HTTP transports share [`Server`].

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The request id as MCP allows (number or string).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// One inbound JSON-RPC request or notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// One outbound JSON-RPC response.
#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub jsonrpc: &'static str,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcResponse {
    pub fn ok(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: RequestId, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// JSON-RPC error codes used by this adapter.
pub const CODE_METHOD_NOT_FOUND: i64 = -32601;
pub const CODE_INVALID_PARAMS: i64 = -32602;
pub const CODE_CAPABILITY_DENIED: i64 = -32001;
pub const CODE_UPSTREAM: i64 = -32002;

/// Transport-neutral protocol engine: request in, response out. `S` is
/// the tool-calling backend (the core-api proxy). Cheaply clonable (the
/// backend rides an `Arc`) so the HTTP transport can hold it as state.
pub struct Server<S: super::tools::Backend> {
    backend: std::sync::Arc<S>,
    tools: Vec<super::catalog::Tool>,
    server_info: Value,
}

impl<S: super::tools::Backend> Clone for Server<S> {
    fn clone(&self) -> Self {
        Self {
            backend: std::sync::Arc::clone(&self.backend),
            tools: self.tools.clone(),
            server_info: self.server_info.clone(),
        }
    }
}

impl<S: super::tools::Backend> Server<S> {
    pub fn new(backend: S, tools: Vec<super::catalog::Tool>) -> Self {
        Self {
            backend: std::sync::Arc::new(backend),
            tools,
            server_info: json!({
                "name": "meridian-mcp",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        }
    }

    pub fn tools(&self) -> &[super::catalog::Tool] {
        &self.tools
    }

    /// Handles one request; `None` for notifications (no id, no reply).
    pub async fn handle(&self, req: RpcRequest) -> Option<RpcResponse> {
        let id = req.id?;
        match req.method.as_str() {
            "initialize" => Some(RpcResponse::ok(
                id,
                json!({
                    "protocolVersion": req.params.get("protocolVersion").cloned().unwrap_or(json!("2025-03-26")),
                    "capabilities": { "tools": {} },
                    "serverInfo": self.server_info,
                }),
            )),
            "ping" => Some(RpcResponse::ok(id, json!({}))),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "inputSchema": t.input_schema,
                        })
                    })
                    .collect();
                Some(RpcResponse::ok(id, json!({ "tools": tools })))
            }
            "tools/call" => {
                let Some(name) = req.params.get("name").and_then(Value::as_str) else {
                    return Some(RpcResponse::err(
                        id,
                        CODE_INVALID_PARAMS,
                        "missing tool name",
                    ));
                };
                let args = req
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let Some(tool) = self.tools.iter().find(|t| t.name == name) else {
                    return Some(RpcResponse::err(
                        id,
                        CODE_METHOD_NOT_FOUND,
                        format!("unknown tool: {name}"),
                    ));
                };
                match self.backend.call_tool(tool, &args).await {
                    Ok(content) => Some(RpcResponse::ok(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": content }],
                            "isError": false,
                        }),
                    )),
                    Err(e) => Some(RpcResponse::err(id, e.code, e.message)),
                }
            }
            other => Some(RpcResponse::err(
                id,
                CODE_METHOD_NOT_FOUND,
                format!("unknown method: {other}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::build_catalog;
    use crate::tools::{Backend, ToolError};
    use serde_json::json;

    struct FakeBackend;

    impl Backend for FakeBackend {
        async fn call_tool(
            &self,
            tool: &crate::catalog::Tool,
            args: &Value,
        ) -> Result<String, ToolError> {
            let path = crate::catalog::resolve_path(tool, args).map_err(|m| ToolError {
                code: CODE_INVALID_PARAMS,
                message: m,
            })?;
            Ok(format!("{path} {args}"))
        }
    }

    fn spec() -> Value {
        json!({ "paths": {
            "/api/clients/{id}": { "get": {
                "operationId": "clients_get_one",
                "summary": "Get one client",
                "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}]
            }}
        }})
    }

    fn rpc(id: i64, method: &str, params: Value) -> RpcRequest {
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(RequestId::Number(id)),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn initialize_hanshake() {
        let server = Server::new(FakeBackend, build_catalog(&spec()));
        let resp = server
            .handle(rpc(1, "initialize", json!({})))
            .await
            .unwrap();
        assert_eq!(
            resp.result.as_ref().unwrap()["serverInfo"]["name"],
            "meridian-mcp"
        );
    }

    #[tokio::test]
    async fn tools_call_proxies_and_errors() {
        let server = Server::new(FakeBackend, build_catalog(&spec()));
        let resp = server
            .handle(rpc(
                2,
                "tools/call",
                json!({ "name": "clients_get_one", "arguments": { "id": "abc" } }),
            ))
            .await
            .unwrap();
        assert!(resp.error.is_none());
        let text = resp.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.starts_with("/api/clients/abc"));

        let missing = server
            .handle(rpc(
                3,
                "tools/call",
                json!({ "name": "clients_get_one", "arguments": {} }),
            ))
            .await
            .unwrap();
        assert_eq!(missing.error.unwrap().code, CODE_INVALID_PARAMS);
    }

    #[tokio::test]
    async fn unknown_tool_and_method() {
        let server = Server::new(FakeBackend, build_catalog(&spec()));
        let resp = server
            .handle(rpc(4, "tools/call", json!({ "name": "nope" })))
            .await
            .unwrap();
        assert_eq!(resp.error.unwrap().code, CODE_METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn notifications_have_no_reply() {
        let server = Server::new(FakeBackend, build_catalog(&spec()));
        let req = RpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: json!({}),
        };
        assert!(server.handle(req).await.is_none());
    }
}
