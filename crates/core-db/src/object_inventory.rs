//! core-db/object_inventory — one declaration of every row column that names
//! an object in the store, and the projections each reader derives from it.
//!
//! Purpose: three readers ask three different questions about the same object
//! store, and each used to carry its own hand-written list of tables.
//!
//! * the **attachment refcount** (`crate::attachments`) asks "does any OTHER
//!   row still OWN these bytes", to decide whether deleting one row may remove
//!   the object. It counts owners only, in the attachment key space.
//! * the **orphan sweep** (`crate::object_gc`) asks "does any row CLAIM this
//!   key", to decide whether an object found in the store is garbage. It must
//!   count owners, every other row that names a key in this bucket, and every
//!   key space the bucket holds — the sweep lists the WHOLE bucket, so a space
//!   it forgets is a space whose live bytes it deletes.
//! * the **tenant erasure** (`crate::erasure`) asks "which keys must the caller
//!   remove after the cascade", so its receipt and its dry run can name them.
//!
//! Three questions, one answer set. [`ENTRIES`] is that set: every column in
//! the schema that can put a key in the bucket, with the role each reader gives
//! it. Each reader is a projection of this list — [`owners`] for the refcount,
//! [`claimed`] for the sweep and for erasure — so adding a key space is one
//! edit, and every reader decides about it in the same change.
//!
//! What makes that mechanical rather than a habit:
//!
//! * `tests/object_inventory_parity.rs` reads the database's own catalogue for
//!   every column in [`KEY_COLUMNS`] and fails when a table holds one that
//!   [`ENTRIES`] has not decided;
//! * the refcount's two published lists are checked projections (they are
//!   consts because they are a published API; `attachments::tests` proves each
//!   equals its projection here);
//! * `tests/attachment_refcount.rs` keeps the older, narrower gate for the
//!   `storage_key` columns the refcount can ever bind.
//!
//! Residual, stated rather than hidden: the catalogue scan knows the column
//! NAMES in [`KEY_COLUMNS`]. A migration that invents a new name for a key
//! column is invisible to it until that name is added here; the sweep's dry run
//! over a real bucket is the backstop, and adding the name is a one-line edit.
//!
//! Non-purpose: no policy (the sweep's grace window and caps live in the
//! worker), no store calls (this module builds keys, it does not move bytes),
//! and no assumption that a key is unique — content addressing means several
//! rows legitimately share one object.

use std::collections::HashSet;

use core_store::plugin_artifact_key;
use sqlx::PgPool;
use uuid::Uuid;

use crate::DbResult;

/// The column names a row can name an object with.
///
/// The parity test scans `information_schema` for exactly these names. A new
/// column name belongs here in the same change that adds the column, or the
/// scan cannot see it.
pub const KEY_COLUMNS: &[&str] = &[
    "module_digest",
    "object_key",
    "previous_module_digest",
    "storage_key",
];

/// How a row's column becomes a key in the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyShape {
    /// The column already holds the whole key, as a `core_store` builder wrote
    /// it. Compare and select it directly.
    Stored,
    /// The column holds the sha256 hex of a plugin module; the key is
    /// `core_store::plugin_artifact_key(org_id, digest)`. The key is rebuilt in
    /// Rust rather than in SQL, so the one definition of that format stays in
    /// `core-store`.
    PluginDigest,
}

/// What each reader must do with a row that names a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    /// An owner in the ATTACHMENT key space: the refcount counts it before a
    /// row's delete may reclaim the object, and the sweep claims it.
    Owner,
    /// The sweep claims it and the attachment refcount does not. The `why` says
    /// which, and there are only two kinds: a different key space (a vault
    /// object, a plugin module, an export bundle — no attachment row can ever
    /// hold that key), or a row that records a fact about bytes it does not own
    /// (a scan verdict).
    Claimed,
    /// Neither reader counts it: the row names a key that is either already
    /// gone, or in no tenant's bucket at all. `why` says which.
    Evidence,
}

/// Which column pins a row to a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScope {
    /// `org_id` names the tenant.
    Org,
    /// `org_id` XOR `client_id` names the tenant, so a client-scoped row belongs
    /// to its client's org and carries a NULL `org_id`. Selecting `WHERE
    /// org_id = $1` alone would miss every one of those rows.
    OrgOrClient,
    /// No tenant column: the row addresses no object in any tenant's bucket.
    Global,
}

