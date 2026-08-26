//! End-to-end core-db query-layer tests against a real Postgres 17
//! (pgvector/pgvector:pg17, prod parity).
//!
//! Purpose: prove the SQL layer — org seeding defaults, users/roles,
//! session lifecycle with replay rejection, ticket number allocation,
//! filtered lists, status transitions setting `resolved_at`, comments and
//! checklists, the single-active-timer invariant, time entries, the
//! transactional outbox (`FOR UPDATE SKIP LOCKED`), notification counts,
//! audit append-only persistence, and RLS isolation (no `app.org_id`
//! scope ⇒ no rows; foreign scope ⇒ no cross-org rows).
//!
//! Non-purpose: domain-rule testing (core-domain unit tests) and HTTP/API
//! behavior (core-api). Database selection follows the documented contract
//! in `tests/common/mod.rs`: `MERIDIAN_TEST_DATABASE_URL`, else
//! `DATABASE_URL` (CI service container), else a local testcontainer.

mod common;

use common::{fixed_uuid, org_tx, test_db, FUTURE, PAST};
use core_db::{
    audit, automations, notifications as notif, orgs, outbox, queue, sla, tickets, timers, users,
    DbError,
};
use core_domain::entities::{
    Actor, AuditEvent, BusinessCalendar, NotificationPreference, SlaCalendar, Ticket, TimeEntry,
    TimerSession,
};
use core_domain::policy::BuiltinRole;
use core_domain::status::{NoteKind, Priority, TicketStatusCategory};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

/// One freshly seeded org with default roles/board/SLA and resolved
/// board-status ids.
struct Seeded {
    org: Uuid,
    board: Uuid,
    roles: HashMap<String, Uuid>,
    entry_status: Uuid,
    waiting_status: Uuid,
    resolved_status: Uuid,
}

/// Deterministic wall-clock anchor (2026-01-15, hour granularity).
fn at(hour: u32) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 1, 15, hour, 0, 0)
        .unwrap()
}

async fn status_id(tx: &mut Transaction<'_, Postgres>, board: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND name = $2")
        .bind(board)
        .bind(name)
        .fetch_one(&mut **tx)
        .await
        .expect("seeded board status")
}

/// Seeds one org (defaults-first: roles + board + SLA policy) in a single
/// RLS-scoped transaction. Distinct `n` keeps fixed UUIDs collision-free
/// across parallel tests sharing one database.
async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("acme-{n}"), "MER")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("role seeds");
    let board = orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board seed");
    orgs::seed_default_sla_policy(&mut *tx, org)
        .await
        .expect("sla seed");
    let entry_status = status_id(&mut tx, board, "New").await;
    let waiting_status = status_id(&mut tx, board, "Waiting").await;
    let resolved_status = status_id(&mut tx, board, "Resolved").await;
    tx.commit().await.expect("commit seed");
    Seeded {
        org,
        board,
        roles,
        entry_status,
        waiting_status,
        resolved_status,
    }
}

async fn seed_user(app: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    users::create_user(
        &mut *tx,
        user,
        s.org,
        &format!("u{n}@acme.test"),
        "Test User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

/// Allocates the next number and inserts a ticket through the public API.
async fn create_ticket(
    conn: &mut PgConnection,
    s: &Seeded,
    id: Uuid,
    title: &str,
    priority: Priority,
    status: Uuid,
    assignee: Option<Uuid>,
) -> Ticket {
    let (prefix, seq) = orgs::next_ticket_number(&mut *conn, s.org)
        .await
        .expect("number alloc");
    let number = format!("{prefix}-{seq}");
    tickets::insert_ticket(
        conn,
        tickets::NewTicket {
            id,
            org_id: s.org,
            number: &number,
            board_id: s.board,
            status_id: status,
            title,
            body: None,
            priority,
            client_id: None,
            site_id: None,
            contact_id: None,
            assignee_id: assignee,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket insert")
}

fn filter_all() -> tickets::TicketFilter {
    tickets::TicketFilter {
        board_id: None,
        assignee_id: None,
        client_id: None,
        category: None,
        priority: None,
        limit: 100,
        offset: 0,
    }
}

fn is_unique_violation(err: &DbError) -> bool {
    matches!(err, DbError::Sql(sqlx::Error::Database(db)) if db.is_unique_violation())
}

/// Any FK/CHECK/UNIQUE constraint rejection surfaced by sqlx.
fn is_constraint_violation(err: &DbError) -> bool {
    matches!(err, DbError::Sql(sqlx::Error::Database(db)) if db.constraint().is_some())
}

async fn count_rows(tx: &mut Transaction<'_, Postgres>, table_sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table_sql} WHERE id = $1"))
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .expect("count")
}

// --- org registration -------------------------------------------------------

#[tokio::test]
async fn org_registration_seeds_default_roles_board_and_sla_policy() {
    let db = test_db().await;
    let s = seed_org(&db.app, 1).await;

    let org = {
        let mut tx = org_tx(&db.app, s.org).await;
        orgs::get_org(&mut *tx, s.org).await.expect("org fetch")
    };
    assert_eq!(org.name, "Acme MSP");
    assert_eq!(org.slug, "acme-1");
    assert_eq!(org.ticket_prefix, "MER");
    assert_eq!(
        org.ticket_sequence, 1000,
        "fresh org starts at sequence base"
    );

    // Five system builtin roles, keyed for assignment.
    assert_eq!(s.roles.len(), 5);
    for builtin in [
        BuiltinRole::Owner,
        BuiltinRole::Admin,
        BuiltinRole::Dispatcher,
        BuiltinRole::Technician,
        BuiltinRole::ReadOnly,
    ] {
        assert!(
            s.roles.contains_key(builtin.as_str()),
            "missing {:?}",
            builtin.as_str()
        );
    }

    // Default board with the four semantic statuses; entry = New.
    let mut tx = org_tx(&db.app, s.org).await;
    let (board_name, is_default): (String, bool) =
        sqlx::query_as("SELECT name, is_default FROM boards WHERE id = $1")
            .bind(s.board)
            .fetch_one(&mut *tx)
            .await
            .expect("board row");
    assert_eq!(board_name, "Service Desk");
    assert!(is_default);
    let statuses: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT name, category, is_entry FROM board_statuses \
         WHERE board_id = $1 ORDER BY sort_order",
    )
    .bind(s.board)
    .fetch_all(&mut *tx)
    .await
    .expect("statuses");
    assert_eq!(
        statuses,
        vec![
            ("New".to_string(), "open".to_string(), true),
            ("In Progress".to_string(), "in_progress".to_string(), false),
            ("Waiting".to_string(), "waiting".to_string(), false),
            ("Resolved".to_string(), "closed".to_string(), false),
        ]
    );

    // Default SLA policy: Standard, flagged default, P1 targets populated.
    let policy = sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("sla policy");
    assert_eq!(policy.name, "Standard");
    assert!(policy.is_default);
    assert_eq!(policy.targets[&Priority::P1].first_response_mins, 15);
    assert_eq!(policy.targets[&Priority::P1].resolution_mins, 480);
    assert_eq!(policy.calendar.timezone, "UTC");
    assert_eq!(policy.calendar.windows.len(), 7);
}

// --- users / roles / sessions -------------------------------------------------

#[tokio::test]
async fn user_create_role_assign_and_roles_for_user() {
    let db = test_db().await;
    let s = seed_org(&db.app, 2).await;
    let user = fixed_uuid(200);

    let created = {
        let mut tx = org_tx(&db.app, s.org).await;
        let u = users::create_user(
            &mut *tx,
            user,
            s.org,
            "Owner@Acme.test",
            "Owner",
            "argon2hash",
            "UTC",
        )
        .await
        .expect("create");
        // Commit explicitly: a dropped transaction rolls back.
        tx.commit().await.expect("commit user");
        u
    };
    assert_eq!(created.email, "Owner@Acme.test");
    assert_eq!(created.org_id, s.org);
    assert!(created.is_active);

    let owner_role = s.roles[BuiltinRole::Owner.as_str()];
    let mut tx = org_tx(&db.app, s.org).await;
    users::assign_role(&mut *tx, user, owner_role)
        .await
        .expect("assign");
    users::assign_role(&mut *tx, user, owner_role)
        .await
        .expect("re-assign is conflict-safe");

    let roles = users::roles_for_user(&mut *tx, user, s.org)
        .await
        .expect("roles");
    assert_eq!(roles.len(), 1, "idempotent re-assign must not duplicate");
    assert_eq!(roles[0].name, "Owner");
    assert_eq!(roles[0].builtin, Some(BuiltinRole::Owner));
    assert!(roles[0].is_system);

    // Credential lookup is case-insensitive on email.
    let creds = users::find_credentials_by_email(&mut *tx, "owner@acme.TEST")
        .await
        .expect("creds");
    assert_eq!(creds.user.id, user);
    assert_eq!(creds.password_hash, "argon2hash");
}

#[tokio::test]
async fn session_lifecycle_rotate_revoke_and_replay_rejection() {
    let db = test_db().await;
    let s = seed_org(&db.app, 3).await;
    let user = seed_user(&db.app, &s, 300).await;
    let session = fixed_uuid(301);

    {
        let mut tx = org_tx(&db.app, s.org).await;
        users::create_session(&mut *tx, session, user, "h1", Some("ci-runner"), FUTURE)
            .await
            .expect("session create");
        let found = users::find_active_session(&mut *tx, session, "h1")
            .await
            .expect("active before rotate");
        assert_eq!(found.user_id, user);
        assert_eq!(found.refresh_token_hash, "h1");
        assert!(found.revoked_at.is_none());
        // Commit so the SECURITY DEFINER bridge (separate session) sees it.
        tx.commit().await.expect("commit session");
    }

    // SECURITY DEFINER bridge resolves (user, org) with no org scope set.
    let resolved = users::resolve_session_org(&db.app, session, "h1")
        .await
        .expect("bridge resolve");
    assert_eq!(resolved, (user, s.org));

    let mut tx = org_tx(&db.app, s.org).await;
    users::rotate_session(&mut *tx, session, "h2", FUTURE)
        .await
        .expect("rotate");

    // Rotation invalidates the old secret: replaying h1 must fail.
    assert!(matches!(
        users::find_active_session(&mut *tx, session, "h1").await,
        Err(DbError::NotFound)
    ));
    assert!(users::find_active_session(&mut *tx, session, "h2")
        .await
        .is_ok());

    // Revocation ends the session even with the current secret.
    users::revoke_session(&mut *tx, session)
        .await
        .expect("revoke");
    assert!(matches!(
        users::find_active_session(&mut *tx, session, "h2").await,
        Err(DbError::NotFound)
    ));

    // Expired sessions are never active, on either lookup path.
    let expired = fixed_uuid(302);
    users::create_session(&mut *tx, expired, user, "h9", None, PAST)
        .await
        .expect("expired create");
    assert!(matches!(
        users::find_active_session(&mut *tx, expired, "h9").await,
        Err(DbError::NotFound)
    ));
    drop(tx);
    assert!(users::resolve_session_org(&db.app, expired, "h9")
        .await
        .is_err());
}

// --- tickets -------------------------------------------------------------------

#[tokio::test]
async fn ticket_number_allocations_are_distinct_and_per_org() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 4).await;
    let sb = seed_org(&db.app, 5).await;

    let mut tx_a = org_tx(&db.app, sa.org).await;
    let first = orgs::next_ticket_number(&mut *tx_a, sa.org)
        .await
        .expect("alloc 1");
    let second = orgs::next_ticket_number(&mut *tx_a, sa.org)
        .await
        .expect("alloc 2");
    assert_ne!(
        first.1, second.1,
        "sequential allocations produce distinct numbers"
    );
    assert_eq!(second.1, first.1 + 1, "sequence increments monotonically");
    assert_eq!(first.0, "MER");
    assert_eq!(first.1, 1001);

    // A different org starts from its own independent sequence.
    let mut tx_b = org_tx(&db.app, sb.org).await;
    let other = orgs::next_ticket_number(&mut *tx_b, sb.org)
        .await
        .expect("alloc B");
    assert_eq!(other.1, 1001, "per-org sequences are independent");
}

#[tokio::test]
async fn ticket_insert_persists_category_and_filters_apply() {
    let db = test_db().await;
    let s = seed_org(&db.app, 6).await;
    let u = seed_user(&db.app, &s, 600).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t1 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(601),
        "P1 outage",
        Priority::P1,
        s.entry_status,
        Some(u),
    )
    .await;
    let t2 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(602),
        "P2 request",
        Priority::P2,
        s.entry_status,
        None,
    )
    .await;
    let t3 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(603),
        "P3 question",
        Priority::P3,
        s.entry_status,
        Some(u),
    )
    .await;
    let _t4 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(604),
        "P4 backlog",
        Priority::P4,
        s.entry_status,
        None,
    )
    .await;

    // Insert re-fetch denormalizes category from the board status row.
    assert_eq!(t1.category, TicketStatusCategory::Open);
    assert_eq!(t1.priority, Priority::P1);
    assert_eq!(t1.status_id, s.entry_status);
    assert!(t1.resolved_at.is_none());

    // Full list sorts by priority rank.
    let all = tickets::list_tickets(&mut *tx, s.org, filter_all())
        .await
        .expect("list all");
    let prios: Vec<Priority> = all.iter().map(|t| t.priority).collect();
    assert_eq!(
        prios,
        vec![Priority::P1, Priority::P2, Priority::P3, Priority::P4]
    );

    // Assignee filter keeps insertion order semantics (priority-ranked).
    let mine = tickets::list_tickets(
        &mut *tx,
        s.org,
        tickets::TicketFilter {
            assignee_id: Some(u),
            ..filter_all()
        },
    )
    .await
    .expect("assignee filter");
    assert_eq!(
        mine.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t1.id, t3.id]
    );

    // Priority and category filters.
    let urgent = tickets::list_tickets(
        &mut *tx,
        s.org,
        tickets::TicketFilter {
            priority: Some(Priority::P1),
            ..filter_all()
        },
    )
    .await
    .expect("priority filter");
    assert_eq!(urgent.iter().map(|t| t.id).collect::<Vec<_>>(), vec![t1.id]);
    let closed = tickets::list_tickets(
        &mut *tx,
        s.org,
        tickets::TicketFilter {
            category: Some(TicketStatusCategory::Closed),
            ..filter_all()
        },
    )
    .await
    .expect("category filter");
    assert!(closed.is_empty(), "nothing closed yet");

    // Pagination slices the ranked list.
    let page = tickets::list_tickets(
        &mut *tx,
        s.org,
        tickets::TicketFilter {
            limit: 2,
            offset: 1,
            ..filter_all()
        },
    )
    .await
    .expect("page");
    assert_eq!(
        page.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t2.id, t3.id]
    );

    // My-work view: assigned, not closed, priority-ranked.
    let work = tickets::my_work(&mut *tx, s.org, u, 50)
        .await
        .expect("my work");
    assert_eq!(
        work.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![t1.id, t3.id]
    );

    // Lookup by number round-trips.
    let by_number = tickets::get_ticket_by_number(&mut *tx, s.org, &t1.number.0.to_string())
        .await
        .expect("by number");
    assert_eq!(by_number.id, t1.id);
}

