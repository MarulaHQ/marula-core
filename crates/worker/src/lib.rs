//! worker — the background process host (Stage 1 scope).
//!
//! Purpose: run the outbox→Redis relay (`core_events::run_relay`) as the
//! primary loop and expose a `/healthz` endpoint that reflects relay
//! liveness. `meridian-cli` hosts this same library via
//! `meridian serve-worker`.
//!
//! Non-purpose: no general job queue, email, SLA evaluation, or webhook
//! delivery — those are Stage 2 scope — and no direct database writes of
//! its own beyond what the relay's mark-published step performs.

pub mod config;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use core_events::Publisher;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Runs the worker until `shutdown` fires: connects to Postgres and Redis,
/// spawns the relay loop, and serves `/healthz` until then.
pub async fn run(
    config: config::WorkerConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let pool = core_db::connect(&config.database_url).await?;
    let publisher = Publisher::connect(&config.redis_url).await?;
    tracing::info!("worker connected; starting event relay");

    let relay_alive = Arc::new(AtomicBool::new(true));
    let relay = tokio::spawn(core_events::run_relay(pool, publisher, config.relay_batch));
    {
        // Flip the health flag if the loop ever exits (it only can by
        // panicking); /healthz then reports unhealthy so the orchestrator
        // restarts the container.
        let relay_alive = Arc::clone(&relay_alive);
        tokio::spawn(async move {
            let _ = relay.await;
            relay_alive.store(false, Ordering::Relaxed);
            tracing::error!("relay loop exited; reporting unhealthy");
        });
    }

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(relay_alive);
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "worker health endpoint listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;
    Ok(())
}

async fn healthz(State(alive): State<Arc<AtomicBool>>) -> Json<serde_json::Value> {
    if alive.load(Ordering::Relaxed) {
        Json(json!({ "status": "ok", "relay": "alive" }))
    } else {
        Json(json!({ "status": "unhealthy", "relay": "stopped" }))
    }
}

/// Resolves when the shared flag flips true (or the channel closes).
async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn healthz_reflects_the_liveness_flag() {
        let alive = Arc::new(AtomicBool::new(true));
        let healthy = healthz(State(Arc::clone(&alive))).await;
        assert_eq!(healthy.0["relay"], "alive");
        alive.store(false, Ordering::Relaxed);
        let unhealthy = healthz(State(alive)).await;
        assert_eq!(unhealthy.0["status"], "unhealthy");
    }

    #[tokio::test]
    async fn shutdown_waits_for_the_flag() {
        let (tx, rx) = watch::channel(false);
        let waiter = tokio::spawn(wait_for_shutdown(rx));
        assert!(!waiter.is_finished());
        tx.send(true).expect("receiver alive");
        waiter.await.expect("resolves");
    }
}
