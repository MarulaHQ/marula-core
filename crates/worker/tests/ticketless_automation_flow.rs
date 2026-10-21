//! Live-database proof for Wave 43 lane 3 (`p43-ticketless-automation-triggers`):
//!
//! **A ticket-less inbound event drives a board-scoped rule end-to-end.**
//!
//! Before this lane `match_rules` bailed unless `payload.ticket_id` resolved
//! a snapshot, so every ticket-less envelope — including every envelope
//! lane 1's generic inbound receiver emits — matched zero rules by
//! construction. The subject is now `(ticket_id: Option, board_id)`, and a
//! board-subject rule fires on an envelope that carries only a board.
//!
//! Covered here:
//! 1. The acceptance: lane-1-shaped envelope (`board_id`, no `ticket_id`)
//!    drives a board-scoped rule, and the match carries `ticket_id: null`.
//! 2. Negative gates — a ticket-subject rule does NOT fire ticket-less; a
//!    board rule scoped to another board does not fire; an automation echo
//!    never fires (the loop guard); a ticket-less envelope naming a board
//!    outside the org does not fire.
//! 3. Condition degradation on a ticket-less subject is not a silent
//!    no-match: a board rule with a ticket-only predicate matches nothing.
//! 4. The ticket path is unregressed — a ticket event still drives an
//!    org-wide ticket rule with its snapshot conditions.
//!
//! Database selection follows the documented worker-test contract:
//! MARULA_TEST_DATABASE_URL or DATABASE_URL (schema reset ONCE per process).

use core_domain::automations::AutomationSubjectKind;
use core_events::Envelope;
use sqlx::PgPool;
use uuid::Uuid;

/// One org, its default board, a user, a second board and a ticket, seeded
/// through the same default-first path the core-db suite uses.
struct Seeded {
    org: Uuid,
    board: Uuid,
    other_board: Uuid,
    user: Uuid,
    ticket: Uuid,
}

async fn test_pool() -> PgPool {
    static BOOTSTRAP: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
    let url = BOOTSTRAP
        .get_or_init(|| async {
            let url = std::env::var("MARULA_TEST_DATABASE_URL")
                .or_else(|_| std::env::var("DATABASE_URL"))
                .expect(
                    "set MARULA_TEST_DATABASE_URL (or DATABASE_URL) to a privileged \
                     Postgres for the ticket-less automation flow suite",
                );
            let admin = core_db::connect(&url).await.expect("connect");
            sqlx::query(
            "DO $$ BEGIN DROP SCHEMA IF EXISTS ops CASCADE; DROP SCHEMA IF EXISTS public CASCADE; END $$",
        )
                .execute(&admin)
                .await
                .expect("drop schema");
            sqlx::query("CREATE SCHEMA public")
                .execute(&admin)
                .await
                .expect("create schema");
            core_db::run_migrations(&admin).await.expect("migrations");
            url
        })
        .await;
    core_db::connect(url).await.expect("pool")
}

fn fixed_uuid(n: u64) -> Uuid {
    Uuid::from_u128(0x7469_636b_6c65_7373_u128 << 64 | u128::from(n))
}