#[tokio::test]
async fn status_transition_sets_resolved_at_once() {
    let db = test_db().await;
    let s = seed_org(&db.app, 7).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(700),
        "Broken printer",
        Priority::P2,
        s.entry_status,
        None,
    )
    .await;
    assert!(t.resolved_at.is_none());

    // Closing stamps resolved_at exactly once.
    tickets::update_status(&mut *tx, s.org, t.id, s.resolved_status)
        .await
        .expect("close");
    let closed = tickets::get_ticket(&mut *tx, s.org, t.id)
        .await
        .expect("closed");
    let stamp = closed.resolved_at.expect("resolved_at set on close");
    assert_eq!(closed.category, TicketStatusCategory::Closed);

    // Reopening preserves the original resolution timestamp…
    tickets::update_status(&mut *tx, s.org, t.id, s.waiting_status)
        .await
        .expect("reopen");
    let reopened = tickets::get_ticket(&mut *tx, s.org, t.id)
        .await
        .expect("reopened");
    assert_eq!(reopened.category, TicketStatusCategory::Waiting);
    assert_eq!(
        reopened.resolved_at,
        Some(stamp),
        "reopen keeps original stamp"
    );

    // …and closing again never overwrites it.
    tickets::update_status(&mut *tx, s.org, t.id, s.resolved_status)
        .await
        .expect("re-close");
    let reclosed = tickets::get_ticket(&mut *tx, s.org, t.id)
        .await
        .expect("reclosed");
    assert_eq!(reclosed.resolved_at, Some(stamp));

    // Transitioning an unknown ticket is NotFound, not silent success.
    assert!(matches!(
        tickets::update_status(&mut *tx, s.org, fixed_uuid(701), s.resolved_status).await,
        Err(DbError::NotFound)
    ));
}

// --- comments / checklist ------------------------------------------------------

#[tokio::test]
async fn comments_support_kind_filtered_listing() {
    let db = test_db().await;
    let s = seed_org(&db.app, 8).await;
    let u = seed_user(&db.app, &s, 800).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(801),
        "VPN drops",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    let c1 = tickets::insert_comment(
        &mut *tx,
        fixed_uuid(802),
        s.org,
        t.id,
        Some(u),
        NoteKind::Internal,
        "internal diagnosis",
    )
    .await
    .expect("comment 1");
    let c2 = tickets::insert_comment(
        &mut *tx,
        fixed_uuid(803),
        s.org,
        t.id,
        None,
        NoteKind::ClientVisible,
        "client update",
    )
    .await
    .expect("comment 2");
    let c3 = tickets::insert_comment(
        &mut *tx,
        fixed_uuid(804),
        s.org,
        t.id,
        None,
        NoteKind::InHouseUpdate,
        "handover",
    )
    .await
    .expect("comment 3");

    let all = tickets::list_comments(&mut *tx, s.org, t.id, None)
        .await
        .expect("all comments");
    assert_eq!(
        all.iter()
            .map(|c| c.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3,
        "three comments stored"
    );
    assert_eq!(c1.kind, NoteKind::Internal);
    assert_eq!(c2.author_id, None);
    assert_eq!(c3.ticket_id, t.id);

    let client_only =
        tickets::list_comments(&mut *tx, s.org, t.id, Some(&[NoteKind::ClientVisible]))
            .await
            .expect("client filter");
    assert_eq!(
        client_only.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![c2.id]
    );

    let internal_mix = tickets::list_comments(
        &mut *tx,
        s.org,
        t.id,
        Some(&[NoteKind::Internal, NoteKind::InHouseUpdate]),
    )
    .await
    .expect("internal mix");
    assert_eq!(internal_mix.len(), 2);
}

#[tokio::test]
async fn checklist_items_sort_toggle_and_missing_notfound() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9).await;
    let u = seed_user(&db.app, &s, 900).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(901),
        "Onboarding",
        Priority::P4,
        s.entry_status,
        None,
    )
    .await;
    tickets::add_checklist_item(&mut *tx, fixed_uuid(902), s.org, t.id, "step b", 1)
        .await
        .expect("item b");
    tickets::add_checklist_item(&mut *tx, fixed_uuid(903), s.org, t.id, "step a", 0)
        .await
        .expect("item a");

    let listed = tickets::list_checklist(&mut *tx, s.org, t.id)
        .await
        .expect("list");
    assert_eq!(
        listed.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
        vec!["step a", "step b"],
        "ordered by sort_order regardless of insert order"
    );
    assert!(!listed[0].checked);

    tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(903), true, Some(u))
        .await
        .expect("check");
    let listed = tickets::list_checklist(&mut *tx, s.org, t.id)
        .await
        .expect("list 2");
    assert!(listed[0].checked);
    assert_eq!(listed[0].checked_by, Some(u));
    assert!(listed[0].checked_at.is_some());

    // Unchecking clears both the auditor and the timestamp.
    tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(903), false, None)
        .await
        .expect("uncheck");
    let listed = tickets::list_checklist(&mut *tx, s.org, t.id)
        .await
        .expect("list 3");
    assert!(!listed[0].checked);
    assert_eq!(listed[0].checked_by, None);
    assert!(listed[0].checked_at.is_none());

    assert!(matches!(
        tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(999), true, None).await,
        Err(DbError::NotFound)
    ));
}

