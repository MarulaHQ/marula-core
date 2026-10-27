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

use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgPool;
use uuid::Uuid;

use crate::{object_inventory, DbResult};

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
///
/// Derived from [`crate::object_inventory`] — the one declaration of every
/// column that names an object — so a key space added there is collected here in
/// the same change. It used to be a hand-written `UNION` over four tables, which
/// silently missed the sealed vault ciphertext (`secret_records.object_key`) and
/// every plugin module (`plugin_installs.module_digest`, its rollback target, and
/// the staged digest). Those rows cascade with the tenant, so only the collection
/// can name their bytes.
///
/// Two statements, not one per table: the tables whose column holds the key
/// outright share one `UNION`, and the digest-addressed plugin entries are asked
/// separately because their key is rebuilt in Rust by `core-store`'s builder
/// rather than spelled in SQL. The set is deduplicated and ordered, which is what
/// the old `UNION` did too: one object shared by two rows is one key.
pub async fn object_keys(pool: &PgPool, org_id: Uuid) -> DbResult<Vec<String>> {
    let entries = object_inventory::claimed();
    let mut keys: BTreeSet<String> = BTreeSet::new();

    let stored: Vec<&object_inventory::KeyEntry> = entries
        .iter()
        .copied()
        .filter(|entry| entry.shape == object_inventory::KeyShape::Stored)
        .collect();
    if !stored.is_empty() {
        let rows: Vec<String> = sqlx::query_scalar(&object_inventory::stored_union(&stored, 1))
            .bind(org_id)
            .fetch_all(pool)
            .await?;
        keys.extend(rows.into_iter().filter(|key| !key.is_empty()));
    }

    for entry in object_inventory::digest_entries_in(&entries) {
        for (key, _org) in entry.keys_for_org(pool, org_id).await? {
            keys.insert(key);
        }
    }

    Ok(keys.into_iter().collect())
}

/// The tables [`object_keys`] asks, in declaration order — the erasure's key
/// space as a list, so a test can hold it against the refcount's owners and the
/// sweep's claims instead of trusting a hand-written `UNION`.
#[must_use]
pub fn object_key_tables() -> Vec<&'static str> {
    object_inventory::claimed()
        .iter()
        .map(|entry| entry.table)
        .collect()
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
    use crate::object_inventory::{KeyEntry, KeyShape, ENTRIES};

    #[test]
    fn outcome_tokens_match_the_migrations_check_constraint() {
        assert_eq!(ErasureOutcome::Completed.as_str(), "completed");
        assert_eq!(
            ErasureOutcome::ObjectsIncomplete.as_str(),
            "objects_incomplete"
        );
    }

    fn entry(table: &str, column: &str) -> &'static KeyEntry {
        ENTRIES
            .iter()
            .find(|entry| entry.table == table && entry.column == column)
            .unwrap_or_else(|| panic!("{table}.{column} is not in the declaration"))
    }

    /// Every table that claims bytes for a tenant is asked by the erasure. A key
    /// space that the sweep protects and the erasure forgets is a tenant whose
    /// receipt under-reports what it left behind.
    #[test]
    fn the_erasure_asks_every_table_that_claims_bytes() {
        let erased = object_key_tables();
        for claim in object_inventory::claimed() {
            assert!(
                erased.contains(&claim.table),
                "{} claims bytes ({}) and the erasure does not collect them",
                claim.table,
                claim.why
            );
        }
    }

    /// The stored half of the collection is one statement, and it names each
    /// stored claim's own column and table. A rewrite that spells a literal list
    /// instead of the projection fails here.
    #[test]
    fn the_stored_union_names_every_stored_claim_by_its_own_column() {
        let entries = object_inventory::claimed();
        let sql = object_inventory::stored_union(&entries, 1);
        for claim in entries
            .iter()
            .filter(|claim| claim.shape == KeyShape::Stored)
        {
            assert!(
                sql.contains(&format!(
                    "SELECT {} FROM {} WHERE",
                    claim.column, claim.table
                )),
                "the erasure statement does not ask {}.{}: {sql}",
                claim.table,
                claim.column
            );
        }
        for claim in entries
            .iter()
            .filter(|claim| claim.shape == KeyShape::PluginDigest)
        {
            assert!(
                !sql.contains(claim.table),
                "{} is digest-addressed and does not belong in the stored union; \
                 its key is rebuilt in Rust: {sql}",
                claim.table
            );
        }
    }

    /// A vault record may be client-scoped: `org_id` NULL and `client_id` set.
    /// The cascade deletes those rows through `clients`, so a collection that
    /// filters on `org_id = $1` alone would leave their ciphertext behind.
    #[test]
    fn a_client_scoped_vault_record_is_still_collected() {
        let sql = entry("secret_records", "object_key").union_arm_sql(1);
        assert!(
            sql.contains("object_key") && sql.contains("FROM secret_records"),
            "{sql}"
        );
        assert!(
            sql.contains("client_id IN (SELECT id FROM clients WHERE org_id = $1)"),
            "the vault's scope is org-or-client, so a client-scoped record must be \
             collected through its client's org: {sql}"
        );
        assert!(sql.contains("object_key <> ''"), "{sql}");
    }

    /// A plugin module's key is rebuilt, not stored: the erasure asks the digest
    /// column and `core-store` builds the address, so the format stays in one
    /// place.
    #[test]
    fn a_plugin_module_is_collected_through_its_digest() {
        let install = entry("plugin_installs", "module_digest");
        assert_eq!(install.shape, KeyShape::PluginDigest);
        let key = install
            .build_key(&"b".repeat(64), Some(Uuid::nil()))
            .expect("a digest and an org make a key");
        assert_eq!(
            key,
            core_store::plugin_artifact_key(Uuid::nil(), &"b".repeat(64))
        );
        assert!(
            entry("plugin_installs", "previous_module_digest")
                .select_sql(Some(1))
                .contains("previous_module_digest <> ''"),
            "a rollback target's bytes are live, so the erasure must collect them"
        );
    }
}
