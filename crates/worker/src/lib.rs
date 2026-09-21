//! worker — the background process host.
//!
//! Purpose: run every background loop Meridian needs: the outbox→Redis
//! relay (`core_events::run_relay`), the notification email fan-out
//! consumer (noop transport until a real MailTransport is configured),
//! the outbound email send loop, the general job queue processor, the
//! periodic SLA evaluation sweep, and the automations stream consumer —
//! plus a `/healthz` endpoint reflecting loop liveness. `meridian-cli`
//! hosts this same library via `meridian serve-worker`.
//!
//! Non-purpose: no domain writes — effects flow back through core-api's
//! internal surface (§4.1); the worker touches only its own queue tables,
//! and the email loop never touches Postgres at all.
//! Purpose: run the outbox→Redis relay (`core_events::run_relay`) as the
//! primary loop plus the notification email fan-out consumer and the
//! unified-search indexer consumer, and expose a `/healthz` endpoint that
//! reflects relay liveness. `meridian-cli` hosts this same library via
//! `meridian serve-worker`.
//!
//! Non-purpose: no general job queue, webhook delivery, or SLA evaluation
//! — those are later-stage scope — and no direct database writes of its own
//! beyond what the relay's mark-published step performs (the search indexer
//! writes through core-api's internal effect surface, §4.1).

pub mod automations;
pub mod config;
pub mod decision;
pub mod delivery;
pub mod effects;
pub mod email;
pub mod notifier;
pub mod pm_sweep;
pub mod queue_runner;
pub mod search_indexer;
pub mod sla_sweep;
pub mod time;
pub mod webhook_out;