// --- timers / time entries -----------------------------------------------------

#[tokio::test]
async fn timer_enforces_single_active_session_per_user() {
    let db = test_db().await;
    let s = seed_org(&db.app, 10).await;
    let u = seed_user(&db.app, &s, 1000).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(1001),
        "Server fan noise",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    tx.commit().await.expect("commit ticket");

    let mut tx = org_tx(&db.app, s.org).await;
    let mut s1 = TimerSession::start(s.org, u, t.id, at(9));
    timers::insert_session(&mut *tx, &s1)
        .await
        .expect("start timer 1");
    tx.commit().await.expect("commit timer 1");

    let mut tx = org_tx(&db.app, s.org).await;
    let active = timers::active_session(&mut *tx, u).await.expect("active");
    assert_eq!(active.expect("one running").id, s1.id);

    // Pausing records the reason in the segment history.
    s1.pause(at(10), Some("vendor call".into())).expect("pause");
    timers::update_session(&mut *tx, &s1)
        .await
        .expect("save pause");
    tx.commit().await.expect("commit pause");

    // Second active timer for the SAME user violates the partial unique
    // index. Run in its own transaction: the expected failure aborts it.
    let s2 = TimerSession::start(s.org, u, t.id, at(11));
    let mut violating = org_tx(&db.app, s.org).await;
    let err = timers::insert_session(&mut *violating, &s2)
        .await
        .expect_err("must violate");
    drop(violating); // roll back the aborted transaction
    assert!(
        is_unique_violation(&err),
        "expected unique violation, got {err:?}"
    );

    // Stopping the first frees the slot; the second then starts cleanly.
    let mut tx = org_tx(&db.app, s.org).await;
    s1.stop(at(12), None).expect("stop domain rule");
    timers::update_session(&mut *tx, &s1)
        .await
        .expect("save stop");
    timers::insert_session(&mut *tx, &s2)
        .await
        .expect("start timer 2");
    tx.commit().await.expect("commit switch");

    let mut tx = org_tx(&db.app, s.org).await;
    let active = timers::active_session(&mut *tx, u).await.expect("active 2");
    assert_eq!(active.expect("running s2").id, s2.id);

    let recent = timers::recent_sessions(&mut *tx, u, 10)
        .await
        .expect("recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, s2.id, "running session docks first");
    assert_eq!(recent[1].stopped_at, Some(at(12)));

    // Segment history survives the JSONB round-trip.
    let stored = timers::get_session(&mut *tx, s1.id)
        .await
        .expect("stored s1");
    assert_eq!(stored.segments.len(), 1);
    assert_eq!(
        stored.segments[0].end_kind,
        Some(core_domain::entities::SegmentEnd::Paused)
    );
    assert_eq!(stored.segments[0].reason.as_deref(), Some("vendor call"));
}

#[tokio::test]
async fn time_entries_insert_list_and_daily_total() {
    let db = test_db().await;
    let s = seed_org(&db.app, 11).await;
    let u = seed_user(&db.app, &s, 1100).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t1 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(1101),
        "Mail migration",
        Priority::P2,
        s.entry_status,
        None,
    )
    .await;
    let t2 = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(1102),
        "Switch config",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;

    let e1 = TimeEntry {
        id: fixed_uuid(1103),
        org_id: s.org,
        ticket_id: t1.id,
        user_id: u,
        started_at: at(9),
        ended_at: at(9) + chrono::Duration::minutes(30),
        minutes: 30,
        billable: true,
        notes: Some("remote hands".into()),
        work_type: Some("incident".into()),
        timer_session_id: None,
        created_at: at(9),
    };
    let e2 = TimeEntry {
        id: fixed_uuid(1104),
        org_id: s.org,
        ticket_id: t2.id,
        user_id: u,
        started_at: at(13),
        ended_at: at(13) + chrono::Duration::minutes(90),
        minutes: 90,
        billable: false,
        notes: None,
        work_type: None,
        timer_session_id: None,
        created_at: at(13),
    };
    timers::insert_time_entry(&mut *tx, &e1)
        .await
        .expect("entry 1");
    timers::insert_time_entry(&mut *tx, &e2)
        .await
        .expect("entry 2");

    // Most recent first.
    let listed = timers::list_time_entries(&mut *tx, s.org, None, None, None, 100)
        .await
        .expect("list");
    assert_eq!(
        listed.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![e2.id, e1.id]
    );

    // Per-ticket filter.
    let for_t1 = timers::list_time_entries(&mut *tx, s.org, Some(t1.id), None, None, 100)
        .await
        .expect("ticket filter");
    assert_eq!(for_t1.iter().map(|e| e.id).collect::<Vec<_>>(), vec![e1.id]);

    // Since-filter excludes the morning entry.
    let since_noon = timers::list_time_entries(&mut *tx, s.org, None, None, Some(at(12)), 100)
        .await
        .expect("since filter");
    assert_eq!(
        since_noon.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![e2.id]
    );

    // Missing-time audit input: total minutes for the UTC day window.
    let total = timers::minutes_logged_on(&mut *tx, u, s.org, at(0), at(23))
        .await
        .expect("daily total");
    assert_eq!(total, 120);

    let fetched = timers::get_time_entry(&mut *tx, s.org, e1.id)
        .await
        .expect("fetch");
    assert_eq!(fetched.minutes, 30);
    assert!(fetched.billable);
}

// --- transactional outbox --------------------------------------------------------

#[tokio::test]
async fn outbox_skip_locked_prevents_double_delivery_and_mark_publishes() {
    let db = test_db().await;
    let s = seed_org(&db.app, 12).await;

    // Three unpublished events, inserted the way mutations do (in-tx).
    let ids: Vec<Uuid> = (1200..1203).map(fixed_uuid).collect();
    for (i, id) in ids.iter().enumerate() {
        let mut tx = org_tx(&db.app, s.org).await;
        outbox::insert(
            &mut tx,
            *id,
            s.org,
            "ticket.created",
            "ticket",
            Some(&format!("TK-{i}")),
            &serde_json::json!({ "title": format!("event {i}") }),
        )
        .await
        .expect("outbox insert");
        tx.commit().await.expect("commit event");
    }

    // Worker 1 locks the batch FOR UPDATE…
    let mut worker1 = org_tx(&db.app, s.org).await;
    let batch = outbox::fetch_unpublished(&mut *worker1, 100)
        .await
        .expect("batch");
    let mut batch_ids: Vec<Uuid> = batch.iter().map(|r| r.id).collect();
    batch_ids.sort();
    let mut expected = ids.clone();
    expected.sort();
    assert_eq!(batch_ids, expected, "worker sees all unpublished events");
    assert_eq!(batch[0].event_type, "ticket.created");
    assert_eq!(batch[0].payload_schema_ref, "core-domain/v1");

    // …so worker 2 concurrently skips every locked row (empty batch).
    let mut worker2 = org_tx(&db.app, s.org).await;
    let stolen = outbox::fetch_unpublished(&mut *worker2, 100)
        .await
        .expect("skip locked");
    assert!(stolen.is_empty(), "SKIP LOCKED must hide locked rows");
    drop(worker2);

    // Relay succeeds → publish → nothing left for anyone.
    outbox::mark_published(&mut *worker1, &ids)
        .await
        .expect("publish");
    worker1.commit().await.expect("commit relay");
    let mut worker3 = org_tx(&db.app, s.org).await;
    let drained = outbox::fetch_unpublished(&mut *worker3, 100)
        .await
        .expect("drain");
    assert!(drained.is_empty(), "published events are not re-fetched");

    // Empty publish batches are a no-op.
    outbox::mark_published(&mut *worker3, &[])
        .await
        .expect("noop");
}

// --- notifications ---------------------------------------------------------------

