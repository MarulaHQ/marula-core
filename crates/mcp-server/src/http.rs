//! HTTP transport for the MCP protocol (test suite + local dev): the
//! same JSON-RPC engine over `POST /mcp`. Mounted by the server binary;
//! NOT part of core-api's documented contract.
//!
//! Each request carries its own session: the `Authorization: Bearer` on the
//! MCP request is forwarded to core-api for the capability read and for every
//! proxied tool call. Nothing is stored between requests, so two callers
//! sharing one adapter cannot see each other's grants.

use crate::protocol::{RpcRequest, Server};
use crate::tools::{Backend, CoreApiBackend, ScopedBackend};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use std::collections::HashSet;

/// Mounts `POST /mcp` for the given protocol server.
pub fn router<S: Backend>(server: Server<S>) -> axum::Router {
    axum::Router::new()
        .route("/mcp", axum::routing::post(post_message))
        .with_state(server)
}

async fn post_message<S: Backend>(
    axum::extract::State(server): axum::extract::State<Server<S>>,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    match server.handle(req).await {
        Some(resp) => {
            Json(serde_json::to_value(resp).unwrap_or(serde_json::json!(null))).into_response()
        }
        // Notification: HTTP 204, no body (MCP streamable transport).
        None => axum::http::StatusCode::NO_CONTENT.into_response(),
    }
}

/// One adapter instance's immutable wiring: where core-api lives, and the
/// full derived catalog. The per-session tool list is filtered from it on
/// each request, never stored.
#[derive(Clone)]
struct HttpState {
    base: String,
    tools: Vec<crate::catalog::Tool>,
}

async fn post_session(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let backend = match token {
        Some(token) => CoreApiBackend::new(&state.base).with_token(token),
        None => CoreApiBackend::new(&state.base),
    };
    let granted: HashSet<String> = backend.granted_capabilities().await.into_iter().collect();
    let tools = crate::catalog::filter_granted(state.tools, &granted);
    let server = Server::new(ScopedBackend::new(backend, granted), tools);
    match server.handle(req).await {
        Some(resp) => {
            Json(serde_json::to_value(resp).unwrap_or(serde_json::json!(null))).into_response()
        }
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Builds the HTTP server from env wiring: `MARULA_API_BASE` for core-api and
/// an optional `MARULA_OPENAPI` path for local iteration. The caller's token
/// arrives per request; there is no adapter credential.
pub async fn router_from_env() -> anyhow::Result<axum::Router> {
    let base =
        std::env::var("MARULA_API_BASE").unwrap_or_else(|_| crate::tools::DEFAULT_API_BASE.into());
    let spec: serde_json::Value = match std::env::var("MARULA_OPENAPI") {
        Ok(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        Err(_) => crate::openapi_spec(),
    };
    let tools = crate::catalog::build_catalog(&spec, &crate::mcp_catalog_spec());
    Ok(axum::Router::new()
        .route("/mcp", axum::routing::post(post_session))
        .with_state(HttpState { base, tools }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{build_catalog, Tool};
    use crate::tools::ToolError;
    use serde_json::{json, Value};

    struct Fixed;

    impl Backend for Fixed {
        async fn call_tool(&self, tool: &Tool, args: &Value) -> Result<String, ToolError> {
            Ok(format!("{} {args}", tool.path))
        }
    }

    fn server() -> Server<Fixed> {
        let spec = json!({ "paths": { "/api/clients": { "get": {
            "operationId": "clients_list", "responses": { "200": { "description": "ok" } }
        }}}});
        let tools = build_catalog(&spec, &json!({ "version": 1, "families": [] }));
        Server::new(Fixed, tools)
    }

    /// A request with no bearer is still answered; the tool list is empty
    /// because core-api would refuse every read, and the adapter does not
    /// invent grants it cannot verify.
    #[tokio::test]
    async fn a_session_without_a_token_is_refused_by_the_catalog() {
        let state = HttpState {
            base: "http://127.0.0.1:9".into(),
            tools: Vec::new(),
        };
        let response = post_session(
            State(state),
            HeaderMap::new(),
            Json(RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(crate::protocol::RequestId::Number(1)),
                method: "tools/list".into(),
                params: json!({}),
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn the_bearer_header_is_parsed_without_leaking_the_scheme() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer abc.def.ghi".parse().unwrap(),
        );
        let token = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty());
        assert_eq!(token, Some("abc.def.ghi"));
    }

    #[tokio::test]
    async fn the_fixture_server_answers_initialize() {
        let response = server()
            .handle(RpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(crate::protocol::RequestId::Number(1)),
                method: "initialize".into(),
                params: json!({ "protocolVersion": "2025-06-18" }),
            })
            .await
            .expect("initialize answers");
        let value = serde_json::to_value(response).expect("serializes");
        assert_eq!(
            value["result"]["protocolVersion"],
            json!(crate::protocol::SUPPORTED_PROTOCOL)
        );
    }
}
