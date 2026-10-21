//! All-in-one hosting (Wave 49 lane R4): one process runs the HTTP API and
//! the background worker loops, so a demo or a tiny self-host install needs
//! a single container instead of two.
//!
//! Why this is a mode and not the default: the worker's cross-org loops read
//! on the OWNER-role Postgres connection while the API serves requests on
//! the RLS-scoped app role. Running both in one process means one process
//! holds both privilege profiles. That is fine for a demo box, and it is
//! deliberately gated behind `MARULA_ALL_IN_ONE=1`; the split topology stays
//! what compose ships.
//!
//! The worker still never writes domain rows itself: its effects go back
//! through the API's `/api/internal/*` surface over HTTP (§4.1), which in
//! this mode points at the same process over loopback.
//!
//! Every builder here takes an injectable environment lookup so the mapping
//! (which of the API's own settings feed the embedded services) is unit
//! tested without touching the process environment.

use std::sync::Arc;

use tokio::sync::watch;

use crate::config::Config;

/// Builds the embedded worker's config from the API's own config, so the two
/// can never disagree about Redis, the at-rest key, or the API's own address.
///
/// Only the worker-specific knobs are pinned — its owner-role URL, its health
/// port, the loopback API address and the at-rest key. Every other worker
/// variable (`EMAIL_*`, `EVAL_INTERVAL_SECS`, `QUEUE_BATCH`, …) still reads
/// the process environment, so those behave exactly as they do for the
/// standalone worker.
pub fn worker_config(config: &Config) -> worker::config::WorkerConfig {
    worker_config_with(config, |key| std::env::var(key).ok())
}

/// The lookup-injectable form of [`worker_config`], for tests.
pub fn worker_config_with(
    config: &Config,
    env: impl Fn(&str) -> Option<String>,
) -> worker::config::WorkerConfig {
    let owner_url = config.worker_database_url.clone().unwrap_or_default();
    let worker_bind = config.worker_bind_addr.clone();
    let redis = config.redis_url.clone();
    let internal_api = env("INTERNAL_API_URL")
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| loopback_api_url(config));
    let core_api_url_env = env("CORE_API_URL");
    // The API's effective at-rest key, as the 64-hex value the worker's own
    // parser wants. Handing it over explicitly keeps webhook delivery working
    // in all-in-one even when `MARULA_SECRET_KEY` is unset and the API
    // derived a dev key from `JWT_SECRET`.
    let secret_hex = hex::encode(config.secret_key.0);
    // The embedded worker's pools are this process's pools, so the two
    // answers must agree (Wave 52 lane P9): pass the API's resolved
    // statement-cache capacity and the direct LISTEN URL down rather than
    // letting the worker re-read the environment and possibly disagree.
    let statement_cache = config.statement_cache_capacity.to_string();
    let listen_url = config.listen_database_url.clone();
    worker::config::WorkerConfig::from_lookup(move |key| match key {
        // Owner role on purpose: the relay reads outbox rows across every
        // org and must not be filtered by the app role's RLS.
        "DATABASE_URL" => Some(owner_url.clone()),
        "BIND_ADDR" => Some(worker_bind.clone()),
        "REDIS_URL" => Some(redis.clone()),
        "INTERNAL_API_URL" => Some(internal_api.clone()),
        "CORE_API_URL" => core_api_url_env
            .clone()
            .or_else(|| Some(internal_api.clone())),
        "MARULA_SECRET_KEY" => env("MARULA_SECRET_KEY").or_else(|| Some(secret_hex.clone())),
        "MARULA_DB_STATEMENT_CACHE" => Some(statement_cache.clone()),
        "MARULA_LISTEN_DATABASE_URL" => listen_url.clone(),
        _ => env(key),
    })
}

/// The base URL the embedded worker uses to reach this process's
/// `/api/internal/*` surface. In-process, so it is a loopback address
/// derived from `bind_addr` — never the public URL, which would hairpin the
/// call through the edge. An explicit `INTERNAL_API_URL` still wins.
pub fn internal_api_url(config: &Config) -> String {
    std::env::var("INTERNAL_API_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| loopback_api_url(config))
}

/// `http://127.0.0.1:<port>` from the API's bind address. A wildcard or
/// specific host both resolve to loopback because the call never leaves the
/// process. Falls back to 8080's port when the bind string carries none.
fn loopback_api_url(config: &Config) -> String {
    let port = config.bind_addr.rsplit(':').next().unwrap_or("8080").trim();
    let port = if port.is_empty() { "8080" } else { port };
    format!("http://127.0.0.1:{port}")
}

/// Starts the worker loops in this process. Errors are logged, not fatal:
/// the API keeps serving even if a background loop dies. `shutdown` is the
/// receiver from `worker::shutdown::install`, shared with the API's own
/// serve, so one signal ends both halves inside one grace window.
pub fn spawn_worker(
    config: Arc<Config>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let worker_config = worker_config(&config);
    tokio::spawn(async move {
        if let Err(error) = worker::run(worker_config, shutdown).await {
            tracing::error!(%error, "all-in-one: the worker loops exited");
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config() -> Config {
        let mut c = Config::test_with_secret("s");
        c.bind_addr = "0.0.0.0:8080".into();
        c.redis_url = "redis://cache:6379/0".into();
        c.worker_bind_addr = "0.0.0.0:8082".into();
        c.worker_database_url = Some("postgres://marula:marula@db:5432/marula".into());
        c.all_in_one = true;
        c
    }

    /// The worker must run on the owner URL, its own health port, and a
    /// loopback API address — and it must inherit the API's at-rest key even
    /// when the environment carries none.
    #[test]
    fn worker_runs_on_the_owner_url_and_loopback_api() {
        let c = config();
        let vars: HashMap<&str, String> = HashMap::new();
        let wc = worker_config_with(&c, |k| vars.get(k).cloned());
        assert_eq!(wc.database_url, "postgres://marula:marula@db:5432/marula");
        assert_eq!(wc.redis_url, "redis://cache:6379/0");
        assert_eq!(wc.bind_addr, "0.0.0.0:8082");
        assert_eq!(wc.internal_api_url, "http://127.0.0.1:8080");
        assert_eq!(wc.email.core_api_url, "http://127.0.0.1:8080");
        assert!(wc.webhook_secret_key.is_some());
    }

    /// An explicit `INTERNAL_API_URL` / `CORE_API_URL` still wins over the
    /// loopback default: an operator behind a proxy knows better.
    #[test]
    fn explicit_api_urls_win_over_loopback() {
        let c = config();
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("INTERNAL_API_URL", "http://api.internal:9000".into());
        vars.insert("CORE_API_URL", "http://api.internal:9000".into());
        let wc = worker_config_with(&c, |k| vars.get(k).cloned());
        assert_eq!(wc.internal_api_url, "http://api.internal:9000");
        assert_eq!(wc.email.core_api_url, "http://api.internal:9000");
    }
}