#[tokio::test]
async fn notifications_track_read_state_and_counts_per_org() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 13).await;
    let ua = seed_user(&db.app, &sa, 1300).await;

    let n1 = fixed_uuid(1301);
    let n2 = fixed_uuid(1302);
    let n3 = fixed_uuid(1303);
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        notif::insert(
            &mut *tx,
            n1,
            sa.org,
            ua,
            "mention",
            None,
            "You were mentioned",
            Some("check ticket"),
        )
        .await
        .expect("n1");
        notif::insert(
            &mut *tx,
            n2,
            sa.org,
            ua,
            "assignment",
            None,
            "Assigned to you",
            None,
        )
        .await
        .expect("n2");
        notif::insert(
            &mut *tx,
            n3,
            sa.org,
            ua,
            "sla_risk",
            None,
            "SLA at risk",
            None,
        )
        .await
        .expect("n3");
        tx.commit().await.expect("commit notifs");
    }

    let mut tx = org_tx(&db.app, sa.org).await;
    assert_eq!(
        notif::unread_count(&mut *tx, sa.org, ua)
            .await
            .expect("count"),
        3
    );

    notif::mark_read(&mut *tx, sa.org, ua, n1)
        .await
        .expect("read");
    tx.commit().await.expect("commit read");

    // Double-read is rejected (read_at IS NULL guard); runs in its own
    // transaction because the expected failure aborts it.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        assert!(
            matches!(
                notif::mark_read(&mut *tx, sa.org, ua, n1).await,
                Err(DbError::NotFound)
            ),
            "double-read must not double-count"
        );
    }

    let mut tx = org_tx(&db.app, sa.org).await;
    assert_eq!(
        notif::unread_count(&mut *tx, sa.org, ua)
            .await
            .expect("count"),
        2
    );

    let unread = notif::list_for_user(&mut *tx, sa.org, ua, true, 100)
        .await
        .expect("unread list");
    let unread_ids: std::collections::HashSet<Uuid> = unread.iter().map(|n| n.id).collect();
    assert_eq!(unread_ids, std::collections::HashSet::from([n2, n3]));
    assert!(unread.iter().all(|n| n.read_at.is_none()));

    // Another org's scope sees zero notifications for this user.
    let sb = seed_org(&db.app, 14).await;
    assert_eq!(
        notif::unread_count(&mut *tx, sb.org, ua)
            .await
            .expect("foreign count"),
        0
    );

    // Preferences upsert idempotently per (user, kind).
    let pref = |email: bool| NotificationPreference {
        user_id: ua,
        kind: core_domain::entities::NotificationKind::Mention,
        in_app: true,
        email,
        quiet_from: Some("22:00".into()),
        quiet_until: Some("07:00".into()),
        timezone: "America/New_York".into(),
    };
    notif::upsert_preference(&mut *tx, &pref(false))
        .await
        .expect("pref 1");
    notif::upsert_preference(&mut *tx, &pref(true))
        .await
        .expect("pref 2");
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notification_preferences WHERE user_id = $1 AND kind = 'mention'",
    )
    .bind(ua)
    .fetch_one(&mut *tx)
    .await
    .expect("pref rows");
    let email: bool = sqlx::query_scalar(
        "SELECT email FROM notification_preferences WHERE user_id = $1 AND kind = 'mention'",
    )
    .bind(ua)
    .fetch_one(&mut *tx)
    .await
    .expect("pref email");
    assert_eq!(rows, 1, "upsert must not duplicate preference rows");
    assert!(email, "last write wins");
}

#[tokio::test]
async fn notifications_cursor_pagination_walks_newest_first() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 18).await;
    let ua = seed_user(&db.app, &sa, 1800).await;

    let mut ids = Vec::new();
    for i in 0..5 {
        let id = fixed_uuid(1401 + i);
        let mut tx = org_tx(&db.app, sa.org).await;
        notif::insert(
            &mut *tx,
            id,
            sa.org,
            ua,
            "mention",
            None,
            &format!("n{i}"),
            None,
        )
        .await
        .expect("insert");
        tx.commit().await.expect("commit");
        ids.push(id);
    }

    // Page size 2: walk the full keyset chain.
    let mut seen = Vec::new();
    let mut cursor: Option<notif::NotificationCursor> = None;
    loop {
        let mut tx = org_tx(&db.app, sa.org).await;
        let (items, next) = notif::list_page(&mut *tx, sa.org, ua, false, cursor, 2)
            .await
            .expect("page");
        tx.commit().await.expect("commit");
        assert!(items.len() <= 2);
        for item in &items {
            seen.push(item.id);
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(
        seen,
        ids.iter().rev().copied().collect::<Vec<_>>(),
        "newest first"
    );

    // Exhausted stream: a cursor from the last page yields an empty page
    // with next_cursor = None.
    let mut tx = org_tx(&db.app, sa.org).await;
    let (_, next) = notif::list_page(&mut *tx, sa.org, ua, false, cursor, 2)
        .await
        .expect("tail page");
    assert!(next.is_none());

    // unread_only composes with the keyset.
    notif::mark_read(&mut *tx, sa.org, ua, ids[4])
        .await
        .expect("read");
    let (unread, next) = notif::list_page(&mut *tx, sa.org, ua, true, None, 10)
        .await
        .expect("unread page");
    assert_eq!(unread.len(), 4, "read one excluded");
    assert!(unread.iter().all(|n| n.id != ids[4]));
    assert!(next.is_none());
}

#[tokio::test]
async fn notifications_bulk_and_all_mark_read_are_idempotent() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 19).await;
    let sb = seed_org(&db.app, 20).await;
    let ua = seed_user(&db.app, &sa, 1900).await;

    let n1 = fixed_uuid(1901);
    let n2 = fixed_uuid(1902);
    let unknown = fixed_uuid(1903); // never inserted anywhere
    let cross_org = fixed_uuid(1904); // seeded under another org_id
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        for (id, title) in [(n1, "a"), (n2, "b")] {
            notif::insert(&mut *tx, id, sa.org, ua, "mention", None, title, None)
                .await
                .expect("insert");
        }
        tx.commit().await.expect("commit");
    }
    // Owner-pool seed: a row whose org scope differs from every app-tx
    // scope used below — proves org scoping, not just id matching.
    notif::insert(
        &db.admin,
        cross_org,
        sb.org,
        ua,
        "mention",
        None,
        "other-org",
        None,
    )
    .await
    .expect("seed cross-org note");

    {
        // Unknown ids flip nothing.
        let mut tx = org_tx(&db.app, sa.org).await;
        let flipped = notif::mark_read_many(&mut *tx, sa.org, ua, &[unknown])
            .await
            .expect("bulk");
        assert_eq!(flipped, 0);

        // One real unread flips exactly one row.
        let flipped = notif::mark_read_many(&mut *tx, sa.org, ua, &[n1])
            .await
            .expect("bulk");
        assert_eq!(flipped, 1);

        // Retrying the same batch flips only the still-unread row; the
        // cross-org row stays untouched despite being in the batch.
        let again = notif::mark_read_many(&mut *tx, sa.org, ua, &[n1, n2, cross_org])
            .await
            .expect("retry");
        assert_eq!(again, 1, "idempotent under retry");

        assert_eq!(
            notif::unread_count(&mut *tx, sa.org, ua).await.unwrap(),
            0,
            "all read"
        );
        let all_again = notif::mark_all_read(&mut *tx, sa.org, ua)
            .await
            .expect("all");
        assert_eq!(all_again, 0, "nothing left to flip");
    }
}

#[tokio::test]
async fn notification_preferences_resolve_defaults_first_chain() {
    use core_db::notifications::{effective_preference, upsert_org_default, OrgDefaultRow};

    let db = test_db().await;
    let sa = seed_org(&db.app, 21).await;
    let ua = seed_user(&db.app, &sa, 2100).await;

    // 1. Built-in fallback when neither user nor org rows exist.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        let eff = effective_preference(&mut *tx, sa.org, ua, "sla_breach")
            .await
            .expect("builtin");
        assert_eq!(eff.source, "builtin");
        assert!(eff.in_app && !eff.email);
        assert!(eff.quiet_from.is_none() && eff.quiet_until.is_none());
        assert_eq!(eff.timezone, "UTC");
    }

    // 2. Org default supplies values absent a user override.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        upsert_org_default(
            &mut *tx,
            &OrgDefaultRow {
                org_id: sa.org,
                kind: "sla_breach".into(),
                in_app: true,
                email: true,
                quiet_from: Some("21:00".into()),
                quiet_until: Some("05:00".into()),
                timezone: "Europe/Berlin".into(),
            },
        )
        .await
        .expect("org default");
        tx.commit().await.expect("commit");

        let mut tx = org_tx(&db.app, sa.org).await;
        let eff = effective_preference(&mut *tx, sa.org, ua, "sla_breach")
            .await
            .expect("org default");
        assert_eq!(eff.source, "org_default");
        assert!(eff.email, "org default enables email");
        assert_eq!(eff.timezone, "Europe/Berlin");
    }

    // 3. A user row wins WHOLE-ROW over the org default.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        notif::upsert_preference(
            &mut *tx,
            &NotificationPreference {
                user_id: ua,
                kind: core_domain::entities::NotificationKind::SlaBreach,
                in_app: true,
                email: false,
                quiet_from: Some("23:00".into()),
                quiet_until: Some("06:00".into()),
                timezone: "Asia/Tokyo".into(),
            },
        )
        .await
        .expect("user pref");
        tx.commit().await.expect("commit");

        let mut tx = org_tx(&db.app, sa.org).await;
        let eff = effective_preference(&mut *tx, sa.org, ua, "sla_breach")
            .await
            .expect("user");
        assert_eq!(eff.source, "user");
        assert!(!eff.email, "user override replaces the org row entirely");
        assert_eq!(eff.quiet_from.as_deref(), Some("23:00"));
        assert_eq!(eff.timezone, "Asia/Tokyo");
    }
}

