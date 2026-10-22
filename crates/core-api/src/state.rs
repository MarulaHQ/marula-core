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
    /// The plugin runtime's pool, started on the first module-bearing
    /// invocation and owned by THIS state (W53-11).
    ///
    /// It lives here rather than in a process-wide `static` so it can be shut
    /// down and so two states — two tests, two instances in one process —
    /// never share a pool. `Default` is a fresh, unstarted slot; the start
    /// happens lazily because it spawns and handshakes worker processes.
    pub plugin_pool: plugin_host::exec::PoolSlot,
}

impl AppState {
    /// Lazily-connected variant for unit tests / never-dialed surfaces.
    pub fn reporting_pool_lazy(database_url: &str) -> Result<PgPool, sqlx::Error> {
        Self::reporting_pool_lazy_with(database_url, DEFAULT_REPORTING_POOL_MAX)
    }

    /// Lazily-connected reporting pool with an explicit ceiling (Wave 49
    /// lane R1). Callers that size the pool from config use this form; the
    /// two-argument builder keeps the shipped default ceiling.
    pub fn reporting_pool_lazy_with(
        database_url: &str,
        max_connections: u32,
    ) -> Result<PgPool, sqlx::Error> {
        Self::reporting_pool_lazy_sized(
            database_url,
            max_connections,
            core_db::DEFAULT_STATEMENT_CACHE_CAPACITY,
        )
    }

    /// Lazily-connected reporting pool with an explicit ceiling AND statement
    /// cache capacity (Wave 52 lane P9). The reporting pool is its own
    /// `PgPool`, so it needs the pooler-safe capacity independently of the app
    /// pool: two pools in one process must not disagree about whether a
    /// statement name may outlive a transaction
    /// (`docs/reference/connection-pooling.md`).
    pub fn reporting_pool_lazy_sized(
        database_url: &str,
        max_connections: u32,
        statement_cache_capacity: u32,
    ) -> Result<PgPool, sqlx::Error> {
        let target = Self::reporting_target(database_url);
        let options =
            core_db::connect_options(&target, statement_cache_capacity).map_err(|e| match e {
                core_db::DbError::Sql(e) => e,
                other => sqlx::Error::Configuration(other.to_string().into()),
            })?;
        Ok(sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_lazy_with(options))
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
        Self::reporting_pool_with(database_url, DEFAULT_REPORTING_POOL_MAX).await
    }

    /// Connects a dedicated reporting pool with an explicit ceiling (Wave
    /// 49 lane R1).
    pub async fn reporting_pool_with(database_url: &str, max_connections: u32) -> PgPool {
        Self::reporting_pool_sized(
            database_url,
            max_connections,
            core_db::DEFAULT_STATEMENT_CACHE_CAPACITY,
        )
        .await
    }

    /// Connects the reporting pool with an explicit ceiling and statement
    /// cache capacity (Wave 52 lane P9). Boot uses this one: the capacity is
    /// an operator lever, so it cannot be a constant here.
    pub async fn reporting_pool_sized(
        database_url: &str,
        max_connections: u32,
        statement_cache_capacity: u32,
    ) -> PgPool {
        let target = Self::reporting_target(database_url);
        let options = core_db::connect_options(&target, statement_cache_capacity)
            .expect("reporting database URL must parse");
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(30))
            .connect_with(options)
            .await
            .expect("reporting app-role pool must connect")
    }
}

/// Reporting-pool ceiling (Wave 49 lane R1, resized by Wave 52 lane P9).
///
/// The reports surface is cold: it answers a user's report run, not a board
/// read, and `docs/perf.md` §5 measured the hot path's saturation at
/// concurrency 8. The old ceiling of 25 spent a quarter of the instance's
/// connection budget on a surface that never reaches it, so the ceiling is
/// the hot path's proven concurrency. Raise it with
/// `MARULA_REPORTING_POOL_MAX` when reports queue.
pub const DEFAULT_REPORTING_POOL_MAX: u32 = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_host::runtime::{IsolationMode, PluginPool, PoolConfig};
    use plugin_host::PluginRuntime;

    fn state() -> AppState {
        // A lazily-connected pool never dials, so this test needs no database.
        let url = "postgres://marula:marula@127.0.0.1:1/marula";
        let config = Arc::new(crate::config::Config::test_with_secret("state-test"));
        AppState {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy(url)
                .expect("a lazily-connected pool parses"),
            oidc: crate::auth::oidc_client::OidcHttpClient::new(),
            mail: Arc::new(crate::email::MailStack::from_config(&config)),
            store: core_store::build(&core_store::StoreChoice::Local(
                std::env::temp_dir().to_string_lossy().into_owned(),
            )),
            routing: crate::routing::build_registry(&config),
            reporting: AppState::reporting_pool_lazy(url).expect("lazy reporting pool"),
            config,
            plugin_pool: Default::default(),
        }
    }

    #[tokio::test]
    async fn two_app_states_do_not_share_a_plugin_pool() {
        let first = state();
        let second = state();
        assert!(
            !first.plugin_pool.is_same_slot(&second.plugin_pool),
            "each state owns its slot; a shared pool is what W53-11 moved out of a static"
        );
        assert!(!first.plugin_pool.is_initialized());

        // Start a real pool in the FIRST state only. `disabled` is a pool with
        // no worker process, which is all this test needs: the point is which
        // state is holding it, not what it runs.
        let started = first
            .plugin_pool
            .get_with(|| async {
                PluginPool::start(PoolConfig {
                    mode: IsolationMode::Disabled,
                    workers: 1,
                    worker_bin: None,
                    health: None,
                    sandbox_limits: Default::default(),
                })
                .await
            })
            .await
            .expect("a disabled pool starts without a worker binary");
        assert!(first.plugin_pool.is_initialized());
        assert!(
            !second.plugin_pool.is_initialized(),
            "starting one state's pool must not start another's"
        );
        assert!(!started.enabled());
    }
}
