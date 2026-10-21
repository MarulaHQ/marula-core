//! End-to-end core-db query-layer tests against a real Postgres 17
//! (pgvector/pgvector:pg18, prod parity).
//!
//! Purpose: prove the SQL layer — org seeding defaults, users/roles,
//! session lifecycle with replay rejection, ticket number allocation,
//! filtered lists, status transitions setting `resolved_at`, comments and
//! checklists, the single-active-timer invariant, time entries, the
//! transactional outbox (`FOR UPDATE SKIP LOCKED`), notification counts,
//! audit append-only persistence, customer sign-offs (Wave 43 track 2 lane
//! 12, migration 0127: insert-only evidence, idempotent replay, a correction
//! as a second row, an append-only revocation trail, the consent snapshot,
//! and the capture-shape CHECK), and RLS isolation (no `app.org_id`
//! scope ⇒ no rows; foreign scope ⇒ no cross-org rows).
//!
//! Non-purpose: domain-rule testing (core-domain unit tests) and HTTP/API
//! behavior (core-api). Database selection follows the documented contract
//! in `tests/common/mod.rs`: `MARULA_TEST_DATABASE_URL`, else
//! `DATABASE_URL` (CI service container), else a local testcontainer.

mod common;

use common::TestDb;
use common::{fixed_uuid, org_tx, test_db, FUTURE, PAST};
use core_db::{
    attachment_scans, audit, automations, notifications as notif, orgs, outbox, portal, queue,
    search, sign_offs, sla, tickets, timers, users, DbError, DbResult,
};
use core_domain::entities::{
    Actor, Attachment, AttachmentKind, AuditEvent, BusinessCalendar, NotificationPreference,
    PortalRole, PortalUser, PortalUserStatus, ScanStatus, SignOffSignerKind, SlaCalendar, Ticket,
    TimeEntry, TimerSession,
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
    // Version 1 of the sign-off consent (Wave 43 track 2 lane 12, 0127).
    orgs::seed_default_sign_off_consent(&mut tx, org)
        .await
        .expect("consent seed");
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
        tag_ids: None,
        limit: 100,
        offset: 0,
        cursor: None,
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

    // Eight system preset roles (RB1, ADR 0004 — migration 0071 roster),
    // keyed for assignment.
    assert_eq!(s.roles.len(), 8);
    for builtin in BuiltinRole::PRESETS {
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
async fn user_create_role_assign_and_direct_roles_for_user() {
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

    let roles = users::direct_roles_for_user(&mut *tx, user, s.org)
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
    let resolved = users::resolve_refresh(&db.app, session, "h1")
        .await
        .expect("bridge resolve")
        .expect("session exists");
    assert_eq!(resolved.user_id, user);
    assert_eq!(resolved.org_id, s.org);
    assert!(resolved.active);

    let mut tx = org_tx(&db.app, s.org).await;
    let parent = users::find_session_any(&mut *tx, session, "h1")
        .await
        .expect("row for rotation");
    let child = fixed_uuid(303);
    users::rotate_session_chain(&mut *tx, &parent, child, "h2", FUTURE)
        .await
        .expect("rotate");

    // Rotation invalidates the old secret: replaying h1 must fail.
    assert!(matches!(
        users::find_active_session(&mut *tx, session, "h1").await,
        Err(DbError::NotFound)
    ));
    assert!(users::find_active_session(&mut *tx, child, "h2")
        .await
        .is_ok());
    // The chain is preserved across the new node.
    let child_row = users::find_session_any(&mut *tx, child, "h2")
        .await
        .expect("child row");
    assert_eq!(child_row.chain_id, parent.chain_id);
    assert_eq!(child_row.rotated_from, Some(parent.id));
    // Commit so the widened bridge (separate session) observes the rotation.
    tx.commit().await.expect("commit rotation");

    // Reuse detection: the rotated token still resolves through the widened
    // bridge, flagged inactive — that exact signal triggers chain revocation.
    let replayed = users::resolve_refresh(&db.app, session, "h1")
        .await
        .expect("bridge resolve on replay")
        .expect("rotated row still visible to bridge");
    assert!(!replayed.active);

    let mut tx = org_tx(&db.app, s.org).await;
    let killed = users::revoke_chain(&mut *tx, replayed.chain_id)
        .await
        .expect("chain revoke");
    assert!(killed >= 1);
    assert!(matches!(
        users::find_active_session(&mut *tx, child, "h2").await,
        Err(DbError::NotFound)
    ));

    // Revocation ends the session even with the current secret.
    users::revoke_session(&mut *tx, child)
        .await
        .expect("revoke");

    // Expired sessions are never active, on either lookup path.
    let expired = fixed_uuid(302);
    users::create_session(&mut *tx, expired, user, "h9", None, PAST)
        .await
        .expect("expired create");
    assert!(matches!(
        users::find_active_session(&mut *tx, expired, "h9").await,
        Err(DbError::NotFound)
    ));
    // Commit so the bridge observes the expired session's committed row.
    tx.commit().await.expect("commit tail");
    let expired_resolved = users::resolve_refresh(&db.app, expired, "h9")
        .await
        .expect("expired rows stay visible for classification");
    assert!(!expired_resolved.expect("row").active);
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

    // Keyset pagination (Wave 15 lane 3): a cursor walk over every page
    // reproduces the full ranked list exactly — including the NULLS LAST
    // due_at contract and the id tiebreaker.
    let full = tickets::list_tickets(&mut *tx, s.org, filter_all())
        .await
        .expect("full list");
    let mut walked: Vec<Ticket> = Vec::new();
    let mut cursor: Option<core_db::tickets::TicketCursor> = None;
    loop {
        let page = tickets::list_tickets(
            &mut *tx,
            s.org,
            tickets::TicketFilter {
                limit: 2,
                cursor,
                ..filter_all()
            },
        )
        .await
        .expect("keyset page");
        let done = page.len() < 2;
        walked.extend(page);
        if done {
            break;
        }
        let last = &walked[walked.len() - 1];
        cursor = Some(core_db::tickets::TicketCursor {
            priority_rank: match last.priority {
                Priority::P1 => 0,
                Priority::P2 => 1,
                Priority::P3 => 2,
                Priority::P4 => 3,
            },
            due_at: last.due_at,
            created_at: last.created_at,
            id: last.id,
        });
    }
    assert_eq!(
        walked.iter().map(|t| t.id).collect::<Vec<_>>(),
        full.iter().map(|t| t.id).collect::<Vec<_>>(),
        "cursor walk must reproduce the ranked list"
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
        tickets::NewTicketComment {
            id: fixed_uuid(802),
            org_id: s.org,
            ticket_id: t.id,
            author_id: Some(u),
            kind: NoteKind::Internal,
            body: "internal diagnosis",
            workflow_meta: None,
        },
    )
    .await
    .expect("comment 1");
    let c2 = tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(803),
            org_id: s.org,
            ticket_id: t.id,
            author_id: None,
            kind: NoteKind::ClientVisible,
            body: "client update",
            workflow_meta: None,
        },
    )
    .await
    .expect("comment 2");
    let c3 = tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(804),
            org_id: s.org,
            ticket_id: t.id,
            author_id: None,
            kind: NoteKind::InHouseUpdate,
            body: "handover",
            workflow_meta: None,
        },
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

    tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(903), true, false, Some(u))
        .await
        .expect("check");
    let listed = tickets::list_checklist(&mut *tx, s.org, t.id)
        .await
        .expect("list 2");
    assert!(listed[0].checked);
    assert_eq!(listed[0].checked_by, Some(u));
    assert!(listed[0].checked_at.is_some());

    // Unchecking clears both the auditor and the timestamp.
    tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(903), false, false, None)
        .await
        .expect("uncheck");
    let listed = tickets::list_checklist(&mut *tx, s.org, t.id)
        .await
        .expect("list 3");
    assert!(!listed[0].checked);
    assert_eq!(listed[0].checked_by, None);
    assert!(listed[0].checked_at.is_none());

    assert!(matches!(
        tickets::set_checklist_item(&mut *tx, s.org, fixed_uuid(999), true, false, None).await,
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

    // 0017 semantics: `is_running` (last segment open) owns the unique slot,
    // so a PAUSED timer no longer blocks a new one — pausing s1 then
    // starting s2 is exactly the auto-pause switch flow and must succeed.
    let mut tx = org_tx(&db.app, s.org).await;
    let s2 = TimerSession::start(s.org, u, t.id, at(11));
    timers::insert_session(&mut *tx, &s2)
        .await
        .expect("paused timer frees the running slot");
    tx.commit().await.expect("commit switch");

    let mut tx = org_tx(&db.app, s.org).await;
    let active = timers::active_session(&mut *tx, u).await.expect("active 2");
    assert_eq!(active.expect("running s2").id, s2.id);

    // Two RUNNING sessions for one user still violate the partial unique
    // index. Run in its own transaction: the expected failure aborts it.
    let s3 = TimerSession::start(s.org, u, t.id, at(13));
    let mut violating = org_tx(&db.app, s.org).await;
    let err = timers::insert_session(&mut *violating, &s3)
        .await
        .expect_err("must violate");
    drop(violating); // roll back the aborted transaction
    assert!(
        is_unique_violation(&err),
        "expected unique violation, got {err:?}"
    );

    let mut tx = org_tx(&db.app, s.org).await;
    let active = timers::active_session(&mut *tx, u).await.expect("active 2");
    assert_eq!(active.expect("running s2").id, s2.id);

    let recent = timers::recent_sessions(&mut *tx, u, 10)
        .await
        .expect("recent");
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, s2.id, "running session docks first");
    assert_eq!(recent[1].id, s1.id);
    assert_eq!(recent[1].stopped_at, None, "paused, not stopped");
    assert!(!recent[1].is_running());

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
        approval_status: None,
        approval_reason_code: None,
        unbillable_reason: None,
        id: fixed_uuid(1103),
        org_id: s.org,
        ticket_id: Some(t1.id),
        phase_id: None,
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
        approval_status: None,
        approval_reason_code: None,
        unbillable_reason: None,
        id: fixed_uuid(1104),
        org_id: s.org,
        ticket_id: Some(t2.id),
        phase_id: None,
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
        None,
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
            wip_limit: None,
            client_visible: None,
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
        None,
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

// --- notes taxonomy + enforced pre-close checklists (the private build tracker 2.8) --------------

use core_db::checklists;
use core_domain::entities::ChecklistProgress;

/// Re-runs migration 0016's defensive taxonomy backfill verbatim and proves
/// it is a no-op that never disturbs existing kinds.
#[tokio::test]
async fn note_kind_backfill_is_idempotent_and_preserves_taxonomy() {
    let db = test_db().await;
    let s = seed_org(&db.app, 50).await;
    let u = seed_user(&db.app, &s, 5000).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(5001),
        "backfill target",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    for (n, kind) in [
        (5002u64, NoteKind::Internal),
        (5003, NoteKind::ClientVisible),
        (5004, NoteKind::InHouseUpdate),
    ] {
        tickets::insert_comment(
            &mut *tx,
            tickets::NewTicketComment {
                id: fixed_uuid(n),
                org_id: s.org,
                ticket_id: t.id,
                author_id: Some(u),
                kind,
                body: "body",
                workflow_meta: None,
            },
        )
        .await
        .expect("comment");
    }
    tx.commit().await.expect("commit");

    // The exact statement from migrations/0016_notes_taxonomy.sql, twice.
    let mut admin = db.admin.begin().await.expect("admin tx");
    for _ in 0..2 {
        sqlx::query("UPDATE ticket_comments SET kind = 'internal' WHERE kind IS NULL")
            .execute(&mut *admin)
            .await
            .expect("backfill");
    }
    admin.commit().await.expect("commit backfill");

    let mut tx = org_tx(&db.app, s.org).await;
    let all = tickets::list_comments(&mut *tx, s.org, t.id, None)
        .await
        .expect("all kinds intact");
    let kinds: std::collections::HashSet<NoteKind> = all.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        std::collections::HashSet::from([
            NoteKind::Internal,
            NoteKind::ClientVisible,
            NoteKind::InHouseUpdate,
        ]),
        "idempotent backfill must not rewrite explicit kinds"
    );
}

/// The portal read path is hard-limited to client-visible notes at the
/// query layer — the whitelist comes from core-domain `notes`, so staff-only
/// kinds can never leak through this function.
#[tokio::test]
async fn client_visible_read_path_returns_only_client_visible() {
    let db = test_db().await;
    let s = seed_org(&db.app, 51).await;
    let u = seed_user(&db.app, &s, 5100).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(5101),
        "portal view",
        Priority::P2,
        s.entry_status,
        None,
    )
    .await;
    let visible = tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(5102),
            org_id: s.org,
            ticket_id: t.id,
            author_id: Some(u),
            kind: NoteKind::ClientVisible,
            body: "client-safe update",
            workflow_meta: None,
        },
    )
    .await
    .expect("visible");
    tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(5103),
            org_id: s.org,
            ticket_id: t.id,
            author_id: None,
            kind: NoteKind::Internal,
            body: "password reset root cause",
            workflow_meta: None,
        },
    )
    .await
    .expect("internal");
    tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(5104),
            org_id: s.org,
            ticket_id: t.id,
            author_id: None,
            kind: NoteKind::InHouseUpdate,
            body: "shift handover",
            workflow_meta: None,
        },
    )
    .await
    .expect("in-house");

    // The pure policy and the DB path must agree exactly.
    let portal_kinds = core_domain::notes::visible_kinds(core_domain::notes::NoteAudience::Portal);
    assert_eq!(portal_kinds, [NoteKind::ClientVisible]);

    let portal_view = tickets::list_client_visible_comments(&mut *tx, s.org, t.id)
        .await
        .expect("portal read");
    assert_eq!(
        portal_view.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![visible.id],
        "portal surface sees ONLY client_visible"
    );

    // Staff filter composes the other direction: exclude client-visible.
    let staff_only = tickets::list_comments(
        &mut *tx,
        s.org,
        t.id,
        Some(&[NoteKind::Internal, NoteKind::InHouseUpdate]),
    )
    .await
    .expect("staff filter");
    assert_eq!(staff_only.len(), 2);
}

/// Template CRUD: one-per-board upsert with replace-all items, RLS isolation
/// against a foreign org, and clean deletion.
#[tokio::test]
async fn checklist_template_upsert_replace_and_rls_isolation() {
    let db = test_db().await;
    let s = seed_org(&db.app, 52).await;
    let foreign = seed_org(&db.app, 53).await;

    let mut tx = org_tx(&db.app, s.org).await;
    // No template by default → enforcement posture is off by construction.
    assert!(checklists::get_template_for_board(&mut *tx, s.org, s.board)
        .await
        .expect("default lookup")
        .is_none());

    let (tpl, items) = checklists::upsert_template(
        &mut tx,
        s.org,
        s.board,
        Uuid::now_v7(),
        "Close-out",
        &["root cause documented".into(), "client notified".into()],
    )
    .await
    .expect("upsert");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].text, "root cause documented");

    // Replace-all: same board keeps ONE template; items are swapped.
    let (_tpl2, items2) = checklists::upsert_template(
        &mut tx,
        s.org,
        s.board,
        Uuid::now_v7(),
        "Close-out v2",
        &["invoice draft ready".into()],
    )
    .await
    .expect("replace");
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM board_checklist_templates WHERE board_id = $1")
            .bind(s.board)
            .fetch_one(&mut *tx)
            .await
            .expect("template count");
    assert_eq!(count, 1, "one template per board");
    assert_eq!(items2.len(), 1);
    assert_ne!(tpl.name, "Close-out v2");

    // Foreign org sees nothing (RLS), even with the same board uuid shape.
    let mut ftx = org_tx(&db.app, foreign.org).await;
    assert!(
        checklists::get_template_for_board(&mut *ftx, foreign.org, s.board)
            .await
            .expect("rls-scoped lookup")
            .is_none()
    );

    checklists::delete_template(&mut *tx, s.org, s.board)
        .await
        .expect("delete");
    assert!(matches!(
        checklists::delete_template(&mut *tx, s.org, s.board).await,
        Err(DbError::NotFound)
    ));
}

/// Instantiation is idempotent, lazily syncs template edits onto tickets,
/// demotes instances when their template item disappears, and progress is
/// derived correctly from the required/ad-hoc split.
#[tokio::test]
async fn template_instantiation_idempotent_lazy_sync_and_progress() {
    let db = test_db().await;
    let s = seed_org(&db.app, 54).await;
    let u = seed_user(&db.app, &s, 5400).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let (_, _items) = checklists::upsert_template(
        &mut tx,
        s.org,
        s.board,
        Uuid::now_v7(),
        "Gate",
        &["step one".into(), "step two".into()],
    )
    .await
    .expect("template");
    let t = create_ticket(
        &mut tx,
        &s,
        fixed_uuid(5401),
        "instantiation probe",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;

    // First instantiation creates both instances; the second is a no-op.
    assert_eq!(
        checklists::instantiate_missing(&mut tx, s.org, s.board, t.id)
            .await
            .expect("first run"),
        2
    );
    assert_eq!(
        checklists::instantiate_missing(&mut tx, s.org, s.board, t.id)
            .await
            .expect("second run"),
        0,
        "idempotent: no duplicates"
    );

    let (listed, p0) = checklists::view(&mut tx, s.org, t.id).await.expect("view");
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|i| i.template_item_id.is_some()));
    assert_eq!(
        p0,
        ChecklistProgress {
            total: 2,
            checked: 0,
            required_open: 2
        }
    );

    // Check one instance off: derived progress follows instantly.
    tickets::set_checklist_item(&mut *tx, s.org, listed[0].id, true, false, Some(u))
        .await
        .expect("check");
    let (_, p1) = checklists::view(&mut tx, s.org, t.id)
        .await
        .expect("view 2");
    assert_eq!(
        p1,
        ChecklistProgress {
            total: 2,
            checked: 1,
            required_open: 1
        }
    );

    // A later template edit flows onto the EXISTING ticket lazily…
    checklists::upsert_template(
        &mut tx,
        s.org,
        s.board,
        Uuid::now_v7(),
        "Gate v2",
        &["step one".into(), "step two".into(), "step three".into()],
    )
    .await
    .expect("edit");
    // …and because item sync is IDENTITY-PRESERVING (matching texts keep
    // their row ids), exactly ONE genuinely-new instance appears on the
    // lazy pass while prior linkage AND checked state survive untouched.
    assert_eq!(
        checklists::instantiate_missing(&mut tx, s.org, s.board, t.id)
            .await
            .expect("lazy sync"),
        1
    );
    let (after_edit, p2) = checklists::view(&mut tx, s.org, t.id)
        .await
        .expect("view 3");
    assert_eq!(after_edit.len(), 3);
    let linked = after_edit
        .iter()
        .filter(|i| i.template_item_id.is_some())
        .count();
    assert_eq!(linked, 3, "edits never break instance provenance");
    assert_eq!(
        p2,
        ChecklistProgress {
            total: 3,
            checked: 1,
            required_open: 2
        },
        "checked state survives template edits"
    );
    tx.commit().await.expect("commit");

    // Deleting the whole template demotes every remaining link (SET NULL).
    let mut tx = org_tx(&db.app, s.org).await;
    checklists::delete_template(&mut *tx, s.org, s.board)
        .await
        .expect("template delete");
    let (_, p3) = checklists::view(&mut tx, s.org, t.id)
        .await
        .expect("view 4");
    assert_eq!(p3.total, 3, "instances survive template removal");
    assert_eq!(p3.required_open, 0, "nothing required once links are gone");
}

// --- SLA v1 + automations v1 (the private build tracker 2.7; seed slots 40–49) ---------------

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
        core_domain::automations::AutomationSubjectKind::Ticket,
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
    let inserted = sla::replace_ladder(&mut tx, s.org, policy_id, &steps)
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
    let replaced = sla::replace_ladder(&mut tx, s.org, policy_id, &replacement)
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
        &mut tx,
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
            &mut tx,
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
        core_domain::automations::AutomationSubjectKind::Ticket,
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
            subject_kind: hit[0].subject_kind,
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
    assert!(parsed.applies_to_board(Some(s.board)));
    assert!(
        !parsed.applies_to_board(Some(Uuid::now_v7())),
        "board scope enforced"
    );
    // (B) A board-scoped rule never matches a subject with no board.
    assert!(
        !parsed.applies_to_board(None),
        "a ticket-less envelope with no board is not on anybody's board"
    );

    // Disabling removes it from the hot path but keeps the row.
    automations::update(
        &mut *tx,
        s.org,
        rule_id,
        &automations::RulePatch {
            name: None,
            board_id: None,
            subject_kind: None,
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
            subject_kind: None,
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
        core_domain::automations::AutomationSubjectKind::Ticket,
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
                subject_kind: stored.subject_kind,
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
            &mut tx,
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
            &mut tx,
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

// --- time discipline: missing-time gap audit (slots 60-69) ---------------------

use core_db::time_audit;

#[tokio::test]
async fn gap_audit_upsert_updates_and_claims_nudge_exactly_once() {
    let db = test_db().await;
    let s = seed_org(&db.app, 60).await;
    let u = seed_user(&db.app, &s, 6000).await;
    let day = chrono::NaiveDate::from_ymd_opt(2026, 1, 15).expect("day");

    let first = {
        let mut tx = org_tx(&db.app, s.org).await;
        let row = time_audit::upsert_gap_audit(
            &mut *tx,
            &time_audit::GapAuditFacts {
                org_id: s.org,
                user_id: u,
                day,
                activity: true,
                logged_minutes: 10,
                expected_minutes: 60,
                has_gap: true,
            },
        )
        .await
        .expect("upsert");
        tx.commit().await.expect("commit");
        row
    };
    assert!(first.has_gap);
    assert!(first.nudge_sent_at.is_none(), "fresh row has not nudged");

    // Exactly one concurrent-style claim wins.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        assert!(time_audit::claim_nudge(&mut *tx, first.id)
            .await
            .expect("claim"));
        assert!(
            !time_audit::claim_nudge(&mut *tx, first.id)
                .await
                .expect("reclaim"),
            "second claim on the same day must lose"
        );
        tx.commit().await.expect("commit");
    }

    // A later re-run refreshes the facts but never resurrects the nudge.
    let second = {
        let mut tx = org_tx(&db.app, s.org).await;
        let row = time_audit::upsert_gap_audit(
            &mut *tx,
            &time_audit::GapAuditFacts {
                org_id: s.org,
                user_id: u,
                day,
                activity: true,
                logged_minutes: 25,
                expected_minutes: 60,
                has_gap: true,
            },
        )
        .await
        .expect("re-upsert");
        tx.commit().await.expect("commit");
        row
    };
    assert_eq!(second.id, first.id, "same (org, user, day) ⇒ same row");
    assert_eq!(second.logged_minutes, 25, "facts refreshed");
    assert!(second.nudge_sent_at.is_some(), "nudge timestamp preserved");

    // The ledger stays one-row-per-day even after many runs.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let rows = time_audit::list_gap_audits(&mut *tx, s.org, u, day, day)
            .await
            .expect("list");
        assert_eq!(rows.len(), 1);
    }
}

#[tokio::test]
async fn org_time_settings_default_and_roundtrip() {
    let db = test_db().await;
    let s = seed_org(&db.app, 61).await;

    let defaults = {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::org_time_settings(&mut *tx, s.org)
            .await
            .expect("settings")
    };
    assert!(defaults.eod_nudges_enabled, "defaults-first: nudges on");
    assert_eq!(defaults.eod_nudge_hour, 18);
    assert_eq!(defaults.time_gap_min_minutes, 60);

    let updated = time_audit::OrgTimeSettings {
        eod_nudges_enabled: false,
        eod_nudge_hour: 17,
        time_gap_min_minutes: 30,
    };
    {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::update_org_time_settings(&mut *tx, s.org, &updated)
            .await
            .expect("update");
        let reread = time_audit::org_time_settings(&mut *tx, s.org)
            .await
            .expect("reread");
        assert_eq!(reread, updated);
    }
    // Unknown org ⇒ NotFound, never a silent default (admin pool: no RLS
    // noise — the app pool's reused sessions carry a defined-but-empty
    // app.org_id that raises 22P02 instead of filtering).
    assert!(matches!(
        time_audit::org_time_settings(&db.admin, fixed_uuid(6199)).await,
        Err(DbError::NotFound)
    ));
}

#[tokio::test]
async fn day_activity_detects_comments_and_overlapping_timer_sessions() {
    let db = test_db().await;
    let s = seed_org(&db.app, 62).await;
    let u = seed_user(&db.app, &s, 6200).await;
    // The comment insert stamps created_at = now(), so the audited window
    // is "the last hour" — the far-past window at the bottom proves quiet
    // days stay quiet.
    let now = chrono::Utc::now();
    let start = now - chrono::Duration::minutes(30);
    let end = now + chrono::Duration::minutes(30);

    let none = {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::day_activity(&mut *tx, s.org, u, start, end)
            .await
            .expect("probe")
    };
    assert!(!none, "quiet day has no activity");

    // A comment inside the window counts.
    let ticket = {
        let mut tx = org_tx(&db.app, s.org).await;
        let t = create_ticket(
            &mut tx,
            &s,
            fixed_uuid(6201),
            "Activity probe",
            Priority::P2,
            s.entry_status,
            None,
        )
        .await;
        tickets::insert_comment(
            &mut *tx,
            tickets::NewTicketComment {
                id: fixed_uuid(6202),
                org_id: s.org,
                ticket_id: t.id,
                author_id: Some(u),
                kind: NoteKind::Internal,
                body: "worked on this",
                workflow_meta: None,
            },
        )
        .await
        .expect("comment");
        tx.commit().await.expect("commit");
        t
    };
    let via_comment = {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::day_activity(&mut *tx, s.org, u, start, end)
            .await
            .expect("probe")
    };
    assert!(via_comment, "comment authored in window ⇒ activity");

    // A session that STARTED the previous evening and is still running
    // overlaps today's window (spans-midnight work is activity).
    let mut overnight =
        TimerSession::start(s.org, u, ticket.id, start - chrono::Duration::hours(2));
    {
        let mut tx = org_tx(&db.app, s.org).await;
        timers::insert_session(&mut *tx, &overnight)
            .await
            .expect("session");
        tx.commit().await.expect("commit");
    }
    let via_timer = {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::day_activity(&mut *tx, s.org, u, start, end)
            .await
            .expect("probe")
    };
    assert!(
        via_timer,
        "running session overlapping the window ⇒ activity"
    );

    // Stopped well before the window and far from it ⇒ no overlap.
    overnight
        .stop(start - chrono::Duration::minutes(25), None)
        .expect("stop");
    {
        let mut tx = org_tx(&db.app, s.org).await;
        timers::update_session(&mut *tx, &overnight)
            .await
            .expect("update session");
        tx.commit().await.expect("commit");
    }
    let far_past = chrono::NaiveDate::from_ymd_opt(2020, 2, 1).expect("day");
    let (s2, e2) = core_domain::time_audit::local_day_window(far_past, chrono_tz::UTC);
    let quiet = {
        let mut tx = org_tx(&db.app, s.org).await;
        time_audit::day_activity(&mut *tx, s.org, u, s2, e2)
            .await
            .expect("probe")
    };
    assert!(!quiet, "no activity on untouched days");
}

