//! Parity between the two inventories of the object-key space (Wave 58 lane
//! `p58-object-inventory`, F61).
//!
//! Purpose: `core_db::object_gc::SOURCES` and the attachment refcount's
//! inventory (migration `crates/core-db/src/attachments.rs`,
//! `ATTACHMENT_REF_TABLES` + `ATTACHMENT_KEY_TABLES_NOT_REFERENCES`) are two
//! readings of the same `storage_key` columns. The refcount decides whether a
//! row's delete may remove bytes; the sweep decides whether an object found in
//! the store is an orphan. They are different questions, so they are different
//! lists — but the sweep's list must be the SUPERSET, and both must be complete
//! against the schema.
//!
//! What each failure means:
//!
//! * a key-holding table in neither inventory: nobody decided, and the sweep
//!   will reclaim that table's objects while rows still advertise them;
//! * an owner the sweep does not query: the same loss, one table at a time;
//! * a source that no longer holds a `storage_key`: the sweep queries a table
//!   for a column it does not have, which is a runtime error on every pass.
//!
//! The catalogue is the database, not the source: a migration that adds a
//! `storage_key` column fails here until both inventories are edited, which is
//! the point. The synthetic cases below inject a table and a shortened list to
//! prove the checker actually fails, rather than trusting that it would.
//!
//! Non-purpose: the sweep's policy (grace, caps, dry-run) is
//! `crates/worker/src/object_gc_sweep.rs`; the refcount's behaviour is
//! `tests/attachment_refcount.rs`. This suite seeds nothing, so it claims no
//! seed slots.

mod common;

use common::test_db;
use core_db::attachments::{ATTACHMENT_KEY_TABLES_NOT_REFERENCES, ATTACHMENT_REF_TABLES};
use core_db::object_gc::SOURCES;
use sqlx::PgConnection;

/// The tables in the schema that hold a `storage_key`: the catalogue's answer to
/// "which tables can put a key in the object store".
async fn key_holders(conn: &mut PgConnection) -> Vec<String> {
    sqlx::query_scalar(
        r#"SELECT c.table_name::text
             FROM information_schema.columns c
             JOIN information_schema.tables t
               ON t.table_schema = c.table_schema
              AND t.table_name = c.table_name
            WHERE c.table_schema = 'public'
              AND c.column_name = 'storage_key'
              AND t.table_type = 'BASE TABLE'
            ORDER BY c.table_name"#,
    )
    .fetch_all(&mut *conn)
    .await
    .expect("catalogue read")
}

fn gc_sources() -> Vec<&'static str> {
    SOURCES.iter().map(|(table, _, _)| *table).collect()
}

fn owners() -> Vec<&'static str> {
    ATTACHMENT_REF_TABLES
        .iter()
        .map(|(table, _)| *table)
        .collect()
}

fn excluded() -> Vec<&'static str> {
    ATTACHMENT_KEY_TABLES_NOT_REFERENCES
        .iter()
        .map(|(table, _)| *table)
        .collect()
}

/// What the two inventories get wrong, if anything, for a given catalogue and
/// pair of lists.
///
/// Pure on purpose: the database test feeds it the real catalogue and the real
/// lists, and the synthetic cases feed it one mutated table or list so the
/// failure modes are proved rather than assumed.
#[derive(Debug, Default, PartialEq, Eq)]
struct Gaps {
    /// Tables that hold a key and that neither inventory has decided about.
    undecided: Vec<String>,
    /// Refcount owners the sweep does not query: it would call their live
    /// objects orphans and delete them.
    unprotected: Vec<String>,
    /// Sweep sources that hold no `storage_key`, so the query is a runtime error.
    phantom: Vec<String>,
    /// Sweep sources the refcount has never decided about, in either list.
    stray: Vec<String>,
}

fn gaps(holders: &[String], gc: &[&str], owners: &[&str], excluded: &[&str]) -> Gaps {
    let decided = |table: &str| owners.contains(&table) || excluded.contains(&table);
    Gaps {
        undecided: holders
            .iter()
            .filter(|held| !decided(held))
            .cloned()
            .collect(),
        unprotected: owners
            .iter()
            .filter(|table| !gc.contains(table))
            .map(|table| (*table).to_string())
            .collect(),
        phantom: gc
            .iter()
            .filter(|table| !holders.iter().any(|held| held == *table))
            .map(|table| (*table).to_string())
            .collect(),
        stray: gc
            .iter()
            .filter(|table| !decided(table))
            .map(|table| (*table).to_string())
            .collect(),
    }
}

