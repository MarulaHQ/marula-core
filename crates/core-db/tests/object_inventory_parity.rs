//! Parity between the one object-key declaration and each of its readers
//! (Wave 58 lane `p58-object-inventory`, F61; extended by Wave 59 lane
//! `p59-inventory`, F66/F67).
//!
//! Purpose: three readers ask three different questions about one object store,
//! and each used to carry its own hand-written list of tables:
//!
//! * the attachment refcount (`attachments::ATTACHMENT_REF_TABLES` plus
//!   `ATTACHMENT_KEY_TABLES_NOT_REFERENCES`) decides whether deleting a row may
//!   reclaim the object;
//! * the orphan sweep (`object_gc::sources`) decides whether an object found in
//!   the store is garbage — and it lists the WHOLE bucket, so a key space it does
//!   not claim is a key space whose live bytes a delete pass frees;
//! * the tenant erasure (`erasure::object_keys`) decides what the receipt and the
//!   dry run name as the tenant's stored bytes.
//!
//! They are different questions, so [`core_db::object_inventory`] declares every
//! key column once with the role each reader gives it, and each reader is a
//! projection of that declaration. This suite holds them together:
//!
//! * a key column in the schema that no entry decided fails;
//! * an owner the sweep does not claim fails;
//! * a `storage_key` claim the refcount neither counts nor excludes fails;
//! * a claim the erasure does not ask fails;
//! * a declared entry the schema no longer has fails.
//!
//! The catalogue is the database, not the source: a migration that adds a key
//! column fails here until the declaration decides it, which is the point. The
//! synthetic cases inject a table, a shortened list and a widened one, so each
//! failure mode is proved rather than assumed; the erasure case below seeds every
//! key space a tenant can own and names, key by key, what must be collected.
//!
//! Non-purpose: the sweep's policy (grace, caps, dry-run) is
//! `crates/worker/src/object_gc_sweep.rs`; the refcount's behaviour is
//! `tests/attachment_refcount.rs`. Seed slots 5900–5919 are this suite's private
//! range.

mod common;

use common::{fixed_uuid, test_db};
use core_db::attachments::{ATTACHMENT_KEY_TABLES_NOT_REFERENCES, ATTACHMENT_REF_TABLES};
use core_db::object_inventory::{KeyEntry, KeyRole, ENTRIES, KEY_COLUMNS};
use core_db::{erasure, object_gc};
use sqlx::PgConnection;

/// The `(table, column)` pairs the database holds that a row could name an
/// object with: the catalogue's answer to "which columns can put a key in the
/// object store".
async fn key_columns(conn: &mut PgConnection) -> Vec<(String, String)> {
    let names: Vec<String> = KEY_COLUMNS.iter().map(|name| (*name).to_string()).collect();
    sqlx::query_as(
        r#"SELECT c.table_name::text, c.column_name::text
             FROM information_schema.columns c
             JOIN information_schema.tables t
               ON t.table_schema = c.table_schema
              AND t.table_name = c.table_name
            WHERE c.table_schema = 'public'
              AND c.column_name = ANY($1)
              AND t.table_type = 'BASE TABLE'
            ORDER BY c.table_name, c.column_name"#,
    )
    .bind(names)
    .fetch_all(&mut *conn)
    .await
    .expect("catalogue read")
}

fn pair(table: &str, column: &str) -> String {
    format!("{table}.{column}")
}