use anyhow::Context as _;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use core_domain::notifications::NoopMailTransport;
use core_events::Publisher;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Runs the worker until `shutdown` fires: connects to Postgres and Redis,
/// spawns all background loops, and serves `/healthz` until then.
pub async fn run(
    config: config::WorkerConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let pool = core_db::connect(&config.database_url).await.context(
        "worker: connecting to Postgres — check DATABASE_URL and that the \
         database is reachable (dev compose: `docker compose up -d postgres`)",
    )?;
    let publisher = Publisher::connect(&config.redis_url).await.context(
        "worker: connecting to Redis — check REDIS_URL and that Redis is \
         reachable (dev compose: `docker compose up -d redis`); the relay \
         and automations loops cannot run without it",
    )?;
    tracing::info!("worker connected; starting background loops");

    let relay_alive = Arc::new(AtomicBool::new(true));
    let relay = tokio::spawn(core_events::run_relay(
        pool.clone(),
        publisher,
        config.relay_batch,
    ));
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

    // Notification email fan-out: noop transport until a real MailTransport
    // adapter is configured (standalone-first "none" mode). Best-effort
    // consumer; liveness is reported by the relay.
    tokio::spawn(notifier::run_notifier(
        pool.clone(),
        config.redis_url.clone(),
        NoopMailTransport,
        shutdown.clone(),
    ));

    // Outbound email send loop (disabled cleanly when unconfigured).
    let email_alive = Arc::new(AtomicBool::new(true));
    let transport = core_mail::build_transport(&config.email.transport_kind);
    {
        let email_alive = Arc::clone(&email_alive);
        let email_config = config.email.clone();
        let email_transport = Arc::clone(&transport);
        let email_shutdown = shutdown.clone();
        tokio::spawn(async move {
            email::run_email_loop(email_config, email_transport, email_shutdown).await;
            email_alive.store(false, Ordering::Relaxed);
        });
    }

    // Scheduled report delivery (TASKLIST R4): transport-unset deploys
    // run the LEDGERED log-skip none mode — schedules still advance and
    // every run stays visible, so this loop has no disable switch.
    let delivery_alive = Arc::new(AtomicBool::new(true));
    {
        let delivery_alive = Arc::clone(&delivery_alive);
        let delivery_pool = pool.clone();
        let database_url = config.database_url.clone();
        let delivery_transport = Arc::clone(&transport);
        let transport_kind = config.email.transport_kind.clone();
        let delivery_config = config.delivery.clone();
        let delivery_shutdown = shutdown.clone();
        tokio::spawn(async move {
            delivery::run_delivery_loop(
                delivery_pool,
                database_url,
                delivery_transport,
                transport_kind,
                delivery_config,
                delivery_shutdown,
            )
            .await;
            delivery_alive.store(false, Ordering::Relaxed);
        });
    }

    // Missing-time EOD nudge scheduler (disabled cleanly when the shared
    // internal token is unset — same none-mode as the email loop).
    let time_alive = Arc::new(AtomicBool::new(true));
    {
        let time_alive = Arc::clone(&time_alive);
        let time_config = config.time.clone();
        let time_pool = pool.clone();
        let time_shutdown = shutdown.clone();
        tokio::spawn(async move {
            time::run_time_loop(time_pool, time_config, time_shutdown).await;
            time_alive.store(false, Ordering::Relaxed);
        });
    }

    // Unified-search indexing: consumes domain events and pushes index
    // writes through core-api's internal effect surface. Disabled cleanly
    // when no internal token is configured (fail-closed on both sides).
    match &config.internal_token {
        Some(token) => {
            let sink = search_indexer::HttpIndexSink::new(&config.internal_api_url, token);
            tokio::spawn(search_indexer::run_search_indexer(
                pool.clone(),
                config.clone(),
                sink,
                shutdown.clone(),
            ));
            tracing::info!("search indexer enabled (pushing through core-api)");
        }
        None => {
            tracing::info!("search indexer disabled: MERIDIAN_INTERNAL_TOKEN not configured");
        }
    }

    // General job queue: claims durable jobs and applies them through
    // core-api's internal machine surface (plus signed webhook delivery).
    let queue_alive = Arc::new(AtomicBool::new(true));
    let counters = Arc::new(queue_runner::QueueCounters::default());
    {
        let effect_api = effects::ReqwestEffectApi::new(
            config.internal_api_url.clone(),
            config.internal_token.clone(),
        );
        let alive = Arc::clone(&queue_alive);
        let counters = Arc::clone(&counters);
        let pool = pool.clone();
        let shutdown = shutdown.clone();
        let batch = config.queue_batch;
        let stale = config.queue_stale_after_secs;
        let secret_key = config.webhook_secret_key;
        tokio::spawn(async move {
            queue_runner::run_queue(
                pool,
                effect_api,
                webhook_out::ReqwestSink::new(),
                secret_key,
                batch,
                stale,
                counters,
                shutdown,
            )
            .await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("queue loop exited; reporting unhealthy");
        });
    }

    // SLA evaluation sweep: DB-clock-driven ladder decisions → verdict jobs.
    let sla_alive = Arc::new(AtomicBool::new(true));
    {
        let alive = Arc::clone(&sla_alive);
        let pool = pool.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            sla_sweep::run_sla_sweep(pool, config.eval_interval_secs, 200, shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("sla sweep exited; reporting unhealthy");
        });
    }

    // Automations consumer: event stream → matched rules → effect jobs.
    let auto_alive = Arc::new(AtomicBool::new(true));
    {
        let alive = Arc::clone(&auto_alive);
        let pool = pool.clone();
        let redis_url = config.redis_url.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            automations::run_automations(pool, redis_url, shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("automations loop exited; reporting unhealthy");
        });
    }

    // PM-schedule tick (Wave 11 lane 9): claims due recurring-ticket
    // schedules, stamps the next occurrence, enqueues pm.generate jobs.
    let pm_alive = Arc::new(AtomicBool::new(true));
    {
        let alive = Arc::clone(&pm_alive);
        let pm_pool = pool.clone();
        let pm_shutdown = shutdown.clone();
        tokio::spawn(async move {
            pm_sweep::run_pm_sweep(pm_pool, config.eval_interval_secs, pm_shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("pm sweep exited; reporting unhealthy");
        });
    }

    // Signed-webhook delivery consumer: event stream → matched
    // subscriptions → durable webhook.deliver jobs (processed by the
    // queue runner above).
    let webhooks_alive = Arc::new(AtomicBool::new(true));
    {
        let alive = Arc::clone(&webhooks_alive);
        let webhook_pool = pool.clone();
        let webhook_redis = config.redis_url.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            webhook_out::run_webhook_out(webhook_pool, webhook_redis, shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("webhooks-out loop exited; reporting unhealthy");
        });
    }

    // Decision surfaces' stream consumer (Wave 23 lane 3): composes
    // state from event envelopes and dispatches ONE speculative fan-out
    // per trigger through core-api's internal decision seam. Disabled
    // cleanly (none mode) when no bot principal is configured — surfaces
    // act exactly as they would without a decision model.
    if config.decision_bot_user_id.is_some() {
        let fan_out: std::sync::Arc<dyn decision::DecisionFanOut> =
            std::sync::Arc::new(decision::ReqwestFanOut::new(
                config.internal_api_url.clone(),
                config.internal_token.clone(),
            ));
        tokio::spawn(decision::run_decision_triage(
            pool.clone(),
            config.clone(),
            fan_out,
            shutdown.clone(),
        ));
        tracing::info!("decision triage enabled (dispatching through core-api)");
    } else {
        tracing::info!("decision triage disabled: MERIDIAN_DECISION_BOT_USER_ID not configured");
    }

    let health = HealthFlags {
        relay_alive,
        email_alive,
        time_alive,
        queue_alive,
        sla_alive,
        automations_alive: auto_alive,
        webhooks_alive,
        delivery_alive,
        pm_alive,
    };
    // Unified-search indexing: consumes domain events and pushes index
    // writes through core-api's internal effect surface. Disabled cleanly
    // when no internal token is configured (fail-closed on both sides).
    match &config.internal_token {
        Some(token) => {
            let sink = search_indexer::HttpIndexSink::new(&config.internal_api_url, token);
            let indexer_pool = pool.clone();
            let indexer_config = config.clone();
            tokio::spawn(search_indexer::run_search_indexer(
                indexer_pool,
                indexer_config,
                sink,
                shutdown.clone(),
            ));
            tracing::info!("search indexer enabled (pushing through core-api)");
        }
        None => {
            tracing::info!("search indexer disabled: MERIDIAN_INTERNAL_TOKEN not configured");
        }
    }

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(Arc::new(health));
    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| {
            format!(
                "worker: binding health endpoint on {} — another process may \
             already hold the port; check BIND_ADDR",
                config.bind_addr
            )
        })?;
    tracing::info!(addr = %config.bind_addr, "worker health endpoint listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;
    Ok(())
}

