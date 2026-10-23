//! Plugin KV state: the store, the quota counter, export and erase (wave 53
//! lane 6, migration 0155; ADR 0023 Decision 5).
//!
//! `plugin_host::host_fns::kv` owns the ABI half — the argument shapes, the
//! namespace/key/value rules and the typed refusals. This module owns the SQL
//! and the row shapes, so the two cannot drift: every limit arrives as a
//! [`KvLimits`] argument rather than being retyped here, exactly the way
//! [`crate::plugin_health`] takes the quarantine threshold from the pool.
//!
//! ## Why a counter row and not a `SUM`
//!
//! `plugin_kv_usage` is read and written in the same transaction as the entry
//! it accounts for. The alternative — `SELECT sum(octet_length(value))` on every
//! `kv_put` — reads every row the install owns, up to 100,000 of them, on a path
//! the ADR budgets in microseconds. [`reconcile`] recomputes the counter from
//! the rows and reports a mismatch, so a lost update is a repair
//! ([`repair`]) rather than a silent overrun.
//!
//! ## What a put costs
//!
//! Four indexed statements inside the caller's transaction, and no scan:
//!
//! 1. `INSERT … SELECT … ON CONFLICT DO NOTHING` on `plugin_kv_usage` — creates
//!    the counter row on an install's first write and writes nothing after that.
//!    The org is DERIVED from `plugin_installs`, not bound, so a pair whose org
//!    and install disagree inserts nothing (`plugin_health`'s pattern).
//! 2. One `SELECT … FOR UPDATE OF u` that locks the counter and, as a scalar
//!    subquery, reads the length of the value it is about to replace. One
//!    primary-key lookup on `plugin_kv_entries`.
//! 3. The upsert on `plugin_kv_entries`, which is a primary-key write and also
//!    derives the org.
//! 4. `UPDATE plugin_kv_usage` with the ABSOLUTE new totals, never an
//!    accumulation, so a retried statement cannot double-count.
//!
//! The `FOR UPDATE` is what makes a concurrent put safe rather than lucky: two
//! writes for one install serialise on the counter row, so the second sees the
//! first's total and is refused if it would cross the limit. ADR 0023 Decision 4
//! already allows one concurrent invocation per install, so this lock is
//! belt-and-braces — and it is the only reason the quota cannot be overrun by
//! one.
//!
//! ## Query style
//!
//! Plain `sqlx::query` / `query_as` with explicit [`FromRow`] structs rather
//! than the compile-time `query!` macro, following [`crate::plugin_health`] and
//! `ai_policy` / `byok`: the module builds without the offline query cache, and
//! a schema change cannot silently pass because a stale `.sqlx` blob says so.
//!
//! Every function takes the caller's connection, which must already carry
//! `app.org_id` (see [`crate::set_org`]). The `org_id` predicate is
//! belt-and-braces beside the RLS policy, matching the other plugin modules.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use crate::{DbError, DbResult};

/// The refusal token for a write that would cross the install's quota. It is
/// the same static string [`plugin_host::abi::Refusal::KvQuotaExceeded`] carries
/// on the wire, so the host maps this error by parsing it rather than by
/// matching a second vocabulary.
///
/// (`plugin-host` depends on this crate, so the dependency cannot run the other
/// way. The shared string IS the contract, and `Refusal::parse` is the join.)
pub const QUOTA_EXCEEDED: &str = "plugin_kv_quota_exceeded";

/// The refusal token for a value above the ABI cap. See [`QUOTA_EXCEEDED`].
pub const VALUE_TOO_LARGE: &str = "plugin_kv_value_too_large";

/// The `source_kind` every receipt from this module carries.
pub const RECEIPT_SOURCE_KIND: &str = "plugin_kv";

/// The `namespace` a whole-install erase records, because the erase spans every
/// namespace the install used.
pub const RECEIPT_ALL_NAMESPACES: &str = "*";