#[tokio::test]
async fn rls_hides_time_gap_audits_across_orgs_and_unscoped() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 63).await;
    let ua = seed_user(&db.app, &sa, 6300).await;
    let day = chrono::NaiveDate::from_ymd_opt(2026, 1, 20).expect("day");
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        time_audit::upsert_gap_audit(
            &mut *tx,
            &time_audit::GapAuditFacts {
                org_id: sa.org,
                user_id: ua,
                day,
                activity: true,
                logged_minutes: 5,
                expected_minutes: 60,
                has_gap: true,
            },
        )
        .await
        .expect("upsert");
        tx.commit().await.expect("commit");
    }

    let sb = seed_org(&db.app, 64).await;
    // Foreign scope sees nothing…
    {
        let mut tx = org_tx(&db.app, sb.org).await;
        assert_eq!(
            count_rows(&mut tx, "time_gap_audits", fixed_uuid(6399)).await,
            0,
            "placeholder id never visible"
        );
        let rows = time_audit::list_gap_audits(&mut *tx, sa.org, ua, day, day)
            .await
            .expect("foreign list");
        assert!(
            rows.is_empty(),
            "cross-org ledger hidden by RLS + org param"
        );
    }
    // …unscoped (virgin pool) sees nothing…
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("unscoped begin");
        let rows = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM time_gap_audits")
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        assert_eq!(rows, 0, "default-deny without app.org_id");
    }
    // …and own scope sees exactly its row.
    {
        let mut tx = org_tx(&db.app, sa.org).await;
        let rows = time_audit::list_gap_audits(&mut *tx, sa.org, ua, day, day)
            .await
            .expect("own list");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].has_gap);
    }
}

// --- the private build tracker 2.11: board groups/favorites, per-board RBAC, my-work -------

/// Board groups: CRUD, ordering, board assignment, ungroup-on-delete, and
/// cross-org RLS isolation of both groups and the annotated nav list.
#[tokio::test]
async fn board_groups_crud_assignment_and_rls() {
    let db = test_db().await;
    let s = seed_org(&db.app, 80).await;
    let foreign = seed_org(&db.app, 81).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let owner_id = fixed_uuid(8500);
    users::create_user(
        &mut *tx,
        owner_id,
        s.org,
        "groupowner@acme.test",
        "G",
        "argon",
        "UTC",
    )
    .await
    .expect("group owner user");
    let g1 = core_db::boards::create_group(&mut tx, s.org, Uuid::now_v7(), owner_id, "Infra", 10)
        .await
        .expect("create group");
    let g2 = core_db::boards::create_group(&mut tx, s.org, Uuid::now_v7(), owner_id, "Apps", 0)
        .await
        .expect("create group");

    // Nav order is position-then-name.
    let listed = core_db::boards::list_groups(&mut *tx, s.org)
        .await
        .expect("list");
    assert_eq!(
        listed.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        vec!["Apps", "Infra"]
    );

    // Rename + reposition.
    let renamed = core_db::boards::update_group(
        &mut tx,
        s.org,
        g1.id,
        core_db::boards::GroupPatch {
            name: Some("Infrastructure"),
            position: Some(-5),
        },
    )
    .await
    .expect("update");
    assert_eq!(renamed.name, "Infrastructure");
    assert_eq!(renamed.position, -5);

    // Assign + annotated listing; then ungroup; then delete (boards survive).
    core_db::boards::set_board_group(&mut tx, s.org, s.board, Some(g1.id))
        .await
        .expect("assign");
    let entries = core_db::boards::list_boards_annotated(&mut *tx, s.org, Uuid::now_v7())
        .await
        .expect("annotated");
    let mine = entries
        .iter()
        .find(|e| e.board.id == s.board)
        .expect("board entry");
    assert_eq!(mine.group_id, Some(g1.id));
    assert!(!mine.favorite);

    core_db::boards::set_board_group(&mut tx, s.org, s.board, None)
        .await
        .expect("ungroup");
    core_db::boards::delete_group(&mut tx, s.org, g1.id)
        .await
        .expect("delete");
    core_db::boards::get_board(&mut *tx, s.org, s.board)
        .await
        .expect("board survives group deletion");
    assert!(matches!(
        core_db::boards::delete_group(&mut tx, s.org, g1.id).await,
        Err(DbError::NotFound)
    ));

    // Foreign org cannot see or mutate this org's groups (RLS).
    let mut ftx = org_tx(&db.app, foreign.org).await;
    assert!(core_db::boards::list_groups(&mut *ftx, foreign.org)
        .await
        .expect("foreign list")
        .is_empty());
    assert!(matches!(
        core_db::boards::get_group(&mut *ftx, foreign.org, g2.id).await,
        Err(DbError::NotFound)
    ));
    // Cross-org group assignment is refused by the explicit guard.
    assert!(matches!(
        core_db::boards::set_board_group(&mut ftx, foreign.org, foreign.board, Some(g2.id)).await,
        Err(DbError::NotFound)
    ));
}

/// Favorites stay unique per (user, board) and per-board membership roles
/// round-trip through upsert/remove with RLS isolation.
#[tokio::test]
async fn board_favorites_and_member_roles_round_trip() {
    use core_domain::policy::BoardRole;

    let db = test_db().await;
    let s = seed_org(&db.app, 82).await;
    let foreign = seed_org(&db.app, 83).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let viewer_id = fixed_uuid(8501);
    users::create_user(
        &mut *tx,
        viewer_id,
        s.org,
        "viewer@acme.test",
        "V",
        "argon",
        "UTC",
    )
    .await
    .expect("viewer user");

    // Favorite idempotency: re-add updates sort_order without a dup row.
    core_db::boards::add_favorite(&mut *tx, viewer_id, s.board, 0)
        .await
        .expect("fav");
    core_db::boards::add_favorite(&mut *tx, viewer_id, s.board, 3)
        .await
        .expect("fav again");
    let favs = core_db::boards::favorites_for_user(&mut *tx, viewer_id)
        .await
        .expect("favs");
    assert_eq!(favs.len(), 1);
    assert_eq!(favs[0].sort_order, 3);
    let annotated = core_db::boards::list_boards_annotated(&mut *tx, s.org, viewer_id)
        .await
        .expect("annotated for user");
    assert!(
        annotated
            .iter()
            .all(|e| e.favorite == (e.board.id == s.board)),
        "favorite annotation follows the (user, board) pair"
    );

    // Role storage: upsert inserts then re-roles in place.
    core_db::boards::upsert_member(&mut tx, s.org, s.board, viewer_id, BoardRole::Viewer)
        .await
        .expect("member insert");
    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, viewer_id)
            .await
            .expect("role lookup"),
        Some(BoardRole::Viewer)
    );
    core_db::boards::upsert_member(&mut tx, s.org, s.board, viewer_id, BoardRole::Admin)
        .await
        .expect("re-role");
    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, viewer_id)
            .await
            .expect("role lookup"),
        Some(BoardRole::Admin)
    );
    let roster = core_db::boards::list_members(&mut tx, s.org, s.board)
        .await
        .expect("roster");
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].1, "V");

    // Removal returns the user to org-governed access (no row).
    core_db::boards::remove_member(&mut tx, s.org, s.board, viewer_id)
        .await
        .expect("remove");
    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, viewer_id)
            .await
            .expect("post-remove lookup"),
        None
    );
    assert!(matches!(
        core_db::boards::remove_member(&mut tx, s.org, s.board, viewer_id).await,
        Err(DbError::NotFound)
    ));

    // Foreign org sees none of this roster (RLS), including rows that do
    // exist in the other org.
    let mut ftx = org_tx(&db.app, foreign.org).await;
    assert!(
        core_db::boards::list_members(&mut ftx, foreign.org, foreign.board)
            .await
            .expect("foreign roster empty")
            .is_empty()
    );
    assert_eq!(
        core_db::boards::role_for_user(&mut *ftx, foreign.org, foreign.board, viewer_id)
            .await
            .expect("foreign role lookup"),
        None,
        "RLS hides other-org memberships"
    );
}

/// The migration's defaults-first backfill (org Owner/Admin holders become
/// board admins on existing boards) — statement-equivalent replay against a
/// post-migration org, pinning exactly who becomes a member and who not.
#[tokio::test]
async fn board_admin_backfill_defaults_first() {
    use core_domain::policy::BoardRole;

    let db = test_db().await;
    let s = seed_org(&db.app, 85).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let owner_id = fixed_uuid(8502);
    users::create_user(
        &mut *tx,
        owner_id,
        s.org,
        "owner@acme.test",
        "O",
        "argon",
        "UTC",
    )
    .await
    .expect("owner user");
    users::assign_role(&mut *tx, owner_id, s.roles["owner"])
        .await
        .expect("owner role");
    let admin_id = fixed_uuid(8503);
    users::create_user(
        &mut *tx,
        admin_id,
        s.org,
        "admin@acme.test",
        "A",
        "argon",
        "UTC",
    )
    .await
    .expect("admin user");
    users::assign_role(&mut *tx, admin_id, s.roles["admin"])
        .await
        .expect("admin role");
    let tech_id = fixed_uuid(8504);
    users::create_user(
        &mut *tx,
        tech_id,
        s.org,
        "tech@acme.test",
        "T",
        "argon",
        "UTC",
    )
    .await
    .expect("tech user");
    users::assign_role(&mut *tx, tech_id, s.roles["technician"])
        .await
        .expect("tech role");

    // Exact backfill statement from 0019_board_organize_rbac.sql: CI's
    // migration run executes it against pre-existing boards; here we prove
    // its selection semantics on freshly seeded equivalents.
    sqlx::query(
        r#"INSERT INTO board_members (org_id, board_id, user_id, role)
           SELECT b.org_id, b.id, ur.user_id, 'admin'::board_role
           FROM boards b
           JOIN roles r ON r.org_id = b.org_id AND r.builtin IN ('owner', 'admin')
           JOIN user_roles ur ON ur.role_id = r.id
           ON CONFLICT (user_id, board_id) DO NOTHING"#,
    )
    .execute(&mut *tx)
    .await
    .expect("backfill replay");

    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, owner_id)
            .await
            .expect("owner lookup"),
        Some(BoardRole::Admin),
        "org owners become board admins"
    );
    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, admin_id)
            .await
            .expect("admin lookup"),
        Some(BoardRole::Admin),
        "org admins become board admins"
    );
    assert_eq!(
        core_db::boards::role_for_user(&mut *tx, s.org, s.board, tech_id)
            .await
            .expect("tech lookup"),
        None,
        "technicians are NOT auto-membered (defaults-first passthrough)"
    );
}

/// The cross-board my-work queue: assigned-to-me open tickets only,
/// priority rank first, then SLA resolution_due falling back to due_at,
/// with client/board/status annotations populated.
#[tokio::test]
async fn my_work_annotated_orders_priority_then_sla_due() {
    use core_domain::entities::TicketSlaState;

    let db = test_db().await;
    let s = seed_org(&db.app, 89).await;

    let mut tx = org_tx(&db.app, s.org).await;
    let me = fixed_uuid(8510);
    users::create_user(&mut *tx, me, s.org, "me@acme.test", "Me", "argon", "UTC")
        .await
        .expect("me user");
    let other = fixed_uuid(8511);
    users::create_user(
        &mut *tx,
        other,
        s.org,
        "other@acme.test",
        "Other",
        "argon",
        "UTC",
    )
    .await
    .expect("other user");

    // One client so the squared-tag annotation has data.
    let client_id = fixed_uuid(8512);
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, 'Acme Corp')")
        .bind(client_id)
        .bind(s.org)
        .execute(&mut *tx)
        .await
        .expect("client insert");

    struct Seed {
        id: Uuid,
        title: &'static str,
        priority: Priority,
        assignee: Option<Uuid>,
        client_id: Option<Uuid>,
        due_at: Option<chrono::DateTime<chrono::Utc>>,
    }
    async fn mk_ticket(conn: &mut Transaction<'_, Postgres>, s: &Seeded, seed: Seed) -> Ticket {
        let (prefix, seq) = orgs::next_ticket_number(&mut **conn, s.org)
            .await
            .expect("number alloc");
        tickets::insert_ticket(
            conn,
            tickets::NewTicket {
                id: seed.id,
                org_id: s.org,
                number: &format!("{prefix}-{seq}"),
                board_id: s.board,
                status_id: s.entry_status,
                title: seed.title,
                body: None,
                priority: seed.priority,
                client_id: seed.client_id,
                site_id: None,
                contact_id: None,
                assignee_id: seed.assignee,
                requester_id: None,
                due_at: seed.due_at,
            },
        )
        .await
        .expect("ticket insert")
    }
    let me_some = Some(me);
    // p2, no SLA state, manual due far in the future.
    let t_p2_manual = mk_ticket(
        &mut tx,
        &s,
        Seed {
            id: fixed_uuid(8520),
            title: "p2 manual due",
            priority: Priority::P2,
            assignee: me_some,
            client_id: Some(client_id),
            due_at: Some(FUTURE),
        },
    )
    .await;
    // p1 with a LATE SLA deadline — priority must still win the top slot.
    let t_p1_late = mk_ticket(
        &mut tx,
        &s,
        Seed {
            id: fixed_uuid(8521),
            title: "p1 late sla",
            priority: Priority::P1,
            assignee: me_some,
            client_id: None,
            due_at: None,
        },
    )
    .await;
    // p2 with an EARLY SLA deadline — beats the manual-due p2 below it.
    let t_p2_early_sla = mk_ticket(
        &mut tx,
        &s,
        Seed {
            id: fixed_uuid(8522),
            title: "p2 early sla",
            priority: Priority::P2,
            assignee: me_some,
            client_id: None,
            due_at: None,
        },
    )
    .await;
    // Assigned to someone else entirely — never in my queue.
    mk_ticket(
        &mut tx,
        &s,
        Seed {
            id: fixed_uuid(8523),
            title: "not mine",
            priority: Priority::P1,
            assignee: Some(other),
            client_id: None,
            due_at: None,
        },
    )
    .await;
    // Mine but closed — excluded by the open-only contract.
    let closed = mk_ticket(
        &mut tx,
        &s,
        Seed {
            id: fixed_uuid(8524),
            title: "mine closed",
            priority: Priority::P1,
            assignee: me_some,
            client_id: None,
            due_at: None,
        },
    )
    .await;
    tickets::update_status(&mut *tx, s.org, closed.id, s.resolved_status)
        .await
        .expect("close");

    // SLA state rows come from the org's seeded default policy; only the
    // deadline is overridden so the ordering fixture stays deterministic.
    let policy = sla::get_default_policy(&mut *tx, s.org)
        .await
        .expect("seeded sla policy");
    let late_due = FUTURE - chrono::Duration::days(1);
    let early_due = PAST + chrono::Duration::days(1);
    for (ticket_id, due) in [(t_p1_late.id, late_due), (t_p2_early_sla.id, early_due)] {
        let state = TicketSlaState::init(s.org, ticket_id, &policy, Priority::P2, PAST);
        sla::upsert_ticket_state(&mut *tx, &state)
            .await
            .expect("sla state upsert");
        sqlx::query(
            "UPDATE ticket_sla_state SET resolution_due = $3 WHERE ticket_id = $1 AND org_id = $2",
        )
        .bind(ticket_id)
        .bind(s.org)
        .bind(due)
        .execute(&mut *tx)
        .await
        .expect("sla due override");
    }
    tx.commit().await.expect("commit seeds");

    let mut app_tx = org_tx(&db.app, s.org).await;
    let rows = core_db::tickets::my_work_annotated(&mut *app_tx, s.org, me, 100)
        .await
        .expect("my-work query");
    let ids: Vec<Uuid> = rows.iter().map(|r| r.ticket.id).collect();
    assert_eq!(
        ids,
        vec![t_p1_late.id, t_p2_early_sla.id, t_p2_manual.id],
        "priority rank first; then COALESCE(sla_resolution_due, due_at)"
    );

    let first = &rows[0];
    assert_eq!(first.sla_resolution_due, Some(late_due));
    assert_eq!(first.client_name, None);
    assert_eq!(first.status_name, "New");
    let annotated = rows
        .iter()
        .find(|r| r.ticket.id == t_p2_manual.id)
        .expect("manual row");
    assert_eq!(annotated.client_name.as_deref(), Some("Acme Corp"));
    assert_eq!(annotated.sla_resolution_due, None);
    assert!(!rows.iter().any(|r| r.ticket.title == "not mine"));
    assert!(!rows
        .iter()
        .any(|r| r.ticket.category == TicketStatusCategory::Closed));
}

// --- client portal v1 (the private build tracker 2.13) ----------------------------------------
//
// Slots 100–109 are this lane's exclusive seed range. The matrix below pins:
// RLS on all three new tables, per-CLIENT scoping on top of org RLS (the
// cross-client isolation contract), exposure gating + status narrowing,
// the portal session family, and portal-tier audit provenance.

/// Seeds one client in an already-seeded org.
async fn seed_client(app: &PgPool, s: &Seeded, n: u64, name: &str) -> Uuid {
    let id = fixed_uuid(n);
    let mut tx = org_tx(app, s.org).await;
    core_db::clients::create_client(&mut *tx, s.org, id, name, None)
        .await
        .expect("client insert");
    tx.commit().await.expect("commit client");
    id
}

async fn create_portal_user(
    conn: &mut PgConnection,
    s: &Seeded,
    client_id: Uuid,
    n: u64,
    email: &str,
    role: PortalRole,
) -> DbResult<PortalUser> {
    portal::create_portal_user(
        conn,
        portal::NewPortalUser {
            id: fixed_uuid(n),
            org_id: s.org,
            client_id,
            email,
            display_name: "Portal Person",
            password_hash: "argon2hash",
            role,
        },
    )
    .await
}

#[tokio::test]
async fn portal_users_lifecycle_rls_and_client_scoping() {
    let db = test_db().await;
    let s = seed_org(&db.app, 100).await;
    let client_a = seed_client(&db.app, &s, 1010, "Client A").await;
    let client_b = seed_client(&db.app, &s, 1011, "Client B").await;

    let ua = {
        let mut tx = org_tx(&db.app, s.org).await;
        let u = create_portal_user(
            &mut tx,
            &s,
            client_a,
            1012,
            "a@client.test",
            PortalRole::Contact,
        )
        .await
        .expect("insert");
        tx.commit().await.expect("commit");
        u
    };
    assert_eq!(ua.status, PortalUserStatus::Invited);
    assert_eq!(ua.role, PortalRole::Contact);
    assert_eq!(ua.client_id, client_a);

    // Same email under the SAME org → unique violation. Runs in its OWN
    // transaction: a violated constraint aborts its transaction in
    // Postgres, and we still need this one afterwards.
    {
        let mut dup_tx = org_tx(&db.app, s.org).await;
        assert!(create_portal_user(
            &mut dup_tx,
            &s,
            client_b,
            1013,
            "a@client.test",
            PortalRole::ItManager
        )
        .await
        .is_err());
        dup_tx.rollback().await.expect("rollback dup probe");
    }
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let it = create_portal_user(
            &mut tx,
            &s,
            client_b,
            1014,
            "it@client.test",
            PortalRole::ItManager,
        )
        .await
        .expect("second identity");

        // Client-scoped directory listing.
        let for_a = portal::list_portal_users(&mut *tx, s.org, Some(client_a))
            .await
            .expect("list A");
        assert_eq!(for_a.len(), 1);
        assert_eq!(for_a[0].id, ua.id);
        let for_b = portal::list_portal_users(&mut *tx, s.org, Some(client_b))
            .await
            .expect("list B");
        assert_eq!(for_b.len(), 1);
        assert_eq!(for_b[0].role, PortalRole::ItManager);

        // Patch: re-tier + activate + password reset.
        let patched = portal::update_portal_user(
            &mut *tx,
            s.org,
            ua.id,
            portal::PortalUserPatch {
                display_name: None,
                role: Some(PortalRole::Approver),
                status: Some(PortalUserStatus::Active),
                password_hash: Some("new-hash".into()),
            },
        )
        .await
        .expect("patch");
        assert_eq!(patched.role, PortalRole::Approver);
        assert_eq!(patched.status, PortalUserStatus::Active);

        // Credentials lookup is case-insensitive and org-scoped.
        let creds = portal::find_portal_credentials_by_email(&mut *tx, s.org, "IT@CLIENT.TEST")
            .await
            .expect("creds")
            .expect("found");
        assert_eq!(creds.id, it.id);
        assert!(
            portal::find_portal_credentials_by_email(&mut *tx, s.org, "nobody@x.test")
                .await
                .expect("creds")
                .is_none()
        );

        // First-login activation only flips invited rows.
        portal::activate_on_first_login(&mut *tx, s.org, it.id)
            .await
            .expect("activate");
        portal::activate_on_first_login(&mut *tx, s.org, it.id)
            .await
            .expect("re-activate noop");
        let active = portal::get_portal_user(&mut *tx, s.org, it.id)
            .await
            .expect("get");
        assert_eq!(active.status, PortalUserStatus::Active);
        tx.commit().await.expect("commit lifecycle tx");
    }

    // Foreign org: zero visibility (RLS), even with the exact uuid.
    let sb = seed_org(&db.app, 105).await;
    {
        let mut foreign = org_tx(&db.app, sb.org).await;
        assert!(matches!(
            portal::get_portal_user(&mut *foreign, s.org, ua.id).await,
            Err(DbError::NotFound)
        ));
        assert_eq!(
            count_rows(&mut foreign, "portal_users", ua.id).await,
            0,
            "cross-org portal users hidden"
        );
        assert_eq!(
            count_rows(&mut foreign, "board_portal_exposure", fixed_uuid(1020)).await,
            0
        );
        assert_eq!(
            count_rows(&mut foreign, "portal_sessions", fixed_uuid(1030)).await,
            0
        );
    }

    // Unscoped (virgin pool): default-deny across ALL new tables.
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("begin unscoped");
        assert_eq!(count_rows(&mut tx, "portal_users", ua.id).await, 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM board_portal_exposure")
                .fetch_one(&mut *tx)
                .await
                .expect("count"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM portal_sessions")
                .fetch_one(&mut *tx)
                .await
                .expect("count"),
            0
        );
    }
}

