//! `realtime` binary entrypoint: loads env config, initializes tracing,
//! and runs the hub until shutdown.
//!
//! Purpose: the process shell only — all behavior lives in the library so
//! `meridian-cli` can host the same service.
//!
//! Non-purpose: no CLI flags or config files; environment variables are
//! the single configuration surface.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared telemetry (TASKLIST 2.18): none-mode unless
    // OTEL_EXPORTER_OTLP_ENDPOINT is set; guard lives for `main`.
    let _telemetry = core_otel::init("realtime");
    realtime::run(realtime::config::RealtimeConfig::from_env()).await
}