// --- audit -----------------------------------------------------------------------

#[tokio::test]
async fn audit_events_insert_and_list_with_actor_roundtrip() {
    let db = test_db().await;
    let s = seed_org(&db.app, 15).await;
    let u = seed_user(&db.app, &s, 1500).await;

    let human = AuditEvent {
        id: fixed_uuid(1501),
        org_id: s.org,
        actor: Actor::Human(u),
        action: "auth.login".into(),
        entity_type: None,
        entity_id: None,
        detail: serde_json::json!({ "mfa": false }),
        ip: Some("203.0.113.7".into()),
        occurred_at: at(9),
    };
    let system = AuditEvent {
        id: fixed_uuid(1502),
        org_id: s.org,
        actor: Actor::System,
        action: "sla.evaluated".into(),
        entity_type: Some("ticket".into()),
        entity_id: Some("TK-1500".into()),
        detail: serde_json::json!({}),
        ip: None,
        occurred_at: at(10),
    };
    let mut tx = org_tx(&db.app, s.org).await;
    audit::insert(&mut *tx, &human).await.expect("audit 1");
    audit::insert(&mut *tx, &system).await.expect("audit 2");

    // Newest-first listing with faithful actor-kind mapping.
    let listed = audit::list(&mut *tx, s.org, 100, None).await.expect("list");
    assert_eq!(
        listed.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![system.id, human.id],
        "ordered by occurred_at DESC"
    );
    assert_eq!(listed[0].actor, Actor::System);
    assert_eq!(listed[1], human);
    assert_eq!(listed[1].action, "auth.login");

    // Cursor variant: strictly-before boundary excludes the boundary row.
    let older = audit::list(&mut *tx, s.org, 100, Some(at(10)))
        .await
        .expect("cursor");
    assert_eq!(
        older.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![human.id]
    );

    // Append-only doctrine: the crate exposes no delete/update path, so the
    // invariant holds by construction — assert inserts remain readable.
    let again = audit::list(&mut *tx, s.org, 100, None)
        .await
        .expect("re-list");
    assert_eq!(again.len(), 2);
}

// --- RLS isolation ----------------------------------------------------------------

#[tokio::test]
async fn rls_blocks_cross_org_and_unscoped_reads() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 16).await;
    let ua = seed_user(&db.app, &sa, 1600).await;

    let ta = {
        let mut tx = org_tx(&db.app, sa.org).await;
        let t = create_ticket(
            &mut tx,
            &sa,
            fixed_uuid(1601),
            "Org A secret",
            Priority::P1,
            sa.entry_status,
            None,
        )
        .await;
        tx.commit().await.expect("commit ticket A");
        t
    };

    let sb = seed_org(&db.app, 17).await;
    let ub = seed_user(&db.app, &sb, 1602).await;
    {
        let mut tx = org_tx(&db.app, sb.org).await;
        create_ticket(
            &mut tx,
            &sb,
            fixed_uuid(1603),
            "Org B secret",
            Priority::P2,
            sb.entry_status,
            None,
        )
        .await;
        tx.commit().await.expect("commit ticket B");
    }

    // No scope at all: every user-data table yields zero rows (default-deny).
    // Uses a VIRGIN pool: Postgres keeps custom GUCs defined after a
    // transaction-local set ends, so on reused sessions
    // `current_setting('app.org_id', true)` returns '' and policies raise
    // 22P02 instead of filtering — a fail-closed finding reported to the
    // operator, not asserted away here.
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("begin unscoped");
        assert_eq!(
            count_rows(&mut tx, "orgs", sa.org).await,
            0,
            "orgs hidden unscoped"
        );
        assert_eq!(count_rows(&mut tx, "tickets", ta.id).await, 0);
        assert_eq!(count_rows(&mut tx, "users", ua).await, 0);
    }

    // Foreign scope: org B cannot see org A's rows anywhere.
    {
        let mut tx = org_tx(&db.app, sb.org).await;
        assert_eq!(
            count_rows(&mut tx, "tickets", ta.id).await,
            0,
            "cross-org tickets hidden"
        );
        assert_eq!(
            count_rows(&mut tx, "users", ua).await,
            0,
            "cross-org users hidden"
        );
        // Query layer agrees: parameterized org + RLS ⇒ NotFound.
        assert!(matches!(
            tickets::get_ticket(&mut *tx, sa.org, ta.id).await,
            Err(DbError::NotFound)
        ));
        let foreign = users::list_users(&mut *tx, sa.org)
            .await
            .expect("foreign list");
        assert!(foreign.is_empty());
    }

    // Own scope: everything visible exactly once.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        assert_eq!(count_rows(&mut tx, "tickets", ta.id).await, 1);
        assert_eq!(count_rows(&mut tx, "users", ua).await, 1);
        assert_eq!(count_rows(&mut tx, "users", ub).await, 0);
        assert_eq!(count_rows(&mut tx, "tickets", fixed_uuid(1603)).await, 0);
    }
}

// --- workflow customization v1 (board statuses / transitions) -------------------

use core_db::boards;
use core_domain::workflow::TransitionEdge;

fn edge(a: Uuid, b: Uuid) -> TransitionEdge {
    TransitionEdge::new(a, b)
}

#[tokio::test]
async fn transitions_replace_and_list_roundtrip() {
    let db = test_db().await;
    let s = seed_org(&db.app, 26).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // Empty by default → the semantic floor governs.
    assert!(boards::list_transitions(&mut *tx, s.board)
        .await
        .expect("initial transitions")
        .is_empty());

    let in_progress = status_id(&mut tx, s.board, "In Progress").await;
    boards::replace_transitions(
        &mut tx,
        s.org,
        s.board,
        &[
            edge(s.entry_status, in_progress),
            edge(in_progress, s.resolved_status),
        ],
    )
    .await
    .expect("replace");
    let edges = boards::list_transitions(&mut *tx, s.board)
        .await
        .expect("edges after replace");
    assert_eq!(edges.len(), 2);
    assert!(edges.contains(&edge(s.entry_status, in_progress)));
    assert!(edges.contains(&edge(in_progress, s.resolved_status)));

    // Replacement is authoritative: a fresh empty list returns to floor mode.
    boards::replace_transitions(&mut tx, s.org, s.board, &[])
        .await
        .expect("clear");
    assert!(boards::list_transitions(&mut *tx, s.board)
        .await
        .expect("cleared")
        .is_empty());
}

#[tokio::test]
async fn transition_edge_with_foreign_status_is_rejected_by_fk() {
    let db = test_db().await;
    let s = seed_org(&db.app, 22).await;
    let mut tx = org_tx(&db.app, s.org).await;
    // A second board in the same org owns the foreign status. Bare
    // db-layer boards carry no statuses until seeded, so add one directly.
    let other_board =
        boards::create_board(&mut *tx, s.org, Uuid::now_v7(), "Overflow", None, false)
            .await
            .expect("second board")
            .id;
    let foreign = boards::add_status(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        other_board,
        "New",
        TicketStatusCategory::Open,
        0,
        true,
    )
    .await
    .expect("foreign status")
    .id;

    let result =
        boards::replace_transitions(&mut tx, s.org, s.board, &[edge(s.entry_status, foreign)])
            .await;
    assert!(
        result
            .as_ref()
            .err()
            .map(is_constraint_violation)
            .unwrap_or(false),
        "cross-board edge must violate the same-board FK, got {result:?}"
    );
}

#[tokio::test]
async fn status_patch_updates_binding_metadata() {
    let db = test_db().await;
    let s = seed_org(&db.app, 23).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let waiting = status_id(&mut tx, s.board, "Waiting").await;

    let updated = boards::update_status(
        &mut tx,
        waiting,
        boards::StatusPatch {
            name: Some("Blocked On Vendor"),
            category: Some(TicketStatusCategory::InProgress),
            sort_order: Some(42),
            is_entry: None,
        },
    )
    .await
    .expect("patch");
    assert_eq!(updated.name, "Blocked On Vendor");
    assert_eq!(updated.category, TicketStatusCategory::InProgress);
    assert_eq!(updated.sort_order, 42);
    assert!(!updated.is_entry);

    // Category re-binding is behavior, not cosmetics: SLA pause semantics
    // now follow InProgress for this label.
    assert!(!updated.category.pauses_sla());

    assert!(matches!(
        boards::update_status(&mut tx, Uuid::now_v7(), boards::StatusPatch::default()).await,
        Err(DbError::NotFound)
    ));
}

#[tokio::test]
async fn status_delete_rejects_when_tickets_reference_it() {
    let db = test_db().await;
    let s = seed_org(&db.app, 24).await;
    let user = seed_user(&db.app, &s, 240).await;

    // The referencing ticket makes deletion impossible.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let _ticket = create_ticket(
            &mut tx,
            &s,
            fixed_uuid(2401),
            "delete guard",
            Priority::P3,
            s.waiting_status,
            Some(user),
        )
        .await;

        let blocked = boards::delete_status(&mut *tx, s.waiting_status).await;
        assert!(
            blocked
                .as_ref()
                .err()
                .map(is_constraint_violation)
                .unwrap_or(false),
            "RESTRICT must protect referenced statuses, got {blocked:?}"
        );
        // Transaction aborts on the failed statement; drop it and start clean.
    }

    // An unused custom status deletes cleanly.
    let mut tx = org_tx(&db.app, s.org).await;
    let custom = boards::add_status(
        &mut tx,
        s.org,
        Uuid::now_v7(),
        s.board,
        "Escalated",
        TicketStatusCategory::Waiting,
        25,
        false,
    )
    .await
    .expect("custom");
    boards::delete_status(&mut *tx, custom.id)
        .await
        .expect("unused delete");
    assert!(matches!(
        boards::get_status(&mut *tx, custom.id).await,
        Err(DbError::NotFound)
    ));
}

