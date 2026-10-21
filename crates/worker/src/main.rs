//! `worker` binary entrypoint: loads env config, initializes tracing, and
//! runs the relay service until shutdown.
//!
//! Purpose: process shell only — behavior lives in the library so
//! `marula-core-cli` can host the identical service.
//!
//! Non-purpose: no CLI flags or config files; environment variables are
//! the single configuration surface.
//!
//! Shutdown is installed BEFORE the config loads and before Postgres or Redis
//! are touched: a container runtime that stops this process during a slow boot
//! (a migration running first, a database still coming up) gets a clean exit
//! rather than a default SIGTERM kill. `worker::shutdown` owns the contract.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared telemetry (the private build tracker 2.18): none-mode unless
    // OTEL_EXPORTER_OTLP_ENDPOINT is set; guard lives for `main`.
    let _telemetry = core_otel::init("worker");
    // SIGINT and SIGTERM both start the bounded drain. The old handler waited
    // on `ctrl_c` only, which is SIGINT — the signal a container runtime never
    // sends, so every rolling stop was a hard kill.
    let shutdown = worker::shutdown::install();
    let config = worker::config::WorkerConfig::from_env();
    worker::run(config, shutdown).await
}
