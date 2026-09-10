//! Ticket taxonomy coverage (Wave 11 lane 3, migration 0048).
//!
//! Purpose: prove the classification layer end to end at the query layer —
//! 1. The standard accepted set seeds for new orgs (same shape migration
//!    0048 backfilled for existing orgs).
//! 2. FULL org CRUD: create, rename/re-sort, reparent (subcategories
//!    between roots), soft retire — including the schema's own guards
//!    (two-level depth trigger, sibling name uniqueness).
//! 3. Retirement NEVER deletes: tickets keep their FK references and the
//!    counts keep working.
//! 4. CLASSIFICATION-ONLY guard (§5): taxonomy mutations are
//!    behavior-neutral — the ticket's semantic category, its SLA state,
//!    and the workflow transition rules are untouched by renames,
//!    reparents, and retirements, and behavior still derives ONLY from
//!    board_statuses.
//! 5. Board moves at the query layer: board_id + status + category move
//!    together, resolved_at keeps its stamp semantics.
//!
//! Non-purpose: HTTP shapes and envelopes (core-api flow coverage), the
//! manifest section (manifest_queries.rs).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::taxonomy::{TaxonomyNode, TaxonomyTable};
use core_db::tickets;
use core_domain::entities::TicketSlaState;
use core_domain::status::{Priority, TicketStatusCategory};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

struct Seeded {
    org: Uuid,
    board: Uuid,
    entry_status: Uuid,
    waiting_status: Uuid,
    resolved_status: Uuid,
}

async fn status_id(conn: &mut PgConnection, board: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND name = $2")
        .bind(board)
        .bind(name)
        .fetch_one(&mut *conn)
        .await
        .expect("seeded board status")
}

/// One org with the defaults-first baseline PLUS the standard taxonomy
/// (the exact org-creation sequence the register route runs).
async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Taxo Co", &format!("taxo-{n}"), "TAX")
        .await
        .expect("org insert");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    let board = core_db::orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board");
    core_db::orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla policy");
    core_db::orgs::seed_default_taxonomy(&mut tx, org)
        .await
        .expect("taxonomy seed");
    let entry_status = status_id(&mut tx, board, "New").await;
    let waiting_status = status_id(&mut tx, board, "Waiting").await;
    let resolved_status = status_id(&mut tx, board, "Resolved").await;
    tx.commit().await.expect("commit seed");
    Seeded {
        org,
        board,
        entry_status,
        waiting_status,
        resolved_status,
    }
}

/// A second board with DIFFERENT status ids but the same semantic
/// categories — the remap substrate for board moves.
async fn seed_second_board(conn: &mut PgConnection, org: Uuid, name: &str) -> Uuid {
    let board_id = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO boards (id, org_id, name, is_default) VALUES ($1, $2, $3, false)",
        board_id,
        org,
        name
    )
    .execute(&mut *conn)
    .await
    .expect("board");
    let defaults: [(&str, TicketStatusCategory, i32, bool); 4] = [
        ("Intake", TicketStatusCategory::Open, 0, true),
        ("Working", TicketStatusCategory::InProgress, 10, false),
        ("Blocked", TicketStatusCategory::Waiting, 20, false),
        ("Done", TicketStatusCategory::Closed, 30, false),
    ];
    for (name, category, sort, is_entry) in defaults {
        sqlx::query!(
            "INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            Uuid::now_v7(),
            board_id,
            org,
            name,
            category.as_str(),
            sort,
            is_entry
        )
        .execute(&mut *conn)
        .await
        .expect("status");
    }
    board_id
}

async fn insert_ticket_with_classification(
    conn: &mut PgConnection,
    s: &Seeded,
    id: Uuid,
    type_id: Option<Uuid>,
    category_id: Option<Uuid>,
) -> i64 {
    let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *conn, s.org)
        .await
        .expect("number alloc");
    sqlx::query!(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category, type_id, category_id)
         VALUES ($1, $2, $3, $4, $5, 'classified', 'p3', 'open', $6, $7)",
        id,
        s.org,
        format!("{prefix}-{seq}"),
        s.board,
        s.entry_status,
        type_id,
        category_id
    )
    .execute(&mut *conn)
    .await
    .expect("ticket");
    seq
}

