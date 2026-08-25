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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    realtime::run(realtime::config::RealtimeConfig::from_env()).await
}
