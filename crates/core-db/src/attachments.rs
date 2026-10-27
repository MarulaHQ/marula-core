//! Attachment metadata queries (the private build tracker 2.14).
//!
//! Purpose: persistence for `attachments` rows — insert, org-scoped
//! reads, comment linking, deletion with content-address refcounting,
//! portal visibility filtering, the dedup lookup that lets N rows share one
//! stored object, and the two admission-facing reads Wave 55 added: the list
//! filter that keeps an undownloadable object out of a list, and the walk a
//! verdict backfill uses.
//!
//! Non-purpose: no byte movement (callers go through
//! `core_store::ObjectStore`), no admission POLICY (the three arms live in
//! `core-api`'s `check_admission`; this file only expresses the same arms as
//! a WHERE clause), no authorization.

use crate::{DbError, DbResult};
use core_domain::entities::{Attachment, AttachmentKind};
use sqlx::{PgConnection, Postgres};
use uuid::Uuid;

const COLS: &str = "id, org_id, kind::text AS kind, ticket_id, ticket_comment_id, \
                    email_message_id, filename, content_type, size_bytes, sha256, \
                    storage_key, uploaded_by_user_id, uploaded_by_portal_user_id, created_at";

/// Same projection, qualified for queries that JOIN ticket_comments
/// (unqualified columns are ambiguous once both tables are in scope).
const QUALIFIED_COLS: &str = "a.id, a.org_id, a.kind::text AS kind, a.ticket_id, \
                    a.ticket_comment_id, a.email_message_id, a.filename, a.content_type, \
                    a.size_bytes, a.sha256, a.storage_key, a.uploaded_by_user_id, \
                    a.uploaded_by_portal_user_id, a.created_at";

fn row_to_attachment(row: sqlx::postgres::PgRow) -> Result<Attachment, sqlx::Error> {
    use sqlx::Row;
    let kind: String = row.get("kind");
    Ok(Attachment {
        id: row.get("id"),
        org_id: row.get("org_id"),
        kind: kind
            .parse()
            .map_err(|e: core_domain::DomainError| sqlx::Error::ColumnDecode {
                index: "kind".into(),
                source: Box::new(e),
            })?,
        ticket_id: row.get("ticket_id"),
        ticket_comment_id: row.get("ticket_comment_id"),
        email_message_id: row.get("email_message_id"),
        filename: row.get("filename"),
        content_type: row.get("content_type"),
        size_bytes: row.get("size_bytes"),
        sha256: row.get("sha256"),
        storage_key: row.get("storage_key"),
        uploaded_by_user_id: row.get("uploaded_by_user_id"),
        uploaded_by_portal_user_id: row.get("uploaded_by_portal_user_id"),
        created_at: row.get("created_at"),
    })
}

/// A new attachment row's fields (bytes already stored by the caller).
pub struct NewAttachment<'a> {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: AttachmentKind,
    pub ticket_id: Option<Uuid>,
    pub ticket_comment_id: Option<Uuid>,
    pub email_message_id: Option<Uuid>,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size_bytes: i64,
    pub sha256: &'a str,
    pub storage_key: &'a str,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
}

/// Inserts one attachment metadata row.
pub async fn insert(conn: &mut PgConnection, a: NewAttachment<'_>) -> DbResult<Attachment> {
    let row = sqlx::query(&format!(
        r#"INSERT INTO attachments (id, org_id, kind, ticket_id, ticket_comment_id,
                                   email_message_id, filename, content_type, size_bytes,
                                   sha256, storage_key, uploaded_by_user_id,
                                   uploaded_by_portal_user_id)
           VALUES ($1, $2, $3::text::attachment_kind, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
           RETURNING {COLS}"#
    ))
    .bind(a.id)
    .bind(a.org_id)
    .bind(a.kind.as_str())
    .bind(a.ticket_id)
    .bind(a.ticket_comment_id)
    .bind(a.email_message_id)
    .bind(a.filename)
    .bind(a.content_type)
    .bind(a.size_bytes)
    .bind(a.sha256)
    .bind(a.storage_key)
    .bind(a.uploaded_by_user_id)
    .bind(a.uploaded_by_portal_user_id)
    .try_map(row_to_attachment)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Fetches one attachment by id within the org.
pub async fn get<'e, E>(executor: E, org_id: Uuid, id: Uuid) -> DbResult<Attachment>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND id = $2"#
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_attachment)
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::NotFound)?;
    Ok(row)
}

