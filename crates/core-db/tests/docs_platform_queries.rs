//! Docs-platform v1 query-layer tests (Wave 11 lane 7, migration 0053) —
//! seed slots 150–169 are this suite's private range.
//!
//! Purpose: prove the SQL layer for the four new constructs — document
//! client scope + typed layout fields on create/list/update, layout CRUD
//! (system layouts refuse delete), cross-entity links (create, dedupe at
//! the DB, resolved target labels, broken-target surfacing), and the
//! password vault (create/update archives history, delete cascades, RLS
//! isolates orgs).
//!
//! Non-purpose: encryption (core-api crypto tests own seal/open) and HTTP
//! behavior.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{document_layouts, document_links, documents, orgs, passwords, users, DbError};
use core_domain::entities::LinkKind;
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    client: Uuid,
}

async fn seed_org(app: &sqlx::PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let client = fixed_uuid(n + 1);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Doc Acme",
        &format!("docs-platform-{n}"),
        "DSP",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Acme Co')")
        .bind(client)
        .bind(org)
        .execute(&mut *tx)
        .await
        .expect("client insert");
    document_layouts::seed_default_document_layouts(&mut tx, org)
        .await
        .expect("layout seed");
    tx.commit().await.expect("commit seed");
    Seeded { org, client }
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@docs-platform.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

#[tokio::test]
async fn standard_layouts_seed_once_and_list_system_first() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 150).await;

    let mut tx = org_tx(&app, s.org).await;
    let layouts = document_layouts::list_layouts(&mut *tx, s.org)
        .await
        .expect("layouts");
    assert!(layouts.len() >= 10, "standard MSP set seeded");
    assert!(layouts.iter().all(|l| l.is_system));
    let server = layouts
        .iter()
        .find(|l| l.slug == "server")
        .expect("server layout");
    assert!(server
        .fields
        .iter()
        .any(|f| f.key == "hostname" && f.required));

    // Idempotent: re-seeding changes nothing.
    document_layouts::seed_default_document_layouts(&mut tx, s.org)
        .await
        .expect("re-seed");
    let again = document_layouts::list_layouts(&mut *tx, s.org)
        .await
        .expect("layouts");
    assert_eq!(layouts.len(), again.len(), "seed is idempotent");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn custom_layout_crud_and_system_delete_refusal() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 152).await;
    let mut tx = org_tx(&app, s.org).await;
    let custom = document_layouts::create_layout(
        &mut *tx,
        s.org,
        fixed_uuid(153),
        "runbook-custom",
        "Custom runbook",
        "",
        "book",
        &serde_json::json!([{"key":"owner","label":"Owner","type":"text","required":true,"position":0}]),
        false,
    )
    .await
    .expect("custom layout");

    let updated = document_layouts::update_layout(
        &mut *tx,
        s.org,
        custom.id,
        "Custom runbook v2",
        "updated",
        "book",
        &serde_json::json!([]),
    )
    .await
    .expect("update");
    assert_eq!(updated.name, "Custom runbook v2");

    // System layouts refuse delete at the query layer.
    let server = document_layouts::list_layouts(&mut *tx, s.org)
        .await
        .expect("layouts")
        .into_iter()
        .find(|l| l.slug == "server")
        .expect("server");
    assert!(matches!(
        document_layouts::delete_layout(&mut *tx, s.org, server.id).await,
        Err(DbError::NotFound)
    ));
    document_layouts::delete_layout(&mut *tx, s.org, custom.id)
        .await
        .expect("custom delete ok");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn documents_carry_client_scope_layout_and_fields() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 154).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 155).await;

    let layouts = document_layouts::list_layouts(&mut *tx, s.org)
        .await
        .expect("layouts");
    let server = layouts.iter().find(|l| l.slug == "server").expect("server");

    let (doc, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(156),
        Some(s.client),
        Some(server.id),
        &serde_json::json!({"hostname": "web-01"}),
        "web-01 server doc",
        "body",
        "",
        author,
    )
    .await
    .expect("create");
    assert_eq!(doc.client_id, Some(s.client));
    assert_eq!(doc.layout_id, Some(server.id));

    // Scope filter: global-only hides the client doc; client filter finds it.
    let globals = documents::list_documents(
        &mut *tx,
        s.org,
        &documents::DocumentFilter {
            client_id: Some(None),
            ..Default::default()
        },
    )
    .await
    .expect("globals");
    assert!(globals.iter().all(|d| d.client_id.is_none()));

    let scoped = documents::list_documents(
        &mut *tx,
        s.org,
        &documents::DocumentFilter {
            client_id: Some(Some(s.client)),
            ..Default::default()
        },
    )
    .await
    .expect("scoped");
    assert_eq!(scoped.len(), 1);

    // Meta update moves scope + fields; version history untouched.
    let after = documents::update_document_meta(
        &mut *tx,
        s.org,
        doc.id,
        "web-01 server doc",
        None,
        None,
        &serde_json::json!({}),
    )
    .await
    .expect("meta update");
    assert_eq!(after.client_id, None, "moved to org-global");
    assert_eq!(
        documents::max_version(&mut *tx, s.org, doc.id)
            .await
            .expect("max"),
        Some(1)
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn links_resolve_labels_and_surface_broken_targets() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 157).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 158).await;

    let (doc, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(159),
        None,
        None,
        &serde_json::json!({}),
        "Hub doc",
        "",
        "",
        author,
    )
    .await
    .expect("doc");
    let sealed_secret: Vec<u8> = vec![1, 2, 3];
    let pw = passwords::create_password(
        &mut *tx,
        s.org,
        fixed_uuid(160),
        Some(s.client),
        "Router admin",
        "admin",
        "",
        &sealed_secret,
        None,
        None,
        author,
    )
    .await
    .expect("password");

    let link = document_links::create_link(
        &mut *tx,
        s.org,
        fixed_uuid(161),
        LinkKind::Document,
        doc.id,
        LinkKind::Password,
        pw.id,
        author,
    )
    .await
    .expect("link");

    let resolved = document_links::list_links(&mut *tx, s.org, LinkKind::Document, doc.id)
        .await
        .expect("links");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].link.id, link.id);
    assert_eq!(resolved[0].target_label.as_deref(), Some("Router admin"));
    assert!(!resolved[0].target_deleted);

    // Duplicate links are rejected at the DB (UNIQUE gate); the failing
    // insert runs in its own tx so the outer one stays usable.
    tx.commit().await.expect("commit");
    let mut dup = org_tx(&app, s.org).await;
    assert!(document_links::create_link(
        &mut *dup,
        s.org,
        fixed_uuid(162),
        LinkKind::Document,
        doc.id,
        LinkKind::Password,
        pw.id,
        author,
    )
    .await
    .is_err());
    dup.rollback().await.expect("rollback dup");
    let mut tx = org_tx(&app, s.org).await;

    // Deleting the target surfaces the link as broken, not dropped.
    passwords::delete_password(&mut *tx, s.org, pw.id)
        .await
        .expect("delete");
    let after = document_links::list_links(&mut *tx, s.org, LinkKind::Document, doc.id)
        .await
        .expect("links");
    assert_eq!(after.len(), 1, "link survives target deletion");
    assert!(after[0].target_deleted);

    document_links::delete_link(&mut *tx, s.org, link.id)
        .await
        .expect("unlink");
    assert!(
        document_links::list_links(&mut *tx, s.org, LinkKind::Document, doc.id)
            .await
            .expect("links")
            .is_empty()
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn password_updates_archive_history_and_delete_cascades() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 163).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 164).await;

    let pw = passwords::create_password(
        &mut *tx,
        s.org,
        fixed_uuid(165),
        None,
        "Org vault root",
        "root",
        "https://example.test",
        b"first-secret",
        None,
        None,
        author,
    )
    .await
    .expect("create");

    let rotated: &[u8] = b"second-secret";
    let updated = passwords::update_password(
        &mut tx,
        s.org,
        pw.id,
        "Org vault root",
        "root",
        "https://example.test",
        Some(rotated),
        None,
        Some(None),
        author,
    )
    .await
    .expect("rotate");

    let history = passwords::list_history(&mut *tx, s.org, pw.id)
        .await
        .expect("history");
    assert_eq!(history.len(), 1, "prior ciphertext archived");
    let sealed = passwords::get_sealed(&mut *tx, s.org, pw.id)
        .await
        .expect("sealed");
    assert_eq!(
        sealed.secret, rotated,
        "current ciphertext is the new secret"
    );
    assert_eq!(updated.expires_at, None, "explicit clear applied");

    // Delete cascades the history rows.
    passwords::delete_password(&mut *tx, s.org, pw.id)
        .await
        .expect("delete");
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM password_history WHERE org_id = $1")
        .bind(s.org)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(left, 0, "history cascade-deleted with the credential");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn rls_blocks_cross_org_docs_platform_reads() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 166).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 167).await;
    let pw = passwords::create_password(
        &mut *tx,
        s.org,
        fixed_uuid(168),
        None,
        "Secret",
        "",
        "",
        b"x",
        None,
        None,
        author,
    )
    .await
    .expect("create");
    let layouts = document_layouts::list_layouts(&mut *tx, s.org)
        .await
        .expect("layouts");
    tx.commit().await.expect("commit");

    let other = seed_org(&app, 1685).await;
    let mut tx2 = org_tx(&app, other.org).await;
    assert!(matches!(
        passwords::get_password(&mut *tx2, other.org, pw.id).await,
        Err(DbError::NotFound)
    ));
    let other_layouts = document_layouts::list_layouts(&mut *tx2, other.org)
        .await
        .expect("layouts");
    assert!(
        !other_layouts
            .iter()
            .any(|l| layouts.iter().any(|mine| mine.id == l.id)),
        "layouts are org-scoped"
    );
    tx2.commit().await.expect("commit");
}
