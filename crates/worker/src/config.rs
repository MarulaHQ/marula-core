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

use core_domain::retry::RetryResolution;
use core_mail::{TransportCredentials, TransportKind};

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
            .decrypt(
                &Nonce::try_from(nonce).map_err(|_| "at-rest nonce has the wrong length")?,
                ct,
            )
            .map_err(|_| "ciphertext malformed or tampered".to_string())
    }
}

/// Settings for the outbound email send loop (the private build tracker 2.4).
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
    /// Idle poll interval between empty claim cycles (`EMAIL_POLL_SECS`,
    /// default 2). A small install can raise it; mail then waits that long
    /// between polls. Errors retry on a fixed short backoff regardless.
    pub poll_secs: u64,
}

/// Settings for the missing-time nudge scheduler (the private build tracker 2.10).
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

/// Settings for the scheduled report delivery loop (the private build tracker R4). Unlike
/// the other loops there is NO disable switch: a transport-unset deploy
/// runs in the ledgered log-skip none mode, so schedules still advance
/// and their runs stay visible.
#[derive(Debug, Clone)]
pub struct DeliveryLoopConfig {
    /// From address on every delivery email (`EMAIL_FROM_ADDRESS` — the
    /// same env core-api enqueues with; defaults to `support@marula.local`).
    pub from_address: String,
    /// Optional From display name (`EMAIL_FROM_NAME`).
    pub from_name: Option<String>,
    /// Schedules claimed per sweep (`DELIVERY_CLAIM_BATCH`).
    pub claim_batch: i64,
    /// Seconds between due-schedule sweeps (`DELIVERY_POLL_SECS`).
    pub poll_secs: u64,
    /// Ceiling on the lazy reporting pool this loop opens for delivery
    /// reads (`DELIVERY_POOL_MAX`, default 5; Wave 49 lane R1).
    pub pool_max: u32,
    /// Statement-cache capacity for that pool (Wave 52 lane P9). It is a
    /// THIRD pool in the worker process, so it inherits neither the app
    /// pool's capacity nor its assumptions; the default matches the app
    /// pool's pooler-safe value.
    pub statement_cache_capacity: u32,
}

