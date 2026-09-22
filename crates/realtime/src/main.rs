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
    // Fail-closed boot (Wave 19 lane 4 GA audit): production refuses to run
    // without an explicit JWT_SECRET; the dev fallback rides
    // MERIDIAN_DEV_TOOLS=1 (see RealtimeConfig::try_from_env).
    let config = realtime::config::RealtimeConfig::try_from_env()
        .map_err(anyhow::Error::msg)
        .expect("realtime configuration fail-closed (try_from_env)");
    realtime::run(config).await
}
