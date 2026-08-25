//! `worker` binary entrypoint: loads env config, initializes tracing, and
//! runs the relay service until shutdown.
//!
//! Purpose: process shell only — behavior lives in the library so
//! `meridian-cli` can host the identical service.
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
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown_tx.send(true);
    });
    worker::run(worker::config::WorkerConfig::from_env(), shutdown_rx).await
}
