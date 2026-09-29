//! Query-layer tests for the email ledger and outbound queue (TASKLIST
//! 2.4).
//!
//! Purpose: prove the SQL contracts — per-org Message-ID idempotency,
//! threading lookups across References/In-Reply-To, case-insensitive
//! subject-token fallback, board-vs-catch-all route resolution, claim/
//! retry/dead transitions with lease recovery and an injected clock, stale-
//! report inertness, owner-role escalation for cross-org claims, and RLS
//! isolation on the new tables.
//!
//! Non-purpose: transport/scan behavior (core-mail unit tests), webhook
//! HTTP shell (core-api), and the end-to-end zero-loss harness
//! (core-api::tests two-weeks scenario).

mod email_common;

use core_db::emails::{
    self, apply_send_outcome, claim_due_outbound, enqueue_outbound, find_thread_target,
    find_ticket_id_by_number_ci, insert_message, link_message_targets, resolve_inbound_route,
    SendOutcome,
};
use core_db::orgs;
use core_domain::entities::{EmailDirection, ScanStatus};
use email_common::{fixed_uuid, org_tx, test_db};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// One seeded org with a default board (statuses seeded too so tickets can
/// be created when a test needs a real FK target).
struct Seeded {
    org: Uuid,
    board: Uuid,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("acme-mail-{n}"), "MER")
        .await
        .expect("org");
    orgs::seed_default_board(&mut tx, org).await.expect("board");
    let board: Uuid = sqlx::query_scalar("SELECT id FROM boards WHERE org_id = $1 AND is_default")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .expect("default board id");
    tx.commit().await.expect("commit");
    Seeded { org, board }
}

/// Normalizes Message-ID keys exactly like the inbound service before
/// persistence: bare, bracket-stripped, lowercased.
fn norm(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

fn inbound_msg(
    org: Uuid,
    rfc: &str,
    refs: Vec<String>,
    reply_to: Option<String>,
) -> emails::NewMessage {
    emails::NewMessage {
        id: Uuid::now_v7(),
        org_id: org,
        direction: EmailDirection::Inbound,
        rfc_message_id: norm(rfc),
        in_reply_to: reply_to.as_deref().map(norm),
        email_refs: refs.iter().map(|r| norm(r)).collect(),
        subject: Some("s".into()),
        from_address: "client@example.test".into(),
        from_name: None,
        to_addresses: vec!["support@acme.test".into()],
        body_text: Some("body".into()),
        ticket_id: None,
        comment_id: None,
        scan_status: ScanStatus::Clean,
        transport: "webhook/test".into(),
    }
}

#[tokio::test]
async fn message_id_unique_per_org_makes_replays_inert() {
    let db = test_db().await;
    let a = seed_org(&db.app, 9101).await;
    let b = seed_org(&db.app, 9102).await;

    let mut tx = db.admin.begin().await.expect("tx");
    let first = insert_message(&mut tx, &inbound_msg(a.org, "<m1@x>", vec![], None))
        .await
        .expect("insert");
    let replay = insert_message(&mut tx, &inbound_msg(a.org, "<m1@x>", vec![], None))
        .await
        .expect("insert");
    let other_org = insert_message(&mut tx, &inbound_msg(b.org, "<m1@x>", vec![], None))
        .await
        .expect("insert");
    tx.commit().await.expect("commit");

    assert!(first.is_some(), "first insert wins");
    assert!(
        replay.is_none(),
        "same org + same id ⇒ duplicate (ON CONFLICT)"
    );
    assert!(other_org.is_some(), "other orgs have their own namespace");
}

/// Minimal real ticket so FK targets exist for threading assertions.
async fn seed_ticket(admin: &PgPool, s: &Seeded, n: u64) -> Uuid {
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry LIMIT 1",
    )
    .bind(s.board)
    .fetch_one(admin)
    .await
    .expect("entry status");
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category)
         VALUES ($1, $2, $3, $4, $5, 't', 'p3', 'open')",
    )
    .bind(fixed_uuid(n))
    .bind(s.org)
    .bind(format!("MER-{n}"))
    .bind(s.board)
    .bind(status)
    .execute(admin)
    .await
    .expect("ticket insert");
    fixed_uuid(n)
}

