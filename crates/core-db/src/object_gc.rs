//! core-db/object_gc — the two directions of drift between rows and bytes.
//!
//! Purpose: the database and the object store are two systems with no shared
//! transaction, and every failure of that seam is silent.
//!
//! * **Bytes with no row.** An upload that wrote the object and then failed to
//!   commit its row; a row deleted while the store delete failed; a restore that
//!   brought back an older row set. Nothing ever reclaims them, so the bucket
//!   grows by the size of every abandoned attempt. The integrity pass calls
//!   these *orphans* and deletes them once they are older than the grace window.
//! * **A row with no bytes.** Restore from a dump older than the object
//!   snapshot, a bucket policy, an operator emptying a volume. The row keeps
//!   advertising a download that fails. The pass records the row's key, and
//!   `attachments.missing_since` / `documents_attachments.missing_since` make
//!   the state readable instead of a 500.
//!
//! This module holds the SQL for both directions so the worker's pass stays a
//! loop and a policy decision, and so a test can call either half without a
//! store.
//!
//! Non-purpose: no store calls (the caller owns the adapter), no deletion
//! policy (the grace window is the caller's), and no assumption that a row's
//! key is unique — content addressing means several rows legitimately share
//! one object, which is why a reference is a set membership question and why
//! deleting a row must never delete its object directly.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

/// One row's claim on one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectReference {
    pub storage_key: String,
    /// Table the claim came from, for the missing-bytes report.
    pub source_table: &'static str,
    pub org_id: Option<Uuid>,
}

/// Every object key any row currently claims.
///
/// The four sources are the four tables that hold a `storage_key`. Correctness
/// of the orphan direction depends on this list being COMPLETE: a table missing
/// here does not leak, it deletes the objects of a table it forgot, which is why
/// the sweep refuses to run when a source reports an error rather than treating
/// it as "no references".
///
/// `email_messages.raw_ref` is deliberately absent: it is a provider-side
/// pointer to a raw MIME payload, not a key in this store.
pub async fn referenced_keys(pool: &PgPool) -> DbResult<Vec<ObjectReference>> {
    let mut out = Vec::new();
    for (table, label) in SOURCES {
        let rows: Vec<(String, Option<Uuid>)> =
            sqlx::query_as(&format!("SELECT storage_key, org_id FROM {table}"))
                .fetch_all(pool)
                .await?;
        out.extend(
            rows.into_iter()
                .map(|(storage_key, org_id)| ObjectReference {
                    storage_key,
                    source_table: label,
                    org_id,
                }),
        );
    }
    Ok(out)
}

/// Marks the attachment rows behind `keys` as missing bytes.
///
/// Idempotent and non-destructive: it sets a timestamp nobody's download path
/// depended on before, and it never deletes the row — an operator may be able
/// to re-upload the same bytes, and the key is a digest, so a matching upload
/// clears the mark (see [`clear_missing`]).
pub async fn mark_missing(pool: &PgPool, keys: &[String]) -> DbResult<u64> {
    if keys.is_empty() {
        return Ok(0);
    }
    let mut updated = 0;
    for table in MISSING_TABLES {
        updated += sqlx::query(&format!(
            "UPDATE {table} SET missing_since = now() \
             WHERE storage_key = ANY($1) AND missing_since IS NULL"
        ))
        .bind(keys)
        .execute(pool)
        .await?
        .rows_affected();
    }
    Ok(updated)
}

/// Clears the mark when the bytes are back, and closes any open finding for the
/// key. This is the repair half: a re-upload stores the same content address, so
/// "the bytes exist again" is enough to make the row whole.
pub async fn clear_missing(pool: &PgPool, keys: &[String]) -> DbResult<u64> {
    if keys.is_empty() {
        return Ok(0);
    }
    let mut updated = 0;
    for table in MISSING_TABLES {
        updated += sqlx::query(&format!(
            "UPDATE {table} SET missing_since = NULL \
             WHERE storage_key = ANY($1) AND missing_since IS NOT NULL"
        ))
        .bind(keys)
        .execute(pool)
        .await?
        .rows_affected();
    }
    sqlx::query(
        "UPDATE object_integrity_findings SET resolved_at = now() \
         WHERE kind = 'bytes_missing' AND resolved_at IS NULL AND storage_key = ANY($1)",
    )
    .bind(keys)
    .execute(pool)
    .await?;
    Ok(updated)
}

