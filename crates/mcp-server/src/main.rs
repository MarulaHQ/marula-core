//! mcp-server binary: MCP protocol adapter over core-api.
//!
//! Transports:
//! - `mcp-server` (no args) — stdio JSON-RPC, the canonical MCP transport;
//! - `mcp-server serve-http` — JSON-RPC over HTTP at `/mcp` for local dev.
//!
//! Env wiring: `MARULA_API_BASE` (core-api URL), `MARULA_API_TOKEN`
//! (the caller's session token, forwarded on every read; the adapter holds
//! no credential of its own), `MARULA_OPENAPI` (spec snapshot path override).
//!
//! Reads only: write tools are Stage 4–5; agent-authored changes enter
//! through the IF8 proposal pipeline, never a direct MCP write.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    match std::env::args().nth(1).as_deref() {
        None => mcp_server::stdio::serve_from_env().await,
        Some("serve-http") => {
            let port: u16 = std::env::var("MARULA_MCP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8190);
            let app = mcp_server::http::router_from_env().await?;
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
            tracing::info!(port, "mcp-server http transport listening");
            axum::serve(listener, app).await?;
            Ok(())
        }
        Some(other) => {
            anyhow::bail!("unknown subcommand {other:?}; usage: mcp-server [serve-http]")
        }
    }
}