/// Runtime settings for the worker process.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Postgres URL; the only database this process touches.
    pub database_url: String,
    /// Direct Postgres URL for this process's ONE `LISTEN` connection
    /// (`MARULA_LISTEN_DATABASE_URL`, Wave 52 lane P9). Unset ⇒
    /// `database_url`.
    ///
    /// A transaction pooler returns a server connection to its pool after
    /// every transaction, so a subscription made through one is attached to
    /// a connection nobody holds: the relay would fall back to its safety
    /// poll with no error anywhere. Point this at Postgres directly whenever
    /// `database_url` names a pooler
    /// (`docs/reference/connection-pooling.md`).
    pub listen_database_url: Option<String>,
    /// sqlx's per-connection prepared-statement cache capacity
    /// (`MARULA_DB_STATEMENT_CACHE`, default 0 = disabled; Wave 52 lane P9).
    /// The worker's owner-role pool runs through the same pooler as
    /// core-api's, so it takes the same answer. A value that does not parse
    /// falls back to 0 rather than refusing boot: 0 is the safe direction
    /// here, unlike a pool ceiling where a silent default is a capacity
    /// incident.
    pub statement_cache_capacity: u32,
    /// Redis URL the outbox relay publishes into.
    pub redis_url: String,
    /// Health-endpoint listen address (`BIND_ADDR`, default
    /// `0.0.0.0:8082`; core-api owns 8080).
    pub bind_addr: String,
    /// Outbox rows relayed per batch (`RELAY_BATCH`).
    pub relay_batch: i64,
    /// Outbound email send loop (Stage 2).
    pub email: EmailLoopConfig,
    /// Missing-time EOD nudge scheduler (Stage 2).
    pub time: TimeLoopConfig,
    /// Scheduled report delivery loop (the private build tracker R4).
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
    /// Idle sleep between empty queue ticks (`QUEUE_IDLE_MS`, default 500).
    /// The general job queue holds webhook and effect work, so the default
    /// stays short; a small install with low job volume can raise it.
    pub queue_idle_ms: u64,
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
    /// Seconds between workflow run-ledger retention sweeps
    /// (`RETENTION_SWEEP_SECS`). The window itself is an ORG SETTING
    /// (`workflow.run_retention_days`, default 30 days); this is only how
    /// often the table is checked against it. Retention is housekeeping,
    /// not user-visible latency, so the default is hours, not seconds.
    pub retention_sweep_secs: u64,
    /// Seconds between org usage refreshes (`USAGE_REFRESH_SECS`). The
    /// usage table is MATERIALIZED (migration 0131), so this is how far
    /// behind a monitoring view can be before the numbers are recomputed;
    /// the payload carries its own `refreshed_at` either way. Minutes, not
    /// seconds: a usage row is a report an operator reads, and the per-org
    /// work is one aggregate.
    pub usage_refresh_secs: u64,
    /// The retention matrix (Wave 52 lane R6): the nine windowed tables, the
    /// profile and any per-table overrides, plus the pass cadence. One
    /// resolution point so the boot log can name the windows this process will
    /// enforce instead of an operator having to read nine modules.
    pub data_retention: crate::data_retention_sweep::DataRetentionConfig,
    /// The reconciliation loop (Wave 52 lane R4): the cadence, the batch and
    /// the two bounds (replay deaths, wake grace). One resolution point so the
    /// boot log can name what this process will repair instead of an operator
    /// reading five modules.
    pub reconcile: crate::reconcile::ReconcileConfig,
    /// The object-store integrity pass (Wave 52 lane R6). Off unless
    /// `MARULA_OBJECT_GC` says otherwise: a pass that deletes bytes should
    /// never be something an operator discovers they enabled.
    pub object_gc: crate::object_gc_sweep::ObjectGcConfig,
    /// Which object store this process walks (`OBJECT_STORE` / `S3_*`, the same
    /// resolution core-api uses). Only read by the integrity pass.
    pub object_store: core_store::StoreChoice,
    /// Event-outbox retention (Wave 52 lane P1): how long a PUBLISHED
    /// `outbox_events` row is kept (`OUTBOX_RETENTION_DAYS`) and how often
    /// the window is enforced (`OUTBOX_SWEEP_SECS`). Deliberately separate
    /// from `retention_sweep_secs` above, which is the workflow run ledger's
    /// — two tables, two windows, two cadences, and confusing them would
    /// either drop events the relay still owes or keep runs forever.
    pub outbox_retention: crate::outbox_retention_sweep::OutboxRetentionConfig,
    /// Entries kept in the event stream before the oldest are trimmed
    /// (`MARULA_EVENT_STREAM_MAXLEN`, default
    /// `core_events::DEFAULT_STREAM_MAXLEN`). Clamped up to
    /// `core_events::MIN_STREAM_MAXLEN` by the publisher, which logs the
    /// resolved value at startup so a typo is visible rather than silent.
    pub stream_maxlen: usize,
    /// Ceiling on the worker's own Postgres pool (`MARULA_DB_POOL_MAX`,
    /// default 20; Wave 49 lane R1). The worker reads across orgs on the
    /// owner role, so this pool is separate from core-api's.
    pub db_pool_max: u32,
    /// How long the relay waits between idle passes (`RELAY_POLL_MS`,
    /// default 5000). Migration `0068` (rebranded by `0114`) fires a
    /// notification on every outbox commit, so this is the BACKSTOP that
    /// bounds delivery when a notification is missed — a reconnect window,
    /// or `MARULA_RELAY_WAKEUP=off`. It is deliberately much slower than the
    /// 250 ms poll it replaces: the poll was the delivery path, this is only
    /// the floor under it.
    pub relay_poll_ms: u64,
    /// Whether the relay LISTENs on the outbox channel
    /// (`MARULA_RELAY_WAKEUP`, default on). Off restores pure polling on
    /// `relay_poll_ms`, which is both the escape hatch if the listener
    /// cannot hold a connection and the baseline this lane measured.
    pub relay_wakeup: bool,
    /// Which loop groups this process runs (`WORKER_ROLES`, default `all`).
    /// The two groups scale differently: the real-time path is safe on every
    /// replica, the sweeps are not — so a scaled deployment runs `realtime`
    /// everywhere and `housekeeping` on one process.
    pub roles: WorkerRoles,
    /// The retry preset every loop reads (`MARULA_ROBUSTNESS`, default
    /// `balanced`) and its per-subsystem overrides
    /// (`MARULA_RETRY_<SUBSYSTEM>_*`). Resolved once at boot; the resolved
    /// matrix is installed into `core_domain::retry` so the queue, the
    /// consumers, the webhook sender and the mail backoff all read the same
    /// numbers.
    pub retry: RetryResolution,
    /// The embedding re-index sweep (Wave 52 lane R14): rows per pass, the
    /// per-minute ceiling that paces it, the idle cadence and the optional UTC
    /// window. The same [`core_domain::embedding::ReindexPolicy`] core-api
    /// describes in its boot log and the CLI prints, so the three cannot
    /// disagree about the rate.
    pub embedding_reindex: core_domain::embedding::ReindexPolicy,
    /// How often the retrieval-distribution sweep records each tenant's
    /// embedding count (`MARULA_RETRIEVAL_WATCH_SECS`, default 300). Wave 52
    /// lane R15: the count is what tells an operator whether the exact vector
    /// scan still fits the tenant (see `retrieval_watch`).
    pub retrieval_watch_secs: u64,
}

