//! Note-drafts + workflow-meta query-layer tests (Wave 17 lane 11,
//! p17-workbar-notes-tasks) — seed slots 180–189 are this suite's private
//! range.
//!
//! Purpose: prove the SQL layer of migration 0080 —
//! (a) `note_drafts` IF2 autosave: one draft per (org, ticket, user),
//!     debounced upsert (read-your-write), delete-on-submit keyed by the
//!     draft's idempotency key, 30-day retention purge, RLS isolation;
//! (b) `ticket_comments.workflow_meta`: user inserts default to NULL
//!     (readable as None); a workflow-marked insert round-trips the jsonb
//!     payload through get/list so the machine-provenance pill renders
//!     everywhere.
//!
//! Non-purpose: domain rules (core-domain notes unit tests) and HTTP
//! behavior (core-api flows). Database selection follows the contract in
//! tests/common/mod.rs (lane-exclusive PG port 54380 via
//! MERIDIAN_TEST_DATABASE_URL).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{note_drafts, orgs, tickets, users, DbError};
use core_domain::status::{NoteKind, Priority};
use sqlx::PgPool;
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    ticket: Uuid,
    user: Uuid,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "WB Acme", &format!("wb-acme-{n}"), "WBX")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    let user = fixed_uuid(n + 1);
    users::create_user(&mut *tx, user, org, &format!("u{n}@wb.test"), "Tech User", "argon2hash", "UTC")
        .await
        .expect("user");
    tx.commit().await.expect("commit seed");
    let (board, ticket) = {
        let mut tx = org_tx(app, org).await;
        let board = core_db::boards::list_boards(&mut *tx, org)
            .await
            .expect("boards")
            .first()
            .expect("default board")
            .id;
        let statuses = core_db::boards::list_statuses(&mut *tx, board)
            .await
            .expect("statuses");
        let open = statuses
            .iter()
            .find(|s| s.category == core_domain::status::TicketStatusCategory::Open)
            .unwrap()
            .id;
        tx.commit().await.expect("commit seed");
        let ticket = fixed_uuid(n + 2);
        let mut tx = org_tx(app, org).await;
        tickets::insert_ticket(
            &mut *tx,
            tickets::NewTicket {
                id: ticket,
                org_id: org,
                number: &format!("WB-{n}"),
                board_id: board,
                status_id: open,
                title: "Workbar test ticket",
                body: None,
                priority: Priority::P3,
                client_id: None,
                site_id: None,
                contact_id: None,
                assignee_id: None,
                requester_id: None,
                due_at: None,
            },
        )
        .await
        .expect("ticket insert");
        tx.commit().await.expect("commit ticket");
        (board, ticket)
    };
    Seeded { org, ticket, user }
}