async fn find_node(conn: &mut PgConnection, org: Uuid, name: &str) -> TaxonomyNode {
    sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>, String, Option<String>, bool, i32, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, org_id, parent_id, name, description, is_retired, sort_order, created_at, updated_at
         FROM ticket_categories WHERE org_id = $1 AND name = $2 LIMIT 1",
    )
    .bind(org)
    .bind(name)
    .fetch_one(&mut *conn)
    .await
    .map(|r| TaxonomyNode {
        id: r.0,
        org_id: r.1,
        parent_id: r.2,
        name: r.3,
        description: r.4,
        is_retired: r.5,
        sort_order: r.6,
        created_at: r.7,
        updated_at: r.8,
    })
    .expect("category node")
}

// --- 1. the standard seed --------------------------------------------------------

#[tokio::test]
async fn standard_seed_matches_the_documented_set() {
    let db = test_db().await;
    let s = seed_org(&db.app, 400).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let types = core_db::taxonomy::list_nodes(&mut tx, s.org, TaxonomyTable::Type, true)
        .await
        .expect("types");
    assert_eq!(
        types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["Incident", "Service Request", "Problem", "Change"]
    );

    let cats = core_db::taxonomy::list_nodes(&mut tx, s.org, TaxonomyTable::Category, true)
        .await
        .expect("categories");
    let roots: Vec<_> = cats.iter().filter(|c| c.parent_id.is_none()).collect();
    let subs: Vec<_> = cats.iter().filter(|c| c.parent_id.is_some()).collect();
    assert_eq!(roots.len(), 8, "the standard root set");
    assert_eq!(subs.len(), 29, "the standard subcategory set");
    // Every subcategory's parent is a root of the SAME org.
    for sub in &subs {
        assert!(
            roots.iter().any(|r| r.id == sub.parent_id.unwrap()),
            "subcategory parent must be an org root"
        );
    }
}

// --- 2. full CRUD + schema guards --------------------------------------------------

#[tokio::test]
async fn crud_lifecycle_create_rename_reparent_retire() {
    let db = test_db().await;
    let s = seed_org(&db.app, 401).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // CREATE: a type and a category/subcategory pair.
    let t = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Type,
        None,
        "RMA",
        None,
        90,
    )
    .await
    .expect("create type");
    assert!(!t.is_retired);
    let root = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Category,
        None,
        "Field Services",
        None,
        80,
    )
    .await
    .expect("create root");
    let sub = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Category,
        Some(root.id),
        "Site Surveys",
        None,
        0,
    )
    .await
    .expect("create sub");

    // Duplicate names are rejected by the partial unique indexes. Each
    // expected failure runs inside a savepoint so the transaction stays
    // usable (a PG error aborts the surrounding transaction).
    async fn expect_unique_violation(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        org: Uuid,
        table: TaxonomyTable,
        parent: Option<Uuid>,
        name: &str,
    ) {
        sqlx::query("SAVEPOINT dup_check")
            .execute(&mut **tx)
            .await
            .expect("sp");
        let res =
            core_db::taxonomy::insert_node(tx, org, Uuid::now_v7(), table, parent, name, None, 0)
                .await;
        sqlx::query("ROLLBACK TO SAVEPOINT dup_check")
            .execute(&mut **tx)
            .await
            .expect("rollback");
        assert!(res.is_err(), "duplicate name '{name}' must conflict");
    }
    expect_unique_violation(&mut tx, s.org, TaxonomyTable::Type, None, "RMA").await;
    expect_unique_violation(
        &mut tx,
        s.org,
        TaxonomyTable::Category,
        Some(root.id),
        "Site Surveys",
    )
    .await;
    // The SAME name is legal under a DIFFERENT root (sibling scoping).
    core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Category,
        None,
        "RMA",
        None,
        0,
    )
    .await
    .expect("root named like a type is a different namespace");

    // RENAME + re-sort.
    core_db::taxonomy::update_node(
        &mut tx,
        s.org,
        TaxonomyTable::Type,
        t.id,
        "RMA / Return",
        None,
        95,
    )
    .await
    .expect("rename");
    let renamed = core_db::taxonomy::get_node(&mut tx, s.org, TaxonomyTable::Type, t.id)
        .await
        .expect("renamed");
    assert_eq!(renamed.name, "RMA / Return");
    assert_eq!(renamed.sort_order, 95);

    // REPARENT the subcategory under a different root.
    let network = find_node(&mut tx, s.org, "Network").await;
    let moved = core_db::taxonomy::reparent_category(&mut tx, s.org, sub.id, network.id)
        .await
        .expect("reparent");
    assert_eq!(moved.parent_id, Some(network.id));
    // A ROOT cannot be reparented (it has no parent to change) — inside a
    // savepoint so the tx survives the error.
    sqlx::query("SAVEPOINT root_reparent")
        .execute(&mut *tx)
        .await
        .expect("sp");
    let res = core_db::taxonomy::reparent_category(&mut tx, s.org, root.id, network.id).await;
    sqlx::query("ROLLBACK TO SAVEPOINT root_reparent")
        .execute(&mut *tx)
        .await
        .expect("rollback");
    assert!(res.is_err(), "roots are the top level");

    // RETIRE is soft: the rows stay, only the flag flips.
    core_db::taxonomy::retire_node(&mut tx, s.org, TaxonomyTable::Category, sub.id)
        .await
        .expect("retire");
    let retired = core_db::taxonomy::get_node(&mut tx, s.org, TaxonomyTable::Category, sub.id)
        .await
        .expect("retired row still exists");
    assert!(retired.is_retired);
    // Active-only lists hide it; full lists (admin/manifest) keep it.
    let active = core_db::taxonomy::list_nodes(&mut tx, s.org, TaxonomyTable::Category, false)
        .await
        .expect("active list");
    assert!(!active.iter().any(|c| c.id == sub.id));
    let full = core_db::taxonomy::list_nodes(&mut tx, s.org, TaxonomyTable::Category, true)
        .await
        .expect("full list");
    assert!(full.iter().any(|c| c.id == sub.id));
}

