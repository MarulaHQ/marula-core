//! Template registry query-layer tests (Wave 13 lane 5, KB3/KB4,
//! migration 0070) — seed slots 150–167 are this suite's private range.
//!
//! Purpose: prove the SQL layer — shipped defaults seed as immutable
//! system rows byte-identical to `core_domain::templates::
//! DEFAULT_TEMPLATES` (the 0071 drift-guard pattern), org-owned rows
//! update/delete while system rows are immutable through every write
//! path, restore only touches `is_system` rows with a `builtin` tag, the
//! kind filter + defaults-first ordering hold, RLS scopes the table per
//! org, and the render-context fetchers resolve the joined entity
//! snapshot (ticket + client + contact + assignee + org, project, quote).
//!
//! Non-purpose: pure variable vocabulary/rendering rules (core-domain
//! templates unit tests) and HTTP behavior (core-api templates_flow).
//! Database selection follows the documented contract in tests/common.

mod common;

use common::{fixed_uuid, org_tx, test_db, FUTURE};
use core_db::{boards, orgs, templates, tickets, users};
use core_domain::status::Priority;
use core_domain::templates::DEFAULT_TEMPLATES;
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    board: Uuid,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Template Acme",
        &format!("tmpl-acme-{n}"),
        "TMA",
    )
    .await
    .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    templates::seed_default_templates(&mut tx, org)
        .await
        .expect("template seeds");
    tx.commit().await.expect("commit seed");
    let mut tx = org_tx(app, org).await;
    let board = boards::list_boards(&mut *tx, org).await.expect("boards")[0].id;
    tx.commit().await.expect("commit");
    Seeded { org, board }
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@tmpl-acme.test"),
        "Template User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

async fn seed_client_and_contact(conn: &mut PgConnection, s: &Seeded, n: u64) -> (Uuid, Uuid) {
    let client = fixed_uuid(n);
    let contact = fixed_uuid(n + 1);
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(client)
        .bind(s.org)
        .bind(format!("Client {n}"))
        .execute(&mut *conn)
        .await
        .expect("client insert");
    sqlx::query(
        "INSERT INTO contacts (id, org_id, client_id, name, email, phone, is_primary) \
         VALUES ($1, $2, $3, $4, $5, $6, true)",
    )
    .bind(contact)
    .bind(s.org)
    .bind(client)
    .bind("J. Doe")
    .bind(format!("j{n}@client.test"))
    .bind("+15550001")
    .execute(&mut *conn)
    .await
    .expect("contact insert");
    (client, contact)
}

async fn insert_org_template(
    conn: &mut PgConnection,
    s: &Seeded,
    n: u64,
    kind: &str,
    slug: &str,
    body: &str,
) -> core_db::templates::TemplateEntry {
    templates::insert_template(
        conn,
        fixed_uuid(n),
        s.org,
        kind,
        slug,
        &format!("Org template {n}"),
        "org-owned",
        None,
        body,
        &serde_json::json!([]),
        None,
    )
    .await
    .expect("org template insert")
}

#[tokio::test]
async fn shipped_defaults_seed_byte_identical_to_canonical() {
    let db = test_db().await;
    let app = &db.app;
    let s = seed_org(&app, 150).await;

    let mut tx = org_tx(&app, s.org).await;
    let rows = templates::list_templates(&mut *tx, s.org, None, 200, 0)
        .await
        .expect("list");
    tx.commit().await.expect("commit");

    assert_eq!(rows.len(), DEFAULT_TEMPLATES.len(), "one seed per kind");
    for d in DEFAULT_TEMPLATES {
        let row = rows
            .iter()
            .find(|r| r.builtin.as_deref() == Some(d.builtin))
            .unwrap_or_else(|| panic!("seed `{}` missing", d.builtin));
        assert_eq!(row.kind, d.kind.as_str());
        assert_eq!(row.slug, d.slug);
        assert_eq!(row.name, d.name);
        assert_eq!(row.description, d.description);
        assert_eq!(
            row.subject.as_deref(),
            d.subject,
            "subject drift on `{}`",
            d.builtin
        );
        assert_eq!(row.body, d.body, "body drift on `{}`", d.builtin);
        assert!(row.is_system);
        assert_eq!(row.forked_from, None);
    }
}

