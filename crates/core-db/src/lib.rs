//! core-db — SQLx queries and versioned migrations for Marula.
//!
//! Purpose: the only place SQL lives. Every query is compile-time checked
//! against the schema (offline `.sqlx` data committed to the repo); RLS is
//! applied per transaction via `set_org`. Migrations are forward-only and
//! checksum-tracked in `_sqlx_migrations` (alembic-style history).
//!
//! Non-purpose: no business rules. Decisions belong in core-domain; this
//! crate persists and retrieves.

pub mod agent_platform;
pub mod ai_policy;
pub mod alerting;
pub mod approvals;
pub mod assets;
pub mod attachment_scans;
pub mod attachments;
pub mod audit;
pub mod automations;
pub mod boards;
pub mod byok;
pub mod calendar_sync;
pub mod capacity;
pub mod catalog;
pub mod checklists;
pub mod client_completeness;
pub mod clients;
pub mod config_identity;
pub mod connector;
pub mod consumer;
pub mod contracts;
pub mod csat;
pub mod custom_fields;
pub mod day_pack;
pub mod decision;
// Dev-tool surface (the private build tracker DX1–DX2): only reachable behind
// MARULA_DEV_TOOLS=1 — the CLI refuses and the API router is not mounted
// without it.
pub mod costs;
pub mod credit_notes;
pub mod dead_work;
pub mod demo;
pub mod directory_groups;
pub mod dispatch;
pub mod document_attachments;
pub mod document_layouts;
pub mod document_links;
pub mod documents;
pub mod emails;
pub mod embedding_space;
pub mod entitlements;
pub mod entity_dedupe;
pub mod erasure;
pub mod event_types;
pub mod export;
pub mod federation;
pub mod fx;
pub mod hardening;
pub mod identity_bundles;
pub mod imports;
pub mod instant;
pub mod integrations;
pub mod invoices;
pub mod kb;
pub mod kb_rich;
pub mod labels;
pub mod loans;
pub mod manifest;
pub mod mfa;
pub mod node_registry;
pub mod note_drafts;
pub mod notifications;
pub mod object_gc;
pub mod object_inventory;
pub mod oidc;
pub mod onboarding;
pub mod org_config;
pub mod org_custom_domain;
pub mod org_exports;
pub mod org_suspension;
pub mod org_usage;
pub mod org_users;
pub mod orgs;
pub mod outbox;
pub mod partitions;
pub mod passwords;
pub mod pipeline;
pub mod plugin_credentials;
pub mod plugin_distribution;
pub mod plugin_health;
pub mod plugin_kv;
pub mod plugin_signing_keys;
pub mod plugin_surface;
pub mod plugins;
pub mod portal;
pub mod projects;
pub mod provenance;
pub mod purchasing;
pub mod queue;
pub mod quotes;
pub mod rag;
pub mod reconcile;
pub mod recurring;
pub mod reply_snippets;
pub mod report_defs;
pub mod report_schedules;
pub mod retention;
pub mod retrieval;
pub mod roles;
pub mod routing;
pub mod scheduling;
pub mod scheduling_links;
pub mod scorecards;
pub mod search;
pub mod search_pins;
pub mod secret_vault;
pub mod service_accounts;
pub mod settings;
pub mod share_links;
pub mod sign_offs;
pub mod sla;
pub mod stock;
pub mod tags;
pub mod taxonomy;
pub mod templates;
pub mod theme_packs;
pub mod ticket_links;
pub mod tickets;
pub mod time_audit;
pub mod time_categories;
pub mod timers;
pub mod tools;
pub mod undo;
pub mod users;
pub mod vault_scale;
pub mod verification;
pub mod webhooks;
pub mod workflow_overrides;
pub mod workflow_runs;

use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool, Postgres, Transaction,
};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sql(#[from] sqlx::Error),
    /// A pool that could not open its first connection, with the refusal the
    /// single attempt underneath it gave. Distinct from [`DbError::Sql`]
    /// because `PoolTimedOut` carries no source: this variant exists so an
    /// operator reads "password authentication failed", not "pool timed out".
    #[error("database: {0}")]
    Connect(String),
    #[error("migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("row not found")]
    NotFound,
    /// Typed refusal from a guard inside a query module (e.g. mutating an
    /// already-billed cost line). The API layer maps this to 409.
    #[error("conflict: {0}")]
    Conflict(String),
}

