//! worker — the background process host.
//!
//! Purpose: run two loops — the outbox→Redis relay (`core_events::run_relay`)
//! and, since Stage 2, the outbound email send loop that transports claimed
//! messages and reports results through core-api — plus a `/healthz`
//! endpoint reflecting both loops' liveness. `meridian-cli` hosts this
//! same library via `meridian serve-worker`.
//!
//! Non-purpose: no general job scheduler and no direct database writes of
//! its own beyond what the relay's mark-published step performs; the email
//! loop never touches Postgres (it talks to core-api exclusively).

pub mod config;
pub mod email;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use core_events::Publisher;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Runs the worker until `shutdown` fires: connects to Postgres and Redis,
/// spawns the relay + email-send loops, and serves `/healthz` until then.
pub async fn run(
    config: config::WorkerConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let pool = core_db::connect(&config.database_url).await?;
    let publisher = Publisher::connect(&config.redis_url).await?;
    tracing::info!("worker connected; starting event relay");

    let relay_alive = Arc::new(AtomicBool::new(true));
    {
        // Flip the health flag if the loop ever exits (it only can by
        // panicking); /healthz then reports unhealthy so the orchestrator
        // restarts the container.
        let relay_alive = Arc::clone(&relay_alive);
        tokio::spawn(async move {
            let handle = tokio::spawn(core_events::run_relay(pool, publisher, config.relay_batch));
            let _ = handle.await;
            relay_alive.store(false, Ordering::Relaxed);
            tracing::error!("relay loop exited; reporting unhealthy");
        });
    }

    // Outbound email send loop (disabled cleanly when unconfigured).
    let email_alive = Arc::new(AtomicBool::new(true));
    {
        let email_alive = Arc::clone(&email_alive);
        let transport = core_mail::build_transport(&config.email.transport_kind);
        let email_config = config.email.clone();
        let email_shutdown = shutdown.clone();
        tokio::spawn(async move {
            email::run_email_loop(email_config, transport, email_shutdown).await;
            email_alive.store(false, Ordering::Relaxed);
        });
    }

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state((relay_alive, email_alive));
    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "worker health endpoint listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;
    Ok(())
}

async fn healthz(
    State((relay, email)): State<(Arc<AtomicBool>, Arc<AtomicBool>)>,
) -> Json<serde_json::Value> {
    let relay_ok = relay.load(Ordering::Relaxed);
    let email_ok = email.load(Ordering::Relaxed);
    if relay_ok && email_ok {
        Json(json!({ "status": "ok", "relay": "alive", "email": "alive" }))
    } else {
        Json(json!({
            "status": "unhealthy",
            "relay": if relay_ok { "alive" } else { "stopped" },
            "email": if email_ok { "alive" } else { "stopped" },
        }))
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
    async fn healthz_reflects_both_loops() {
        let relay = Arc::new(AtomicBool::new(true));
        let email = Arc::new(AtomicBool::new(true));
        let healthy = healthz(State((Arc::clone(&relay), Arc::clone(&email)))).await;
        assert_eq!(healthy.0["relay"], "alive");
        assert_eq!(healthy.0["email"], "alive");
        email.store(false, Ordering::Relaxed);
        let degraded = healthz(State((Arc::clone(&relay), Arc::clone(&email)))).await;
        assert_eq!(degraded.0["status"], "unhealthy");
        assert_eq!(degraded.0["email"], "stopped");
        assert_eq!(degraded.0["relay"], "alive");
    }

    #[tokio::test]
    async fn shutdown_waits_for_the_flag() {
        let (tx, rx) = watch::channel(false);
        let waiter = tokio::spawn(wait_for_shutdown(rx));
        assert!(!waiter.is_finished());
        tx.send(true).expect("receiver alive");
        waiter.await.expect("resolves");
    }

    #[tokio::test]
    async fn email_loop_exits_immediately_when_unconfigured() {
        // None-mode: no token ⇒ the loop returns instead of spinning.
        let (_, rx) = watch::channel(false);
        let cfg = config::EmailLoopConfig {
            core_api_url: "http://127.0.0.1:1".into(),
            internal_token: None,
            transport_kind: core_mail::TransportKind::None,
            claim_batch: 10,
            lease_secs: 300,
        };
        // Awaiting resolves only if the loop returned.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            email::run_email_loop(cfg, Arc::new(core_mail::RecordingTransport::new()), rx),
        )
        .await
        .expect("unconfigured loop must exit promptly");
    }
}
