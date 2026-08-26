//! core-api binary entry: config, tracing, pool, migrations, serve.
//! Runs migrations on boot (checksummed history in `_sqlx_migrations`).

use anyhow::Context;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .init();

    let config = std::sync::Arc::new(core_api::config::Config::from_env());
    tracing::info!(addr = %config.bind_addr, "starting core-api");

    let pool = core_db::connect(&config.database_url)
        .await
        .context("connecting to Postgres")?;
    core_db::run_migrations(&pool)
        .await
        .context("running migrations")?;
    core_db::assert_rls_coverage(&pool)
        .await
        .context("RLS coverage check")?;
    tracing::info!("migrations applied; RLS coverage verified");

    let state = core_api::state::AppState {
        pool,
        config: std::sync::Arc::clone(&config),
        oidc: core_api::auth::oidc_client::OidcHttpClient::new(),
    };
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