impl DbError {
    /// Normalizes a raw driver error for typed matching: a `fetch_one`
    /// miss arrives as `Sql(RowNotFound)`, but every caller that branches
    /// on absence matches `DbError::NotFound` (e.g. the staged-plugin
    /// adopt's mint arm — Wave 34 lane 4 cross-lane fix). Idempotent on
    /// already-normalized values.
    pub fn normalized(self) -> Self {
        match self {
            DbError::Sql(sqlx::Error::RowNotFound) => DbError::NotFound,
            other => other,
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

/// Historical app-pool ceiling (Wave 49 lane R1). `docs/perf.md` §5 sized
/// the first instance against this number; it stays the default so no
/// existing deployment changes behaviour until an operator moves it.
pub const DEFAULT_POOL_MAX: u32 = 20;

/// Acquire timeout the app pool shipped with (`docs/perf.md` §5).
pub const DEFAULT_ACQUIRE_TIMEOUT_SECS: u64 = 10;

/// Prepared-statement cache capacity per connection default.
///
/// `0` disables sqlx's statement cache, which is what a transaction pooler
/// needs: sqlx then parses a statement and executes it inside the same
/// transaction under a fresh name, and never asks a later transaction to
/// reuse a name that a different server connection may never have seen.
/// `docs/reference/connection-pooling.md` carries the measurement and the
/// alternative (bounded cache) for a deployment that runs without a pooler.
pub const DEFAULT_STATEMENT_CACHE_CAPACITY: u32 = 0;

/// App-pool sizing knobs (Wave 49 lane R1).
///
/// Purpose: carry the pool ceiling and acquire timeout as data instead of
/// the literal `20` this module shipped. A second instance, a bigger box
/// or a tiny demo install each needs the ceiling to move without a code
/// edit; a session pooler (PgBouncer/pgcat) in front is the sanctioned way
/// to raise the fleet total past Postgres's `max_connections`.
///
/// Non-purpose: this type reads no environment. The app config owns the
/// env parse and its fail-closed posture, then hands the resolved value
/// here — which is also what the future deploy descriptor will emit.
#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    /// Ceiling on simultaneous connections this pool will hold.
    pub max_connections: u32,
    /// How long a caller waits for a free connection before erroring.
    pub acquire_timeout: std::time::Duration,
    /// sqlx's per-connection prepared-statement cache capacity (Wave 52 lane
    /// P9). `0` disables it; see [`DEFAULT_STATEMENT_CACHE_CAPACITY`].
    pub statement_cache_capacity: u32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_POOL_MAX,
            acquire_timeout: std::time::Duration::from_secs(DEFAULT_ACQUIRE_TIMEOUT_SECS),
            statement_cache_capacity: DEFAULT_STATEMENT_CACHE_CAPACITY,
        }
    }
}

/// Connects a pool with an explicit ceiling and timeout.
///
/// The URL is parsed into connect options rather than handed to
/// `PgPoolOptions::connect(&str)` so the statement-cache capacity travels
/// with the pool: a transaction pooler in front (`docs/reference/connection-pooling.md`)
/// makes a cross-transaction statement name unsafe.
pub async fn connect_with(url: &str, config: PoolConfig) -> DbResult<PgPool> {
    let options = connect_options(url, config.statement_cache_capacity)?;
    let target = target_of(&options);
    let attempt = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options.clone())
        .await;
    match attempt {
        Ok(pool) => Ok(pool),
        Err(sqlx::Error::PoolTimedOut) => {
            // A pool that cannot open its *first* connection reports only that
            // it ran out of time, and `PoolTimedOut` carries no source. The
            // refusal underneath — a wrong password, a closed port, a database
            // that is not there — is the part an operator can act on, so ask
            // once more without the pool and keep that answer. Wave 60 lane W60-6
            // spent a run guessing because this line read "pool timed out" with
            // Postgres healthy beside it.
            match tokio::time::timeout(
                config.acquire_timeout,
                <sqlx::PgConnection as sqlx::Connection>::connect_with(&options),
            )
            .await
            {
                Ok(Err(refusal)) => Err(DbError::Connect(format!(
                    "cannot open a connection to {target}: {refusal} \
                     (the pool waited {}s for its first connection)",
                    config.acquire_timeout.as_secs()
                ))),
                // The single attempt hung or succeeded, so the pool's verdict
                // is all that is known: report it with the host named, since a
                // bare timeout in a log is what this arm exists to prevent.
                _ => Err(DbError::Connect(format!(
                    "cannot open a connection to {target}: the attempt did not \
                     answer within {}s",
                    config.acquire_timeout.as_secs()
                ))),
            }
        }
        Err(other) => Err(DbError::Sql(other)),
    }
}