/// The list side of the download door's admission rule, in one place.
///
/// `check_admission` (`crates/core-api/src/routes/attachments.rs`) decides per
/// OBJECT whether the bytes may leave: an admitted verdict passes, a held one
/// is refused, and an object with no verdict on record is refused only where a
/// scanner is installed — a none-mode deployment could never have written the
/// row, so it keeps the streaming posture it had. A list that shows an object
/// its own download then refuses is a promise the door breaks, so those three
/// arms are expressed here once and every list query interpolates the result
/// instead of each handler filtering by hand.
///
/// `param` is the 1-based bind index of the `scanner_installed` boolean the
/// caller passes. False makes the whole clause a no-op, which is what keeps a
/// none-mode deployment's lists exactly as they were.
///
/// Two consequences are part of the contract, not accidents:
///   * a released object lists again, because a release is the operator
///     admitting it after the fact (the row keeps `infected` as evidence);
///   * a HELD object disappears from lists in a deployment with no scanner
///     too — that arm is about the verdict, not about who could write one.
///
/// The table alias is `a`, so every query using this names its table `a`.
#[must_use]
pub fn admitted_verdict_filter(param: usize) -> String {
    format!(
        r#" AND (${param}::bool IS FALSE OR EXISTS (
                   SELECT 1 FROM attachment_scans s
                    WHERE s.org_id = a.org_id AND s.sha256 = a.sha256
                      AND (s.status <> 'infected' OR s.released_at IS NOT NULL)))"#
    )
}

/// Every attachment hanging off one ticket (any kind) that the download door
/// would also hand out.
///
/// `scanner_installed` is the door's flag, not a per-row fact: see
/// [`admitted_verdict_filter`] for why the two must be read together.
pub async fn list_for_ticket<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    scanner_installed: bool,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           WHERE a.org_id = $1 AND a.ticket_id = $2{}
           ORDER BY a.created_at, a.id"#,
        admitted_verdict_filter(3)
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(scanner_installed)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Attachments bound to one specific comment that the download door would
/// also hand out (same admission filter as [`list_for_ticket`]).
pub async fn list_for_comment<'e, E>(
    executor: E,
    org_id: Uuid,
    comment_id: Uuid,
    scanner_installed: bool,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           WHERE a.org_id = $1 AND a.ticket_comment_id = $2{}
           ORDER BY a.created_at, a.id"#,
        admitted_verdict_filter(3)
    ))
    .bind(org_id)
    .bind(comment_id)
    .bind(scanner_installed)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Attachments extracted from one inbound email message.
pub async fn list_for_email<'e, E>(
    executor: E,
    org_id: Uuid,
    email_message_id: Uuid,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {COLS} FROM attachments WHERE org_id = $1 AND email_message_id = $2 ORDER BY created_at, id"#
    ))
    .bind(org_id)
    .bind(email_message_id)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// Dedup lookup: the storage key this org ALREADY holds for these exact
/// bytes, when any prior attachment references them.
pub async fn find_existing_object_key(
    conn: &mut PgConnection,
    org_id: Uuid,
    sha256_hex: &str,
) -> DbResult<Option<String>> {
    let key: Option<String> = sqlx::query_scalar(
        "SELECT storage_key FROM attachments WHERE org_id = $1 AND sha256 = $2 LIMIT 1",
    )
    .bind(org_id)
    .bind(sha256_hex)
    .fetch_optional(conn)
    .await?;
    Ok(key)
}

