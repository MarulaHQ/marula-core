//! HTTP transport for the MCP protocol (test suite + local dev): the
//! same JSON-RPC engine over `POST /mcp`. Mounted by the server binary;
//! NOT part of core-api's documented contract.

use crate::protocol::{RpcRequest, Server};
use crate::tools::{Backend, ScopedBackend};
use axum::{response::IntoResponse, Json};

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

/// Builds the HTTP server from env wiring (same contract as stdio).
pub async fn router_from_env() -> anyhow::Result<axum::Router> {
    let base = std::env::var("MERIDIAN_API_BASE")
        .unwrap_or_else(|_| crate::tools::DEFAULT_API_BASE.into());
    let backend = crate::tools::CoreApiBackend::new(&base);
    let spec: serde_json::Value = match std::env::var("MERIDIAN_OPENAPI") {
        Ok(path) => serde_json::from_str(&std::fs::read_to_string(&path)?)?,
        Err(_) => crate::openapi_spec(),
    };
    let granted = match std::env::var("MERIDIAN_API_KEY") {
        Ok(key) => backend.granted_capabilities(&key).await,
        Err(_) => Vec::new(),
    };
    let granted_set: std::collections::HashSet<String> = granted.into_iter().collect();
    let scoped = ScopedBackend::new(backend, granted_set.clone());
    let tools = crate::catalog::filter_granted(crate::catalog::build_catalog(&spec), &granted_set);
    Ok(router(Server::new(scoped, tools)))
}
