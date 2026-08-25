//! Worker configuration, loaded from the environment.
//!
//! Purpose: the relay host's settings with defaults mirroring
//! `core_api::config::Config` so dev (`docker compose up`) and CI need no
//! extra env plumbing.
//!
//! Non-purpose: nothing realtime-specific (no bind/JWT concerns beyond the
//! health endpoint) and no per-job tuning — the general job queue is Stage
//! 2 scope and will grow its own config section then.
//!
//! Operational note: the relay drains outbox rows for *all* orgs and never
//! sets `app.org_id`, so the role in `DATABASE_URL` must be able to read
//! across org RLS (table owner or BYPASSRLS). Single-role compose setups
//! get this for free; split-role dev databases must point the worker at
//! the owner role.

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
}

impl WorkerConfig {
    /// Reads `DATABASE_URL`, `REDIS_URL`, `BIND_ADDR`, `RELAY_BATCH` from
    /// the process environment, falling back to dev defaults.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same resolution against any key lookup; keeps tests free of global
    /// environment mutation.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            // Owner role on purpose: the relay drains outbox rows for all
            // orgs and must not be RLS-constrained (see module note above).
            // A constrained role here fails SILENTLY — RLS filters every
            // row, the loop stays idle, and nothing is ever published.
            database_url: lookup("DATABASE_URL").unwrap_or_else(|| {
                "postgres://meridian:meridian@localhost:15432/meridian".to_string()
            }),
            redis_url: lookup("REDIS_URL")
                .unwrap_or_else(|| "redis://localhost:16379/0".to_string()),
            bind_addr: lookup("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8082".to_string()),
            relay_batch: lookup("RELAY_BATCH")
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
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
    }

    #[test]
    fn env_values_override_every_field() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("DATABASE_URL", "postgres://db".into());
        vars.insert("REDIS_URL", "redis://r".into());
        vars.insert("BIND_ADDR", "127.0.0.1:1".into());
        vars.insert("RELAY_BATCH", "7".into());
        let cfg = WorkerConfig::from_lookup(|k| vars.get(k).cloned());
        assert_eq!(cfg.database_url, "postgres://db");
        assert_eq!(cfg.redis_url, "redis://r");
        assert_eq!(cfg.bind_addr, "127.0.0.1:1");
        assert_eq!(cfg.relay_batch, 7);
    }

    #[test]
    fn malformed_batch_falls_back_to_default() {
        let cfg =
            WorkerConfig::from_lookup(|k| (k == "RELAY_BATCH").then(|| "not-a-number".to_string()));
        assert_eq!(cfg.relay_batch, 100);
    }
}