/// The host and database a connection is being made to, without credentials.
///
/// A diagnostic must be safe to paste into a log or an issue: the password and
/// the user name stay out of it, and the address stays in.
fn target_of(options: &PgConnectOptions) -> String {
    let database = options.get_database().unwrap_or("(default)");
    format!("{}:{}/{}", options.get_host(), options.get_port(), database)
}

/// Parses a Postgres URL and applies the statement-cache capacity.
///
/// Shared with the reporting pool so every pool this crate can build reads
/// the same knob from the same place.
pub fn connect_options(url: &str, statement_cache_capacity: u32) -> DbResult<PgConnectOptions> {
    let options = PgConnectOptions::from_str(url)?;
    Ok(options.statement_cache_capacity(statement_cache_capacity as usize))
}

/// Connects the app pool with the historical defaults ([`PoolConfig::default`]).
pub async fn connect(url: &str) -> DbResult<PgPool> {
    connect_with(url, PoolConfig::default()).await
}

/// Applies all pending migrations in filename order. Idempotent: applied
/// migrations are checksum-verified and skipped.
pub async fn run_migrations(pool: &PgPool) -> DbResult<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    // Record the node catalogue release this build runs: a workflow
    // document can then say which release it was validated against, and
    // two builds claiming the same version with different declarations
    // stay detectable. It is BOOKKEEPING, not a schema invariant — the
    // migrations above are what make the database correct, so a failed pin
    // is logged and the boot continues rather than blaming the schema.
    // `record_pin` refuses to rewrite a recorded release anyway.
    if let Err(e) = record_catalogue_pin(pool).await {
        tracing::debug!(error = %e, "node catalogue pin not recorded");
    }
    Ok(())
}