#[derive(Clone)]
struct HealthFlags {
    relay_alive: Arc<AtomicBool>,
    email_alive: Arc<AtomicBool>,
    time_alive: Arc<AtomicBool>,
    queue_alive: Arc<AtomicBool>,
    sla_alive: Arc<AtomicBool>,
    automations_alive: Arc<AtomicBool>,
    webhooks_alive: Arc<AtomicBool>,
    delivery_alive: Arc<AtomicBool>,
    pm_alive: Arc<AtomicBool>,
}

async fn healthz(State(flags): State<Arc<HealthFlags>>) -> Json<serde_json::Value> {
    let ok = flags.relay_alive.load(Ordering::Relaxed)
        && flags.email_alive.load(Ordering::Relaxed)
        && flags.time_alive.load(Ordering::Relaxed)
        && flags.queue_alive.load(Ordering::Relaxed)
        && flags.sla_alive.load(Ordering::Relaxed)
        && flags.automations_alive.load(Ordering::Relaxed)
        && flags.webhooks_alive.load(Ordering::Relaxed)
        && flags.delivery_alive.load(Ordering::Relaxed)
        && flags.pm_alive.load(Ordering::Relaxed);
    if ok {
        Json(
            json!({ "status": "ok", "relay": "alive", "email": "alive", "queue": "alive", "sla": "alive", "automations": "alive", "webhooks": "alive", "delivery": "alive", "pm": "alive" }),
        )
    } else {
        Json(json!({ "status": "unhealthy" }))
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
    async fn healthz_reflects_the_liveness_flags() {
        let flags = HealthFlags {
            relay_alive: Arc::new(AtomicBool::new(true)),
            email_alive: Arc::new(AtomicBool::new(true)),
            time_alive: Arc::new(AtomicBool::new(true)),
            queue_alive: Arc::new(AtomicBool::new(true)),
            sla_alive: Arc::new(AtomicBool::new(true)),
            automations_alive: Arc::new(AtomicBool::new(true)),
            webhooks_alive: Arc::new(AtomicBool::new(true)),
            delivery_alive: Arc::new(AtomicBool::new(true)),
            pm_alive: Arc::new(AtomicBool::new(true)),
        };
        let healthy = healthz(State(Arc::new(flags.clone()))).await;
        assert_eq!(healthy.0["relay"], "alive");
        assert_eq!(healthy.0["email"], "alive");
        assert_eq!(healthy.0["webhooks"], "alive");
        assert_eq!(healthy.0["delivery"], "alive");
        assert_eq!(healthy.0["status"], "ok");

        flags.queue_alive.store(false, Ordering::Relaxed);
        let unhealthy = healthz(State(Arc::new(flags))).await;
        assert_eq!(unhealthy.0["status"], "unhealthy");
    }
}
