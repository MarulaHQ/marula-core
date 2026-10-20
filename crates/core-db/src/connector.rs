//! Migration connector persistence (the private build tracker D3): the checkpoint substrate
//! for third-party bulk pushes plus the keyset page queries the bulk pull
//! exports walk.
//!
//! Purpose: `migration_push_sessions` / `migration_push_chunks` (migration
//! 0042) remember WHERE a multi-chunk push stopped — one session row per
//! connector run over exactly one D2 import batch, one committed-chunk row
//! carrying the content fingerprint and the full per-row report so an
//! exact re-send replays instead of re-executing. The pull side is
//! stateless: `page_entity_ids` walks one entity family's internal ids in
//! uuidv7 (time) order and `external_key_map` joins the stable
//! source-system keys back onto each page.
//!
//! Non-purpose: no chunk payloads (the D2 engine never stores row bytes),
//! no conflict-policy logic (core-domain owns decisions), no batch
//! lifecycle here (that is `imports` — the session pairs 1:1 with a batch
//! and defers to it).

use crate::DbResult;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Row};
use uuid::Uuid;

/// One connector push session over one D2 import batch.
#[derive(Debug, Clone)]
pub struct PushSession {
    pub id: Uuid,
    pub org_id: Uuid,
    pub batch_id: Uuid,
    pub label: String,
    pub source_note: Option<String>,
    /// Driving identity — NO foreign key by design (machine drivers are
    /// api-key ids; same provenance doctrine as import_batches, 0027).
    pub created_by: Uuid,
    /// "user" | "api_key".
    pub created_by_kind: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// One COMMITTED chunk: the checkpoint row a resumed push replays from.
#[derive(Debug, Clone)]
pub struct PushChunk {
    pub id: Uuid,
    pub org_id: Uuid,
    pub session_id: Uuid,
    pub chunk_index: i32,
    /// sha256 over the canonical decoded payload — the replay contract.
    pub fingerprint: String,
    /// The full per-row report of the committed chunk (JSON object).
    pub summary: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub async fn insert_push_session<'e, E>(executor: E, session: &PushSession) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO migration_push_sessions
             (id, org_id, batch_id, label, source_note, created_by, created_by_kind)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(session.id)
    .bind(session.org_id)
    .bind(session.batch_id)
    .bind(&session.label)
    .bind(&session.source_note)
    .bind(session.created_by)
    .bind(&session.created_by_kind)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_push_session<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<PushSession>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, batch_id, label, source_note, created_by, created_by_kind,
                  created_at, finished_at
           FROM migration_push_sessions WHERE org_id = $1 AND id = $2"#,
    )
    .bind(org_id)
    .bind(id)
    .fetch_one(executor)
    .await?;
    Ok(push_session_from(&row))
}

fn push_session_from(row: &sqlx::postgres::PgRow) -> PushSession {
    PushSession {
        id: row.get("id"),
        org_id: row.get("org_id"),
        batch_id: row.get("batch_id"),
        label: row.get("label"),
        source_note: row.get("source_note"),
        created_by: row.get("created_by"),
        created_by_kind: row.get("created_by_kind"),
        created_at: row.get("created_at"),
        finished_at: row.get("finished_at"),
    }
}

/// Closes the session span exactly once; `false` means it was already
/// finished (the caller surfaces the typed conflict).
pub async fn mark_push_session_finished<'e, E>(
    executor: E,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<bool>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "UPDATE migration_push_sessions SET finished_at = now() \
         WHERE org_id = $1 AND id = $2 AND finished_at IS NULL",
    )
    .bind(org_id)
    .bind(id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn insert_push_chunk<'e, E>(executor: E, chunk: &PushChunk) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"INSERT INTO migration_push_chunks (id, org_id, session_id, chunk_index, fingerprint, summary)
           VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(chunk.id)
    .bind(chunk.org_id)
    .bind(chunk.session_id)
    .bind(chunk.chunk_index)
    .bind(&chunk.fingerprint)
    .bind(&chunk.summary)
    .execute(executor)
    .await?;
    Ok(())
}

