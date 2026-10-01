//! Org time-category taxonomy (Wave 39 lane 1, migration 0110).
//!
//! The semantic catalog that `time_entries.work_type` keys onto: admin
//! CRUD + reorder, capture-surface reads (pickers ordered by `position`),
//! and the kept one-time backfill mapping (`legacy_value → slug`).
//!
//! Reads are RLS-scoped through the caller's `app.org_id` GUC. Slug
//! uniqueness is org-scoped by the migration's unique index.

use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::DbResult;
use core_domain::entities::TimeCategory;

pub struct NewTimeCategory<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub slug: &'a str,
    pub label: &'a str,
    pub description: Option<&'a str>,
    pub default_billable: bool,
    pub requires_unbillable_reason: bool,
    pub rate_card_affinity: bool,
    pub position: i32,
    pub created_by: Option<Uuid>,
}

fn category(id: Uuid, org_id: Uuid, row: &sqlx::postgres::PgRow) -> TimeCategory {
    use sqlx::Row;
    // (Row trait already imported here; helper sites need their own use.)
    TimeCategory {
        id,
        org_id,
        slug: row.get("slug"),
        label: row.get("label"),
        description: row.get("description"),
        default_billable: row.get("default_billable"),
        requires_unbillable_reason: row.get("requires_unbillable_reason"),
        rate_card_affinity: row.get("rate_card_affinity"),
        position: row.get("position"),
        retired: row.get("retired"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

const CATEGORY_COLUMNS: &str = "id, org_id, slug, label, description, default_billable, \
     requires_unbillable_reason, rate_card_affinity, position, retired, created_by, \
     created_at, updated_at";

/// The org's categories, ordered for capture pickers. `include_retired`
/// serves the settings admin (which manages retirement); capture
/// surfaces read without it.
pub async fn list_categories<'e, E>(
    executor: E,
    org_id: Uuid,
    include_retired: bool,
) -> DbResult<Vec<TimeCategory>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {CATEGORY_COLUMNS}
             FROM time_categories
            WHERE org_id = $1 AND ($2::boolean OR NOT retired)
            ORDER BY position, slug"#
    );
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(include_retired)
        .fetch_all(executor)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        use sqlx::Row;
        let id: Uuid = row.try_get("id")?;
        out.push(category(id, org_id, row));
    }
    Ok(out)
}

pub async fn get_category<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<TimeCategory>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {CATEGORY_COLUMNS}
             FROM time_categories WHERE org_id = $1 AND id = $2"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(id)
        .fetch_one(executor)
        .await?;
    Ok(category(id, org_id, &row))
}

pub async fn get_category_by_slug<'e, E>(
    executor: E,
    org_id: Uuid,
    slug: &str,
) -> DbResult<Option<TimeCategory>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let sql = format!(
        r#"SELECT {CATEGORY_COLUMNS}
             FROM time_categories WHERE org_id = $1 AND slug = $2"#
    );
    let row = sqlx::query(&sql)
        .bind(org_id)
        .bind(slug)
        .fetch_optional(executor)
        .await?;
    match row {
        Some(row) => {
            use sqlx::Row;
            let id: Uuid = row.try_get("id")?;
            Ok(Some(category(id, org_id, &row)))
        }
        None => Ok(None),
    }
}

pub async fn insert_category<'e, E>(executor: E, c: &NewTimeCategory<'_>) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO time_categories
             (id, org_id, slug, label, description, default_billable,
              requires_unbillable_reason, rate_card_affinity, position, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(c.id)
    .bind(c.org_id)
    .bind(c.slug)
    .bind(c.label)
    .bind(c.description)
    .bind(c.default_billable)
    .bind(c.requires_unbillable_reason)
    .bind(c.rate_card_affinity)
    .bind(c.position)
    .bind(c.created_by)
    .execute(executor)
    .await?;
    Ok(())
}

/// Content update: all fields are absolute (route resolves absent
/// fields to current values before calling). Slug is NOT editable —
/// entries and rate cards key onto it.
#[allow(clippy::too_many_arguments)]
pub async fn update_category<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    label: &str,
    description: Option<&str>,
    default_billable: bool,
    requires_unbillable_reason: bool,
    rate_card_affinity: bool,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"UPDATE time_categories
              SET label = $3, description = $4, default_billable = $5,
                  requires_unbillable_reason = $6, rate_card_affinity = $7,
                  updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(label)
    .bind(description)
    .bind(default_billable)
    .bind(requires_unbillable_reason)
    .bind(rate_card_affinity)
    .execute(executor)
    .await?;
    Ok(())
}