/// One column in the schema that can name an object, with what each reader owes
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEntry {
    /// Table holding the rows.
    pub table: &'static str,
    /// The key-holding column (one of [`KEY_COLUMNS`]).
    pub column: &'static str,
    /// How the column becomes a key.
    pub shape: KeyShape,
    /// What the readers do with it.
    pub role: KeyRole,
    /// How the row is attributed to a tenant.
    pub scope: KeyScope,
    /// Why this entry is in the list, in the words of the migration that made
    /// it. The reason is what stops a later reader from "tidying" an entry out.
    pub why: &'static str,
}

/// Every row column that can name an object in the store.
///
/// Order is deliberate: the attachment space first (its owners, then the rows
/// that merely name keys in it), then the other spaces, then the rows that
/// count for nobody. Two readers keep that order in their output.
pub const ENTRIES: &[KeyEntry] = &[
    KeyEntry {
        table: "attachments",
        column: "storage_key",
        shape: KeyShape::Stored,
        role: KeyRole::Owner,
        scope: KeyScope::Org,
        why: "0024 — ticket, ticket-comment and inbound-email files, written by \
              core-store's attachment_key into the per-org content-addressed space",
    },
    KeyEntry {
        table: "documents_attachments",
        column: "storage_key",
        shape: KeyShape::Stored,
        role: KeyRole::Owner,
        scope: KeyScope::Org,
        why: "0089 — document files on the same content address, which is why a \
              ticket-side delete must count this table before reclaiming bytes",
    },
    KeyEntry {
        table: "attachment_scans",
        column: "storage_key",
        shape: KeyShape::Stored,
        role: KeyRole::Claimed,
        scope: KeyScope::Org,
        why: "0139 — the verdict's storage_key is the admission key for \
              clean/skipped bytes and the quarantine key for infected ones, so \
              these rows name objects in this bucket. Not an owner: the row is \
              evidence about the object and outlives the bytes by design, which \
              is why the refcount excludes it and the sweep claims it",
    },
    KeyEntry {
        table: "org_exports",
        column: "storage_key",
        shape: KeyShape::Stored,
        role: KeyRole::Claimed,
        scope: KeyScope::Org,
        why: "0089 — export bundles under org/<org>/exports/…, a different key \
              space from attachments but the same bucket, so the sweep must \
              claim them; the refcount excludes them because no export key can \
              equal an attachment key",
    },
    KeyEntry {
        table: "secret_records",
        column: "object_key",
        shape: KeyShape::Stored,
        role: KeyRole::Claimed,
        scope: KeyScope::OrgOrClient,
        why: "0078 — sealed vault document ciphertext under \
              org/<org>/secrets/<ciphertext sha256>. Its own refcount is \
              secret_vault::count_other_file_refs; the attachment refcount must \
              not ask it (a vault key is never an attachment key), and the sweep \
              must, or a delete pass frees ciphertext a reveal still needs. The \
              scope is org-or-client because a client-scoped record carries \
              client_id and a NULL org_id",
    },
    KeyEntry {
        table: "plugin_installs",
        column: "module_digest",
        shape: KeyShape::PluginDigest,
        role: KeyRole::Claimed,
        scope: KeyScope::Org,
        why: "0076/0156 — the install's pinned module under \
              org/<org>/plugins/<digest> (core_store::plugin_artifact_key). \
              Content-addressed, so two installs of one module share it; a \
              revoke is a status and keeps the bytes, which is exactly why the \
              sweep must claim them",
    },
    KeyEntry {
        table: "plugin_installs",
        column: "previous_module_digest",
        shape: KeyShape::PluginDigest,
        role: KeyRole::Claimed,
        scope: KeyScope::Org,
        why: "0156 — the rollback target's digest. The old bytes stay in the \
              store on purpose so a rollback is a pointer move, so these look \
              unreferenced to a sweep that only reads the current pin",
    },
    KeyEntry {
        table: "plugin_staged_items",
        column: "module_digest",
        shape: KeyShape::PluginDigest,
        role: KeyRole::Claimed,
        scope: KeyScope::Org,
        why: "0079/0156 — the module admitted while the row was staged, before \
              any install exists. The bytes are in the org's bucket and a \
              pending row is not an install, so the install table alone does \
              not cover them",
    },
    KeyEntry {
        table: "plugin_versions",
        column: "module_digest",
        shape: KeyShape::PluginDigest,
        role: KeyRole::Evidence,
        scope: KeyScope::Global,
        why: "0076/0156 — the global registry's digest. The registry names a \
              module; each tenant admits it into ITS bucket under its own org. \
              No tenant key can be derived from a registry row, and nothing \
              in the bucket is addressed by it",
    },
    KeyEntry {
        table: "object_integrity_findings",
        column: "storage_key",
        shape: KeyShape::Stored,
        role: KeyRole::Evidence,
        scope: KeyScope::Org,
        why: "0146 — a past pass's evidence about a key: orphan bytes already \
              deleted, or bytes already missing. Counting it would keep a dead \
              key alive forever and protect any later upload at that address \
              from ever being reclaimed, which is why neither reader counts it",
    },
];

