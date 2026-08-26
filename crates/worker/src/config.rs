//! Worker configuration, loaded from the environment.
//!
//! Purpose: settings for every background loop the worker hosts — the
//! outbox relay, the general job queue, the SLA evaluation sweep, and the
//! automations stream consumer. Defaults mirror the dev compose stack.
//!
//! Non-purpose: no realtime/HTTP concerns beyond the health endpoint, and
//! no per-job policy — retry budgets ride on each job row.
//!
//! Operational note: the relay, sweep, and queue all read across orgs and
//! never set `app.org_id`, so the role in `DATABASE_URL` must be able to
//! bypass org RLS (table owner or BYPASSRLS). A constrained role here
//! fails SILENTLY (zero rows, zero errors).

/// Runtime settings for the worker process.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Postgres URL; the only database this process touches.
    pub database_url: String,
    /// Redis URL the outbox relay publishes into.
    pub redis_url: String,
    /// Health-endpoint listen address (`BIND_ADDR`, default
    /// `0.0.0.0:8082`; core-api owns 8080, realtime 8081).
    pub bind_addr: String,
    /// Outbox rows relayed per batch (`RELAY_BATCH`).
    pub relay_batch: i64,
    /// Base URL of core-api for machine effect calls (`INTERNAL_API_URL`).
    pub internal_api_url: String,
    /// Bearer token for `/api/internal/*` (`MERIDIAN_INTERNAL_TOKEN`).
    /// Unset ⇒ effect calls are skipped with a loud warning; the queue
    /// dead-letters those jobs instead of spinning retries forever.
    pub internal_token: Option<String>,
    /// Seconds between SLA sweeps (`EVAL_INTERVAL_SECS`). The HOST clock
    /// only paces this cadence; every DECISION reads `now()` from Postgres.
    pub eval_interval_secs: u64,
    /// Jobs claimed per queue tick (`QUEUE_BATCH`).
    pub queue_batch: i64,
    /// Lease age after which a crashed claimant's job is re-armed
    /// (`QUEUE_STALE_SECS`, DB-clock comparison).
    pub queue_stale_after_secs: i64,
}

impl WorkerConfig {
    /// Reads env vars, falling back to dev defaults.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same resolution against any key lookup; keeps tests free of global
    /// environment mutation.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            // Owner role on purpose: see module note above. A constrained
            // role here fails SILENTLY — RLS filters every row without
            // erroring and nothing is ever published or evaluated.
            database_url: lookup("DATABASE_URL").unwrap_or_else(|| {
                "postgres://meridian:meridian@localhost:15432/meridian".to_string()
            }),
            redis_url: lookup("REDIS_URL")
                .unwrap_or_else(|| "redis://localhost:16379/0".to_string()),
            bind_addr: lookup("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8082".to_string()),
            relay_batch: lookup("RELAY_BATCH")
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            internal_api_url: lookup("INTERNAL_API_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
            internal_token: lookup("MERIDIAN_INTERNAL_TOKEN"),
            eval_interval_secs: lookup("EVAL_INTERVAL_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            queue_batch: lookup("QUEUE_BATCH")
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            queue_stale_after_secs: lookup("QUEUE_STALE_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_match_dev_ports_and_owner_role() {
        let cfg = WorkerConfig::from_lookup(|_| None);
        // The default MUST be the table owner: a constrained role silently
        // drains nothing (RLS filters all rows without erroring).
        assert_eq!(
            cfg.database_url,
            "postgres://meridian:meridian@localhost:15432/meridian"
        );
        assert_eq!(cfg.redis_url, "redis://localhost:16379/0");
        assert_eq!(cfg.bind_addr, "0.0.0.0:8082");
        assert_eq!(cfg.relay_batch, 100);
        assert_eq!(cfg.internal_api_url, "http://127.0.0.1:8080");
        assert_eq!(cfg.internal_token, None);
        assert_eq!(cfg.eval_interval_secs, 60);
        assert_eq!(cfg.queue_batch, 10);
        assert_eq!(cfg.queue_stale_after_secs, 300);
    }

    #[test]
    fn env_values_override_every_field() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("DATABASE_URL", "postgres://db".into());
        vars.insert("REDIS_URL", "redis://r".into());
        vars.insert("BIND_ADDR", "127.0.0.1:1".into());
        vars.insert("RELAY_BATCH", "7".into());
        vars.insert("INTERNAL_API_URL", "http://api:9000".into());
        vars.insert("MERIDIAN_INTERNAL_TOKEN", "sekrit".into());
        vars.insert("EVAL_INTERVAL_SECS", "15".into());
        vars.insert("QUEUE_BATCH", "3".into());
        vars.insert("QUEUE_STALE_SECS", "60".into());
        let cfg = WorkerConfig::from_lookup(|k| vars.get(k).cloned());
        assert_eq!(cfg.database_url, "postgres://db");
        assert_eq!(cfg.redis_url, "redis://r");
        assert_eq!(cfg.bind_addr, "127.0.0.1:1");
        assert_eq!(cfg.relay_batch, 7);
        assert_eq!(cfg.internal_api_url, "http://api:9000");
        assert_eq!(cfg.internal_token.as_deref(), Some("sekrit"));
        assert_eq!(cfg.eval_interval_secs, 15);
        assert_eq!(cfg.queue_batch, 3);
        assert_eq!(cfg.queue_stale_after_secs, 60);
    }

    #[test]
    fn malformed_values_fall_back_to_defaults() {
        let cfg = WorkerConfig::from_lookup(|k| match k {
            "RELAY_BATCH" | "EVAL_INTERVAL_SECS" | "QUEUE_BATCH" | "QUEUE_STALE_SECS" => {
                Some("not-a-number".into())
            }
            _ => None,
        });
        assert_eq!(cfg.relay_batch, 100);
        assert_eq!(cfg.eval_interval_secs, 60);
        assert_eq!(cfg.queue_batch, 10);
        assert_eq!(cfg.queue_stale_after_secs, 300);
    }
}