/// Which loop groups a worker process runs.
///
/// The split exists because the two groups scale differently. The real-time
/// path — the outbox relay and the stream/queue consumers — is safe on every
/// replica: the relay claims outbox rows inside one transaction and the
/// consumers use Redis consumer groups. The sweeps claim due rows by hand
/// and would duplicate work if several replicas ran them, which is exactly
/// what happened before this switch existed: scaling the worker horizontally
/// duplicated all seventeen loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerRoles {
    realtime: bool,
    housekeeping: bool,
}

impl WorkerRoles {
    pub const fn all() -> Self {
        Self {
            realtime: true,
            housekeeping: true,
        }
    }

    pub const fn realtime_only() -> Self {
        Self {
            realtime: true,
            housekeeping: false,
        }
    }

    pub const fn housekeeping_only() -> Self {
        Self {
            realtime: false,
            housekeeping: true,
        }
    }

    pub const fn realtime(self) -> bool {
        self.realtime
    }

    pub const fn housekeeping(self) -> bool {
        self.housekeeping
    }

    /// The configured groups, for `/healthz` and the startup log.
    pub const fn label(self) -> &'static str {
        match (self.realtime, self.housekeeping) {
            (true, true) => "all",
            (true, false) => "realtime",
            (false, true) => "housekeeping",
            (false, false) => "none",
        }
    }

    /// `all` (the default), `realtime`, `housekeeping`, or a comma-separated
    /// combination. An empty value means `all`; an unknown token is an error
    /// the caller reports and then falls back to `all` — a typo must never
    /// silently stop delivery, and running everything is what the worker did
    /// before the split existed.
    pub fn parse(value: &str) -> Result<Self, String> {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
            return Ok(Self::all());
        }
        let mut realtime = false;
        let mut housekeeping = false;
        for token in trimmed.split(',') {
            match token.trim().to_ascii_lowercase().as_str() {
                "all" => return Ok(Self::all()),
                "realtime" => realtime = true,
                "housekeeping" => housekeeping = true,
                other => {
                    return Err(format!(
                        "unknown WORKER_ROLES token `{other}` (expected `all`, `realtime`, \
                         `housekeeping`, or a comma-separated list)"
                    ))
                }
            }
        }
        if !realtime && !housekeeping {
            return Err("WORKER_ROLES names no runnable group".to_string());
        }
        Ok(Self {
            realtime,
            housekeeping,
        })
    }
}

