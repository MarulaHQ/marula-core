//! Ecosystem-connector persistence (Wave 22): the VoIP call-log ingest
//! read model + the distributor pricing-feed batches/entries/outcomes.
//! The ConnectorInstallation CONSENT store lives in webhooks.rs.
//! Non-purpose: connector CONFIG encryption (routes own the sealing)
//! and adapter delivery (app-bridges crate).
use crate::{DbError, DbResult};
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

// --- voip call logs --------------------------------------------------------

#[derive(Debug, Clone)]
pub struct VoipCallLog {
    pub id: Uuid,
    pub org_id: Uuid,
    pub installation_id: Uuid,
    pub call_id: Option<String>,
    /// `in` | `out`.
    pub direction: String,
    pub external_number: String,
    pub duration_seconds: i32,
    pub summary: String,
    pub linked_ticket_id: Option<Uuid>,
    pub recorded_at: DateTime<Utc>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct VoipCallLogRow {
    id: Uuid,
    org_id: Uuid,
    installation_id: Uuid,
    call_id: Option<String>,
    direction: String,
    external_number: String,
    duration_seconds: i32,
    summary: String,
    linked_ticket_id: Option<Uuid>,
    recorded_at: DateTime<Utc>,
    occurred_at: DateTime<Utc>,
}

fn row_to_call_log(row: VoipCallLogRow) -> VoipCallLog {
    VoipCallLog {
        id: row.id,
        org_id: row.org_id,
        installation_id: row.installation_id,
        call_id: row.call_id,
        direction: row.direction,
        external_number: row.external_number,
        duration_seconds: row.duration_seconds,
        summary: row.summary,
        linked_ticket_id: row.linked_ticket_id,
        recorded_at: row.recorded_at,
        occurred_at: row.occurred_at,
    }
}

pub async fn insert_voip_call_log(tx: &mut PgConnection, model: &VoipCallLog) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO voip_call_logs
             (id, org_id, installation_id, call_id, direction, external_number, duration_seconds,
              summary, linked_ticket_id, recorded_at, occurred_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(model.id)
    .bind(model.org_id)
    .bind(model.installation_id)
    .bind(&model.call_id)
    .bind(&model.direction)
    .bind(model.external_number.trim())
    .bind(model.duration_seconds)
    .bind(&model.summary)
    .bind(model.linked_ticket_id)
    .bind(model.recorded_at)
    .bind(model.occurred_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn list_voip_call_logs<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Option<Uuid>,
) -> DbResult<Vec<VoipCallLog>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as::<_, VoipCallLogRow>(
        r#"SELECT id, org_id, installation_id, call_id, direction, external_number,
                  duration_seconds, summary, linked_ticket_id, recorded_at, occurred_at
           FROM voip_call_logs
           WHERE org_id = $1 AND ($2::uuid IS NULL OR linked_ticket_id = $2)
           ORDER BY occurred_at DESC LIMIT 200"#,
    )
    .bind(org_id)
    .bind(ticket_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row_to_call_log).collect())
}

/// The consented installation for a typed connector kind; NotFound means
/// the org has not consented to this connector yet.
pub async fn get_connector_installation(
    executor: &mut PgConnection,
    org_id: Uuid,
    connector_type: &str,
) -> DbResult<(Uuid, Vec<u8>)> {
    let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        r#"SELECT id, config_encrypted FROM connector_installations
           WHERE org_id = $1 AND connector_type = $2
           ORDER BY updated_at DESC LIMIT 1"#,
    )
    .bind(org_id)
    .bind(connector_type)
    .fetch_optional(&mut *executor)
    .await?;
    row.ok_or(DbError::NotFound)
}

// --- pricing feeds ---------------------------------------------------------

