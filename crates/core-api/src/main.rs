//! core-api binary entry: config, tracing, pool, migrations, serve.
//! Startup migrations run only with RUN_MIGRATIONS=1 (checksummed history
//! in `_sqlx_migrations`); compose's `migrate` one-shot as the schema
//! owner is the normal path — the RLS role cannot CREATE TABLE on fresh
//! installs (the private build tracker backend-gaps lane).
//!
//! With `MARULA_ALL_IN_ONE=1` this same process also hosts the worker
//! loops (Wave 49 lane R4); see `core_api::all_in_one`.
//!
//! Shutdown (lane R5): one `worker::shutdown` contract for both topologies —
//! SIGINT or SIGTERM stops the listener, refuses new requests with 503, and
//! gives in-flight requests a bounded window to finish. The split topology
//! used to serve with no graceful shutdown at all while all-in-one had one, so
//! a rolling deploy cut live requests.

use anyhow::Context;
use axum::http::StatusCode;
use axum::routing::get;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use worker::shutdown::{self, Tracker};

/// How long `/readyz` waits for its own `SELECT 1` before calling the database
/// unreachable. Short on purpose: a probe that outlives its period is a probe
/// that reports the wrong thing.
const READINESS_QUERY_TIMEOUT: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared telemetry (the private build tracker 2.18): none-mode unless
    // OTEL_EXPORTER_OTLP_ENDPOINT is set; guard lives for `main`.
    let _telemetry = core_otel::init("core-api");

    // Installed before anything that can block: a stop during boot (a slow
    // migration, a database still coming up) must exit cleanly instead of
    // waiting for the runtime's kill.
    let shutdown = shutdown::install();
    let tracker = Tracker::new();

    let config = std::sync::Arc::new(
        core_api::config::Config::from_env()
            .expect("configuration fail-closed (from_env): see the operator copy above"),
    );
    tracing::info!(addr = %config.bind_addr, "starting core-api");
    // Trusted-proxy posture (Wave 47 lane A10): warns — never fails — when
    // an internet-reachable bind runs on the narrow default trust set, so
    // an edge deployment learns that public peers' proxy headers are
    // ignored before it misreads its own throttle keys.
    config.warn_trusted_proxy_posture();
    // The retry matrix is process-global: install it before any loop or
    // outbound mail send reads a backoff, and seed the jitter stream so two
    // replicas never retry in lockstep. In all-in-one mode the worker loops
    // this process hosts read the same matrix.
    core_domain::retry::install(config.retry.matrix);
    core_domain::retry::seed_jitter(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
            ^ u64::from(std::process::id()),
    );
    // Plugin distribution trust posture at boot (Wave 19 lane 2, ADR 0009
    // rotation semantics): an empty static trust set = structural
    // signature verification only — ACCEPTABLE DEV POSTURE, never
    // production. Operators register keys via `marula plugin-key add`
    // (docs/reference/plugin-signing-rotation-runbook.md); the DB
    // registry wins over this static set at request time.
    if core_api::config::bootstrap_signing_keys(&config).is_empty() {
        tracing::warn!(
            "plugin distribution signing keys UNCONFIGURED — artifact \
             signatures degrade to structural-only checks; register an \
             operator key (docs/reference/plugin-signing-rotation-runbook.md). \
             Never production."
        );
    }

    let pool = core_db::connect_with(&config.database_url, config.pool_config())
        .await
        .context(
            "connecting to Postgres — check DATABASE_URL and that the database \
             is reachable (dev compose: `docker compose up -d postgres`)",
        )?;
    // Startup migrations are OPT-IN (the private build tracker backend-gaps lane, 2026-09-10):
    // running them as `marula_app` fails on fresh installs — Postgres 17
    // denies even `CREATE TABLE IF NOT EXISTS _sqlx_migrations` to the RLS
    // role (0001 grants USAGE on public only). Compose runs the `migrate`
    // one-shot as the OWNER before the API starts; native dev sets
    // RUN_MIGRATIONS=1 (or lets the migrate one-shot run first).
    if std::env::var("RUN_MIGRATIONS").as_deref() == Ok("1") {
        core_db::run_migrations(&pool).await.context(
            "running migrations — on fresh installs these must run as the \
             schema owner, not marula_app: run the compose `migrate` \
             one-shot first, or keep RUN_MIGRATIONS=1 only for native dev",
        )?;
        core_db::assert_rls_coverage(&pool).await.context(
            "RLS coverage check — a table lacks row-level security; \
                 see the RLS conventions block in crates/core-db before \
                 adding tables to the allowlist",
        )?;
        tracing::info!("migrations applied; RLS coverage verified");
    } else {
        tracing::info!(
            "RUN_MIGRATIONS not set; skipping startup migrations (compose `migrate` one-shot or RUN_MIGRATIONS=1 owns them)"
        );
    }

    let state = core_api::state::AppState {
        pool,
        config: std::sync::Arc::clone(&config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
        mail: std::sync::Arc::new(core_api::email::MailStack::from_config(&config)),
        reporting: core_api::state::AppState::reporting_pool_sized(
            &config.database_url,
            config.reporting_pool_max,
            config.statement_cache_capacity,
        )
        .await,
        store: {
            let choice = config.object_store.clone();
            tracing::info!(?choice, "object store selected");
            core_store::build(&choice)
        },
        routing: {
            let registry = core_api::routing::build_registry(&config);
            tracing::info!(status = ?registry.status_view(), "routing provider posture");
            registry
        },
    };
    // Reference cache (Wave 52 lane P3): one in-process cache per replica,
    // invalidated by the same `marula_events` listener that fans frames out.
    // Initialised before the listener starts so the first event already has
    // something to invalidate.
    let cache = core_api::cache::init(
        config.ref_cache_ttl_secs,
        config.ref_cache_max_entries as usize,
    );
    tracing::info!(
        ttl_secs = config.ref_cache_ttl_secs,
        max_entries = config.ref_cache_max_entries,
        "reference cache ready"
    );
    tokio::spawn(core_api::cache::sweep_loop(std::sync::Arc::clone(&cache)));

    // Live delivery (Wave 12 lane 12): ONE dedicated LISTEN connection per
    // process receives `marula_events` notifications (migration 0068)
    // and fans contract events + presence frames out to SSE subscribers.
    // It dials `MARULA_LISTEN_DATABASE_URL` when set (Wave 52 lane P9): a
    // transaction pooler cannot hold a subscription, so a pooled deployment
    // points this one connection straight at Postgres.
    if config.listen_database_url.is_some() {
        tracing::info!("LISTEN connection uses MARULA_LISTEN_DATABASE_URL (bypassing the pooler)");
    }
    tokio::spawn(core_api::realtime::run_listener(
        config.listen_url().to_string(),
    ));
    // The readiness probe needs the pool after the router takes ownership of
    // the state.
    let readiness_pool = state.pool.clone();
    let mut app = core_api::router(state);

    // Browser clients (apps/web) need CORS when served from another origin.
    if let Some(origin) = &config.cors_origin {
        use axum::http::{header, HeaderValue, Method};
        app = app.layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(
                    origin
                        .parse::<HeaderValue>()
                        .context("parsing CORS_ORIGIN")?,
                )
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers([
                    header::CONTENT_TYPE,
                    header::AUTHORIZATION,
                    // Cookie-session CSRF double-submit header (Wave 19
                    // lane 5); bearer-mode callers are unaffected.
                    header::HeaderName::from_static("x-marula-csrf"),
                ])
                // httpOnly refresh cookies ride cross-origin app→API
                // calls (same-site), so responses must be credentialed.
                .allow_credentials(true),
        );
        tracing::info!(origin = %origin, "CORS enabled");
    }

    // Liveness and readiness, unmounted from the API generations on purpose: a
    // probe is not an API route, so it answers on the same path in every
    // version and never appears in the pinned OpenAPI document.
    //
    // `/livez` says only that this process is answering. `/readyz` checks the
    // database (a bounded `SELECT 1`) and the shutdown flag — before this,
    // readiness pointed at `/api/v1/health`, which is a constant "ok", so a
    // replica with an unreachable database stayed Ready and kept receiving
    // traffic.
    app = app
        .route("/livez", get(livez))
        .route("/readyz", get(readyz))
        .layer(axum::Extension(readiness_pool))
        .layer(axum::Extension(Arc::clone(&tracker)))
        // Outermost: a request that arrives inside the shutdown window is
        // refused here, before any handler or middleware does work for it.
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&tracker),
            shutdown::track_and_refuse,
        ));

    // Served WITH connect info so every request carries its direct peer
    // address: real-client-IP resolution (Wave 47 lane A10) keys off the
    // peer, never off the caller-supplied `X-Forwarded-For` first hop.
    let listener = tokio::net::TcpListener::bind(
        &std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
    )
    .await
    .context("binding listener")?;
    tracing::info!("core-api listening");

    // All-in-one (Wave 49 lane R4): run the worker loops in THIS process. Both
    // topologies now share one stop path — the embedded worker watches the same
    // flag the HTTP server does, so a single SIGTERM drains the request queue
    // and the loops inside one grace window.
    let embedded_worker = if config.all_in_one {
        let handle =
            core_api::all_in_one::spawn_worker(std::sync::Arc::clone(&config), shutdown.clone());
        tracing::info!(
            worker = %config.worker_bind_addr,
            "all-in-one: worker loops hosted in-process"
        );
        Some(handle)
    } else {
        None
    };

    let window = shutdown::grace_window();
    let serving = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown::wait(shutdown.clone()));
    shutdown::serve_and_drain("core-api", serving, shutdown, tracker, window).await?;
    if let Some(handle) = embedded_worker {
        // The API has drained its requests; the loops in this process drain
        // against the same window before the process leaves.
        shutdown::wait_for_task(handle, window).await;
    }
    Ok(())
}