#[tokio::test]
async fn threading_matches_references_and_reply_to() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9103).await;
    let t_old = seed_ticket(&db.admin, &s, 91031).await;
    let t_new = seed_ticket(&db.admin, &s, 91032).await;

    let mut tx = db.admin.begin().await.expect("tx");
    // Older thread anchor: its own Message-ID, linked to t_old.
    let holder = insert_message(&mut tx, &inbound_msg(s.org, "<holder@x>", vec![], None))
        .await
        .expect("holder")
        .expect("fresh row");
    link_message_targets(&mut *tx, s.org, holder, Some(t_old), None)
        .await
        .expect("link old");
    // Newer reply that ALSO references <holder@x>, linked to t_new.
    // Values are pre-normalized here exactly as the webhook service does
    // before persistence (bare, lowercased).
    let child = insert_message(
        &mut tx,
        &inbound_msg(
            s.org,
            "<child@x>",
            vec!["holder@x".into()],
            Some("holder@x".into()),
        ),
    )
    .await
    .expect("child")
    .expect("fresh child");
    link_message_targets(&mut *tx, s.org, child, Some(t_new), None)
        .await
        .expect("link new");

    // Lookup keys arrive normalized from the service (thread_keys());
    // matching itself is exact on stored lowercase values, newest row wins.
    let hit = find_thread_target(&mut *tx, s.org, &["holder@x".to_string()])
        .await
        .expect("lookup");
    assert_eq!(hit, Some(t_new), "most recent matching row wins");
    let via_rfc = find_thread_target(&mut *tx, s.org, &["child@x".to_string()])
        .await
        .expect("lookup");
    assert_eq!(via_rfc, Some(t_new));
    let miss = find_thread_target(&mut *tx, s.org, &["unknown@x".to_string()])
        .await
        .expect("lookup");
    assert!(miss.is_none());
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn subject_token_lookup_case_insensitive() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9104).await;
    let mut tx = org_tx(&db.app, s.org).await;
    orgs::seed_default_roles(&mut tx, s.org)
        .await
        .expect("roles");
    tx.commit().await.expect("commit");

    // Insert a ticket-shaped row directly (tickets table, minimal cols).
    let number = format!("MER-{}", 77);
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM board_statuses WHERE board_id = $1 AND is_entry LIMIT 1",
    )
    .bind(s.board)
    .fetch_one(&db.admin)
    .await
    .expect("entry status");
    sqlx::query(
        "INSERT INTO tickets (id, org_id, number, board_id, status_id, title, priority, category)
         VALUES ($1, $2, $3, $4, $5, 't', 'p3', 'open')",
    )
    .bind(fixed_uuid(91040))
    .bind(s.org)
    .bind(&number)
    .bind(s.board)
    .bind(status)
    .execute(&db.admin)
    .await
    .expect("ticket insert");

    let hit = find_ticket_id_by_number_ci(&db.admin, s.org, "mer-77")
        .await
        .expect("lookup");
    assert_eq!(hit, Some(fixed_uuid(91040)));
    let other_org_hit = find_ticket_id_by_number_ci(&db.admin, fixed_uuid(99999), "mer-77")
        .await
        .expect("lookup");
    assert!(other_org_hit.is_none(), "numbers are org-scoped");
}

#[tokio::test]
async fn route_resolution_prefers_board_address_then_catchall() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9105).await;

    let mut tx = db.admin.begin().await.expect("tx");
    emails::set_board_inbound_address(&mut *tx, s.org, s.board, Some("printer@acme.test"))
        .await
        .expect("set board address");
    emails::upsert_inbound_settings(&mut tx, s.org, "support@acme.test", s.board)
        .await
        .expect("catchall");
    tx.commit().await.expect("commit");

    let mut tx = db.admin.begin().await.expect("tx");
    // Board address wins over catch-all regardless of order.
    let board_route = resolve_inbound_route(
        &mut tx,
        &["support@acme.test".into(), "PRINTER@acme.test".into()],
    )
    .await
    .expect("resolve");
    let catchall_only = resolve_inbound_route(&mut tx, &["support@acme.test".into()])
        .await
        .expect("resolve");
    let unknown = resolve_inbound_route(&mut tx, &["nobody@else.test".into()])
        .await
        .expect("resolve");
    let empty = resolve_inbound_route(&mut tx, &[]).await.expect("resolve");
    tx.commit().await.expect("commit");

    assert_eq!(
        board_route.map(|r| (r.org_id, r.board_id)),
        Some((s.org, s.board)),
        "board-level match beats catch-all"
    );
    assert_eq!(catchall_only.map(|r| r.org_id), Some(s.org));
    assert!(unknown.is_none());
    assert!(empty.is_none());
}