/// RLS-scoped transaction helper (mirrors the core-db + indexer harnesses).
async fn org_tx(app: &PgPool, org_id: Uuid) -> sqlx::Transaction<'_, sqlx::Postgres> {
    let mut tx = app.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set_org");
    tx
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let user = fixed_uuid(n + 100);
    let ticket = fixed_uuid(n + 200);
    let other_board = fixed_uuid(n + 300);
    let mut tx = org_tx(app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Ticketless Co", &format!("tl-{n}"), "TL")
        .await
        .expect("org");
    core_db::orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("roles");
    let board = core_db::orgs::seed_default_board(&mut tx, org)
        .await
        .expect("board");
    // A real second board: rules are FK-bound to boards, so an off-board
    // rule needs a board that exists.
    core_db::boards::create_board(&mut *tx, org, other_board, "Second Board", None, false)
        .await
        .expect("second board");
    let entry_status: Uuid =
        sqlx::query_scalar("SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry")
            .bind(board)
            .fetch_one(&mut *tx)
            .await
            .expect("entry status");
    core_db::users::create_user(
        &mut *tx,
        user,
        org,
        &format!("ops{n}@ticketless.test"),
        "Ops User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    core_db::tickets::insert_ticket(
        &mut tx,
        core_db::tickets::NewTicket {
            id: ticket,
            org_id: org,
            number: "TL-1",
            board_id: board,
            status_id: entry_status,
            title: "Mail server down",
            body: Some("smtp is rejecting mail"),
            priority: core_domain::status::Priority::P1,
            client_id: None,
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await
    .expect("ticket");
    tx.commit().await.expect("commit");
    Seeded {
        org,
        board,
        other_board,
        user,
        ticket,
    }
}

/// Lane 1's inbound receiver shape: a board, no ticket, provider data
/// nested under `data`.
fn inbound_envelope(event_type: &str, org_id: Uuid, board_id: Option<Uuid>) -> Envelope {
    Envelope {
        id: Uuid::now_v7(),
        event_type: event_type.to_string(),
        org_id,
        entity: "webhook".into(),
        entity_id: Some("order-42".into()),
        occurred_at: chrono::Utc::now(),
        payload_schema_ref: "acme-io/webhook-v1".into(),
        payload: serde_json::json!({
            "board_id": board_id,
            "source_ref": "acme-prod",
            "entity_id": "order-42",
            "data": { "order_id": "ORD-42", "amount": 999 },
        }),
    }
}

fn notify_action(user: Uuid) -> serde_json::Value {
    serde_json::json!([{
        "type": "notify_users",
        "user_ids": [user.to_string()],
        "kind": "ticket_update",
        "title": "Inbound order",
    }])
}

#[allow(clippy::too_many_arguments)]
async fn insert_rule(
    pool: &PgPool,
    org: Uuid,
    board: Option<Uuid>,
    subject_kind: AutomationSubjectKind,
    name: &str,
    trigger: &str,
    conditions: serde_json::Value,
    actions: serde_json::Value,
) -> Uuid {
    let id = Uuid::now_v7();
    let mut tx = org_tx(pool, org).await;
    core_db::automations::insert(
        &mut *tx,
        id,
        org,
        board,
        subject_kind,
        name,
        trigger,
        conditions,
        actions,
    )
    .await
    .expect("rule insert");
    tx.commit().await.expect("commit");
    id
}

// --- tests ------------------------------------------------------------------------

/// THE acceptance: a ticket-less inbound event drives a board-scoped rule.
#[tokio::test]
async fn ticket_less_inbound_event_drives_a_board_scoped_rule() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 21).await;
    let rule = insert_rule(
        &pool,
        s.org,
        Some(s.board),
        AutomationSubjectKind::Board,
        "Inbound order → notify",
        "webhook.received",
        serde_json::json!([]),
        notify_action(s.user),
    )
    .await;

    // Exactly the envelope lane 1's receiver writes into the outbox.
    let envelope = inbound_envelope("webhook.received", s.org, Some(s.board));
    let matched = worker::automations::match_rules(&pool, &envelope).await;

    assert_eq!(matched.len(), 1, "one board-scoped rule must fire");
    let hit = &matched[0];
    assert_eq!(hit.rule_id, rule);
    assert_eq!(hit.org_id, s.org);
    assert!(hit.ticket_id.is_none(), "there is no ticket to act on");
    assert_eq!(hit.board_id, Some(s.board), "the match is board-scoped");
    assert_eq!(
        hit.actions,
        notify_action(s.user),
        "actions survive verbatim"
    );
}