/// Soft retire / restore. Entries and rate-card rows keep the slug.
pub async fn set_category_retired<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
    retired: bool,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"UPDATE time_categories
              SET retired = $3, updated_at = now()
            WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .bind(retired)
    .execute(executor)
    .await?;
    Ok(())
}

/// Whole-list reorder: positions are rewritten verbatim from the caller's
/// ordered id list (settings drag-order). Unknown ids are ignored; known
/// ids absent from the list keep their old position.
pub async fn reorder_categories(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    ordered_ids: &[Uuid],
) -> DbResult<()> {
    // One statement per row; taxonomy scale is tiny (dozens of rows).
    for (idx, id) in ordered_ids.iter().enumerate() {
        sqlx::query(
            r#"UPDATE time_categories
                  SET position = $3, updated_at = now()
                WHERE org_id = $1 AND id = $2"#,
        )
        .bind(org_id)
        .bind(id)
        .bind((idx * 10) as i32)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// How many entries carry each slug — the settings admin's usage column.
pub async fn entry_counts_by_slug<'e, E>(
    executor: E,
    org_id: Uuid,
) -> DbResult<std::collections::HashMap<String, i64>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT work_type, COUNT(*) AS n
             FROM time_entries
            WHERE org_id = $1 AND work_type IS NOT NULL
            GROUP BY work_type"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.work_type.unwrap_or_default(), r.n.unwrap_or(0)))
        .collect())
}

// ---- org provisioning: the default taxonomy every org gets ----

/// The ten standard categories (`slug`, `label`, `description`,
/// `default_billable`, `requires_unbillable_reason`,
/// `rate_card_affinity`, `position`). The migration seeds these for
/// orgs that exist at migration time; this function seeds them for
/// orgs created afterward (demo seeder, sign-up provisioning) — same
/// vocabulary, idempotent via the (org_id, slug) unique index.
pub const STANDARD_TIME_CATEGORIES: &[(&str, &str, &str, bool, bool, bool, i32)] = &[
    (
        "remote",
        "Remote",
        "Work done remotely (phone, email, remote tools)",
        true,
        false,
        true,
        0,
    ),
    (
        "on_site",
        "On-site",
        "Work at the client location",
        true,
        false,
        true,
        10,
    ),
    (
        "depot",
        "Depot",
        "Bench / workshop repair work",
        true,
        false,
        true,
        20,
    ),
    (
        "travel",
        "Travel",
        "Travel time to or from client sites",
        true,
        false,
        false,
        30,
    ),
    (
        "incident",
        "Incident",
        "Time against incident response or emergencies",
        true,
        false,
        true,
        40,
    ),
    (
        "project",
        "Project",
        "Time against project phases rather than tickets",
        true,
        false,
        true,
        50,
    ),
    (
        "internal",
        "Internal",
        "Internal admin, meetings, and own-system work",
        false,
        false,
        false,
        60,
    ),
    (
        "warranty",
        "Warranty",
        "Work covered by warranty — not billable to the client",
        false,
        true,
        false,
        70,
    ),
    (
        "training",
        "Training",
        "Training and enablement time",
        false,
        false,
        false,
        80,
    ),
    (
        "admin",
        "Admin",
        "Administrative time not attributable to client work",
        false,
        false,
        false,
        90,
    ),
];

/// Legacy free-text values the one-time backfill normalizes, seeded per
/// org so capture keeps accepting what orgs actually send (`onsite` from
/// the dispatch path, `on-site` from older sheets, `Project work` from
/// the old wrap-up menu) after the taxonomy ships. Same alias list the
/// migration uses, kept live in the mapping table per org.
pub const STANDARD_WORK_TYPE_ALIASES: &[(&str, &str)] = &[
    ("onsite", "on_site"),
    ("on-site", "on_site"),
    ("on site", "on_site"),
    ("on_site", "on_site"),
    ("remote", "remote"),
    ("remote support", "remote"),
    ("depot", "depot"),
    ("travel", "travel"),
    ("project work", "project"),
    ("project", "project"),
    ("consulting", "project"),
    ("internal", "internal"),
    ("training", "training"),
    ("warranty", "warranty"),
    ("admin", "admin"),
    ("incident", "incident"),
];