/// The stored checkpoint for one chunk index, if that chunk committed.
pub async fn get_push_chunk<'e, E>(
    executor: E,
    org_id: Uuid,
    session_id: Uuid,
    chunk_index: i32,
) -> DbResult<Option<PushChunk>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(
        r#"SELECT id, org_id, session_id, chunk_index, fingerprint, summary, created_at
           FROM migration_push_chunks WHERE org_id = $1 AND session_id = $2 AND chunk_index = $3"#,
    )
    .bind(org_id)
    .bind(session_id)
    .bind(chunk_index)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|row| PushChunk {
        id: row.get("id"),
        org_id: row.get("org_id"),
        session_id: row.get("session_id"),
        chunk_index: row.get("chunk_index"),
        fingerprint: row.get("fingerprint"),
        summary: row.get("summary"),
        created_at: row.get("created_at"),
    }))
}

/// Every committed chunk of a session in order — the resume view.
pub async fn list_push_chunks<'e, E>(
    executor: E,
    org_id: Uuid,
    session_id: Uuid,
) -> DbResult<Vec<PushChunk>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"SELECT id, org_id, session_id, chunk_index, fingerprint, summary, created_at
           FROM migration_push_chunks WHERE org_id = $1 AND session_id = $2
           ORDER BY chunk_index ASC"#,
    )
    .bind(org_id)
    .bind(session_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .iter()
        .map(|row| PushChunk {
            id: row.get("id"),
            org_id: row.get("org_id"),
            session_id: row.get("session_id"),
            chunk_index: row.get("chunk_index"),
            fingerprint: row.get("fingerprint"),
            summary: row.get("summary"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// The static family → table map for the pull exports. A whitelist, never
/// caller input interpolated unchecked.
fn table_for(kind: &str) -> Option<&'static str> {
    match kind {
        "client" => Some("clients"),
        "site" => Some("sites"),
        "contact" => Some("contacts"),
        "board" => Some("boards"),
        "ticket" => Some("tickets"),
        "time_entry" => Some("time_entries"),
        "kb_category" => Some("kb_categories"),
        "kb_article" => Some("kb_articles"),
        _ => None,
    }
}

/// Keyset page of internal ids for one entity family, ascending uuidv7
/// (creation-time) order. `after = None` starts from the beginning; the
/// sentinel of a nil uuid is safe because every internal id is a v7.
pub async fn page_entity_ids<'e, E>(
    executor: E,
    entity_kind: &str,
    org_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> DbResult<Vec<Uuid>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let Some(table) = table_for(entity_kind) else {
        return Err(crate::DbError::NotFound);
    };
    let sql =
        format!("SELECT id FROM {table} WHERE org_id = $1 AND id > $2 ORDER BY id ASC LIMIT $3");
    let rows = sqlx::query(&sql)
        .bind(org_id)
        .bind(after.unwrap_or(Uuid::nil()))
        .bind(limit.clamp(1, 500))
        .fetch_all(executor)
        .await?;
    Ok(rows.iter().map(|row| row.get("id")).collect())
}

/// Stable source-system keys for a page of internal ids: internal_id →
/// external_key. Absent from the result ⇒ the row was never pushed under a
/// key (hand-created data).
pub async fn external_key_map<'e, E>(
    executor: E,
    entity_kind: &str,
    org_id: Uuid,
    internal_ids: &[Uuid],
) -> DbResult<Vec<(Uuid, String)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    if internal_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT internal_id, external_key FROM external_ids \
         WHERE org_id = $1 AND entity_kind = $2 AND internal_id = ANY($3)",
    )
    .bind(org_id)
    .bind(entity_kind)
    .bind(internal_ids)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("internal_id"), row.get("external_key")))
        .collect())
}