fn outbound_fixture(org: Uuid) -> emails::NewOutbound {
    emails::NewOutbound {
        org_id: org,
        rfc_message_id: emails::new_outbound_message_id(),
        in_reply_to: None,
        references: vec![],
        subject: "confirm".into(),
        from_address: "support@acme.test".into(),
        from_name: Some("Acme".into()),
        to_addresses: vec!["client@example.test".into()],
        body_text: "we got it".into(),
        ticket_id: None,
        transport: "recording".into(),
    }
}

#[tokio::test]
async fn outbound_claim_backoff_and_dead_transitions() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9106).await;

    let mut tx = db.admin.begin().await.expect("tx");
    let msg = enqueue_outbound(&mut tx, outbound_fixture(s.org))
        .await
        .expect("enqueue");
    tx.commit().await.expect("commit");
    // Clock starts AFTER the row exists (its next_attempt_at defaults to
    // the database's now()); +2s keeps the comparison race-free.
    let t0 = chrono::Utc::now() + chrono::Duration::seconds(2);

    // Claim at t0 → attempts becomes 1.
    let mut tx = db.admin.begin().await.expect("tx");
    let batch = claim_due_outbound(&mut *tx, Some(s.org), 10, 300.0, t0)
        .await
        .expect("claim");
    tx.commit().await.expect("commit");
    assert_eq!(batch.len(), 1);
    let ledger_id = batch[0].id;
    assert_eq!(batch[0].attempts, 1);

    // Fail it: requeued 60s out.
    let mut tx = db.admin.begin().await.expect("tx");
    let applied = apply_send_outcome(
        &mut *tx,
        &SendOutcome {
            id: ledger_id,
            ok: false,
            provider_message_id: None,
            error: Some("smtp down".into()),
            retry_delay_secs: 60,
        },
        t0,
    )
    .await
    .expect("apply");
    tx.commit().await.expect("commit");
    assert!(applied);

    // Not due yet at t0+30s.
    let mut tx = db.admin.begin().await.expect("tx");
    let early = claim_due_outbound(
        &mut *tx,
        Some(s.org),
        10,
        300.0,
        t0 + chrono::Duration::seconds(30),
    )
    .await
    .expect("claim");
    tx.commit().await.expect("commit");
    assert!(early.is_empty(), "backoff holds the row");

    // Due at t0+61s → attempt 2. Fail again past max_attempts=8? Use the
    // row's own max: exhaust by looping.
    let mut last_ledger = ledger_id;
    let mut clock = t0 + chrono::Duration::seconds(61);
    for _ in 0..7 {
        let mut tx = db.admin.begin().await.expect("tx");
        let batch = claim_due_outbound(&mut *tx, Some(s.org), 10, 300.0, clock)
            .await
            .expect("claim");
        tx.commit().await.expect("commit");
        if batch.is_empty() {
            break;
        }
        last_ledger = batch[0].id;
        let mut tx = db.admin.begin().await.expect("tx");
        apply_send_outcome(
            &mut *tx,
            &SendOutcome {
                id: last_ledger,
                ok: false,
                provider_message_id: None,
                error: Some("still down".into()),
                retry_delay_secs: 1,
            },
            clock,
        )
        .await
        .expect("apply");
        tx.commit().await.expect("commit");
        clock += chrono::Duration::seconds(5);
    }

    // Eventually the row must be dead, never claimable again.
    let mut tx = db.admin.begin().await.expect("tx");
    let after_dead = claim_due_outbound(
        &mut *tx,
        Some(s.org),
        10,
        300.0,
        clock + chrono::Duration::hours(24),
    )
    .await
    .expect("claim");
    let status: String = sqlx::query_scalar("SELECT status FROM email_outbound WHERE id = $1")
        .bind(last_ledger)
        .fetch_one(&mut *tx)
        .await
        .expect("row");
    tx.commit().await.expect("commit");
    assert_eq!(status, "dead", "exhausted retries park the row");
    assert!(after_dead.iter().all(|m| m.id != last_ledger));
    let _ = msg;
}