/// What the declaration and its readers get wrong, if anything, for a given
/// catalogue and set of projections.
///
/// Pure on purpose: the database test feeds it the real catalogue and the real
/// projections, and the synthetic cases feed it one mutated table or list so the
/// failure modes are proved rather than assumed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Gaps {
    /// Key columns in the schema that no entry decided about.
    undecided: Vec<String>,
    /// Declared entries the schema no longer has, so a reader queries a column
    /// that is not there.
    phantom: Vec<String>,
    /// Owners the sweep does not claim: it would call their live bytes orphans
    /// and delete them.
    unprotected: Vec<String>,
    /// `storage_key` claims the refcount neither counts nor excludes, so no
    /// reader can tell which question they belong to.
    stray: Vec<String>,
    /// Claims the erasure does not ask, so the receipt under-reports what it
    /// left behind.
    unerased: Vec<String>,
    /// Entries that were decided without a reason.
    unreasoned: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn gaps(
    catalogue: &[(String, String)],
    declared: &[KeyEntry],
    sweep: &[(&str, &str)],
    owners: &[&str],
    excluded: &[&str],
    erased: &[&str],
) -> Gaps {
    let catalogue_pairs: Vec<String> = catalogue
        .iter()
        .map(|(table, column)| pair(table, column))
        .collect();
    let declared_pairs: Vec<String> = declared
        .iter()
        .map(|entry| pair(entry.table, entry.column))
        .collect();
    let decided = |table: &str| owners.contains(&table) || excluded.contains(&table);

    Gaps {
        undecided: catalogue_pairs
            .iter()
            .filter(|held| !declared_pairs.contains(held))
            .cloned()
            .collect(),
        phantom: declared_pairs
            .iter()
            .filter(|declared| !catalogue_pairs.contains(declared))
            .cloned()
            .collect(),
        unprotected: declared
            .iter()
            .filter(|entry| entry.role == KeyRole::Owner)
            .filter(|entry| {
                !sweep
                    .iter()
                    .any(|(table, column)| *table == entry.table && *column == entry.column)
            })
            .map(|entry| pair(entry.table, entry.column))
            .collect(),
        stray: declared
            .iter()
            .filter(|entry| entry.role != KeyRole::Evidence && entry.column == "storage_key")
            .filter(|entry| !decided(entry.table))
            .map(|entry| entry.table.to_string())
            .collect(),
        unerased: declared
            .iter()
            .filter(|entry| entry.role != KeyRole::Evidence)
            .filter(|entry| !erased.contains(&entry.table))
            .map(|entry| pair(entry.table, entry.column))
            .collect(),
        unreasoned: declared
            .iter()
            .filter(|entry| entry.why.trim().is_empty())
            .map(|entry| pair(entry.table, entry.column))
            .collect(),
    }
}

fn declared_entries() -> Vec<KeyEntry> {
    ENTRIES.to_vec()
}

fn sweep_entries() -> Vec<(&'static str, &'static str)> {
    object_gc::sources()
        .iter()
        .map(|entry| (entry.table, entry.column))
        .collect()
}

fn owner_tables() -> Vec<&'static str> {
    ATTACHMENT_REF_TABLES
        .iter()
        .map(|(table, _)| *table)
        .collect()
}

fn excluded_tables() -> Vec<&'static str> {
    ATTACHMENT_KEY_TABLES_NOT_REFERENCES
        .iter()
        .map(|(table, _)| *table)
        .collect()
}

fn erased_tables() -> Vec<&'static str> {
    erasure::object_key_tables()
}

/// The live tree: every key column is decided once, every owner is claimed by the
/// sweep, every `storage_key` claim is decided by the refcount, and every claim
/// is collected by the erasure.
#[tokio::test]
async fn the_two_inventories_cover_the_same_key_space() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let catalogue = key_columns(&mut conn).await;

    let declared = declared_entries();
    let found = gaps(
        &catalogue,
        &declared,
        &sweep_entries(),
        &owner_tables(),
        &excluded_tables(),
        &erased_tables(),
    );
    assert_eq!(
        found,
        Gaps::default(),
        "the object-key declaration and its readers disagree with the schema. \
         undecided = key columns no entry decided (add each to \
         object_inventory::ENTRIES with the role each reader gives it); \
         phantom = entries whose column no longer exists; \
         unprotected = owners the sweep would delete the live bytes of; \
         stray = storage_key claims the refcount neither counts nor excludes; \
         unerased = claims the erasure does not collect; \
         unreasoned = entries with no why. Catalogue: {catalogue:?}"
    );
}