/// Re-applies migration 0011's defensive seed + entry backfill against a
/// board stripped to a single non-entry status — proving the SQL restores
/// full category coverage and a valid entry without touching healthy data.
#[tokio::test]
async fn migration_seed_logic_restores_missing_category_defaults() {
    let db = test_db().await;
    let s = seed_org(&db.app, 25).await;
    let bare_board = {
        let mut tx = org_tx(&db.app, s.org).await;
        let b = boards::create_board(&mut *tx, s.org, Uuid::now_v7(), "Legacy", None, false)
            .await
            .expect("bare board")
            .id;
        tx.commit().await.expect("commit board");
        b
    };
    {
        let mut admin = db.admin.begin().await.expect("admin tx");
        sqlx::query("DELETE FROM board_statuses WHERE board_id = $1 AND name <> 'Resolved'")
            .bind(bare_board)
            .execute(&mut *admin)
            .await
            .expect("strip");
        sqlx::query("UPDATE board_statuses SET is_entry = false WHERE board_id = $1")
            .bind(bare_board)
            .execute(&mut *admin)
            .await
            .expect("no entry");
        admin.commit().await.expect("commit strip");
    }

    // The exact statements from migrations/0011_board_statuses.sql.
    let mut admin = db.admin.begin().await.expect("admin tx 2");
    sqlx::query(
        r#"INSERT INTO board_statuses (id, board_id, org_id, name, category, sort_order, is_entry)
           SELECT gen_random_uuid(), b.id, b.org_id,
                  d.name || CASE WHEN EXISTS (
                    SELECT 1 FROM board_statuses x
                    WHERE x.board_id = b.id AND x.name = d.name AND x.category <> d.category
                  ) THEN ' (default)' ELSE '' END,
                  d.category, d.sort_order, false
           FROM boards b
           CROSS JOIN (VALUES ('New'::text,'open'::text,0), ('In Progress','in_progress',10),
                              ('Waiting','waiting',20), ('Resolved','closed',30))
                AS d(name, category, sort_order)
           WHERE NOT EXISTS (
             SELECT 1 FROM board_statuses s2
             WHERE s2.board_id = b.id AND s2.category = d.category
           )"#,
    )
    .execute(&mut *admin)
    .await
    .expect("seed defaults");
    sqlx::query(
        r#"UPDATE board_statuses s SET is_entry = true
           WHERE s.is_entry = false
             AND NOT EXISTS (SELECT 1 FROM board_statuses e WHERE e.board_id = s.board_id AND e.is_entry)
             AND s.id = (
               SELECT c.id FROM board_statuses c WHERE c.board_id = s.board_id
               ORDER BY (c.category = 'open') DESC, c.sort_order, c.id LIMIT 1)"#,
    )
    .execute(&mut *admin)
    .await
    .expect("entry backfill");
    admin.commit().await.expect("commit seed");

    let mut tx = org_tx(&db.app, s.org).await;
    let cats: Vec<(String, bool)> = sqlx::query_as(
        "SELECT category, is_entry FROM board_statuses WHERE board_id = $1 ORDER BY sort_order",
    )
    .bind(bare_board)
    .fetch_all(&mut *tx)
    .await
    .expect("restored");
    let categories: Vec<&str> = cats.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(
        categories,
        vec!["open", "in_progress", "waiting", "closed"],
        "all four categories restored"
    );
    assert_eq!(
        cats.iter().filter(|(_, e)| *e).count(),
        1,
        "exactly one entry restored"
    );
    assert!(cats[0].1, "entry is the open-category status");

    // Healthy board untouched: still exactly its four seeded statuses.
    let healthy: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM board_statuses WHERE board_id = $1")
            .bind(s.board)
            .fetch_one(&mut *tx)
            .await
            .expect("healthy count");
    assert_eq!(healthy, 4);
}

// --- SLA v1 + automations v1 (TASKLIST 2.7; seed slots 40–49) ---------------

/// Calendar body for tests: 9–5 Mon–Fri in `tz`, optional downtime.
fn test_calendar(
    tz: &str,
    maintenance: Vec<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
) -> BusinessCalendar {
    BusinessCalendar {
        name: "test cal".into(),
        timezone: tz.into(),
        windows: [
            Some(("09:00".into(), "17:00".into())),
            Some(("09:00".into(), "17:00".into())),
            Some(("09:00".into(), "17:00".into())),
            Some(("09:00".into(), "17:00".into())),
            Some(("09:00".into(), "17:00".into())),
            None,
            None,
        ],
        holidays: vec![],
        maintenance: maintenance
            .into_iter()
            .enumerate()
            .map(|(i, (s, e))| core_domain::entities::MaintenanceWindow {
                name: format!("m{i}"),
                starts_at: s,
                ends_at: e,
            })
            .collect(),
    }
}

#[tokio::test]
async fn sla_calendars_crud_attach_and_detach() {
    let db = test_db().await;
    let s = seed_org(&db.app, 40).await;

    let cal = SlaCalendar {
        id: Uuid::now_v7(),
        org_id: s.org,
        name: "NOC hours".into(),
        calendar: test_calendar("America/New_York", vec![]),
    };
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let inserted = sla::insert_calendar(&mut *tx, &cal).await.expect("insert");
        assert_eq!(inserted.calendar.timezone, "America/New_York");
        assert_eq!(inserted.name, "NOC hours");

        let listed = sla::list_calendars(&mut *tx, s.org).await.expect("list");
        assert_eq!(listed.len(), 1);

        // Attach to the seeded default policy; resolution prefers the row.
        let policy = sla::get_default_policy(&mut *tx, s.org)
            .await
            .expect("policy");
        sla::set_policy_calendar(&mut *tx, s.org, policy.id, Some(cal.id))
            .await
            .expect("attach");
        tx.commit().await.expect("commit");
    }

    // Owner-pool resolution path used by the worker sweep.
    let policy_id = {
        let mut t2 = org_tx(&db.app, s.org).await;
        let p = sla::get_default_policy(&mut *t2, s.org)
            .await
            .expect("policy")
            .id;
        t2.commit().await.expect("commit");
        p
    };
    let pool = &db.admin;
    let record = {
        let mut tx = pool.begin().await.expect("tx");
        sla::get_policy_record(&mut *tx, s.org, policy_id)
            .await
            .expect("record")
    };
    assert_eq!(record.calendar_id, Some(cal.id));
    let resolved = sla::resolve_policy_calendar(pool, &record)
        .await
        .expect("resolve");
    assert_eq!(resolved.timezone, "America/New_York");

    // Patch the timezone through the partial update.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let patched = sla::update_calendar(
            &mut *tx,
            s.org,
            cal.id,
            &sla::CalendarPatch {
                name: None,
                timezone: Some("Australia/Sydney"),
                windows: None,
                holidays: None,
                maintenance: None,
            },
        )
        .await
        .expect("patch");
        assert_eq!(patched.calendar.timezone, "Australia/Sydney");

        // Delete detaches instead of destroying the policy.
        sla::delete_calendar(&mut *tx, s.org, cal.id)
            .await
            .expect("delete");
        assert!(matches!(
            sla::get_calendar(&mut *tx, s.org, cal.id).await,
            Err(DbError::NotFound)
        ));
        let policy_id = sla::get_default_policy(&mut *tx, s.org)
            .await
            .expect("policy")
            .id;
        let record = sla::get_policy_record(&mut *tx, s.org, policy_id)
            .await
            .expect("record after detach");
        assert_eq!(record.calendar_id, None, "detach on delete");
        // Embedded document still resolves.
        let embedded = sla::resolve_policy_calendar(&db.admin, &record)
            .await
            .expect("embedded");
        assert_eq!(embedded.timezone, "UTC");
    }
}

/// RLS isolation across every new 0015 user-data table.
#[tokio::test]
async fn rls_isolates_sla_v1_tables() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 41).await;
    let sb = seed_org(&db.app, 42).await;

    let mut tx_a = org_tx(&db.app, sa.org).await;
    sla::insert_calendar(
        &mut *tx_a,
        &SlaCalendar {
            id: Uuid::now_v7(),
            org_id: sa.org,
            name: "A only".into(),
            calendar: test_calendar("UTC", vec![]),
        },
    )
    .await
    .expect("calendar A");
    automations::insert(
        &mut *tx_a,
        Uuid::now_v7(),
        sa.org,
        None,
        "a-rule",
        "ticket.created",
        serde_json::json!([]),
        serde_json::json!([{ "type": "set_priority", "priority": "p3" }]),
    )
    .await
    .expect("rule A");
    queue::enqueue(
        &mut *tx_a,
        &queue::NewJob {
            org_id: sa.org,
            kind: "sla.verdict".into(),
            payload: serde_json::json!({}),
            max_attempts: 5,
        },
    )
    .await
    .expect("job A");
    tx_a.commit().await.expect("commit A");

    // Org B sees none of it.
    let mut tx_b = org_tx(&db.app, sb.org).await;
    assert!(sla::list_calendars(&mut *tx_b, sb.org)
        .await
        .expect("cals")
        .is_empty());
    assert!(
        automations::list(&mut *tx_b, sb.org)
            .await
            .expect("rules")
            .is_empty(),
        "org B must not see org A rules"
    );
    let claimed = queue::claim_batch(&mut *tx_b, 10, "worker-b")
        .await
        .expect("claim");
    assert!(
        claimed.is_empty(),
        "RLS must hide org A jobs from org B claims"
    );
    drop(tx_b);

    // Unscoped connections (no app.org_id ever set) see nothing either.
    let unscoped = db.fresh_app_pool().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
        .fetch_one(&unscoped)
        .await
        .expect("unscoped count");
    assert_eq!(count, 0, "unscoped reads are default-deny");
}

