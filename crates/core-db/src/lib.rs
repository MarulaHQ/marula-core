//! core-db — SQLx queries and versioned migrations for Meridian.
//!
//! Purpose: the only place SQL lives. Every query is compile-time checked
//! against the schema (offline `.sqlx` data committed to the repo); RLS is
//! applied per transaction via `set_org`. Migrations are forward-only and
//! checksum-tracked in `_sqlx_migrations` (alembic-style history).
//!
//! Non-purpose: no business rules. Decisions belong in core-domain; this
//! crate persists and retrieves.

pub mod audit;
pub mod automations;
pub mod boards;
pub mod checklists;
pub mod clients;
pub mod emails;
pub mod mfa;
pub mod notifications;
pub mod oidc;
pub mod orgs;
pub mod outbox;
pub mod queue;
pub mod sla;
pub mod tickets;
pub mod timers;
pub mod users;

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

/// Binds the transaction to one org for RLS (`SET LOCAL` semantics: scoped
/// to the current transaction, auto-reset on commit/rollback).
pub async fn set_org(tx: &mut Transaction<'_, Postgres>, org_id: Uuid) -> DbResult<()> {
    sqlx::query("SELECT set_config('app.org_id', $1, true)")
        .bind(org_id.to_string())
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
