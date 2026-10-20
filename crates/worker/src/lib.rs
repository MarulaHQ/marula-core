//! worker — the background process host.
//!
//! Purpose: run every background loop Marula needs: the outbox→Redis
//! relay (`core_events::run_relay`), the notification email fan-out
//! consumer (noop transport until a real MailTransport is configured),
//! the outbound email send loop, the general job queue processor, the
//! periodic SLA evaluation sweep, and the automations stream consumer —
//! plus a `/healthz` endpoint reflecting loop liveness. `marula-core-cli`
//! hosts this same library via `marula serve-worker`.
//!
//! Non-purpose: no domain writes — effects flow back through core-api's
//! internal surface (§4.1); the worker touches only its own queue tables,
//! and the email loop never touches Postgres at all.
//! Purpose: run the outbox→Redis relay (`core_events::run_relay`) as the
//! primary loop plus the notification email fan-out consumer and the
//! unified-search indexer consumer, and expose a `/healthz` endpoint that
//! reflects relay liveness. `marula-core-cli` hosts this same library via
//! `marula serve-worker`.
//!
//! Non-purpose: no general job queue, webhook delivery, or SLA evaluation
//! — those are later-stage scope — and no direct database writes of its own
//! beyond what the relay's mark-published step performs (the search indexer
//! writes through core-api's internal effect surface, §4.1).

pub mod automations;
pub mod config;
pub mod csat;
pub mod decision;
pub mod dedupe_sweep;
pub mod delivery;
pub mod digest;
pub mod effects;
pub mod email;
pub mod mail_config;
pub mod notifier;
pub mod outbox_retention_sweep;
pub mod pm_sweep;
pub mod project_billing;
pub mod queue_runner;
pub mod reconnect;
pub mod rotation_sweep;
pub mod run_retention_sweep;
pub mod search_indexer;
pub mod sla_sweep;
pub mod time;
pub mod usage_refresh_sweep;
pub mod webhook_out;
pub mod workflow;

use anyhow::Context as _;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use core_events::Publisher;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::watch;

/// Entries removed per `XTRIM` when clearing a pre-existing stream backlog at
/// startup. Sized so one pass is a few milliseconds of Valkey's single
/// threaded event loop rather than a visible stall.
const STREAM_TRIM_CHUNK: usize = 5_000;

/// Passes the startup backlog trim may spend, so a stream carrying hundreds of
/// millions of entries cannot turn worker startup into a long hang. The bound
/// on `XADD` keeps trimming afterwards; this only decides how much of an old
/// backlog is cleared eagerly.
const STREAM_TRIM_PASSES: usize = 200;

/// Runs the worker until `shutdown` fires: connects to Postgres and Redis,
/// spawns all background loops, and serves `/healthz` until then.
/// Installs the resolved retry matrix and seeds the jitter stream.
///
/// Idempotent by construction (`OnceLock`): in all-in-one mode core-api may
/// install first, and both resolve from the same environment. The seed matters
/// even though the matrix does not — two replicas started from the same image
/// must not sample the same jitter sequence, or they reconnect in lockstep.
pub fn install_retry(config: &config::WorkerConfig) {
    core_domain::retry::install(config.retry.matrix);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ u64::from(std::process::id());
    core_domain::retry::seed_jitter(seed);
}