#[tokio::test]
async fn escalation_ladder_replace_and_unique_step_order() {
    let db = test_db().await;
    let s = seed_org(&db.app, 43).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let policy_id = sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("policy")
        .id;

    let steps = vec![
        sla::LadderStepInput {
            step_index: 0,
            trigger_kind: "risk".into(),
            at_pct: 75,
            notify_user_ids: vec![],
            enabled: true,
        },
        sla::LadderStepInput {
            step_index: 1,
            trigger_kind: "breach".into(),
            at_pct: 100,
            notify_user_ids: vec![],
            enabled: true,
        },
    ];
    let inserted = sla::replace_ladder(&mut *tx, s.org, policy_id, &steps)
        .await
        .expect("replace");
    assert_eq!(inserted.len(), 2);
    assert_eq!(inserted[0].trigger_kind, "risk");

    let listed = sla::list_steps_for_policies(&mut *tx, &[policy_id])
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].at_pct, 75, "ordered by threshold");

    // Replacing swaps the whole ladder.
    let replacement = vec![sla::LadderStepInput {
        step_index: 0,
        trigger_kind: "escalate".into(),
        at_pct: 125,
        notify_user_ids: vec![],
        enabled: true,
    }];
    let replaced = sla::replace_ladder(&mut *tx, s.org, policy_id, &replacement)
        .await
        .expect("second replace");
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].trigger_kind, "escalate");

    // Duplicate step_index violates the per-policy uniqueness.
    let dupe = sqlx::query(
        "INSERT INTO sla_escalation_steps \
         (id, org_id, policy_id, step_index, trigger_kind, at_pct, notify_user_ids) \
         VALUES ($1, $2, $3, 0, 'risk', 10, '[]'::jsonb)",
    )
    .bind(Uuid::now_v7())
    .bind(s.org)
    .bind(policy_id)
    .execute(&mut *tx)
    .await;
    assert!(
        matches!(&dupe,
            Err(e) if e.as_database_error().map(|d| d.constraint().is_some()).unwrap_or(false)),
        "unique (policy, index)"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn fired_ledger_and_breach_flag_fire_exactly_once() {
    let db = test_db().await;
    let s = seed_org(&db.app, 44).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut *tx,
        &s,
        Uuid::now_v7(),
        "fire once",
        Priority::P1,
        s.entry_status,
        None,
    )
    .await;
    let policy = sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("policy");
    let state = core_domain::entities::TicketSlaState::init(
        s.org,
        ticket.id,
        &policy,
        Priority::P1,
        ticket.created_at,
    );
    sla::upsert_ticket_state(&mut *tx, &state)
        .await
        .expect("state init");
    let step_id = {
        let ladder = sla::replace_ladder(
            &mut *tx,
            s.org,
            policy.id,
            &[sla::LadderStepInput {
                step_index: 0,
                trigger_kind: "breach".into(),
                at_pct: 100,
                notify_user_ids: vec![],
                enabled: true,
            }],
        )
        .await
        .expect("ladder");
        ladder[0].id
    };
    tx.commit().await.expect("commit setup");

    let first = {
        let mut tx = org_tx(&db.app, s.org).await;
        let recorded = sla::try_record_fired(&mut *tx, s.org, ticket.id, step_id, "resolution")
            .await
            .expect("record 1");
        assert!(recorded, "first fire wins");
        let breached = sla::mark_breached(&mut *tx, ticket.id)
            .await
            .expect("breach update");
        tx.commit().await.expect("commit first fire");
        breached
    };
    assert!(first, "breach flips once");

    let mut tx = org_tx(&db.app, s.org).await;
    let replay = sla::try_record_fired(&mut *tx, s.org, ticket.id, step_id, "resolution")
        .await
        .expect("record 2");
    assert!(!replay, "ledger PK blocks the second fire");
    let re_breach = sla::mark_breached(&mut *tx, ticket.id)
        .await
        .expect("update 2");
    assert!(!re_breach, "already-breached clock does not flip again");

    // Per-target independence: same step may fire for the other clock.
    let fr = sla::try_record_fired(&mut *tx, s.org, ticket.id, step_id, "first_response")
        .await
        .expect("record fr");
    assert!(fr, "targets are independent ledger keys");

    let fired = sla::list_fired_for_tickets(&mut *tx, &[ticket.id])
        .await
        .expect("fired");
    assert_eq!(fired.len(), 2);
}

#[tokio::test]
async fn automation_rules_crud_and_trigger_listing() {
    let db = test_db().await;
    let s = seed_org(&db.app, 45).await;
    let mut tx = org_tx(&db.app, s.org).await;

    let rule_id = Uuid::now_v7();
    let actions = serde_json::json!([
        { "type": "assign_user", "user_id": fixed_uuid(46) },
        { "type": "add_internal_comment", "body": "auto-routed" }
    ]);
    automations::insert(
        &mut *tx,
        rule_id,
        s.org,
        Some(s.board),
        "p1 triage",
        "ticket.created",
        serde_json::json!([{ "type": "priority_is", "priority": "p1" }]),
        actions.clone(),
    )
    .await
    .expect("insert");

    // Trigger listing returns only matching enabled rules of THIS org.
    let hit = automations::list_enabled_by_trigger(&mut *tx, s.org, "ticket.created")
        .await
        .expect("by trigger");
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].id, rule_id);
    assert!(
        automations::list_enabled_by_trigger(&mut *tx, s.org, "timer.started")
            .await
            .expect("other trigger")
            .is_empty()
    );

    // Typed round-trip through the domain parser.
    let parsed = core_domain::automations::AutomationRule::parse(
        &core_domain::automations::AutomationRuleRow {
            id: hit[0].id,
            org_id: hit[0].org_id,
            board_id: hit[0].board_id,
            name: hit[0].name.clone(),
            trigger_event: hit[0].trigger_event.clone(),
            conditions: hit[0].conditions.clone(),
            actions: hit[0].actions.clone(),
            enabled: hit[0].enabled,
        },
    )
    .expect("valid documents parse");
    assert_eq!(parsed.conditions.len(), 1);
    assert_eq!(parsed.actions.len(), 2);
    assert!(parsed.applies_to_board(s.board));
    assert!(
        !parsed.applies_to_board(Uuid::now_v7()),
        "board scope enforced"
    );

    // Disabling removes it from the hot path but keeps the row.
    automations::update(
        &mut *tx,
        s.org,
        rule_id,
        &automations::RulePatch {
            name: None,
            board_id: None,
            trigger_event: None,
            conditions: None,
            actions: None,
            enabled: Some(false),
        },
    )
    .await
    .expect("disable");
    assert!(
        automations::list_enabled_by_trigger(&mut *tx, s.org, "ticket.created")
            .await
            .expect("post-disable")
            .is_empty()
    );
    let fetched = automations::get(&mut *tx, s.org, rule_id)
        .await
        .expect("still there");
    assert!(!fetched.enabled);

    // Clearing the board scope widens to all boards (Some(None) convention).
    automations::update(
        &mut *tx,
        s.org,
        rule_id,
        &automations::RulePatch {
            name: None,
            board_id: Some(None),
            trigger_event: None,
            conditions: None,
            actions: None,
            enabled: Some(true),
        },
    )
    .await
    .expect("clear scope");
    assert_eq!(
        automations::get(&mut *tx, s.org, rule_id)
            .await
            .expect("refetched")
            .board_id,
        None
    );

    automations::delete(&mut *tx, s.org, rule_id)
        .await
        .expect("delete");
    assert!(matches!(
        automations::get(&mut *tx, s.org, rule_id).await,
        Err(DbError::NotFound)
    ));

    // Malformed documents STORE fine (JSONB is schemaless); the parse-skip
    // contract keeps evaluation alive.
    automations::insert(
        &mut *tx,
        Uuid::now_v7(),
        s.org,
        None,
        "broken rule",
        "ticket.created",
        serde_json::json!("not an array"),
        serde_json::json!([{ "type": "detonate" }]),
    )
    .await
    .expect("storage tolerates malformed docs");
    let stored = &automations::list_enabled_by_trigger(&mut *tx, s.org, "ticket.created")
        .await
        .expect("list")[0];
    assert!(
        core_domain::automations::AutomationRule::parse(
            &core_domain::automations::AutomationRuleRow {
                id: stored.id,
                org_id: stored.org_id,
                board_id: stored.board_id,
                name: stored.name.clone(),
                trigger_event: stored.trigger_event.clone(),
                conditions: stored.conditions.clone(),
                actions: stored.actions.clone(),
                enabled: stored.enabled,
            },
        )
        .is_err(),
        "malformed rule parses to Err, never panics"
    );
}

