//! Worker configuration, loaded from the environment.
//!
//! Purpose: settings for every background loop the worker hosts — the
//! outbox relay, the outbound email send loop, the general job queue, the
//! SLA evaluation sweep, and the automations stream consumer. Defaults
//! mirror the dev compose stack.
//!
//! Non-purpose: no realtime/HTTP concerns beyond the health endpoint, and
//! no per-job policy — retry budgets ride on each job row (email) or the
//! job row itself (queue).
//!
//! Operational note: the relay, sweep, and queue all read across orgs and
//! never set `app.org_id`, so the role in `DATABASE_URL` must be able to
//! bypass org RLS (table owner or BYPASSRLS). A constrained role here
//! fails SILENTLY (zero rows, zero errors) — for the relay this means
//! nothing is ever published.

use core_mail::TransportKind;

/// The at-rest secret-encryption key (AES-256, same `MARULA_SECRET_KEY`
/// env core-api uses). Needed ONLY by the webhook delivery loop to decrypt
/// subscription secrets before signing; unset ⇒ webhook jobs dead-letter
/// fail-closed (documented none mode) — nothing is delivered plaintext.
#[derive(Debug, Clone, Copy)]
pub struct AtRestKey(pub [u8; 32]);

impl AtRestKey {
    /// Parses the 64-hex-char env value.
    pub fn from_env_value(value: Option<&str>) -> Option<Self> {
        let hex_key = value?;
        if hex_key.len() != 64 {
            return None;
        }
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(hex_key, &mut bytes).ok()?;
        Some(Self(bytes))
    }

    /// Decrypts `nonce || ciphertext` produced by core-api's seal(). The
    /// wire format contract lives in both modules' docs (§6 secrets rule).
    pub fn decrypt(&self, sealed: &[u8]) -> Result<Vec<u8>, String> {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Nonce};
        const NONCE_LEN: usize = 12;
        if sealed.len() <= NONCE_LEN {
            return Err("ciphertext too short".into());
        }
        let cipher = Aes256Gcm::new_from_slice(&self.0).map_err(|e| e.to_string())?;
        let (nonce, ct) = sealed.split_at(NONCE_LEN);
        cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| "ciphertext malformed or tampered".to_string())
    }
}

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

/// Settings for the missing-time nudge scheduler (TASKLIST 2.10).
#[derive(Debug, Clone)]
pub struct TimeLoopConfig {
    /// core-api base URL the loop submits due audit batches to.
    pub core_api_url: String,
    /// Shared internal token (`EMAIL_INTERNAL_TOKEN` — the worker↔api
    /// secret; the env name is historical). Unset ⇒ the loop stays idle.
    pub internal_token: Option<String>,
    /// Seconds between due-item sweeps.
    pub poll_secs: u64,
}

/// Settings for the scheduled report delivery loop (TASKLIST R4). Unlike
/// the other loops there is NO disable switch: a transport-unset deploy
/// runs in the ledgered log-skip none mode, so schedules still advance
/// and their runs stay visible.
#[derive(Debug, Clone)]
pub struct DeliveryLoopConfig {
    /// From address on every delivery email (`EMAIL_FROM_ADDRESS` — the
    /// same env core-api enqueues with; defaults to `support@meridian.local`).
    pub from_address: String,
    /// Optional From display name (`EMAIL_FROM_NAME`).
    pub from_name: Option<String>,
    /// Schedules claimed per sweep (`DELIVERY_CLAIM_BATCH`).
    pub claim_batch: i64,
    /// Seconds between due-schedule sweeps (`DELIVERY_POLL_SECS`).
    pub poll_secs: u64,
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
    /// Missing-time EOD nudge scheduler (Stage 2).
    pub time: TimeLoopConfig,
    /// Scheduled report delivery loop (TASKLIST R4).
    pub delivery: DeliveryLoopConfig,
    /// Base URL of core-api for machine effect calls (`INTERNAL_API_URL`).
    pub internal_api_url: String,
    /// Bearer token for `/api/internal/*` (`MARULA_INTERNAL_TOKEN`).
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
    /// At-rest decryption key for webhook subscription secrets
    /// (`MARULA_SECRET_KEY`, 64 hex chars). Unset ⇒ webhook delivery
    /// jobs dead-letter fail-closed.
    pub webhook_secret_key: Option<AtRestKey>,
    /// The org principal decision runs are attributed to
    /// (`MARULA_DECISION_BOT_USER_ID`). Decision answers record
    /// `created_by` with a real-user FK, so a real member id is
    /// required; unset ⇒ the decision subsystem stays disabled
    /// (documented none mode — surfaces act exactly as they would
    /// without a decision model).
    pub decision_bot_user_id: Option<uuid::Uuid>,
}

