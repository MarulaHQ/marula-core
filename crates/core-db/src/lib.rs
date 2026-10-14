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
pub mod attachments;
pub mod audit;
pub mod automations;
pub mod boards;
pub mod byok;
pub mod calendar_sync;
pub mod catalog;
pub mod checklists;
pub mod client_completeness;
pub mod clients;
pub mod config_identity;
pub mod connector;
pub mod contracts;
pub mod csat;
pub mod custom_fields;
pub mod day_pack;
pub mod decision;
// Dev-tool surface (TASKLIST DX1–DX2): only reachable behind
// MARULA_DEV_TOOLS=1 — the CLI refuses and the API router is not mounted
// without it.
pub mod costs;
pub mod credit_notes;
pub mod demo;
pub mod dispatch;
pub mod document_attachments;
pub mod document_layouts;
pub mod document_links;
pub mod documents;
pub mod emails;
pub mod entity_dedupe;
pub mod event_types;
pub mod export;
pub mod federation;
pub mod fx;
pub mod hardening;
pub mod imports;
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
pub mod oidc;
pub mod onboarding;
pub mod org_config;
pub mod org_exports;
pub mod org_usage;
pub mod org_users;
pub mod orgs;
pub mod outbox;
pub mod passwords;
pub mod pipeline;
pub mod plugin_distribution;
pub mod plugin_signing_keys;
pub mod plugin_surface;
pub mod plugins;
pub mod portal;
pub mod projects;
pub mod provenance;
pub mod purchasing;
pub mod queue;
pub mod quotes;
pub mod recurring;
pub mod reply_snippets;
pub mod report_defs;
pub mod report_schedules;
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

use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sql(#[from] sqlx::Error),
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

pub async fn connect(url: &str) -> DbResult<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(url)
        .await?;
    Ok(pool)
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
pub async fn assert_rls_coverage(pool: &PgPool) -> DbResult<()> {
    let offenders: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relkind = 'r'
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