/// Concurrent claimants take disjoint sets — SKIP LOCKED under live lock
/// contention. Worker A holds its (uncommitted) claim while B claims, the
/// exact topology of two workers overlapping in production.
#[tokio::test]
async fn job_queue_concurrent_claims_are_disjoint() {
    let db = test_db().await;
    let s = seed_org(&db.app, 46).await;

    let mut ids = Vec::new();
    {
        let mut tx = org_tx(&db.app, s.org).await;
        for i in 0..6 {
            ids.push(
                queue::enqueue(
                    &mut *tx,
                    &queue::NewJob {
                        org_id: s.org,
                        kind: format!("kind{i}"),
                        payload: serde_json::json!({ "i": i }),
                        max_attempts: 3,
                    },
                )
                .await
                .expect("enqueue"),
            );
        }
        tx.commit().await.expect("commit jobs");
    }

    // Worker A claims four and HOLDS the transaction open.
    let mut hold_tx = org_tx(&db.app, s.org).await;
    let a = queue::claim_batch(&mut *hold_tx, 4, "worker-a")
        .await
        .expect("claim a");
    assert_eq!(a.len(), 4);

    // Worker B races against A's live row locks: it must see only the rest.
    let mut tx_b = org_tx(&db.app, s.org).await;
    let b = queue::claim_batch(&mut *tx_b, 4, "worker-b")
        .await
        .expect("claim b");
    assert_eq!(b.len(), 2, "SKIP LOCKED hands out only uncontended rows");

    let mut claimed: Vec<Uuid> = a.iter().chain(b.iter()).map(|j| j.id).collect();
    let before = claimed.len();
    claimed.sort();
    claimed.dedup();
    assert_eq!(claimed.len(), before, "no job claimed twice");
    assert_eq!(claimed.len(), 6, "everything claimed exactly once");
    assert!(a.iter().chain(b.iter()).all(|j| j.attempts == 1));

    // A third racer gets nothing while both leases are live.
    let mut tx_c = org_tx(&db.app, s.org).await;
    assert!(queue::claim_batch(&mut *tx_c, 4, "worker-c")
        .await
        .expect("c")
        .is_empty());
    tx_b.commit().await.expect("commit b");
    tx_c.commit().await.expect("commit c");
    hold_tx.commit().await.expect("commit a");
}

#[tokio::test]
async fn job_queue_retry_backoff_dead_letter_lifecycle() {
    let db = test_db().await;
    let s = seed_org(&db.app, 47).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let job_id = queue::enqueue(
        &mut *tx,
        &queue::NewJob {
            org_id: s.org,
            kind: "sla.verdict".into(),
            payload: serde_json::json!({ "ticket": "x" }),
            max_attempts: 2,
        },
    )
    .await
    .expect("enqueue");
    tx.commit().await.expect("commit");

    // Attempt 1 fails → retry scheduled one hour out.
    let mut tx = org_tx(&db.app, s.org).await;
    queue::reclaim_stale(&mut *tx, 300).await.expect("reap");
    let batch = queue::claim_batch(&mut *tx, 5, "w").await.expect("claim 1");
    assert_eq!(batch.len(), 1);
    queue::schedule_retry(&mut *tx, job_id, 3600, "api unreachable")
        .await
        .expect("retry");
    tx.commit().await.expect("commit retry");

    // Backoff holds: nothing claimable right now.
    let mut tx = org_tx(&db.app, s.org).await;
    queue::reclaim_stale(&mut *tx, 300).await.expect("reap");
    assert!(queue::claim_batch(&mut *tx, 5, "w")
        .await
        .expect("held")
        .is_empty());

    // Force due and fail again → attempts exhausted → dead-letter.
    sqlx::query("UPDATE jobs SET run_after = now() - interval '1 second' WHERE id = $1")
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .expect("force due");
    queue::reclaim_stale(&mut *tx, 300).await.expect("reap");
    let batch = queue::claim_batch(&mut *tx, 5, "w").await.expect("claim 2");
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].attempts, 2);
    assert_eq!(batch[0].max_attempts, 2);
    queue::mark_dead(&mut *tx, job_id, "api unreachable, giving up")
        .await
        .expect("dead");
    tx.commit().await.expect("commit dead");

    // Dead jobs never come back; stats report them.
    let mut tx = org_tx(&db.app, s.org).await;
    queue::reclaim_stale(&mut *tx, 300).await.expect("reap");
    assert!(queue::claim_batch(&mut *tx, 5, "w")
        .await
        .expect("skip dead")
        .is_empty());
    let stats = queue::stats(&mut *tx).await.expect("stats");
    assert_eq!(stats.dead, 1);
    assert_eq!(stats.pending, 0);
}

/// Crashed claimants lose their lease and the job runs again.
#[tokio::test]
async fn job_queue_reclaims_stale_running_jobs() {
    let db = test_db().await;
    let s = seed_org(&db.app, 48).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let job_id = queue::enqueue(
        &mut *tx,
        &queue::NewJob {
            org_id: s.org,
            kind: "automation.effects".into(),
            payload: serde_json::json!({}),
            max_attempts: 5,
        },
    )
    .await
    .expect("enqueue");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, s.org).await;
    let first = queue::claim_batch(&mut *tx, 5, "crashed-worker")
        .await
        .expect("claim");
    assert_eq!(first.len(), 1);

    // Simulate a crash: the lease went stale without completion.
    sqlx::query("UPDATE jobs SET claimed_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .expect("age the lease");

    assert_eq!(
        queue::reclaim_stale(&mut *tx, 300)
            .await
            .expect("reap count"),
        1,
        "exactly one stale lease re-armed"
    );
    let reclaimed = queue::claim_batch(&mut *tx, 5, "replacement-worker")
        .await
        .expect("reclaim");
    assert_eq!(
        reclaimed.len(),
        1,
        "stale running rows re-arm within the same claim"
    );
    assert_eq!(reclaimed[0].id, job_id);
    assert_eq!(reclaimed[0].attempts, 2, "attempt counter advanced");
    assert_eq!(reclaimed[0].claimed_by, "replacement-worker");

    // Fresh leases are NOT stolen even when the sweep runs.
    let still_running = queue::claim_batch(&mut *tx, 5, "another-worker")
        .await
        .expect("no steal");
    assert!(still_running.is_empty());
}

/// The worker's sweep read: open tickets with resolved calendars; closed
/// tickets drop out; attached vs embedded both resolve.
#[tokio::test]
async fn evaluation_candidates_resolve_open_tickets_and_calendars() {
    let db = test_db().await;
    let s = seed_org(&db.app, 49).await;

    let (open_ticket, closed_ticket) = {
        let mut tx = org_tx(&db.app, s.org).await;
        let policy = sla::get_default_policy(&mut *tx, s.org)
            .await
            .expect("policy");
        let open = create_ticket(
            &mut *tx,
            &s,
            Uuid::now_v7(),
            "open candidate",
            Priority::P2,
            s.entry_status,
            None,
        )
        .await;
        sla::upsert_ticket_state(
            &mut *tx,
            &core_domain::entities::TicketSlaState::init(
                s.org,
                open.id,
                &policy,
                Priority::P2,
                open.created_at,
            ),
        )
        .await
        .expect("open state");
        let closed = create_ticket(
            &mut *tx,
            &s,
            Uuid::now_v7(),
            "closed candidate",
            Priority::P3,
            s.entry_status,
            None,
        )
        .await;
        sla::upsert_ticket_state(
            &mut *tx,
            &core_domain::entities::TicketSlaState::init(
                s.org,
                closed.id,
                &policy,
                Priority::P3,
                closed.created_at,
            ),
        )
        .await
        .expect("closed state");
        tickets::update_status(&mut *tx, s.org, closed.id, s.resolved_status)
            .await
            .expect("close");
        tx.commit().await.expect("commit");
        (open, closed)
    };

    // Owner-role read path (worker doctrine): embedded-calendar default
    // policy resolves; the closed ticket is filtered out.
    let candidates = {
        let conn = &db.admin;
        let mut tx = conn.begin().await.expect("admin tx");
        let all = sla::list_evaluation_candidates(&mut *tx, 200)
            .await
            .expect("candidates");
        tx.commit().await.expect("commit");
        all
    };
    assert!(
        candidates.iter().any(|c| c.ticket_id == open_ticket.id),
        "open ticket is a candidate"
    );
    assert!(
        !candidates.iter().any(|c| c.ticket_id == closed_ticket.id),
        "closed tickets never evaluated"
    );
    let mine = candidates
        .iter()
        .find(|c| c.ticket_id == open_ticket.id)
        .unwrap();
    assert_eq!(mine.calendar.timezone, "UTC", "embedded fallback governs");
    assert!(mine.state.resolution_due.is_some());
    assert!(!mine.state.breached);

    // Attaching a calendar flips resolution to the attached row.
    let mut tx = org_tx(&db.app, s.org).await;
    let cal = sla::insert_calendar(
        &mut *tx,
        &SlaCalendar {
            id: Uuid::now_v7(),
            org_id: s.org,
            name: "attached".into(),
            calendar: test_calendar("Europe/Berlin", vec![]),
        },
    )
    .await
    .expect("cal");
    let policy_id = sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("policy")
        .id;
    sla::set_policy_calendar(&mut *tx, s.org, policy_id, Some(cal.id))
        .await
        .expect("attach");
    tx.commit().await.expect("commit attach");

    let candidates = {
        let mut admin = db.admin.begin().await.expect("admin tx");
        sla::list_evaluation_candidates(&mut *admin, 200)
            .await
            .expect("candidates 2")
    };
    let mine = candidates
        .iter()
        .find(|c| c.ticket_id == open_ticket.id)
        .unwrap();
    assert_eq!(
        mine.calendar.timezone, "Europe/Berlin",
        "attached calendar wins"
    );
}