async fn record_catalogue_pin(pool: &PgPool) -> DbResult<()> {
    let mut tx = pool.begin().await?;
    node_registry::record_pin(
        &mut tx,
        core_domain::node_registry::CATALOGUE_VERSION,
        &core_domain::node_registry::catalogue_digest(),
        core_domain::node_registry::all().len() as i32,
        Some("core-db::run_migrations"),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The highest migration version embedded in THIS binary (compile-time);
/// the dev status surface reports it as the migration head.
pub fn migration_head() -> Option<i64> {
    sqlx::migrate!("./migrations")
        .migrations
        .iter()
        .map(|migration| migration.version)
        .max()
}

/// Binds the transaction to one org for RLS (`SET LOCAL` semantics: scoped
/// to the current transaction, auto-reset on commit/rollback).
pub async fn set_org(tx: &mut Transaction<'_, Postgres>, org_id: Uuid) -> DbResult<()> {
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Allows the append-only ledger guards to be dropped FOR THIS TRANSACTION.
///
/// The demo/reset tooling (and its tests) wipe an org by cascade, which walks
/// `stock_movements` and trips the append-only trigger. Tenant code never
/// calls this, so the ledger guarantee is unchanged; a forgotten call fails
/// closed. Migration 0119 defines the guard's side of this flag.
pub async fn set_dev_reset_bypass(tx: &mut Transaction<'_, Postgres>) -> DbResult<()> {
    sqlx::query("SELECT set_config('app.dev_reset_bypass', 'on', true)")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Binds the transaction to one org AND one user. The extra `app.user_id`
/// GUC lets per-user RLS policies (settings) hide sibling users' rows even
/// inside a correctly-org-scoped transaction.
pub async fn set_org_and_user(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    user_id: Uuid,
) -> DbResult<()> {
    sqlx::query("SELECT set_config('app.org_id', $1, true), set_config('app.user_id', $2, true)")
        .bind(org_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// CI gate helper: every user-data table (has org_id, not an RLS-internal
/// table) must carry at least one RLS policy. A migration adding a
/// policy-less table fails this query.
///
/// `relkind` is `('r', 'p')` and not `'r'` alone: a PARTITIONED TABLE reports
/// `'p'`, so converting a table to a partition (migration 0146 does this to
/// `auth_attempt_log`) would otherwise walk it out of the one gate that checks
/// it — the table would keep RLS only because someone remembered to re-enable
/// it on the new parent. The child partitions are `'r'` and are checked too,
/// which matters because a policy on the parent does not answer for a query
/// that targets a child directly.
pub async fn assert_rls_coverage(pool: &PgPool) -> DbResult<()> {
    let offenders: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relkind IN ('r', 'p')
          AND EXISTS (
            SELECT 1 FROM information_schema.columns col
            WHERE col.table_schema = 'public'
              AND col.table_name = c.relname
              AND col.column_name = 'org_id'
          )
          AND c.relrowsecurity = false
        "#,
    )
    .fetch_all(pool)
    .await?;
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(DbError::Sql(sqlx::Error::Configuration(
            format!("tables missing RLS: {offenders:?}").into(),
        )))
    }
}

#[cfg(test)]
mod pool_config_tests {
    use super::*;

    /// The default must stay the number `docs/perf.md` §5 sized the first
    /// instance against — moving it silently would invalidate that budget.
    #[test]
    fn default_pool_config_is_the_historical_ceiling() {
        let cfg = PoolConfig::default();
        assert_eq!(cfg.max_connections, DEFAULT_POOL_MAX);
        assert_eq!(DEFAULT_POOL_MAX, 20);
        assert_eq!(cfg.acquire_timeout.as_secs(), DEFAULT_ACQUIRE_TIMEOUT_SECS);
    }

    #[test]
    fn pool_config_is_overridable() {
        let cfg = PoolConfig {
            max_connections: 5,
            acquire_timeout: std::time::Duration::from_secs(2),
            statement_cache_capacity: 64,
        };
        assert_eq!(cfg.max_connections, 5);
        assert_eq!(cfg.acquire_timeout.as_secs(), 2);
        assert_eq!(cfg.statement_cache_capacity, 64);
        assert_ne!(cfg.max_connections, DEFAULT_POOL_MAX);
    }

    /// The shipped default has to be the pooler-safe one, or every deployment
    /// that puts PgBouncer in front of the pools takes the cross-transaction
    /// statement-name failure in `docs/reference/connection-pooling.md`.
    #[test]
    fn default_pool_config_disables_the_statement_cache() {
        assert_eq!(PoolConfig::default().statement_cache_capacity, 0);
        assert_eq!(DEFAULT_STATEMENT_CACHE_CAPACITY, 0);
    }

    /// The knob has to reach sqlx's connect options, not just sit in the
    /// struct: this is the value a transaction pooler's
    /// `max_prepared_statements` is compared against.
    #[test]
    fn statement_cache_capacity_reaches_the_connect_options() {
        use sqlx::ConnectOptions;

        let capacity_of = |url: &str, capacity: u32| -> String {
            let options = connect_options(url, capacity).expect("parse");
            options
                .to_url_lossy()
                .query_pairs()
                .find(|(k, _)| k == "statement-cache-capacity")
                .map(|(_, v)| v.into_owned())
                .expect("connect options carry the capacity")
        };

        assert_eq!(capacity_of("postgres://u:p@127.0.0.1:1/db", 0), "0");
        assert_eq!(capacity_of("postgres://u:p@127.0.0.1:1/db", 64), "64");
    }

    /// Port 1 on loopback refuses instantly, so this is a refusal the pool will
    /// spend its whole acquire timeout retrying. The point of the assertion is
    /// that the operator is told *why*: `PoolTimedOut` alone is what made Wave
    /// 60 lane W60-6 guess for a day while Postgres was healthy beside it.
    #[tokio::test]
    async fn a_pool_that_cannot_connect_reports_the_refusal_and_the_target() {
        let config = PoolConfig {
            max_connections: 1,
            acquire_timeout: std::time::Duration::from_secs(2),
            ..PoolConfig::default()
        };
        let error = connect_with("postgres://marula:secret@127.0.0.1:1/marula", config)
            .await
            .expect_err("nothing listens on port 1");
        let message = error.to_string();
        assert!(
            matches!(error, DbError::Connect(_)),
            "a connect failure is its own variant, got {message}"
        );
        assert!(
            message.contains("127.0.0.1:1/marula"),
            "the message names the target: {message}"
        );
        assert!(
            !message.contains("secret") && !message.contains('@'),
            "the message carries no credential: {message}"
        );
    }
}