#[tokio::test]
async fn portal_ticket_reads_never_cross_clients_and_honor_exposure() {
    let db = test_db().await;
    let s = seed_org(&db.app, 106).await;
    let client_a = seed_client(&db.app, &s, 1040, "Cross-check Co").await;
    let client_b = seed_client(&db.app, &s, 1041, "Other Co").await;

    // Two tickets on the SAME board, one per client.
    let ticket_a = {
        let mut tx = org_tx(&db.app, s.org).await;
        let t = create_ticket(
            &mut tx,
            &s,
            fixed_uuid(1042),
            "Client A issue",
            Priority::P2,
            s.entry_status,
            None,
        )
        .await;
        // Attach client A via direct update (helper leaves client NULL).
        sqlx::query("UPDATE tickets SET client_id = $1 WHERE id = $2 AND org_id = $3")
            .bind(client_a)
            .bind(fixed_uuid(1042))
            .bind(s.org)
            .execute(&mut *tx)
            .await
            .expect("attach client");
        tx.commit().await.expect("commit A");
        t
    };
    let ticket_b = {
        let mut tx = org_tx(&db.app, s.org).await;
        let t = create_ticket(
            &mut tx,
            &s,
            fixed_uuid(1043),
            "Client B secret",
            Priority::P1,
            s.entry_status,
            None,
        )
        .await;
        sqlx::query("UPDATE tickets SET client_id = $1 WHERE id = $2 AND org_id = $3")
            .bind(client_b)
            .bind(fixed_uuid(1043))
            .bind(s.org)
            .execute(&mut *tx)
            .await
            .expect("attach client");
        tx.commit().await.expect("commit B");
        t
    };

    // Exposure exists ONLY for client A.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        portal::upsert_exposure(
            &mut *tx,
            portal::NewExposure {
                org_id: s.org,
                board_id: s.board,
                client_id: client_a,
                exposed: true,
                visible_status_ids: vec![],
            },
        )
        .await
        .expect("expose board to A");
        tx.commit().await.expect("commit exposure");
    }

    // Client A sees its own ticket; client B's ticket is NotFound — same
    // org, same board, different client. This is THE isolation matrix row.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let listed = portal::list_client_tickets(
            &mut *tx,
            s.org,
            client_a,
            portal::PortalTicketFilter {
                board_id: None,
                category: None,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .expect("list for A");
        assert_eq!(listed.len(), 1, "only own ticket");
        assert_eq!(listed[0].id, ticket_a.id);

        assert!(matches!(
            portal::get_client_ticket(&mut *tx, s.org, client_a, ticket_b.id).await,
            Err(DbError::NotFound)
        ));
        assert!(
            matches!(
                portal::get_client_ticket(&mut *tx, s.org, client_b, ticket_b.id).await,
                Err(DbError::NotFound),
            ),
            "unexposed board hides client B's own ticket too"
        );
    }

    // Status narrowing: expose to B restricted to the ENTRY status only —
    // a waiting-status ticket disappears from the list.
    {
        let waiting = {
            let mut tx = org_tx(&db.app, s.org).await;
            let t = create_ticket(
                &mut tx,
                &s,
                fixed_uuid(1044),
                "Client B waiting",
                Priority::P3,
                s.waiting_status,
                None,
            )
            .await;
            sqlx::query("UPDATE tickets SET client_id = $1 WHERE id = $2 AND org_id = $3")
                .bind(client_b)
                .bind(fixed_uuid(1044))
                .bind(s.org)
                .execute(&mut *tx)
                .await
                .expect("attach client");
            tx.commit().await.expect("commit");
            t
        };
        let mut tx = org_tx(&db.app, s.org).await;
        portal::upsert_exposure(
            &mut *tx,
            portal::NewExposure {
                org_id: s.org,
                board_id: s.board,
                client_id: client_b,
                exposed: true,
                visible_status_ids: vec![s.entry_status],
            },
        )
        .await
        .expect("narrow exposure");
        tx.commit().await.expect("commit narrow");

        let mut tx = org_tx(&db.app, s.org).await;
        let listed = portal::list_client_tickets(
            &mut *tx,
            s.org,
            client_b,
            portal::PortalTicketFilter {
                board_id: None,
                category: None,
                limit: 50,
                offset: 0,
            },
        )
        .await
        .expect("list for B");
        assert_eq!(listed.len(), 1, "narrowed status hides the waiting ticket");
        assert_eq!(listed[0].id, ticket_b.id);
        assert!(matches!(
            portal::get_client_ticket(&mut *tx, s.org, client_b, waiting.id).await,
            Err(DbError::NotFound)
        ));
    }

    // Category filter path stays consistent with the join.
    let mut tx = org_tx(&db.app, s.org).await;
    let open_only = portal::list_client_tickets(
        &mut *tx,
        s.org,
        client_a,
        portal::PortalTicketFilter {
            board_id: None,
            category: Some(TicketStatusCategory::Open),
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("category list");
    assert_eq!(open_only.len(), 1);
}

#[tokio::test]
async fn portal_sessions_are_a_disjoint_token_family() {
    let db = test_db().await;
    let s = seed_org(&db.app, 107).await;
    let client = seed_client(&db.app, &s, 1050, "Session Co").await;
    let pu = {
        let mut tx = org_tx(&db.app, s.org).await;
        let u = create_portal_user(
            &mut tx,
            &s,
            client,
            1051,
            "sess@client.test",
            PortalRole::Contact,
        )
        .await
        .expect("insert");
        portal::update_portal_user(
            &mut *tx,
            s.org,
            u.id,
            portal::PortalUserPatch {
                display_name: None,
                role: None,
                status: Some(PortalUserStatus::Active),
                password_hash: None,
            },
        )
        .await
        .expect("activate");
        tx.commit().await.expect("commit");
        u
    };

    let session_id = fixed_uuid(1052);
    let refresh_hash = "deadbeef-hash";
    {
        let mut tx = org_tx(&db.app, s.org).await;
        portal::create_session(
            &mut *tx,
            session_id,
            s.org,
            pu.id,
            refresh_hash,
            Some("test-agent"),
            FUTURE,
        )
        .await
        .expect("session insert");
        tx.commit().await.expect("commit session");
    }

    // Live session resolves through the SECURITY DEFINER bridge.
    let resolved = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_session($1, $2)",
    )
    .bind(session_id)
    .bind(refresh_hash)
    .fetch_optional(&db.admin)
    .await
    .expect("resolve");
    assert_eq!(resolved, Some((pu.id, s.org, client)));

    // Wrong token hash resolves to nothing.
    let wrong = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_session($1, $2)",
    )
    .bind(session_id)
    .bind("not-the-hash")
    .fetch_optional(&db.admin)
    .await
    .expect("resolve");
    assert_eq!(wrong, None);

    // Rotation revokes; replay then fails closed.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        portal::revoke_session(&mut *tx, session_id)
            .await
            .expect("revoke");
        tx.commit().await.expect("commit revoke");
    }
    let replayed = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_session($1, $2)",
    )
    .bind(session_id)
    .bind(refresh_hash)
    .fetch_optional(&db.admin)
    .await
    .expect("resolve after revoke");
    assert_eq!(replayed, None);

    // Login resolver skips disabled identities entirely.
    let login = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_login($1)",
    )
    .bind("sess@client.test")
    .fetch_optional(&db.admin)
    .await
    .expect("login resolve");
    assert_eq!(login, Some((pu.id, s.org, client)));
    {
        let mut tx = org_tx(&db.app, s.org).await;
        portal::update_portal_user(
            &mut *tx,
            s.org,
            pu.id,
            portal::PortalUserPatch {
                display_name: None,
                role: None,
                status: Some(PortalUserStatus::Disabled),
                password_hash: None,
            },
        )
        .await
        .expect("disable");
        tx.commit().await.expect("commit disable");
    }
    let disabled = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT portal_user_id, org_id, client_id FROM portal_resolve_login($1)",
    )
    .bind("sess@client.test")
    .fetch_optional(&db.admin)
    .await
    .expect("login resolve disabled");
    assert_eq!(disabled, None, "disabled identities never resolve");
}

#[tokio::test]
async fn portal_audit_provenance_roundtrips_per_tier() {
    let db = test_db().await;
    let s = seed_org(&db.app, 108).await;
    let actors = [
        (Actor::PortalContact(fixed_uuid(1060)), "portal_contact"),
        (Actor::PortalApprover(fixed_uuid(1061)), "portal_approver"),
        (
            Actor::PortalItManager(fixed_uuid(1062)),
            "portal_it_manager",
        ),
    ];
    {
        let mut tx = org_tx(&db.app, s.org).await;
        for (actor, _) in &actors {
            audit::insert(
                &mut *tx,
                &AuditEvent {
                    id: Uuid::now_v7(),
                    org_id: s.org,
                    actor: actor.clone(),
                    action: "portal.ticket_reply".into(),
                    entity_type: Some("ticket_comment".into()),
                    entity_id: Some(Uuid::now_v7().to_string()),
                    detail: serde_json::json!({}),
                    ip: None,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await
            .expect("audit insert");
        }
        tx.commit().await.expect("commit audits");
    }
    let mut tx = org_tx(&db.app, s.org).await;
    let events = audit::list(&mut *tx, s.org, 100, None).await.expect("list");
    for (actor, expected_kind) in &actors {
        let stored = events
            .iter()
            .find(|e| e.actor == *actor)
            .unwrap_or_else(|| {
                panic!("actor {expected_kind} round-tripped");
            });
        // The stored actor must deserialize back to the SAME tier variant —
        // provenance survives the storage layer exactly.
        let kind_in_db: String = sqlx::query_scalar(
            "SELECT actor_kind FROM audit_events WHERE org_id = $1 AND actor_id = $2",
        )
        .bind(s.org)
        .bind(match actor {
            Actor::PortalContact(u) | Actor::PortalApprover(u) | Actor::PortalItManager(u) => {
                u.to_string()
            }
            _ => unreachable!(),
        })
        .fetch_one(&mut *tx)
        .await
        .expect("kind");
        assert_eq!(kind_in_db.as_str(), *expected_kind);
        assert!(stored.action.starts_with("portal."));
    }

    // The widened CHECK still rejects unknown kinds.
    let rejected = sqlx::query(
        "INSERT INTO audit_events (id, org_id, actor_kind, action) VALUES ($1, $2, 'wizard', 'x')",
    )
    .bind(Uuid::now_v7())
    .bind(s.org)
    .execute(&mut *tx)
    .await;
    assert!(
        rejected.is_err(),
        "unknown actor_kind must violate the CHECK"
    );
}

// --- Plugin runtime v1 (the private build tracker 2.15; seed slots 90–99) --------------------

use core_db::webhooks as wh;

fn sealed_secret(key: [u8; 32], secret: &str) -> Vec<u8> {
    // Mirrors core-api crypto::seal(): nonce(12) || AES-256-GCM ct.
    use aes_gcm::aead::{Aead, KeyInit, OsRng};
    use aes_gcm::{AeadCore, Aes256Gcm};
    let cipher = Aes256Gcm::new_from_slice(&key).expect("key");
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(cipher.encrypt(&nonce, secret.as_bytes()).expect("seal"));
    out
}

const SUITE_AT_REST_KEY: [u8; 32] = [11; 32];

#[tokio::test]
async fn webhook_crud_and_rls_isolation() {
    let db = test_db().await;
    let app = db.app.clone();
    let sa = seed_org(&app, 90).await;
    let sb = seed_org(&app, 91).await;
    let user_a = seed_user(&app, &sa, 9000).await;

    let secret = "whsec_db_suite";
    let sealed = sealed_secret(SUITE_AT_REST_KEY, secret);
    let id = fixed_uuid(901);

    // Insert + read back inside org A.
    let row = {
        let mut tx = org_tx(&app, sa.org).await;
        let row = wh::insert_webhook(
            &mut *tx,
            id,
            sa.org,
            "https://hooks.acme.test/marula",
            &sealed,
            &["ticket.created".into(), "ticket.closed".into()],
            user_a,
            None,
        )
        .await
        .expect("insert webhook");
        tx.commit().await.expect("commit");
        row
    };
    assert!(row.enabled);
    assert_eq!(row.event_types.len(), 2);
    // The stored ciphertext must decrypt back to the original secret
    // through the exact at-rest format core-api's seal() produces.
    {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Nonce};
        let cipher = Aes256Gcm::new_from_slice(&SUITE_AT_REST_KEY).expect("key");
        let (nonce, ct) = row.secret_encrypted.split_at(12);
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .expect("stored ciphertext must be authentic");
        assert_eq!(String::from_utf8(plain).as_deref(), Ok(secret));
    }

    // Cross-org visibility is denied by RLS.
    {
        let mut tx = org_tx(&app, sb.org).await;
        assert!(matches!(
            wh::get_webhook(&mut *tx, sb.org, id).await,
            Err(DbError::NotFound)
        ));
        assert!(wh::list_webhooks(&mut *tx, sb.org)
            .await
            .expect("list b")
            .is_empty());
    }

    // Wildcard matching: empty filter matches anything; exact filter only
    // its own types; disabled subscriptions match nothing.
    {
        let mut tx = org_tx(&app, sa.org).await;
        wh::update_webhook(&mut *tx, sa.org, id, None, Some(&[]), None)
            .await
            .expect("wildcard");
        tx.commit().await.expect("commit");
    }
    // Worker posture: cross-org matching runs on the OWNER pool with no
    // org scope set (see crates/worker config note).
    let matched = wh::find_enabled_for_event(&db.admin, sa.org, "timer.started")
        .await
        .expect("wildcard match");
    assert_eq!(matched.len(), 1, "empty filter is a wildcard");
    {
        let mut tx = org_tx(&app, sa.org).await;
        wh::update_webhook(
            &mut *tx,
            sa.org,
            id,
            None,
            Some(&["sla.escalation_fired".to_string()]),
            None,
        )
        .await
        .expect("narrow");
        tx.commit().await.expect("commit");
    }
    let matched = wh::find_enabled_for_event(&db.admin, sa.org, "timer.started")
        .await
        .expect("no match");
    assert!(matched.is_empty(), "filtered subscription must not match");

    // Disabled ⇒ never matched even on its own event type.
    {
        let mut tx = org_tx(&app, sa.org).await;
        wh::update_webhook(&mut *tx, sa.org, id, None, None, Some(false))
            .await
            .expect("disable");
        tx.commit().await.expect("commit");
    }
    let matched = wh::find_enabled_for_event(&db.admin, sa.org, "sla.escalation_fired")
        .await
        .expect("disabled");
    assert!(matched.is_empty());

    // Delete removes the row for its org only.
    {
        let mut tx = org_tx(&app, sa.org).await;
        wh::delete_webhook(&mut *tx, sa.org, id)
            .await
            .expect("delete");
        tx.commit().await.expect("commit");
    }
}

/// Wave 43 lane 4 (A): a consented connector's declared endpoints seed
/// real, install-attributed subscriptions — and the install's lifecycle
/// governs them from there.
#[tokio::test]
async fn seeded_connector_endpoints_follow_the_install_lifecycle() {
    use core_db::plugins;

    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 96).await;
    let admin = seed_user(&app, &s, 9060).await;

    let entry = core_domain::entities::PluginCatalogEntry {
        id: fixed_uuid(961),
        slug: "rmm-connector".into(),
        name: "RMM".into(),
        description: "d".into(),
        publisher: "p".into(),
        // Owned by no org in the fixture: a seeded/operator entry.
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: core_domain::entities::PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let manifest = core_domain::entities::PluginManifestV2 {
        name: "RMM".into(),
        version: "1.0.0".into(),
        interface_version: core_domain::entities::PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "d".into(),
        capabilities: vec!["automation.triggers".into()],
        settings_schema: None,
        artifact_kind: Some(core_domain::entities::plugins::PluginRuntimeKind::HttpConnector),
        connector_profile: Some(core_domain::entities::ConnectorProfile {
            webhook_endpoints: vec![
                "https://hooks.rmm.test/one".into(),
                "https://hooks.rmm.test/two".into(),
            ],
            scopes: vec![],
            routing: None,
        }),
        surface_profile: None,
    };
    let install = core_domain::entities::PluginInstall {
        id: fixed_uuid(962),
        org_id: s.org,
        catalog_id: entry.id,
        slug: "rmm-connector".into(),
        version: "1.0.0".into(),
        // DISABLED is the created state — the seeded rows must be inert.
        status: core_domain::entities::PluginInstallStatus::Disabled,
        granted_capabilities: vec!["automation.triggers".to_string()],
        installed_by: Some(admin),
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        revoked_at: None,
        revoked_by: None,
    };
    {
        let mut tx = org_tx(&app, s.org).await;
        plugins::create_catalog_entry(
            &mut tx,
            &entry,
            &core_domain::entities::PluginVersion {
                id: fixed_uuid(963),
                catalog_id: entry.id,
                version: "1.0.0".into(),
                manifest,
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("entry");
        plugins::create_install(&mut tx, &install)
            .await
            .expect("install");
        tx.commit().await.expect("commit");
    }

    let sealed = sealed_secret(SUITE_AT_REST_KEY, "whsec_seeded");
    let endpoints = [
        "https://hooks.rmm.test/one".to_string(),
        "https://hooks.rmm.test/two".to_string(),
    ];

    // 1) Seeding creates one attributed row per declared endpoint.
    let seeded = {
        let mut tx = org_tx(&app, s.org).await;
        let n = wh::seed_install_webhooks(&mut *tx, s.org, install.id, &endpoints, &sealed, admin)
            .await
            .expect("seed");
        tx.commit().await.expect("commit");
        n
    };
    assert_eq!(seeded, 2, "one row per declared endpoint");

    // A re-seed is idempotent, and a residue drop fires only for an
    // endpoint the manifest no longer declares.
    {
        let mut tx = org_tx(&app, s.org).await;
        let again =
            wh::seed_install_webhooks(&mut *tx, s.org, install.id, &endpoints, &sealed, admin)
                .await
                .expect("re-seed");
        tx.commit().await.expect("commit");
        assert_eq!(again, 0, "ON CONFLICT DO NOTHING creates nothing");
    }
    {
        let mut tx = org_tx(&app, s.org).await;
        let after = wh::seed_install_webhooks(
            &mut *tx,
            s.org,
            install.id,
            &["https://hooks.rmm.test/one".to_string()],
            &sealed,
            admin,
        )
        .await
        .expect("narrow");
        tx.commit().await.expect("commit");
        assert_eq!(after, 0);
    }

    let owned = {
        let mut tx = org_tx(&app, s.org).await;
        let rows = wh::list_webhooks(&mut *tx, s.org).await.expect("list");
        tx.commit().await.expect("commit");
        rows
    };
    assert_eq!(owned.len(), 1, "the undeclared endpoint is the residue");
    assert_eq!(owned[0].owner_install_id, Some(install.id));
    assert_eq!(owned[0].created_by, Some(admin));

    // 2) While the install is DISABLED the seeded row matches nothing —
    //    the liveness gate, which is what keeps `enabled = true` from
    //    live-delivering before anyone enabled the connector.
    let matched = wh::find_enabled_for_event(&db.admin, s.org, "ticket.created")
        .await
        .expect("match");
    assert!(
        matched.is_empty(),
        "a disabled install's seeded rows must not deliver"
    );

    // 3) Enabling the install makes them live.
    {
        let mut tx = org_tx(&app, s.org).await;
        plugins::update_install(
            &mut tx,
            s.org,
            install.id,
            Some(core_domain::entities::PluginInstallStatus::Enabled),
            None,
            None,
        )
        .await
        .expect("enable");
        tx.commit().await.expect("commit");
    }
    let matched = wh::find_enabled_for_event(&db.admin, s.org, "ticket.created")
        .await
        .expect("match");
    assert_eq!(matched.len(), 1, "an enabled install's rows deliver");
    assert_eq!(matched[0].owner_install_id, Some(install.id));

    // 4) Revoking tears them down outright, and the liveness predicate
    //    independently makes any straggler inert.
    {
        let mut tx = org_tx(&app, s.org).await;
        plugins::revoke_catalog_entry(&mut tx, entry.id, "supply chain", chrono::Utc::now())
            .await
            .expect("revoke");
        tx.commit().await.expect("commit");
    }
    let after = wh::list_webhooks(&db.admin, s.org).await.expect("list");
    assert!(
        after.is_empty(),
        "revoke removes the install's subscriptions"
    );
    let matched = wh::find_enabled_for_event(&db.admin, s.org, "ticket.created")
        .await
        .expect("match");
    assert!(matched.is_empty(), "and nothing delivers after revoke");
}

/// A human-created subscription carries no owner, so the liveness gate's
/// LEFT JOIN must leave it untouched: it delivers whether or not any
/// plugin install exists.
#[tokio::test]
async fn unowned_subscriptions_are_unaffected_by_the_liveness_gate() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 97).await;
    let user = seed_user(&app, &s, 9070).await;
    {
        let mut tx = org_tx(&app, s.org).await;
        wh::insert_webhook(
            &mut *tx,
            fixed_uuid(971),
            s.org,
            "https://hooks.acme.test/human",
            &sealed_secret(SUITE_AT_REST_KEY, "whsec_human"),
            &[],
            user,
            None,
        )
        .await
        .expect("insert");
        tx.commit().await.expect("commit");
    }
    let matched = wh::find_enabled_for_event(&db.admin, s.org, "ticket.created")
        .await
        .expect("match");
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].owner_install_id, None);
    assert_eq!(matched[0].url, "https://hooks.acme.test/human");
}

#[tokio::test]
async fn delivery_ledger_records_every_attempt_exactly_once_and_pages() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 92).await;
    let user = seed_user(&app, &s, 9020).await;
    let hook = fixed_uuid(921);
    {
        let mut tx = org_tx(&app, s.org).await;
        wh::insert_webhook(
            &mut *tx,
            hook,
            s.org,
            "https://hooks.acme.test/sink",
            &sealed_secret(SUITE_AT_REST_KEY, "whsec_x"),
            &[],
            user,
            None,
        )
        .await
        .expect("insert");
        tx.commit().await.expect("commit");
    }

    let event_id = fixed_uuid(922);
    // Attempts 1..=3 land; a duplicate report of attempt 2 is swallowed.
    for attempt in 1..=3_i32 {
        let delivered = (attempt == 3).then(chrono::Utc::now);
        let retry = (attempt < 3)
            .then(|| chrono::Utc::now() + chrono::Duration::seconds(30 * attempt as i64));
        let mut tx = app.begin().await.expect("tx");
        core_db::set_org(&mut tx, s.org).await.expect("scope");
        let logged = wh::insert_delivery_attempt(
            &mut tx,
            s.org,
            &wh::NewDeliveryAttempt {
                webhook_id: hook,
                event_id,
                event_type: "ticket.created".into(),
                attempt,
                response_status: Some(if attempt == 3 { 200 } else { 500 }),
                next_retry_at: retry,
                delivered_at: delivered,
                last_error: (attempt < 3).then(|| "receiver returned 500".to_string()),
            },
        )
        .await
        .expect("log attempt");
        assert!(logged, "first report of attempt {attempt} must insert");
        tx.commit().await.expect("commit");
    }
    // Exactly-once under worker at-least-once replays:
    let mut tx = app.begin().await.expect("tx");
    core_db::set_org(&mut tx, s.org).await.expect("scope");
    let replayed = wh::insert_delivery_attempt(
        &mut tx,
        s.org,
        &wh::NewDeliveryAttempt {
            webhook_id: hook,
            event_id,
            event_type: "ticket.created".into(),
            attempt: 2,
            response_status: Some(500),
            next_retry_at: None,
            delivered_at: None,
            last_error: Some("duplicate report".into()),
        },
    )
    .await
    .expect("replay log");
    assert!(!replayed, "duplicate attempt report must be ignored");

    // Cursor pagination walks newest-first without repeats or gaps.
    let page1 = wh::list_deliveries(&mut tx, s.org, hook, None, 2)
        .await
        .expect("page 1");
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].attempt, 3, "newest first");
    let cursor = (page1.last().expect("row").created_at, page1[1].id);
    let page2 = wh::list_deliveries(&mut tx, s.org, hook, Some(cursor), 2)
        .await
        .expect("page 2");
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].attempt, 1);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn api_keys_hash_lifecycle_scopes_and_revocation() {
    let db = test_db().await;
    let app = db.app.clone();
    let s = seed_org(&app, 93).await;
    let creator = seed_user(&app, &s, 9030).await;

    let raw = "mrl_0123456789abcdef0123456789abcdef";
    let hash = format!("{:064x}", 0xbeef_u128); // hex(HMAC(...)) shape
    let id = fixed_uuid(961);

    let key = {
        let mut tx = org_tx(&app, s.org).await;
        let key = wh::insert_api_key(
            &mut *tx,
            id,
            s.org,
            "NinjaOne sync",
            &hash,
            &raw[..12],
            &["tickets:read".to_string(), "clients:read".to_string()],
            creator,
        )
        .await
        .expect("create key");
        tx.commit().await.expect("commit");
        key
    };
    assert_eq!(key.scopes.len(), 2);
    assert_eq!(key.prefix, "mrl_01234567");
    assert!(key.revoked_at.is_none());

    // Active lookup resolves cross-org by hash (auth path).
    let found = wh::find_active_api_key_by_hash(&app, &hash)
        .await
        .expect("lookup")
        .expect("active key resolves");
    assert_eq!(found.id, id);
    assert_eq!(found.org_id, s.org);

    // Throttled last-used touch writes once per window.
    wh::touch_api_key_last_used(&app, id, 60)
        .await
        .expect("touch");
    let first_stamp = wh::find_active_api_key_by_hash(&app, &hash)
        .await
        .expect("lookup")
        .expect("key")
        .last_used_at;
    assert!(first_stamp.is_some());
    wh::touch_api_key_last_used(&app, id, 3600)
        .await
        .expect("throttled touch no-op");
    let unchanged = wh::find_active_api_key_by_hash(&app, &hash)
        .await
        .expect("lookup")
        .expect("key")
        .last_used_at;
    assert_eq!(first_stamp, unchanged, "inside throttle window: no write");

    // Revocation hides the key from auth lookups immediately.
    {
        let mut tx = org_tx(&app, s.org).await;
        wh::revoke_api_key(&mut *tx, s.org, id)
            .await
            .expect("revoke");
        tx.commit().await.expect("commit");
    }
    assert!(
        wh::find_active_api_key_by_hash(&app, &hash)
            .await
            .expect("lookup")
            .is_none(),
        "revoked keys never authenticate"
    );
    // Idempotent second revoke still reports NotFound via rows_affected==0
    // guard? No — revoked_at IS NOT NULL filter means zero rows: NotFound.
    let mut tx = org_tx(&app, s.org).await;
    assert!(matches!(
        wh::revoke_api_key(&mut *tx, s.org, id).await,
        Err(DbError::NotFound)
    ));
}

#[tokio::test]
async fn connector_installations_upsert_consent_and_isolation() {
    let db = test_db().await;
    let app = db.app.clone();
    let sa = seed_org(&app, 94).await;
    let sb = seed_org(&app, 95).await;
    let admin_a = seed_user(&app, &sa, 9040).await;

    let config = sealed_secret(SUITE_AT_REST_KEY, r#"{"api_key":"nj-secret"}"#);
    let scopes = vec!["tickets:read".to_string()];
    let first = {
        let mut tx = org_tx(&app, sa.org).await;
        let row = wh::upsert_connector_installation(
            &mut *tx,
            Uuid::now_v7(),
            sa.org,
            "ninjaone",
            &config,
            &scopes,
            admin_a,
        )
        .await
        .expect("install");
        tx.commit().await.expect("commit");
        row
    };

    // Re-consent replaces config + scopes in place (same row).
    let new_config = sealed_secret(SUITE_AT_REST_KEY, r#"{"api_key":"nj-rotated"}"#);
    let updated = {
        let mut tx = org_tx(&app, sa.org).await;
        let row = wh::upsert_connector_installation(
            &mut *tx,
            Uuid::now_v7(),
            sa.org,
            "ninjaone",
            &new_config,
            &["tickets:read".to_string(), "clients:read".to_string()],
            admin_a,
        )
        .await
        .expect("re-consent");
        tx.commit().await.expect("commit");
        row
    };
    assert_eq!(updated.id, first.id, "one installation per connector type");
    assert_eq!(updated.capability_scopes.len(), 2);

    let list_a = {
        let mut tx = org_tx(&app, sa.org).await;
        let rows = wh::list_connector_installations(&mut *tx, sa.org)
            .await
            .expect("list a");
        tx.commit().await.expect("commit");
        rows
    };
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].connector_type, "ninjaone");

    // Org B sees nothing of org A's installations.
    let list_b = {
        let mut tx = org_tx(&app, sb.org).await;
        let rows = wh::list_connector_installations(&mut *tx, sb.org)
            .await
            .expect("list b");
        tx.commit().await.expect("commit");
        rows
    };
    assert!(list_b.is_empty());
    {
        let mut tx = org_tx(&app, sb.org).await;
        assert!(matches!(
            wh::delete_connector_installation(&mut *tx, sb.org, "ninjaone").await,
            Err(DbError::NotFound)
        ));
    }
}

