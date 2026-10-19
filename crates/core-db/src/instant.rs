//! Reads over the instant-first write path's per-entity counter
//! (`entity_write_seqs`, migration 0028).
//!
//! Purpose: expose the monotonic `seq` the write path allocates, so a reader
//! can name the entity's current revision without materializing the entity
//! itself. The API's conditional reads use it as a weak validator.
//!
//! Non-purpose: this module never writes and never gates a write — the
//! If-Match-style allocation and the locking increment live in
//! `core-api::instant::entity_seq`, which owns the row lock. Everything here
//! is a plain read.

use crate::DbResult;
use sqlx::Postgres;
use uuid::Uuid;

/// The entity's current write seq, or `None` when it has no counter row.
///
/// One indexed lookup: the table's primary key is
/// `(org_id, entity, entity_id)`, so the predicate below is a point read on
/// that index — no scan, no extra index needed. `None` means the entity has
/// never been written through the instant-first path (or does not exist in
/// this org); callers that need "revision 1 for a fresh entity" should treat
/// it that way rather than substituting a default here.
pub async fn read_entity_seq<'e, E>(
    executor: E,
    org_id: Uuid,
    entity: &str,
    entity_id: Uuid,
) -> DbResult<Option<i64>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let seq: Option<i64> = sqlx::query_scalar(
        "SELECT seq FROM entity_write_seqs \
         WHERE org_id = $1 AND entity = $2 AND entity_id = $3",
    )
    .bind(org_id)
    .bind(entity)
    .bind(entity_id)
    .fetch_optional(executor)
    .await?;
    Ok(seq)
}
