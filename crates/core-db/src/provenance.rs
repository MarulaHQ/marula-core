//! Provenance read-model queries (the private build tracker IF9, doctrine #9).
//!
//! Reads the `provenance_events` projection (migration 0043) — one row per
//! changed field per applied write — so provenance questions ("who changed
//! this record/field", "what did this agent session write") are indexed
//! lookups, never raw-row scans over the write trail. The projection is
//! maintained by the write-path trigger; this module only reads it, with
//! the same org-scoped, keyset-paged conventions as the 2.3 audit query
//! API (`audit::list_filtered`).

use crate::DbResult;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Postgres;
use uuid::Uuid;

/// One projected provenance row: a single field change within one applied
/// write. Agent context columns are `None` for human writes.
#[derive(Debug, Clone)]
pub struct ProvenanceRow {
    pub write_id: Uuid,
    /// 1..n position of this field within its write (field-name order) —
    /// the keyset tie-break across a multi-field write.
    pub field_seq: i32,
    pub entity: String,
    pub entity_id: Uuid,
    pub seq: i64,
    pub op: String,
    pub actor_kind: String,
    pub actor_id: Option<Uuid>,
    pub field: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub session_id: Option<Uuid>,
    pub skill: Option<String>,
    pub ruleset_hash: Option<String>,
    pub is_undo: bool,
    pub idempotency_key: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

/// Filters for the provenance query API. All optional; unset = unfiltered.
/// `entity_id` and `field` are only meaningful together with `entity`
/// (validated at the API layer — they index under it).
#[derive(Debug, Default, Clone)]
pub struct ProvenanceFilter {
    /// Actor-kind filter from the write-trail vocabulary (human, agent,
    /// import, ...).
    pub actor_kind: Option<String>,
    /// The "who" filter: a user (or machine actor) uuid.
    pub actor_id: Option<Uuid>,
    pub entity: Option<String>,
    pub entity_id: Option<Uuid>,
    pub field: Option<String>,
    /// Agent provenance, when present (IF8-approved writes).
    pub session_id: Option<Uuid>,
    pub skill: Option<String>,
    pub ruleset_hash: Option<String>,
    /// Restrict to undo reverts (IF7) — or exclude them with `Some(false)`.
    pub is_undo: Option<bool>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    /// Keyset cursor: strictly older than (occurred_at, write_id); ties
    /// inside the same write continue at the NEXT field (ascending
    /// field_seq, matching the newest-first page order).
    pub cursor: Option<(DateTime<Utc>, Uuid, i32)>,
}

/// Provenance listing with keyset pagination on
/// (occurred_at DESC, write_id DESC, field_seq ASC).
pub async fn list_filtered<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: ProvenanceFilter,
    limit: i64,
) -> DbResult<Vec<ProvenanceRow>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT write_id, field_seq, entity, entity_id, seq, op, actor_kind, actor_id,
                  field, before, after, session_id, skill, ruleset_hash,
                  is_undo, idempotency_key, occurred_at
           FROM provenance_events
           WHERE org_id = $1
             AND ($2::text IS NULL OR actor_kind = $2)
             AND ($3::uuid IS NULL OR actor_id = $3)
             AND ($4::text IS NULL OR entity = $4)
             AND ($5::uuid IS NULL OR entity_id = $5)
             AND ($6::text IS NULL OR field = $6)
             AND ($7::uuid IS NULL OR session_id = $7)
             AND ($8::text IS NULL OR skill = $8)
             AND ($9::text IS NULL OR ruleset_hash = $9)
             AND ($10::boolean IS NULL OR is_undo = $10)
             AND ($11::timestamptz IS NULL OR occurred_at >= $11)
             AND ($12::timestamptz IS NULL OR occurred_at <= $12)
             AND ($13::timestamptz IS NULL OR $14::uuid IS NULL OR
                  (occurred_at, write_id) < ($13::timestamptz, $14::uuid)
                  OR ((occurred_at = $13::timestamptz AND write_id = $14::uuid)
                      AND field_seq > $15))
           ORDER BY occurred_at DESC, write_id DESC, field_seq ASC
           LIMIT $16"#,
    )
    .bind(org_id)
    .bind(filter.actor_kind.as_deref())
    .bind(filter.actor_id)
    .bind(filter.entity.as_deref())
    .bind(filter.entity_id)
    .bind(filter.field.as_deref())
    .bind(filter.session_id)
    .bind(filter.skill.as_deref())
    .bind(filter.ruleset_hash.as_deref())
    .bind(filter.is_undo)
    .bind(filter.from)
    .bind(filter.to)
    .bind(filter.cursor.map(|c| c.0))
    .bind(filter.cursor.map(|c| c.1))
    .bind(filter.cursor.map(|c| c.2))
    .bind(limit.clamp(1, 500))
    .fetch_all(executor)
    .await?;
    Ok(rows.into_iter().map(row_from_pg).collect())
}

fn row_from_pg(row: sqlx::postgres::PgRow) -> ProvenanceRow {
    use sqlx::Row;
    ProvenanceRow {
        write_id: row.get("write_id"),
        field_seq: row.get("field_seq"),
        entity: row.get("entity"),
        entity_id: row.get("entity_id"),
        seq: row.get("seq"),
        op: row.get("op"),
        actor_kind: row.get("actor_kind"),
        actor_id: row.get("actor_id"),
        field: row.get("field"),
        before: row.get("before"),
        after: row.get("after"),
        session_id: row.get("session_id"),
        skill: row.get("skill"),
        ruleset_hash: row.get("ruleset_hash"),
        is_undo: row.get("is_undo"),
        idempotency_key: row.get("idempotency_key"),
        occurred_at: row.get("occurred_at"),
    }
}
