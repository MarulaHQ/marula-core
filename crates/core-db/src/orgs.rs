//! Org lifecycle: creation bootstraps default roles, default board with
//! semantic statuses, and the default SLA policy in one transaction
//! (defaults-first doctrine — a new org is immediately operational).

use crate::{DbError, DbResult};
use core_domain::entities::Org;
use core_domain::policy::BuiltinRole;
use core_domain::status::TicketStatusCategory;
use sqlx::{PgConnection, Postgres};
use std::collections::HashMap;
use uuid::Uuid;

pub async fn insert_org<'e, E>(
    executor: E,
    id: Uuid,
    name: &str,
    slug: &str,
    ticket_prefix: &str,
) -> DbResult<Org>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"INSERT INTO orgs (id, name, slug, ticket_prefix)
           VALUES ($1, $2, $3, $4)
           RETURNING id, name, slug, ticket_prefix, ticket_sequence, base_currency, created_at"#,
        id,
        name,
        slug,
        ticket_prefix,
    )
    .fetch_one(executor)
    .await?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        base_currency: row.base_currency,
        created_at: row.created_at,
    })
}

pub async fn get_org<'e, E>(executor: E, id: Uuid) -> DbResult<Org>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"SELECT id, name, slug, ticket_prefix, ticket_sequence, base_currency, created_at
           FROM orgs WHERE id = $1"#,
        id
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        base_currency: row.base_currency,
        created_at: row.created_at,
    })
}

/// Atomically allocates the next ticket number for an org (row-locked
/// increment; safe under concurrency).
pub async fn next_ticket_number<'e, E>(executor: E, org_id: Uuid) -> DbResult<(String, i64)>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query!(
        r#"UPDATE orgs SET ticket_sequence = ticket_sequence + 1
           WHERE id = $1
           RETURNING ticket_prefix, ticket_sequence"#,
        org_id
    )
    .fetch_one(executor)
    .await?;
    Ok((row.ticket_prefix, row.ticket_sequence))
}

/// Updates the editable profile fields (General settings / first-run
/// wizard). `None` leaves a field untouched; the slug is immutable.
pub async fn update_profile(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    name: Option<&str>,
    ticket_prefix: Option<&str>,
) -> DbResult<Org> {
    let row = sqlx::query!(
        r#"UPDATE orgs SET
             name = COALESCE($2, name),
             ticket_prefix = COALESCE($3, ticket_prefix)
           WHERE id = $1
           RETURNING id, name, slug, ticket_prefix, ticket_sequence, base_currency, created_at"#,
        org_id,
        name,
        ticket_prefix,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        base_currency: row.base_currency,
        created_at: row.created_at,
    })
}

/// Counts conversion snapshots across the five lock moments (ADR 0002)
/// for one org. Used by the base-currency guard and the lane's tests.
pub async fn count_fx_snapshots<'e, E>(executor: E, org_id: Uuid) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n: Option<i64> = sqlx::query_scalar!(
        r#"SELECT (
              (SELECT count(*) FROM ticket_expenses  WHERE org_id = $1 AND fx_rate_used IS NOT NULL) +
              (SELECT count(*) FROM ticket_cost_lines WHERE org_id = $1 AND fx_rate_used IS NOT NULL) +
              (SELECT count(*) FROM invoices          WHERE org_id = $1 AND fx_rate_used IS NOT NULL) +
              (SELECT count(*) FROM invoice_lines     WHERE org_id = $1 AND fx_rate_used IS NOT NULL) +
              (SELECT count(*) FROM invoice_payments  WHERE org_id = $1 AND fx_rate_used IS NOT NULL) +
              (SELECT count(*) FROM device_assets     WHERE org_id = $1 AND purchase_fx_rate_used IS NOT NULL)
            )"#,
        org_id
    )
    .fetch_one(executor)
    .await?;
    Ok(n.unwrap_or(0))
}

