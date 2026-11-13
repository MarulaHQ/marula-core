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

use crate::{object_inventory, DbResult};

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
/// Correctness of the orphan direction depends on the claim set being COMPLETE:
/// a table missing there does not leak, it deletes the objects of a table it
/// forgot, which is why the sweep refuses to run when a source reports an error
/// rather than treating it as "no references".
///
/// The set is a projection of [`crate::object_inventory`] — the one declaration
/// of every column that can name an object — so a new key space is one edit, not
/// one edit per reader. A vault document's sealed ciphertext and a plugin's
/// module bytes are in this same bucket; a sweep that did not claim them would
/// free them a day after the last write while rows still pointed at them.
pub async fn referenced_keys(pool: &PgPool) -> DbResult<Vec<ObjectReference>> {
    let mut out = Vec::new();
    for entry in object_inventory::claimed() {
        for (storage_key, org_id) in entry.keys(pool).await? {
            out.push(ObjectReference {
                storage_key,
                source_table: entry.table,
                org_id,
            });
        }
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
    for (table, column) in MISSING_TABLES {
        updated += sqlx::query(&format!(
            "UPDATE {table} SET missing_since = now() \
             WHERE {column} = ANY($1) AND missing_since IS NULL"
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
    for (table, column) in MISSING_TABLES {
        updated += sqlx::query(&format!(
            "UPDATE {table} SET missing_since = NULL \
             WHERE {column} = ANY($1) AND missing_since IS NOT NULL"
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

/// The tables whose rows claim an object key — [`crate::object_inventory`]'s
/// claim set, which is what "does any row reference this key" means.
///
/// The relation between the two readers is one-way and mechanical:
///
/// * the refcount asks "does any row still OWN these bytes", to decide whether a
///   row's delete may remove the object. It counts owners only.
/// * this list asks "does any row CLAIM this key", to decide whether an object
///   found in the store is an orphan. It must count owners and every other row
///   that names a key in this bucket, including tables that merely record a fact
///   about the bytes, and every other key space the bucket holds.
///
/// So this list must be a SUPERSET of the refcount's owners, and each extra
/// entry must state why the refcount does not count it.
/// [`crate::object_inventory`] holds both projections over one declaration, and
/// `object_gc::tests` plus `tests/object_inventory_parity.rs` enforce both halves,
/// so a key-holding table that is added to one reader and not the other fails a
/// test instead of leaking or reclaiming silently.
///
/// `email_messages.raw_ref` is absent from the declaration for the same class of
/// reason `object_integrity_findings` is: it is a provider-side pointer to a raw
/// MIME payload, not a key in this store.
#[must_use]
pub fn sources() -> Vec<&'static object_inventory::KeyEntry> {
    object_inventory::claimed()
}

/// The tables whose rows can say their bytes are gone, with the column that
/// names the key.
///
/// One entry per key space the bucket holds and whose rows own bytes: the two
/// attachment tables (one key space), and the vault (its own). A CHECKED
/// PROJECTION of [`crate::object_inventory`] — owners, plus the vault column —
/// and `every_owning_key_space_can_say_its_bytes_are_gone` fails when the two
/// disagree, so a new owning table either arrives here or fails a test.
///
/// Plugin bytes are deliberately absent: their column holds a digest and the
/// key is rebuilt in Rust, so a store key cannot be turned back into the row
/// without carrying the raw column through the pass. That is a signature
/// change, recorded rather than guessed.
pub const MISSING_TABLES: &[(&str, &str)] = &[
    ("attachments", "storage_key"),
    ("documents_attachments", "storage_key"),
    ("secret_records", "object_key"),
];

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

    /// The inverse of [`Self::as_str`]. `None` for anything the column's CHECK
    /// would refuse, so a caller filtering by kind turns a typo into a 422
    /// instead of a silently empty page.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "orphan_deleted" => Some(Self::OrphanDeleted),
            "bytes_missing" => Some(Self::BytesMissing),
            _ => None,
        }
    }
}

/// One finding, read back. The write side is [`Finding`]; this is the same row
/// once the pass has committed it, with the identity and timestamps the ledger
/// assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFinding {
    pub id: Uuid,
    /// Groups every finding of one pass, so a pass is reviewable as a unit.
    pub pass_id: Uuid,
    pub found_at: chrono::DateTime<chrono::Utc>,
    pub kind: FindingKind,
    pub storage_key: String,
    /// Bytes reclaimed (orphan) or bytes that should have been there (missing).
    pub size_bytes: Option<i64>,
    /// Which table referenced the key, for a missing-bytes finding.
    pub source_table: Option<String>,
    /// Set when a later pass saw the key healthy again.
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub note: Option<String>,
}

/// What an operator asked to see of the finding ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindingFilter {
    pub kind: Option<FindingKind>,
    /// Resolved findings are history: the default list is what is still open,
    /// because that is what needs a decision.
    pub include_resolved: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Open findings by direction, for the surface's header counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenFindingCounts {
    pub orphans: i64,
    pub missing: i64,
}

/// The `query_as` row shape behind [`list_findings`]. A named alias because the
/// tuple is wider than clippy's complexity budget, and the shape belongs to one
/// statement.
type FindingRow = (
    Uuid,
    Uuid,
    chrono::DateTime<chrono::Utc>,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<String>,
);

/// One page of the finding ledger, newest first.
///
/// The org scope is the row's own: a finding's `org_id` is derived from the key
/// prefix (`org/<uuid>/...`), and a key that predates that convention is stored
/// with NULL and is visible to every org — migration 0146's policy says so. The
/// explicit `$1` predicate is therefore belt AND braces beside RLS: it keeps the
/// statement's intent readable, and it is what the RLS pool-read gate looks for.
pub async fn list_findings<'e, E>(
    executor: E,
    org_id: Uuid,
    filter: FindingFilter,
) -> DbResult<Vec<IntegrityFinding>>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows: Vec<FindingRow> = sqlx::query_as(
        r#"SELECT id, pass_id, found_at, kind, storage_key, size_bytes, source_table,
                  resolved_at, note
           FROM object_integrity_findings
           WHERE (org_id = $1 OR org_id IS NULL)
             AND ($2::text IS NULL OR kind = $2)
             AND ($3::boolean OR resolved_at IS NULL)
           ORDER BY found_at DESC, id DESC
           LIMIT $4 OFFSET $5"#,
    )
    .bind(org_id)
    .bind(filter.kind.map(FindingKind::as_str))
    .bind(filter.include_resolved)
    .bind(filter.limit.clamp(1, 200))
    .bind(filter.offset.max(0))
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(
                id,
                pass_id,
                found_at,
                kind,
                storage_key,
                size_bytes,
                source_table,
                resolved_at,
                note,
            )| {
                // The column's CHECK admits exactly two tokens, so a parse miss is
                // a schema drift: dropping the row would hide it, and inventing a
                // kind would mislabel it. `parse` returning None is unreachable
                // through this table, and the type says so rather than panicking.
                Some(IntegrityFinding {
                    id,
                    pass_id,
                    found_at,
                    kind: FindingKind::parse(&kind)?,
                    storage_key,
                    size_bytes,
                    source_table,
                    resolved_at,
                    note,
                })
            },
        )
        .collect())
}

