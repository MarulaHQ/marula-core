//! Routing-aware persistence (Wave 32 lane 7): per-technician journey
//! start points.
//!
//! Purpose: one dispatcher-editable start point per (user, day) — home,
//! office, a site, or an ad-hoc address — that travel estimates run from.
//! Absence is legal: the read path auto-suggests the previous day's last
//! booked site, and a day with neither resolves to no start point (a
//! typed warning, never a guess).
//!
//! Non-purpose: provider config, estimate computation (core-api), or
//! appointment provenance (columns on `appointments`).

use crate::{DbError, DbResult};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

/// One journey start point row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct JourneyStartPointRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub day: NaiveDate,
    /// `home` | `office` | `site` | `address`.
    pub kind: String,
    pub site_id: Option<Uuid>,
    pub address: Option<String>,
    pub label: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const JOURNEY_COLS: &str = "id, org_id, user_id, day, kind, site_id, address, label, latitude, \
     longitude, created_by, created_at, updated_at";

/// Upserts the dispatcher's start point for one technician-day. The
/// unique (org, user, day) key makes this a SET, not a log.
pub async fn upsert_journey_start_point(
    conn: &mut PgConnection,
    row: &JourneyStartPointRow,
) -> DbResult<JourneyStartPointRow> {
    let sql = format!(
        r#"INSERT INTO journey_start_points
             (id, org_id, user_id, day, kind, site_id, address, label, latitude, longitude,
              created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           ON CONFLICT (org_id, user_id, day) DO UPDATE SET
             kind = EXCLUDED.kind,
             site_id = EXCLUDED.site_id,
             address = EXCLUDED.address,
             label = EXCLUDED.label,
             latitude = EXCLUDED.latitude,
             longitude = EXCLUDED.longitude,
             created_by = EXCLUDED.created_by,
             updated_at = now()
           RETURNING {JOURNEY_COLS}"#
    );
    let row = sqlx::query_as::<_, JourneyStartPointRow>(&sql)
        .bind(row.id)
        .bind(row.org_id)
        .bind(row.user_id)
        .bind(row.day)
        .bind(&row.kind)
        .bind(row.site_id)
        .bind(&row.address)
        .bind(&row.label)
        .bind(row.latitude)
        .bind(row.longitude)
        .bind(row.created_by)
        .fetch_one(conn)
        .await
        .map_err(DbError::Sql)?;
    Ok(row)
}

/// The explicit start point for one technician-day, when set.
pub async fn get_journey_start_point(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
) -> DbResult<Option<JourneyStartPointRow>> {
    let sql = format!(
        "SELECT {JOURNEY_COLS} FROM journey_start_points \
         WHERE org_id = $1 AND user_id = $2 AND day = $3"
    );
    sqlx::query_as::<_, JourneyStartPointRow>(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(day)
        .fetch_optional(executor)
        .await
        .map_err(DbError::Sql)
}

/// Start points across a day range (the suggestion fallback walks
/// backwards day by day; bounded by the caller).
pub async fn list_journey_start_points_between(
    executor: impl sqlx::Executor<'_, Database = Postgres>,
    org_id: Uuid,
    user_id: Uuid,
    from_day: NaiveDate,
    to_day: NaiveDate,
) -> DbResult<Vec<JourneyStartPointRow>> {
    let sql = format!(
        "SELECT {JOURNEY_COLS} FROM journey_start_points \
         WHERE org_id = $1 AND user_id = $2 AND day >= $3 AND day <= $4 \
         ORDER BY day"
    );
    sqlx::query_as::<_, JourneyStartPointRow>(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(from_day)
        .bind(to_day)
        .fetch_all(executor)
        .await
        .map_err(DbError::Sql)
}