/// Money rows denominated in the CURRENT base (no snapshot — a
/// single-currency fact in the base needs none). A base change re-labels
/// these: every existing document's currency becomes a NON-base currency
/// with no snapshot and no re-snapshot path (the issue-side lock moment
/// has passed), so the guard must count them too — otherwise an org that
/// traded purely in its old base can flip silently and strand its
/// history half-snapshotted once payments start locking.
pub async fn count_base_currency_money<'e, E>(executor: E, org_id: Uuid) -> DbResult<i64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let n: Option<i64> = sqlx::query_scalar!(
        r#"SELECT (
              (SELECT count(*) FROM ticket_expenses  WHERE org_id = $1 AND currency = (SELECT base_currency FROM orgs WHERE id = $1)) +
              (SELECT count(*) FROM ticket_cost_lines WHERE org_id = $1 AND currency = (SELECT base_currency FROM orgs WHERE id = $1)) +
              (SELECT count(*) FROM invoices          WHERE org_id = $1 AND currency = (SELECT base_currency FROM orgs WHERE id = $1) AND fx_rate_used IS NULL) +
              (SELECT count(*) FROM invoice_payments  WHERE org_id = $1 AND currency = (SELECT base_currency FROM orgs WHERE id = $1) AND fx_rate_used IS NULL) +
              (SELECT count(*) FROM credit_notes      WHERE org_id = $1 AND currency = (SELECT base_currency FROM orgs WHERE id = $1))
            )"#,
        org_id
    )
    .fetch_one(executor)
    .await?;
    Ok(n.unwrap_or(0))
}

/// Changes the org base currency (ADR 0002, FX1). Governed: the write is
/// REFUSED while any conversion snapshot exists — snapshots are locked to
/// the base they were taken against, so a base change requires a full
/// re-snapshot (a governed, operator-driven act outside the write path;
/// the core never revaluates in place). The guard counts rows carrying
/// any snapshot column across the five lock moments, AND rows denominated
/// in the current base without a snapshot — those would be re-labeled
/// into a non-base currency they never agreed to. Only a truly
/// money-empty org (or one that has completed the documented governed
/// re-snapshot) may change its base.
pub async fn change_base_currency(
    conn: &mut PgConnection,
    org_id: Uuid,
    base_currency: &str,
) -> DbResult<Org> {
    let snapshots = count_fx_snapshots(&mut *conn, org_id).await?;
    if snapshots > 0 {
        return Err(DbError::Conflict(format!(
            "org has {snapshots} fx conversion snapshot(s); changing the base currency \
             requires the documented full re-snapshot path"
        )));
    }
    let base_money = count_base_currency_money(&mut *conn, org_id).await?;
    if base_money > 0 {
        return Err(DbError::Conflict(format!(
            "org has {base_money} document(s) denominated in the current base currency \
             with no conversion snapshot; changing the base currency would strand them \
             without a base equivalent — complete the documented re-snapshot path first"
        )));
    }
    let row = sqlx::query!(
        r#"UPDATE orgs SET base_currency = $2
           WHERE id = $1
           RETURNING id, name, slug, ticket_prefix, ticket_sequence, base_currency, created_at"#,
        org_id,
        base_currency,
    )
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(Org {
        id: row.id,
        name: row.name,
        slug: row.slug,
        ticket_prefix: row.ticket_prefix,
        ticket_sequence: row.ticket_sequence,
        base_currency: row.base_currency,
        created_at: row.created_at,
    })
}

pub fn permissions_for_builtin(role: BuiltinRole) -> serde_json::Value {
    serde_json::to_value(role.permissions()).unwrap_or_else(|_| serde_json::json!([]))
}

/// The default role set shipped with every org: (name, builtin, grants, system).
/// RB1 (ADR 0004): the full eight-preset roster — every org carries rows
/// for all shipped presets on the same matrix as custom roles; orgs fork
/// them, the canonical rows stay restorable.
pub fn default_roles() -> Vec<(String, Option<BuiltinRole>, serde_json::Value, bool)> {
    [
        (BuiltinRole::Owner, "Owner"),
        (BuiltinRole::Admin, "Admin"),
        (BuiltinRole::Dispatcher, "Dispatcher"),
        (BuiltinRole::SeniorTech, "Senior Tech"),
        (BuiltinRole::Technician, "Technician"),
        (BuiltinRole::Bookkeeper, "Bookkeeper"),
        (BuiltinRole::Auditor, "Auditor"),
        (BuiltinRole::ReadOnly, "Read Only"),
    ]
    .into_iter()
    .map(|(b, name)| (name.to_string(), Some(b), permissions_for_builtin(b), true))
    .collect()
}