/// The live tree: every table that holds a key is decided by the refcount, every
/// owner is a sweep source, and every source is both real and decided.
#[tokio::test]
async fn the_two_inventories_cover_the_same_key_space() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let holders = key_holders(&mut conn).await;

    let found = gaps(&holders, &gc_sources(), &owners(), &excluded());
    assert_eq!(
        found,
        Gaps::default(),
        "the object-key inventories disagree with the schema. \
         undecided = tables that hold a storage_key and that neither the refcount \
         nor the sweep knows about (add each to ATTACHMENT_REF_TABLES or \
         ATTACHMENT_KEY_TABLES_NOT_REFERENCES, then to object_gc::SOURCES with a \
         reason); \
         unprotected = owners the sweep would delete the live bytes of; \
         phantom = sweep sources with no storage_key column; \
         stray = sweep sources the refcount never decided. Catalogue: {holders:?}"
    );
}

/// The catalogue half, proved: a table added to the schema with a `storage_key`
/// and no inventory entry is reported as undecided. Runs in a transaction that
/// is rolled back, so the probe table cannot reach another test or another
/// binary against the same database.
#[tokio::test]
async fn a_new_key_holding_table_is_reported_until_both_inventories_decide() {
    let db = test_db().await;
    let mut tx = db.admin.begin().await.expect("probe transaction");
    sqlx::query(
        "CREATE TABLE p58_probe_key_holders (
             id uuid PRIMARY KEY,
             org_id uuid NOT NULL,
             storage_key text NOT NULL
         )",
    )
    .execute(&mut *tx)
    .await
    .expect("probe table");

    // Read on the transaction's own connection: uncommitted DDL is visible to
    // the transaction that wrote it and to nobody else.
    let holders = key_holders(&mut tx).await;
    let found = gaps(&holders, &gc_sources(), &owners(), &excluded());

    tx.rollback().await.expect("probe rollback");

    assert!(
        holders.iter().any(|held| held == "p58_probe_key_holders"),
        "the probe table must be in the catalogue the checker reads; \
         catalogue was {holders:?}"
    );
    assert_eq!(
        found.undecided,
        vec!["p58_probe_key_holders".to_string()],
        "a key-holding table that neither inventory decided must be the one \
         reported as undecided"
    );
    assert!(
        found.unprotected.is_empty() && found.phantom.is_empty() && found.stray.is_empty(),
        "adding an undecided table must not disturb the other three verdicts: \
         {found:?}"
    );
}

/// The superset half, proved from the other direction: drop one owner from the
/// sweep's list (a stand-in for the edit a lane makes when it adds a table to the
/// refcount and forgets the sweep) and the checker names exactly that table.
#[tokio::test]
async fn an_owner_the_sweep_forgets_is_reported() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let holders = key_holders(&mut conn).await;

    let dropped = "documents_attachments";
    assert!(
        owners().contains(&dropped),
        "this case is about losing an owner; {dropped} must still be one"
    );
    let shortened: Vec<&str> = gc_sources()
        .into_iter()
        .filter(|table| *table != dropped)
        .collect();
    assert_eq!(
        shortened.len(),
        gc_sources().len() - 1,
        "the shortened list must have actually dropped the owner"
    );

    let found = gaps(&holders, &shortened, &owners(), &excluded());
    assert_eq!(
        found.unprotected,
        vec![dropped.to_string()],
        "the checker must name the owner the sweep stopped querying"
    );
    assert!(found.undecided.is_empty(), "{found:?}");
}

/// A sweep source the refcount has decided nothing about is reported as stray,
/// and is caught before it becomes a query error by the phantom check only if
/// the table does not exist — so both are asserted here.
#[tokio::test]
async fn a_source_outside_the_refcounts_universe_is_reported() {
    let db = test_db().await;
    let mut conn = db.admin.acquire().await.expect("admin connection");
    let holders = key_holders(&mut conn).await;

    let mut extended = gc_sources();
    extended.push("p58_table_the_refcount_never_decided");

    let found = gaps(&holders, &extended, &owners(), &excluded());
    assert_eq!(
        found.stray,
        vec!["p58_table_the_refcount_never_decided".to_string()],
        "a sweep source outside both refcount lists must be reported"
    );
    assert_eq!(
        found.phantom,
        vec!["p58_table_the_refcount_never_decided".to_string()],
        "a sweep source that does not hold a storage_key must be reported too"
    );
}