/// The catalogue half, proved: a table added to the schema with a key column and
/// no declaration entry is reported as undecided. Runs in a transaction that is
/// rolled back, so the probe table cannot reach another test or another binary
/// against the same database.
#[tokio::test]
async fn a_new_key_holding_table_is_reported_until_both_inventories_decide() {
    let db = test_db().await;
    let mut tx = db.admin.begin().await.expect("probe transaction");
    sqlx::query(
        "CREATE TABLE p59_probe_key_columns (
             id uuid PRIMARY KEY,
             org_id uuid NOT NULL,
             object_key text NOT NULL
         )",
    )
    .execute(&mut *tx)
    .await
    .expect("probe table");

    // Read on the transaction's own connection: uncommitted DDL is visible to
    // the transaction that wrote it and to nobody else.
    let catalogue = key_columns(&mut tx).await;
    let declared = declared_entries();
    let found = gaps(
        &catalogue,
        &declared,
        &sweep_entries(),
        &owner_tables(),
        &excluded_tables(),
        &erased_tables(),
    );

    tx.rollback().await.expect("probe rollback");

    assert!(
        catalogue
            .iter()
            .any(|(table, column)| table == "p59_probe_key_columns" && column == "object_key"),
        "the probe table must be in the catalogue the checker reads; \
         catalogue was {catalogue:?}"
    );
    assert_eq!(
        found.undecided,
        vec!["p59_probe_key_columns.object_key".to_string()],
        "a key column the declaration never decided must be the one reported"
    );
    assert!(
        found.unprotected.is_empty()
            && found.phantom.is_empty()
            && found.stray.is_empty()
            && found.unerased.is_empty()
            && found.unreasoned.is_empty(),
        "adding an undecided column must not disturb the other verdicts: {found:?}"
    );
}

/// The superset half, proved from the other direction: drop one owner from the
/// sweep's projection (a stand-in for the edit a lane makes when it adds a table
/// to the refcount and forgets the sweep) and the checker names exactly that
/// column.
#[tokio::test]
async fn an_owner_the_sweep_forgets_is_reported() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let catalogue = key_columns(&mut conn).await;

    let dropped = "documents_attachments";
    assert!(
        owner_tables().contains(&dropped),
        "this case is about losing an owner; {dropped} must still be one"
    );
    let shortened: Vec<(&str, &str)> = sweep_entries()
        .into_iter()
        .filter(|(table, _)| *table != dropped)
        .collect();
    assert_eq!(
        shortened.len(),
        sweep_entries().len() - 1,
        "the shortened list must have actually dropped the owner"
    );

    let declared = declared_entries();
    let found = gaps(
        &catalogue,
        &declared,
        &shortened,
        &owner_tables(),
        &excluded_tables(),
        &erased_tables(),
    );
    assert_eq!(
        found.unprotected,
        vec![format!("{dropped}.storage_key")],
        "the checker must name the owner the sweep stopped claiming"
    );
    assert!(found.undecided.is_empty(), "{found:?}");
}

/// A sweep source the refcount has decided nothing about is reported as stray,
/// and is caught before it becomes a query error by the phantom check only if the
/// table does not exist — so both are asserted here.
#[tokio::test]
async fn a_source_outside_the_refcounts_universe_is_reported() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let catalogue = key_columns(&mut conn).await;

    let mut extended = declared_entries();
    extended.push(KeyEntry {
        table: "p59_table_the_refcount_never_decided",
        column: "storage_key",
        shape: core_db::object_inventory::KeyShape::Stored,
        role: KeyRole::Owner,
        scope: core_db::object_inventory::KeyScope::Org,
        why: "a synthetic owner, injected by the parity suite to prove the checker fails",
    });

    let found = gaps(
        &catalogue,
        &extended,
        &sweep_entries(),
        &owner_tables(),
        &excluded_tables(),
        &erased_tables(),
    );
    assert_eq!(
        found.stray,
        vec!["p59_table_the_refcount_never_decided".to_string()],
        "a storage_key claim outside both refcount lists must be reported"
    );
    assert_eq!(
        found.phantom,
        vec!["p59_table_the_refcount_never_decided.storage_key".to_string()],
        "a declared entry the schema does not have must be reported too"
    );
}

