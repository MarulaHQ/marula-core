//! DocumentVersion query-layer tests (the private build tracker D4, agent-platform §6) —
//! seed slots 140–149 are this suite's private range.
//!
//! Purpose: prove the SQL layer — document creation carries version 1,
//! edits APPEND (gap-free numbering, head bump), history is newest-first,
//! old rows stay byte-identical after later appends (append-only), restore
//! provenance is stored, the version UNIQUE gate holds, and RLS isolates
//! orgs.
//!
//! Non-purpose: restore/diff policy (core-documents unit tests) and HTTP
//! behavior (core-api documents_flow).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{documents, orgs, users, DbError};
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@doc-acme.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

async fn seed_org(app: &sqlx::PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Doc Acme", &format!("doc-acme-{n}"), "DCX")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    tx.commit().await.expect("commit seed");
    Seeded { org }
}

#[tokio::test]
async fn create_document_appends_version_one_in_one_transaction() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 140).await;
    let author = seed_user(&app, &s, 142).await;
    let mut tx = org_tx(&app, s.org).await;

    let (doc, v1) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(141),
        None,
        None,
        &serde_json::Value::Null,
        "Onboarding runbook",
        "# Day one\n1. laptop",
        "Initial version",
        author,
    )
    .await
    .expect("document + version 1");
    assert_eq!(doc.current_version, 1);
    assert_eq!(v1.version, 1);
    assert_eq!(v1.title, "Onboarding runbook");
    assert_eq!(v1.restored_from_version, None);
    assert_eq!(
        documents::max_version(&mut *tx, s.org, doc.id)
            .await
            .expect("max"),
        Some(1)
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn edits_append_gap_free_and_leave_history_untouched() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 143).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 144).await;
    let doc_id = fixed_uuid(145);

    let (_, v1) = documents::create_document(
        &mut tx,
        s.org,
        doc_id,
        None,
        None,
        &serde_json::Value::Null,
        "Runbook",
        "v1 body",
        "",
        author,
    )
    .await
    .expect("create");
    let v1_created_at = v1.created_at;

    for expected in [2, 3, 4] {
        documents::insert_version(
            &mut *tx,
            s.org,
            doc_id,
            expected,
            &format!("Runbook v{expected}"),
            &format!("v{expected} body"),
            "edit",
            author,
            None,
        )
        .await
        .expect("append");
    }

    let head = documents::get_document(&mut *tx, s.org, doc_id)
        .await
        .expect("head");
    assert_eq!(head.current_version, 4);
    assert_eq!(head.title, "Runbook v4");
    assert_eq!(
        documents::max_version(&mut *tx, s.org, doc_id)
            .await
            .expect("max"),
        Some(4)
    );

    // Newest-first history with COMPLETE bodies (diffable without extra
    // fetches).
    let history = documents::list_versions(&mut *tx, s.org, doc_id)
        .await
        .expect("history");
    let numbers: Vec<i32> = history.iter().map(|v| v.version).collect();
    assert_eq!(numbers, vec![4, 3, 2, 1]);

    // APPEND-ONLY: the oldest row is byte-identical to what was written —
    // no code path mutates or removes it, and later appends did not.
    let v1_now = documents::get_version(&mut *tx, s.org, doc_id, 1)
        .await
        .expect("v1 still readable");
    assert_eq!(v1_now.body, "v1 body");
    assert_eq!(v1_now.title, "Runbook");
    assert_eq!(v1_now.created_at, v1_created_at);

    // Single version fetch hits the exact row.
    let v2 = documents::get_version(&mut *tx, s.org, doc_id, 2)
        .await
        .expect("v2");
    assert_eq!(v2.body, "v2 body");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn restore_provenance_is_stored_on_the_appended_row() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 146).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 147).await;
    let doc_id = fixed_uuid(148);

    documents::create_document(
        &mut tx,
        s.org,
        doc_id,
        None,
        None,
        &serde_json::Value::Null,
        "Policy",
        "strict",
        "",
        author,
    )
    .await
    .expect("create");
    documents::insert_version(
        &mut *tx,
        s.org,
        doc_id,
        2,
        "Policy",
        "loose",
        "relaxed rules",
        author,
        None,
    )
    .await
    .expect("v2");

    // A restore is just an append that NAMES its source.
    let restored = documents::insert_version(
        &mut *tx,
        s.org,
        doc_id,
        3,
        "Policy",
        "strict",
        "Restored from version 1",
        author,
        Some(1),
    )
    .await
    .expect("restore append");
    assert_eq!(restored.restored_from_version, Some(1));
    assert_eq!(restored.body, "strict");
    let head = documents::get_document(&mut *tx, s.org, doc_id)
        .await
        .expect("head");
    assert_eq!(head.current_version, 3);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn version_numbers_are_unique_per_document() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 1490).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 1491).await;
    let doc_id = fixed_uuid(1492);

    documents::create_document(
        &mut tx,
        s.org,
        doc_id,
        None,
        None,
        &serde_json::Value::Null,
        "Doc",
        "one",
        "",
        author,
    )
    .await
    .expect("create");
    let dupe = documents::insert_version(
        &mut *tx,
        s.org,
        doc_id,
        1,
        "Doc",
        "overwritten?",
        "",
        author,
        None,
    )
    .await;
    assert!(matches!(dupe, Err(DbError::Sql(sqlx::Error::Database(_)))));
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn rls_blocks_cross_org_document_reads() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 1493).await;
    let author = seed_user(&app, &s, 1495).await;
    let mut tx = org_tx(&app, s.org).await;
    let (doc, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(1494),
        None,
        None,
        &serde_json::Value::Null,
        "Private plan",
        "hidden",
        "",
        author,
    )
    .await
    .expect("create");
    tx.commit().await.expect("commit");

    let other = seed_org(&app, 1496).await;
    let mut tx2 = org_tx(&app, other.org).await;
    assert!(matches!(
        documents::get_document(&mut *tx2, other.org, doc.id).await,
        Err(DbError::NotFound)
    ));
    assert!(matches!(
        documents::get_version(&mut *tx2, other.org, doc.id, 1).await,
        Err(DbError::NotFound)
    ));
    assert!(documents::list_versions(&mut *tx2, other.org, doc.id)
        .await
        .expect("history is empty for foreign org")
        .is_empty());
    tx2.commit().await.expect("commit");
}