/// How many findings are still open, by direction. Two numbers on one scan.
pub async fn open_finding_counts<'e, E>(executor: E, org_id: Uuid) -> DbResult<OpenFindingCounts>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let (orphans, missing): (i64, i64) = sqlx::query_as(
        r#"SELECT
             count(*) FILTER (WHERE kind = 'orphan_deleted'),
             count(*) FILTER (WHERE kind = 'bytes_missing')
           FROM object_integrity_findings
           WHERE (org_id = $1 OR org_id IS NULL)
             AND resolved_at IS NULL"#,
    )
    .bind(org_id)
    .fetch_one(executor)
    .await?;
    Ok(OpenFindingCounts { orphans, missing })
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
    for entry in object_inventory::claimed() {
        for (storage_key, org_id) in entry.rows_naming(pool, keys).await? {
            index.entry(storage_key.clone()).or_insert(ObjectReference {
                storage_key,
                source_table: entry.table,
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
    use crate::object_inventory::KeyShape;

    fn source_columns() -> Vec<(&'static str, &'static str)> {
        sources()
            .iter()
            .map(|entry| (entry.table, entry.column))
            .collect()
    }

    /// The direction that loses bytes: a table the refcount counts as an owner
    /// and this list does not name is a table whose objects the sweep calls
    /// orphans and deletes while rows still advertise them.
    #[test]
    fn every_refcount_owner_is_a_gc_source() {
        let sources = source_columns();
        for entry in ATTACHMENT_REF_TABLES {
            let (table, why) = (entry.table, entry.why);
            assert!(
                sources.iter().any(|(source, _)| *source == table),
                "{table} is an attachment refcount owner ({why}) but the orphan \
                 sweep does not ask it, so the sweep would delete its live objects. \
                 Add it to object_inventory::ENTRIES as an Owner."
            );
        }
    }

    /// Every key space whose rows own bytes can say its bytes are gone. The
    /// vault used to be the exception: a reveal of ciphertext that had vanished
    /// had no row-level answer, only whatever the store adapter said (F76).
    #[test]
    fn every_owning_key_space_can_say_its_bytes_are_gone() {
        let declared: Vec<(&str, &str)> = object_inventory::ENTRIES
            .iter()
            .filter(|entry| {
                entry.role == object_inventory::KeyRole::Owner || entry.column == "object_key"
            })
            .map(|entry| (entry.table, entry.column))
            .collect();
        assert_eq!(
            MISSING_TABLES,
            declared.as_slice(),
            "an owning key space cannot mark its bytes missing, or MISSING_TABLES \
             names a table that owns nothing"
        );
    }

    /// The other direction: a source the refcount has never decided about means
    /// one of the two readers is missing a table, and the next reader cannot
    /// tell which question it belongs to. A column of another shape is outside
    /// the refcount's universe by construction, so only `storage_key` claims are
    /// required to be decided there.
    #[test]
    fn every_storage_key_gc_source_is_in_the_refcounts_universe() {
        let mut decided: Vec<&str> = ATTACHMENT_REF_TABLES.iter().map(|e| e.table).collect();
        decided.extend(ATTACHMENT_KEY_TABLES_NOT_REFERENCES.iter().map(|(t, _)| *t));
        for entry in sources() {
            if entry.column != "storage_key" {
                continue;
            }
            assert!(
                decided.contains(&entry.table),
                "{} is an orphan-sweep source ({}) that the attachment refcount \
                 neither counts nor excludes. Decide it there first: \
                 ATTACHMENT_REF_TABLES if its rows own attachment bytes, \
                 ATTACHMENT_KEY_TABLES_NOT_REFERENCES with the reason they do not.",
                entry.table,
                entry.why
            );
        }
    }

    /// The reason is what stops a later reader from "tidying" a source out of
    /// the list because it looks redundant with the attachment owners.
    #[test]
    fn every_gc_source_states_why_it_claims_the_key() {
        for entry in sources() {
            assert!(
                !entry.why.trim().is_empty(),
                "{} claims keys with no reason",
                entry.table
            );
        }
    }

    /// Two entries for one column would query it twice and report two findings
    /// for one object; one entry per column is what makes the list reviewable.
    /// A table can legitimately appear twice when it holds two key columns.
    #[test]
    fn no_column_is_a_source_twice() {
        let sources = source_columns();
        let mut seen = sources.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            sources.len(),
            "the claim set names a column twice: {sources:?}"
        );
    }

    /// A key rebuilt from a digest must land inside the same org prefix every
    /// other claim lives under: that prefix is what makes a store listing
    /// attributable to a tenant, and the erasure deletes by it. A stored key is
    /// already a key, so the entry returns it unchanged.
    #[test]
    fn a_rebuilt_key_lands_inside_the_org_prefix() {
        let org = Uuid::nil();
        let digest = "a".repeat(64);
        for entry in sources() {
            let key = entry
                .build_key(&digest, Some(org))
                .expect("a claim builds a key");
            match entry.shape {
                KeyShape::Stored => assert_eq!(
                    key, digest,
                    "{} must not rewrite a key it holds outright",
                    entry.table
                ),
                KeyShape::PluginDigest => {
                    assert_eq!(
                        key,
                        core_store::plugin_artifact_key(org, &digest),
                        "{} must build the plugin key through core-store",
                        entry.table
                    );
                    assert!(
                        key.starts_with(&format!("org/{org}/plugins/")),
                        "{} builds {key}, outside its own org namespace",
                        entry.table
                    );
                }
            }
        }
    }
}