impl WorkerConfig {
    /// Reads `DATABASE_URL`, `REDIS_URL`, `BIND_ADDR`, `RELAY_BATCH`,
    /// `CORE_API_URL` and `MARULA_INTERNAL_TOKEN` from the process
    /// environment, falling back to dev defaults.
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
            // Owner role on purpose: see module note above. A constrained
            // role here fails SILENTLY — RLS filters every row without
            // erroring and nothing is ever published or evaluated.
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
            time: TimeLoopConfig {
                core_api_url: env("CORE_API_URL")
                    .unwrap_or_else(|| "http://localhost:8080".to_string()),
                internal_token: env("EMAIL_INTERNAL_TOKEN"),
                poll_secs: env("TIME_AUDIT_POLL_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            },
            delivery: DeliveryLoopConfig {
                from_address: env("EMAIL_FROM_ADDRESS")
                    .unwrap_or_else(|| "support@meridian.local".into()),
                from_name: env("EMAIL_FROM_NAME"),
                claim_batch: env("DELIVERY_CLAIM_BATCH")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(20),
                poll_secs: env("DELIVERY_POLL_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
            },
            webhook_secret_key: AtRestKey::from_env_value(lookup("MARULA_SECRET_KEY").as_deref()),
            internal_api_url: lookup("INTERNAL_API_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8080".to_string()),
            internal_token: lookup("MARULA_INTERNAL_TOKEN"),
            eval_interval_secs: lookup("EVAL_INTERVAL_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(60),
            queue_batch: lookup("QUEUE_BATCH")
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            queue_stale_after_secs: lookup("QUEUE_STALE_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            decision_bot_user_id: lookup("MARULA_DECISION_BOT_USER_ID")
                .and_then(|v| v.parse().ok()),
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
        // Fail-closed none mode: no key ⇒ webhook delivery stays dead.
        assert!(cfg.webhook_secret_key.is_none());
    }

    #[test]
    fn at_rest_key_parses_hex_and_decrypts_roundtrip() {
        use aes_gcm::aead::{Aead, KeyInit, OsRng};
        use aes_gcm::{AeadCore, Aes256Gcm};
        let key = AtRestKey([7_u8; 32]);
        let cipher = Aes256Gcm::new_from_slice(&key.0).expect("key");
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut sealed = cipher.encrypt(&nonce, b"whsec".as_ref()).expect("encrypt");
        let mut out = nonce.to_vec();
        out.append(&mut sealed);
        assert_eq!(key.decrypt(&out).expect("decrypt"), b"whsec");

        assert!(AtRestKey::from_env_value(Some("nothex")).is_none());
        assert!(AtRestKey::from_env_value(Some(&"z".repeat(64))).is_none());
        let valid = AtRestKey::from_env_value(Some(&"07".repeat(32))).expect("valid hex");
        assert_eq!(valid.0, [7_u8; 32]);
        assert_eq!(valid.decrypt(b"short").unwrap_err(), "ciphertext too short");
    }

    #[test]
    fn env_values_override_every_field() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("DATABASE_URL", "postgres://db".into());
        vars.insert("REDIS_URL", "redis://r".into());
        vars.insert("BIND_ADDR", "127.0.0.1:1".into());
        vars.insert("RELAY_BATCH", "7".into());
        vars.insert("INTERNAL_API_URL", "http://api:9000".into());
        vars.insert("MARULA_INTERNAL_TOKEN", "sekrit".into());
        vars.insert("EVAL_INTERVAL_SECS", "15".into());
        vars.insert("QUEUE_BATCH", "3".into());
        vars.insert("QUEUE_STALE_SECS", "60".into());
        vars.insert("MARULA_SECRET_KEY", "ab".repeat(32));
        vars.insert("CORE_API_URL", "http://api:9000".into());
        vars.insert("MARULA_INTERNAL_TOKEN", "tok".into());
        let cfg = WorkerConfig::from_lookup(|k| vars.get(k).cloned());
        assert_eq!(cfg.database_url, "postgres://db");
        assert_eq!(cfg.redis_url, "redis://r");
        assert_eq!(cfg.bind_addr, "127.0.0.1:1");
        assert_eq!(cfg.relay_batch, 7);
        assert_eq!(cfg.internal_api_url, "http://api:9000");
        assert_eq!(cfg.internal_token.as_deref(), Some("tok"));
        assert_eq!(cfg.eval_interval_secs, 15);
        assert_eq!(cfg.queue_batch, 3);
        assert_eq!(cfg.queue_stale_after_secs, 60);
        assert!(cfg.webhook_secret_key.is_some());
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