/// Binds an unattached upload to its comment. Succeeds only when the row
/// is still unlinked and belongs to the same ticket — double-link races
/// surface as `None`.
pub async fn link_to_comment(
    conn: &mut PgConnection,
    org_id: Uuid,
    attachment_id: Uuid,
    ticket_id: Uuid,
    comment_id: Uuid,
) -> DbResult<Option<Attachment>> {
    let row = sqlx::query(&format!(
        r#"UPDATE attachments SET ticket_comment_id = $3
           WHERE org_id = $1 AND id = $2 AND ticket_id = $4
             AND kind::text = 'ticket_comment' AND ticket_comment_id IS NULL
           RETURNING {COLS}"#
    ))
    .bind(org_id)
    .bind(attachment_id)
    .bind(comment_id)
    .bind(ticket_id)
    .try_map(row_to_attachment)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

/// Deletes one row; returns it so the caller can reclaim the object only
/// when [`count_other_refs`] reports zero remaining references.
pub async fn delete(
    conn: &mut PgConnection,
    org_id: Uuid,
    id: Uuid,
) -> DbResult<Option<Attachment>> {
    let row = sqlx::query(&format!(
        r#"DELETE FROM attachments WHERE org_id = $1 AND id = $2 RETURNING {COLS}"#
    ))
    .bind(org_id)
    .bind(id)
    .try_map(row_to_attachment)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

// --- the refcount: who still points at a stored object ------------------------

/// Every table whose rows can reference one stored attachment object, with the
/// migration that created it and what its rows own.
///
/// `attachments` (0024) holds ticket, ticket-comment and inbound-email files;
/// `documents_attachments` (0089) holds document files. They write into ONE
/// content-addressed key space per org (`core_store::attachment_key` →
/// `org/<org>/attachments/<sha256>`), so the same bytes attached on two
/// surfaces are stored once and referenced twice. A reclaim decision is
/// therefore not complete until EVERY table here has been asked: counting one
/// of them is how a document's bytes were deleted out from under its row
/// (Wave 56, audit record `attachments.rs:count-other-refs`).
///
/// This list feeds [`other_refs_sql`], so a table added here is counted in the
/// same edit. A table that carries the key without owning the object belongs in
/// [`ATTACHMENT_KEY_TABLES_NOT_REFERENCES`] with its reason. The
/// `attachment_refcount` suite reads the database's own catalogue and fails
/// when a table holding a `storage_key` column is in neither list, so a new
/// referencing table cannot be forgotten quietly.
///
/// It is a CHECKED PROJECTION of [`crate::object_inventory::ENTRIES`], not a
/// second authority: `owners()` there filters the declaration, and
/// `attachments::tests` fails when this constant and that projection stop
/// naming the same tables. The why-strings are the refcount's own wording; the
/// declaration carries the longer one. It stays a const because it is published
/// API, and a const cannot be built from an iterator at compile time — the
/// equality test is what makes the projection mechanical.
///
/// The orphan sweep claims a wider set from the same declaration
/// ([`crate::object_gc::sources`]): every table here, plus every table that
/// merely names a key in this bucket and every other key space the bucket
/// holds. A table added to the declaration without a decision here still makes
/// the sweep claim it, which is the safe direction; the test closes the unsafe
/// one.
pub const ATTACHMENT_REF_TABLES: &[(&str, &str)] = &[
    (
        "attachments",
        "0024 — ticket, ticket-comment and inbound-email files",
    ),
    ("documents_attachments", "0089 — document files"),
];

/// Tables that hold an attachment object's key and must NOT keep it alive,
/// each with the reason it is excluded from [`ATTACHMENT_REF_TABLES`].
///
/// These rows record a fact ABOUT the object (a verdict, a finding) or live in
/// a different key space. Letting them hold the bytes hostage would leak
/// objects instead of protecting them, which is why the exclusion is stated
/// rather than left to a `WHERE` clause nobody can read.
///
/// Like the owner list this is a checked projection of
/// [`crate::object_inventory::ENTRIES`] — specifically its `storage_key` columns
/// that are not owners. Columns of another shape (`secret_records.object_key`,
/// `plugin_installs.module_digest`, …) are outside this list by construction:
/// the refcount binds a key that a `storage_key` row holds outright, so a vault
/// key or a plugin digest can never match an arm here. The sweep is the reader
/// that must claim those, and it does, from the same declaration.
pub const ATTACHMENT_KEY_TABLES_NOT_REFERENCES: &[(&str, &str)] = &[
    (
        "attachment_scans",
        "0139 — the scan verdict about the object, not an owner of it: the row is \
         evidence and outlives the bytes by design, and a re-upload of the same \
         digest reuses it",
    ),
    (
        "org_exports",
        "0089 — a different key space (`org/<org>/exports/<id>/bundle.json`, built \
         by the export route); its digest names an export bundle, never an \
         attachment object",
    ),
    (
        "object_integrity_findings",
        "0146 — an integrity pass's evidence about a key, written after the fact \
         and keyed by no owner",
    ),
];

/// The refcount statement: `count(*)` over one `UNION ALL` arm per table in
/// [`ATTACHMENT_REF_TABLES`], so "is this object still referenced anywhere" is
/// one round trip rather than one query per surface — and no surface can be
/// skipped by a caller that only knows about its own table.
///
/// Every arm binds the same three parameters: the org (the key space is per
/// org), the storage key, and `$3` — the caller's own row, which the ticket
/// side still has on disk when it asks.
#[must_use]
pub fn other_refs_sql() -> String {
    let mut sql = String::from("SELECT count(*) FROM (");
    for (i, (table, _why)) in ATTACHMENT_REF_TABLES.iter().enumerate() {
        if i > 0 {
            sql.push_str(" UNION ALL ");
        }
        sql.push_str("SELECT 1 FROM ");
        sql.push_str(table);
        sql.push_str(" WHERE org_id = $1 AND storage_key = $2 AND id <> $3");
    }
    sql.push_str(") refs");
    sql
}

/// How many OTHER rows still reference this storage key after a delete —
/// across every table in [`ATTACHMENT_REF_TABLES`], because they share one key
/// space. The object is reclaimed only on zero.
pub async fn count_other_refs(
    conn: &mut PgConnection,
    org_id: Uuid,
    storage_key: &str,
    excluding_id: Uuid,
) -> DbResult<i64> {
    let n: i64 = sqlx::query_scalar(&other_refs_sql())
        .bind(org_id)
        .bind(storage_key)
        .bind(excluding_id)
        .fetch_one(conn)
        .await?;
    Ok(n)
}

/// Routing targets of an inbound message (for authz on its attachments).
pub async fn email_targets(
    conn: &mut PgConnection,
    org_id: Uuid,
    email_message_id: Uuid,
) -> DbResult<Option<(Option<Uuid>, Option<Uuid>)>> {
    let row: Option<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT ticket_id, comment_id FROM email_messages WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(email_message_id)
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

// --- portal visibility (the private build tracker 2.13 exposure × the private build tracker 2.14 files) -------

/// Portal-visible attachments for one exposed ticket: kind=ticket rows
/// always, comment rows when their note kind is in the visible set, and
/// email rows always (inbound mail creates client-visible notes only).
/// Invisible rows are indistinguishable from absent ones (no existence
/// leak through 404s).
///
/// Wave 55 (F31): visibility is not the last word — the SAME object is also
/// filtered by [`admitted_verdict_filter`], so a client list never offers a
/// file the portal download would answer with a 409.
pub async fn list_portal_visible<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    visible_comment_kinds: &[String],
    scanner_installed: bool,
) -> DbResult<Vec<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           LEFT JOIN ticket_comments c ON c.id = a.ticket_comment_id
           WHERE a.org_id = $1 AND a.ticket_id = $2
             AND (a.kind::text = 'ticket'
                  OR c.kind = ANY($3)
                  OR a.kind::text = 'email_message'){}
           ORDER BY a.created_at, a.id"#,
        admitted_verdict_filter(4)
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(visible_comment_kinds.to_vec())
    .bind(scanner_installed)
    .try_map(row_to_attachment)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// One portal-visible attachment by id under the same visibility rule as
/// [`list_portal_visible`]; `None` covers both missing and hidden.
pub async fn find_portal_visible<'e, E>(
    executor: E,
    org_id: Uuid,
    ticket_id: Uuid,
    attachment_id: Uuid,
    visible_comment_kinds: &[String],
) -> DbResult<Option<Attachment>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let row = sqlx::query(&format!(
        r#"SELECT {QUALIFIED_COLS} FROM attachments a
           LEFT JOIN ticket_comments c ON c.id = a.ticket_comment_id
           WHERE a.org_id = $1 AND a.ticket_id = $2 AND a.id = $3
             AND (a.kind::text = 'ticket'
                  OR c.kind = ANY($4)
                  OR a.kind::text = 'email_message')"#
    ))
    .bind(org_id)
    .bind(ticket_id)
    .bind(attachment_id)
    .bind(visible_comment_kinds.to_vec())
    .try_map(row_to_attachment)
    .fetch_optional(executor)
    .await?;
    Ok(row)
}

