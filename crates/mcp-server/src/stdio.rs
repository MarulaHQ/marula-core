//! stdio transport: `rmcp`'s io service over stdin/stdout.
//!
//! The caller's session token arrives as `MARULA_API_TOKEN`. Access tokens are
//! short-lived, so a long session restarts the adapter with a fresh one rather
//! than the adapter keeping a refresh chain it would then have to protect.

use crate::marula::MarulaServer;
use rmcp::{transport::stdio, ServiceExt};

pub async fn serve_from_env() -> anyhow::Result<()> {
    let base = std::env::var("MARULA_API_BASE").unwrap_or_else(|_| crate::DEFAULT_API_BASE.into());
    let token = std::env::var("MARULA_API_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    let spec: serde_json::Value = match std::env::var("MARULA_OPENAPI") {
        Ok(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        Err(_) => crate::openapi_spec(),
    };
    let server = MarulaServer::new(&base, &spec, &crate::mcp_catalog_spec(), token);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