/// The erasure is the reader that must not fall behind: a claim it does not
/// collect is a tenant whose receipt under-reports the bytes it left behind. Drop
/// the vault space from the erasure's list and the checker names it.
#[tokio::test]
async fn a_key_space_the_erasure_forgets_is_reported() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let catalogue = key_columns(&mut conn).await;

    let dropped = "secret_records";
    assert!(
        erased_tables().contains(&dropped),
        "the erasure must already collect the vault's ciphertext keys"
    );
    let shortened: Vec<&str> = erased_tables()
        .into_iter()
        .filter(|table| *table != dropped)
        .collect();

    let declared = declared_entries();
    let found = gaps(
        &catalogue,
        &declared,
        &sweep_entries(),
        &owner_tables(),
        &excluded_tables(),
        &shortened,
    );
    assert_eq!(
        found.unerased,
        vec![format!("{dropped}.object_key")],
        "the checker must name the key space the erasure stopped collecting"
    );
}

/// The erasure, end to end: one org holding a row in every key space, and the
/// collected set is exactly those keys — no more, no fewer. Also proves the
/// dedup: two attachment rows sharing one content address are one key.
#[tokio::test]
async fn the_erasure_collects_every_key_space_a_tenant_owns() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");

    let org = fixed_uuid(5_900);
    let client = fixed_uuid(5_901);
    let email = fixed_uuid(5_902);
    let document = fixed_uuid(5_903);
    let user = fixed_uuid(5_904);
    let org_folder = fixed_uuid(5_905);
    let client_folder = fixed_uuid(5_906);
    let catalog = fixed_uuid(5_907);
    let install = fixed_uuid(5_908);
    let staged = fixed_uuid(5_909);
    let export = fixed_uuid(5_910);

    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'Purge Me', 'purge-me')")
        .bind(org)
        .execute(&mut *conn)
        .await
        .expect("org insert");
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Purge Client')")
        .bind(client)
        .bind(org)
        .execute(&mut *conn)
        .await
        .expect("client insert");
    sqlx::query(
        "INSERT INTO users (id, org_id, email, display_name, password_hash)
         VALUES ($1, $2, 'purge@example.test', 'Purge User', 'x')",
    )
    .bind(user)
    .bind(org)
    .execute(&mut *conn)
    .await
    .expect("user insert");

    // The attachment space: two ticket/email rows sharing ONE content address,
    // and a document row on the same address family.
    let shared = "a".repeat(64);
    sqlx::query(
        "INSERT INTO email_messages (id, org_id, direction, rfc_message_id, from_address,
                                     scan_status, transport)
         VALUES ($1, $2, 'inbound', '<purge@example.test>', 'client@example.test',
                 'clean', 'webhook/test')",
    )
    .bind(email)
    .bind(org)
    .execute(&mut *conn)
    .await
    .expect("email insert");
    for id in [fixed_uuid(5_911), fixed_uuid(5_912)] {
        sqlx::query(
            "INSERT INTO attachments (id, org_id, kind, email_message_id, filename,
                                      content_type, size_bytes, sha256, storage_key)
             VALUES ($1, $2, 'email_message', $3, 'invoice.pdf', 'application/pdf',
                     12, $4, $5)",
        )
        .bind(id)
        .bind(org)
        .bind(email)
        .bind(&shared)
        .bind(core_store::attachment_key(org, &shared))
        .execute(&mut *conn)
        .await
        .expect("attachment insert");
    }
    sqlx::query("INSERT INTO documents (id, org_id, title) VALUES ($1, $2, 'Purge Doc')")
        .bind(document)
        .bind(org)
        .execute(&mut *conn)
        .await
        .expect("document insert");
    let document_sha = "b".repeat(64);
    sqlx::query(
        "INSERT INTO documents_attachments (id, org_id, document_id, filename, content_type,
                                            size_bytes, sha256, storage_key)
         VALUES ($1, $2, $3, 'spec.pdf', 'application/pdf', 12, $4, $5)",
    )
    .bind(fixed_uuid(5_913))
    .bind(org)
    .bind(document)
    .bind(&document_sha)
    .bind(core_store::attachment_key(org, &document_sha))
    .execute(&mut *conn)
    .await
    .expect("document attachment insert");

    // Two of the four tables the old hand-written UNION knew, one row each.
    let verdict_sha = "c".repeat(64);
    sqlx::query(
        "INSERT INTO attachment_scans
           (org_id, sha256, status, scanner, size_bytes, content_type, filename, storage_key)
         VALUES ($1, $2, 'skipped', 'none', 12, 'text/plain', 'notes.txt', $3)",
    )
    .bind(org)
    .bind(&verdict_sha)
    .bind(core_store::attachment_key(org, &verdict_sha))
    .execute(&mut *conn)
    .await
    .expect("scan insert");
    let export_key = format!("org/{org}/exports/{export}/bundle.json");
    sqlx::query(
        "INSERT INTO org_exports (id, org_id, bundle_version, size_bytes, sha256, storage_key)
         VALUES ($1, $2, 1, 12, $3, $4)",
    )
    .bind(export)
    .bind(org)
    .bind("d".repeat(64))
    .bind(&export_key)
    .execute(&mut *conn)
    .await
    .expect("export insert");

    // The vault space: one org-scoped record and one CLIENT-scoped record, which
    // carries a NULL org_id and would be missed by a plain `org_id = $1`.
    for (folder, org_id, client_id, slug) in [
        (org_folder, Some(org), None, "documents"),
        (client_folder, None, Some(client), "documents"),
    ] {
        sqlx::query(
            "INSERT INTO secret_folders (id, org_id, client_id, slug, label, kind)
             VALUES ($1, $2, $3, $4, 'Documents', 'documents')",
        )
        .bind(folder)
        .bind(org_id)
        .bind(client_id)
        .bind(slug)
        .execute(&mut *conn)
        .await
        .expect("folder insert");
    }
    let org_vault_key = format!("org/{org}/secrets/{}", "e".repeat(64));
    let client_vault_key = format!("org/{org}/secrets/{}", "f".repeat(64));
    for (id, folder, org_id, client_id, key) in [
        (
            fixed_uuid(5_914),
            org_folder,
            Some(org),
            None,
            org_vault_key.as_str(),
        ),
        (
            fixed_uuid(5_915),
            client_folder,
            None,
            Some(client),
            client_vault_key.as_str(),
        ),
    ] {
        sqlx::query(
            "INSERT INTO secret_records (id, org_id, client_id, folder_id, kind, title,
                                         object_key, file_sha256, file_bytes)
             VALUES ($1, $2, $3, $4, 'document', $5, $6, $7, 12)",
        )
        .bind(id)
        .bind(org_id)
        .bind(client_id)
        .bind(folder)
        .bind(format!("Vault file {}", id.simple()))
        .bind(key)
        .bind("e".repeat(64))
        .execute(&mut *conn)
        .await
        .expect("vault record insert");
    }

    // The plugin space: one install with a current pin AND a rollback target,
    // and one staged import whose module was admitted before any install existed.
    sqlx::query(
        "INSERT INTO plugin_catalog (id, slug, name, publisher, interface_version)
         VALUES ($1, 'purge-plugin', 'Purge Plugin', 'test', 2)",
    )
    .bind(catalog)
    .execute(&mut *conn)
    .await
    .expect("catalog insert");
    let pin = "1".repeat(64);
    let previous = "2".repeat(64);
    sqlx::query(
        "INSERT INTO plugin_installs
           (id, org_id, catalog_id, version, status, module_digest,
            previous_version, previous_module_digest, installed_by)
         VALUES ($1, $2, $3, '1.0.0', 'enabled', $4, '0.9.0', $5, $6)",
    )
    .bind(install)
    .bind(org)
    .bind(catalog)
    .bind(&pin)
    .bind(&previous)
    .bind(user)
    .execute(&mut *conn)
    .await
    .expect("install insert");
    let staged_pin = "3".repeat(64);
    sqlx::query(
        "INSERT INTO plugin_staged_items
           (id, org_id, artifact_kind, source_kind, manifest, module_digest,
            artifact_digest, uploaded_by)
         VALUES ($1, $2, 'plugin', 'local_zip', '{\"name\":\"staged\"}'::jsonb, $3, $4, $5)",
    )
    .bind(staged)
    .bind(org)
    .bind(&staged_pin)
    .bind("4".repeat(64))
    .bind(user)
    .execute(&mut *conn)
    .await
    .expect("staged item insert");

    // A second tenant, so the collection can be shown to stop at the boundary.
    let other = fixed_uuid(5_916);
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'Keep Me', 'keep-me')")
        .bind(other)
        .execute(&mut *conn)
        .await
        .expect("other org insert");
    let other_folder = fixed_uuid(5_917);
    sqlx::query(
        "INSERT INTO secret_folders (id, org_id, slug, label, kind)
         VALUES ($1, $2, 'documents', 'Documents', 'documents')",
    )
    .bind(other_folder)
    .bind(other)
    .execute(&mut *conn)
    .await
    .expect("other folder insert");
    let other_vault_key = format!("org/{other}/secrets/{}", "9".repeat(64));
    sqlx::query(
        "INSERT INTO secret_records (id, org_id, folder_id, kind, title, object_key,
                                     file_sha256, file_bytes)
         VALUES ($1, $2, $3, 'document', 'Kept file', $4, $5, 12)",
    )
    .bind(fixed_uuid(5_918))
    .bind(other)
    .bind(other_folder)
    .bind(&other_vault_key)
    .bind("9".repeat(64))
    .execute(&mut *conn)
    .await
    .expect("other vault record");

    let mut expected = vec![
        core_store::attachment_key(org, &shared),
        core_store::attachment_key(org, &document_sha),
        core_store::attachment_key(org, &verdict_sha),
        export_key.clone(),
        org_vault_key.clone(),
        client_vault_key.clone(),
        core_store::plugin_artifact_key(org, &pin),
        core_store::plugin_artifact_key(org, &previous),
        core_store::plugin_artifact_key(org, &staged_pin),
    ];
    expected.sort();

    let keys = erasure::object_keys(&db.admin, org)
        .await
        .expect("collect the tenant's keys");
    assert_eq!(
        keys, expected,
        "the erasure must collect one key per object, from every key space: the \
         attachment space (two rows on one content address collapse to one key), \
         document attachments, scan verdicts, export bundles, the sealed vault \
         (org-scoped AND client-scoped), and plugin modules (pinned, rollback \
         target, staged)"
    );
    assert!(
        !keys.contains(&other_vault_key),
        "the collection must stop at the tenant boundary"
    );
    assert_eq!(
        keys.len(),
        9,
        "ten rows claim these objects and two share one content address: {keys:?}"
    );

    // The rows the new spaces contribute do cascade, which is why only the
    // collection can name their bytes.
    let purged = erasure::purge_rows(&db.admin, org)
        .await
        .expect("purge the tenant");
    assert_eq!(purged, 1);
    for table in [
        "secret_records",
        "secret_folders",
        "plugin_installs",
        "plugin_staged_items",
    ] {
        let left: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE org_id = $1"))
                .bind(org)
                .fetch_one(&db.admin)
                .await
                .unwrap_or_else(|e| panic!("count {table}: {e}"));
        assert_eq!(left, 0, "{table} rows must cascade with the org");
    }
    let client_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_records WHERE client_id = $1")
            .bind(client)
            .fetch_one(&db.admin)
            .await
            .expect("client-scoped rows");
    assert_eq!(
        client_rows, 0,
        "a client-scoped record has no org_id; it cascades through its client"
    );
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM secret_records WHERE org_id = $1")
        .bind(other)
        .fetch_one(&db.admin)
        .await
        .expect("other tenant rows");
    assert_eq!(kept, 1, "erasing one tenant must not touch another's vault");
}