/// The subject-kind gate is symmetric, and the loop guard covers both.
#[tokio::test]
async fn ticket_less_envelopes_only_reach_board_rules() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 22).await;

    // A ticket-subject rule on the SAME event type must not fire.
    insert_rule(
        &pool,
        s.org,
        Some(s.board),
        AutomationSubjectKind::Ticket,
        "ticket rule",
        "webhook.received",
        serde_json::json!([]),
        notify_action(s.user),
    )
    .await;
    // A board rule scoped to a board the envelope does not name.
    insert_rule(
        &pool,
        s.org,
        Some(s.other_board),
        AutomationSubjectKind::Board,
        "other board rule",
        "webhook.received",
        serde_json::json!([]),
        notify_action(s.user),
    )
    .await;

    let matched = worker::automations::match_rules(
        &pool,
        &inbound_envelope("webhook.received", s.org, Some(s.board)),
    )
    .await;
    assert!(
        matched.is_empty(),
        "neither a ticket rule nor an off-board board rule may fire"
    );

    // An org-wide BOARD rule is the shape that fires (no board on the rule).
    insert_rule(
        &pool,
        s.org,
        None,
        AutomationSubjectKind::Board,
        "org-wide board rule",
        "webhook.received",
        serde_json::json!([]),
        notify_action(s.user),
    )
    .await;
    let matched = worker::automations::match_rules(
        &pool,
        &inbound_envelope("webhook.received", s.org, Some(s.board)),
    )
    .await;
    assert_eq!(matched.len(), 1, "the org-wide board rule fires");
    assert!(matched[0].ticket_id.is_none());

    // An echo of that very effect carries the loop guard and never re-fires.
    let mut echoed = inbound_envelope("webhook.received", s.org, Some(s.board));
    echoed.payload["via_automation"] = serde_json::json!(true);
    let echoed = worker::automations::match_rules(&pool, &echoed).await;
    assert!(
        echoed.is_empty(),
        "the loop guard holds on the ticket-less path"
    );

    // A ticket-less envelope naming a board outside this org resolves no
    // subject at all — it must not widen into the org-wide rule either.
    let foreign = inbound_envelope("webhook.received", s.org, Some(Uuid::now_v7()));
    let matched = worker::automations::match_rules(&pool, &foreign).await;
    assert!(
        matched.is_empty(),
        "a board outside the org is not a scope that widens to org-wide rules"
    );
}

/// Degradation is loud, not silent: a board rule whose only condition reads
/// a ticket fact matches nothing (the worker warns per event).
#[tokio::test]
async fn ticket_only_conditions_degrade_on_a_board_subject() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 23).await;
    insert_rule(
        &pool,
        s.org,
        None,
        AutomationSubjectKind::Board,
        "p1 only",
        "webhook.received",
        serde_json::json!([{ "type": "priority_is", "priority": "p1" }]),
        notify_action(s.user),
    )
    .await;
    // The empty-needle wildcard is the one predicate that holds on a
    // ticket-less subject, so the same rule can fire when it says nothing.
    insert_rule(
        &pool,
        s.org,
        None,
        AutomationSubjectKind::Board,
        "wildcard",
        "webhook.received",
        serde_json::json!([{ "type": "title_contains", "needle": "" }]),
        notify_action(s.user),
    )
    .await;

    let matched = worker::automations::match_rules(
        &pool,
        &inbound_envelope("webhook.received", s.org, Some(s.board)),
    )
    .await;
    assert_eq!(
        matched.len(),
        1,
        "only the subject-agnostic rule fires; the priority rule degrades"
    );
}

/// The ticket path is unregressed: a ticket event still drives an org-wide
/// ticket rule, evaluated against the loaded snapshot.
#[tokio::test]
async fn ticket_events_still_drive_snapshot_conditions() {
    let pool = test_pool().await;
    let s = seed_org(&pool, 24).await;
    insert_rule(
        &pool,
        s.org,
        None,
        AutomationSubjectKind::Ticket,
        "unassigned p1",
        "ticket.created",
        serde_json::json!([
            { "type": "priority_is", "priority": "p1" },
            { "type": "unassigned" },
        ]),
        notify_action(s.user),
    )
    .await;

    let envelope = Envelope {
        id: Uuid::now_v7(),
        event_type: "ticket.created".to_string(),
        org_id: s.org,
        entity: "ticket".into(),
        entity_id: Some(s.ticket.to_string()),
        occurred_at: chrono::Utc::now(),
        payload_schema_ref: "core-domain/v1".into(),
        payload: serde_json::json!({
            "ticket_id": s.ticket.to_string(),
            "board_id": s.board.to_string(),
        }),
    };
    let matched = worker::automations::match_rules(&pool, &envelope).await;
    assert_eq!(matched.len(), 1, "the ticket rule still fires");
    assert_eq!(matched[0].ticket_id, Some(s.ticket));
    assert_eq!(matched[0].board_id, Some(s.board));

    // The same rule must NOT fire on a ticket-less envelope — the subject
    // it consumes is a ticket.
    let ticketless = inbound_envelope("ticket.created", s.org, Some(s.board));
    assert!(worker::automations::match_rules(&pool, &ticketless)
        .await
        .is_empty());
}