// --- Scheduling/calendars/availability (the private build tracker 2.12; seed slots 110–119) ---

use chrono::TimeZone;
use core_db::scheduling;
use core_domain::entities::{
    Appointment, AppointmentKind, AppointmentStatus, AvailabilityWindow, CalendarExclusion,
    OnCallBlock,
};

fn appt(org: Uuid, user: Uuid, id: u64, from_h: u32, to_h: u32) -> Appointment {
    let day = |h: u32| {
        chrono::Utc.with_ymd_and_hms(2026, 6, 1, h, 0, 0).unwrap() // Monday 2026-06-01
    };
    Appointment {
        id: fixed_uuid(id),
        org_id: org,
        user_id: user,
        ticket_id: None,
        client_id: None,
        site_id: None,
        title: format!("appt-{id}"),
        starts_at: day(from_h),
        ends_at: day(to_h),
        kind: AppointmentKind::OnSite,
        status: AppointmentStatus::Scheduled,
        travel_minutes: 15,
        notes: None,
        scheduling_link_id: None,
        travel_from: None,
        travel_distance_meters: None,
        travel_provider: None,
        travel_quality: None,
        created_at: at(1),
        updated_at: at(1),
    }
}

#[tokio::test]
async fn appointment_exclude_blocks_overlap_and_cancel_frees_slot() {
    let db = test_db().await;
    let s = seed_org(&db.app, 110).await;
    let u = seed_user(&db.app, &s, 11200).await;
    let other = seed_user(&db.app, &s, 11201).await;

    // Base set: Mon 09–11 plus legal neighbours (touching span,
    // different assignee). One transaction, no violations.
    let mut tx = org_tx(&db.app, s.org).await;
    scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1102, 9, 11))
        .await
        .expect("base insert");
    scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1105, 11, 13))
        .await
        .expect("back-to-back ok");
    scheduling::create_appointment(&mut *tx, &appt(s.org, other, 1106, 9, 11))
        .await
        .expect("different assignee ok");
    tx.commit().await.expect("commit base set");

    // Exact overlap rejected by the EXCLUDE constraint (own transaction:
    // a Postgres error aborts its transaction, so violations go last).
    let mut tx = org_tx(&db.app, s.org).await;
    match scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1103, 10, 12)).await {
        Err(core_db::DbError::Sql(sqlx::Error::Database(d))) => assert_eq!(
            d.constraint(),
            Some("appointments_no_double_booking"),
            "expected the double-booking exclusion"
        ),
        other => panic!("expected double-booking violation, got {other:?}"),
    }
    drop(tx);

    // …and a nested one is rejected too.
    let mut tx = org_tx(&db.app, s.org).await;
    assert!(is_constraint_violation(
        &scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1104, 10, 11))
            .await
            .unwrap_err()
    ));
    drop(tx);

    // `done` bookings still occupy history: booking over one is blocked.
    let mut tx = org_tx(&db.app, s.org).await;
    let mut done = scheduling::get_appointment(&mut *tx, s.org, fixed_uuid(1102))
        .await
        .expect("fetch");
    done.status = AppointmentStatus::Done;
    scheduling::update_appointment(&mut *tx, &done)
        .await
        .expect("mark done");
    tx.commit().await.expect("commit done flip");

    let mut tx = org_tx(&db.app, s.org).await;
    assert!(is_constraint_violation(
        &scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1107, 10, 12))
            .await
            .unwrap_err(),
    ));
    drop(tx);

    // Cancelling frees the slot immediately — the rebook lands in the
    // SAME transaction, proving the predicate reads live state.
    let mut tx = org_tx(&db.app, s.org).await;
    let mut cancelled = scheduling::get_appointment(&mut *tx, s.org, fixed_uuid(1105))
        .await
        .expect("fetch");
    cancelled.status = AppointmentStatus::Cancelled;
    scheduling::update_appointment(&mut *tx, &cancelled)
        .await
        .expect("cancel");
    scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1108, 11, 12))
        .await
        .expect("cancelled slot is rebookable");
    tx.commit().await.expect("commit cancel+rebook");

    // A move onto an occupied slot fails through the same constraint.
    let mut tx = org_tx(&db.app, s.org).await;
    let mut mover = scheduling::get_appointment(&mut *tx, s.org, fixed_uuid(1108))
        .await
        .expect("fetch");
    mover.starts_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0).unwrap();
    mover.ends_at = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap();
    assert!(is_constraint_violation(
        &scheduling::update_appointment(&mut *tx, &mover)
            .await
            .unwrap_err()
    ));
    drop(tx);

    // Range listing returns live rows only (cancelled hidden), ordered.
    let mut tx = org_tx(&db.app, s.org).await;
    let from = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let to = chrono::Utc.with_ymd_and_hms(2026, 6, 2, 0, 0, 0).unwrap();
    let mine = scheduling::list_appointments_between(&mut *tx, s.org, &[u], from, to)
        .await
        .expect("list mine");
    assert_eq!(
        mine.iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![fixed_uuid(1102), fixed_uuid(1108)],
        "done visible, cancelled hidden"
    );
    tx.commit().await.expect("commit list");
}

#[tokio::test]
async fn availability_oncall_exclusion_rules_and_merged_calendar() {
    let db = test_db().await;
    let s = seed_org(&db.app, 111).await;
    let u = seed_user(&db.app, &s, 11210).await;

    let window = AvailabilityWindow {
        id: fixed_uuid(1111),
        org_id: s.org,
        user_id: u,
        weekday: 0, // Monday
        start_local: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        end_local: chrono::NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        timezone: "UTC".into(),
        effective_from: None,
        effective_until: None,
        created_at: at(1),
    };
    let mut tx = org_tx(&db.app, s.org).await;
    scheduling::create_availability_window(&mut *tx, &window)
        .await
        .expect("window insert");
    tx.commit().await.expect("commit window");

    // Exact duplicate template rejected (expression unique index).
    let dupe = AvailabilityWindow {
        id: Uuid::now_v7(),
        ..window.clone()
    };
    let mut tx = org_tx(&db.app, s.org).await;
    assert!(
        is_constraint_violation(
            &scheduling::create_availability_window(&mut *tx, &dupe)
                .await
                .unwrap_err()
        ),
        "duplicate availability template"
    );
    drop(tx);

    // On-call stacking for one person is rejected…
    let block = OnCallBlock {
        id: fixed_uuid(1112),
        org_id: s.org,
        user_id: u,
        starts_at: chrono::Utc.with_ymd_and_hms(2026, 6, 1, 18, 0, 0).unwrap(),
        ends_at: chrono::Utc.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap(),
        category: "primary".into(),
        notes: Some("weeknights".into()),
        created_at: at(1),
    };
    let mut tx = org_tx(&db.app, s.org).await;
    scheduling::create_on_call_block(&mut *tx, &block)
        .await
        .expect("first rotation ok");
    tx.commit().await.expect("commit rotation");

    let stacked = OnCallBlock {
        id: Uuid::now_v7(),
        starts_at: chrono::Utc.with_ymd_and_hms(2026, 6, 2, 6, 0, 0).unwrap(),
        ..block.clone()
    };
    let mut tx = org_tx(&db.app, s.org).await;
    match scheduling::create_on_call_block(&mut *tx, &stacked).await {
        Err(core_db::DbError::Sql(sqlx::Error::Database(d))) => {
            assert_eq!(d.constraint(), Some("on_call_no_overlap"))
        }
        other => panic!("expected on_call_no_overlap violation, got {other:?}"),
    }
    drop(tx);

    // Time-off exclusion + an appointment share one clean transaction,
    // then the merged-view readers and scoped deletes run.
    let exclusion = CalendarExclusion {
        id: fixed_uuid(1113),
        org_id: s.org,
        user_id: u,
        starts_at: chrono::Utc.with_ymd_and_hms(2026, 6, 3, 0, 0, 0).unwrap(),
        ends_at: chrono::Utc.with_ymd_and_hms(2026, 6, 5, 0, 0, 0).unwrap(),
        reason: Some("vacation".into()),
        created_at: at(1),
    };
    let mut tx = org_tx(&db.app, s.org).await;
    scheduling::create_calendar_exclusion(&mut *tx, &exclusion)
        .await
        .expect("exclusion insert");
    scheduling::create_appointment(&mut *tx, &appt(s.org, u, 1114, 9, 11))
        .await
        .expect("appt for merged view");
    tx.commit().await.expect("commit fixtures");

    let mut tx = org_tx(&db.app, s.org).await;
    let from = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let to = chrono::Utc.with_ymd_and_hms(2026, 6, 8, 0, 0, 0).unwrap();
    let windows = scheduling::list_availability_windows(&mut *tx, s.org, u)
        .await
        .expect("windows list");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].weekday, 0);
    let on_call = scheduling::list_on_call_blocks_between(&mut *tx, s.org, &[u], from, to)
        .await
        .expect("on-call list");
    assert_eq!(on_call.len(), 1);
    assert_eq!(on_call[0].category, "primary");
    let exclusions = scheduling::list_calendar_exclusions_between(&mut *tx, s.org, u, from, to)
        .await
        .expect("exclusions list");
    assert_eq!(exclusions.len(), 1);
    assert_eq!(exclusions[0].reason.as_deref(), Some("vacation"));

    // Deletes round-trip with ownership scoping (org+user must match).
    scheduling::delete_calendar_exclusion(&mut tx, s.org, fixed_uuid(1113), u)
        .await
        .expect("delete own exclusion");
    assert!(matches!(
        scheduling::delete_on_call_block(&mut tx, s.org, fixed_uuid(1112), fixed_uuid(11999)).await,
        Err(core_db::DbError::NotFound),
    ));
    tx.commit().await.expect("commit deletes");
}

#[tokio::test]
async fn rls_isolates_scheduling_tables() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 112).await;
    let sb = seed_org(&db.app, 113).await;
    let ua = seed_user(&db.app, &sa, 11220).await;

    let mut tx_a = org_tx(&db.app, sa.org).await;
    scheduling::create_appointment(&mut *tx_a, &appt(sa.org, ua, 1121, 9, 10))
        .await
        .expect("appt A");
    scheduling::create_availability_window(
        &mut *tx_a,
        &AvailabilityWindow {
            id: fixed_uuid(1122),
            org_id: sa.org,
            user_id: ua,
            weekday: 1,
            start_local: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end_local: chrono::NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            timezone: "UTC".into(),
            effective_from: None,
            effective_until: None,
            created_at: at(1),
        },
    )
    .await
    .expect("window A");
    scheduling::create_on_call_block(
        &mut *tx_a,
        &OnCallBlock {
            id: fixed_uuid(1123),
            org_id: sa.org,
            user_id: ua,
            starts_at: chrono::Utc.with_ymd_and_hms(2026, 6, 1, 20, 0, 0).unwrap(),
            ends_at: chrono::Utc.with_ymd_and_hms(2026, 6, 2, 6, 0, 0).unwrap(),
            category: "secondary".into(),
            notes: None,
            created_at: at(1),
        },
    )
    .await
    .expect("on-call A");
    scheduling::create_calendar_exclusion(
        &mut *tx_a,
        &CalendarExclusion {
            id: fixed_uuid(1124),
            org_id: sa.org,
            user_id: ua,
            starts_at: chrono::Utc.with_ymd_and_hms(2026, 6, 4, 0, 0, 0).unwrap(),
            ends_at: chrono::Utc.with_ymd_and_hms(2026, 6, 5, 0, 0, 0).unwrap(),
            reason: None,
            created_at: at(1),
        },
    )
    .await
    .expect("exclusion A");
    tx_a.commit().await.expect("commit A");

    // Org B sees none of it through every scoped reader.
    let mut tx_b = org_tx(&db.app, sb.org).await;
    let from = chrono::Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
    let to = chrono::Utc.with_ymd_and_hms(2026, 6, 8, 0, 0, 0).unwrap();
    assert!(
        scheduling::list_appointments_between(&mut *tx_b, sb.org, &[ua], from, to)
            .await
            .expect("appts B")
            .is_empty()
    );
    assert!(
        scheduling::list_availability_windows(&mut *tx_b, sb.org, ua)
            .await
            .expect("windows B")
            .is_empty()
    );
    assert!(
        scheduling::list_on_call_blocks_between(&mut *tx_b, sb.org, &[ua], from, to)
            .await
            .expect("on-call B")
            .is_empty()
    );
    assert!(
        scheduling::list_calendar_exclusions_between(&mut *tx_b, sb.org, ua, from, to)
            .await
            .expect("exclusions B")
            .is_empty()
    );
    drop(tx_b);

    // Unscoped connections default-deny across all four tables.
    let unscoped = db.fresh_app_pool().await;
    for table in [
        "appointments",
        "availability_windows",
        "on_call_blocks",
        "calendar_exclusions",
    ] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE id IS NOT NULL"
        ))
        .fetch_one(&unscoped)
        .await
        .unwrap_or_else(|e| panic!("unscoped count {table}: {e}"));
        assert_eq!(count, 0, "unscoped reads on {table} are default-deny");
    }
}

// --- unified search v1 (the private build tracker 2.16) --------------------------------------

/// Thin wrapper so search tests read like the domain operations they model.
async fn idx_upsert(
    conn: &mut PgConnection,
    org: Uuid,
    kind: search::SearchKind,
    entity_id: Uuid,
    parent_id: Option<Uuid>,
    title: &str,
    body: &str,
) {
    search::upsert_row(
        &mut *conn, org, kind, entity_id, parent_id, title, body, None,
    )
    .await
    .expect("search upsert");
}

#[tokio::test]
async fn search_upsert_is_exactly_once_and_updates_in_place() {
    let db = test_db().await;
    let s = seed_org(&db.app, 120).await;
    let entity = fixed_uuid(126);
    let mut tx = org_tx(&db.app, s.org).await;

    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        None,
        "Mail relay down",
        "outbound mail queuing",
    )
    .await;
    // Replay of the same (org, kind, entity) — e.g. a redelivered event.
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        None,
        "Mail relay down",
        "updated: outbound mail flushed",
    )
    .await;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_index WHERE org_id = $1")
        .bind(s.org)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(count, 1, "replayed upserts must not duplicate rows");

    let hits = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "relay",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("page");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Mail relay down");
    assert!(
        hits[0].body_excerpt.contains("flushed"),
        "row content reflects the latest upsert"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_delete_is_idempotent() {
    let db = test_db().await;
    let s = seed_org(&db.app, 121).await;
    let client = fixed_uuid(127);
    let mut tx = org_tx(&db.app, s.org).await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Client,
        client,
        None,
        "Northwind Traders",
        "",
    )
    .await;
    assert!(
        search::delete_row(&mut *tx, s.org, search::SearchKind::Client, client)
            .await
            .expect("delete"),
        "first delete removes the row"
    );
    assert!(
        !search::delete_row(&mut *tx, s.org, search::SearchKind::Client, client)
            .await
            .expect("delete again"),
        "second delete reports nothing removed"
    );
    tx.commit().await.expect("commit");
}

/// Seeds the shared websearch corpus: kinds exercise phrases, exclusions,
/// and the trigram prefix leg. Org `n` gets its own isolated copy.
async fn seed_websearch_corpus(app: &PgPool, n: u64) -> Seeded {
    let s = seed_org(app, n).await;
    let mut tx = org_tx(app, s.org).await;
    for (i, (title, body)) in [
        ("Printer offline after firmware update", "office printer"),
        ("VPN drops intermittently", "network tunnel unstable"),
        ("Firmware upgrade plan", "router network maintenance"),
        ("Printer queue jammed", "tray 2 sensor stuck"),
    ]
    .into_iter()
    .enumerate()
    {
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(120 + i as u64),
            None,
            title,
            body,
        )
        .await;
    }
    tx.commit().await.expect("commit corpus");
    s
}

#[tokio::test]
async fn search_websearch_phrases_exclusions_and_prefix_tokens() {
    let db = test_db().await;
    let s = seed_websearch_corpus(&db.app, 122).await;
    let mut tx = org_tx(&db.app, s.org).await;

    // Phrase: only adjacency counts ("firmware update", not "firmware … plan").
    let phrase = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "\"firmware update\"",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("phrase query");
    assert_eq!(phrase.len(), 1, "phrase matches adjacent tokens only");
    assert!(phrase[0].title.contains("offline"));

    // Exclusion: printer hits minus offline ones.
    let excluded = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "printer -offline",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("exclusion query");
    assert_eq!(excluded.len(), 1);
    assert!(excluded[0].title.contains("queue"));

    // Partial token via the trigram leg: no lexeme "net" exists anywhere.
    let prefix = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "net",
        true,
        None,
        10,
        0,
    )
    .await
    .expect("prefix query");
    assert_eq!(prefix.len(), 2, "'net' finds 'network' rows");
    // With the trigram leg off, the same query is a no-op (FTS-only).
    let fts_only = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "net",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("fts-only query");
    assert!(fts_only.is_empty());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_ranking_prefers_title_matches_over_body_mentions() {
    let db = test_db().await;
    let s = seed_org(&db.app, 123).await;
    let mut tx = org_tx(&db.app, s.org).await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        fixed_uuid(124),
        None,
        "Backup failure on srv01",
        "routine check",
    )
    .await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        fixed_uuid(125),
        None,
        "Weekly report",
        "the backup failure details were attached",
    )
    .await;

    let hits = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "backup",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("ranked query");
    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits[0].title, "Backup failure on srv01",
        "title-weighted hit outranks body mention"
    );
    assert!(hits[0].score > hits[1].score);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_kind_filter_and_pagination() {
    let db = test_db().await;
    let s = seed_org(&db.app, 124).await;
    let mut tx = org_tx(&db.app, s.org).await;
    for i in 0..3_u64 {
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(126 + i),
            None,
            &format!("alpha ticket {i}"),
            "",
        )
        .await;
    }
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Client,
        fixed_uuid(129),
        None,
        "alpha client",
        "",
    )
    .await;

    let tickets_only = search::search_count(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "alpha",
        false,
        None,
        search::SEARCH_COUNT_CAP,
    )
    .await
    .expect("kind-filtered count");
    assert_eq!(tickets_only.total, 3);
    assert!(!tickets_only.capped);

    let all_kinds = search::search_count(
        &mut *tx,
        s.org,
        &["ticket".to_string(), "client".to_string()],
        "alpha",
        false,
        None,
        search::SEARCH_COUNT_CAP,
    )
    .await
    .expect("all-kind count");
    assert_eq!(all_kinds.total, 4);

    let page_one = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "alpha",
        false,
        None,
        2,
        0,
    )
    .await
    .expect("page one");
    let page_two = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "alpha",
        false,
        None,
        2,
        2,
    )
    .await
    .expect("page two");
    assert_eq!(page_one.len(), 2);
    assert_eq!(page_two.len(), 1);
    let page_ids: std::collections::HashSet<Uuid> = page_one
        .iter()
        .chain(page_two.iter())
        .map(|h| h.entity_id)
        .collect();
    assert_eq!(page_ids.len(), 3, "pages never overlap");
    tx.commit().await.expect("commit");
}

/// The keyset walk must reproduce the offset walk row for row, including
/// inside a score tie (five rows share one title, so five share one rank).
/// That tie is the case a cursor without the `(indexed_at, entity_id)`
/// tail would skip or repeat, and it is why the anchor carries all three.
#[tokio::test]
async fn search_keyset_walk_matches_the_offset_walk() {
    let db = test_db().await;
    let s = seed_org(&db.app, 134).await;
    let mut tx = org_tx(&db.app, s.org).await;
    for i in 0..10_u64 {
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(160 + i),
            None,
            if i < 5 {
                "gamma outage"
            } else {
                "gamma printer"
            },
            "",
        )
        .await;
    }
    let kinds = ["ticket".to_string()];

    let mut offset_ids = Vec::new();
    for page in 0..4 {
        let hits = search::search_page(&mut *tx, s.org, &kinds, "gamma", false, None, 3, page * 3)
            .await
            .expect("offset page");
        offset_ids.extend(hits.into_iter().map(|h| h.entity_id));
    }

    let mut keyset_ids = Vec::new();
    let mut cursor = None;
    loop {
        let hits =
            search::search_page_after(&mut *tx, s.org, &kinds, "gamma", false, None, 3, cursor)
                .await
                .expect("keyset page");
        let Some(last) = hits.last() else { break };
        cursor = Some(search::SearchCursor {
            score: last.score,
            indexed_at: last.indexed_at,
            entity_id: last.entity_id,
        });
        keyset_ids.extend(hits.into_iter().map(|h| h.entity_id));
        assert!(keyset_ids.len() <= 12, "cursor walk did not terminate");
    }

    assert_eq!(
        keyset_ids, offset_ids,
        "the walk serves the same rows in the same order"
    );
    assert_eq!(keyset_ids.len(), 10);
    let unique: std::collections::HashSet<Uuid> = keyset_ids.iter().copied().collect();
    assert_eq!(unique.len(), 10, "no row is served twice");
    tx.commit().await.expect("commit");
}