// --- the verdict backfill's walk (Wave 55, F30) -------------------------------

/// One stored object with no admission verdict on record, as the backfill
/// needs it: the org scope to record under, the digest the verdict is keyed
/// by, and the metadata the verdict row carries.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UnverdictObject {
    pub org_id: Uuid,
    pub sha256: String,
    pub storage_key: String,
    pub content_type: String,
    pub filename: String,
    pub size_bytes: i64,
    pub ticket_id: Option<Uuid>,
    pub uploaded_by_user_id: Option<Uuid>,
    pub uploaded_by_portal_user_id: Option<Uuid>,
}

/// The object set the backfill walks, written once so the paged walk and the
/// count cannot disagree about what "has no verdict" means.
///
/// The two tables share ONE content-addressed key space per org
/// (`core_store::attachment_key`), and a verdict is keyed by
/// `(org_id, sha256)` rather than by owner, so an object referenced from both
/// tables is one object and must produce one verdict — hence `DISTINCT ON`,
/// oldest row first. `$1` narrows the walk to one org; NULL means every org,
/// which is why the operator command needs a connection that bypasses RLS.
const UNVERDICT_OBJECTS: &str = r#"
WITH objects AS (
    SELECT a.org_id, a.sha256, a.storage_key, a.content_type, a.filename,
           a.size_bytes, a.ticket_id, a.uploaded_by_user_id,
           a.uploaded_by_portal_user_id, a.created_at
      FROM attachments a
     WHERE ($1::uuid IS NULL OR a.org_id = $1)
    UNION ALL
    SELECT d.org_id, d.sha256, d.storage_key, d.content_type, d.filename,
           d.size_bytes, NULL::uuid, d.uploaded_by,
           NULL::uuid, d.created_at
      FROM documents_attachments d
     WHERE ($1::uuid IS NULL OR d.org_id = $1)
), missing AS (
    SELECT DISTINCT ON (o.org_id, o.sha256)
           o.org_id, o.sha256, o.storage_key, o.content_type, o.filename,
           o.size_bytes, o.ticket_id, o.uploaded_by_user_id,
           o.uploaded_by_portal_user_id
      FROM objects o
     WHERE NOT EXISTS (
               SELECT 1 FROM attachment_scans s
                WHERE s.org_id = o.org_id AND s.sha256 = o.sha256)
     ORDER BY o.org_id, o.sha256, o.created_at
)
"#;