/// The entries whose rows name bytes a tenant still needs: owners and claims.
///
/// Both the sweep and the erasure read this projection. The sweep asks it of
/// the whole bucket; the erasure asks it of one tenant.
#[must_use]
pub fn claimed_in(entries: &[KeyEntry]) -> Vec<&KeyEntry> {
    entries
        .iter()
        .filter(|entry| entry.role != KeyRole::Evidence)
        .collect()
}

/// The entries the attachment refcount counts: owners in the attachment space.
#[must_use]
pub fn owners_in(entries: &[KeyEntry]) -> Vec<&KeyEntry> {
    entries
        .iter()
        .filter(|entry| entry.role == KeyRole::Owner)
        .collect()
}

/// The `storage_key` holders the refcount does not count. Scoped to that
/// column on purpose: the refcount binds a `storage_key`-shaped key, so a
/// column of another shape is outside its universe by construction, and listing
/// it here would make its published excluded-list claim something it does not
/// mean.
#[must_use]
pub fn storage_key_uncounted_in(entries: &[KeyEntry]) -> Vec<&KeyEntry> {
    entries
        .iter()
        .filter(|entry| entry.column == "storage_key" && entry.role != KeyRole::Owner)
        .collect()
}

/// [`claimed_in`] over the live declaration.
#[must_use]
pub fn claimed() -> Vec<&'static KeyEntry> {
    claimed_in(ENTRIES)
}

/// [`owners_in`] over the live declaration.
#[must_use]
pub fn owners() -> Vec<&'static KeyEntry> {
    owners_in(ENTRIES)
}

/// [`storage_key_uncounted_in`] over the live declaration.
#[must_use]
pub fn storage_key_uncounted() -> Vec<&'static KeyEntry> {
    storage_key_uncounted_in(ENTRIES)
}

impl KeyEntry {
    /// Keeps rows whose column actually holds a key: `module_digest` is `''` for
    /// a declarative-only install, `object_key` is NULL until a vault record has
    /// a file, and an empty string is never a valid key.
    #[must_use]
    pub fn validity_sql(&self) -> String {
        format!("{} IS NOT NULL AND {} <> ''", self.column, self.column)
    }

    /// The tenant predicate, for a caller that binds the org at `$param`.
    #[must_use]
    pub fn scope_sql(&self, param: usize) -> String {
        match self.scope {
            KeyScope::Org => format!("org_id = ${param}"),
            KeyScope::OrgOrClient => format!(
                "(org_id = ${param} OR client_id IN \
                 (SELECT id FROM clients WHERE org_id = ${param}))"
            ),
            // Not reachable through the readers: `claimed()` drops Global
            // entries. Kept total so a mis-wired caller fails a test, not a
            // tenant.
            KeyScope::Global => "false".to_string(),
        }
    }

    /// `SELECT` for this table's rows: the key material and the row's org.
    ///
    /// `org` narrows to one tenant; `None` is the whole bucket. The key is
    /// rebuilt from the material in Rust ([`KeyEntry::build_key`]) rather than
    /// in SQL, so a key format lives in exactly one place.
    #[must_use]
    pub fn select_sql(&self, org: Option<usize>) -> String {
        let mut sql = format!(
            "SELECT {} AS key_material, org_id FROM {} WHERE {}",
            self.column,
            self.table,
            self.validity_sql()
        );
        if let Some(param) = org {
            sql.push_str(" AND ");
            sql.push_str(&self.scope_sql(param));
        }
        sql.push_str(" ORDER BY org_id");
        sql
    }