/// The count stops at the cap and says so. `cap + 1` is the sentinel that
/// separates "exactly at the cap" from "more than the cap"; without it a
/// capped count and an exact count look identical at the boundary.
#[tokio::test]
async fn search_count_caps_instead_of_scanning_the_whole_predicate() {
    let db = test_db().await;
    let s = seed_org(&db.app, 135).await;
    let mut tx = org_tx(&db.app, s.org).await;
    for i in 0..5_u64 {
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(170 + i),
            None,
            "delta item",
            "",
        )
        .await;
    }
    let kinds = ["ticket".to_string()];

    let exact = search::search_count(&mut *tx, s.org, &kinds, "delta", false, None, 10)
        .await
        .expect("count under the cap");
    assert_eq!(
        exact,
        search::SearchCount {
            total: 5,
            capped: false
        }
    );

    let capped = search::search_count(&mut *tx, s.org, &kinds, "delta", false, None, 3)
        .await
        .expect("count over the cap");
    assert_eq!(
        capped,
        search::SearchCount {
            total: 3,
            capped: true
        }
    );

    let at_cap = search::search_count(&mut *tx, s.org, &kinds, "delta", false, None, 5)
        .await
        .expect("count exactly at the cap");
    assert_eq!(
        at_cap,
        search::SearchCount {
            total: 5,
            capped: false
        }
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_rls_blocks_cross_org_hits_and_unscoped_writes() {
    let db = test_db().await;
    let a = seed_org(&db.admin, 125).await;
    let b = seed_org(&db.admin, 126).await;
    // Identical corpora in two orgs; isolation must hold regardless.
    for seeded in [&a, &b] {
        let mut tx = org_tx(&db.app, seeded.org).await;
        idx_upsert(
            &mut tx,
            seeded.org,
            search::SearchKind::Ticket,
            fixed_uuid(128),
            None,
            "firewall audit findings",
            "",
        )
        .await;
        tx.commit().await.expect("commit");
    }

    // RLS-scoped reader in org B sees exactly org B's row.
    let mut scoped_b = org_tx(&db.app, b.org).await;
    let hits_b = search::search_page(
        &mut *scoped_b,
        b.org,
        &["ticket".to_string()],
        "firewall",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("scoped query");
    assert_eq!(hits_b.len(), 1);
    tx_commit_org_check(scoped_b, a.org, "firewall").await;
}

/// Within an org-B transaction, org A rows are invisible even when the
/// query string would match them (belt-and-braces against filter drift).
async fn tx_commit_org_check(mut tx: Transaction<'_, Postgres>, other_org: Uuid, q: &str) {
    let leaks = search::search_page(
        &mut *tx,
        other_org,
        &["ticket".to_string()],
        q,
        false,
        None,
        10,
        0,
    )
    .await
    .expect("other-org query under B scope");
    assert!(leaks.is_empty(), "RLS scope must hide foreign-org rows");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_unscoped_app_role_cannot_read_or_write() {
    let db = test_db().await;
    let s = seed_org(&db.admin, 127).await;
    {
        let mut tx = org_tx(&db.app, s.org).await;
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(129),
            None,
            "secret projector code",
            "",
        )
        .await;
        tx.commit().await.expect("commit");
    }
    // A pool that has NEVER set app.org_id sees zero rows…
    let fresh = db.fresh_app_pool().await;
    let mut unscoped = fresh.begin().await.expect("begin");
    let hits = search::search_page(
        &mut *unscoped,
        s.org,
        &["ticket".to_string()],
        "projector",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("unscoped read");
    assert!(
        hits.is_empty(),
        "default-deny: unset org scope reads nothing"
    );
    // …and the write path is equally denied by the policy's implicit WITH CHECK.
    let result = search::upsert_row(
        &mut *unscoped,
        s.org,
        search::SearchKind::Ticket,
        fixed_uuid(128),
        None,
        "should not land",
        "",
        None,
    )
    .await;
    match result {
        Err(DbError::Sql(sqlx::Error::Database(db_err))) => {
            assert_eq!(db_err.code().unwrap_or_default(), "42501")
        }
        other => panic!("expected RLS violation, got {other:?}"),
    }
}

// --- attachments (the private build tracker 2.14) --------------------------------------------

/// Inserts one attachment row through the public query API.
#[allow(clippy::too_many_arguments)]
async fn put_attachment(
    conn: &mut PgConnection,
    org: Uuid,
    id: Uuid,
    kind: core_domain::entities::AttachmentKind,
    ticket_id: Option<Uuid>,
    comment_id: Option<Uuid>,
    sha_hex: &str,
    uploader: Option<Uuid>,
) -> Attachment {
    let key = format!("org/{org}/attachments/{sha_hex}");
    core_db::attachments::insert(
        conn,
        core_db::attachments::NewAttachment {
            id,
            org_id: org,
            kind,
            ticket_id,
            ticket_comment_id: comment_id,
            email_message_id: None,
            filename: "evidence.png",
            content_type: "image/png",
            size_bytes: 42,
            sha256: sha_hex,
            storage_key: &key,
            uploaded_by_user_id: uploader,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("attachment insert")
}

/// Dedup shares one stored object between rows; refcount-gated delete
/// reclaims only when the LAST reference goes; linking is one-shot.
#[tokio::test]
async fn attachments_dedup_link_and_refcounted_delete() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 130).await;
    let ua = seed_user(&db.app, &sa, 13000).await;

    let mut tx = org_tx(&db.app, sa.org).await;
    let t1 = create_ticket(
        &mut tx,
        &sa,
        fixed_uuid(13001),
        "Attach me",
        Priority::P2,
        sa.entry_status,
        None,
    )
    .await;
    let c1 = tickets::insert_comment(
        &mut *tx,
        tickets::NewTicketComment {
            id: fixed_uuid(13002),
            org_id: sa.org,
            ticket_id: t1.id,
            author_id: Some(ua),
            kind: NoteKind::Internal,
            body: "with file",
            workflow_meta: None,
        },
    )
    .await
    .expect("comment");

    // First upload claims the content-addressed object…
    let a1 = put_attachment(
        &mut tx,
        sa.org,
        fixed_uuid(13003),
        core_domain::entities::AttachmentKind::TicketComment,
        Some(t1.id),
        Some(c1.id),
        &"a".repeat(64),
        Some(ua),
    )
    .await;
    assert_eq!(
        core_db::attachments::find_existing_object_key(&mut tx, sa.org, &"a".repeat(64))
            .await
            .expect("dedup"),
        Some(a1.storage_key.clone())
    );

    // …second row REUSES it (same bytes attached elsewhere).
    let a2 = put_attachment(
        &mut tx,
        sa.org,
        fixed_uuid(13004),
        core_domain::entities::AttachmentKind::TicketComment,
        Some(t1.id),
        None,
        &"a".repeat(64),
        Some(ua),
    )
    .await;
    assert_eq!(a1.storage_key, a2.storage_key);

    // Linking the still-unlinked upload is one-shot.
    let linked = core_db::attachments::link_to_comment(&mut tx, sa.org, a2.id, t1.id, c1.id)
        .await
        .expect("link")
        .expect("linked");
    assert_eq!(linked.ticket_comment_id, Some(c1.id));
    assert!(
        core_db::attachments::link_to_comment(&mut tx, sa.org, a2.id, t1.id, c1.id)
            .await
            .expect("relink")
            .is_none()
    );

    // Listing sees both rows for the ticket and per-comment.
    assert_eq!(
        core_db::attachments::list_for_ticket(&mut *tx, sa.org, t1.id)
            .await
            .expect("list")
            .len(),
        2
    );
    assert_eq!(
        core_db::attachments::list_for_comment(&mut *tx, sa.org, c1.id)
            .await
            .expect("per-comment")
            .len(),
        2
    );

    tx.commit().await.expect("commit");

    // Delete row 1: object stays (row 2 still references the key).
    let mut tx = org_tx(&db.app, sa.org).await;
    let gone = core_db::attachments::delete(&mut tx, sa.org, a1.id)
        .await
        .expect("del")
        .expect("row");
    assert_eq!(gone.sha256, "a".repeat(64));
    let refs = core_db::attachments::count_other_refs(&mut tx, sa.org, &a1.storage_key, a1.id)
        .await
        .expect("refs");
    assert_eq!(refs, 1, "sibling reference keeps the object alive");

    // Delete row 2: zero refs → caller may reclaim the object.
    core_db::attachments::delete(&mut tx, sa.org, a2.id)
        .await
        .expect("del2")
        .expect("row2");
    let refs = core_db::attachments::count_other_refs(&mut tx, sa.org, &a1.storage_key, a2.id)
        .await
        .expect("refs");
    assert_eq!(refs, 0);
    tx.commit().await.expect("commit del");
}

/// RLS: attachments are invisible cross-org and default-deny unscoped.
#[tokio::test]
async fn rls_isolates_attachments_table() {
    use core_domain::entities::AttachmentKind;
    let db = test_db().await;
    let sa = seed_org(&db.app, 132).await;
    let sb = seed_org(&db.app, 133).await;
    let ua = seed_user(&db.app, &sa, 13300).await;

    let mut tx_a = org_tx(&db.app, sa.org).await;
    let t1 = create_ticket(
        &mut tx_a,
        &sa,
        fixed_uuid(13301),
        "Secret file",
        Priority::P3,
        sa.entry_status,
        None,
    )
    .await;
    put_attachment(
        &mut tx_a,
        sa.org,
        fixed_uuid(13302),
        AttachmentKind::Ticket,
        Some(t1.id),
        None,
        &"b".repeat(64),
        Some(ua),
    )
    .await;
    tx_a.commit().await.expect("commit A");

    // Org B cannot read or delete org A's attachment.
    let mut tx_b = org_tx(&db.app, sb.org).await;
    assert!(
        core_db::attachments::get(&mut *tx_b, sb.org, fixed_uuid(13302))
            .await
            .is_err()
    );
    assert!(
        core_db::attachments::list_for_ticket(&mut *tx_b, sb.org, t1.id)
            .await
            .expect("empty")
            .is_empty()
    );
    assert!(
        core_db::attachments::delete(&mut tx_b, sb.org, fixed_uuid(13302))
            .await
            .expect("no-op delete")
            .is_none()
    );
    // Dedup lookup never leaks across orgs either.
    assert_eq!(
        core_db::attachments::find_existing_object_key(&mut tx_b, sb.org, &"b".repeat(64))
            .await
            .expect("dedup b"),
        None
    );
    tx_b.commit().await.expect("commit B");

    // Unscoped connection (no app.org_id) sees nothing.
    let unscoped = db.fresh_app_pool().await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&unscoped)
        .await
        .unwrap_or_else(|e| panic!("unscoped count attachments: {e}"));
    assert_eq!(count, 0, "unscoped reads on attachments are default-deny");
}

// --- Custom fields v1 (the private build tracker 2.17; seed slots 140–149) --------------------

use core_db::custom_fields;

fn field_def(
    org: Uuid,
    board: Option<Uuid>,
    id: Uuid,
    name: &str,
    kind: &str,
    config: serde_json::Value,
    required: bool,
) -> custom_fields::FieldDefRow {
    custom_fields::FieldDefRow {
        id,
        org_id: org,
        board_id: board,
        entity: "ticket".into(),
        name: name.into(),
        kind: kind.into(),
        config,
        required,
        sort_order: 0,
    }
}

#[tokio::test]
async fn custom_field_defs_crud_board_scoping_and_cascade() {
    let db = test_db().await;
    let s = seed_org(&db.app, 140).await;
    let owned = fixed_uuid(1401);
    let org_wide = fixed_uuid(1402);
    let foreign_board = fixed_uuid(1499);

    {
        let mut tx = org_tx(&db.app, s.org).await;
        custom_fields::insert_def(
            &mut tx,
            &field_def(
                s.org,
                Some(s.board),
                owned,
                "Tier",
                "select",
                serde_json::json!({"options": ["Gold"]}),
                true,
            ),
        )
        .await
        .expect("insert board-scoped def");
        custom_fields::insert_def(
            &mut tx,
            &field_def(
                s.org,
                None,
                org_wide,
                "Notes",
                "text",
                serde_json::json!({}),
                false,
            ),
        )
        .await
        .expect("insert org-wide def");

        // Board view unions its own defs with the org-wide ones.
        let listed = custom_fields::list_defs_for_board(&mut *tx, s.org, s.board)
            .await
            .expect("list defs");
        let ids: Vec<Uuid> = listed.iter().map(|d| d.id).collect();
        assert!(ids.contains(&owned) && ids.contains(&org_wide));

        // A different board sees only the org-wide def.
        let other_view = custom_fields::list_defs_for_board(&mut *tx, s.org, foreign_board)
            .await
            .expect("list defs other board");
        assert_eq!(other_view.len(), 1);
        assert_eq!(other_view[0].id, org_wide);

        // Partial update flips required and renames; untouched columns stay.
        let updated = custom_fields::update_def(
            &mut tx,
            s.org,
            owned,
            &custom_fields::DefPatch {
                name: Some("Support tier"),
                config: None,
                required: None,
                sort_order: Some(7),
            },
        )
        .await
        .expect("update def");
        assert_eq!(updated.name, "Support tier");
        assert!(updated.required, "required unchanged");
        assert_eq!(updated.sort_order, 7);

        // Archived defs drop out of the active read.
        sqlx::query("UPDATE custom_field_defs SET archived_at = now() WHERE id = $1")
            .bind(org_wide)
            .execute(&mut *tx)
            .await
            .expect("archive");
        let active = custom_fields::list_defs_for_board(&mut *tx, s.org, s.board)
            .await
            .expect("list active");
        assert_eq!(active.len(), 1, "archived def hidden");

        // get_def misses archived rows too.
        match custom_fields::get_def(&mut tx, s.org, org_wide).await {
            Err(DbError::NotFound) => {}
            other => panic!("archived def must be NotFound, got {other:?}"),
        }
        tx.commit().await.expect("commit");
    }

    // Deleting a definition cascades its stored values.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        let ticket = create_ticket(
            &mut tx,
            &s,
            fixed_uuid(1403),
            "cascade host",
            Priority::P2,
            s.entry_status,
            None,
        )
        .await;
        custom_fields::upsert_value(&mut tx, s.org, owned, ticket.id, &serde_json::json!("Gold"))
            .await
            .expect("store value");
        tx.commit().await.expect("commit value");
        let mut tx = org_tx(&db.app, s.org).await;
        custom_fields::delete_def(&mut tx, s.org, owned)
            .await
            .expect("delete def cascades");
        let remaining = custom_fields::list_values(&mut *tx, s.org, ticket.id)
            .await
            .expect("values after cascade");
        assert!(remaining.is_empty(), "values cascade-deleted");
        // Second delete is NotFound.
        match custom_fields::delete_def(&mut tx, s.org, owned).await {
            Err(DbError::NotFound) => {}
            other => panic!("double delete must be NotFound, got {other:?}"),
        }
        tx.commit().await.expect("commit delete");
    }
}

#[tokio::test]
async fn custom_field_values_upsert_clear_and_batch_fetch() {
    let db = test_db().await;
    let s = seed_org(&db.app, 141).await;
    let def_a = fixed_uuid(1411);
    let def_b = fixed_uuid(1412);
    let t1 = fixed_uuid(1413);
    let t2 = fixed_uuid(1414);

    {
        let mut tx = org_tx(&db.app, s.org).await;
        custom_fields::insert_def(
            &mut tx,
            &field_def(
                s.org,
                Some(s.board),
                def_a,
                "Channel",
                "select",
                serde_json::json!({"options": ["Email", "Phone"]}),
                false,
            ),
        )
        .await
        .expect("def a");
        custom_fields::insert_def(
            &mut tx,
            &field_def(
                s.org,
                None,
                def_b,
                "Hours",
                "number",
                serde_json::json!({}),
                false,
            ),
        )
        .await
        .expect("def b");
        for t in [t1, t2] {
            create_ticket(
                &mut tx,
                &s,
                t,
                "batch host",
                Priority::P2,
                s.entry_status,
                None,
            )
            .await;
        }
        tx.commit().await.expect("seed commit");
    }

    {
        let mut tx = org_tx(&db.app, s.org).await;
        // Upsert writes then overwrites (ON CONFLICT path).
        custom_fields::upsert_value(&mut tx, s.org, def_a, t1, &serde_json::json!("Email"))
            .await
            .expect("first write");
        custom_fields::upsert_value(&mut tx, s.org, def_a, t1, &serde_json::json!("Phone"))
            .await
            .expect("overwrite");
        custom_fields::upsert_value(&mut tx, s.org, def_b, t1, &serde_json::json!(4.5))
            .await
            .expect("second field");
        custom_fields::upsert_value(&mut tx, s.org, def_a, t2, &serde_json::json!("Email"))
            .await
            .expect("other ticket");

        // One batched query groups values per ticket.
        let grouped = custom_fields::list_values_many(&mut *tx, s.org, &[t1, t2])
            .await
            .expect("batch fetch");
        assert_eq!(grouped[&t1].len(), 2);
        assert_eq!(grouped[&t2].len(), 1);
        assert_eq!(
            grouped[&t1][0].value,
            serde_json::json!("Phone"),
            "overwritten"
        );
        // Definition context rides along.
        assert_eq!(grouped[&t1][0].name, "Channel");

        // Empty id slice short-circuits to an empty map.
        let none = custom_fields::list_values_many(&mut *tx, s.org, &[])
            .await
            .expect("empty batch");
        assert!(none.is_empty());

        // Clear removes exactly one (field, ticket) pair.
        let removed = custom_fields::clear_value(&mut tx, s.org, def_a, t1)
            .await
            .expect("clear");
        assert!(removed);
        let after = custom_fields::list_values(&mut *tx, s.org, t1)
            .await
            .expect("after clear");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].field_id, def_b);
        // Clearing again reports nothing removed.
        let again = custom_fields::clear_value(&mut tx, s.org, def_a, t1)
            .await
            .expect("clear again");
        assert!(!again);
        tx.commit().await.expect("commit");
    }
}

#[tokio::test]
async fn custom_fields_rls_blocks_cross_org_and_unscoped() {
    let db = test_db().await;
    let sa = seed_org(&db.app, 142).await;
    let sb = seed_org(&db.app, 143).await;
    let def_a = fixed_uuid(1421);

    {
        let mut tx = org_tx(&db.app, sa.org).await;
        custom_fields::insert_def(
            &mut tx,
            &field_def(
                sa.org,
                Some(sa.board),
                def_a,
                "Secret tier",
                "text",
                serde_json::json!({}),
                false,
            ),
        )
        .await
        .expect("org A def");
        let ticket = create_ticket(
            &mut tx,
            &sa,
            fixed_uuid(1422),
            "rls host",
            Priority::P2,
            sa.entry_status,
            None,
        )
        .await;
        custom_fields::upsert_value(&mut tx, sa.org, def_a, ticket.id, &serde_json::json!("x"))
            .await
            .expect("org A value");
        tx.commit().await.expect("commit");
    }

    // Foreign org scope hides both the definition and the value row.
    {
        let mut tx = org_tx(&db.app, sb.org).await;
        assert_eq!(
            count_rows(&mut tx, "custom_field_defs", def_a).await,
            0,
            "cross-org defs hidden"
        );
    }

    // No scope at all: default-deny on both tables.
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("begin unscoped");
        assert_eq!(
            count_rows(&mut tx, "custom_field_defs", def_a).await,
            0,
            "defs hidden unscoped"
        );
        let values = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM custom_field_values WHERE field_id = $1",
        )
        .bind(def_a)
        .fetch_one(&mut *tx)
        .await
        .expect("count values");
        assert_eq!(values, 0, "values hidden unscoped");
    }
}

// ===========================================================================
// Read-model reporting views (the private build tracker R0 / migration 0026) — fixture math
// against hand-computed expectations, plus RLS-through-views isolation.
//
// Seed-slot discipline (lane contract): the ONLY fixed_uuid anchors used
// here are 180..=189. Child entities derive deterministic ids by offsetting
// slot 184's u128, preserving cross-lane collision-freedom without
// consuming extra fixed slots.
// ===========================================================================

fn lane_uuid(k: u64) -> Uuid {
    Uuid::from_u128(fixed_uuid(184).as_u128() + k as u128)
}

fn view_day() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()
}

/// Wall-clock anchor for fixtures: 2026-06-10T<hour>:00:00Z.
fn view_at(hour: i64) -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc
        .with_ymd_and_hms(2026, 6, 10, hour as u32, 0, 0)
        .unwrap()
}

#[allow(dead_code)]
struct ViewFixtures {
    org_a: Seeded,
    org_b: Seeded,
    user_a1: Uuid,
    user_a2: Uuid,
}

/// Shared ONCE across the two view tests: they would otherwise race on the
/// same deterministic org ids (parallel #[tokio::test]s share one database).
static VIEW_FIXTURES: tokio::sync::OnceCell<ViewFixtures> = tokio::sync::OnceCell::const_new();

async fn view_fixtures(db: &TestDb) -> &'static ViewFixtures {
    VIEW_FIXTURES
        .get_or_init(|| async { seed_view_fixtures_impl(db).await })
        .await
}

async fn seed_view_fixtures_impl(db: &TestDb) -> ViewFixtures {
    let org_a = seed_org(&db.app, 180).await;
    let org_b = seed_org(&db.app, 181).await;
    let user_a1 = fixed_uuid(182);
    let user_a2 = fixed_uuid(183);
    let client_a = lane_uuid(0);

    // Users + client in org A.
    let mut tx = org_tx(&db.app, org_a.org).await;
    users::create_user(
        &mut *tx,
        user_a1,
        org_a.org,
        "a1@views.test",
        "Alpha One",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user a1");
    users::create_user(
        &mut *tx,
        user_a2,
        org_a.org,
        "a2@views.test",
        "Beta Two",
        "argon2hash",
        "Europe/Berlin",
    )
    .await
    .expect("user a2");
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1,$2,'Client Alpha')")
        .bind(client_a)
        .bind(org_a.org)
        .execute(&mut *tx)
        .await
        .expect("client");
    tx.commit().await.expect("users commit");

    let mut tx = org_tx(&db.app, org_a.org).await;

    // Anchor creations on the fixture day so latency/window math is
    // hand-computable regardless of wall clock.
    // Tickets through the public insert path (number allocation included).
    let t_closed = create_ticket(
        &mut tx,
        &org_a,
        lane_uuid(1),
        "closed promptly",
        Priority::P3,
        org_a.entry_status,
        Some(user_a1),
    )
    .await;
    let t_wait = create_ticket(
        &mut tx,
        &org_a,
        lane_uuid(2),
        "client shouted first",
        Priority::P3,
        org_a.waiting_status,
        None,
    )
    .await;
    let t_open_breach = create_ticket(
        &mut tx,
        &org_a,
        lane_uuid(3),
        "breached open",
        Priority::P1,
        org_a.entry_status,
        Some(user_a2),
    )
    .await;

    for t in [&t_closed.id, &t_wait.id] {
        sqlx::query("UPDATE tickets SET created_at = $2 WHERE id = $1")
            .bind(t)
            .bind(view_at(0))
            .execute(&mut *tx)
            .await
            .expect("created pin");
    }
    sqlx::query("UPDATE tickets SET created_at = $2 WHERE id = $1")
        .bind(t_open_breach.id)
        .bind(view_at(0))
        .execute(&mut *tx)
        .await
        .expect("created pin");

    // Closed ticket: real workflow move into the closed category (sets the
    // denormalized category + resolved_at transactionally), then pin the
    // resolution instant for hand-computed latency math. The board's
    // resolved-status id is fetched under the same org scope.
    let resolved_status_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id=$1 AND category='closed' LIMIT 1",
    )
    .bind(org_a.board)
    .fetch_one(&mut *tx)
    .await
    .expect("resolved status");
    tickets::update_status(&mut *tx, org_a.org, t_closed.id, resolved_status_id)
        .await
        .expect("close move");
    sqlx::query("UPDATE tickets SET resolved_at = $2 WHERE id = $1")
        .bind(t_closed.id)
        .bind(view_at(8))
        .execute(&mut *tx)
        .await
        .expect("resolve pin");

    // Portal-style reply BEFORE any staff comment: author_id NULL ⇒ never a
    // staff response.
    sqlx::query(
        "INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body, created_at)
         VALUES ($1,$2,$3,NULL,'client_visible','portal question',$4)",
    )
    .bind(lane_uuid(4))
    .bind(org_a.org)
    .bind(t_wait.id)
    .bind(view_at(2))
    .execute(&mut *tx)
    .await
    .expect("portal comment");

    // Staff internal comment at h6 → the true first response for t_wait.
    sqlx::query(
        "INSERT INTO ticket_comments (id, org_id, ticket_id, author_id, kind, body, created_at)
         VALUES ($1,$2,$3,$4,'internal','we are on it',$5)",
    )
    .bind(lane_uuid(5))
    .bind(org_a.org)
    .bind(t_wait.id)
    .bind(user_a1)
    .bind(view_at(6))
    .execute(&mut *tx)
    .await
    .expect("staff comment");

    // Time entries with hand-computable sums (60m billable / 30m not).
    sqlx::query(
        "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable)
         VALUES ($1,$2,$3,$4,$5,$6,$7,true)",
    )
    .bind(lane_uuid(10))
    .bind(org_a.org)
    .bind(t_closed.id)
    .bind(user_a1)
    .bind(view_at(9))
    .bind(view_at(10))
    .bind(60i32)
    .execute(&mut *tx)
    .await
    .expect("entry billable");

    sqlx::query(
        "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable)
         VALUES ($1,$2,$3,$4,$5,$6,$7,false)",
    )
    .bind(lane_uuid(11))
    .bind(org_a.org)
    .bind(t_wait.id)
    .bind(user_a2)
    .bind(view_at(11))
    .bind(view_at(11) + chrono::Duration::minutes(30))
    .bind(30i32)
    .execute(&mut *tx)
    .await
    .expect("entry non-billable");

    // SLA state: closed ticket fully met; open ticket breached/unresolved.
    sqlx::query(
        "INSERT INTO ticket_sla_state
           (ticket_id, org_id, policy_id, first_response_due, first_responded_at,
            resolution_due, breached)
         SELECT $1,$2,p.id,$3,$4,$5,false FROM sla_policies p WHERE p.org_id=$2 LIMIT 1",
    )
    .bind(t_closed.id)
    .bind(org_a.org)
    .bind(view_at(4))
    .bind(view_at(1))
    .bind(view_at(12))
    .execute(&mut *tx)
    .await
    .expect("sla met row");

    sqlx::query(
        "INSERT INTO ticket_sla_state
           (ticket_id, org_id, policy_id, first_response_due, resolution_due, breached)
         SELECT $1,$2,p.id,$3,$4,true FROM sla_policies p WHERE p.org_id=$2 LIMIT 1",
    )
    .bind(t_open_breach.id)
    .bind(org_a.org)
    .bind(view_at(2))
    .bind(view_at(12))
    .execute(&mut *tx)
    .await
    .expect("sla breach row");

    // Gap audit ledger: one clean day + one gap day for a1.
    for (id_slot, day, logged, gap) in [
        (lane_uuid(12), view_day(), 300i32, false),
        (
            lane_uuid(13),
            view_day() - chrono::Duration::days(1),
            120i32,
            true,
        ),
    ] {
        sqlx::query(
            "INSERT INTO time_gap_audits (id, org_id, user_id, day, activity, logged_minutes, expected_minutes, has_gap)
             VALUES ($1,$2,$3,$4,true,$5,480,$6)",
        )
        .bind(id_slot)
        .bind(org_a.org)
        .bind(user_a1)
        .bind(day)
        .bind(logged)
        .bind(gap)
        .execute(&mut *tx)
        .await
        .expect("gap audit row");
    }

    tx.commit().await.expect("fixture commit");

    ViewFixtures {
        org_a,
        org_b,
        user_a1,
        user_a2,
    }
}

#[derive(sqlx::FromRow, Debug)]
#[allow(dead_code)]
struct TicketRow {
    number: String,
    status_name: String,
    category: String,
    client_name: Option<String>,
    assignee_name: Option<String>,
    breached: bool,
}

