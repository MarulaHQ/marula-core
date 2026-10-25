//! Attachment object refcount across the shared content-addressed key space
//! (Wave 56 lane `p56-attachment-refcount`, audit record
//! `crates/core-db/src/attachments.rs:count-other-refs:missing-documents-refcount`).
//!
//! Purpose: prove that one stored object is reclaimed only when NO table that
//! references it still holds its key. `attachments` (tickets, comments, inbound
//! mail) and `documents_attachments` (documents) write into the same per-org key
//! space, so a count that asks one table lets a ticket-side delete remove bytes
//! a document row still advertises. The first three cases recover that
//! invariant; the fourth keeps reclaim working when the last reference goes.
//!
//! The last two cases are a gate on the inventory itself: the database's own
//! catalogue is asked which tables hold a `storage_key`, so a migration that
//! adds another one fails here until it is either counted or excluded with a
//! reason. Source cannot be made to fail; the schema can.
//!
//! Non-purpose: HTTP behaviour (core-api `attachments_flow`) and the object
//! store (core-store's own tests). Database selection follows the contract in
//! `tests/common/mod.rs`.
//!
//! Seed slots 250–279 are this suite's private range.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{attachments, document_attachments, orgs, users};
use core_domain::entities::{Attachment, AttachmentKind};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    board: Uuid,
    entry_status: Uuid,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Refcount Acme",
        &format!("refcount-{n}"),
        "RFC",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    let board = orgs::seed_default_board(&mut tx, org).await.expect("board");
    let entry_status: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry LIMIT 1",
    )
    .bind(board)
    .fetch_one(&mut *tx)
    .await
    .expect("entry status");
    tx.commit().await.expect("seed commit");
    Seeded {
        org,
        board,
        entry_status,
    }
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@refcount.test"),
        "Refcount User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("user commit");
    user
}

/// The per-org content-addressed key both attachment tables use.
fn key(org: Uuid, sha_hex: &str) -> String {
    format!("org/{org}/attachments/{sha_hex}")
}

async fn seed_ticket(conn: &mut PgConnection, s: &Seeded, id: Uuid) -> Uuid {
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category)
         VALUES ($1, $2, $3, $4, $5, 'Shared bytes', 'p3', 'open')",
    )
    .bind(id)
    .bind(s.org)
    .bind(format!("RFC-{}", id.simple()))
    .bind(s.board)
    .bind(s.entry_status)
    .execute(&mut *conn)
    .await
    .expect("ticket insert");
    id
}

async fn seed_email_message(conn: &mut PgConnection, org: Uuid, id: Uuid) -> Uuid {
    sqlx::query(
        "INSERT INTO email_messages (id, org_id, direction, rfc_message_id, from_address,
                                     scan_status, transport)
         VALUES ($1, $2, 'inbound', $3, 'client@example.test', 'clean', 'webhook/test')",
    )
    .bind(id)
    .bind(org)
    .bind(format!("<{}@refcount.test>", id.simple()))
    .execute(&mut *conn)
    .await
    .expect("email message insert");
    id
}

async fn seed_document(conn: &mut PgConnection, org: Uuid, author: Uuid, id: Uuid) -> Uuid {
    sqlx::query(
        "INSERT INTO documents (id, org_id, title, current_version, created_by, updated_by)
         VALUES ($1, $2, 'Runbook', 1, $3, $3)",
    )
    .bind(id)
    .bind(org)
    .bind(author)
    .execute(&mut *conn)
    .await
    .expect("document insert");
    id
}

