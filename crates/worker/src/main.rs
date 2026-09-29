//! `worker` binary entrypoint: loads env config, initializes tracing, and
//! runs the relay service until shutdown.
//!
//! Purpose: process shell only — behavior lives in the library so
//! `marula-core-cli` can host the identical service.
//!
//! Non-purpose: no CLI flags or config files; environment variables are
//! the single configuration surface.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared telemetry (TASKLIST 2.18): none-mode unless
    // OTEL_EXPORTER_OTLP_ENDPOINT is set; guard lives for `main`.
    let _telemetry = core_otel::init("worker");
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });
    worker::run(worker::config::WorkerConfig::from_env(), shutdown_rx).await
}