#[tokio::test]
async fn views_project_fixture_math_exactly() {
    let db = test_db().await;
    let fx = view_fixtures(&db).await;

    let mut tx = org_tx(&db.app, fx.org_a.org).await;

    // --- ops.v1_tickets ----------------------------------------------------
    let tickets_rows: Vec<TicketRow> = sqlx::query_as(
        r#"SELECT number, status_name, category, client_name, assignee_name, breached
           FROM ops.v1_tickets WHERE org_id = $1 ORDER BY number"#,
    )
    .bind(fx.org_a.org)
    .fetch_all(&mut *tx)
    .await
    .expect("v1_tickets scan");
    assert_eq!(tickets_rows.len(), 3, "{tickets_rows:?}");
    assert!(
        tickets_rows
            .iter()
            .any(|r| r.category == "closed" && r.status_name == "Resolved" && !r.breached),
        "closed workflow truth surfaced: {tickets_rows:?}"
    );

    // SLA columns ride along where state exists and coalesce to false else.
    let (n_with_sla, n_without): (i64, i64) = sqlx::query_as(
        r#"SELECT COUNT(*) FILTER (WHERE resolution_due IS NOT NULL),
                                COUNT(*) FILTER (WHERE resolution_due IS NULL)
                          FROM ops.v1_tickets WHERE org_id = $1"#,
    )
    .bind(fx.org_a.org)
    .fetch_one(&mut *tx)
    .await
    .expect("sla split");
    assert_eq!((n_with_sla, n_without), (2, 1));

    // --- ops.v1_time_entries ----------------------------------------------
    #[derive(sqlx::FromRow, Debug)]
    #[allow(dead_code)]
    struct EntryRow {
        minutes: i32,
        billable: bool,
        ticket_number: String,
        user_display_name: Option<String>,
    }
    let entry_rows: Vec<EntryRow> = sqlx::query_as(
        r#"SELECT minutes, billable, ticket_number, user_display_name
           FROM ops.v1_time_entries WHERE org_id = $1 ORDER BY started_at"#,
    )
    .bind(fx.org_a.org)
    .fetch_all(&mut *tx)
    .await
    .expect("v1_time_entries scan");
    assert_eq!(entry_rows.len(), 2, "{entry_rows:?}");
    assert_eq!((entry_rows[0].minutes, entry_rows[0].billable), (60, true));
    assert_eq!(
        entry_rows[0].user_display_name.as_deref(),
        Some("Alpha One")
    );
    assert_eq!(entry_rows[1].user_display_name.as_deref(), Some("Beta Two"));

    // --- ops.v1_sla_outcomes ----------------------------------------------
    #[derive(sqlx::FromRow, Debug)]
    struct OutcomeRow {
        response_met: Option<bool>,
        resolution_met: Option<bool>,
        breached: bool,
        calendar_name: Option<String>,
    }
    let outcomes: Vec<OutcomeRow> = sqlx::query_as(
        r#"SELECT response_met, resolution_met, breached, calendar_name
           FROM ops.v1_sla_outcomes WHERE org_id = $1"#,
    )
    .bind(fx.org_a.org)
    .fetch_all(&mut *tx)
    .await
    .expect("v1_sla_outcomes scan");
    assert_eq!(outcomes.len(), 2);
    let met_row = outcomes.iter().find(|r| !r.breached).expect("met row");
    assert_eq!(met_row.response_met, Some(true));
    assert_eq!(met_row.resolution_met, Some(true));
    assert_eq!(
        met_row.calendar_name, None,
        "embedded-calendar policy has no attached calendar"
    );
    let breach_row = outcomes.iter().find(|r| r.breached).expect("breach row");
    assert_eq!(
        breach_row.resolution_met, None,
        "unresolved ⇒ three-valued NULL"
    );

    // --- ops.v1_first_response --------------------------------------------
    #[derive(sqlx::FromRow, Debug)]
    #[allow(dead_code)]
    struct FirstResponseRow {
        ticket_number: String,
        created_at: chrono::DateTime<chrono::Utc>,
        first_staff_comment_at: Option<chrono::DateTime<chrono::Utc>>,
        first_response_secs: Option<i64>,
    }
    let fr_rows: Vec<FirstResponseRow> = sqlx::query_as(
        r#"SELECT ticket_number, created_at, first_staff_comment_at, first_response_secs
           FROM ops.v1_first_response WHERE org_id = $1 ORDER BY ticket_number"#,
    )
    .bind(fx.org_a.org)
    .fetch_all(&mut *tx)
    .await
    .expect("v1_first_response scan");
    assert_eq!(fr_rows.len(), 3);

    let wait_row = fr_rows
        .iter()
        .find(|r| r.first_staff_comment_at.is_some())
        .expect("answered");
    assert_eq!(
        wait_row.first_staff_comment_at.unwrap(),
        view_at(6),
        "portal reply at h2 is EXCLUDED; staff at h6 measured"
    );
    assert_eq!(
        wait_row.first_response_secs.unwrap(),
        (view_at(6) - view_at(0)).num_seconds(),
        "latency anchored on creation"
    );
    for unanswered in fr_rows
        .iter()
        .filter(|r| r.first_staff_comment_at.is_none())
    {
        assert!(
            unanswered.first_response_secs.is_none(),
            "unanswered ⇒ NULL latency"
        );
    }

    // --- ops.v1_gap_days ----------------------------------------------------
    #[derive(sqlx::FromRow, Debug, PartialEq)]
    struct GapRow {
        has_gap: bool,
        logged_minutes: i32,
        expected_minutes: i32,
        user_timezone: String,
    }
    let gaps: Vec<GapRow> = sqlx::query_as(
        r#"SELECT has_gap, logged_minutes, expected_minutes, user_timezone
           FROM ops.v1_gap_days WHERE org_id = $1 ORDER BY day"#,
    )
    .bind(fx.org_a.org)
    .fetch_all(&mut *tx)
    .await
    .expect("v1_gap_days scan");
    assert_eq!(gaps.len(), 2);
    // ORDER BY day ASC ⇒ June 9 (gap) first, June 10 clean second.
    assert_eq!(
        gaps[0],
        GapRow {
            has_gap: true,
            logged_minutes: 120,
            expected_minutes: 480,
            user_timezone: "UTC".into()
        }
    );
    assert_eq!(
        gaps[1],
        GapRow {
            has_gap: false,
            logged_minutes: 300,
            expected_minutes: 480,
            user_timezone: "UTC".into()
        }
    );

    tx.commit().await.ok();
}

#[tokio::test]
async fn rls_through_views_isolates_cross_org_and_unscoped_sessions() {
    let db = test_db().await;
    let fx = view_fixtures(&db).await;

    // Org B gets its own ticket + entry + comment (same physical tables).
    let mut tx = org_tx(&db.app, fx.org_b.org).await;
    let user_b = fixed_uuid(185);
    users::create_user(
        &mut *tx,
        user_b,
        fx.org_b.org,
        "b1@views.test",
        "Rival Tech",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("org b user");
    let t_b = create_ticket(
        &mut tx,
        &fx.org_b,
        lane_uuid(20),
        "rival ticket",
        Priority::P2,
        fx.org_b.entry_status,
        None,
    )
    .await;
    sqlx::query(
        "INSERT INTO time_entries (id, org_id, ticket_id, user_id, started_at, ended_at, minutes, billable)
         SELECT $1,$2,t.id,u.id,$3,$4,90,true FROM tickets t CROSS JOIN users u
         WHERE t.id=$5 AND u.org_id=$2 LIMIT 1",
    )
    .bind(lane_uuid(21))
    .bind(fx.org_b.org)
    .bind(view_at(9))
    .bind(view_at(10))
    .bind(t_b.id)
    .execute(&mut *tx)
    .await
    .expect("org b entry");
    tx.commit().await.expect("org b commit");

    // Scoped org A session: EVERY view returns zero org B rows.
    let mut tx = org_tx(&db.app, fx.org_a.org).await;
    for (view, key_col) in [
        ("ops.v1_tickets", "id"),
        ("ops.v1_time_entries", "id"),
        ("ops.v1_sla_outcomes", "ticket_id"),
        ("ops.v1_gap_days", "id"),
        ("ops.v1_first_response", "ticket_id"),
    ] {
        let foreign: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {view} WHERE {key_col} = $1"))
                .bind(t_b.id)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or_else(|e| panic!("{view}: {e}"));
        assert_eq!(foreign, 0, "{view} leaked org B rows through RLS");
    }

    // Org B scope sees ONLY org B data: one ticket, one entry, no gap days.
    drop(tx);
    let mut tx = org_tx(&db.app, fx.org_b.org).await;
    let (b_tickets, b_entries, b_gaps): (i64, i64, i64) = sqlx::query_as(
        r#"SELECT (SELECT COUNT(*) FROM ops.v1_tickets WHERE org_id=$1),
                  (SELECT COUNT(*) FROM ops.v1_time_entries WHERE org_id=$1),
                  (SELECT COUNT(*) FROM ops.v1_gap_days WHERE org_id=$1)"#,
    )
    .bind(fx.org_b.org)
    .fetch_one(&mut *tx)
    .await
    .expect("org b counts");
    assert_eq!((b_tickets, b_entries, b_gaps), (1, 1, 0));

    // Unscoped session (no app.org_id ever set): default deny on every view.
    drop(tx);
    let fresh = db.fresh_app_pool().await;
    for view in [
        "ops.v1_tickets",
        "ops.v1_time_entries",
        "ops.v1_sla_outcomes",
        "ops.v1_gap_days",
        "ops.v1_first_response",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {view}"))
            .fetch_one(&fresh)
            .await
            .unwrap_or_else(|e| panic!("{view} unscoped scan failed: {e}"));
        assert_eq!(count, 0, "{view} leaked rows without an org binding");
    }
}

// ===========================================================================
// Instant-first write-path storage (the private build tracker IF1–IF3, migration 0028) —
// org-scoped RLS on the idempotency/write-trail tables plus the per-entity
// seq counter contract.
//
// Seed-slot discipline (lane contract): fixed_uuid anchors 186 and 187 from
// this lane's reserved 180..=189 band; child entities derive from 188.
// ===========================================================================

fn if_uuid(k: u64) -> Uuid {
    Uuid::from_u128(fixed_uuid(188).as_u128() + k as u128)
}

static IF_FIXTURES: tokio::sync::OnceCell<(Uuid, Uuid)> = tokio::sync::OnceCell::const_new();

async fn if_fixtures(db: &TestDb) -> (Uuid, Uuid) {
    *IF_FIXTURES
        .get_or_init(|| async {
            let org_a = seed_org(&db.app, 186).await.org;
            let org_b = seed_org(&db.app, 187).await.org;
            // Owner-pool writes: one claimed idempotency key + one trail row
            // + a seq counter per org, so both orgs hold physically distinct
            // rows with IDENTICAL keys/ids — only RLS can tell them apart.
            for (org, marker, key_row) in [(org_a, "a", 1), (org_b, "b", 11)] {
                let entity_id = if_uuid(0);
                sqlx::query(
                    "INSERT INTO idempotency_keys (id, org_id, key, entity, entity_id, request_hash, response, actor_id) \
                     VALUES ($1, $2, 'shared-key', 'ticket', $3, 'hash-' || $4, NULL, $3)",
                )
                .bind(if_uuid(key_row))
                .bind(org)
                .bind(entity_id)
                .bind(marker)
                .execute(&db.admin)
                .await
                .expect("seed idempotency key");
                sqlx::query(
                    "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, before, after) \
                     VALUES ($1, $2, 'ticket', $3, 1, 'create', $3, NULL, jsonb_build_object('marker', $4))",
                )
                .bind(if_uuid(key_row + 1))
                .bind(org)
                .bind(entity_id)
                .bind(marker)
                .execute(&db.admin)
                .await
                .expect("seed trail row");
                sqlx::query(
                    "INSERT INTO entity_write_seqs (org_id, entity, entity_id, seq) VALUES ($1, 'ticket', $2, 1)",
                )
                .bind(org)
                .bind(entity_id)
                .execute(&db.admin)
                .await
                .expect("seed counter");
            }
            (org_a, org_b)
        })
        .await
}

#[tokio::test]
async fn instant_first_tables_are_org_scoped_under_rls() {
    let db = test_db().await;
    let (org_a, org_b) = if_fixtures(&db).await;
    let entity_id = if_uuid(0);

    // Scoped to org A: only org A's rows are visible, even though org B's
    // rows share the key and entity id.
    let mut tx = org_tx(&db.app, org_a).await;
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM idempotency_keys")
        .fetch_all(&mut *tx)
        .await
        .expect("keys visible to org A");
    assert_eq!(keys, vec!["shared-key".to_string()]);
    let hash: String =
        sqlx::query_scalar("SELECT request_hash FROM idempotency_keys WHERE key = 'shared-key'")
            .fetch_one(&mut *tx)
            .await
            .expect("org A row");
    assert_eq!(hash, "hash-a", "the visible row is org A's, not org B's");
    let trail_marker: String = sqlx::query_scalar(
        "SELECT after->>'marker' FROM entity_writes WHERE entity = 'ticket' AND entity_id = $1",
    )
    .bind(entity_id)
    .fetch_one(&mut *tx)
    .await
    .expect("trail row");
    assert_eq!(trail_marker, "a");

    // Scoped to org B: the mirror image.
    let mut tx = org_tx(&db.app, org_b).await;
    let hash: String =
        sqlx::query_scalar("SELECT request_hash FROM idempotency_keys WHERE key = 'shared-key'")
            .fetch_one(&mut *tx)
            .await
            .expect("org B row");
    assert_eq!(hash, "hash-b");
    let counters: Vec<i64> = sqlx::query_scalar("SELECT seq FROM entity_write_seqs")
        .fetch_all(&mut *tx)
        .await
        .expect("counter visible to org B");
    assert_eq!(counters, vec![1]);

    // Unscoped (no app.org_id binding): default-deny on all three tables.
    let fresh = db.fresh_app_pool().await;
    let mut tx = fresh.begin().await.expect("begin unscoped");
    for table in ["idempotency_keys", "entity_writes", "entity_write_seqs"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{table} unscoped scan failed: {e}"));
        assert_eq!(count, 0, "{table} leaked rows without an org binding");
    }

    // Uniqueness contracts on the same fixtures: one test so the suite's
    // parallel startup holds fewer pools against Postgres max_connections.
    idempotency_key_is_unique_per_org_and_seq_counter_is_isolated(&db).await;
}

async fn idempotency_key_is_unique_per_org_and_seq_counter_is_isolated(db: &TestDb) {
    let (org_a, org_b) = if_fixtures(db).await;
    let entity_id = if_uuid(0);

    // Same org + same key: the UNIQUE constraint is the replay gate.
    let duplicate = sqlx::query(
        "INSERT INTO idempotency_keys (id, org_id, key, entity, entity_id, request_hash, response, actor_id) \
         VALUES ($1, $2, 'shared-key', 'ticket', $3, 'x', NULL, $3)",
    )
    .bind(if_uuid(3))
    .bind(org_a)
    .bind(entity_id)
    .execute(&db.admin)
    .await;
    assert!(
        duplicate.is_err(),
        "duplicate (org, key) must violate UNIQUE"
    );

    // Different org, same key: the mirror-image fixture rows prove
    // idempotency is org-scoped, never global — both orgs hold the key.
    let keyed_rows: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE org_id = $1), COUNT(*) FILTER (WHERE org_id = $2) \
         FROM idempotency_keys WHERE key = 'shared-key'",
    )
    .bind(org_a)
    .bind(org_b)
    .fetch_one(&db.admin)
    .await
    .expect("keyed rows");
    assert_eq!(
        keyed_rows,
        (1, 1),
        "the same key must claim its own row per org"
    );

    // The (org, entity, entity_id, seq) UNIQUE backing the monotonic
    // contract: a second write at the same seq in one org is rejected.
    let duplicate_seq = sqlx::query(
        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, before, after) \
         VALUES ($1, $2, 'ticket', $3, 1, 'update', $3, NULL, '{}')",
    )
    .bind(if_uuid(5))
    .bind(org_a)
    .bind(entity_id)
    .execute(&db.admin)
    .await;
    assert!(duplicate_seq.is_err(), "seq must be unique per entity");
}

// ===========================================================================
// Undo read model (the private build tracker IF7; migration 0041) — the per-field /
// per-record views over the 0028 write trail: view math against
// hand-computed expectations plus RLS-through-views isolation.
//
// Seed-slot discipline (lane contract): org anchors 260 and 261 (unused
// fixed slots); child ids derive from slot 280's u128, clear of every
// other lane's inserts.
// ===========================================================================

fn undo_uuid(k: u64) -> Uuid {
    Uuid::from_u128(fixed_uuid(280).as_u128() + k as u128)
}

static UNDO_FIXTURES: tokio::sync::OnceCell<(Uuid, Uuid)> = tokio::sync::OnceCell::const_new();

/// Two orgs holding IDENTICAL three-write trails for the SAME
/// (entity, entity_id) pair — only RLS through the views can tell them
/// apart. Writes: create (T1/p3) → update (title T1→T2) → update
/// (priority p3→p1).
async fn undo_fixtures(db: &TestDb) -> (Uuid, Uuid) {
    *UNDO_FIXTURES
        .get_or_init(|| async {
            let org_a = seed_org(&db.app, 260).await.org;
            let org_b = seed_org(&db.app, 261).await.org;
            let entity_id = undo_uuid(0);
            for (org, marker, id_base) in [(org_a, "a", 1u64), (org_b, "b", 11u64)] {
                let marker_json = format!("\"{marker}\"");
                let rows: [(i64, &str, Option<String>, String); 3] = [
                    (
                        1,
                        "create",
                        None,
                        format!(
                            "{{\"title\": \"T1\", \"priority\": \"p3\", \"marker\": {marker_json}}}"
                        ),
                    ),
                    (
                        2,
                        "update",
                        Some(format!(
                            "{{\"title\": \"T1\", \"priority\": \"p3\", \"marker\": {marker_json}}}"
                        )),
                        format!(
                            "{{\"title\": \"T2\", \"priority\": \"p3\", \"marker\": {marker_json}}}"
                        ),
                    ),
                    (
                        3,
                        "update",
                        Some(format!(
                            "{{\"title\": \"T2\", \"priority\": \"p3\", \"marker\": {marker_json}}}"
                        )),
                        format!(
                            "{{\"title\": \"T2\", \"priority\": \"p1\", \"marker\": {marker_json}}}"
                        ),
                    ),
                ];
                for (seq, op, before, after) in rows {
                    sqlx::query(
                        "INSERT INTO entity_writes (id, org_id, entity, entity_id, seq, op, actor_id, idempotency_key, before, after) \
                         VALUES ($1, $2, 'ticket', $3, $4, $5, $6, $7, $8::jsonb, $9::jsonb)",
                    )
                    .bind(undo_uuid(id_base + seq as u64))
                    .bind(org)
                    .bind(entity_id)
                    .bind(seq)
                    .bind(op)
                    .bind(undo_uuid(0))
                    .bind(format!("key-{marker}-{seq}"))
                    .bind(before)
                    .bind(after)
                    .execute(&db.admin)
                    .await
                    .expect("seed undo trail row");
                }
            }
            (org_a, org_b)
        })
        .await
}

#[tokio::test]
async fn undo_views_expose_record_and_field_history_with_rls_isolation() {
    let db = test_db().await;
    let (org_a, org_b) = undo_fixtures(&db).await;
    let entity_id = undo_uuid(0);

    // Org A: newest-first record stack with honest restorable flags.
    {
        let mut tx = org_tx(&db.app, org_a).await;
        let steps = core_db::undo::record_stack(&mut *tx, org_a, "ticket", entity_id, 50)
            .await
            .expect("org A record stack");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].seq, 3, "newest write first");
        assert_eq!(steps[0].op, "update");
        assert_eq!(steps[0].after["priority"], "p1");
        assert_eq!(steps[0].before.as_ref().unwrap()["priority"], "p3");
        assert!(steps[0].restorable);
        assert_eq!(steps[1].seq, 2);
        assert_eq!(steps[1].after["title"], "T2");
        assert_eq!(steps[1].idempotency_key.as_deref(), Some("key-a-2"));
        assert_eq!(steps[2].seq, 1);
        assert_eq!(steps[2].op, "create");
        assert!(steps[2].before.is_none());
        assert!(!steps[2].restorable, "creates are not value-restorable");

        // Field view: one row per (write, field), prior/new side by side —
        // riding the writes that survived the steps limit.
        let step_seqs: Vec<i64> = steps.iter().map(|s| s.seq).collect();
        let fields = core_db::undo::field_stack(&mut *tx, org_a, "ticket", entity_id, &step_seqs)
            .await
            .expect("org A field stack");
        let title_seq2 = fields
            .iter()
            .find(|f| f.seq == 2 && f.field == "title")
            .expect("seq 2 title row");
        assert_eq!(title_seq2.prior.as_ref().unwrap(), "T1");
        assert_eq!(title_seq2.new_value.as_ref().unwrap(), "T2");
        assert!(title_seq2.changed);
        let priority_seq2 = fields
            .iter()
            .find(|f| f.seq == 2 && f.field == "priority")
            .expect("seq 2 priority row");
        assert!(!priority_seq2.changed, "seq 2 did not move priority");
        let priority_seq3 = fields
            .iter()
            .find(|f| f.seq == 3 && f.field == "priority")
            .expect("seq 3 priority row");
        assert_eq!(priority_seq3.prior.as_ref().unwrap(), "p3");
        assert_eq!(priority_seq3.new_value.as_ref().unwrap(), "p1");
        assert!(priority_seq3.changed);
        let title_create = fields
            .iter()
            .find(|f| f.seq == 1 && f.field == "title")
            .expect("create title row");
        assert!(title_create.prior.is_none(), "creates have no prior value");
        assert_eq!(title_create.new_value.as_ref().unwrap(), "T1");
        assert!(title_create.changed);

        // Single-step lookup (the revert target read).
        let step = core_db::undo::trail_step(&mut *tx, org_a, "ticket", entity_id, 2)
            .await
            .expect("trail step query")
            .expect("seq 2 exists");
        assert_eq!(step.op, "update");
        assert_eq!(step.before.as_ref().unwrap()["title"], "T1");
        assert!(
            core_db::undo::trail_step(&mut *tx, org_a, "ticket", entity_id, 99)
                .await
                .expect("missing seq query")
                .is_none(),
            "unknown seq is a clean miss"
        );
    }

    // Org B: the mirror image — same ids, its own rows, nothing of A's.
    {
        let mut tx = org_tx(&db.app, org_b).await;
        let steps = core_db::undo::record_stack(&mut *tx, org_b, "ticket", entity_id, 50)
            .await
            .expect("org B record stack");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].idempotency_key.as_deref(), Some("key-b-2"));
        let marker: String = sqlx::query_scalar(
            "SELECT after->>'marker' FROM ops.v1_record_undo_stack \
             WHERE org_id = $1 AND entity = 'ticket' AND entity_id = $2 AND seq = 1",
        )
        .bind(org_b)
        .bind(entity_id)
        .fetch_one(&mut *tx)
        .await
        .expect("org B trail row through the view");
        assert_eq!(marker, "b", "the visible rows are org B's own");
    }

    // Unscoped (no app.org_id binding): default-deny through both views.
    {
        let fresh = db.fresh_app_pool().await;
        let mut tx = fresh.begin().await.expect("begin unscoped");
        let record_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ops.v1_record_undo_stack")
            .fetch_one(&mut *tx)
            .await
            .expect("unscoped record scan");
        let field_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ops.v1_field_undo_stack")
            .fetch_one(&mut *tx)
            .await
            .expect("unscoped field scan");
        assert_eq!(
            record_rows, 0,
            "record view leaks rows without an org binding"
        );
        assert_eq!(
            field_rows, 0,
            "field view leaks rows without an org binding"
        );
    }

    // Cross-org reads stay empty even when the target id collides.
    {
        let mut tx = org_tx(&db.app, org_a).await;
        let foreign = core_db::undo::trail_step(&mut *tx, org_b, "ticket", entity_id, 2)
            .await
            .expect("cross-org step query");
        assert!(foreign.is_none(), "org A cannot read org B's trail step");
    }
}

#[tokio::test]
async fn undo_views_limit_and_order_delegately() {
    let db = test_db().await;
    let (org_a, _) = undo_fixtures(&db).await;
    let entity_id = undo_uuid(0);
    let mut tx = org_tx(&db.app, org_a).await;

    let limited = core_db::undo::record_stack(&mut *tx, org_a, "ticket", entity_id, 2)
        .await
        .expect("limited stack");
    assert_eq!(
        limited.iter().map(|s| s.seq).collect::<Vec<_>>(),
        vec![3, 2],
        "LIMIT keeps the NEWEST steps, not the oldest"
    );

    // A limit above the row count is a plain pass-through; the clamp keeps
    // pathological callers bounded.
    let clamped = core_db::undo::record_stack(&mut *tx, org_a, "ticket", entity_id, 100_000)
        .await
        .expect("clamped stack");
    assert_eq!(clamped.len(), 3);
}

// --- vector search leg (§4.5; migration 0062) ---------------------------------

fn embedding(seed: f32) -> Vec<f32> {
    vec![seed; core_domain::SEARCH_EMBEDDING_DIM]
}

#[tokio::test]
async fn search_vector_migration_shape_extension_table_and_rls() {
    let db = test_db().await;

    // The pgvector extension is installed (dev/prod run pgvector/pgvector:pg18;
    // migration 0062 enables it explicitly for stock-PG-with-extension too).
    let ext: Option<String> =
        sqlx::query_scalar("SELECT extname FROM pg_extension WHERE extname = 'vector'")
            .fetch_one(&db.admin)
            .await
            .expect("pg_extension probe");
    assert_eq!(ext.as_deref(), Some("vector"), "pgvector must be installed");

    // Column shape: fixed-width vector(768), matching SEARCH_EMBEDDING_DIM.
    let typ: String = sqlx::query_scalar(
        "SELECT format_type(atttypid, atttypmod) FROM pg_attribute \
         WHERE attrelid = 'search_embeddings'::regclass AND attname = 'embedding'",
    )
    .fetch_one(&db.admin)
    .await
    .expect("embedding column");
    assert_eq!(typ, "vector(768)");

    // RLS is ON with exactly the org policy, like its sibling tables.
    let rls: bool = sqlx::query_scalar(
        "SELECT relrowsecurity FROM pg_class WHERE relname = 'search_embeddings'",
    )
    .fetch_one(&db.admin)
    .await
    .expect("relrowsecurity");
    assert!(rls, "search_embeddings must enforce RLS");
    let policy: String = sqlx::query_scalar(
        "SELECT polname FROM pg_policy WHERE polrelid = 'search_embeddings'::regclass",
    )
    .fetch_one(&db.admin)
    .await
    .expect("policy");
    assert_eq!(policy, "search_embeddings_org");
}

#[tokio::test]
async fn search_vector_rls_isolates_orgs_and_default_denies() {
    let db = test_db().await;
    let a = seed_org(&db.app, 600).await;
    let b = seed_org(&db.app, 601).await;
    let (org_a, org_b) = (a.org, b.org);
    let (ticket_a, ticket_b) = (fixed_uuid(602), fixed_uuid(603));

    // Seed two orgs' index rows + vectors as the owner role.
    for (org, entity, seed) in [(org_a, ticket_a, 0.1_f32), (org_b, ticket_b, 0.2_f32)] {
        let mut tx = org_tx(&db.app, org).await;
        idx_upsert(
            &mut tx,
            org,
            search::SearchKind::Ticket,
            entity,
            None,
            "Unit",
            "test",
        )
        .await;
        search::upsert_embedding(
            &mut *tx,
            org,
            search::SearchKind::Ticket,
            entity,
            "test-model",
            "test-model",
            &embedding(seed),
        )
        .await
        .expect("vector upsert");
        tx.commit().await.expect("commit");
    }

    // Org-scoped app tx sees exactly its own vectors.
    let mut tx = org_tx(&db.app, org_a).await;
    let seen: Vec<Uuid> =
        sqlx::query_scalar("SELECT entity_id FROM search_embeddings ORDER BY entity_id")
            .fetch_all(&mut *tx)
            .await
            .expect("org-scoped read");
    assert_eq!(seen, vec![ticket_a], "org A must not see org B's vectors");

    // The no-scope state default-denies (true empty, not an error).
    let unscoped = db.fresh_app_pool().await;
    let none: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_embeddings")
        .fetch_one(&unscoped)
        .await
        .expect("unscoped default-deny count");
    assert_eq!(none, 0, "no app.org_id ⇒ default deny, zero rows");
}