/// One page of objects missing a verdict, past a `(org_id, sha256)` keyset
/// cursor and ordered by it.
///
/// Keyset rather than offset because the walk WRITES as it goes: rows leave
/// the set as they are judged, so an offset would skip half of them. The
/// cursor also means an object the walk cannot judge (bytes gone, digest
/// mismatch, scanner outage) is stepped over instead of being re-selected
/// forever.
pub async fn list_unverdict_objects<'e, E>(
    executor: E,
    org_id: Option<Uuid>,
    after: Option<(Uuid, String)>,
    limit: i64,
) -> DbResult<Vec<UnverdictObject>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let (after_org, after_sha) = after.unwrap_or((Uuid::nil(), String::new()));
    let rows = sqlx::query_as::<_, UnverdictObject>(&format!(
        r#"{UNVERDICT_OBJECTS}
           SELECT org_id, sha256, storage_key, content_type, filename,
                  size_bytes, ticket_id, uploaded_by_user_id,
                  uploaded_by_portal_user_id
             FROM missing
            WHERE (org_id, sha256) > ($2::uuid, $3::text)
            ORDER BY org_id, sha256
            LIMIT $4"#
    ))
    .bind(org_id)
    .bind(after_org)
    .bind(after_sha)
    .bind(limit)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

/// How many objects per org still have no verdict: the number a dry run
/// reports and the number `--confirm` is checked against.
pub async fn unverdict_counts<'e, E>(
    executor: E,
    org_id: Option<Uuid>,
) -> DbResult<Vec<(Uuid, i64)>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query_as::<_, (Uuid, i64)>(&format!(
        r#"{UNVERDICT_OBJECTS}
           SELECT org_id, count(*)::bigint FROM missing GROUP BY org_id ORDER BY org_id"#
    ))
    .bind(org_id)
    .fetch_all(executor)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cols_cover_every_entity_field() {
        // Guard against silent column drift between COLS and the entity.
        for col in [
            "id",
            "org_id",
            "kind",
            "ticket_id",
            "ticket_comment_id",
            "email_message_id",
            "filename",
            "content_type",
            "size_bytes",
            "sha256",
            "storage_key",
            "uploaded_by_user_id",
            "uploaded_by_portal_user_id",
            "created_at",
        ] {
            assert!(COLS.contains(col), "COLS missing {col}");
        }
    }

    #[test]
    fn qualified_cols_qualify_every_column() {
        for col in COLS.split(',') {
            let bare = col.trim().split(' ').next().unwrap_or("");
            assert!(
                QUALIFIED_COLS.contains(&format!("a.{bare}")),
                "QUALIFIED_COLS missing a.{bare}"
            );
        }
    }

    /// The refcount statement is generated from the inventory, so the two
    /// cannot drift: every listed table gets an arm with the same three binds,
    /// and no excluded table is counted.
    #[test]
    fn the_refcount_names_every_referencing_table_and_binds_three_params() {
        let sql = other_refs_sql();
        assert!(
            sql.starts_with("SELECT count(*) FROM ("),
            "one statement over one UNION ALL set: {sql}"
        );
        for (table, _why) in ATTACHMENT_REF_TABLES {
            assert!(
                sql.contains(&format!(
                    "FROM {table} WHERE org_id = $1 AND storage_key = $2 AND id <> $3"
                )),
                "the inventory names {table} but the statement does not ask it: {sql}"
            );
        }
        assert_eq!(
            sql.matches("$1").count(),
            ATTACHMENT_REF_TABLES.len(),
            "every arm binds the org: {sql}"
        );
        assert_eq!(
            sql.matches(" UNION ALL ").count(),
            ATTACHMENT_REF_TABLES.len() - 1,
            "one arm per table, joined by UNION ALL: {sql}"
        );
        for (table, why) in ATTACHMENT_KEY_TABLES_NOT_REFERENCES {
            assert!(
                !sql.contains(&format!("FROM {table} ")),
                "{table} is excluded on purpose ({why}) and must not be counted: {sql}"
            );
        }
    }

    /// The exclusion list carries a reason for every table, because the reason
    /// is what stops a future reader from "fixing" the omission.
    #[test]
    fn every_excluded_table_states_why() {
        for (table, why) in ATTACHMENT_KEY_TABLES_NOT_REFERENCES {
            assert!(
                !why.trim().is_empty(),
                "{table} is excluded without a reason"
            );
            assert!(
                !ATTACHMENT_REF_TABLES.iter().any(|(t, _)| t == table),
                "{table} is in both lists, so the inventory disagrees with itself"
            );
        }
    }

    /// The owner list is a PROJECTION of the one declaration, not a second
    /// authority. Adding a key space there without adding it here (or the
    /// reverse) fails: the unsafe half would let a delete reclaim bytes a
    /// sibling table still holds.
    #[test]
    fn the_owner_list_is_the_declarations_owning_projection() {
        let declared: Vec<&str> = crate::object_inventory::owners()
            .iter()
            .map(|entry| entry.table)
            .collect();
        let listed: Vec<&str> = ATTACHMENT_REF_TABLES.iter().map(|(t, _)| *t).collect();
        for table in &declared {
            assert!(
                listed.contains(table),
                "{table} owns attachment bytes in object_inventory::ENTRIES but the \
                 refcount does not count it, so a delete can reclaim bytes it still \
                 holds. Add it to ATTACHMENT_REF_TABLES."
            );
        }
        for table in &listed {
            assert!(
                declared.contains(table),
                "ATTACHMENT_REF_TABLES counts {table} and the declaration does not \
                 call its rows owners. Decide it in object_inventory::ENTRIES first."
            );
        }
    }

    /// The same for the excluded list: it must be exactly the declaration's
    /// uncounted `storage_key` columns, so a table that stops holding a key
    /// cannot linger here as a stale reason.
    #[test]
    fn the_excluded_list_is_the_declarations_uncounted_projection() {
        let declared: Vec<&str> = crate::object_inventory::storage_key_uncounted()
            .iter()
            .map(|entry| entry.table)
            .collect();
        let listed: Vec<&str> = ATTACHMENT_KEY_TABLES_NOT_REFERENCES
            .iter()
            .map(|(t, _)| *t)
            .collect();
        for table in &declared {
            assert!(
                listed.contains(table),
                "{table} holds a storage_key and the declaration does not count it as \
                 an owner, but the refcount does not exclude it either. Exclude it \
                 with the reason, or the next reader cannot tell which question it \
                 belongs to."
            );
        }
        for table in &listed {
            assert!(
                declared.contains(table),
                "ATTACHMENT_KEY_TABLES_NOT_REFERENCES excludes {table}, which the \
                 declaration does not list among its uncounted storage_key columns."
            );
        }
    }

    /// The list filter is the SQL spelling of `core-api`'s `check_admission`.
    /// Its ARMS are what makes a list agree with a door, so each one is named
    /// here: a change to the door that is not mirrored here fails this test's
    /// reading, and a change here that drops an arm fails it outright.
    #[test]
    fn the_admission_filter_names_every_arm_of_the_doors_rule() {
        let filter = admitted_verdict_filter(4);
        assert!(
            filter.contains("$4::bool IS FALSE"),
            "the flag's bind index must be the caller's, or the query binds nowhere: {filter}"
        );
        assert!(
            filter.contains("s.org_id = a.org_id AND s.sha256 = a.sha256"),
            "the verdict is keyed by (org, digest), not by owner: {filter}"
        );
        assert!(
            filter.contains("s.status <> 'infected'"),
            "an admitted verdict (clean or skipped) is the pass arm: {filter}"
        );
        assert!(
            filter.contains("s.released_at IS NOT NULL"),
            "a released object is admitted again; without this arm a release could not list: {filter}"
        );
        assert!(
            filter.contains("EXISTS"),
            "an ABSENT verdict must fail the clause when the flag is true: {filter}"
        );
    }
}
