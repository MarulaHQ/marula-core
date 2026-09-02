//! stdio JSON-RPC transport: the canonical MCP transport. One JSON-RPC
//! message per line (newline-delimited), responses echoed to stdout,
//! telemetry to stderr.

use crate::protocol::{RpcRequest, Server};
use crate::tools::{Backend, ScopedBackend};
use std::io::{BufRead, Write};
use tokio::sync::mpsc;

/// Serves one stdio session to completion. `granted_namespaces` pre-filters
/// the catalog (capability registry); every tool call additionally
/// re-checks core-side via the caller's credentials.
pub async fn serve<S: Backend>(server: Server<S>) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    // Blocking stdin reads on a dedicated thread; line-delimited frames.
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            match lock.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line.trim().to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    while let Some(line) = rx.recv().await {
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => {
                let Some(resp) = server.handle(req).await else {
                    continue;
                };
                resp
            }
            Err(e) => {
                // Unparseable frame: JSON-RPC id-less parse error.
                crate::protocol::RpcResponse::err(
                    crate::protocol::RequestId::Number(0),
                    -32700,
                    format!("parse error: {e}"),
                )
            }
        };
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

/// Builds the stdio server from environment wiring:
/// - `MERIDIAN_API_BASE` — core-api base URL (default loopback:8080);
/// - `MERIDIAN_API_KEY` — the machine key whose capability grants scope
///   the catalog;
/// - `MERIDIAN_OPENAPI` — optional spec path override for local iteration
///   (default: the snapshot embedded at build time).
pub async fn serve_from_env() -> anyhow::Result<()> {
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
    let server = Server::new(scoped, tools);
    serve(server).await
}