#[tokio::test]
async fn search_embedding_upsert_is_idempotent_and_backfill_excludes_current() {
    let db = test_db().await;
    let s = seed_org(&db.app, 602).await;
    let entity = fixed_uuid(604);
    let mut tx = org_tx(&db.app, s.org).await;

    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        None,
        "VPN drops",
        "tunnel flaps hourly",
    )
    .await;

    // Backfill sweep 1: the row needs an embedding.
    let model = "bge-base-test";
    let targets = search::rows_needing_embedding(&mut *tx, s.org, model, 10)
        .await
        .expect("sweep 1");
    assert_eq!(targets.len(), 1, "unembedded row is a sweep target");
    assert_eq!(targets[0].title, "VPN drops");

    // Embed it; the sweep is empty afterwards.
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        model,
        model,
        &embedding(0.5),
    )
    .await
    .expect("embed");
    let targets = search::rows_needing_embedding(&mut *tx, s.org, model, 10)
        .await
        .expect("sweep 2");
    assert!(
        targets.is_empty(),
        "embedded row with matching hash is excluded"
    );

    // Replaying the same vector changes nothing (exactly-once).
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        model,
        model,
        &embedding(0.5),
    )
    .await
    .expect("replay");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM search_embeddings WHERE org_id = $1")
        .bind(s.org)
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(
        count, 1,
        "vector upserts are exactly-once per (org,kind,entity)"
    );

    // Content moved on ⇒ the hash predicate re-targets the row.
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        None,
        "VPN drops",
        "tunnel flaps twice hourly",
    )
    .await;
    let targets = search::rows_needing_embedding(&mut *tx, s.org, model, 10)
        .await
        .expect("sweep 3");
    assert_eq!(targets.len(), 1, "stale content hash re-embeds");

    // A different model also re-targets (provenance change).
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        entity,
        "other-model",
        "other-model",
        &embedding(0.6),
    )
    .await
    .expect("other model");
    let targets = search::rows_needing_embedding(&mut *tx, s.org, model, 10)
        .await
        .expect("sweep 4");
    assert_eq!(targets.len(), 1, "model change re-embeds");

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_hybrid_fuses_fts_and_vector_legs_by_rrf() {
    let db = test_db().await;
    let s = seed_org(&db.app, 605).await;
    // `both`   matches the FTS query AND is the vector query's nearest hit.
    // `lexical` only matches FTS. `semantic` only the vector leg.
    let (both, lexical, semantic) = (fixed_uuid(605), fixed_uuid(606), fixed_uuid(607));
    let mut tx = org_tx(&db.app, s.org).await;

    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        both,
        None,
        "Mail relay outage",
        "smtp failing",
    )
    .await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        lexical,
        None,
        "Mail relay down",
        "smtp bouncing",
    )
    .await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        semantic,
        None,
        "Unrelated ticket",
        "network switch config",
    )
    .await;

    // Vectors: unit-ish basis vectors; the query sits nearest to `both`
    // (distance 0), then `semantic` (orthogonal), while `lexical` sits far.
    let mut near = embedding(0.0);
    near[0] = 1.0;
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        both,
        "bge-base-test",
        "bge-base-test",
        &near,
    )
    .await
    .expect("embed both");
    let mut orthogonal = embedding(0.0);
    orthogonal[1] = 1.0;
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        semantic,
        "bge-base-test",
        "bge-base-test",
        &orthogonal,
    )
    .await
    .expect("embed");
    let mut far = embedding(0.0);
    far[767] = 1.0;
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        lexical,
        "bge-base-test",
        "bge-base-test",
        &far,
    )
    .await
    .expect("embed");

    let hits = search::search_hybrid_page(
        &mut tx,
        s.org,
        &["ticket".to_string()],
        "relay",
        false,
        None,
        &["bge-base-test".to_string()],
        &near,
        10,
        0,
    )
    .await
    .expect("hybrid page");

    let ids: Vec<Uuid> = hits.iter().map(|h| h.entity_id).collect();
    assert_eq!(
        ids.first(),
        Some(&both),
        "double-leg hit outranks both single legs"
    );
    assert!(
        ids.contains(&lexical) && ids.contains(&semantic),
        "union of both legs, not intersection"
    );

    // Legacy path is untouched: same query, no vector ⇒ FTS-only results.
    let legacy = search::search_page(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "relay",
        false,
        None,
        10,
        0,
    )
    .await
    .expect("legacy page");
    let legacy_ids: Vec<Uuid> = legacy.iter().map(|h| h.entity_id).collect();
    assert_eq!(legacy_ids.len(), 2, "only the two lexical matches");
    assert!(legacy_ids.contains(&both) && legacy_ids.contains(&lexical));
    assert!(
        !legacy_ids.contains(&semantic),
        "the vector-only hit stays out of the FTS path"
    );

    // The union count powers hybrid pagination metadata.
    let total = search::search_hybrid_count(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "relay",
        false,
        None,
        &["bge-base-test".to_string()],
        search::SEARCH_COUNT_CAP,
    )
    .await
    .expect("hybrid count");
    assert_eq!(
        total.total, 3,
        "lexical matches ∪ embedded rows of the kind"
    );
    assert!(!total.capped);
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn search_hybrid_vector_leg_respects_kind_allow_list() {
    let db = test_db().await;
    let s = seed_org(&db.app, 608).await;
    let (ticket, client) = (fixed_uuid(609), fixed_uuid(610));
    let mut tx = org_tx(&db.app, s.org).await;

    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Ticket,
        ticket,
        None,
        "Firewall upgrade",
        "tonight",
    )
    .await;
    idx_upsert(
        &mut tx,
        s.org,
        search::SearchKind::Client,
        client,
        None,
        "Firewall client",
        "",
    )
    .await;
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Ticket,
        ticket,
        "bge-base-test",
        "bge-base-test",
        &embedding(0.3),
    )
    .await
    .expect("embed");
    search::upsert_embedding(
        &mut *tx,
        s.org,
        search::SearchKind::Client,
        client,
        "bge-base-test",
        "bge-base-test",
        &embedding(0.3),
    )
    .await
    .expect("embed");

    // A caller permitted to see tickets only gets ticket vectors only —
    // the kind allow-list rides the vector leg exactly like the FTS leg.
    let hits = search::search_hybrid_page(
        &mut tx,
        s.org,
        &["ticket".to_string()],
        "unmatched-token",
        false,
        None,
        &["bge-base-test".to_string()],
        &embedding(0.3),
        10,
        0,
    )
    .await
    .expect("hybrid page");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, "ticket");

    let total = search::search_hybrid_count(
        &mut *tx,
        s.org,
        &["ticket".to_string()],
        "unmatched-token",
        false,
        None,
        &["bge-base-test".to_string()],
        search::SEARCH_COUNT_CAP,
    )
    .await
    .expect("count");
    assert_eq!(
        total.total, 1,
        "vector candidates outside the allow-list never count"
    );
    tx.commit().await.expect("commit");
}

/// A fused walk must serve the same rows as the fused offset walk, and must
/// report `has_more: false` at the end of the candidate window instead of
/// handing out a cursor that comes back empty. The window is why this works at
/// all: every page fuses the same 50-row candidate set, so the ranks behind
/// the scores do not move between pages.
#[tokio::test]
async fn search_hybrid_walk_matches_the_offset_walk_and_ends_honestly() {
    let db = test_db().await;
    let s = seed_org(&db.app, 136).await;
    let mut tx = org_tx(&db.app, s.org).await;
    for i in 0..7_u64 {
        idx_upsert(
            &mut tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(180 + i),
            None,
            "relay probe",
            "network relay body",
        )
        .await;
        let mut v = embedding(0.0);
        v[usize::try_from(i).unwrap()] = 1.0;
        search::upsert_embedding(
            &mut *tx,
            s.org,
            search::SearchKind::Ticket,
            fixed_uuid(180 + i),
            "bge-base-test",
            "bge-base-test",
            &v,
        )
        .await
        .expect("embed");
    }
    let kinds = ["ticket".to_string()];
    let mut query = embedding(0.0);
    query[0] = 1.0;

    let mut offset_ids = Vec::new();
    for page in 0..3 {
        let hits = search::search_hybrid_page(
            &mut tx,
            s.org,
            &kinds,
            "relay",
            false,
            None,
            &["bge-base-test".to_string()],
            &query,
            3,
            page * 3,
        )
        .await
        .expect("hybrid offset page");
        offset_ids.extend(hits.into_iter().map(|h| h.entity_id));
    }

    let mut cursor_ids = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = search::search_hybrid_page_after(
            &mut tx,
            s.org,
            &kinds,
            "relay",
            false,
            None,
            &["bge-base-test".to_string()],
            &query,
            3,
            cursor,
        )
        .await
        .expect("fused page");
        cursor = page.hits.last().map(|h| search::FusedCursor {
            score: h.score,
            entity_kind: h.entity_kind.clone(),
            entity_id: h.entity_id,
        });
        let more = page.has_more;
        cursor_ids.extend(page.hits.into_iter().map(|h| h.entity_id));
        pages += 1;
        if !more {
            break;
        }
        assert!(pages < 10, "fused walk did not terminate");
    }

    assert_eq!(
        cursor_ids, offset_ids,
        "the fused walk serves the same rows in the same order"
    );
    assert_eq!(cursor_ids.len(), 7);
    let unique: std::collections::HashSet<Uuid> = cursor_ids.iter().copied().collect();
    assert_eq!(unique.len(), 7, "no row is served twice");
    tx.commit().await.expect("commit");
}
// --- sign-offs (Wave 43 track 2 lane 12, migration 0127) --------------------
//
// The evidence row is insert-only, so these tests prove that shape rather
// than an update path: a capture round-trips, a replay of the same
// idempotency key is a typed conflict, a correction is a second row that
// names the original rather than editing it, revocation appends a seq and
// never touches the original, and a consent revision leaves the snapshot on
// an existing sign-off alone.

/// The wording `seed_default_sign_off_consent` inserts as version 1. Referenced
/// rather than re-typed so a copy change reads as one edit.
const SIGN_OFF_CONSENT: &str = orgs::DEFAULT_SIGN_OFF_CONSENT;

async fn sign_off_attachment(
    conn: &mut PgConnection,
    org: Uuid,
    id: Uuid,
    ticket: Uuid,
    sha_hex: &str,
    uploader: Option<Uuid>,
) -> Attachment {
    let key = format!("org/{org}/attachments/{sha_hex}");
    core_db::attachments::insert(
        conn,
        core_db::attachments::NewAttachment {
            id,
            org_id: org,
            kind: AttachmentKind::SignOff,
            ticket_id: Some(ticket),
            ticket_comment_id: None,
            email_message_id: None,
            filename: "signature.png",
            content_type: "image/png",
            size_bytes: 1_024,
            sha256: sha_hex,
            storage_key: &key,
            uploaded_by_user_id: uploader,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("sign-off payload insert")
}

/// The sha256 the PNG path hex-encodes: 32 bytes of `ab`.
fn sha256_ab() -> String {
    "ab".repeat(32)
}

#[tokio::test]
async fn sign_off_capture_round_trips_with_its_consent_snapshot() {
    let db = test_db().await;
    let s = seed_org(&db.app, 610).await;
    let witness = seed_user(&db.app, &s, 61000).await;
    let ticket_id = fixed_uuid(61001);
    let attachment_id = fixed_uuid(61002);
    let sign_off_id = fixed_uuid(61003);

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Boiler will not light",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;

    let written = sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: sign_off_id,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(attachment_id),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-1",
        },
    )
    .await
    .expect("sign-off insert");

    assert_eq!(written.signer_name, "Dana Whitfield");
    assert_eq!(written.consent_version, 1);
    assert_eq!(written.consent_text_snapshot, SIGN_OFF_CONSENT);
    assert!(!written.typed_agreement);
    assert_eq!(written.attachment_id, Some(attachment_id));

    let read_back = sign_offs::get_sign_off(&mut tx, s.org, sign_off_id)
        .await
        .expect("sign-off read");
    assert_eq!(read_back, written);

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_replay_of_the_same_idempotency_key_is_a_typed_conflict() {
    let db = test_db().await;
    let s = seed_org(&db.app, 611).await;
    let witness = seed_user(&db.app, &s, 61100).await;
    let ticket_id = fixed_uuid(61101);
    let attachment_id = fixed_uuid(61102);

    // The first capture COMMITS before the replay is attempted. Running the
    // replay against a still-open transaction holding the same unique key
    // makes Postgres block rather than refuse, so the shapes this test cares
    // about would be unobservable behind a lock.
    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Fuse keeps blowing",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;

    let args = |id: Uuid| sign_offs::NewSignOff {
        id,
        org_id: s.org,
        ticket_id: ticket.id,
        attachment_id: Some(attachment_id),
        typed_agreement: false,
        signer_name: "Dana Whitfield",
        signer_kind: SignOffSignerKind::Customer,
        witnessed_by_user_id: Some(witness),
        captured_at: at(9),
        consent_version: 1,
        consent_text_snapshot: SIGN_OFF_CONSENT,
        supersedes_sign_off_id: None,
        idempotency_key: "capture-replay",
    };
    let original = sign_offs::insert_sign_off(&mut tx, args(fixed_uuid(61103)))
        .await
        .expect("first capture");
    tx.commit().await.expect("commit");

    // The offline queue replays the identical op with a fresh row id but the
    // same key. This is the unique violation the API maps to the typed
    // `sign_off_already_captured` conflict.
    let mut replaying = org_tx(&db.app, s.org).await;
    let replay = sign_offs::insert_sign_off(&mut replaying, args(fixed_uuid(61104))).await;
    drop(replaying);
    assert!(
        is_unique_violation(replay.as_ref().expect_err("replay must conflict")),
        "a replayed capture must land on its idempotency key, not a second row"
    );

    // Exactly one row carries that key, and it is the original.
    let mut tx = org_tx(&db.app, s.org).await;
    let live: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sign_offs WHERE idempotency_key = 'capture-replay'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count by key");
    assert_eq!(
        live, 1,
        "the original row survives a replay rather than being duplicated"
    );
    let read_back = sign_offs::get_sign_off(&mut tx, s.org, original.id)
        .await
        .expect("read");
    assert_eq!(read_back, original);
    assert_eq!(read_back.id, fixed_uuid(61103));
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_correction_is_a_second_row_that_names_the_original() {
    let db = test_db().await;
    let s = seed_org(&db.app, 612).await;
    let witness = seed_user(&db.app, &s, 61200).await;
    let ticket_id = fixed_uuid(61201);
    let first_attachment = fixed_uuid(61202);
    let second_attachment = fixed_uuid(61203);
    let first_sign_off = fixed_uuid(61204);
    let correction = fixed_uuid(61205);

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Thermostat stuck",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        first_attachment,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        second_attachment,
        ticket.id,
        &"cd".repeat(32),
        Some(witness),
    )
    .await;

    let original = sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: first_sign_off,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(first_attachment),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-first",
        },
    )
    .await
    .expect("original capture");
    assert_eq!(original.supersedes_sign_off_id, None);

    let replacing = sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: correction,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(second_attachment),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(10),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: Some(first_sign_off),
            idempotency_key: "capture-correction",
        },
    )
    .await
    .expect("correction insert");
    assert_eq!(replacing.supersedes_sign_off_id, Some(first_sign_off));

    // The original is not written: the relation points backward, and reading
    // it back proves its own column is still empty.
    let re_read = sign_offs::get_sign_off(&mut tx, s.org, first_sign_off)
        .await
        .expect("original read");
    assert_eq!(
        re_read.supersedes_sign_off_id, None,
        "an original sign-off is never written to record what replaced it"
    );

    // The view derives `superseded_by` for whoever asks.
    let views = sign_offs::list_sign_off_views_for_ticket(&mut tx, s.org, ticket.id)
        .await
        .expect("views");
    let original_view = views
        .iter()
        .find(|view| view.sign_off.id == first_sign_off)
        .expect("original view");
    assert_eq!(original_view.superseded_by, Some(correction));
    assert!(original_view.revocation.is_none());

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_revocation_appends_seq_and_leaves_the_row_alone() {
    let db = test_db().await;
    let s = seed_org(&db.app, 613).await;
    let witness = seed_user(&db.app, &s, 61300).await;
    let revoker = seed_user(&db.app, &s, 61301).await;
    let ticket_id = fixed_uuid(14301 + 1_000);
    let attachment_id = fixed_uuid(61302);
    let sign_off_id = fixed_uuid(61303);

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Water leak under the sink",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;
    let before = sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: sign_off_id,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(attachment_id),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-revoked",
        },
    )
    .await
    .expect("capture");

    let revocation = sign_offs::insert_revocation(
        &mut tx,
        Uuid::now_v7(),
        s.org,
        sign_off_id,
        "Customer disputed the work",
        revoker,
    )
    .await
    .expect("revocation");
    assert_eq!(revocation.seq, 1);
    assert_eq!(revocation.revoked_by_user_id, Some(revoker));

    // The row itself is untouched, and the view reports the revocation.
    let after = sign_offs::get_sign_off(&mut tx, s.org, sign_off_id)
        .await
        .expect("read");
    assert_eq!(
        after, before,
        "revoking appends to the trail and writes nothing on the sign-off"
    );
    let views = sign_offs::list_sign_off_views_for_ticket(&mut tx, s.org, ticket.id)
        .await
        .expect("views");
    let view = views.first().expect("view");
    assert_eq!(view.revocation.as_ref().map(|r| r.seq), Some(1));
    assert_eq!(
        view.attachment_sha256.as_deref(),
        Some(sha256_ab().as_str())
    );

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_payload_is_downloadable_only_for_a_drawn_mark() {
    let db = test_db().await;
    let s = seed_org(&db.app, 614).await;
    let witness = seed_user(&db.app, &s, 61400).await;
    let ticket_id = fixed_uuid(61401);
    let attachment_id = fixed_uuid(61402);
    let drawn = fixed_uuid(61403);
    let typed = fixed_uuid(61404);

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "No hot water",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;

    // The drawn path.
    sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: drawn,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(attachment_id),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-drawn",
        },
    )
    .await
    .expect("drawn capture");

    // The typed path: same row shape, no payload.
    sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: typed,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: None,
            typed_agreement: true,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-typed",
        },
    )
    .await
    .expect("typed capture");

    let payload = sign_offs::payload_for_sign_off(&mut tx, s.org, drawn)
        .await
        .expect("drawn payload");
    assert_eq!(payload.sha256, sha256_ab());
    assert_eq!(payload.filename, "signature.png");

    // A typed agreement has no object, so the inner JOIN yields no row and
    // the route answers 404 rather than inventing bytes.
    let typed_payload = sign_offs::payload_for_sign_off(&mut tx, s.org, typed).await;
    assert!(
        matches!(typed_payload, Err(DbError::NotFound)),
        "expected NotFound, got {typed_payload:?}"
    );

    // The view carries the payload facts only for the drawn one.
    let views = sign_offs::list_sign_off_views_for_ticket(&mut tx, s.org, ticket.id)
        .await
        .expect("views");
    let typed_view = views
        .iter()
        .find(|view| view.sign_off.id == typed)
        .expect("typed view");
    assert!(typed_view.sign_off.typed_agreement);
    assert_eq!(typed_view.attachment_sha256, None);
    assert_eq!(typed_view.attachment_filename, None);

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_consent_revision_leaves_an_existing_snapshot_alone() {
    let db = test_db().await;
    let s = seed_org(&db.app, 615).await;
    let admin = seed_user(&db.app, &s, 61500).await;
    let witness = seed_user(&db.app, &s, 61501).await;
    let ticket_id = fixed_uuid(61501);
    let attachment_id = fixed_uuid(61502);
    let sign_off_id = fixed_uuid(61503);

    let mut tx = org_tx(&db.app, s.org).await;
    let seeded = sign_offs::current_consent(&mut tx, s.org)
        .await
        .expect("seeded consent");
    assert_eq!(seeded.version, 1);

    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Radiator cold upstairs",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;
    sign_offs::insert_sign_off(
        &mut tx,
        sign_offs::NewSignOff {
            id: sign_off_id,
            org_id: s.org,
            ticket_id: ticket.id,
            attachment_id: Some(attachment_id),
            typed_agreement: false,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: seeded.version,
            consent_text_snapshot: &seeded.body,
            supersedes_sign_off_id: None,
            idempotency_key: "capture-v1",
        },
    )
    .await
    .expect("capture under v1");

    // The org revises its wording.
    let revised = "I confirm the work described above was completed to my satisfaction and that I authorised anyone signing for me.";
    let revision = sign_offs::insert_consent(
        &mut tx,
        Uuid::now_v7(),
        s.org,
        seeded.version + 1,
        revised,
        admin,
    )
    .await
    .expect("consent revision");
    assert_eq!(revision.version, 2);

    // The sign-off still says what this signer actually read.
    let after = sign_offs::get_sign_off(&mut tx, s.org, sign_off_id)
        .await
        .expect("read");
    assert_eq!(after.consent_text_snapshot, seeded.body);
    assert_eq!(after.consent_version, 1);

    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn sign_off_conflict_reports_its_constraint_for_a_typed_conflict() {
    let db = test_db().await;
    let s = seed_org(&db.app, 616).await;
    let witness = seed_user(&db.app, &s, 61600).await;
    let ticket_id = fixed_uuid(61601);
    let attachment_id = fixed_uuid(61603);

    let mut tx = org_tx(&db.app, s.org).await;
    let ticket = create_ticket(
        &mut tx,
        &s,
        ticket_id,
        "Extractor fan noisy",
        Priority::P3,
        s.entry_status,
        None,
    )
    .await;
    sign_off_attachment(
        &mut tx,
        s.org,
        attachment_id,
        ticket.id,
        &sha256_ab(),
        Some(witness),
    )
    .await;
    tx.commit().await.expect("commit");

    // The key is a caller-supplied literal rather than a formatted string:
    // `NewSignOff` borrows it, and a borrow of a temporary inside the closure
    // is exactly the mistake this line's shape invites.
    let args = |id: Uuid, attachment_id: Option<Uuid>, typed: bool, key: &'static str| {
        sign_offs::NewSignOff {
            id,
            org_id: s.org,
            ticket_id,
            attachment_id,
            typed_agreement: typed,
            signer_name: "Dana Whitfield",
            signer_kind: SignOffSignerKind::Customer,
            witnessed_by_user_id: Some(witness),
            captured_at: at(9),
            consent_version: 1,
            consent_text_snapshot: SIGN_OFF_CONSENT,
            supersedes_sign_off_id: None,
            idempotency_key: key,
        }
    };

    // Neither path. The capture-shape CHECK has to refuse it, because a row
    // with no mark and no agreement is not evidence of anything.
    let mut neither_tx = org_tx(&db.app, s.org).await;
    let neither = sign_offs::insert_sign_off(
        &mut neither_tx,
        args(fixed_uuid(61604), None, false, "capture-neither"),
    )
    .await;
    drop(neither_tx);
    assert!(
        is_constraint_violation(&neither.expect_err("neither path must be refused")),
        "a capture with neither a mark nor a typed agreement is not evidence"
    );

    // Both at once, refused by the same CHECK.
    let mut both_tx = org_tx(&db.app, s.org).await;
    let both = sign_offs::insert_sign_off(
        &mut both_tx,
        args(fixed_uuid(61605), Some(attachment_id), true, "capture-both"),
    )
    .await;
    drop(both_tx);
    assert!(is_constraint_violation(
        &both.expect_err("both paths must be refused")
    ));

    // The reserved portal kind with a staff witness attached. The API rejects
    // this kind outright today; the signer-shape CHECK is the backstop.
    let mut portal_tx = org_tx(&db.app, s.org).await;
    let portal = sign_offs::insert_sign_off(
        &mut portal_tx,
        sign_offs::NewSignOff {
            signer_kind: SignOffSignerKind::PortalClient,
            ..args(
                fixed_uuid(61606),
                Some(attachment_id),
                false,
                "capture-portal",
            )
        },
    )
    .await;
    drop(portal_tx);
    assert!(is_constraint_violation(
        &portal.expect_err("portal signing with a witness must be refused")
    ));

    // A portal sign-off with NO witness passes the CHECK: that is the shape a
    // future portal lane lands, and the refusal belongs to the API, not here.
    let mut portal_ok = org_tx(&db.app, s.org).await;
    let accepted = sign_offs::insert_sign_off(
        &mut portal_ok,
        sign_offs::NewSignOff {
            witnessed_by_user_id: None,
            signer_kind: SignOffSignerKind::PortalClient,
            ..args(
                fixed_uuid(61607),
                Some(attachment_id),
                false,
                "capture-portal-ok",
            )
        },
    )
    .await
    .expect("the reserved shape is storable; the route refuses it, not the schema");
    drop(portal_ok);
    assert_eq!(accepted.witnessed_by_user_id, None);

    // Nothing landed: all three refused shapes.
    let mut check = org_tx(&db.app, s.org).await;
    let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sign_offs WHERE id IN ($1, $2, $3)")
        .bind(fixed_uuid(61604))
        .bind(fixed_uuid(61605))
        .bind(fixed_uuid(61606))
        .fetch_one(&mut *check)
        .await
        .expect("count");
    assert_eq!(live, 0);
    drop(check);
}