pub async fn create_pricing_feed_batch(
    tx: &mut PgConnection,
    id: Uuid,
    org_id: Uuid,
    source: &str,
    note: &str,
    uploaded_by: Option<Uuid>,
    now: DateTime<Utc>,
) -> DbResult<()> {
    sqlx::query(
        r#"INSERT INTO pricing_feed_batches (id, org_id, source, note, uploaded_by, uploaded_at)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(source.trim())
    .bind(note)
    .bind(uploaded_by)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct PricingFeedEntry {
    pub sku: Option<String>,
    pub currency: String,
    pub unit_price_minor: i64,
    pub cost_minor: i64,
}

pub async fn insert_pricing_feed_entries(
    tx: &mut PgConnection,
    batch_id: Uuid,
    org_id: Uuid,
    entries: &[PricingFeedEntry],
) -> DbResult<()> {
    for e in entries {
        sqlx::query(
            r#"INSERT INTO pricing_feed_entries (id, batch_id, org_id, sku, currency, unit_price_minor, cost_minor)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(Uuid::now_v7())
        .bind(batch_id)
        .bind(org_id)
        .bind(e.sku.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(&e.currency)
        .bind(e.unit_price_minor)
        .bind(e.cost_minor)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// One applied-feed overlay: sets the catalog row's cost + unit price by
/// SKU (lower-case match), stamps the batch applied, and records the
/// matched/unmatched outcome — the typed receipt the ops UI reads.
pub async fn apply_pricing_feed_batch(
    tx: &mut PgConnection,
    org_id: Uuid,
    batch_id: Uuid,
    applied_by: Option<Uuid>,
    now: DateTime<Utc>,
) -> DbResult<i64> {
    let entries: Vec<(String, String, i64, i64)> = sqlx::query_as(
        r#"SELECT sku, currency, unit_price_minor, cost_minor
           FROM pricing_feed_entries
           WHERE org_id = $1 AND batch_id = $2 ORDER BY sku"#,
    )
    .bind(org_id)
    .bind(batch_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut matched_count = 0i64;
    for (sku, currency, unit_minor, cost_minor) in entries.iter() {
        let updated = sqlx::query_scalar::<_, Uuid>(
            r#"UPDATE catalog_items
               SET cost_minor = $4, currency = $5, unit_price_minor = $6, updated_at = $7
               WHERE org_id = $1 AND active AND sku IS NOT NULL AND lower(sku) = $2
               RETURNING id"#,
        )
        .bind(org_id)
        .bind(sku.to_lowercase())
        .bind(currency)
        .bind(*cost_minor)
        .bind(currency)
        .bind(*unit_minor)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        matched_count += i64::from(updated.is_some());
        sqlx::query(
            r#"INSERT INTO pricing_feed_outcomes (id, batch_id, org_id, sku, matched_catalog_item_id, matched, reason)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(Uuid::now_v7())
        .bind(batch_id)
        .bind(org_id)
        .bind(sku)
        .bind(updated)
        .bind(updated.is_some())
        .bind(if updated.is_some() { "" } else { "no active catalog item for this SKU" })
        .execute(&mut *tx)
        .await?;
    }
    // Stamp the batch applied AFTER the overlay rolls forward (same
    // transaction — a crash rolls the whole overlay back together). The
    // `applied = false` predicate is the typed double-apply refusal: a
    // batch applies exactly once — never silently overwrites twice.
    let stamped = sqlx::query(
        r#"UPDATE pricing_feed_batches
           SET applied = true, applied_at = $3, applied_by = $4
           WHERE org_id = $1 AND id = $2 AND applied = false"#,
    )
    .bind(org_id)
    .bind(batch_id)
    .bind(now)
    .bind(applied_by)
    .execute(&mut *tx)
    .await?;
    if stamped.rows_affected() == 0 {
        return Err(DbError::Conflict("the batch already applied".into()));
    }
    Ok(matched_count)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PricingFeedOutcome {
    pub sku: Option<String>,
    pub matched_catalog_item_id: Option<Uuid>,
    pub matched: bool,
    pub reason: String,
}

impl PricingFeedOutcome {
    pub fn from_row(row: PricingFeedOutcomeRow) -> Self {
        Self {
            sku: row.sku,
            matched_catalog_item_id: row.matched_catalog_item_id,
            matched: row.matched,
            reason: row.reason,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct PricingFeedOutcomeRow {
    sku: Option<String>,
    matched_catalog_item_id: Option<Uuid>,
    matched: bool,
    reason: String,
}

pub async fn list_pricing_feed_outcomes<'e, E>(
    executor: E,
    org_id: Uuid,
    batch_id: Uuid,
) -> DbResult<Vec<PricingFeedOutcome>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows = sqlx::query_as::<_, PricingFeedOutcomeRow>(
        r#"SELECT sku, matched_catalog_item_id, matched, reason
           FROM pricing_feed_outcomes
           WHERE org_id = $1 AND batch_id = $2
           ORDER BY ( CASE WHEN matched THEN 0 ELSE 1 END ), sku"#,
    )
    .bind(org_id)
    .bind(batch_id)
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(PricingFeedOutcome::from_row).collect())
}
