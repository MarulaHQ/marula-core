//! HTTP transport: `rmcp`'s Streamable HTTP service mounted at `/mcp`.
//!
//! Stateless by construction. The SDK serves modern protocol revisions
//! statelessly per SEP-2567, and `legacy_session_mode(false)` gives legacy
//! clients the same treatment, so no session state survives a request. The
//! caller's bearer is read out of the request parts the SDK hands the handler.

use crate::marula::MarulaServer;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};

pub fn router(server: MarulaServer) -> Router {
    router_with(StreamableHttpServerConfig::default(), server)
}

fn router_with(config: StreamableHttpServerConfig, server: MarulaServer) -> Router {
    let config = config
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        config,
    );
    Router::new().nest_service("/mcp", service)
}

/// Builds the router from env wiring: `MARULA_API_BASE` for core-api and an
/// optional `MARULA_OPENAPI` path for local iteration. There is no adapter
/// credential; every request carries the caller's.
pub async fn router_from_env() -> anyhow::Result<Router> {
    let base = std::env::var("MARULA_API_BASE").unwrap_or_else(|_| crate::DEFAULT_API_BASE.into());
    let spec: serde_json::Value = match std::env::var("MARULA_OPENAPI") {
        Ok(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        Err(_) => crate::openapi_spec(),
    };
    let server = MarulaServer::new(&base, &spec, &crate::mcp_catalog_spec(), None);
    // The SDK accepts loopback hosts by default, which is the safe default for
    // a local adapter. A deployment behind a real hostname names itself here.
    let mut config = StreamableHttpServerConfig::default();
    if let Ok(hosts) = std::env::var("MARULA_MCP_ALLOWED_HOSTS") {
        let hosts: Vec<String> = hosts
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        if !hosts.is_empty() {
            config = config.with_allowed_hosts(hosts);
        }
    }
    Ok(router_with(config, server))
}