/// Org usage materialization (the private build tracker Wave 47 lane A6, migration 0131).
///
/// The lane's three load-bearing behaviours, in the order a caller meets
/// them: an org with no row reads as ABSENT (not as zeros, which would read
/// as a measured empty org), a refresh is IDEMPOTENT (the org_id primary
/// key makes a replay the same row), and RLS keeps the surface
/// org-private through the app role — org B's transaction cannot read org
/// A's row, and a pool that never bound an org sees nothing at all.
#[tokio::test]
async fn org_usage_row_is_absent_until_refreshed_idempotent_and_rls_isolated() {
    let db = test_db().await;
    // Fixed ids in a band no other lane in this suite touches.
    let a = seed_org(&db.app, 7001).await;
    let b = seed_org(&db.app, 7002).await;
    seed_user(&db.app, &a, 7101).await;
    let disabled = seed_user(&db.app, &a, 7102).await;

    // --- absent before any refresh -----------------------------------------
    let mut tx = org_tx(&db.app, a.org).await;
    assert!(
        core_db::org_usage::get_usage(&mut *tx, a.org)
            .await
            .expect("read")
            .is_none(),
        "an org the sweep has never reached has NO row — zero counts would be \
         indistinguishable from a measured empty org"
    );
    // ...and the sweep's claim says exactly that.
    let due = core_db::org_usage::claim_stale_orgs(&mut *tx, 300, 50)
        .await
        .expect("claim");
    assert!(
        due.contains(&a.org),
        "an org with no usage row is stale by definition"
    );
    drop(tx);

    // --- real rows to count: one disabled account + one ticket per category
    let mut tx = org_tx(&db.app, a.org).await;
    sqlx::query("UPDATE users SET is_active = false WHERE id = $1")
        .bind(disabled)
        .execute(&mut *tx)
        .await
        .expect("deactivate");
    create_ticket(
        &mut tx,
        &a,
        fixed_uuid(7103),
        "open one",
        Priority::P3,
        a.entry_status,
        None,
    )
    .await;
    create_ticket(
        &mut tx,
        &a,
        fixed_uuid(7104),
        "waiting one",
        Priority::P3,
        a.waiting_status,
        None,
    )
    .await;
    create_ticket(
        &mut tx,
        &a,
        fixed_uuid(7105),
        "resolved one",
        Priority::P3,
        a.resolved_status,
        None,
    )
    .await;
    // COMMIT, not drop: dropping an sqlx Transaction rolls back, and this
    // transaction holds the writes the assertions below count (the disabled
    // account and the three tickets).
    tx.commit().await.expect("commit seed rows");

    // --- refresh: exact counts, not estimates ------------------------------
    // The owner pool is the worker's posture: the sweep refreshes across
    // orgs and binds no app.org_id, so the write must not depend on one.
    let row = core_db::org_usage::refresh_orgs(&db.admin, &[a.org])
        .await
        .expect("refresh")
        .into_iter()
        .next()
        .expect("a real org produces a row");
    assert_eq!(row.org_id, a.org);
    assert_eq!(
        row.active_users, 1,
        "the disabled account is not using anything"
    );
    assert_eq!(row.tickets_open, 1);
    assert_eq!(row.tickets_waiting, 1);
    assert_eq!(row.tickets_closed, 1);
    assert_eq!(row.tickets_in_progress, 0);
    assert_eq!(row.tickets_total(), 3);
    assert_eq!(row.automation_runs, 0);
    assert_eq!(row.webhook_deliveries, 0);
    assert!(row.refreshed_at <= chrono::Utc::now());

    // --- idempotency: the same row, updated ---------------------------------
    let again = core_db::org_usage::refresh_orgs(&db.admin, &[a.org])
        .await
        .expect("second refresh")
        .into_iter()
        .next()
        .expect("row");
    assert_eq!(again.org_id, row.org_id, "the primary key is the identity");
    assert_eq!(again.active_users, row.active_users);
    assert_eq!(again.tickets_total(), row.tickets_total());
    assert_eq!(again.object_store_bytes, row.object_store_bytes);
    assert!(
        again.refreshed_at >= row.refreshed_at,
        "a replay updates the row's timestamp, it never forks a second row"
    );
    let live: i64 = sqlx::query_scalar("SELECT count(*) FROM org_usage WHERE org_id = $1")
        .bind(a.org)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(live, 1, "one org, one row, forever");

    // A second org is claimed and refreshed while the first row stands.
    let fresh = core_db::org_usage::refresh_orgs(&db.admin, &[b.org])
        .await
        .expect("second org refresh")
        .into_iter()
        .next()
        .expect("row");
    assert_eq!(fresh.org_id, b.org);
    assert_eq!(fresh.active_users, 0);

    // --- the claim is staleness-ordered, not org-id-ordered ----------------
    // `a` was refreshed first (so its row is older by construction here only
    // A deterministic age gap is what proves the ordering: age BOTH rows by
    // different amounts, then assert the older one is claimed first.
    // Age BOTH rows, with `a` older than `b`, so both are genuinely stale
    // and the claim's ordering is what decides their sequence. Aging only
    // `a` would leave `b` fresh (refreshed seconds ago, inside the 300s
    // window) and the ordering assertion would be testing nothing — the
    // first version of this test asserted `b` was claimed while the comment
    // beside it said `b` was the most recent refresh, which is why it failed.
    sqlx::query("UPDATE org_usage SET refreshed_at = now() - interval '2 hours' WHERE org_id = $1")
        .bind(a.org)
        .execute(&db.admin)
        .await
        .expect("age a");
    sqlx::query("UPDATE org_usage SET refreshed_at = now() - interval '1 hours' WHERE org_id = $1")
        .bind(b.org)
        .execute(&db.admin)
        .await
        .expect("age b");
    // A third org with NO row at all sorts before both (never measured is
    // the stalest state there is), and it is created by the claim itself.
    let c = seed_org(&db.app, 7005).await;
    // A limit large enough to cover every stale org in the shared database:
    // the suite has no per-test reset, so other tests' never-refreshed orgs
    // are also due, and a production-sized 50 would push THIS test's orgs
    // out of the window entirely (which is exactly what the first version
    // did — `mine` came back empty and the ordering assertion was comparing
    // against nothing).
    let due = core_db::org_usage::claim_stale_orgs(&db.admin, 300, 100_000)
        .await
        .expect("claim with an unmeasured org");
    // Scope the assertions to THIS test's orgs. The suite shares one
    // database with no per-test reset, so another test's never-refreshed
    // org is legitimately in the same claim — asserting on the fleet-wide
    // first entry couples this test to whatever else has run.
    let mine: Vec<Uuid> = due
        .iter()
        .copied()
        .filter(|id| *id == a.org || *id == b.org || *id == c.org)
        .collect();
    assert_eq!(
        mine.first().copied(),
        Some(c.org),
        "an org with no row is the stalest of this test's entries"
    );
    let stale_a = mine
        .iter()
        .position(|id| *id == a.org)
        .expect("a is stale and must be claimed");
    let stale_b = mine
        .iter()
        .position(|id| *id == b.org)
        .expect("b is stale and must be claimed");
    assert!(
        stale_a < stale_b,
        "the older row is claimed before the newer one, whatever the org ids"
    );

    // A row refreshed INSIDE the interval is not due, whichever arm the
    // optimizer picks. This is the property the whole sweep depends on: the
    // two-arm claim (index range scan for stale rows, anti-join for missing
    // ones) must not start returning fresh rows, or a short interval would
    // re-refresh a quiet tenant forever.
    let rows_now = core_db::org_usage::refresh_orgs(&db.admin, &due)
        .await
        .expect("drain the backlog");
    let quiet = core_db::org_usage::claim_stale_orgs(&db.admin, 300, 100_000)
        .await
        .expect("claim after a drain");
    // Scoped to this test's orgs, for the same reason the claim above is: the
    // suite shares one database with no per-test reset and other tests run in
    // parallel, so the fleet-wide claim is legitimately non-empty when this
    // runs. What must hold is that THIS test's just-refreshed orgs are gone
    // from it — asserting globally made this flaky the moment a sibling test
    // committed an org between the two claims.
    let still_due: Vec<Uuid> = quiet
        .iter()
        .copied()
        .filter(|id| *id == a.org || *id == b.org || *id == c.org)
        .collect();
    assert!(
        still_due.is_empty(),
        "an org refreshed seconds ago is not stale — otherwise the sweep would \
         re-do work on a quiet tenant every tick"
    );
    assert_eq!(
        rows_now.len(),
        due.len(),
        "every claimed org got a row out of the batch refresh"
    );

    // --- the batch refresh writes every org in one statement ---------------
    // The sweep's path: one round-trip per batch rather than per org.
    let rows = core_db::org_usage::refresh_orgs(&db.admin, &[a.org, b.org, c.org])
        .await
        .expect("batch refresh");
    assert_eq!(rows.len(), 3, "every real org in the batch gets a row");
    let mut written: Vec<Uuid> = rows.iter().map(|r| r.org_id).collect();
    written.sort();
    let mut expected = vec![a.org, b.org, c.org];
    expected.sort();
    assert_eq!(written, expected);
    // Nonexistent orgs are simply absent, not an error — a batch that spans
    // a deletion writes fewer rows than it was given.
    let surviving = core_db::org_usage::refresh_orgs(&db.admin, &[a.org, fixed_uuid(999_001)])
        .await
        .expect("batch with a deleted org");
    assert_eq!(
        surviving.len(),
        1,
        "a vanished org writes nothing, silently"
    );
    assert_eq!(surviving[0].org_id, a.org);
    // An empty batch is a no-op rather than a round-trip.
    let none = core_db::org_usage::refresh_orgs(&db.admin, &[])
        .await
        .expect("empty batch");
    assert!(none.is_empty(), "an empty batch writes nothing");

    // --- RLS isolation ------------------------------------------------------
    let mut tx_b = org_tx(&db.app, b.org).await;
    let own = core_db::org_usage::get_usage(&mut *tx_b, b.org)
        .await
        .expect("own read");
    assert_eq!(
        own.expect("own row").org_id,
        b.org,
        "an org reads its own materialized row"
    );
    assert!(
        core_db::org_usage::get_usage(&mut *tx_b, a.org)
            .await
            .expect("cross-org read")
            .is_none(),
        "org B must not be able to read org A's usage row"
    );
    drop(tx_b);

    // An app pool that has NEVER bound an org sees nothing at all.
    let unscoped = db.fresh_app_pool().await;
    let none_seen: i64 = sqlx::query_scalar("SELECT count(*) FROM org_usage")
        .fetch_one(&unscoped)
        .await
        .expect("unscoped read");
    assert_eq!(none_seen, 0, "no app.org_id binding ⇒ default deny");
}

/// Configured quota limits are org-scope SETTINGS data — one key per
/// metric — and the family is closed plus typed: a key outside it and a
/// value that is not an integer both configure NOTHING rather than
/// accidentally constraining a metric. The pairing the quota surface
/// renders (metric → usage column → percentage) is checked against the
/// materialized row, so the percentages a dashboard shows are the ones the
/// API computes from the same functions.
#[tokio::test]
async fn configured_quota_limits_are_org_settings_and_pair_with_usage() {
    use core_domain::usage::{percent_of, UsageMetric};

    let db = test_db().await;
    let a = seed_org(&db.app, 7003).await;
    let b = seed_org(&db.app, 7004).await;
    seed_user(&db.app, &a, 7106).await;
    seed_user(&db.app, &a, 7107).await;
    core_db::org_usage::refresh_orgs(&db.admin, &[a.org])
        .await
        .expect("refresh")
        .into_iter()
        .next()
        .expect("row");

    let mut tx = org_tx(&db.app, a.org).await;
    for (key, value) in [
        // Three real limits...
        ("usage_quota.active_users", serde_json::json!(10)),
        ("usage_quota.managed_devices", serde_json::json!(5)),
        ("usage_quota.tickets", serde_json::json!(50)),
        // ...a key outside the closed family (a teapot quota)...
        ("usage_quota.teapots", serde_json::json!(3)),
        // ...and a value that is not an integer (a limit counts a real
        // thing; a word is a provisioning error, not a limit).
        ("usage_quota.webhook_deliveries", serde_json::json!("many")),
    ] {
        core_db::settings::upsert_org_setting(&mut tx, a.org, key, &value, fixed_uuid(7106))
            .await
            .expect("limit setting");
    }
    let limits = core_db::org_usage::list_configured_limits(&mut *tx, a.org)
        .await
        .expect("limits");
    assert_eq!(
        limits,
        vec![
            (UsageMetric::ActiveUsers, 10),
            (UsageMetric::Tickets, 50),
            (UsageMetric::ManagedDevices, 5),
        ],
        "only whole-number limits in the family survive, in metric order"
    );
    assert!(
        core_db::org_usage::list_configured_limits(&mut *tx, b.org)
            .await
            .expect("other org limits")
            .is_empty(),
        "an org that configured nothing has no limits — there is no default"
    );
    drop(tx);

    // The pairing: the materialized row supplies each metric's usage and
    // the pure percentage arithmetic turns the pair into a report.
    let usage = core_db::org_usage::get_usage(&db.admin, a.org)
        .await
        .expect("read")
        .expect("row");
    let limit_for = |metric: UsageMetric| {
        limits
            .iter()
            .find(|(m, _)| *m == metric)
            .map(|(_, limit)| *limit)
    };
    assert_eq!(usage.metric_value(UsageMetric::ActiveUsers), 2);
    assert_eq!(
        percent_of(2, limit_for(UsageMetric::ActiveUsers)),
        Some(20.0)
    );
    assert_eq!(usage.tickets_total(), 0);
    assert_eq!(
        percent_of(usage.tickets_total(), limit_for(UsageMetric::Tickets)),
        Some(0.0)
    );
    // A metric with no configured limit has no percentage, in the API and
    // in the arithmetic that backs it.
    assert_eq!(
        percent_of(0, limit_for(UsageMetric::ObjectStoreBytes)),
        None
    );
}

/// The object-store byte sum's two vault arms (migration 0131, the
/// highest-risk part of the read model): every client-scope record is
/// attributed to its org exactly once, and no org-scope record is counted
/// twice.
///
/// `secret_records` carries `CHECK (num_nonnulls(org_id, client_id) = 1)`
/// (0078), so the view's split into an org-scope arm and a client-scope arm
/// is provably disjoint IF that invariant holds — which is exactly what this
/// pins. A future edit to either arm, a dropped constraint, or a `client_id`
/// outliving its org would otherwise mis-attribute bytes silently, because
/// nothing else in the suite touches `secret_records`.
///
/// Also pins the per-row `GREATEST(file_bytes, 0)` clamp: a malformed
/// negative row must contribute 0 rather than cancelling its siblings' real
/// bytes (a net-sum clamp would report a plausible 0 while real objects are
/// stored, and still pass the `object_store_bytes >= 0` CHECK).
#[tokio::test]
async fn org_usage_object_store_bytes_sum_both_vault_arms_once_each() {
    let db = test_db().await;
    let a = seed_org(&db.app, 7006).await;
    let other = seed_org(&db.app, 7007).await;
    seed_user(&db.app, &a, 7108).await;

    // A client in EACH org, so the client-scope arm has something to
    // attribute in both directions — including the "this client belongs to
    // the OTHER org" case that would leak bytes across the seam.
    //
    // Own-org writes first, in a transaction bound to org A.
    let mut tx = org_tx(&db.app, a.org).await;
    core_db::clients::create_client(&mut *tx, a.org, fixed_uuid(7109), "Owned Client", None)
        .await
        .expect("own client");
    // One folder per scope: the record's FK requires one, and the folder's
    // own scope pair mirrors the record's (equality enforced in the write
    // path, so a cross-scope insert would be rejected rather than silently
    // attributed).
    let own_folder = seed_secret_folder(&mut tx, FolderScope::Org(a.org), 9610, "documents").await;
    let own_client_folder = seed_secret_folder(
        &mut tx,
        FolderScope::Client(fixed_uuid(7109)),
        9611,
        "documents",
    )
    .await;

    // Org-scope record: 100 bytes, attributed by `sr.org_id`.
    seed_secret_document(
        &mut tx,
        fixed_uuid(7111),
        Some(a.org),
        None,
        own_folder,
        Some(100),
    )
    .await;
    // Client-scope record for a client of THIS org: 30 bytes, attributed
    // through the client. This is the arm that must NOT be dropped.
    seed_secret_document(
        &mut tx,
        fixed_uuid(7112),
        None,
        Some(fixed_uuid(7109)),
        own_client_folder,
        Some(30),
    )
    .await;
    // A null-`file_bytes` row (a non-document record with no object): must
    // contribute nothing, and must not turn the sum NULL.
    seed_secret_document(
        &mut tx,
        fixed_uuid(7113),
        Some(a.org),
        None,
        own_folder,
        None,
    )
    .await;
    // A malformed NEGATIVE row: contributes 0, never a cancellation of the
    // sibling's real bytes.
    seed_secret_document(
        &mut tx,
        fixed_uuid(7114),
        Some(a.org),
        None,
        own_folder,
        Some(-500),
    )
    .await;
    // COMMIT, not drop: an sqlx Transaction rolls back when it is dropped,
    // and this one holds the vault writes the assertions below count.
    tx.commit().await.expect("commit vault seed rows");

    // A client-scope record belonging to the OTHER org, so a leak reads as a
    // difference rather than as an unmeasurable number. The foreign client
    // and its folder are created HERE, in a transaction bound to the other
    // org, because RLS refuses a cross-org insert: seeding them from org A's
    // transaction (where this test first put them) fails with 42501.
    let mut tx_other = org_tx(&db.app, other.org).await;
    core_db::clients::create_client(
        &mut *tx_other,
        other.org,
        fixed_uuid(7110),
        "Foreign Client",
        None,
    )
    .await
    .expect("foreign client");
    let foreign_client_folder = seed_secret_folder(
        &mut tx_other,
        FolderScope::Client(fixed_uuid(7110)),
        9612,
        "documents",
    )
    .await;
    seed_secret_folder(
        &mut tx_other,
        FolderScope::Org(other.org),
        9613,
        "documents",
    )
    .await;
    seed_secret_document(
        &mut tx_other,
        fixed_uuid(7115),
        None,
        Some(fixed_uuid(7110)),
        foreign_client_folder,
        Some(7000),
    )
    .await;
    // COMMIT, not drop: an sqlx Transaction rolls back when it is dropped,
    // and this one holds the vault writes the assertions below count.
    tx_other.commit().await.expect("commit vault seed rows");

    let row = core_db::org_usage::refresh_orgs(&db.admin, &[a.org])
        .await
        .expect("refresh")
        .into_iter()
        .next()
        .expect("row");
    // 100 (org arm) + 30 (client arm); the null row adds nothing and the
    // -500 row adds 0 instead of subtracting.
    assert_eq!(
        row.object_store_bytes, 130,
        "both vault arms counted exactly once each, clamped per row"
    );

    // The other org sees only its own client-scope bytes — no leakage across
    // the seam in either direction.
    let foreign = core_db::org_usage::refresh_orgs(&db.admin, &[other.org])
        .await
        .expect("refresh foreign")
        .into_iter()
        .next()
        .expect("row");
    assert_eq!(
        foreign.object_store_bytes, 7000,
        "a client-scope record is attributed to its client's org, not leaked"
    );

    // And an org with no vault records at all reports 0, not NULL — the
    // COALESCE inside each term is what keeps the CHECK satisfied.
    let bare = seed_org(&db.app, 7008).await;
    let empty = core_db::org_usage::refresh_orgs(&db.admin, &[bare.org])
        .await
        .expect("refresh empty")
        .into_iter()
        .next()
        .expect("row");
    assert_eq!(empty.object_store_bytes, 0);
}

/// A vault folder's scope. `secret_folders` carries
/// `CHECK (num_nonnulls(org_id, client_id) = 1)`, so a folder is org-scoped
/// OR client-scoped, never both — this type makes the illegal combination
/// unrepresentable instead of relying on the caller to pass the right shape.
#[derive(Clone, Copy)]
enum FolderScope {
    Org(Uuid),
    Client(Uuid),
}

impl FolderScope {
    fn org_id(self) -> Option<Uuid> {
        match self {
            FolderScope::Org(id) => Some(id),
            FolderScope::Client(_) => None,
        }
    }
    fn client_id(self) -> Option<Uuid> {
        match self {
            FolderScope::Org(_) => None,
            FolderScope::Client(id) => Some(id),
        }
    }
}

/// Seeds a vault folder in the given scope, mirroring the folder/record
/// scope-pair equality the write path enforces.
async fn seed_secret_folder(
    tx: &mut sqlx::PgConnection,
    scope: FolderScope,
    folder_slot: u64,
    kind: &str,
) -> Uuid {
    // Distinct slot per folder: the ids are fixed so parallel rows in this
    // suite never collide, and the folders must be distinct because the
    // (scope, parent, slug) unique index is what a real vault enforces.
    let id = fixed_uuid(folder_slot);
    sqlx::query(
        "INSERT INTO secret_folders (id, org_id, client_id, slug, label, kind) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(scope.org_id())
    .bind(scope.client_id())
    .bind(format!("folder-{id}"))
    .bind("Documents")
    .bind(kind)
    .execute(&mut *tx)
    .await
    .expect("folder insert");
    id
}

/// Seeds one vault document record in the given scope. `file_bytes` is the
/// recorded payload size (None for a record with no object in the store).
async fn seed_secret_document(
    tx: &mut sqlx::PgConnection,
    id: Uuid,
    org_id: Option<Uuid>,
    client_id: Option<Uuid>,
    folder_id: Uuid,
    file_bytes: Option<i64>,
) {
    sqlx::query(
        r#"INSERT INTO secret_records
             (id, org_id, client_id, folder_id, kind, title, object_key,
              file_sha256, file_bytes)
           VALUES ($1, $2, $3, $4, 'document', $5, $6, $7, $8)"#,
    )
    .bind(id)
    .bind(org_id)
    .bind(client_id)
    .bind(folder_id)
    .bind(format!("doc-{id}"))
    .bind(file_bytes.map(|_| format!("vault/object/{id}")))
    .bind(file_bytes.map(|_| "0".repeat(64)))
    .bind(file_bytes)
    .execute(&mut *tx)
    .await
    .expect("secret record insert");
}

// --- attachment scan verdicts (Wave 52 lane P6a, migration 0139) -------------

/// The verdict belongs to the OBJECT, not the upload: a second admission of
/// the same digest keeps the first record, an infection ESCALATES bytes that
/// passed before, and the schema refuses a half-set infection.
#[tokio::test]
async fn attachment_scan_verdicts_are_first_wins_and_escalate() {
    let db = test_db().await;
    let s = seed_org(&db.app, 190).await;
    let mut tx = org_tx(&db.app, s.org).await;
    let sha = "b".repeat(64);
    let admit_key = format!("org/{}/attachments/{sha}", s.org);
    let q_key = format!("org/{}/quarantine/{sha}", s.org);

    let clean = attachment_scans::record_admitted(
        &mut tx,
        attachment_scans::NewScan {
            org_id: s.org,
            sha256: &sha,
            status: ScanStatus::Clean,
            detail: None,
            scanner: "clamd",
            size_bytes: 42,
            content_type: "application/pdf",
            filename: "invoice.pdf",
            storage_key: &admit_key,
            ticket_id: None,
            uploaded_by_user_id: None,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("first admission records");
    assert!(matches!(
        clean,
        attachment_scans::AdmitOutcome::Recorded(scan) if scan.status == ScanStatus::Clean
    ));

    // A second upload of the same bytes does NOT rewrite the record: the
    // first admission is what happened, and paying for a second scan must not
    // be a way to launder the row.
    let again = attachment_scans::record_admitted(
        &mut tx,
        attachment_scans::NewScan {
            org_id: s.org,
            sha256: &sha,
            status: ScanStatus::Skipped,
            detail: Some("none mode"),
            scanner: "none",
            size_bytes: 42,
            content_type: "application/pdf",
            filename: "renamed.pdf",
            storage_key: &admit_key,
            ticket_id: None,
            uploaded_by_user_id: None,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("second admission is absorbed");
    match again {
        attachment_scans::AdmitOutcome::AlreadyRecorded(scan) => {
            assert_eq!(scan.status, ScanStatus::Clean, "the first verdict stands");
            assert_eq!(scan.scanner, "clamd");
            assert_eq!(scan.filename, "invoice.pdf");
        }
        attachment_scans::AdmitOutcome::Recorded(_) => {
            panic!("a second admission must not write a new verdict")
        }
    }

    // Scanner databases move: bytes that passed can be found later. The
    // record escalates and follows the quarantine copy the caller wrote.
    let escalated = attachment_scans::record_quarantine(
        &mut tx,
        attachment_scans::NewScan {
            org_id: s.org,
            sha256: &sha,
            status: ScanStatus::Infected,
            detail: Some("stream: Eicar-Test-Signature FOUND"),
            scanner: "clamd",
            size_bytes: 42,
            content_type: "application/pdf",
            filename: "invoice.pdf",
            storage_key: &q_key,
            ticket_id: None,
            uploaded_by_user_id: None,
            uploaded_by_portal_user_id: None,
        },
    )
    .await
    .expect("infection escalates the admitted row");
    assert!(escalated.is_quarantined());
    assert_eq!(escalated.status, ScanStatus::Infected);
    assert_eq!(escalated.storage_key, q_key);
    assert!(
        escalated.quarantined_at.is_some(),
        "an infection carries its timestamp"
    );

    let listed = attachment_scans::list_quarantined(&mut tx, s.org)
        .await
        .expect("quarantine list");
    assert_eq!(listed.len(), 1, "the escalation appears in the admin list");
    assert_eq!(listed[0].sha256, sha);

    tx.commit().await.expect("commit");

    // The schema refuses a half-set infection. This runs on its OWN
    // transaction: a failed statement aborts the surrounding one, so a bad
    // insert sharing the committed work above would only prove that Postgres
    // rolls back after an error.
    let mut tx = org_tx(&db.app, s.org).await;
    let half_set = sqlx::query(
        "INSERT INTO attachment_scans (org_id, sha256, status, scanner, size_bytes, \
         content_type, filename, storage_key) \
         VALUES ($1, $2, 'infected', 'clamd', 5, 'text/plain', 'x', 'org/x/quarantine/y')",
    )
    .bind(s.org)
    .bind("c".repeat(64))
    .execute(&mut *tx)
    .await;
    assert!(
        half_set.is_err(),
        "status='infected' with no quarantined_at must be refused"
    );
}