/// The sweep sees the same claim set, on the same data: a sweep that claimed only
/// the attachment tables would call the vault's and the plugins' objects orphans.
#[tokio::test]
async fn the_sweep_claims_the_vault_and_the_plugin_modules() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");

    let org = fixed_uuid(6_000);
    let folder = fixed_uuid(6_001);
    let catalog = fixed_uuid(6_002);
    let digest = "8".repeat(64);

    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, 'Sweep Me', 'sweep-me')")
        .bind(org)
        .execute(&mut *conn)
        .await
        .expect("org insert");
    sqlx::query(
        "INSERT INTO secret_folders (id, org_id, slug, label, kind)
         VALUES ($1, $2, 'documents', 'Documents', 'documents')",
    )
    .bind(folder)
    .bind(org)
    .execute(&mut *conn)
    .await
    .expect("folder insert");
    let vault_key = format!("org/{org}/secrets/{}", "7".repeat(64));
    sqlx::query(
        "INSERT INTO secret_records (id, org_id, folder_id, kind, title, object_key,
                                     file_sha256, file_bytes)
         VALUES ($1, $2, $3, 'document', 'Sweep file', $4, $5, 12)",
    )
    .bind(fixed_uuid(6_003))
    .bind(org)
    .bind(folder)
    .bind(&vault_key)
    .bind("7".repeat(64))
    .execute(&mut *conn)
    .await
    .expect("vault record");
    sqlx::query(
        "INSERT INTO plugin_catalog (id, slug, name, publisher, interface_version)
         VALUES ($1, 'sweep-plugin', 'Sweep Plugin', 'test', 2)",
    )
    .bind(catalog)
    .execute(&mut *conn)
    .await
    .expect("catalog insert");
    sqlx::query(
        "INSERT INTO plugin_installs (id, org_id, catalog_id, version, status, module_digest)
         VALUES ($1, $2, $3, '1.0.0', 'enabled', $4)",
    )
    .bind(fixed_uuid(6_004))
    .bind(org)
    .bind(catalog)
    .bind(&digest)
    .execute(&mut *conn)
    .await
    .expect("install insert");

    let refs = object_gc::referenced_keys(&db.admin)
        .await
        .expect("references");
    let claims: Vec<(&str, &str)> = refs
        .iter()
        .map(|reference| (reference.storage_key.as_str(), reference.source_table))
        .collect();
    assert!(
        claims
            .iter()
            .any(|(key, table)| *key == vault_key && *table == "secret_records"),
        "a sealed vault object must be claimed, or a delete pass frees ciphertext a \
         reveal still needs: {claims:?}"
    );
    let plugin_key = core_store::plugin_artifact_key(org, &digest);
    assert!(
        claims
            .iter()
            .any(|(key, table)| *key == plugin_key && *table == "plugin_installs"),
        "a pinned plugin module must be claimed, or a delete pass frees the module \
         its install runs: {claims:?}"
    );

    let index = object_gc::reference_index(&db.admin, &[vault_key.clone(), plugin_key])
        .await
        .expect("reference index");
    assert_eq!(
        index.get(&vault_key).map(|r| r.source_table),
        Some("secret_records"),
        "a missing vault object must be attributed to the table that claims it"
    );
    assert_eq!(
        index
            .get(&core_store::plugin_artifact_key(org, &digest))
            .map(|r| r.source_table),
        Some("plugin_installs"),
        "a missing plugin module must be attributed, and its key rebuilt from the \
         digest rather than compared as a literal"
    );

    // Leave nothing behind for the next binary in this database.
    erasure::purge_rows(&db.admin, org).await.expect("cleanup");
}
