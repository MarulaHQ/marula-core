//! core-api binary entry: config, tracing, pool, migrations, serve.
//! Startup migrations run only with RUN_MIGRATIONS=1 (checksummed history
//! in `_sqlx_migrations`); compose's `migrate` one-shot as the schema
//! owner is the normal path — the RLS role cannot CREATE TABLE on fresh
//! installs (TASKLIST backend-gaps lane).

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Shared telemetry (TASKLIST 2.18): none-mode unless
    // OTEL_EXPORTER_OTLP_ENDPOINT is set; guard lives for `main`.
    let _telemetry = core_otel::init("core-api");

    let config = std::sync::Arc::new(core_api::config::Config::from_env());
    tracing::info!(addr = %config.bind_addr, "starting core-api");

    let pool = core_db::connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    // Startup migrations are OPT-IN (TASKLIST backend-gaps lane, 2026-09-10):
    // running them as `meridian_app` fails on fresh installs — Postgres 17
    // denies even `CREATE TABLE IF NOT EXISTS _sqlx_migrations` to the RLS
    // role (0001 grants USAGE on public only). Compose runs the `migrate`
    // one-shot as the OWNER before the API starts; native dev sets
    // RUN_MIGRATIONS=1 (or lets the migrate one-shot run first).
    if std::env::var("RUN_MIGRATIONS").as_deref() == Ok("1") {
        core_db::run_migrations(&pool)
            .await
            .context("running migrations")?;
        core_db::assert_rls_coverage(&pool)
            .await
            .context("RLS coverage check")?;
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
        reporting: core_api::state::AppState::reporting_pool(&config.database_url).await,
        store: {
            let choice = config.object_store.clone();
            tracing::info!(?choice, "object store selected");
            core_store::build(&choice)
        },
    };
    // Live delivery (Wave 12 lane 12): ONE dedicated LISTEN connection per
    // process receives `meridian_events` notifications (migration 0068)
    // and fans contract events + presence frames out to SSE subscribers.
    tokio::spawn(core_api::realtime::run_listener(
        config.database_url.clone(),
    ));
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
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
        );
        tracing::info!(origin = %origin, "CORS enabled");
    }

    let listener = tokio::net::TcpListener::bind(
        &std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
    )
    .await
    .context("binding listener")?;
    tracing::info!("core-api listening");
    axum::serve(listener, app).await?;
    Ok(())
}