/// Seeds default roles for an org; returns role id keyed by builtin name.
/// Multi-statement: takes a connection (callers inside transactions pass
/// `&mut *tx`, which deref-coerces).
pub async fn seed_default_roles(
    conn: &mut PgConnection,
    org_id: Uuid,
) -> DbResult<HashMap<String, Uuid>> {
    let mut map = HashMap::new();
    for (name, builtin, permissions, is_system) in default_roles() {
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"INSERT INTO roles (id, org_id, name, builtin, permissions, is_system)
               VALUES ($1, $2, $3, $4, $5, $6)"#,
            id,
            org_id,
            name,
            builtin.map(|b| b.as_str().to_string()),
            permissions,
            is_system,
        )
        .execute(&mut *conn)
        .await?;
        if let Some(b) = builtin {
            map.insert(b.as_str().to_string(), id);
        }
    }
    Ok(map)
}

/// Seeds the default board with the four semantic statuses.
pub async fn seed_default_board(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Uuid> {
    let board_id = Uuid::now_v7();
    sqlx::query!(
        r#"INSERT INTO boards (id, org_id, name, description, is_default)
           VALUES ($1, $2, 'Service Desk', 'Default service board', true)"#,
        board_id,
        org_id
    )
    .execute(&mut *conn)
    .await?;
    let defaults: [(&str, TicketStatusCategory, i32, bool); 4] = [
        ("New", TicketStatusCategory::Open, 0, true),
        ("In Progress", TicketStatusCategory::InProgress, 10, false),
        ("Waiting", TicketStatusCategory::Waiting, 20, false),
        ("Resolved", TicketStatusCategory::Closed, 30, false),
    ];
    for (name, category, sort, is_entry) in defaults {
        sqlx::query!(
            r#"INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            Uuid::now_v7(),
            board_id,
            org_id,
            name,
            category.as_str(),
            sort,
            is_entry,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(board_id)
}

/// Seeds the default SLA policy (industry-standard PSA targets, 9-5 M-F UTC
/// calendar) — an org can run SLAs with zero configuration.
pub async fn seed_default_sla_policy<'e, E>(executor: E, org_id: Uuid) -> DbResult<Uuid>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let id = Uuid::now_v7();
    let targets = serde_json::json!({
        "p1": { "first_response_mins": 15, "resolution_mins": 480 },
        "p2": { "first_response_mins": 60, "resolution_mins": 1440 },
        "p3": { "first_response_mins": 240, "resolution_mins": 4320 },
        "p4": { "first_response_mins": 480, "resolution_mins": 10080 }
    });
    let calendar = serde_json::json!({
        "name": "9to5 M-F",
        "timezone": "UTC",
        "windows": [
            ["09:00","17:00"],["09:00","17:00"],["09:00","17:00"],
            ["09:00","17:00"],["09:00","17:00"], null, null
        ],
        "holidays": []
    });
    sqlx::query!(
        r#"INSERT INTO sla_policies (id, org_id, name, is_default, targets, calendar)
           VALUES ($1, $2, 'Standard', true, $3, $4)"#,
        id,
        org_id,
        targets,
        calendar,
    )
    .execute(executor)
    .await?;
    Ok(id)
}

/// Seeds the standard accepted type/category/subcategory taxonomy (Wave 11
/// lane 3) — the identical set migration 0048 backfilled for existing orgs,
/// so every org starts with the client-agnostic IT/MSP baseline. The
/// canonical data lives in `taxonomy::STANDARD_TAXONOMY` (single Rust
/// source of truth; the migration carries its own SQL copy by necessity).
/// Seeds version 1 of the customer sign-off consent (Wave 43 track 2 lane 12,
/// migration 0127).
///
/// The migration backfills orgs that already exist; this covers every org
/// created afterwards, because an org with no consent row can never capture a
/// signature and would silently ship a disabled pad. Idempotent per org: the
/// UNIQUE (org_id, version) makes a double-seed a no-op-shaped conflict the
/// caller can ignore.
pub const DEFAULT_SIGN_OFF_CONSENT: &str =
    "I confirm the work described above was completed to my satisfaction and I am authorised to sign off on it.";

pub async fn seed_default_sign_off_consent(conn: &mut PgConnection, org_id: Uuid) -> DbResult<()> {
    // `sqlx::query`, not `sqlx::query!`: the compile-time macro would need a
    // fresh `cargo sqlx prepare` for every consent-shape change, and the
    // offline cache in a release boot is already stale for anything new.
    sqlx::query(
        r#"INSERT INTO sign_off_consents (id, org_id, version, body, created_by)
           VALUES ($1, $2, 1, $3, NULL)
           ON CONFLICT (org_id, version) DO NOTHING"#,
    )
    .bind(Uuid::now_v7())
    .bind(org_id)
    .bind(DEFAULT_SIGN_OFF_CONSENT)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn seed_default_taxonomy(conn: &mut PgConnection, org_id: Uuid) -> DbResult<()> {
    for (name, ord) in crate::taxonomy::STANDARD_TYPES {
        sqlx::query!(
            r#"INSERT INTO ticket_types (id, org_id, name, sort_order)
               VALUES ($1, $2, $3, $4)"#,
            Uuid::now_v7(),
            org_id,
            name,
            ord,
        )
        .execute(&mut *conn)
        .await?;
    }
    // Roots first, then children join their root by name.
    for (name, ord) in crate::taxonomy::STANDARD_CATEGORY_ROOTS {
        sqlx::query!(
            r#"INSERT INTO ticket_categories (id, org_id, parent_id, name, sort_order)
               VALUES ($1, $2, NULL, $3, $4)"#,
            Uuid::now_v7(),
            org_id,
            name,
            ord,
        )
        .execute(&mut *conn)
        .await?;
    }
    for (root_name, name, ord) in crate::taxonomy::STANDARD_SUBCATEGORIES {
        sqlx::query!(
            r#"INSERT INTO ticket_categories (id, org_id, parent_id, name, sort_order)
               SELECT $1, $2, c.id, $3, $4
               FROM ticket_categories c
               WHERE c.org_id = $2 AND c.parent_id IS NULL AND c.name = $5"#,
            Uuid::now_v7(),
            org_id,
            name,
            ord,
            root_name,
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Wave 19 lane 5: the two auth policy switches (migration 0088). Both are
/// deny-by-default columns; an admin's explicit PATCH is the only path to
/// `true`. Returns the persisted row shapes via plain runners (no `query!`
/// offline-cache churn).
pub async fn set_auth_policy(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    magic_link_sign_in: Option<bool>,
    require_email_verification: Option<bool>,
) -> DbResult<()> {
    let (ml, ev) = (magic_link_sign_in, require_email_verification);
    if ml.is_none() && ev.is_none() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE orgs SET \
             magic_link_sign_in = COALESCE($2, magic_link_sign_in), \
             require_email_verification = COALESCE($3, require_email_verification) \
           WHERE id = $1",
    )
    .bind(org_id)
    .bind(ml)
    .bind(ev)
    .execute(executor)
    .await?;
    Ok(())
}
/// Wave 19 lane 5: reads `require_email_verification` (deny-by-default
/// column from 0088) for the login gate.
pub async fn require_email_verification<'e, E>(executor: E, org_id: Uuid) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let flag: Option<bool> =
        sqlx::query_scalar("SELECT require_email_verification FROM orgs WHERE id = $1")
            .bind(org_id)
            .fetch_optional(executor)
            .await?
            .flatten();
    Ok(flag.unwrap_or(false))
}