/// `GET /livez` — the process answers; nothing else is asked.
async fn livez() -> StatusCode {
    StatusCode::OK
}

/// `GET /readyz` — 200 only when this replica is not draining and its database
/// answers inside [`READINESS_QUERY_TIMEOUT`].
async fn readyz(
    axum::Extension(pool): axum::Extension<PgPool>,
    axum::Extension(tracker): axum::Extension<Arc<Tracker>>,
) -> (StatusCode, &'static str) {
    if tracker.closing() {
        return (StatusCode::SERVICE_UNAVAILABLE, "draining");
    }
    match tokio::time::timeout(
        READINESS_QUERY_TIMEOUT,
        sqlx::query("SELECT 1").execute(&pool),
    )
    .await
    {
        Ok(Ok(_)) => (StatusCode::OK, "ready"),
        Ok(Err(error)) => {
            tracing::warn!(%error, "readiness probe: the database refused a connection");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable")
        }
        Err(_) => {
            tracing::warn!(
                timeout_ms = READINESS_QUERY_TIMEOUT.as_millis() as u64,
                "readiness probe: the database did not answer in time"
            );
            (StatusCode::SERVICE_UNAVAILABLE, "database timed out")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Extension;
    use sqlx::postgres::PgPoolOptions;

    /// Nothing listens on port 1, so the pool fails to connect: the state a
    /// replica with a broken database is in, and the state the old constant
    /// `/api/v1/health` reported as Ready.
    fn unreachable_pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://marula:marula@127.0.0.1:1/marula")
            .expect("lazy pool")
    }

    #[tokio::test]
    async fn readiness_fails_when_the_database_does_not_answer() {
        let (status, reason) =
            readyz(Extension(unreachable_pool()), Extension(Tracker::new())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            reason == "database unreachable" || reason == "database timed out",
            "unexpected readiness reason: {reason}"
        );
    }

    #[tokio::test]
    async fn readiness_fails_while_draining() {
        let tracker = Tracker::new();
        tracker.close();
        let (status, reason) = readyz(Extension(unreachable_pool()), Extension(tracker)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reason, "draining");
    }

    /// Liveness answers with no state at all: a probe that needs the database
    /// restarts the fleet when the database is the thing that is down.
    #[tokio::test]
    async fn liveness_asks_nothing_of_the_database() {
        assert_eq!(livez().await, StatusCode::OK);
    }
}