    /// One `UNION` arm for the erasure: the key alone, no org needed because
    /// the caller already knows which tenant it is erasing.
    #[must_use]
    pub fn union_arm_sql(&self, org_param: usize) -> String {
        format!(
            "SELECT {} FROM {} WHERE {} AND {}",
            self.column,
            self.table,
            self.validity_sql(),
            self.scope_sql(org_param)
        )
    }

    /// Turns one row's key material into a storage key. `None` when the shape
    /// needs an org the row does not carry (a plugin digest on a global row),
    /// which the readers treat as "claims nothing".
    #[must_use]
    pub fn build_key(&self, material: &str, org_id: Option<Uuid>) -> Option<String> {
        match self.shape {
            KeyShape::Stored => (!material.is_empty()).then(|| material.to_string()),
            KeyShape::PluginDigest => {
                let org = org_id?;
                (!material.is_empty())
                    .then(|| plugin_artifact_key(org, &material.trim().to_ascii_lowercase()))
            }
        }
    }

    /// The part of `key` this row's column would hold, for the reverse lookup a
    /// missing-bytes report needs.
    fn material<'a>(&self, key: &'a str) -> Option<&'a str> {
        match self.shape {
            KeyShape::Stored => Some(key),
            KeyShape::PluginDigest => key.rsplit('/').next(),
        }
    }

    /// Every key this table's rows name, with the org each row is attributed to.
    ///
    /// One query per entry, deliberately: a `UNION` across shapes would have to
    /// spell the plugin key format in SQL, and that format belongs in
    /// `core-store` only.
    ///
    /// The pool must bypass row-level security. Every table here carries an
    /// org-isolation policy, and a pass that ran as the RLS-constrained
    /// application role would see no rows, conclude every object is unreferenced
    /// and delete them. The worker's pool and the erasure verb both connect as
    /// the owner for this reason.
    pub async fn keys(&self, pool: &PgPool) -> DbResult<Vec<(String, Option<Uuid>)>> {
        let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(&self.select_sql(None))
            .fetch_all(pool)
            .await?;
        Ok(self.rebuild(rows))
    }

    /// The same, for one tenant's rows.
    pub async fn keys_for_org(
        &self,
        pool: &PgPool,
        org_id: Uuid,
    ) -> DbResult<Vec<(String, Option<Uuid>)>> {
        let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(&self.select_sql(Some(1)))
            .bind(org_id)
            .fetch_all(pool)
            .await?;
        Ok(self.rebuild(rows))
    }

    /// The rows that name any of `keys`, so a finding can say which table made
    /// the claim without one query per key.
    pub async fn rows_naming(
        &self,
        pool: &PgPool,
        keys: &[String],
    ) -> DbResult<Vec<(String, Option<Uuid>)>> {
        let wanted: HashSet<&str> = keys.iter().map(String::as_str).collect();
        let materials: Vec<&str> = keys
            .iter()
            .filter_map(|key| self.material(key))
            .filter(|material| !material.is_empty())
            .collect();
        if materials.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, Option<Uuid>)> = sqlx::query_as(&format!(
            "SELECT {} AS key_material, org_id FROM {} WHERE {} = ANY($1)",
            self.column, self.table, self.column
        ))
        .bind(&materials)
        .fetch_all(pool)
        .await?;
        Ok(self
            .rebuild(rows)
            .into_iter()
            .filter(|(key, _)| wanted.contains(key.as_str()))
            .collect())
    }

    fn rebuild(&self, rows: Vec<(String, Option<Uuid>)>) -> Vec<(String, Option<Uuid>)> {
        rows.into_iter()
            .filter_map(|(material, org_id)| {
                self.build_key(&material, org_id).map(|key| (key, org_id))
            })
            .collect()
    }
}

/// One `UNION` over every claimed entry whose column holds the key outright.
///
/// This is the erasure's single statement: one round trip for a tenant's whole
/// stored-key set, whatever the number of tables. Entries whose key is rebuilt
/// from a digest are NOT here — the caller asks those separately and builds the
/// key in Rust.
#[must_use]
pub fn stored_union(entries: &[&KeyEntry], org_param: usize) -> String {
    let arms: Vec<String> = entries
        .iter()
        .filter(|entry| entry.shape == KeyShape::Stored)
        .map(|entry| entry.union_arm_sql(org_param))
        .collect();
    assert!(
        !arms.is_empty(),
        "the declaration has no stored-key entry, which the erasure statement cannot be empty of"
    );
    arms.join(" UNION ")
}

