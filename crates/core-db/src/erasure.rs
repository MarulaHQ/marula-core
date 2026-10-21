//! core-db/erasure — what "delete this tenant" means, in SQL.
//!
//! Purpose: a per-tenant erasure has three surfaces with three different
//! mechanisms, and only one of them is a transaction.
//!
//! * **Rows.** One `DELETE FROM orgs` cascades through every table that names
//!   the org — including the audit trail, the search index and the RAG vectors,
//!   which all carry an org foreign key on purpose. That is the correct
//!   behaviour for an erasure: those rows contain personal data about the tenant
//!   being erased. The proof that a tenant was erased therefore cannot be a row
//!   in one of those tables; it is [`insert_receipt`], which deliberately has no
//!   foreign key.
//! * **Objects.** Bytes have no foreign key, so the cascade cannot reach them.
//!   This module cannot delete them either (the store adapter lives above it), so
//!   it COLLECTS the keys before the delete and hands them to the caller. Collect
//!   first, delete second, delete bytes third: that order is deliberate. A
//!   failure after the row delete leaves orphaned bytes, which the integrity pass
//!   finds and reclaims. A failure after the byte delete would leave rows
//!   advertising bytes that cannot be recovered, which no pass can fix.
//! * **Backups.** Nothing here can reach them, and no implementation honestly
//!   can: an erasure inside the live system cannot retroactively edit an
//!   encrypted archive written last week. The contract says so out loud, states
//!   the window (the backup retention), and names the remedy.
//!
//! Non-purpose: no authorization (the CLI verb requires an explicit confirmation
//! and an operator identity), no HTTP surface (there is no route that can delete
//! a tenant, by design), and no anonymisation mode (a request to keep the rows
//! without the person is a different operation with a different receipt).

use std::collections::BTreeMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

/// What an operator needs to see before they delete the wrong tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgFacts {
    pub org_id: Uuid,
    /// Slug when the org has one, else the name. Captured before the delete
    /// because it is the only human-readable handle that survives it.
    pub label: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Users, for the "is this the right tenant" question.
    pub users: i64,
}

/// Reads the org's identity, or `None` when there is no such org.
pub async fn org_facts(pool: &PgPool, org_id: Uuid) -> DbResult<Option<OrgFacts>> {
    let row: Option<(Uuid, String, Option<chrono::DateTime<chrono::Utc>>, i64)> = sqlx::query_as(
        r#"SELECT o.id,
                  coalesce(o.slug, o.name) AS label,
                  o.created_at,
                  (SELECT count(*) FROM users u WHERE u.org_id = o.id) AS users
             FROM orgs o
            WHERE o.id = $1"#,
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(org_id, label, created_at, users)| OrgFacts {
        org_id,
        label,
        created_at,
        users,
    }))
}

/// Row counts per org-scoped table — the "here is what will go" answer.
///
/// Catalog-driven on purpose: a hand-written list would fall behind the schema
/// silently, exactly the way every surface list in this lane's brief had
/// fallen behind. Tables are visited in name order so two runs print the same
/// thing.
pub async fn surface_counts(pool: &PgPool, org_id: Uuid) -> DbResult<BTreeMap<String, i64>> {
    let tables: Vec<String> = sqlx::query_scalar(
        r#"SELECT table_name FROM information_schema.columns
            WHERE table_schema = 'public' AND column_name = 'org_id'
            ORDER BY table_name"#,
    )
    .fetch_all(pool)
    .await?;
    let mut counts = BTreeMap::new();
    for table in tables {
        // The name comes from the catalog, not from input, and is quoted the
        // same way psql would.
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM \"{}\" WHERE org_id = $1",
            table.replace('"', "\"\"")
        ))
        .bind(org_id)
        .fetch_one(pool)
        .await?;
        if count > 0 {
            counts.insert(table, count);
        }
    }
    Ok(counts)
}

/// Every object key the tenant's rows claim, collected BEFORE the delete so the
/// caller can remove the bytes afterwards.
pub async fn object_keys(pool: &PgPool, org_id: Uuid) -> DbResult<Vec<String>> {
    let keys: Vec<String> = sqlx::query_scalar(
        r#"SELECT storage_key FROM attachments WHERE org_id = $1
           UNION
           SELECT storage_key FROM documents_attachments WHERE org_id = $1
           UNION
           SELECT storage_key FROM attachment_scans WHERE org_id = $1
           UNION
           SELECT storage_key FROM org_exports WHERE org_id = $1"#,
    )
    .bind(org_id)
    .fetch_all(pool)
    .await?;
    Ok(keys)
}

/// Deletes the org row, and every row the schema cascades from it. Returns the
/// number of `orgs` rows removed (0 or 1 — the caller decides what a 0 means).
pub async fn purge_rows(pool: &PgPool, org_id: Uuid) -> DbResult<u64> {
    let result = sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(org_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// One erasure, as evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub purged_org_id: Uuid,
    pub org_label: String,
    pub requested_by: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub rows_deleted: BTreeMap<String, i64>,
    pub objects_deleted: i32,
    pub objects_failed: i32,
    pub outcome: ErasureOutcome,
    pub note: Option<String>,
}

/// The receipt's verdict, in the database's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErasureOutcome {
    /// Rows and bytes are both gone.
    Completed,
    /// Rows are gone; some bytes are not. They remain findable by the integrity
    /// pass, and the receipt says how many, because a silent partial erasure is
    /// the failure this table exists to expose.
    ObjectsIncomplete,
}

impl ErasureOutcome {
    /// The token stored in `org_erasure_receipts.outcome`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::ObjectsIncomplete => "objects_incomplete",
        }
    }
}

/// Writes the receipt and returns its id. This is the erasure's proof, so it is
/// written even when the object leg failed.
pub async fn insert_receipt(pool: &PgPool, receipt: &Receipt) -> DbResult<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO org_erasure_receipts
             (id, purged_org_id, org_label, requested_by, started_at, finished_at,
              rows_deleted, objects_deleted, objects_failed, outcome, note)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(id)
    .bind(receipt.purged_org_id)
    .bind(&receipt.org_label)
    .bind(&receipt.requested_by)
    .bind(receipt.started_at)
    .bind(receipt.finished_at)
    .bind(serde_json::to_value(&receipt.rows_deleted).unwrap_or_default())
    .bind(receipt.objects_deleted)
    .bind(receipt.objects_failed)
    .bind(receipt.outcome.as_str())
    .bind(receipt.note.as_deref())
    .execute(pool)
    .await?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_tokens_match_the_migrations_check_constraint() {
        assert_eq!(ErasureOutcome::Completed.as_str(), "completed");
        assert_eq!(
            ErasureOutcome::ObjectsIncomplete.as_str(),
            "objects_incomplete"
        );
    }
}