/// One ticket-owned attachment row on the shared key.
async fn insert_ticket_attachment(
    conn: &mut PgConnection,
    org: Uuid,
    id: Uuid,
    ticket_id: Uuid,
    sha_hex: &str,
    uploader: Uuid,
) -> Attachment {
    let storage_key = key(org, sha_hex);
    attachments::insert(
        conn,
        attachments::NewAttachment {
            id,
            org_id: org,
            kind: AttachmentKind::Ticket,
            ticket_id: Some(ticket_id),
            ticket_comment_id: None,
            email_message_id: None,
            filename: "shared.pdf",
            content_type: "application/pdf",
            size_bytes: 64,
            sha256: sha_hex,
            storage_key: &storage_key,
            uploaded_by_user_id: Some(uploader),
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("ticket attachment insert")
}

/// One inbound-email attachment row on the shared key: same table, different
/// owner column, and the key space does not care which one names the owner.
async fn insert_email_attachment(
    conn: &mut PgConnection,
    org: Uuid,
    id: Uuid,
    email_message_id: Uuid,
    sha_hex: &str,
) -> Attachment {
    let storage_key = key(org, sha_hex);
    attachments::insert(
        conn,
        attachments::NewAttachment {
            id,
            org_id: org,
            kind: AttachmentKind::EmailMessage,
            ticket_id: None,
            ticket_comment_id: None,
            email_message_id: Some(email_message_id),
            filename: "shared.pdf",
            content_type: "application/pdf",
            size_bytes: 64,
            sha256: sha_hex,
            storage_key: &storage_key,
            uploaded_by_user_id: None,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("email attachment insert")
}

async fn insert_document_attachment(
    conn: &mut PgConnection,
    org: Uuid,
    id: Uuid,
    document_id: Uuid,
    sha_hex: &str,
    uploader: Uuid,
) -> document_attachments::DocumentAttachmentRow {
    let storage_key = key(org, sha_hex);
    document_attachments::insert(
        conn,
        document_attachments::NewDocumentAttachment {
            id,
            org_id: org,
            document_id,
            filename: "shared.pdf",
            content_type: "application/pdf",
            size_bytes: 64,
            sha256: sha_hex,
            storage_key: &storage_key,
            uploaded_by: Some(uploader),
        },
    )
    .await
    .expect("document attachment insert")
}

/// The record's exact path: bytes attached to a document, then the same bytes
/// attached to a ticket by someone with no claim on the document. Deleting the
/// ticket row must leave the object alone, because the document row still
/// advertises it.
#[tokio::test]
async fn an_object_referenced_only_by_a_document_is_not_reclaimed() {
    let db = test_db().await;
    let s = seed_org(&db.app, 250).await;
    let user = seed_user(&db.app, &s, 251).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = seed_ticket(&mut tx, &s, fixed_uuid(252)).await;
    let sha = "d".repeat(64);

    let ticket_row =
        insert_ticket_attachment(&mut tx, s.org, fixed_uuid(253), ticket, &sha, user).await;
    let document = seed_document(&mut tx, s.org, user, fixed_uuid(254)).await;
    insert_document_attachment(&mut tx, s.org, fixed_uuid(255), document, &sha, user).await;

    // The ticket-side delete asks BEFORE it removes its own row, so the row it
    // is deleting is excluded by id and the document row is the one left.
    let refs =
        attachments::count_other_refs(&mut tx, s.org, &ticket_row.storage_key, ticket_row.id)
            .await
            .expect("refcount");
    assert_eq!(
        refs, 1,
        "the document attachment holds the same key, so object_removed must be false"
    );

    attachments::delete(&mut tx, s.org, ticket_row.id)
        .await
        .expect("delete ticket row")
        .expect("row existed");
    let refs =
        attachments::count_other_refs(&mut tx, s.org, &ticket_row.storage_key, ticket_row.id)
            .await
            .expect("refcount after delete");
    assert_eq!(refs, 1, "the document's reference outlives the ticket row");
}

/// The key space does not care which owner column a row uses: an inbound-email
/// attachment is a sibling reference just as a document row is.
#[tokio::test]
async fn an_object_referenced_only_by_an_email_message_is_not_reclaimed() {
    let db = test_db().await;
    let s = seed_org(&db.app, 256).await;
    let user = seed_user(&db.app, &s, 257).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = seed_ticket(&mut tx, &s, fixed_uuid(258)).await;
    let sha = "e".repeat(64);

    let ticket_row =
        insert_ticket_attachment(&mut tx, s.org, fixed_uuid(259), ticket, &sha, user).await;
    let email = seed_email_message(&mut tx, s.org, fixed_uuid(260)).await;
    insert_email_attachment(&mut tx, s.org, fixed_uuid(261), email, &sha).await;

    let refs =
        attachments::count_other_refs(&mut tx, s.org, &ticket_row.storage_key, ticket_row.id)
            .await
            .expect("refcount");
    assert_eq!(refs, 1, "the email attachment's row is a reference");
}

/// Two references on different surfaces: removing one is not removing the last,
/// and each removal is counted down to the one that finally frees the object.
#[tokio::test]
async fn an_object_referenced_by_two_places_survives_until_the_last_one_goes() {
    let db = test_db().await;
    let s = seed_org(&db.app, 262).await;
    let user = seed_user(&db.app, &s, 263).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = seed_ticket(&mut tx, &s, fixed_uuid(264)).await;
    let sha = "f".repeat(64);

    let ticket_row =
        insert_ticket_attachment(&mut tx, s.org, fixed_uuid(265), ticket, &sha, user).await;
    let document = seed_document(&mut tx, s.org, user, fixed_uuid(266)).await;
    let document_row =
        insert_document_attachment(&mut tx, s.org, fixed_uuid(267), document, &sha, user).await;
    let email = seed_email_message(&mut tx, s.org, fixed_uuid(268)).await;
    let email_row = insert_email_attachment(&mut tx, s.org, fixed_uuid(269), email, &sha).await;

    let storage_key = ticket_row.storage_key.clone();
    let refs = attachments::count_other_refs(&mut tx, s.org, &storage_key, ticket_row.id)
        .await
        .expect("refcount");
    assert_eq!(refs, 2, "document and email rows both reference the key");

    // The ticket row goes; the object stays for the other two.
    attachments::delete(&mut tx, s.org, ticket_row.id)
        .await
        .expect("delete ticket row")
        .expect("row existed");
    assert_eq!(
        attachments::count_other_refs(&mut tx, s.org, &storage_key, ticket_row.id)
            .await
            .expect("refcount"),
        2
    );

    // The document row goes; the email row is now the last reference, and the
    // document-side helper must agree with the ticket-side one.
    document_attachments::delete(&mut tx, s.org, document, document_row.id)
        .await
        .expect("delete document row")
        .expect("row existed");
    assert_eq!(
        document_attachments::count_other_refs(&mut tx, s.org, &storage_key, document_row.id)
            .await
            .expect("refcount"),
        1,
        "the email attachment is still a reference after the document's row is gone"
    );

    // The email row goes; now zero, and only now may the caller reclaim.
    attachments::delete(&mut tx, s.org, email_row.id)
        .await
        .expect("delete email row")
        .expect("row existed");
    assert_eq!(
        attachments::count_other_refs(&mut tx, s.org, &storage_key, email_row.id)
            .await
            .expect("refcount"),
        0,
        "the last reference is the one that frees the object"
    );
}

/// No reference anywhere means reclaim — and another org holding the same key
/// string is not a reference in THIS org, because the count is org-scoped.
#[tokio::test]
async fn an_object_referenced_nowhere_is_reclaimed_and_foreign_orgs_do_not_count() {
    let db = test_db().await;
    let s = seed_org(&db.app, 270).await;
    let other = seed_org(&db.app, 271).await;
    let user = seed_user(&db.app, &s, 272).await;
    let sha = "a".repeat(64);

    let storage_key;
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let ticket = seed_ticket(&mut tx, &s, fixed_uuid(273)).await;
        let row =
            insert_ticket_attachment(&mut tx, s.org, fixed_uuid(274), ticket, &sha, user).await;
        storage_key = row.storage_key.clone();

        // Another org's document attachment, carrying the SAME key string. The
        // key embeds the org, so this cannot arise through the app — but the
        // count is org-scoped, and a foreign row must never keep this org's
        // object alive.
        let other_user = seed_user(&db.app, &other, 275).await;
        let mut other_tx = org_tx(&db.app, other.org).await;
        let other_doc = seed_document(&mut other_tx, other.org, other_user, fixed_uuid(276)).await;
        let foreign = insert_document_attachment(
            &mut other_tx,
            other.org,
            fixed_uuid(277),
            other_doc,
            &sha,
            other_user,
        )
        .await;
        assert_eq!(foreign.storage_key, storage_key);
        other_tx.commit().await.expect("foreign org commit");

        let refs = attachments::count_other_refs(&mut tx, s.org, &storage_key, row.id)
            .await
            .expect("refcount");
        assert_eq!(refs, 0, "a foreign org's row is not a reference here");

        attachments::delete(&mut tx, s.org, row.id)
            .await
            .expect("delete last row")
            .expect("row existed");
        assert_eq!(
            attachments::count_other_refs(&mut tx, s.org, &storage_key, row.id)
                .await
                .expect("refcount after delete"),
            0
        );
        tx.commit().await.expect("own org commit");
    }
}

/// Every table that holds a `storage_key` is either counted or excluded with a
/// reason. A migration adding another one fails here rather than silently
/// re-opening the missing-reference class this suite exists to close.
#[tokio::test]
async fn the_refcount_inventory_matches_every_table_that_holds_a_key() {
    let db = test_db().await;
    let holders: Vec<String> = sqlx::query_scalar(
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
    .fetch_all(&db.admin)
    .await
    .expect("catalogue read");

    let mut classified: Vec<String> = attachments::ATTACHMENT_REF_TABLES
        .iter()
        .map(|(table, _why)| (*table).to_string())
        .collect();
    classified.extend(
        attachments::ATTACHMENT_KEY_TABLES_NOT_REFERENCES
            .iter()
            .map(|(table, _why)| (*table).to_string()),
    );
    classified.sort();

    let unclassified: Vec<&String> = holders
        .iter()
        .filter(|held| !classified.contains(held))
        .collect();
    assert!(
        unclassified.is_empty(),
        "a table holds a storage_key and the refcount inventory does not know it: \
         {unclassified:?}. Add it to ATTACHMENT_REF_TABLES if its rows own attachment \
         objects, or to ATTACHMENT_KEY_TABLES_NOT_REFERENCES with the reason they do not."
    );
    assert_eq!(
        holders, classified,
        "the inventory names a table that no longer holds a storage_key, or one that \
         never did"
    );
}

/// The statement the refcount runs may only name tables that actually have the
/// columns it binds; a typo in the inventory would otherwise surface as a
/// runtime error on someone's delete.
#[tokio::test]
async fn every_referencing_table_has_the_columns_the_refcount_binds() {
    let db = test_db().await;
    for (table, _why) in attachments::ATTACHMENT_REF_TABLES {
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1",
        )
        .bind(table)
        .fetch_all(&db.admin)
        .await
        .unwrap_or_else(|e| panic!("catalogue read for {table}: {e}"));

        for required in ["id", "org_id", "storage_key"] {
            assert!(
                columns.iter().any(|c| c == required),
                "{table} is counted by the refcount but has no {required} column"
            );
        }
    }
}