/// The claimed entries whose key must be rebuilt from a digest.
#[must_use]
pub fn digest_entries_in<'a>(entries: &[&'a KeyEntry]) -> Vec<&'a KeyEntry> {
    entries
        .iter()
        .copied()
        .filter(|entry| entry.shape == KeyShape::PluginDigest)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_column(entries: &[KeyEntry]) -> Vec<(&'static str, &'static str)> {
        entries.iter().map(|e| (e.table, e.column)).collect()
    }

    /// Every declared column name is one the catalogue scan looks for, and
    /// every name the scan looks for is used by at least one entry. Either half
    /// failing means the scan and the declaration disagree about what a key
    /// column is.
    #[test]
    fn the_scan_vocabulary_and_the_declaration_agree_both_ways() {
        for entry in ENTRIES {
            assert!(
                KEY_COLUMNS.contains(&entry.column),
                "{} declares column {} but the catalogue scan does not look for \
                 that name, so a new table using it would go unseen",
                entry.table,
                entry.column
            );
        }
        for column in KEY_COLUMNS {
            assert!(
                ENTRIES.iter().any(|entry| entry.column == *column),
                "the scan looks for {column} and no entry uses it: either the \
                 declaration lost a table or the name is stale"
            );
        }
    }

    #[test]
    fn every_entry_states_why_it_is_in_the_inventory() {
        for entry in ENTRIES {
            assert!(
                !entry.why.trim().is_empty(),
                "{}.{} is declared with no reason",
                entry.table,
                entry.column
            );
        }
    }

    #[test]
    fn no_column_is_declared_twice() {
        let mut seen = table_column(ENTRIES);
        let len = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            len,
            "a (table, column) pair is declared twice: {:?}",
            table_column(ENTRIES)
        );
    }

    /// A global row addresses no tenant's bucket, so it can never be a reader's
    /// claim: `claimed()` drops it, and a Global entry with a non-Evidence role
    /// would be an entry no reader could ever use.
    #[test]
    fn a_global_entry_is_evidence_and_nothing_else() {
        for entry in ENTRIES {
            if entry.scope == KeyScope::Global {
                assert_eq!(
                    entry.role,
                    KeyRole::Evidence,
                    "{}.{} is Global, so no tenant scope can reach it; it must \
                     be Evidence",
                    entry.table,
                    entry.column
                );
            }
        }
    }

    /// The refcount's arm compares the column to the bound key, so an owner
    /// must hold the key outright and be pinned by `org_id`. An owner of
    /// another shape would need a Rust-side key build inside `other_refs_sql`,
    /// which is not what that statement does.
    #[test]
    fn every_owner_is_a_stored_org_scoped_key() {
        for entry in owners() {
            assert_eq!(
                entry.shape,
                KeyShape::Stored,
                "{} is a refcount owner, so its column must hold the key",
                entry.table
            );
            assert_eq!(
                entry.scope,
                KeyScope::Org,
                "{} is a refcount owner, so the refcount's own arm must reach \
                 it with org_id = $1",
                entry.table
            );
        }
    }

    /// The two projections are disjoint and together are exactly the
    /// `storage_key` universe.
    #[test]
    fn the_refcounts_two_lists_partition_the_storage_key_columns() {
        let owners = owners();
        let uncounted = storage_key_uncounted();
        for entry in &owners {
            assert!(
                !uncounted
                    .iter()
                    .any(|other| other.table == entry.table && other.column == entry.column),
                "{} is counted and excluded at once",
                entry.table
            );
        }
        let counted = owners.len() + uncounted.len();
        let expected = ENTRIES
            .iter()
            .filter(|entry| entry.column == "storage_key")
            .count();
        assert_eq!(
            counted, expected,
            "every storage_key entry must be counted or excluded"
        );
    }

    /// A claim the sweep cannot query is a table whose objects it deletes. The
    /// projection must cover the owners, in both directions.
    #[test]
    fn the_sweeps_projection_covers_every_owner() {
        let claimed = claimed();
        for entry in owners() {
            assert!(
                claimed
                    .iter()
                    .any(|claim| claim.table == entry.table && claim.column == entry.column),
                "{} owns attachment bytes and the sweep would not claim them",
                entry.table
            );
        }
        assert!(
            claimed.len() >= owners().len(),
            "the sweep's set is a superset of the refcount's"
        );
    }
}
