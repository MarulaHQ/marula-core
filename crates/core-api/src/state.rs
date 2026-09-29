//! Shared application state.
//!
//! Note: core-api never publishes to Redis directly — mutations only write
//! the transactional outbox; the worker is the single stream publisher (§4.2).

use crate::email::MailStack;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<crate::config::Config>,
    /// Stateless OIDC protocol client (discovery/exchange/JWKS).
    pub oidc: crate::auth::oidc_client::OidcHttpClient,
    /// Email edge: transport + scanner adapters built from config.
    pub mail: Arc<MailStack>,
    /// Object storage edge (attachments): local-fs none mode by default,
    /// S3/MinIO when configured. See crates/core-store.
    pub store: Arc<dyn core_store::ObjectStore>,
    /// Routing provider registry (Wave 32 lane 7): operator-configured
    /// providers behind the `core-routing` seam. Empty ⇒ NONE mode (the
    /// built-in heuristic answers; suggestion-grade features stay silent).
    pub routing: Arc<core_routing::RoutingRegistry>,
    /// RLS-enforced pool for views-only surfaces (`routes::reports`).
    ///
    /// Reports compile org-blind SQL and rely ENTIRELY on row-level
    /// security flowing through the security-invoker `ops.v1_*` views, so
    /// they run on the RLS-constrained application role with an explicit
    /// `app.org_id` bind per request. Keep it a SEPARATE instance from
    /// `pool`: privilege profiles differ, and pooling them together would
    /// couple maintenance surfaces to the strictest role.
    ///
    /// Build with [`AppState::reporting_pool`] (rewrites credentials to
    /// `marula_app` when the configured URL carries elevated creds).
    pub reporting: PgPool,
}

impl AppState {
    /// Lazily-connected variant for unit tests / never-dialed surfaces.
    pub fn reporting_pool_lazy(database_url: &str) -> Result<PgPool, sqlx::Error> {
        let target = Self::reporting_target(database_url);
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(25)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_lazy(&target)
    }

    /// Rewrites credentials down to the application role when needed.
    fn reporting_target(database_url: &str) -> String {
        if database_url.contains("marula_app:") {
            return database_url.to_string();
        }
        let (scheme_rest, rest) = database_url
            .split_once("://")
            .unwrap_or(("postgres", database_url));
        match rest.split_once('@') {
            Some((_creds, host)) => format!("{scheme_rest}://marula_app:marula_app@{host}"),
            None => format!("{scheme_rest}://marula_app:marula_app@{rest}"),
        }
    }

    /// Connects a dedicated reporting pool from a database URL.
    pub async fn reporting_pool(database_url: &str) -> PgPool {
        let target = Self::reporting_target(database_url);
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(25)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect(&target)
            .await
            .expect("reporting app-role pool must connect")
    }
}