pub async fn run(
    config: config::WorkerConfig,
    shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    install_retry(&config);
    let pool = core_db::connect_with(
        &config.database_url,
        core_db::PoolConfig {
            max_connections: config.db_pool_max,
            acquire_timeout: std::time::Duration::from_secs(core_db::DEFAULT_ACQUIRE_TIMEOUT_SECS),
        },
    )
    .await
    .context(
        "worker: connecting to Postgres — check DATABASE_URL and that the \
         database is reachable (dev compose: `docker compose up -d postgres`)",
    )?;
    let publisher = {
        // The stream is bounded by COUNT, and the bound is applied on every
        // XADD (Wave 52 lane P1). Read the reason in
        // `core_events::DEFAULT_STREAM_MAXLEN`: the shipped Valkey refuses
        // writes once it hits its memory ceiling, so an unbounded stream does
        // not grow slowly — it stops the entire event pipeline at once.
        let bound = core_events::StreamBound::from_entries(config.stream_maxlen);
        if core_events::StreamBound::was_clamped(config.stream_maxlen) {
            tracing::warn!(
                configured = config.stream_maxlen,
                resolved = bound.maxlen(),
                "MARULA_EVENT_STREAM_MAXLEN below the floor; clamped"
            );
        }
        let mut publisher = Publisher::connect_bounded(&config.redis_url, bound)
            .await
            .context(
                "worker: connecting to Redis — check REDIS_URL and that Redis is \
                 reachable (dev compose: `docker compose up -d redis`); the relay \
                 and automations loops cannot run without it",
            )?;
        tracing::info!(
            stream_maxlen = bound.maxlen(),
            "event stream bound resolved"
        );

        // Clear any backlog a previous unbounded build left in the stream, in
        // bounded chunks, BEFORE the relay starts writing: the first XADD
        // would otherwise carry the whole trim itself and block Valkey for its
        // duration. A failure here is not fatal — the bound on XADD still
        // applies — so it is logged and the worker starts anyway.
        match publisher
            .drain_backlog(STREAM_TRIM_CHUNK, STREAM_TRIM_PASSES)
            .await
        {
            Ok((0, _)) => {}
            Ok((removed, settled)) => {
                if settled {
                    tracing::info!(removed, "trimmed a pre-existing event-stream backlog");
                } else {
                    tracing::warn!(
                        removed,
                        "event-stream backlog not fully trimmed; the next publishes continue"
                    );
                }
            }
            Err(error) => {
                tracing::warn!(%error, "event-stream backlog trim failed; continuing");
            }
        }
        publisher
    };
    tracing::info!("worker connected; starting background loops");
    tracing::info!(
        roles = config.roles.label(),
        relay_wakeup = config.relay_wakeup,
        relay_poll_ms = config.relay_poll_ms,
        "worker loop groups"
    );

    let relay_alive = Arc::new(AtomicBool::new(true));
    if config.roles.realtime() {
        // Wakeup path (Wave 52 lane P4): migration 0068 fires a notification
        // inside the committing transaction, so the relay runs on the event
        // rather than on a 250 ms poll. `relay_poll_ms` is the backstop for
        // the reconnect window, not the delivery path.
        let pacing = if config.relay_wakeup {
            let wakeup = Arc::new(tokio::sync::Notify::new());
            tokio::spawn(core_events::run_wakeup_listener(
                config.database_url.clone(),
                Arc::clone(&wakeup),
            ));
            tracing::info!(
                channel = core_events::NOTIFY_CHANNEL,
                safety_poll_ms = config.relay_poll_ms,
                "outbox relay is notification-driven"
            );
            core_events::RelayPacing::notified(
                std::time::Duration::from_millis(config.relay_poll_ms),
                wakeup,
            )
        } else {
            tracing::info!(
                poll_ms = config.relay_poll_ms,
                "outbox relay polls only (MARULA_RELAY_WAKEUP is off)"
            );
            core_events::RelayPacing::polling(std::time::Duration::from_millis(
                config.relay_poll_ms,
            ))
        };
        let relay = tokio::spawn(core_events::run_relay(
            pool.clone(),
            publisher,
            config.relay_batch,
            pacing,
        ));
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

    // Notification email fan-out (Wave 34 lane 8): the worker decides WHO
    // gets mail (prefs + quiet hours), core-api renders the branded wrapper
    // and enqueues it onto the outbound ledger — so notification mail gets
    // retries and a delivery state like every other message.
    if config.roles.realtime() {
        tokio::spawn(notifier::run_notifier(
            pool.clone(),
            config.redis_url.clone(),
            Arc::new(notifier::HttpNotificationSink::new(
                config.internal_api_url.clone(),
                config.internal_token.clone(),
            )),
            shutdown.clone(),
        ));
    }

    // Outbound email send loop: transports resolve PER ORG at send time
    // (settings + sealed credential; env transport is the fallback).
    let email_alive = Arc::new(AtomicBool::new(true));
    let transport = core_mail::build_transport(&config.email.transport_kind);
    let mail_resolver: Arc<dyn mail_config::OrgMailResolver> =
        Arc::new(mail_config::DbOrgMailResolver::new(
            pool.clone(),
            config.webhook_secret_key,
            config.email.transport_kind.clone(),
        ));
    if config.roles.realtime() {
        let email_alive = Arc::clone(&email_alive);
        let email_config = config.email.clone();
        let email_resolver = Arc::clone(&mail_resolver);
        let email_shutdown = shutdown.clone();
        tokio::spawn(async move {
            email::run_email_loop(email_config, email_resolver, email_shutdown).await;
            email_alive.store(false, Ordering::Relaxed);
        });
    }

    // Daily digest sweep (Wave 34 lane 8): opted-in users get one email at
    // their chosen local hour; core-api owns the once-per-day marker.
    if config.roles.housekeeping() {
        tokio::spawn(digest::run_digest_loop(
            pool.clone(),
            digest::DigestLoopConfig {
                core_api_url: config.internal_api_url.clone(),
                internal_token: config.internal_token.clone(),
                poll_secs: std::env::var("DIGEST_POLL_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(900),
            },
            shutdown.clone(),
        ));
    }

    // Scheduled report delivery (TASKLIST R4): transport-unset deploys
    // run the LEDGERED log-skip none mode — schedules still advance and
    // every run stays visible, so this loop has no disable switch.
    let delivery_alive = Arc::new(AtomicBool::new(true));
    if config.roles.housekeeping() {
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
    if config.roles.housekeeping() {
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
    if config.roles.realtime() {
        if let Some(token) = &config.internal_token {
            let sink = search_indexer::HttpIndexSink::new(&config.internal_api_url, token);
            tokio::spawn(search_indexer::run_search_indexer(
                pool.clone(),
                config.clone(),
                sink,
                shutdown.clone(),
            ));
            tracing::info!("search indexer enabled (pushing through core-api)");
        } else {
            tracing::info!("search indexer disabled: MARULA_INTERNAL_TOKEN not configured");
        }
    }

    // General job queue: claims durable jobs and applies them through
    // core-api's internal machine surface (plus signed webhook delivery).
    let queue_alive = Arc::new(AtomicBool::new(true));
    let counters = Arc::new(queue_runner::QueueCounters::default());
    if config.roles.realtime() {
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
        let idle_ms = config.queue_idle_ms;
        let secret_key = config.webhook_secret_key;
        tokio::spawn(async move {
            queue_runner::run_queue(
                pool,
                effect_api,
                webhook_out::ReqwestSink::new(),
                secret_key,
                batch,
                stale,
                idle_ms,
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
    if config.roles.housekeeping() {
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
    if config.roles.realtime() {
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

    // CSAT surveys on resolution (Wave 34 lane 2): ticket.closed → policy
    // (send-on-resolve, per-client interval, quiet hours) → core-api-rendered
    // tokenized mail. Disabled cleanly when no internal token is configured.
    let csat_alive = Arc::new(AtomicBool::new(true));
    if config.roles.realtime() {
        let alive = Arc::clone(&csat_alive);
        let pool = pool.clone();
        let redis_url = config.redis_url.clone();
        let csat_config = csat::CsatLoopConfig {
            core_api_url: config.internal_api_url.clone(),
            internal_token: config.internal_token.clone(),
        };
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            csat::run_csat(pool, redis_url, csat_config, shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("csat loop exited; reporting unhealthy");
        });
    }

    // Project billing releases (Wave 39 lane 4): project.milestone_completed
    // + phase-completed fees → project_billing_releases ledger rows; the
    // invoice run consumes the ledger (durable consumer group, at-least-once
    // with ledger-side idempotency). No internal token needed: it writes
    // only its own ledger inserts through core_db.
    if config.roles.realtime() {
        let pool = pool.clone();
        let redis_url = config.redis_url.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            project_billing::run_project_billing(pool, redis_url, shutdown).await;
            tracing::error!("project_billing loop exited; reporting unhealthy");
        });
    }

    // PM-schedule tick (Wave 11 lane 9): claims due recurring-ticket
    // schedules, stamps the next occurrence, enqueues pm.generate jobs.
    let pm_alive = Arc::new(AtomicBool::new(true));
    if config.roles.housekeeping() {
        let alive = Arc::clone(&pm_alive);
        let pm_pool = pool.clone();
        let pm_shutdown = shutdown.clone();
        tokio::spawn(async move {
            pm_sweep::run_pm_sweep(pm_pool, config.eval_interval_secs, pm_shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("pm sweep exited; reporting unhealthy");
        });
    }

    // Secret-rotation reminders (Wave 19 lane 3): claims due rotate_at
    // rows across both vault subjects, enqueues secrets.rotation jobs.
    let rotation_alive = Arc::new(AtomicBool::new(true));
    // Org usage refresh (Wave 47 lane A6, migration 0131): keeps the
    // materialized usage table's per-org rows within one interval of the
    // read-model view. Declared with the other liveness flags because
    // /healthz reads it long after the block below has closed.
    let usage_alive = Arc::new(AtomicBool::new(true));
    if config.roles.housekeeping() {
        let alive = Arc::clone(&usage_alive);
        let usage_pool = pool.clone();
        let usage_shutdown = shutdown.clone();
        let interval_secs = config.usage_refresh_secs;
        tokio::spawn(async move {
            usage_refresh_sweep::run_usage_refresh_sweep(usage_pool, interval_secs, usage_shutdown)
                .await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("org usage refresh sweep exited; reporting unhealthy");
        });
    }
    // Workflow run-ledger retention (Wave 43 lane 14): one housekeeping pass
    // per interval that prunes each org's finished runs past its own window.
    // Declared here with the other liveness flags because /healthz reads it
    // long after the block below has closed.
    let retention_alive = Arc::new(AtomicBool::new(true));
    // Event-outbox retention (Wave 52 lane P1): declared alongside, for the
    // same reason — /healthz reads it long after the spawn block closes.
    let outbox_alive = Arc::new(AtomicBool::new(true));
    if config.roles.housekeeping() {
        let alive = Arc::clone(&rotation_alive);
        let rot_pool = pool.clone();
        let rot_shutdown = shutdown.clone();
        tokio::spawn(async move {
            rotation_sweep::run_rotation_sweep(rot_pool, config.eval_interval_secs, rot_shutdown)
                .await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("rotation sweep exited; reporting unhealthy");
        });
        // Workflow run-ledger retention (Wave 43 lane 14): one housekeeping
        // pass per interval that prunes each org's finished runs past its
        // own window. No internal token and no effect calls — a purge is
        // housekeeping, never a domain effect.
        {
            let alive = Arc::clone(&retention_alive);
            let retention_pool = pool.clone();
            let retention_shutdown = shutdown.clone();
            let interval_secs = config.retention_sweep_secs;
            tokio::spawn(async move {
                run_retention_sweep::run_run_retention_sweep(
                    retention_pool,
                    interval_secs,
                    retention_shutdown,
                )
                .await;
                alive.store(false, Ordering::Relaxed);
                tracing::error!("run retention sweep exited; reporting unhealthy");
            });
        }

        // Event-outbox retention (Wave 52 lane P1): the same shape for the
        // other table that grew forever. Only PUBLISHED rows are eligible —
        // an unpublished row is an event the relay still owes a consumer.
        {
            let alive = Arc::clone(&outbox_alive);
            let outbox_pool = pool.clone();
            let outbox_shutdown = shutdown.clone();
            let retention = config.outbox_retention;
            tokio::spawn(async move {
                outbox_retention_sweep::run_outbox_retention_sweep(
                    outbox_pool,
                    retention,
                    outbox_shutdown,
                )
                .await;
                alive.store(false, Ordering::Relaxed);
                tracing::error!("outbox retention sweep exited; reporting unhealthy");
            });
        }

        // Jev intelligent-dedupe scheduled passes (Wave 22 lane 2): claims due
        // `dedupe_workflows` orgs and dispatches the internal run seam (the
        // runner itself is core-api's; this loop never touches domain tables).
        {
            let effects = std::sync::Arc::new(effects::ReqwestEffectApi::new(
                config.internal_api_url.clone(),
                config.internal_token.clone(),
            ));
            let dedupe_pool = pool.clone();
            let dedupe_shutdown = shutdown.clone();
            tokio::spawn(async move {
                dedupe_sweep::run_dedupe_sweep(
                    dedupe_pool,
                    effects,
                    config.eval_interval_secs,
                    dedupe_shutdown,
                )
                .await;
            });
        }
    }

    // Signed-webhook delivery consumer: event stream → matched
    // subscriptions → durable webhook.deliver jobs (processed by the
    // queue runner above).
    let webhooks_alive = Arc::new(AtomicBool::new(true));
    if config.roles.realtime() {
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
    if config.roles.realtime() {
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
            tracing::info!("decision triage disabled: MARULA_DECISION_BOT_USER_ID not configured");
        }
    }

    // Workflow executor stream consumer (Wave 43 lane 15): matches each
    // envelope against enabled workflow documents (to START runs) and
    // against parked runs waiting on the event type (to WAKE them), durable
    // against the shared queue. Node effects cross to core-api org-scoped.
    let workflow_alive = Arc::new(AtomicBool::new(true));
    if config.roles.realtime() {
        let alive = Arc::clone(&workflow_alive);
        let workflow_pool = pool.clone();
        let workflow_redis = config.redis_url.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            workflow::run_workflow_stream(workflow_pool, workflow_redis, shutdown).await;
            alive.store(false, Ordering::Relaxed);
            tracing::error!("workflow loop exited; reporting unhealthy");
        });
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
        rotation_alive,
        retention_alive,
        outbox_alive,
        usage_alive,
        workflow_alive,
        roles: config.roles,
    };
    // The search indexer is spawned once, above, with the other real-time
    // consumers. A second identical spawn used to sit here: it read the same
    // stream with a plain XREAD (no consumer group), so both instances
    // indexed every event — duplicate work on the hottest loop in the
    // process. Removed rather than gated.

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
    rotation_alive: Arc<AtomicBool>,
    retention_alive: Arc<AtomicBool>,
    outbox_alive: Arc<AtomicBool>,
    usage_alive: Arc<AtomicBool>,
    workflow_alive: Arc<AtomicBool>,
    /// Which groups this process runs. A loop it does not own is reported
    /// as `disabled` and does not affect `status`: a realtime-only replica
    /// is healthy even though it runs no sweeps.
    roles: crate::config::WorkerRoles,
}

async fn healthz(State(flags): State<Arc<HealthFlags>>) -> Json<serde_json::Value> {
    let roles = flags.roles;
    // (name, alive, owned here)
    let loops: [(&str, bool, bool); 14] = [
        (
            "relay",
            flags.relay_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "email",
            flags.email_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "queue",
            flags.queue_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "automations",
            flags.automations_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "webhooks",
            flags.webhooks_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "workflows",
            flags.workflow_alive.load(Ordering::Relaxed),
            roles.realtime(),
        ),
        (
            "sla",
            flags.sla_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "delivery",
            flags.delivery_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "pm",
            flags.pm_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "rotation",
            flags.rotation_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "retention",
            flags.retention_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "outbox",
            flags.outbox_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "usage",
            flags.usage_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
        (
            "time",
            flags.time_alive.load(Ordering::Relaxed),
            roles.housekeeping(),
        ),
    ];
    // Only the loops this process owns decide health.
    let ok = loops.iter().all(|(_, alive, owned)| !owned || *alive);
    if ok {
        let mut body = serde_json::Map::new();
        body.insert("status".into(), json!("ok"));
        body.insert("glitchtip".into(), json!(dsn_state("GLITCHTIP_DSN_WORKER")));
        body.insert("roles".into(), json!(roles.label()));
        for (name, alive, owned) in loops {
            body.insert(
                name.to_string(),
                json!(if !owned {
                    "disabled"
                } else if alive {
                    "alive"
                } else {
                    "down"
                }),
            );
        }
        Json(serde_json::Value::Object(body))
    } else {
        Json(json!({ "status": "unhealthy", "glitchtip": dsn_state("GLITCHTIP_DSN_WORKER") }))
    }
}

/// `configured` | `unset` — GlitchTip DSN presence for the self-host
/// settings pane (Wave 36 lane 3); the DSN value never leaves env.
fn dsn_state(name: &str) -> &'static str {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => "configured",
        _ => "unset",
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
            rotation_alive: Arc::new(AtomicBool::new(true)),
            relay_alive: Arc::new(AtomicBool::new(true)),
            email_alive: Arc::new(AtomicBool::new(true)),
            time_alive: Arc::new(AtomicBool::new(true)),
            queue_alive: Arc::new(AtomicBool::new(true)),
            sla_alive: Arc::new(AtomicBool::new(true)),
            automations_alive: Arc::new(AtomicBool::new(true)),
            webhooks_alive: Arc::new(AtomicBool::new(true)),
            delivery_alive: Arc::new(AtomicBool::new(true)),
            pm_alive: Arc::new(AtomicBool::new(true)),
            retention_alive: Arc::new(AtomicBool::new(true)),
            outbox_alive: Arc::new(AtomicBool::new(true)),
            usage_alive: Arc::new(AtomicBool::new(true)),
            workflow_alive: Arc::new(AtomicBool::new(true)),
            roles: crate::config::WorkerRoles::all(),
        };
        let healthy = healthz(State(Arc::new(flags.clone()))).await;
        assert_eq!(healthy.0["relay"], "alive");
        assert_eq!(healthy.0["email"], "alive");
        assert_eq!(healthy.0["webhooks"], "alive");
        assert_eq!(healthy.0["delivery"], "alive");
        assert_eq!(healthy.0["outbox"], "alive");
        assert_eq!(healthy.0["roles"], "all");
        assert_eq!(healthy.0["status"], "ok");

        flags.queue_alive.store(false, Ordering::Relaxed);
        let unhealthy = healthz(State(Arc::new(flags))).await;
        assert_eq!(unhealthy.0["status"], "unhealthy");
    }

    /// A role-limited replica is healthy for the loops it owns and reports
    /// the rest as `disabled`, so an orchestrator does not read a
    /// realtime-only worker as eight dead sweeps.
    #[tokio::test]
    async fn healthz_ignores_loops_this_process_does_not_own() {
        let mut flags = HealthFlags {
            rotation_alive: Arc::new(AtomicBool::new(true)),
            relay_alive: Arc::new(AtomicBool::new(true)),
            email_alive: Arc::new(AtomicBool::new(true)),
            time_alive: Arc::new(AtomicBool::new(true)),
            queue_alive: Arc::new(AtomicBool::new(true)),
            sla_alive: Arc::new(AtomicBool::new(true)),
            automations_alive: Arc::new(AtomicBool::new(true)),
            webhooks_alive: Arc::new(AtomicBool::new(true)),
            delivery_alive: Arc::new(AtomicBool::new(true)),
            pm_alive: Arc::new(AtomicBool::new(true)),
            retention_alive: Arc::new(AtomicBool::new(true)),
            outbox_alive: Arc::new(AtomicBool::new(true)),
            usage_alive: Arc::new(AtomicBool::new(true)),
            workflow_alive: Arc::new(AtomicBool::new(true)),
            roles: crate::config::WorkerRoles::realtime_only(),
        };
        let body = healthz(State(Arc::new(flags.clone()))).await;
        assert_eq!(body.0["status"], "ok");
        assert_eq!(body.0["roles"], "realtime");
        assert_eq!(body.0["relay"], "alive");
        assert_eq!(body.0["webhooks"], "alive");
        assert_eq!(body.0["sla"], "disabled");
        assert_eq!(body.0["outbox"], "disabled");
        assert_eq!(body.0["time"], "disabled");

        // A sweep that has never been started cannot take the replica down,
        // but the loops it does own still can.
        flags.roles = crate::config::WorkerRoles::housekeeping_only();
        let body = healthz(State(Arc::new(flags.clone()))).await;
        assert_eq!(body.0["status"], "ok");
        assert_eq!(body.0["relay"], "disabled");
        flags.rotation_alive.store(false, Ordering::Relaxed);
        let body = healthz(State(Arc::new(flags))).await;
        assert_eq!(body.0["status"], "unhealthy");
    }
}