/// Seeds [`STANDARD_TIME_CATEGORIES`] plus their standard legacy aliases
/// for one org. Idempotent: existing (org_id, slug) rows are left
/// untouched, so admin edits survive.
pub async fn seed_default_time_categories(conn: &mut PgConnection, org_id: Uuid) -> DbResult<()> {
    for (slug, label, description, default_billable, requires_reason, affinity, position) in
        STANDARD_TIME_CATEGORIES
    {
        sqlx::query(
            r#"INSERT INTO time_categories
                 (id, org_id, slug, label, description, default_billable,
                  requires_unbillable_reason, rate_card_affinity, position, retired)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, false)
               ON CONFLICT (org_id, slug) DO NOTHING"#,
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(*slug)
        .bind(*label)
        .bind(*description)
        .bind(*default_billable)
        .bind(*requires_reason)
        .bind(*affinity)
        .bind(*position)
        .execute(&mut *conn)
        .await?;
    }
    // Aliases AFTER the categories: the composite FK requires the target
    // slug to exist in this org.
    for (legacy, slug) in STANDARD_WORK_TYPE_ALIASES {
        sqlx::query(
            r#"INSERT INTO time_category_work_type_map
                 (id, org_id, legacy_value, category_slug, source)
               VALUES ($1, $2, $3, $4, 'backfill')
               ON CONFLICT (org_id, legacy_value) DO NOTHING"#,
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(*legacy)
        .bind(*slug)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Resolves a submitted `work_type` onto a live category slug.
///
/// Order: exact slug first, then the org's kept mapping table (the
/// one-time backfill rows plus any admin alias). This is what keeps
/// legacy free text working — `onsite` normalizes onto `on_site` the
/// same way the migration normalized historical rows — while genuinely
/// unknown values still fail loudly instead of being guessed.
pub async fn resolve_work_type_slug(
    conn: &mut PgConnection,
    org_id: Uuid,
    raw: &str,
) -> DbResult<Option<String>> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let exact = sqlx::query_scalar!(
        r#"SELECT slug FROM time_categories
            WHERE org_id = $1 AND slug = $2 AND NOT retired"#,
        org_id,
        value.to_lowercase()
    )
    .fetch_optional(&mut *conn)
    .await?;
    if exact.is_some() {
        return Ok(exact);
    }
    let mapped = sqlx::query_scalar!(
        r#"SELECT category_slug FROM time_category_work_type_map
            WHERE org_id = $1 AND lower(btrim(legacy_value)) = lower(btrim($2))"#,
        org_id,
        value
    )
    .fetch_optional(&mut *conn)
    .await?;
    let Some(slug) = mapped else { return Ok(None) };
    // A mapping onto a retired category is not a live target anymore.
    let live = sqlx::query_scalar!(
        r#"SELECT slug FROM time_categories
            WHERE org_id = $1 AND slug = $2 AND NOT retired"#,
        org_id,
        &slug
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(live)
}

// ---- backfill mapping table (kept per lane scope B) ----

pub struct WorkTypeMapRow {
    pub legacy_value: String,
    pub category_slug: String,
    pub source: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn list_work_type_map<'e, E>(executor: E, org_id: Uuid) -> DbResult<Vec<WorkTypeMapRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query!(
        r#"SELECT legacy_value, category_slug, source, created_at
             FROM time_category_work_type_map
            WHERE org_id = $1
            ORDER BY legacy_value"#,
        org_id
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| WorkTypeMapRow {
            legacy_value: r.legacy_value,
            category_slug: r.category_slug,
            source: r.source,
            created_at: r.created_at,
        })
        .collect())
}

/// Manual alias (admin-provided): maps a legacy/free-text value onto a
/// slug for future reporting reads. `ON CONFLICT` re-points an existing
/// mapping (including a backfilled one) — the table is the live record.
pub async fn upsert_work_type_map<'e, E>(
    executor: E,
    org_id: Uuid,
    legacy_value: &str,
    category_slug: &str,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO time_category_work_type_map
             (id, org_id, legacy_value, category_slug, source)
           VALUES (gen_random_uuid(), $1, $2, $3, 'manual')
           ON CONFLICT (org_id, legacy_value)
           DO UPDATE SET category_slug = EXCLUDED.category_slug, source = 'manual'"#,
    )
    .bind(org_id)
    .bind(legacy_value)
    .bind(category_slug)
    .execute(executor)
    .await?;
    Ok(())
}

/// Recategorizes entries whose CURRENT work_type matches `from` onto
/// `to` (admin aliasing made retroactive, always explicit).
pub async fn apply_work_type_map<'e, E>(
    executor: E,
    org_id: Uuid,
    from: &str,
    to: &str,
) -> DbResult<u64>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        r#"UPDATE time_entries
              SET work_type = $3
            WHERE org_id = $1 AND lower(btrim(work_type)) = lower(btrim($2))
              AND work_type IS DISTINCT FROM $3"#,
    )
    .bind(org_id)
    .bind(from)
    .bind(to)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}
