//! realtime — the WebSocket hub (§4.4).
//!
//! Purpose: authenticate WebSocket clients with the same JWT access tokens
//! as `core-api`, subscribe to the Redis event stream
//! (`core_events::EVENT_STREAM`) via XREAD, fan envelopes out over
//! per-user and per-board channels, and track in-memory presence per org.
//!
//! Non-purpose: no database access at all, no business logic, no permission
//! decisions beyond org tenancy from token claims, and no event-replay
//! storage or cursors — clients reconcile state via REST snapshots.

pub mod auth;
pub mod channel;
pub mod config;
pub mod hub;
pub mod presence;
pub mod protocol;
pub mod relay;
pub mod ws;

use axum::routing::get;
use axum::Router;
use tokio::sync::watch;

/// Serves the hub until Ctrl-C: Redis reader task plus Axum HTTP/WS server
/// exposing `GET /healthz` and `GET /ws`.
///
/// The standalone binary and `marula serve-realtime` use this. A host that
/// already owns the process signal — the all-in-one mode in core-api (Wave
/// 49 lane R4) — calls [`run_until`] with its own shutdown flag instead, so
/// one Ctrl-C stops every embedded service without two handlers racing.
pub async fn run(config: config::RealtimeConfig) -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(shutdown_signal(shutdown_tx));
    run_until(config, shutdown_rx).await
}

/// The same hub, but it installs NO signal handler: it serves until
/// `shutdown` flips true. The Redis reader and the WS server share that one
/// flag, so the caller owns the whole lifecycle.
pub async fn run_until(
    config: config::RealtimeConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let hub = hub::Hub::new();

    let reader = tokio::spawn(relay::run_stream_reader(
        config.redis_url.clone(),
        hub.clone(),
        shutdown.clone(),
    ));

    let state = ws::AppState::new(hub, config.jwt_secret, config.jwt_eddsa_public_key);
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws::upgrade))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "realtime hub listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;

    reader.abort();
    Ok(())
}

/// Completes when `shutdown` is true: at once if it already is, otherwise at
/// the next flip.
async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    let _ = shutdown.changed().await;
}

/// Liveness for orchestrators plus the GlitchTip DSN presence the self-host
/// settings pane reports (Wave 36 lane 3). Presence only — the DSN value
/// never leaves the process environment. Probes check the status code, so
/// the body stays additive.
async fn healthz() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "glitchtip": dsn_state("GLITCHTIP_DSN_REALTIME"),
    }))
}

/// `configured` | `unset`.
fn dsn_state(name: &str) -> &'static str {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => "configured",
        _ => "unset",
    }
}

/// Flips the shared shutdown flag so the blocking XREAD loop wakes up.
async fn shutdown_signal(shutdown_tx: watch::Sender<bool>) {
    let _ = tokio::signal::ctrl_c().await;
    let _ = shutdown_tx.send(true);
}

#[cfg(test)]
pub(crate) mod testing {
    //! Shared fixture builders for unit tests across this crate's modules.

    use core_events::Envelope;

    /// A minimal routable envelope for routing/hub tests.
    pub(crate) fn envelope(entity: &str, payload: serde_json::Value) -> Envelope {
        Envelope {
            id: uuid::Uuid::now_v7(),
            event_type: format!("{entity}.test"),
            org_id: uuid::Uuid::now_v7(),
            entity: entity.to_string(),
            entity_id: None,
            occurred_at: chrono::Utc::now(),
            payload_schema_ref: "core-domain/v1".into(),
            payload,
        }
    }
}
