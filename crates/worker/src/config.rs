//! Worker configuration, loaded from the environment.
//!
//! Purpose: the relay host's settings plus the outbound email send-loop
//! settings, with defaults mirroring `core_api::config::Config` so dev
//! (`docker compose up`) and CI need no extra env plumbing.
//!
//! Non-purpose: nothing realtime-specific (no bind/JWT concerns beyond the
//! health endpoint) and no per-job tuning knobs beyond batch/lease sizes.
//!
//! Operational note: the relay drains outbox rows for *all* orgs and never
//! sets `app.org_id`, so the role in `DATABASE_URL` must be able to read
//! across org RLS (table owner or BYPASSRLS). Single-role compose setups
//! get this for free; split-role dev databases must point the worker at
//! the owner role.

use core_mail::TransportKind;

/// Settings for the outbound email send loop (TASKLIST 2.4).
#[derive(Debug, Clone)]
pub struct EmailLoopConfig {
    /// core-api base URL the loop claims from / reports to.
    pub core_api_url: String,
    /// Shared internal token (`EMAIL_INTERNAL_TOKEN`). Unset ⇒ the loop
    /// stays idle (documented none mode — mail remains queued, logged).
    pub internal_token: Option<String>,
    /// Which transport adapter performs sends. MUST match what core-api
    /// recorded at enqueue time for coherent ledger labels.
    pub transport_kind: TransportKind,
    /// Rows claimed per cycle.
    pub claim_batch: i32,
    /// Lease seconds passed to the claim endpoint.
    pub lease_secs: i64,
}

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
    /// Outbound email send loop (Stage 2).
    pub email: EmailLoopConfig,
}

impl WorkerConfig {
    /// Reads `DATABASE_URL`, `REDIS_URL`, `BIND_ADDR`, `RELAY_BATCH` plus
    /// the email block from the process environment, falling back to dev
    /// defaults.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same resolution against any key lookup; keeps tests free of global
    /// environment mutation.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let env = |k: &str| lookup(k);
        let smtp = {
            let host = env("SMTP_HOST");
            host.map(|host| {
                (
                    host,
                    env("SMTP_PORT").and_then(|p| p.parse().ok()).unwrap_or(25),
                    env("SMTP_USERNAME"),
                    env("SMTP_PASSWORD"),
                )
            })
        };
        let postmark = env("POSTMARK_SERVER_TOKEN").map(|token| {
            (
                token,
                env("POSTMARK_BASE_URL").unwrap_or_else(|| "https://api.postmarkapp.com".into()),
            )
        });
        Self {
            // Owner role on purpose: the relay drains outbox rows for all
            // orgs and must not be RLS-constrained (see module note above).
            // A constrained role here fails SILENTLY — RLS filters every
            // row, the loop stays idle, and nothing is ever published.
            database_url: env("DATABASE_URL").unwrap_or_else(|| {
                "postgres://meridian:meridian@localhost:15432/meridian".to_string()
            }),
            redis_url: env("REDIS_URL").unwrap_or_else(|| "redis://localhost:16379/0".to_string()),
            bind_addr: env("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8082".to_string()),
            relay_batch: env("RELAY_BATCH")
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
            email: EmailLoopConfig {
                core_api_url: env("CORE_API_URL")
                    .unwrap_or_else(|| "http://localhost:8080".to_string()),
                internal_token: env("EMAIL_INTERNAL_TOKEN"),
                transport_kind: TransportKind::parse(
                    &env("EMAIL_TRANSPORT").unwrap_or_default(),
                    smtp,
                    postmark,
                ),
                claim_batch: env("EMAIL_CLAIM_BATCH")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(25),
                lease_secs: env("EMAIL_LEASE_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            },
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
        // Email loop defaults to disabled-none until configured.
        assert!(cfg.email.internal_token.is_none());
        assert_eq!(cfg.email.transport_kind, TransportKind::None);
        assert_eq!(cfg.email.core_api_url, "http://localhost:8080");
    }

    #[test]
    fn env_values_override_every_field() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("DATABASE_URL", "postgres://db".into());
        vars.insert("REDIS_URL", "redis://r".into());
        vars.insert("BIND_ADDR", "127.0.0.1:1".into());
        vars.insert("RELAY_BATCH", "7".into());
        vars.insert("CORE_API_URL", "http://api:9000".into());
        vars.insert("EMAIL_INTERNAL_TOKEN", "tok".into());
        vars.insert("EMAIL_TRANSPORT", "smtp".into());
        vars.insert("SMTP_HOST", "mailpit".into());
        vars.insert("SMTP_PORT", "1025".into());
        let cfg = WorkerConfig::from_lookup(|k| vars.get(k).cloned());
        assert_eq!(cfg.database_url, "postgres://db");
        assert_eq!(cfg.redis_url, "redis://r");
        assert_eq!(cfg.bind_addr, "127.0.0.1:1");
        assert_eq!(cfg.relay_batch, 7);
        assert_eq!(cfg.email.core_api_url, "http://api:9000");
        assert_eq!(
            cfg.email.transport_kind,
            TransportKind::Smtp {
                host: "mailpit".into(),
                port: 1025,
                username: None,
                password: None
            }
        );
    }

    #[test]
    fn malformed_values_fall_back_to_defaults() {
        let cfg = WorkerConfig::from_lookup(|k| match k {
            "RELAY_BATCH" => Some("not-a-number".into()),
            "EMAIL_TRANSPORT" => Some("carrier-pigeon".into()),
            _ => None,
        });
        assert_eq!(cfg.relay_batch, 100);
        // Unknown transport names degrade to none — never a surprise send.
        assert_eq!(cfg.email.transport_kind, TransportKind::None);
    }
}
