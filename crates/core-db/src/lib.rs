//! core-db — SQLx queries and versioned migrations for Meridian.
//!
//! Purpose: the only place SQL lives. Every query is compile-time checked
//! against the schema (offline `.sqlx` data committed to the repo); RLS is
//! applied per transaction via `set_org`. Migrations are forward-only and
//! checksum-tracked in `_sqlx_migrations` (alembic-style history).
//!
//! Non-purpose: no business rules. Decisions belong in core-domain; this
//! crate persists and retrieves.

pub mod agent_platform;
pub mod alerting;
pub mod approvals;
pub mod assets;
pub mod attachments;
pub mod audit;
pub mod automations;
pub mod boards;
pub mod byok;
pub mod catalog;
pub mod checklists;
pub mod clients;
pub mod config_identity;
pub mod connector;
pub mod contracts;
pub mod custom_fields;
// Dev-tool surface (TASKLIST DX1–DX2): only reachable behind
// MERIDIAN_DEV_TOOLS=1 — the CLI refuses and the API router is not mounted
// without it.
pub mod costs;
pub mod credit_notes;
pub mod demo;
pub mod dispatch;
pub mod document_layouts;
pub mod document_links;
pub mod documents;
pub mod emails;
pub mod export;
pub mod federation;
pub mod fx;
pub mod imports;
pub mod invoices;
pub mod kb;
pub mod kb_rich;
pub mod manifest;
pub mod mfa;
pub mod notifications;
pub mod oidc;
pub mod org_config;
pub mod org_users;
pub mod orgs;
pub mod outbox;
pub mod passwords;
pub mod pipeline;
pub mod portal;
pub mod projects;
pub mod provenance;
pub mod queue;
pub mod quotes;
pub mod recurring;
pub mod report_defs;
pub mod report_schedules;
pub mod roles;
pub mod scheduling;
pub mod scheduling_links;
pub mod search;
pub mod settings;
pub mod sla;
pub mod tags;
pub mod taxonomy;
pub mod templates;
pub mod ticket_links;
pub mod tickets;
pub mod time_audit;
pub mod timers;
pub mod undo;
pub mod users;
pub mod webhooks;
pub mod workflow_overrides;

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