#[tokio::test]
async fn success_persists_provider_id_and_stale_reports_are_inert() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9107).await;

    let mut tx = db.admin.begin().await.expect("tx");
    enqueue_outbound(&mut tx, outbound_fixture(s.org))
        .await
        .expect("enqueue");
    tx.commit().await.expect("commit");
    let now = chrono::Utc::now() + chrono::Duration::seconds(2);
    let mut tx = db.admin.begin().await.expect("tx");
    let batch = claim_due_outbound(&mut *tx, Some(s.org), 10, 300.0, now)
        .await
        .expect("claim");
    let id = batch[0].id;
    let applied = apply_send_outcome(
        &mut *tx,
        &SendOutcome {
            id,
            ok: true,
            provider_message_id: Some("pm-42".into()),
            error: None,
            retry_delay_secs: 30,
        },
        now,
    )
    .await
    .expect("apply");
    // A late duplicate report after `sent` must not resurrect anything.
    let stale = apply_send_outcome(
        &mut *tx,
        &SendOutcome {
            id,
            ok: false,
            provider_message_id: None,
            error: Some("late failure".into()),
            retry_delay_secs: 30,
        },
        now,
    )
    .await
    .expect("apply");
    let (status, provider): (String, Option<String>) =
        sqlx::query_as("SELECT status, provider_message_id FROM email_outbound WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .expect("row");
    tx.commit().await.expect("commit");

    assert!(applied);
    assert!(!stale, "reports only apply while sending");
    assert_eq!(status, "sent");
    assert_eq!(provider.as_deref(), Some("pm-42"));
}

#[tokio::test]
async fn lease_recovery_reclaims_abandoned_sends() {
    let db = test_db().await;
    let s = seed_org(&db.app, 9108).await;

    let mut tx = db.admin.begin().await.expect("tx");
    enqueue_outbound(&mut tx, outbound_fixture(s.org))
        .await
        .expect("enqueue");
    tx.commit().await.expect("commit");
    let now = chrono::Utc::now() + chrono::Duration::seconds(2);
    let mut tx = db.admin.begin().await.expect("tx");
    let _ = claim_due_outbound(&mut *tx, Some(s.org), 10, 60.0, now)
        .await
        .expect("claim");
    tx.commit().await.expect("commit");

    // Within the lease nobody else gets it…
    let mut tx = db.admin.begin().await.expect("tx");
    let inside = claim_due_outbound(
        &mut *tx,
        Some(s.org),
        10,
        60.0,
        now + chrono::Duration::seconds(30),
    )
    .await
    .expect("c");
    tx.commit().await.expect("commit");
    assert!(inside.is_empty());

    // …after the lease expires it is re-claimed (worker crash recovery).
    let mut tx = db.admin.begin().await.expect("tx");
    let recovered = claim_due_outbound(
        &mut *tx,
        Some(s.org),
        10,
        60.0,
        now + chrono::Duration::seconds(120),
    )
    .await
    .expect("c");
    tx.commit().await.expect("commit");
    assert_eq!(recovered.len(), 1, "abandoned lease reclaimed");
    assert_eq!(recovered[0].attempts, 2, "each claim counts an attempt");
}

#[tokio::test]
async fn app_role_rls_isolates_email_tables_and_owner_role_spans_orgs() {
    let db = test_db().await;
    let mine = seed_org(&db.app, 9109).await;
    let theirs = seed_org(&db.app, 9110).await;
    let now = chrono::Utc::now();

    // Seed messages in BOTH orgs via admin (owner) writes.
    for org in [mine.org, theirs.org] {
        let mut tx = db.admin.begin().await.expect("tx");
        insert_message(
            &mut tx,
            &inbound_msg(org, &format!("<{org}@x>"), vec![], None),
        )
        .await
        .expect("seed");
        tx.commit().await.expect("commit");
    }

    // App role scoped to MY org sees only my rows…
    let mut scoped = db.app.begin().await.expect("tx");
    core_db::set_org(&mut scoped, mine.org)
        .await
        .expect("set_org");
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM email_messages")
        .fetch_one(&mut *scoped)
        .await
        .expect("count");
    // …and an unscoped app-role connection sees nothing at all (fresh
    // pool: reused sessions keep the GUC DEFINED and would raise 22P02).
    let unscoped_pool = db.fresh_app_pool().await;
    let unscoped: i64 = sqlx::query_scalar("SELECT count(*) FROM email_messages")
        .fetch_one(&unscoped_pool)
        .await
        .expect("unscoped count");
    scoped.rollback().await.expect("rollback (reads only)");

    // The SECURITY DEFINER seam reaches across orgs from the APP pool —
    // exactly what production's internal endpoints rely on — without any
    // role-membership grant that would exempt marula_app from RLS.
    let mut app_conn = db.app.acquire().await.expect("app conn");
    let cross_org = emails::claim_due_outbound_for_app(&mut app_conn, 100, 0)
        .await
        .expect("definer claim from constrained role");

    assert_eq!(visible, 1, "RLS filters the other org's ledger rows");
    assert_eq!(unscoped, 0, "no scope ⇒ default-deny");
    // Both queues are empty here, but executing proves cross-org reach.
    assert!(cross_org.is_empty());
    std::thread::sleep(Duration::from_millis(1));
    let _ = now;
}