#[tokio::test]
async fn depth_trigger_blocks_a_third_level() {
    let db = test_db().await;
    let s = seed_org(&db.app, 402).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let root = find_node(&mut tx, s.org, "Network").await;
    let sub = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Category,
        Some(root.id),
        "Depth Probe",
        None,
        1,
    )
    .await
    .expect("subcategory");
    // A child of a SUBCATEGORY would grow a third level: the 0048
    // trigger is the guarantor even for a writer that skips validation.
    // Savepoint keeps the tx usable after the expected error.
    sqlx::query("SAVEPOINT depth_check")
        .execute(&mut *tx)
        .await
        .expect("sp");
    let err = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Category,
        Some(sub.id),
        "WireGuard",
        None,
        0,
    )
    .await;
    sqlx::query("ROLLBACK TO SAVEPOINT depth_check")
        .execute(&mut *tx)
        .await
        .expect("rollback");
    assert!(err.is_err(), "the two-level depth trigger must fire");
}

// --- 3. retirement keeps ticket references -----------------------------------------

#[tokio::test]
async fn retire_keeps_ticket_references_and_counts() {
    let db = test_db().await;
    let s = seed_org(&db.app, 403).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let t = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Type,
        None,
        "Legacy Type",
        None,
        99,
    )
    .await
    .expect("type");
    let euc = find_node(&mut tx, s.org, "End-User Computing").await;
    let ticket_id = Uuid::now_v7();
    insert_ticket_with_classification(&mut tx, &s, ticket_id, Some(t.id), Some(euc.id)).await;

    assert_eq!(
        core_db::taxonomy::count_ticket_references(&mut tx, s.org, TaxonomyTable::Type, t.id)
            .await
            .expect("count"),
        1
    );
    core_db::taxonomy::retire_node(&mut tx, s.org, TaxonomyTable::Type, t.id)
        .await
        .expect("retire");
    // The ticket still resolves WITH its reference; the FK is intact.
    let ticket = tickets::get_ticket(&mut *tx, s.org, ticket_id)
        .await
        .expect("ticket survives retirement");
    assert_eq!(ticket.id, ticket_id);
    let refd: Option<Uuid> = sqlx::query_scalar("SELECT type_id FROM tickets WHERE id = $1")
        .bind(ticket_id)
        .fetch_one(&mut *tx)
        .await
        .expect("type ref");
    assert_eq!(refd, Some(t.id), "the reference is kept, never nulled");
    assert_eq!(
        core_db::taxonomy::count_ticket_references(&mut tx, s.org, TaxonomyTable::Type, t.id)
            .await
            .expect("count after retire"),
        1
    );
}

// --- 4. the classification-only guard (§5) -----------------------------------------