/// The limits one store enforces. Constructed by the host from
/// `plugin_host::abi` and `plugin_host::kv`, never restated here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvLimits {
    /// One value (`abi::KV_VALUE_MAX_BYTES`).
    pub max_value_bytes: usize,
    /// Keys per install (`plugin_host::kv::KV_KEYS_PER_INSTALL`).
    pub max_keys: i64,
    /// Bytes per install (`plugin_host::kv::KV_BYTES_PER_INSTALL`).
    pub max_bytes: i64,
    /// Keys per list page (`abi::KV_LIST_PAGE`).
    pub page: i64,
}

impl KvLimits {
    pub const fn new(max_value_bytes: usize, max_keys: i64, max_bytes: i64, page: i64) -> Self {
        KvLimits {
            max_value_bytes,
            max_keys,
            max_bytes,
            page,
        }
    }
}

/// An install's quota position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvUsage {
    pub bytes: i64,
    pub keys: i64,
}

/// One stored entry, as an export or a restore sees it. Deliberately the SAME
/// type on both sides: what [`export_page`] produces is what [`restore`]
/// consumes, so a backup cannot drift from its restore.
///
/// The encoding is the caller's. ADR 0023 Decision 5 puts it outside the
/// database, so this type carries bytes and the export surface decides how to
/// spell them on the wire.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct KvEntry {
    pub namespace: String,
    pub key: String,
    pub value: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// One page of keys under a prefix. `next_cursor` is `None` on the last page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvKeyPage {
    pub keys: Vec<String>,
    pub next_cursor: Option<String>,
}

/// The counter recomputed from the rows beside the counter as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvReconcile {
    pub install_id: Uuid,
    pub recorded: KvUsage,
    pub actual: KvUsage,
}

impl KvReconcile {
    /// Whether the stored counter still matches the rows.
    pub fn matched(&self) -> bool {
        self.recorded == self.actual
    }
}

/// The proof that an install's state is gone. Same vocabulary as
/// `core_domain::rag::RagDeletionReceipt`: what was removed, how much, a hash
/// per removed value, who asked, when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvEraseReceipt {
    pub receipt_id: Uuid,
    pub org_id: Uuid,
    pub install_id: Uuid,
    /// [`RECEIPT_ALL_NAMESPACES`] for a whole-install erase.
    pub namespace: String,
    /// [`RECEIPT_SOURCE_KIND`].
    pub source_kind: String,
    /// The install id as text, the role `source_id` plays in the RAG receipt.
    pub source_id: String,
    pub deleted_entries: i32,
    pub deleted_bytes: i64,
    /// One sha256 per removed value, ordered by `(namespace, key)`.
    pub content_hashes: Vec<String>,
    pub deleted_by: Option<Uuid>,
    pub deleted_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct UsageProbe {
    used_bytes: i64,
    used_keys: i64,
    prior_bytes: Option<i64>,
}

#[derive(FromRow)]
struct ReconcileRow {
    install_id: Uuid,
    recorded_bytes: i64,
    recorded_keys: i64,
    actual_bytes: i64,
    actual_keys: i64,
}

#[derive(FromRow)]
struct RemovedRow {
    namespace: String,
    key: String,
    bytes: i64,
    hash: String,
}

#[derive(FromRow)]
struct ReceiptRow {
    receipt_id: Uuid,
    org_id: Uuid,
    install_id: Uuid,
    namespace: String,
    source_kind: String,
    source_id: String,
    deleted_entries: i32,
    deleted_bytes: i64,
    content_hashes: Vec<String>,
    deleted_by: Option<Uuid>,
    deleted_at: DateTime<Utc>,
}

impl From<ReceiptRow> for KvEraseReceipt {
    fn from(row: ReceiptRow) -> Self {
        KvEraseReceipt {
            receipt_id: row.receipt_id,
            org_id: row.org_id,
            install_id: row.install_id,
            namespace: row.namespace,
            source_kind: row.source_kind,
            source_id: row.source_id,
            deleted_entries: row.deleted_entries,
            deleted_bytes: row.deleted_bytes,
            content_hashes: row.content_hashes,
            deleted_by: row.deleted_by,
            deleted_at: row.deleted_at,
        }
    }
}

// ---------------------------------------------------------------------------
// The four verbs
// ---------------------------------------------------------------------------

