//! Docs-platform v1 query-layer tests (Wave 11 lane 7, migration 0053) —
//! seed slots 150–169 are this suite's private range; the Wave 55
//! link-label gate tests (`p55-link-labels`) own 190–209.
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
use core_db::{
    boards, document_layouts, document_links, documents, orgs, passwords, secret_vault, tickets,
    users, DbError,
};
use core_domain::entities::{LinkKind, ResolvedLink, SecretFolderKind, SecretRecordKind};
use core_domain::policy::{BoardRole, LinkViewScope};
use core_domain::status::Priority;
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

/// Every gate open, for the caller at `user`. The resolver's own row
/// behavior is tested with this; the gate tests narrow ONE flag at a time so
/// a failure names the gate that leaked.
fn all_views(user: Uuid) -> LinkViewScope {
    LinkViewScope {
        documents: true,
        passwords: true,
        clients: true,
        tickets: true,
        kb_articles: true,
        assets: true,
        secrets: true,
        user_id: user,
        role_ids: Vec::new(),
    }
}

/// The one row in a link list that carries `kind`.
fn row_of(links: &[ResolvedLink], kind: LinkKind) -> &ResolvedLink {
    links
        .iter()
        .find(|l| l.link.to_kind == kind)
        .expect("link row for the kind")
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

    let scope = all_views(author);
    let resolved = document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &scope)
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
    let after = document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &scope)
        .await
        .expect("links");
    assert_eq!(after.len(), 1, "link survives target deletion");
    assert!(after[0].target_deleted);

    document_links::delete_link(&mut *tx, s.org, link.id)
        .await
        .expect("unlink");
    assert!(
        document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &scope)
            .await
            .expect("links")
            .is_empty()
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn link_labels_follow_the_target_modules_view_gate() {
    // Slots 190 (org+client), 191 (author), 192 (hub doc), 193 (linked doc),
    // 194 (password), 195/196 (links).
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 190).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 191).await;

    let (hub, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(192),
        None,
        None,
        &serde_json::json!({}),
        "Hub doc",
        "",
        "",
        author,
    )
    .await
    .expect("hub doc");
    let (target, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(193),
        None,
        None,
        &serde_json::json!({}),
        "Linked runbook",
        "",
        "",
        author,
    )
    .await
    .expect("target doc");
    let pw = passwords::create_password(
        &mut *tx,
        s.org,
        fixed_uuid(194),
        Some(s.client),
        "Acme domain admin",
        "admin",
        "",
        &[1, 2, 3],
        None,
        None,
        None,
        author,
    )
    .await
    .expect("password");
    document_links::create_link(
        &mut *tx,
        s.org,
        fixed_uuid(195),
        LinkKind::Document,
        hub.id,
        LinkKind::Document,
        target.id,
        author,
    )
    .await
    .expect("doc link");
    document_links::create_link(
        &mut *tx,
        s.org,
        fixed_uuid(196),
        LinkKind::Document,
        hub.id,
        LinkKind::Password,
        pw.id,
        author,
    )
    .await
    .expect("password link");

    // A caller who may read documents and nothing else: the label of their
    // own module resolves, the vault title does not.
    let docs_only = LinkViewScope {
        documents: true,
        ..Default::default()
    };
    let restricted =
        document_links::list_links(&mut tx, s.org, LinkKind::Document, hub.id, &docs_only)
            .await
            .expect("links");
    let doc_row = row_of(&restricted, LinkKind::Document);
    assert_eq!(doc_row.target_label.as_deref(), Some("Linked runbook"));
    assert!(!doc_row.target_denied);
    let pw_row = row_of(&restricted, LinkKind::Password);
    assert_eq!(
        pw_row.target_label, None,
        "a target outside the caller's gates carries no title"
    );
    assert!(pw_row.target_denied);
    assert!(!pw_row.target_deleted, "restricted is not gone");
    assert_eq!(
        pw_row.target_client_id, None,
        "the client association is target data too"
    );

    // The same panel one grant wider: the label is now a read the caller may
    // make, and nothing about the row changed but the gate.
    let resolved = document_links::list_links(
        &mut tx,
        s.org,
        LinkKind::Document,
        hub.id,
        &all_views(author),
    )
    .await
    .expect("links");
    let pw_row = row_of(&resolved, LinkKind::Password);
    assert_eq!(pw_row.target_label.as_deref(), Some("Acme domain admin"));
    assert!(!pw_row.target_denied);
    assert!(!pw_row.target_deleted);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn ticket_link_labels_follow_the_ticket_gate_and_its_board_layer() {
    // Slots 197 (org+client), 198 (author), 199 (doc), 200 (ticket),
    // 201 (link).
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 197).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 198).await;

    let (doc, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(199),
        None,
        None,
        &serde_json::json!({}),
        "Hub doc",
        "",
        "",
        author,
    )
    .await
    .expect("hub doc");
    let board = boards::list_boards(&mut *tx, s.org)
        .await
        .expect("boards")
        .into_iter()
        .next()
        .expect("default board seeded");
    let status = boards::default_status_for_board(&mut *tx, board.id)
        .await
        .expect("entry status");
    let ticket = tickets::insert_ticket(
        &mut tx,
        tickets::NewTicket {
            id: fixed_uuid(200),
            org_id: s.org,
            number: "MAR-1901",
            board_id: board.id,
            status_id: status.id,
            title: "Printer down",
            body: None,
            priority: Priority::P3,
            client_id: Some(s.client),
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket");
    document_links::create_link(
        &mut *tx,
        s.org,
        fixed_uuid(201),
        LinkKind::Document,
        doc.id,
        LinkKind::Ticket,
        ticket.id,
        author,
    )
    .await
    .expect("ticket link");

    // A board membership row never expands access: with the ticket module
    // closed, a board viewer still gets no label.
    boards::upsert_member(&mut tx, s.org, board.id, author, BoardRole::Viewer)
        .await
        .expect("board membership");
    let no_tickets = LinkViewScope {
        documents: true,
        ..Default::default()
    };
    let restricted =
        document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &no_tickets)
            .await
            .expect("links");
    let ticket_row = row_of(&restricted, LinkKind::Ticket);
    assert_eq!(ticket_row.target_label, None);
    assert!(ticket_row.target_denied);
    assert!(!ticket_row.target_deleted);

    // With the module open the board layer composes and allows: every board
    // role carries `BoardOp::View`.
    let resolved = document_links::list_links(
        &mut tx,
        s.org,
        LinkKind::Document,
        doc.id,
        &all_views(author),
    )
    .await
    .expect("links");
    let ticket_row = row_of(&resolved, LinkKind::Ticket);
    assert_eq!(
        ticket_row.target_label.as_deref(),
        Some("MAR-1901 Printer down")
    );
    assert!(!ticket_row.target_denied);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn secret_link_labels_respect_the_vaults_restricted_folder_gate() {
    // Slots 202 (org+client), 203 (author), 204 (doc), 205 (restricted
    // folder), 206 (record), 207 (link).
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 202).await;
    let mut tx = org_tx(&app, s.org).await;
    let author = seed_user(&app, &s, 203).await;
    let tech_role: Uuid =
        sqlx::query_scalar("SELECT id FROM roles WHERE org_id = $1 AND builtin = 'technician'")
            .bind(s.org)
            .fetch_one(&mut *tx)
            .await
            .expect("technician role");

    let (doc, _) = documents::create_document(
        &mut tx,
        s.org,
        fixed_uuid(204),
        None,
        None,
        &serde_json::json!({}),
        "Hub doc",
        "",
        "",
        author,
    )
    .await
    .expect("hub doc");
    let folder = secret_vault::create_folder(
        &mut tx,
        s.org,
        None,
        None,
        fixed_uuid(205),
        "msp-api-secrets",
        "MSP API secrets",
        SecretFolderKind::ApiKeys,
        false,
        true,
        author,
    )
    .await
    .expect("restricted folder");
    let record = secret_vault::create_record(
        &mut tx,
        s.org,
        None,
        folder.id,
        fixed_uuid(206),
        SecretRecordKind::Password,
        "Acme domain admin",
        None,
        None,
        None,
        None,
        None,
        &[],
        author,
    )
    .await
    .expect("secret record");
    document_links::create_link(
        &mut *tx,
        s.org,
        fixed_uuid(207),
        LinkKind::Document,
        doc.id,
        LinkKind::Secret,
        record.id,
        author,
    )
    .await
    .expect("secret link");

    // The module is open; the restricted chain denies this role, exactly as
    // it does on the vault's own list.
    let tech = LinkViewScope {
        documents: true,
        secrets: true,
        user_id: author,
        role_ids: vec![tech_role],
        ..Default::default()
    };
    let denied = document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &tech)
        .await
        .expect("links");
    let secret_row = row_of(&denied, LinkKind::Secret);
    assert_eq!(
        secret_row.target_label, None,
        "a restricted folder's title is not a label"
    );
    assert!(secret_row.target_denied);

    // A read grant on the chain opens it — the gate the resolver consults is
    // the one the vault itself uses.
    secret_vault::upsert_grant(&mut tx, s.org, tech_role, folder.id, true, false, author)
        .await
        .expect("read grant");
    let granted = document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &tech)
        .await
        .expect("links");
    let secret_row = row_of(&granted, LinkKind::Secret);
    assert_eq!(
        secret_row.target_label.as_deref(),
        Some("Acme domain admin")
    );
    assert!(!secret_row.target_denied);

    // Closing the module again redacts it regardless of the grant.
    let module_closed = LinkViewScope {
        documents: true,
        user_id: author,
        role_ids: vec![tech_role],
        ..Default::default()
    };
    let closed =
        document_links::list_links(&mut tx, s.org, LinkKind::Document, doc.id, &module_closed)
            .await
            .expect("links");
    let secret_row = row_of(&closed, LinkKind::Secret);
    assert!(secret_row.target_denied);
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
        None,
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
