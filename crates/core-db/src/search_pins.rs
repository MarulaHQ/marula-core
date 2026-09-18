//! Search pins (Wave 17 lane 13, migration 0089): operator-curated
//! persistent results for /search. Recent chips stay local-first — pins
//! are the server side of the SAME surface: per-user, per-org, one row
//! per pinned result (kind + id + title + the query snapshot that
//! produced it).
//!
//! Purpose: upsert/list/delete of pins. A pin re-runs its query and
//! re-resolves from the metric itself — the stored title is the snapshot
//! taken at pin time (an honest render), never a live fake. Uniqueness:
//! one pinned result per (user, kind, id); the API upserts so re-pinning
//! is idempotent, and `uq_search_pins_result`'s violation maps to a
//! typed conflict for the genuine concurrent race.
//!
//! Non-purpose: no org-wide pin sharing (operator-curated = per user),
//! no events (a pin is a bookmark, not a state change worth the wire).

use crate::DbResult;
use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchPinRow {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub query: String,
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct NewSearchPin<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub user_id: Uuid,
    pub query: &'a str,
    pub entity_kind: &'a str,
    pub entity_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: &'a str,
}

/// Upserts one pin by its (user, kind, id) identity. `Ok(None)` = the row
/// already existed (idempotent re-pin) — the caller answers with the
/// stored pin instead.
pub async fn upsert(
    conn: &mut PgConnection,
    p: NewSearchPin<'_>,
) -> DbResult<Option<SearchPinRow>> {
    let row = sqlx::query_as::<_, SearchPinRow>(
        "INSERT INTO search_pins (id, org_id, user_id, query, entity_kind, entity_id, parent_id, title)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (org_id, user_id, entity_kind, entity_id) DO NOTHING
         RETURNING id, org_id, user_id, query, entity_kind, entity_id, parent_id, title, created_at",
    )
    .bind(p.id)
    .bind(p.org_id)
    .bind(p.user_id)
    .bind(p.query)
    .bind(p.entity_kind)
    .bind(p.entity_id)
    .bind(p.parent_id)
    .bind(p.title)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// Fetches one pin (org + owner scoped) — the 404/conflict resolution
/// before delete.
pub async fn get(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    pin_id: Uuid,
) -> DbResult<SearchPinRow> {
    sqlx::query_as::<_, SearchPinRow>(
        "SELECT id, org_id, user_id, query, entity_kind, entity_id, parent_id, title, created_at \
         FROM search_pins WHERE org_id = $1 AND user_id = $2 AND id = $3",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(pin_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(crate::DbError::NotFound)
}

/// Newest-first pin list for one user.
pub async fn list_for_user(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<SearchPinRow>> {
    let rows = sqlx::query_as::<_, SearchPinRow>(
        "SELECT id, org_id, user_id, query, entity_kind, entity_id, parent_id, title, created_at \
         FROM search_pins WHERE org_id = $1 AND user_id = $2 \
         ORDER BY created_at DESC, id DESC \
         LIMIT LEAST(GREATEST($3, 1), 200) OFFSET GREATEST($4, 0)",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Fetches one pin by (org, owner, kind, id) — the re-pin identity
/// resolution after an idempotent no-op upsert.
pub async fn find_by_identity(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> DbResult<Option<SearchPinRow>> {
    let row = sqlx::query_as::<_, SearchPinRow>(
        "SELECT id, org_id, user_id, query, entity_kind, entity_id, parent_id, title, created_at \
         FROM search_pins \
         WHERE org_id = $1 AND user_id = $2 AND entity_kind = $3 AND entity_id = $4",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(entity_kind)
    .bind(entity_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// Deletes one SPECIFIC user's pin (a list may only unpin what it owns);
/// returns the deleted row (`None` = already gone / invisible).
pub async fn delete(
    conn: &mut PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    pin_id: Uuid,
) -> DbResult<Option<SearchPinRow>> {
    let row = sqlx::query_as::<_, SearchPinRow>(
        "DELETE FROM search_pins WHERE org_id = $1 AND user_id = $2 AND id = $3 \
         RETURNING id, org_id, user_id, query, entity_kind, entity_id, parent_id, title, created_at",
    )
    .bind(org_id)
    .bind(user_id)
    .bind(pin_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}