/// Reads one value. `None` for an absent key and for one past its `expires_at`:
/// an expired key is invisible the moment it expires, whether or not
/// [`sweep_expired`] has run.
pub async fn get(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    namespace: &str,
    key: &str,
) -> DbResult<Option<Vec<u8>>> {
    let value = sqlx::query_scalar::<_, Vec<u8>>(
        r#"SELECT value
             FROM plugin_kv_entries
            WHERE org_id = $1
              AND install_id = $2
              AND namespace = $3
              AND key = $4
              AND (expires_at IS NULL OR expires_at > now())"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(namespace)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(value)
}

/// Writes one value, maintaining the quota counter in the same transaction.
///
/// Refuses with [`DbError::Conflict`] carrying [`QUOTA_EXCEEDED`] or
/// [`VALUE_TOO_LARGE`]; the caller's transaction is left usable, so a host
/// function can refuse and the invocation's other work still commits.
///
/// `org_id` is the caller's tenant. The rows are written with the org DERIVED
/// from `plugin_installs`, so passing an `org_id` that does not own
/// `install_id` writes nothing and answers [`DbError::NotFound`] — the same
/// guarantee [`crate::plugin_health::record_failure`] gives, for the same
/// reason.
// The middle five are one write (tenant, install, namespace, key, value) and
// the last two are its terms. A struct would add a type whose fields are
// spelled out at every call site and shorten nothing.
#[allow(clippy::too_many_arguments)]
pub async fn put(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    namespace: &str,
    key: &str,
    value: &[u8],
    expires_at: Option<DateTime<Utc>>,
    limits: &KvLimits,
) -> DbResult<()> {
    if value.len() > limits.max_value_bytes {
        return Err(DbError::Conflict(VALUE_TOO_LARGE.to_string()));
    }

    // 1. Create the counter row on the install's first write. `DO NOTHING`
    //    means this is a no-op on every later one.
    //
    //    `org_id` is DERIVED from `plugin_installs` rather than bound from the
    //    caller, the way `plugin_health::record_failure` does it: a pair whose
    //    org and install disagree then inserts nothing at all, because
    //    `plugin_installs` is itself RLS-scoped and hides the other tenant's
    //    install. That is a stronger guarantee than the policy on this table,
    //    which can only check the org.
    sqlx::query(
        r#"INSERT INTO plugin_kv_usage (org_id, install_id, bytes, keys, updated_at)
           SELECT i.org_id, i.id, 0, 0, now()
             FROM plugin_installs i
            WHERE i.id = $1
           ON CONFLICT (install_id) DO NOTHING"#,
    )
    .bind(install_id)
    .execute(&mut *conn)
    .await?;

    // 2. Lock the counter and read the value this write replaces, in one round
    //    trip. `FOR UPDATE OF u` locks only the counter row; the subquery's
    //    lookup on `plugin_kv_entries` is a primary-key read that locks nothing.
    let probe = sqlx::query_as::<_, UsageProbe>(
        r#"SELECT u.bytes AS used_bytes,
                  u.keys::bigint AS used_keys,
                  (SELECT octet_length(e.value)::bigint
                     FROM plugin_kv_entries e
                    WHERE e.install_id = u.install_id
                      AND e.namespace  = $2
                      AND e.key        = $3) AS prior_bytes
             FROM plugin_kv_usage u
            WHERE u.install_id = $1
              AND u.org_id = $4
              FOR UPDATE OF u"#,
    )
    .bind(install_id)
    .bind(namespace)
    .bind(key)
    .bind(org_id)
    .fetch_optional(&mut *conn)
    .await?
    // No counter row after the insert above means the install is not visible
    // under this tenant's scope, or its counter names another org — both are a
    // refusal rather than a write.
    .ok_or(DbError::NotFound)?;

    // 3. Admit or refuse. An overwrite moves the byte total by the difference
    //    and leaves the key count alone, so rewriting a key at the same size
    //    costs nothing against the quota.
    let prior = probe.prior_bytes.unwrap_or(0);
    let next_bytes = probe.used_bytes - prior + value.len() as i64;
    let next_keys = probe.used_keys + i64::from(probe.prior_bytes.is_none());
    if next_bytes > limits.max_bytes || next_keys > limits.max_keys {
        return Err(DbError::Conflict(QUOTA_EXCEEDED.to_string()));
    }

    // 4. The write the counter is accounting for. `org_id` is derived here too,
    //    so the entry and the counter cannot disagree about the tenant even if a
    //    caller passed a pair that does.
    let written = sqlx::query(
        r#"INSERT INTO plugin_kv_entries
             (org_id, install_id, namespace, key, value, expires_at, created_at, updated_at)
           SELECT i.org_id, i.id, $2, $3, $4, $5, now(), now()
             FROM plugin_installs i
            WHERE i.id = $1
           ON CONFLICT (install_id, namespace, key) DO UPDATE
             SET value = EXCLUDED.value,
                 expires_at = EXCLUDED.expires_at,
                 updated_at = now()"#,
    )
    .bind(install_id)
    .bind(namespace)
    .bind(key)
    .bind(value)
    .bind(expires_at)
    .execute(&mut *conn)
    .await?;
    // Zero rows means the install is not visible under this scope, which is a
    // refusal rather than a write that silently did nothing.
    if written.rows_affected() == 0 {
        return Err(DbError::NotFound);
    }

    // 5. Absolute totals, never `bytes = bytes + $2`: a retried statement then
    //    cannot double-count, and the lock above already serialised the writers.
    sqlx::query(
        r#"UPDATE plugin_kv_usage
              SET bytes = $2, keys = $3, updated_at = now()
            WHERE install_id = $1"#,
    )
    .bind(install_id)
    .bind(next_bytes)
    .bind(next_keys)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

/// Deletes one key and gives its bytes back to the quota. Deleting an absent key
/// is success and moves nothing, the way every other store behaves.
///
/// Returns whether a row was there, which the host does not need (the ABI's
/// `kv_delete` answer is only "did it work") and a caller reading a log might.
pub async fn delete(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    namespace: &str,
    key: &str,
) -> DbResult<bool> {
    let removed = sqlx::query_scalar::<_, i64>(
        r#"DELETE FROM plugin_kv_entries
            WHERE org_id = $1
              AND install_id = $2
              AND namespace = $3
              AND key = $4
        RETURNING octet_length(value)::bigint"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(namespace)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await?;

    let Some(bytes) = removed else {
        return Ok(false);
    };

    // `GREATEST(…, 0)` rather than a bare subtraction: the counter is a cache of
    // the rows, and a reconcile that has not run yet must never be able to drive
    // it negative and trip the CHECK.
    sqlx::query(
        r#"UPDATE plugin_kv_usage
              SET bytes = GREATEST(bytes - $2, 0),
                  keys  = GREATEST(keys - 1, 0),
                  updated_at = now()
            WHERE install_id = $1"#,
    )
    .bind(install_id)
    .bind(bytes)
    .execute(&mut *conn)
    .await?;

    Ok(true)
}

/// One page of keys under `prefix`, ordered by key and keyset-paginated on the
/// key itself so a page is an index range scan rather than an `OFFSET`.
///
/// `prefix` is matched with `left(key, length($6)) = $6`, not `LIKE`: a key may
/// contain `%` or `_`, and a prefix that quietly becomes a wildcard would let a
/// guest read keys outside the prefix it asked for.
pub async fn list_keys(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    namespace: &str,
    prefix: &str,
    cursor: Option<&str>,
    limits: &KvLimits,
) -> DbResult<KvKeyPage> {
    let page = limits.page.clamp(1, 500);
    // One extra row is the "is there another page" test, so the caller never
    // gets a cursor that leads to an empty page.
    let mut keys = sqlx::query_scalar::<_, String>(
        r#"SELECT key
             FROM plugin_kv_entries
            WHERE org_id = $1
              AND install_id = $2
              AND namespace = $3
              AND ($5 = '' OR key > $5)
              AND ($6 = '' OR left(key, length($6)) = $6)
              AND (expires_at IS NULL OR expires_at > now())
            ORDER BY key
            LIMIT $4"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(namespace)
    .bind(page + 1)
    .bind(cursor.unwrap_or(""))
    .bind(prefix)
    .fetch_all(&mut *conn)
    .await?;

    let next_cursor = if keys.len() as i64 > page {
        keys.truncate(page as usize);
        keys.last().cloned()
    } else {
        None
    };
    Ok(KvKeyPage { keys, next_cursor })
}

// ---------------------------------------------------------------------------
// Export and restore
// ---------------------------------------------------------------------------

/// One page of the install's entries, ordered by `(namespace, key)`.
///
/// This is the export's read half and the restore's input shape. `after` is the
/// `(namespace, key)` of the last row the caller wrote out; pass
/// `("", "")` for the first page, because the schema forbids an empty namespace
/// and an empty key, so every row sorts after that pair. A row comparison
/// (`(namespace, key) > ($3, $4)`) is a primary-key range scan, so a 256 MiB
/// export is a loop over pages and never one statement holding the install.
pub async fn export_page(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    after: (&str, &str),
    limit: i64,
) -> DbResult<Vec<KvEntry>> {
    let rows = sqlx::query_as::<_, KvEntry>(
        r#"SELECT namespace, key, value, expires_at
             FROM plugin_kv_entries
            WHERE org_id = $1
              AND install_id = $2
              AND (namespace, key) > ($3, $4)
            ORDER BY namespace, key
            LIMIT $5"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(after.0)
    .bind(after.1)
    .bind(limit.clamp(1, 1_000))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Re-applies exported entries through the ordinary [`put`] path, so a restore
/// meets the same limits and the same quota as the writes it is restoring. An
/// export that no longer fits the quota is refused rather than silently
/// truncated, which is the difference between a restore and a lossy import.
///
/// Returns how many entries were written. A restore is idempotent: `put` is an
/// upsert, so replaying the same export twice lands the same rows.
pub async fn restore(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    entries: &[KvEntry],
    limits: &KvLimits,
) -> DbResult<usize> {
    for entry in entries {
        put(
            conn,
            org_id,
            install_id,
            &entry.namespace,
            &entry.key,
            &entry.value,
            entry.expires_at,
            limits,
        )
        .await?;
    }
    Ok(entries.len())
}

// ---------------------------------------------------------------------------
// Erase
// ---------------------------------------------------------------------------

/// Erases one install's state: `DELETE` by `install_id`, a counter reset and a
/// receipt, in the caller's transaction so all three land or none do.
///
/// The delete and the receipt are one statement, mirroring
/// `rag::delete_source_in_namespace`: the receipt is minted from the `RETURNING`
/// rows of the delete it describes, so a receipt with a count cannot exist
/// without the delete that produced it. A second erase mints a second receipt
/// with `deleted_entries = 0` rather than failing, which is what an idempotent
/// uninstall retry needs.
///
/// Cost: at the 256 MiB byte quota the statement touches at most 256 MiB of
/// values, because it hashes each one as it removes it. That is an operator
/// action inside an uninstall's transaction, never a hot path.
pub async fn erase(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    deleted_by: Option<Uuid>,
) -> DbResult<KvEraseReceipt> {
    let receipt_id = Uuid::now_v7();
    let source_id = install_id.to_string();

    let rows = sqlx::query_as::<_, RemovedRow>(
        r#"DELETE FROM plugin_kv_entries
            WHERE org_id = $1 AND install_id = $2
        RETURNING namespace,
                  key,
                  octet_length(value)::bigint AS bytes,
                  encode(sha256(value), 'hex') AS hash"#,
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_all(&mut *conn)
    .await?;

    // Ordered here rather than by an aggregate `ORDER BY`, because the hashes
    // have to line up with the order the export would have produced and the
    // delete's own row order is unspecified.
    let mut rows = rows;
    rows.sort_by(|a, b| (&a.namespace, &a.key).cmp(&(&b.namespace, &b.key)));
    let deleted_entries = rows.len() as i32;
    let deleted_bytes = rows.iter().map(|row| row.bytes).sum::<i64>();
    let content_hashes: Vec<String> = rows.into_iter().map(|row| row.hash).collect();

    let row = sqlx::query_as::<_, ReceiptRow>(
        r#"INSERT INTO plugin_kv_erasure_receipts
             (receipt_id, org_id, install_id, namespace, source_kind, source_id,
              deleted_entries, deleted_bytes, content_hashes, deleted_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING receipt_id, org_id, install_id, namespace, source_kind, source_id,
                  deleted_entries, deleted_bytes, content_hashes, deleted_by, deleted_at"#,
    )
    .bind(receipt_id)
    .bind(org_id)
    .bind(install_id)
    .bind(RECEIPT_ALL_NAMESPACES)
    .bind(RECEIPT_SOURCE_KIND)
    .bind(&source_id)
    .bind(deleted_entries)
    .bind(deleted_bytes)
    .bind(&content_hashes)
    .bind(deleted_by)
    .fetch_one(&mut *conn)
    .await?;

    // The counter is reset, not deleted: the row carries the org for RLS, and a
    // zero row keeps the install's first post-erase put on the same fast path.
    // `org_id` is filtered here as well as in the delete so the owner pool — which
    // bypasses RLS — cannot zero a counter that belongs to another tenant.
    sqlx::query(
        r#"UPDATE plugin_kv_usage
              SET bytes = 0, keys = 0, updated_at = now()
            WHERE install_id = $1 AND org_id = $2"#,
    )
    .bind(install_id)
    .bind(org_id)
    .execute(&mut *conn)
    .await?;

    Ok(row.into())
}

/// Reads one erase receipt by id, under RLS.
pub async fn get_erase_receipt(
    conn: &mut PgConnection,
    org_id: Uuid,
    receipt_id: Uuid,
) -> DbResult<Option<KvEraseReceipt>> {
    let row = sqlx::query_as::<_, ReceiptRow>(
        r#"SELECT receipt_id, org_id, install_id, namespace, source_kind, source_id,
                  deleted_entries, deleted_bytes, content_hashes, deleted_by, deleted_at
             FROM plugin_kv_erasure_receipts
            WHERE org_id = $1 AND receipt_id = $2"#,
    )
    .bind(org_id)
    .bind(receipt_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into))
}

/// Lists an install's erase receipts, newest first. This is the "did the erase
/// happen" answer an operator asks for, and it survives the uninstall that
/// produced it because the receipt carries no foreign key to the install.
pub async fn list_erase_receipts(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    limit: i64,
) -> DbResult<Vec<KvEraseReceipt>> {
    let rows = sqlx::query_as::<_, ReceiptRow>(
        r#"SELECT receipt_id, org_id, install_id, namespace, source_kind, source_id,
                  deleted_entries, deleted_bytes, content_hashes, deleted_by, deleted_at
             FROM plugin_kv_erasure_receipts
            WHERE org_id = $1 AND install_id = $2
            ORDER BY deleted_at DESC, receipt_id DESC
            LIMIT $3"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

// ---------------------------------------------------------------------------
// Reconcile, repair and the TTL statement
// ---------------------------------------------------------------------------

/// Recomputes one install's usage from its rows and returns it beside the stored
/// counter. A mismatch means a write landed without its counter update (or the
/// other way round), and [`repair`] fixes it.
pub async fn reconcile(
    conn: &mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
) -> DbResult<KvReconcile> {
    let row = sqlx::query_as::<_, ReconcileRow>(
        r#"SELECT $2::uuid AS install_id,
                  COALESCE(u.bytes, 0) AS recorded_bytes,
                  COALESCE(u.keys::bigint, 0) AS recorded_keys,
                  COALESCE(a.actual_bytes, 0) AS actual_bytes,
                  COALESCE(a.actual_keys, 0) AS actual_keys
             FROM (SELECT 1) AS one
             LEFT JOIN plugin_kv_usage u ON u.install_id = $2
             LEFT JOIN (SELECT COALESCE(sum(octet_length(value)), 0)::bigint AS actual_bytes,
                               count(*)::bigint AS actual_keys
                          FROM plugin_kv_entries
                         WHERE org_id = $1 AND install_id = $2) a ON true"#,
    )
    .bind(org_id)
    .bind(install_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(KvReconcile {
        install_id: row.install_id,
        recorded: KvUsage {
            bytes: row.recorded_bytes,
            keys: row.recorded_keys,
        },
        actual: KvUsage {
            bytes: row.actual_bytes,
            keys: row.actual_keys,
        },
    })
}

/// Reconciles every install in the org and LOGS each mismatch, which is the
/// periodic check ADR 0023 Decision 5 asks for. Returns only the rows that
/// disagreed, because a clean org is an empty result and a log line per clean
/// install is noise that trains an operator to stop reading.
///
/// Deliberately reports rather than repairs: a check that silently fixes what it
/// finds cannot tell you it found something. [`repair`] is the second step, and
/// the caller decides whether to take it.
pub async fn reconcile_org(conn: &mut PgConnection, org_id: Uuid) -> DbResult<Vec<KvReconcile>> {
    let rows = sqlx::query_as::<_, ReconcileRow>(
        r#"SELECT i.id AS install_id,
                  COALESCE(u.bytes, 0) AS recorded_bytes,
                  COALESCE(u.keys::bigint, 0) AS recorded_keys,
                  COALESCE(a.actual_bytes, 0) AS actual_bytes,
                  COALESCE(a.actual_keys, 0) AS actual_keys
             FROM plugin_installs i
             LEFT JOIN plugin_kv_usage u ON u.install_id = i.id
             LEFT JOIN (SELECT install_id,
                               sum(octet_length(value))::bigint AS actual_bytes,
                               count(*)::bigint AS actual_keys
                          FROM plugin_kv_entries
                         WHERE org_id = $1
                         GROUP BY install_id) a ON a.install_id = i.id
            WHERE i.org_id = $1
            ORDER BY i.id"#,
    )
    .bind(org_id)
    .fetch_all(&mut *conn)
    .await?;

    let mut mismatched = Vec::new();
    for row in rows {
        let found = KvReconcile {
            install_id: row.install_id,
            recorded: KvUsage {
                bytes: row.recorded_bytes,
                keys: row.recorded_keys,
            },
            actual: KvUsage {
                bytes: row.actual_bytes,
                keys: row.actual_keys,
            },
        };
        if !found.matched() {
            tracing::warn!(
                org_id = %org_id,
                install_id = %found.install_id,
                recorded_bytes = found.recorded.bytes,
                actual_bytes = found.actual.bytes,
                recorded_keys = found.recorded.keys,
                actual_keys = found.actual.keys,
                "plugin KV quota counter disagrees with the rows"
            );
            mismatched.push(found);
        }
    }
    Ok(mismatched)
}

/// Sets one install's counter to what its rows actually hold. The repair half of
/// [`reconcile_org`].
///
/// `org_id` is bound rather than derived, and that is the one place in this
/// module where it is: a repair has to be able to write a counter for an install
/// whose rows are already gone, and the caller's org comes from
/// [`reconcile_org`], which is itself org-scoped, so a pair that disagrees
/// cannot reach here.
pub async fn repair(conn: &mut PgConnection, org_id: Uuid, install_id: Uuid) -> DbResult<KvUsage> {
    let found = reconcile(conn, org_id, install_id).await?;
    sqlx::query(
        r#"INSERT INTO plugin_kv_usage (org_id, install_id, bytes, keys, updated_at)
           VALUES ($1, $2, $3, $4, now())
           ON CONFLICT (install_id) DO UPDATE
             SET bytes = EXCLUDED.bytes,
                 keys = EXCLUDED.keys,
                 updated_at = now()"#,
    )
    .bind(org_id)
    .bind(install_id)
    .bind(found.actual.bytes)
    .bind(found.actual.keys as i32)
    .execute(&mut *conn)
    .await?;
    Ok(found.actual)
}

/// The TTL statement: removes up to `batch` expired rows and gives their bytes
/// back to the counters they were charged against, in one statement.
///
/// This is NOT a sweeper — no loop, no clock, no interval. ADR 0023 Decision 5
/// says the existing retention job sweeps plugin TTL, and this is the statement
/// it runs. Registration is a follow-up outside this lane's files: the retention
/// matrix enforces per-TABLE age windows through `updated_at`, and a per-ROW
/// `expires_at` is a different mechanism, so the matrix needs a variant before
/// it can carry this table.
///
/// Correctness does not wait for it. Every read in this module filters
/// `expires_at > now()`, so an expired key is invisible the moment it expires;
/// the sweep only reclaims the bytes it is still charged for.
pub async fn sweep_expired(conn: &mut PgConnection, batch: i64) -> DbResult<u64> {
    let removed = sqlx::query(
        r#"WITH expired AS (
             SELECT install_id, namespace, key
               FROM plugin_kv_entries
              WHERE expires_at IS NOT NULL AND expires_at <= now()
              ORDER BY expires_at
              LIMIT $1
           ),
           gone AS (
             DELETE FROM plugin_kv_entries e
              USING expired x
              WHERE e.install_id = x.install_id
                AND e.namespace = x.namespace
                AND e.key = x.key
             RETURNING e.install_id, octet_length(e.value) AS bytes
           ),
           per_install AS (
             SELECT install_id, sum(bytes)::bigint AS bytes, count(*)::integer AS keys
               FROM gone
              GROUP BY install_id
           )
           UPDATE plugin_kv_usage u
              SET bytes = GREATEST(u.bytes - p.bytes, 0),
                  keys  = GREATEST(u.keys - p.keys, 0),
                  updated_at = now()
             FROM per_install p
            WHERE u.install_id = p.install_id"#,
    )
    .bind(batch.clamp(1, 10_000))
    .execute(&mut *conn)
    .await?;
    // The statement's row count is the number of COUNTERS updated, not the
    // number of entries removed; callers that need the latter read
    // `deleted_entries` from a reconcile or count before and after.
    Ok(removed.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_tokens_are_the_abi_strings() {
        // The join with `plugin_host::abi::Refusal` is the string itself; a
        // rename on either side has to be caught here or the host maps a quota
        // refusal onto `plugin_abi_frame_invalid` and the audit row lies.
        assert_eq!(QUOTA_EXCEEDED, "plugin_kv_quota_exceeded");
        assert_eq!(VALUE_TOO_LARGE, "plugin_kv_value_too_large");
        assert!(QUOTA_EXCEEDED.starts_with("plugin_"));
        assert!(VALUE_TOO_LARGE.starts_with("plugin_"));
    }

    #[test]
    fn a_matching_counter_reconciles_and_a_drifted_one_does_not() {
        let matched = KvReconcile {
            install_id: Uuid::nil(),
            recorded: KvUsage { bytes: 8, keys: 2 },
            actual: KvUsage { bytes: 8, keys: 2 },
        };
        assert!(matched.matched());
        let drifted = KvReconcile {
            actual: KvUsage { bytes: 9, keys: 2 },
            ..matched
        };
        assert!(!drifted.matched());
    }

    #[test]
    fn a_receipt_carries_the_rag_vocabulary() {
        let receipt = KvEraseReceipt {
            receipt_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            namespace: RECEIPT_ALL_NAMESPACES.to_string(),
            source_kind: RECEIPT_SOURCE_KIND.to_string(),
            source_id: "install".into(),
            deleted_entries: 0,
            deleted_bytes: 0,
            content_hashes: Vec::new(),
            deleted_by: None,
            deleted_at: Utc::now(),
        };
        assert_eq!(receipt.namespace, "*");
        assert_eq!(receipt.source_kind, "plugin_kv");
    }

    #[test]
    fn limits_are_arguments_not_constants() {
        let limits = KvLimits::new(1024, 10, 4096, 2);
        assert_eq!(limits.max_value_bytes, 1024);
        assert_eq!(limits.max_keys, 10);
        assert_eq!(limits.max_bytes, 4096);
        assert_eq!(limits.page, 2);
    }
}