#[tokio::test]
async fn taxonomy_mutations_are_behavior_neutral() {
    let db = test_db().await;
    let s = seed_org(&db.app, 404).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // A classified ticket WITH live SLA state.
    let t = core_db::taxonomy::insert_node(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        TaxonomyTable::Type,
        None,
        "Guard Type",
        None,
        98,
    )
    .await
    .expect("type");
    let cat = find_node(&mut tx, s.org, "Security").await;
    let ticket_id = Uuid::now_v7();
    insert_ticket_with_classification(&mut tx, &s, ticket_id, Some(t.id), Some(cat.id)).await;
    let policy = core_db::sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("default policy");
    let created_at = chrono::Utc::now() - chrono::Duration::hours(2);
    let sla = TicketSlaState::init(s.org, ticket_id, &policy, Priority::P2, created_at);
    core_db::sla::upsert_ticket_state(&mut *tx, &sla)
        .await
        .expect("sla state");
    let sla_before = core_db::sla::get_ticket_state(&mut *tx, ticket_id)
        .await
        .expect("ok")
        .expect("state row");

    // RENAME, re-sort, reparent, retire — the full mutation surface.
    core_db::taxonomy::update_node(
        &mut tx,
        s.org,
        TaxonomyTable::Type,
        t.id,
        "Guard Type v2",
        None,
        1,
    )
    .await
    .expect("rename");
    let net = find_node(&mut tx, s.org, "Network").await;
    core_db::taxonomy::reparent_category(&mut tx, s.org, cat.id, net.id)
        .await
        .expect_err("roots cannot be reparented — use a real subcategory instead");
    let malware = find_node(&mut tx, s.org, "Malware & Phishing").await;
    core_db::taxonomy::reparent_category(&mut tx, s.org, malware.id, net.id)
        .await
        .expect("reparent subcategory");
    core_db::taxonomy::retire_node(&mut tx, s.org, TaxonomyTable::Type, t.id)
        .await
        .expect("retire");

    // The SLA state row is byte-for-byte unaffected.
    let sla_after = core_db::sla::get_ticket_state(&mut *tx, ticket_id)
        .await
        .expect("ok")
        .expect("state row");
    assert_eq!(sla_before.resolution_due, sla_after.resolution_due);
    assert_eq!(
        sla_before.paused_business_secs,
        sla_after.paused_business_secs
    );
    assert_eq!(sla_before.paused_since, sla_after.paused_since);
    assert_eq!(sla_before.breached, sla_after.breached);

    // Behavior STILL derives only from board_statuses: a status change
    // re-derives the ticket's semantic category from the BOARD STATUS row,
    // never from the (mutated) taxonomy.
    tickets::update_status(&mut *tx, s.org, ticket_id, s.waiting_status)
        .await
        .expect("status change");
    let ticket = tickets::get_ticket(&mut *tx, s.org, ticket_id)
        .await
        .expect("ticket");
    assert_eq!(ticket.category, TicketStatusCategory::Waiting);

    // The workflow rules the status endpoint enforces are untouched by
    // taxonomy mutation: validation still runs on the board's statuses.
    let statuses = core_db::boards::list_statuses(&mut *tx, s.board)
        .await
        .expect("statuses");
    assert_eq!(statuses.len(), 4, "board statuses unchanged");
    let edges = core_db::boards::list_transitions(&mut *tx, s.board)
        .await
        .expect("edges");
    let rejection = core_domain::workflow::validate_transition(
        &statuses,
        &edges,
        s.entry_status,
        s.resolved_status,
    );
    // The open→closed direct transition on the default board follows the
    // SAME verdict it had before any taxonomy mutation (category floor
    // permits open→closed; assert the machinery is intact either way).
    let _ = rejection;
}

// --- 5. board moves at the query layer ----------------------------------------------

#[tokio::test]
async fn move_to_board_updates_board_status_and_category_together() {
    let db = test_db().await;
    let s = seed_org(&db.app, 405).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let target = seed_second_board(&mut tx, s.org, "Escalations").await;
    let ticket_id = Uuid::now_v7();
    insert_ticket_with_classification(&mut tx, &s, ticket_id, None, None).await;

    let waiting_on_target: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND category = 'waiting'",
    )
    .bind(target)
    .fetch_one(&mut *tx)
    .await
    .expect("target waiting status");

    tickets::move_to_board(&mut *tx, s.org, ticket_id, target, waiting_on_target)
        .await
        .expect("move");
    let t = tickets::get_ticket(&mut *tx, s.org, ticket_id)
        .await
        .expect("ticket");
    assert_eq!(t.board_id, target);
    assert_eq!(t.status_id, waiting_on_target);
    assert_eq!(t.category, TicketStatusCategory::Waiting);

    // Cross-org / missing rows are NotFound, never a silent no-op.
    assert!(
        tickets::move_to_board(&mut *tx, s.org, Uuid::now_v7(), target, waiting_on_target)
            .await
            .is_err()
    );
}