impl WorkerConfig {
    /// Reads `DATABASE_URL`, `REDIS_URL`, `BIND_ADDR`, `RELAY_BATCH`,
    /// `CORE_API_URL` and `MARULA_INTERNAL_TOKEN` from the process
    /// environment, falling back to dev defaults.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// The URL the relay's ONE `LISTEN` connection dials (Wave 52 lane P9).
    ///
    /// `MARULA_LISTEN_DATABASE_URL` when set, otherwise the pool's URL. A
    /// transaction pooler cannot hold a subscription, so a pooled deployment
    /// must set it; unset keeps the single-box behaviour.
    pub fn listen_url(&self) -> &str {
        self.listen_database_url
            .as_deref()
            .unwrap_or(&self.database_url)
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
        // Both halves of the key pair are required. A lone access key id
        // cannot sign a request, so it resolves to None and the transport
        // falls back to none-mode (mail stays queued) instead of failing
        // every send with a 403 the operator has to decode.
        let ses = match (
            env("AWS_SES_ACCESS_KEY_ID").filter(|v| !v.trim().is_empty()),
            env("AWS_SES_SECRET_ACCESS_KEY").filter(|v| !v.trim().is_empty()),
        ) {
            (Some(access_key_id), Some(secret_access_key)) => Some((
                access_key_id,
                secret_access_key,
                // Region is part of the endpoint, so it defaults rather than
                // being optional — guessing "whichever" sends mail to the
                // wrong data centre.
                env("AWS_SES_REGION")
                    .filter(|v| !v.trim().is_empty())
                    .unwrap_or_else(|| "eu-west-1".into()),
                env("AWS_SES_ENDPOINT").filter(|v| !v.trim().is_empty()),
            )),
            _ => None,
        };
        Self {
            // Owner role on purpose: see module note above. A constrained
            // role here fails SILENTLY — RLS filters every row without
            // erroring and nothing is ever published or evaluated.
            database_url: env("DATABASE_URL")
                .unwrap_or_else(|| "postgres://marula:marula@localhost:15432/marula".to_string()),
            listen_database_url: env("MARULA_LISTEN_DATABASE_URL")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            statement_cache_capacity: env("MARULA_DB_STATEMENT_CACHE")
                .and_then(|v| v.parse().ok())
                .unwrap_or(core_db::DEFAULT_STATEMENT_CACHE_CAPACITY),
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
                    TransportCredentials {
                        smtp,
                        postmark,
                        ses,
                    },
                ),
                claim_batch: env("EMAIL_CLAIM_BATCH")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(25),
                lease_secs: env("EMAIL_LEASE_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
                poll_secs: env("EMAIL_POLL_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(2),
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
                    .unwrap_or_else(|| "support@marula.local".into()),
                from_name: env("EMAIL_FROM_NAME"),
                claim_batch: env("DELIVERY_CLAIM_BATCH")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(20),
                poll_secs: env("DELIVERY_POLL_SECS")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
                pool_max: env("DELIVERY_POOL_MAX")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5),
                statement_cache_capacity: env("MARULA_DB_STATEMENT_CACHE")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(core_db::DEFAULT_STATEMENT_CACHE_CAPACITY),
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
            queue_idle_ms: lookup("QUEUE_IDLE_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(500),
            decision_bot_user_id: lookup("MARULA_DECISION_BOT_USER_ID")
                .and_then(|v| v.parse().ok()),
            retention_sweep_secs: lookup("RETENTION_SWEEP_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(6 * 60 * 60),
            usage_refresh_secs: lookup("USAGE_REFRESH_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::usage_refresh_sweep::DEFAULT_INTERVAL_SECS),
            data_retention: crate::data_retention_sweep::DataRetentionConfig::from_lookup(|key| {
                lookup(key)
            }),
            reconcile: crate::reconcile::ReconcileConfig::from_lookup(|key| lookup(key)),
            object_gc: crate::object_gc_sweep::ObjectGcConfig::from_lookup(|key| lookup(key)),
            embedding_reindex: core_domain::embedding::ReindexPolicy::from_lookup(&|key| {
                lookup(key)
            }),
            retrieval_watch_secs: crate::retrieval_watch::interval_from_lookup(&|key| lookup(key)),
            object_store: core_store::StoreChoice::from_lookup(|key| lookup(key)),
            outbox_retention: crate::outbox_retention_sweep::OutboxRetentionConfig::from_values(
                lookup("OUTBOX_SWEEP_SECS").and_then(|v| v.parse().ok()),
                lookup("OUTBOX_RETENTION_DAYS").and_then(|v| v.parse().ok()),
            ),
            stream_maxlen: lookup("MARULA_EVENT_STREAM_MAXLEN")
                .and_then(|v| v.parse().ok())
                .unwrap_or(core_events::DEFAULT_STREAM_MAXLEN),
            db_pool_max: lookup("MARULA_DB_POOL_MAX")
                .and_then(|v| v.parse().ok())
                .unwrap_or(core_db::DEFAULT_POOL_MAX),
            relay_poll_ms: lookup("RELAY_POLL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(5_000),
            relay_wakeup: match lookup("MARULA_RELAY_WAKEUP") {
                // An explicit value wins; anything but a clear "off" keeps
                // the wakeup on, because losing it to a typo would silently
                // reintroduce the poll the trigger replaced.
                Some(v) => !matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "off" | "0" | "false" | "no"
                ),
                None => true,
            },
            roles: match lookup("WORKER_ROLES") {
                None => WorkerRoles::all(),
                Some(v) => WorkerRoles::parse(&v).unwrap_or_else(|error| {
                    tracing::warn!(%error, value = %v, "WORKER_ROLES invalid; running all loops");
                    WorkerRoles::all()
                }),
            },
            // One preset sets every budget; the per-subsystem env group is the
            // operator's escape hatch. A refused override is logged rather
            // than silently replaced by a guess.
            retry: {
                let resolved = core_domain::retry::RetryMatrix::from_lookup(|key| lookup(key));
                for warning in &resolved.warnings {
                    tracing::warn!(%warning, "retry override refused; keeping the preset value");
                }
                tracing::info!(
                    preset = resolved.matrix.preset().as_str(),
                    "retry policy resolved"
                );
                resolved
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
            "postgres://marula:marula@localhost:15432/marula"
        );
        assert_eq!(cfg.redis_url, "redis://localhost:16379/0");
        assert_eq!(cfg.bind_addr, "0.0.0.0:8082");
        assert_eq!(cfg.relay_batch, 100);
        assert_eq!(cfg.internal_api_url, "http://127.0.0.1:8080");
        assert_eq!(cfg.internal_token, None);
        assert_eq!(cfg.eval_interval_secs, 60);
        assert_eq!(cfg.queue_batch, 10);
        assert_eq!(cfg.queue_stale_after_secs, 300);
        // Wave 49 lane R6: the two fast-idle cadences are knobs now; the
        // defaults keep the historical behaviour.
        assert_eq!(cfg.queue_idle_ms, 500);
        assert_eq!(cfg.email.poll_secs, 2);
        // Fail-closed none mode: no key ⇒ webhook delivery stays dead.
        assert!(cfg.webhook_secret_key.is_none());
        // Retention ships the standard profile, and the object pass is OFF: a
        // process that deletes bytes has to be asked to.
        assert_eq!(
            cfg.data_retention.plan.profile,
            core_db::retention::RetentionProfile::Standard
        );
        assert_eq!(cfg.object_gc.mode, crate::object_gc_sweep::GcMode::Off);
        assert_eq!(
            cfg.object_store,
            core_store::StoreChoice::Local("./data/objects".into())
        );
        // No SES variables ⇒ none mode, not a half-configured adapter.
        assert_eq!(cfg.email.transport_kind, TransportKind::None);
    }

    /// SES is the third transport choice, and the failure mode that matters
    /// is a PARTIAL credential: an access key id with no secret cannot sign
    /// anything, and resolving that to a live adapter produces a 403 on
    /// every single send. It has to resolve to none-mode instead, so the mail
    /// stays queued and the ledger says why.
    #[test]
    fn ses_resolves_only_with_both_halves_of_the_key_pair() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("EMAIL_TRANSPORT", "ses".into());
        vars.insert("AWS_SES_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
        vars.insert("AWS_SES_SECRET_ACCESS_KEY", "shhh".into());
        vars.insert("AWS_SES_REGION", "us-east-1".into());
        let cfg = WorkerConfig::from_lookup(move |k| vars.get(k).cloned());
        assert_eq!(
            cfg.email.transport_kind,
            TransportKind::Ses {
                access_key_id: "AKIAEXAMPLE".into(),
                secret_access_key: "shhh".into(),
                region: "us-east-1".into(),
                endpoint: None,
            }
        );
        assert_eq!(cfg.email.transport_kind.as_str(), "ses");

        // Key id alone ⇒ none mode, not a broken adapter.
        let mut half: HashMap<&str, String> = HashMap::new();
        half.insert("EMAIL_TRANSPORT", "ses".into());
        half.insert("AWS_SES_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
        let cfg = WorkerConfig::from_lookup(move |k| half.get(k).cloned());
        assert_eq!(cfg.email.transport_kind, TransportKind::None);

        // Secret alone ⇒ the same.
        let mut half: HashMap<&str, String> = HashMap::new();
        half.insert("EMAIL_TRANSPORT", "ses".into());
        half.insert("AWS_SES_SECRET_ACCESS_KEY", "shhh".into());
        let cfg = WorkerConfig::from_lookup(move |k| half.get(k).cloned());
        assert_eq!(cfg.email.transport_kind, TransportKind::None);
    }

    #[test]
    fn ses_region_defaults_and_blank_values_are_treated_as_unset() {
        // Region is part of the endpoint, so it has a default rather than
        // being optional — a missing region that resolved to "whichever"
        // would send mail to the wrong data centre.
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("EMAIL_TRANSPORT", "SES".into());
        vars.insert("AWS_SES_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
        vars.insert("AWS_SES_SECRET_ACCESS_KEY", "shhh".into());
        let cfg = WorkerConfig::from_lookup(move |k| vars.get(k).cloned());
        assert_eq!(
            cfg.email.transport_kind,
            TransportKind::Ses {
                access_key_id: "AKIAEXAMPLE".into(),
                secret_access_key: "shhh".into(),
                region: "eu-west-1".into(),
                endpoint: None,
            }
        );

        // A whitespace-only region is not a region. Treating " " as the
        // region would produce an endpoint like `email. .amazonaws.com`.
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("EMAIL_TRANSPORT", "ses".into());
        vars.insert("AWS_SES_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
        vars.insert("AWS_SES_SECRET_ACCESS_KEY", "shhh".into());
        vars.insert("AWS_SES_REGION", "   ".into());
        vars.insert("AWS_SES_ENDPOINT", "  ".into());
        let cfg = WorkerConfig::from_lookup(move |k| vars.get(k).cloned());
        assert_eq!(
            cfg.email.transport_kind,
            TransportKind::Ses {
                access_key_id: "AKIAEXAMPLE".into(),
                secret_access_key: "shhh".into(),
                region: "eu-west-1".into(),
                endpoint: None,
            }
        );
    }

    /// A typo in `EMAIL_TRANSPORT` must not select some other provider, and
    /// must not send anywhere at all.
    #[test]
    fn an_unknown_transport_name_never_selects_a_provider() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        // Every credential present, and the name is misspelled.
        vars.insert("EMAIL_TRANSPORT", "postmzak".into());
        vars.insert("POSTMARK_SERVER_TOKEN", "tok".into());
        vars.insert("AWS_SES_ACCESS_KEY_ID", "AKIAEXAMPLE".into());
        vars.insert("AWS_SES_SECRET_ACCESS_KEY", "shhh".into());
        vars.insert("SMTP_HOST", "localhost".into());
        let cfg = WorkerConfig::from_lookup(move |k| vars.get(k).cloned());
        assert_eq!(cfg.email.transport_kind, TransportKind::None);
    }

    #[test]
    fn at_rest_key_parses_hex_and_decrypts_roundtrip() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::Aes256Gcm;
        let key = AtRestKey([7_u8; 32]);
        let cipher = Aes256Gcm::new_from_slice(&key.0).expect("key");
        let mut nonce_bytes = [0u8; 12];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
        let nonce = aes_gcm::Nonce::from(nonce_bytes);
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
        vars.insert("QUEUE_IDLE_MS", "2000".into());
        vars.insert("EMAIL_POLL_SECS", "45".into());
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
        assert_eq!(cfg.queue_idle_ms, 2000);
        assert_eq!(cfg.email.poll_secs, 45);
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

    /// Wave 49 lane R1: the worker's own pool and the delivery reporting
    /// pool carry env ceilings, defaulting to the historical numbers.
    #[test]
    fn pool_ceilings_default_and_override() {
        let cfg = WorkerConfig::from_lookup(|_| None);
        assert_eq!(cfg.db_pool_max, 20);
        assert_eq!(cfg.delivery.pool_max, 5);

        let cfg = WorkerConfig::from_lookup(|k| match k {
            "MARULA_DB_POOL_MAX" => Some("40".into()),
            "DELIVERY_POOL_MAX" => Some("8".into()),
            _ => None,
        });
        assert_eq!(cfg.db_pool_max, 40);
        assert_eq!(cfg.delivery.pool_max, 8);
    }

    /// Wave 52 lane P1: the stream bound and the outbox window are knobs, and
    /// their defaults are the shipped ones. The bound matters more than most:
    /// unset must resolve to a BOUNDED stream, because the failure it guards
    /// is Valkey refusing writes at its memory ceiling, not a slow tail.
    #[test]
    fn outbox_window_and_stream_bound_default_and_override() {
        let cfg = WorkerConfig::from_lookup(|_| None);
        assert_eq!(cfg.stream_maxlen, core_events::DEFAULT_STREAM_MAXLEN);
        assert_eq!(
            cfg.outbox_retention,
            crate::outbox_retention_sweep::OutboxRetentionConfig {
                interval_secs: crate::outbox_retention_sweep::DEFAULT_INTERVAL_SECS,
                retention_days: crate::outbox_retention_sweep::DEFAULT_RETENTION_DAYS,
            }
        );

        let cfg = WorkerConfig::from_lookup(|k| match k {
            "MARULA_EVENT_STREAM_MAXLEN" => Some("200000".into()),
            "OUTBOX_RETENTION_DAYS" => Some("30".into()),
            "OUTBOX_SWEEP_SECS" => Some("900".into()),
            _ => None,
        });
        assert_eq!(cfg.stream_maxlen, 200_000);
        assert_eq!(cfg.outbox_retention.retention_days, 30);
        assert_eq!(cfg.outbox_retention.interval_secs, 900);
    }

    /// The wakeup is on by default and only a clear negative turns it off:
    /// a typo must not silently restore the poll the trigger replaced.
    #[test]
    fn relay_wakeup_defaults_on_and_negative_values_turn_it_off() {
        let cfg = WorkerConfig::from_lookup(|_| None);
        assert!(cfg.relay_wakeup);
        assert_eq!(cfg.relay_poll_ms, 5_000);

        for off in ["off", "OFF", "0", "false", "no"] {
            let cfg = WorkerConfig::from_lookup(|k| match k {
                "MARULA_RELAY_WAKEUP" => Some(off.into()),
                _ => None,
            });
            assert!(!cfg.relay_wakeup, "`{off}` must mean off");
        }
        for on in ["on", "1", "true", "yes", "banana"] {
            let cfg = WorkerConfig::from_lookup(|k| match k {
                "MARULA_RELAY_WAKEUP" => Some(on.into()),
                _ => None,
            });
            assert!(cfg.relay_wakeup, "`{on}` must keep the wakeup on");
        }

        // The measurement baseline is the old worker: poll only, at 250 ms.
        let baseline = WorkerConfig::from_lookup(|k| match k {
            "MARULA_RELAY_WAKEUP" => Some("off".into()),
            "RELAY_POLL_MS" => Some("250".into()),
            _ => None,
        });
        assert!(!baseline.relay_wakeup);
        assert_eq!(baseline.relay_poll_ms, 250);
    }

    #[test]
    fn worker_roles_parse_every_named_shape() {
        assert_eq!(WorkerRoles::parse("").unwrap(), WorkerRoles::all());
        assert_eq!(WorkerRoles::parse("  ").unwrap(), WorkerRoles::all());
        assert_eq!(WorkerRoles::parse("all").unwrap(), WorkerRoles::all());
        assert_eq!(
            WorkerRoles::parse("realtime").unwrap(),
            WorkerRoles::realtime_only()
        );
        assert_eq!(
            WorkerRoles::parse("HOUSEKEEPING").unwrap(),
            WorkerRoles::housekeeping_only()
        );
        assert_eq!(
            WorkerRoles::parse("realtime,housekeeping").unwrap(),
            WorkerRoles::all()
        );
        assert_eq!(
            WorkerRoles::parse(" housekeeping , realtime ").unwrap(),
            WorkerRoles::all()
        );
        // An unknown token is refused, so the caller can warn instead of
        // guessing which half the operator meant.
        assert!(WorkerRoles::parse("sweeps").is_err());
        assert!(WorkerRoles::parse("realtime,worker").is_err());
        // A lone comma names nothing runnable.
        assert!(WorkerRoles::parse(",").is_err());
    }

    /// A default config runs every loop: the split must not change behaviour
    /// for an install that never sets the switch.
    #[test]
    fn default_config_runs_every_loop_group() {
        let cfg = WorkerConfig::from_lookup(|_| None);
        assert!(cfg.roles.realtime());
        assert!(cfg.roles.housekeeping());
    }
}