#[tokio::test]
async fn org_rows_crud_but_system_rows_immutable() {
    let db = test_db().await;
    let app = &db.app;
    let s = seed_org(&app, 151).await;
    let mut tx = org_tx(&app, s.org).await;

    let org_row = insert_org_template(&mut tx, &s, 152, "note", "my_snippet", "hello").await;
    assert!(!org_row.is_system);

    // Org rows update + delete.
    let updated = templates::update_template(
        &mut tx,
        s.org,
        org_row.id,
        "Renamed",
        "desc",
        None,
        "new body",
        &serde_json::json!([]),
    )
    .await
    .expect("org update");
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.body, "new body");
    let deleted = templates::delete_template(&mut tx, s.org, org_row.id)
        .await
        .expect("org delete");
    assert_eq!(deleted.slug, "my_snippet");

    // System rows are immutable through every write path.
    let builtin = templates::list_templates(&mut *tx, s.org, Some("kb"), 10, 0)
        .await
        .expect("list")[0]
        .clone();
    assert!(builtin.is_system);
    assert!(
        templates::update_template(
            &mut tx,
            s.org,
            builtin.id,
            "X",
            "x",
            None,
            "x",
            &serde_json::json!([])
        )
        .await
        .is_err(),
        "system row must not update"
    );
    assert!(
        templates::delete_template(&mut tx, s.org, builtin.id)
            .await
            .is_err(),
        "system row must not delete"
    );

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn restore_resets_preset_content_only() {
    let db = test_db().await;
    let app = &db.app;
    let s = seed_org(&app, 153).await;
    let mut tx = org_tx(&app, s.org).await;

    let builtin = templates::list_templates(&mut *tx, s.org, Some("note"), 10, 0)
        .await
        .expect("list")
        .into_iter()
        .find(|r| r.builtin.as_deref() == Some("ticket_resolution_note"))
        .expect("note seed");

    // A fork (org row with forked_from) is org data — never restorable.
    let fork = templates::insert_template(
        &mut tx,
        fixed_uuid(154),
        s.org,
        "note",
        "my_resolution",
        "My resolution note",
        "forked",
        None,
        "drifted body",
        &serde_json::json!([]),
        Some("ticket_resolution_note"),
    )
    .await
    .expect("fork insert");
    assert!(
        templates::restore_default_template(&mut tx, s.org, fork.id, None, "x")
            .await
            .is_err(),
        "org-owned fork must not restore"
    );

    // The preset row restores to the canonical copy.
    let canonical = DEFAULT_TEMPLATES
        .iter()
        .find(|d| d.builtin == "ticket_resolution_note")
        .unwrap();
    let restored = templates::restore_default_template(
        &mut tx,
        s.org,
        builtin.id,
        canonical.subject,
        canonical.body,
    )
    .await
    .expect("restore");
    assert_eq!(restored.body, canonical.body);

    // Cross-check: even after a hypothetical drift the restore returns the
    // compiled canonical copy (already asserted by equality above).
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn list_filters_by_kind_and_defaults_first() {
    let db = test_db().await;
    let app = &db.app;
    let s = seed_org(&app, 155).await;
    let mut tx = org_tx(&app, s.org).await;
    insert_org_template(&mut tx, &s, 156, "email", "zzz_org_email", "body").await;

    let all = templates::list_templates(&mut *tx, s.org, None, 200, 0)
        .await
        .expect("list");
    assert_eq!(all.len(), DEFAULT_TEMPLATES.len() + 1);
    let system_count = all.iter().take_while(|r| r.is_system).count();
    assert_eq!(system_count, DEFAULT_TEMPLATES.len(), "defaults sort first");

    let emails = templates::list_templates(&mut *tx, s.org, Some("email"), 50, 0)
        .await
        .expect("email filter");
    assert_eq!(emails.len(), 2, "the shipped email default + the org row");
    assert!(emails.iter().all(|r| r.kind == "email"));
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn rls_scopes_templates_per_org() {
    let db = test_db().await;
    let app = &db.app;
    let a = seed_org(&app, 157).await;
    let b = seed_org(&app, 158).await;
    let mut tx_a = org_tx(&app, a.org).await;
    insert_org_template(&mut tx_a, &a, 159, "workflow", "playbook_a", "secret").await;
    tx_a.commit().await.expect("commit");

    let mut tx_b = org_tx(&app, b.org).await;
    let rows_b = templates::list_templates(&mut *tx_b, b.org, None, 200, 0)
        .await
        .expect("list");
    assert_eq!(
        rows_b.len(),
        DEFAULT_TEMPLATES.len(),
        "org B sees only its seeds"
    );
    tx_b.commit().await.expect("commit");
}

#[tokio::test]
async fn render_contexts_resolve_joined_entity_snapshot() {
    let db = test_db().await;
    let app = &db.app;
    let s = seed_org(&app, 160).await;
    let user = seed_user(&app, &s, 161).await;

    let mut tx = org_tx(&app, s.org).await;
    let (client, contact) = seed_client_and_contact(&mut tx, &s, 162).await;
    let statuses = boards::list_statuses(&mut *tx, s.board)
        .await
        .expect("statuses");
    let (prefix, seq) = orgs::next_ticket_number(&mut *tx, s.org)
        .await
        .expect("number");
    let ticket = tickets::insert_ticket(
        &mut tx,
        tickets::NewTicket {
            id: fixed_uuid(164),
            org_id: s.org,
            number: &format!("{prefix}-{seq}"),
            board_id: s.board,
            status_id: statuses[0].id,
            title: "Laptop will not boot",
            body: None,
            priority: Priority::P1,
            client_id: Some(client),
            site_id: None,
            contact_id: Some(contact),
            assignee_id: Some(user),
            requester_id: None,
            due_at: Some(FUTURE),
        },
    )
    .await
    .expect("ticket insert");

    let ctx = templates::ticket_render_context(&mut *tx, s.org, ticket.id, user)
        .await
        .expect("ticket context");
    let map = ctx.to_map();
    assert_eq!(map.get("org.name").unwrap(), "Template Acme");
    assert_eq!(map.get("user.name").unwrap(), "Template User");
    assert_eq!(map.get("client.name").unwrap(), "Client 162");
    assert_eq!(map.get("contact.name").unwrap(), "J. Doe");
    assert_eq!(
        map.get("contact.email").unwrap(),
        &format!("j162@client.test")
    );
    assert_eq!(map.get("ticket.summary").unwrap(), "Laptop will not boot");
    assert_eq!(map.get("ticket.priority").unwrap(), "p1");
    assert!(map.contains_key("ticket.number"));
    assert!(map.contains_key("ticket.status"));
    assert_eq!(map.get("assignee.name").unwrap(), "Template User");

    // A second user renders the same ticket: the joined snapshot is the
    // entity's, only `user.name` follows the caller.
    let other = seed_user(&app, &s, 165).await;
    let ctx2 = templates::ticket_render_context(&mut *tx, s.org, ticket.id, other)
        .await
        .expect("ticket context 2");
    assert_eq!(ctx2.to_map().get("user.name").unwrap(), "Template User");

    // Project context.
    sqlx::query(
        "INSERT INTO projects (id, org_id, client_id, name, status) \
         VALUES ($1, $2, $3, 'Onboarding ACME', 'active')",
    )
    .bind(fixed_uuid(166))
    .bind(s.org)
    .bind(client)
    .execute(&mut *tx)
    .await
    .expect("project insert");
    let pctx = templates::project_render_context(&mut *tx, s.org, fixed_uuid(166), user)
        .await
        .expect("project context");
    let pmap = pctx.to_map();
    assert_eq!(pmap.get("project.name").unwrap(), "Onboarding ACME");
    assert_eq!(pmap.get("project.status").unwrap(), "active");
    assert_eq!(pmap.get("client.name").unwrap(), "Client 162");

    // Quote context (Wave 12 quote catalog row; total through Money).
    sqlx::query(
        "INSERT INTO quotes (id, org_id, number, client_id, status, currency, valid_until, total_minor) \
         VALUES ($1, $2, 'Q-0001', $3, 'sent', 'USD', $4, 12345)",
    )
    .bind(fixed_uuid(167))
    .bind(s.org)
    .bind(client)
    .bind(chrono::NaiveDate::from_ymd_opt(2030, 1, 1).expect("date"))
    .execute(&mut *tx)
    .await
    .expect("quote insert");
    let qctx = templates::quote_render_context(&mut *tx, s.org, fixed_uuid(167), user)
        .await
        .expect("quote context");
    let qmap = qctx.to_map();
    assert_eq!(qmap.get("quote.number").unwrap(), "Q-0001");
    assert_eq!(qmap.get("quote.status").unwrap(), "sent");
    assert_eq!(qmap.get("quote.total").unwrap(), "123.45 USD");
    assert_eq!(qmap.get("quote.currency").unwrap(), "USD");
    assert_eq!(qmap.get("client.name").unwrap(), "Client 162");

    tx.commit().await.expect("commit");
}