/// The tables that hold object keys, with the label a finding records.
const SOURCES: [(&str, &str); 4] = [
    ("attachments", "attachments"),
    ("documents_attachments", "documents_attachments"),
    ("attachment_scans", "attachment_scans"),
    ("org_exports", "org_exports"),
];

/// The tables whose rows can say their bytes are gone.
const MISSING_TABLES: [&str; 2] = ["attachments", "documents_attachments"];

/// One finding, ready to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: FindingKind,
    pub org_id: Option<Uuid>,
    pub storage_key: String,
    pub size_bytes: Option<i64>,
    pub source_table: Option<&'static str>,
    pub note: Option<String>,
}

/// Which direction a finding belongs to. The database constrains the vocabulary
/// to these two tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    OrphanDeleted,
    BytesMissing,
}

impl FindingKind {
    /// The token stored in `object_integrity_findings.kind`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrphanDeleted => "orphan_deleted",
            Self::BytesMissing => "bytes_missing",
        }
    }
}

/// Records one pass's findings in as few statements as the shape allows.
///
/// One row per finding: a pass that reclaimed a thousand orphaned uploads is a
/// thousand rows, which is the point — the ledger is what makes "the bucket grew
/// 4 GB last month" answerable.
pub async fn record_findings(pool: &PgPool, pass_id: Uuid, findings: &[Finding]) -> DbResult<u64> {
    if findings.is_empty() {
        return Ok(0);
    }
    let kinds: Vec<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
    let org_ids: Vec<Option<Uuid>> = findings.iter().map(|f| f.org_id).collect();
    let keys: Vec<&str> = findings.iter().map(|f| f.storage_key.as_str()).collect();
    let sizes: Vec<Option<i64>> = findings.iter().map(|f| f.size_bytes).collect();
    let sources: Vec<Option<&str>> = findings.iter().map(|f| f.source_table).collect();
    let notes: Vec<Option<&str>> = findings.iter().map(|f| f.note.as_deref()).collect();
    let result = sqlx::query(
        r#"INSERT INTO object_integrity_findings
             (id, pass_id, kind, org_id, storage_key, size_bytes, source_table, note)
           SELECT gen_random_uuid(), $1, k, o, s, sz, src, n
             FROM UNNEST($2::text[], $3::uuid[], $4::text[], $5::bigint[], $6::text[], $7::text[])
               AS t(k, o, s, sz, src, n)"#,
    )
    .bind(pass_id)
    .bind(&kinds)
    .bind(&org_ids)
    .bind(&keys)
    .bind(&sizes)
    .bind(&sources)
    .bind(&notes)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// What the last pass over a table found, for the health surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MissingCounts {
    pub attachments: i64,
    pub documents_attachments: i64,
}

/// How many rows currently advertise bytes that are not there.
pub async fn missing_counts(pool: &PgPool) -> DbResult<MissingCounts> {
    let row: (i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM attachments WHERE missing_since IS NOT NULL),
             (SELECT count(*) FROM documents_attachments WHERE missing_since IS NOT NULL)"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(MissingCounts {
        attachments: row.0,
        documents_attachments: row.1,
    })
}

/// Fetches one page of the rows behind a set of keys, so a finding can name the
/// table and the org without a second query per key.
pub async fn reference_index(
    pool: &PgPool,
    keys: &[String],
) -> DbResult<HashMap<String, ObjectReference>> {
    let mut index = HashMap::new();
    if keys.is_empty() {
        return Ok(index);
    }
    for (table, label) in SOURCES {
        let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(&format!(
            "SELECT storage_key, org_id FROM {table} WHERE storage_key = ANY($1)"
        ))
        .bind(keys)
        .fetch_all(pool)
        .await?;
        for (storage_key, org_id) in rows {
            index.entry(storage_key.clone()).or_insert(ObjectReference {
                storage_key,
                source_table: label,
                org_id,
            });
        }
    }
    Ok(index)
}
