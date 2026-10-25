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
/// Correctness of the orphan direction depends on [`SOURCES`] being COMPLETE: a
/// table missing there does not leak, it deletes the objects of a table it
/// forgot, which is why the sweep refuses to run when a source reports an error
/// rather than treating it as "no references".
pub async fn referenced_keys(pool: &PgPool) -> DbResult<Vec<ObjectReference>> {
    let mut out = Vec::new();
    for (table, label, _why) in SOURCES {
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

/// The tables whose rows claim an object key, with the label a finding records
/// and the reason the table is a source at all.
///
/// This is the second inventory over one storage-key space. The first is the
/// attachment refcount ([`crate::attachments::ATTACHMENT_REF_TABLES`] and its
/// exclusion list), and the two answer different questions:
///
/// * the refcount asks "does any row still OWN these bytes", to decide whether a
///   row's delete may remove the object. It counts owners only.
/// * this list asks "does any row CLAIM this key", to decide whether an object
///   found in the store is an orphan. It must count owners and every other row
///   that names a key in this bucket, including tables that merely record a fact
///   about the bytes.
///
/// So the relation is one-way and mechanical: this list must be a SUPERSET of
/// the refcount's owners, and every extra entry must belong to the refcount's
/// exclusion list. `object_gc::tests` and `tests/object_inventory_parity.rs`
/// enforce both halves, so a key-holding table that is added to one inventory
/// and not the other fails a test instead of leaking or reclaiming silently.
///
/// `object_integrity_findings` holds a `storage_key` too, and is deliberately
/// absent: a finding records that a key was seen in a past pass (0146), so
/// counting it would keep a deleted object's key alive forever and protect any
/// later upload at the same address from ever being reclaimed.
///
/// `email_messages.raw_ref` is absent for the same class of reason: it is a
/// provider-side pointer to a raw MIME payload, not a key in this store.
pub const SOURCES: &[(&str, &str, &str)] = &[
    (
        "attachments",
        "attachments",
        "0024 — ticket, ticket-comment and inbound-email files; counted by the \
         attachment refcount as an owner",
    ),
    (
        "documents_attachments",
        "documents_attachments",
        "0089 — document files, on the same content-addressed key; counted by the \
         attachment refcount as an owner",
    ),
    (
        "attachment_scans",
        "attachment_scans",
        "0139 — the scan verdict's `storage_key` is the admission key for \
         clean/skipped bytes and the quarantine key for infected ones, so these \
         rows name objects in this bucket. Not an owner (the refcount excludes \
         it), which makes this entry the conservative half of the pair: a \
         verdict row keeps the bytes it names from being reclaimed",
    ),
    (
        "org_exports",
        "org_exports",
        "0089 — export bundles under a different key space; not an attachment, so \
         the refcount excludes it, but the bundle is stored in this bucket and the \
         sweep lists the whole bucket",
    ),
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
    for (table, label, _why) in SOURCES {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{ATTACHMENT_KEY_TABLES_NOT_REFERENCES, ATTACHMENT_REF_TABLES};

    fn source_tables() -> Vec<&'static str> {
        SOURCES.iter().map(|(table, _, _)| *table).collect()
    }

    /// The direction that loses bytes: a table the refcount counts as an owner
    /// and this list does not name is a table whose objects the sweep calls
    /// orphans and deletes while rows still advertise them.
    #[test]
    fn every_refcount_owner_is_a_gc_source() {
        let sources = source_tables();
        for (table, why) in ATTACHMENT_REF_TABLES {
            assert!(
                sources.contains(table),
                "{table} is an attachment refcount owner ({why}) but the orphan \
                 sweep does not ask it, so the sweep would delete its live objects. \
                 Add it to object_gc::SOURCES."
            );
        }
    }

    /// The other direction: a source the refcount has never decided about means
    /// one of the two inventories is missing a table, and the next reader cannot
    /// tell which question it belongs to.
    #[test]
    fn every_gc_source_is_in_the_refcounts_universe() {
        let mut decided: Vec<&str> = ATTACHMENT_REF_TABLES.iter().map(|(t, _)| *t).collect();
        decided.extend(ATTACHMENT_KEY_TABLES_NOT_REFERENCES.iter().map(|(t, _)| *t));
        for (table, _, why) in SOURCES {
            assert!(
                decided.contains(table),
                "{table} is an orphan-sweep source ({why}) that the attachment \
                 refcount neither counts nor excludes. Decide it there first: \
                 ATTACHMENT_REF_TABLES if its rows own attachment bytes, \
                 ATTACHMENT_KEY_TABLES_NOT_REFERENCES with the reason they do not."
            );
        }
    }

    /// The reason is what stops a later reader from "tidying" a source out of
    /// the list because it looks redundant with the attachment owners.
    #[test]
    fn every_gc_source_states_why_it_claims_the_key() {
        for (table, label, why) in SOURCES {
            assert!(!why.trim().is_empty(), "{table} claims keys with no reason");
            assert!(!label.trim().is_empty(), "{table} has no finding label");
        }
    }

    /// Two entries for one table would query it twice and report two findings
    /// for one object; one entry per table is what makes the list reviewable.
    #[test]
    fn no_table_is_a_source_twice() {
        let sources = source_tables();
        let mut seen = sources.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            sources.len(),
            "SOURCES names a table twice: {sources:?}"
        );
    }
}