#[tokio::test]
async fn draft_upsert_is_one_row_per_user_ticket() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 180).await;

    let mut tx = org_tx(&app, s.org).await;
    let row = note_drafts::upsert_draft(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        s.user,
        "diagnosed printer",
        "key-a",
    )
    .await
    .expect("upsert 1");
    assert_eq!(row.body, "diagnosed printer");

    // Debounced overwrite: SAME row, last-writer-wins, versioned updated_at.
    let row = note_drafts::upsert_draft(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        s.user,
        "diagnosed printer; firmware reflashed",
        "key-b",
    )
    .await
    .expect("upsert 2");
    assert_eq!(row.body, "diagnosed printer; firmware reflashed");

    let fetched = note_drafts::get_draft(&mut *tx, s.org, s.ticket, s.user)
        .await
        .expect("get");
    assert_eq!(
        fetched.expect("draft exists").idempotency_key,
        "key-b",
        "read-your-write on the debounced autosave"
    );

    // Delete-on-submit: only the claim-matching key clears the row.
    let cleared = note_drafts::delete_submitted(&mut *tx, s.org, s.ticket, s.user, "key-b")
        .await
        .expect("delete");
    assert!(cleared);
    assert!(
        note_drafts::get_draft(&mut *tx, s.org, s.ticket, s.user)
            .await
            .expect("get after")
            .is_none()
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn drafts_are_isolated_per_user() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 181).await;
    let other = fixed_uuid(199);
    {
        let mut tx = org_tx(&app, s.org).await;
        users::create_user(&mut *tx, other, s.org, "b199@wb.test", "B One", "argon2hash", "UTC")
            .await
            .expect("user b");
        tx.commit().await.expect("commit");
    }
    let mut tx = org_tx(&app, s.org).await;
    note_drafts::upsert_draft(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        s.user,
        "mine",
        "k1",
    )
    .await
    .expect("upsert");
    assert!(note_drafts::get_draft(&mut *tx, s.org, s.ticket, other)
        .await
        .expect("other read")
        .is_none());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn retention_purge_drops_only_stale_rows() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 182).await;
    let mut tx = org_tx(&app, s.org).await;
    note_drafts::upsert_draft(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        s.user,
        "fresh",
        "kf",
    )
    .await
    .expect("upsert");
    sqlx::query("UPDATE note_drafts SET updated_at = now() - interval '31 days'")
        .execute(&mut *tx)
        .await
        .expect("age the row");
    let purged = note_drafts::purge_stale(&mut *tx).await.expect("purge");
    assert!(purged >= 1);
    assert!(note_drafts::get_draft(&mut *tx, s.org, s.ticket, s.user)
        .await
        .expect("get")
        .is_none());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn workflow_meta_round_trips_on_the_row() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 183).await;
    let mut tx = org_tx(&app, s.org).await;

    // User-written note: meta NULL.
    let plain = tickets::insert_comment(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        Some(s.user),
        NoteKind::Internal,
        "- Checked the port",
        None,
    )
    .await
    .expect("insert plain");
    assert!(plain.workflow_meta.is_none());

    // Machine note: meta carries the workflow payload.
    let meta = serde_json::json!({
        "kind": "task_complete",
        "auto": true,
        "item_ids": [Uuid::now_v7()],
    });
    let machine = tickets::insert_comment(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        s.ticket,
        Some(s.user),
        NoteKind::Internal,
        "- **Completed:** Patch firewall",
        Some(&meta),
    )
    .await
    .expect("insert machine");
    assert!(machine.workflow_meta.is_some(), "machine note carries workflow_meta");

    // Handwritten (different kind) untouched by the classification above.
    let fetched_all = tickets::list_comments(&mut *tx, s.org, s.ticket, None)
        .await
        .expect("list");
    assert_eq!(fetched_all.len(), 2);
    assert_eq!(
        fetched_all
            .iter()
            .filter(|c| c.workflow_meta.is_some())
            .count(),
        1,
        "exactly one machine-marked note in the ledger"
    );
    tx.commit().await.expect("commit");

    assert_eq!(plain.body, "- Checked the port");
}

/// Guard: the typed 409 path's db preconditions — items and their
/// terminal flags read from the SQL layer (checklist view shape).
#[tokio::test]
async fn checklist_view_carries_na_state() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 184).await;
    let item_id = fixed_uuid(186);
    let mut tx = org_tx(&app, s.org).await;
    tickets::add_checklist_item(&mut *tx, item_id, s.org, s.ticket, "Boot scan", 0)
        .await
        .expect("add item");
    tickets::set_checklist_item(&mut *tx, s.org, item_id, false, true, Some(s.user))
        .await
        .expect("mark na");
    let (items, _) = core_db::checklists::view(&mut *tx, s.org, s.ticket)
        .await
        .expect("view");
    tx.commit().await.expect("commit");
    let na = items.iter().find(|i| i.id == item_id).expect("item");
    assert!(na.na && !na.checked);
}

#[tokio::test]
async fn rls_blocks_cross_org_draft_reads() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 185).await;
    let other = seed_org(&app, 189).await;
    {
        let mut tx = org_tx(&app, s.org).await;
        note_drafts::upsert_draft(
            &mut *tx,
            Uuid::now_v7(),
            s.org,
            s.ticket,
            s.user,
            "only mine",
            "k9",
        )
        .await
        .expect("upsert");
        tx.commit().await.expect("commit");
    }
    // A foreign org context cannot even see the row (RLS sparse).
    let mut tx = org_tx(&app, other.org).await;
    let row = tickets::list_comments(&mut *tx, other.org, s.ticket, None).await;
    assert!(matches!(row, Ok(v) if v.is_empty()));
    let hidden = note_drafts::get_draft(&mut *tx, other.org, s.ticket, other.user)
        .await
        .expect("no cross-org");
    assert!(hidden.is_none());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn ticket_draft_visibility_requires_scope() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 187).await;
    let ghost = Uuid::now_v7();
    let mut tx = org_tx(&app, s.org).await;
    assert!(
        matches!(
            tickets::get_ticket(&mut *tx, s.org, ghost).await,
            Err(DbError::NotFound)
        ),
        "unknown ticket is invisible in scope"
    );
    tx.commit().await.expect("commit");
}

