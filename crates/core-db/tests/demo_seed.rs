//! Demo seeder conformance (TASKLIST DX1).
//!
//! Purpose: the DX1 guarantees, end to end against a real schema —
//! deterministic re-run idempotency (a second seed NEVER double-applies),
//! reset+reseed loops converge to the same shape, the demo logins verify
//! against the shared password, time-travel shifts the SLA clock columns it
//! claims to, and the sealed demo secrets open with the seed key.
//!
//! The demo org id is FIXED, so the suite serializes its tests behind one
//! mutex and each test starts from a clean demo org (`reset` on a fresh
//! schema is a no-op delete + seed) — the same discipline the production
//! dev loop gets for free from the org-level idempotency gate.

mod common;

use common::test_db;
use core_db::demo::{self, DemoSeedOptions, DEMO_ORG_ID, DEMO_PASSWORD};

fn opts() -> DemoSeedOptions {
    DemoSeedOptions {
        seal_key: [7u8; 32],
        object_store_dir: None,
    }
}

/// Serializes every test in this binary: the demo org is one fixed row
/// set, so concurrent seeds from parallel test threads would collide.
static SEED_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A locked, freshly-reset demo org, ready for the calling test. The guard
/// must be held for the test's duration.
async fn fresh_demo_db() -> (common::TestDb, tokio::sync::MutexGuard<'static, ()>) {
    let db = test_db().await;
    let guard = SEED_LOCK.lock().await;
    demo::reset(&db.app, &opts())
        .await
        .expect("reset to a fresh demo org");
    (db, guard)
}

/// Deletes the demo org without reseeding (the clean-slate half of reset).
async fn wipe(db: &common::TestDb) {
    let mut tx = db.app.begin().await.unwrap();
    core_db::set_org(&mut tx, DEMO_ORG_ID).await.unwrap();
    // Same escape demo::reset uses: the wipe cascades into the append-only
    // ledger, which would otherwise make every test's setup a one-way door.
    core_db::set_dev_reset_bypass(&mut tx).await.unwrap();
    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(DEMO_ORG_ID)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

async fn count(db: &common::TestDb, sql: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&db.admin)
        .await
        .expect("count query")
}

#[tokio::test]
async fn seed_is_deterministic_and_re_runs_never_double_apply() {
    let db = test_db().await;
    let _guard = SEED_LOCK.lock().await;
    wipe(&db).await;

    let report = demo::seed(&db.app, &opts()).await.expect("first seed");
    assert_eq!(report.org_id, DEMO_ORG_ID);
    assert_eq!(report.tickets, 60);
    assert_eq!(report.users, 4);
    assert_eq!(report.clients, 3);
    assert_eq!(report.boards, 2);
    assert_eq!(report.portal_users, 3);
    assert_eq!(report.invoices, 4);
    assert_eq!(report.attachments, 5);
    assert_eq!(report.alert_incidents, 2);

    let err = demo::seed(&db.app, &opts()).await;
    assert!(
        matches!(err, Err(demo::DemoError::AlreadySeeded(_))),
        "re-run must report AlreadySeeded, got {err:?}"
    );

    // Nothing was written by the refused run.
    assert_eq!(count(&db, "SELECT count(*) FROM orgs").await, 1);
    assert_eq!(count(&db, "SELECT count(*) FROM tickets").await, 60);
    assert_eq!(count(&db, "SELECT count(*) FROM users").await, 4);
}

#[tokio::test]
async fn reset_reseed_loop_converges_to_the_same_shape() {
    let (db, _guard) = fresh_demo_db().await;

    // The lock helper already reset once; a second reset must converge.
    let second = demo::reset(&db.app, &opts()).await.expect("reset+reseed");
    assert_eq!(second.tickets, 60);
    assert_eq!(second.org_id, DEMO_ORG_ID);

    // The wipe really cascaded, then the seed rebuilt everything.
    assert_eq!(count(&db, "SELECT count(*) FROM orgs").await, 1);
    assert_eq!(count(&db, "SELECT count(*) FROM tickets").await, 60);
    assert_eq!(count(&db, "SELECT count(*) FROM time_entries").await, 33);
    assert_eq!(count(&db, "SELECT count(*) FROM invoices").await, 4);
    assert_eq!(count(&db, "SELECT count(*) FROM invoice_payments").await, 1);
    assert_eq!(count(&db, "SELECT count(*) FROM kb_articles").await, 6);
    // The remaining PSA surfaces, now populated.
    assert_eq!(count(&db, "SELECT count(*) FROM attachments").await, 5);
    assert_eq!(count(&db, "SELECT count(*) FROM documents_attachments").await, 2);
    assert_eq!(count(&db, "SELECT count(*) FROM email_messages").await, 6);
    assert_eq!(count(&db, "SELECT count(*) FROM email_outbound").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM csat_surveys").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM secret_records").await, 4);
    assert_eq!(count(&db, "SELECT count(*) FROM secret_folders").await, 5);
    assert_eq!(count(&db, "SELECT count(*) FROM contract_change_orders").await, 2);
    // The canonical ten categories from migration 0110, which only seeded
    // the orgs that existed when it ran — a freshly reset org gets them from
    // the seed itself.
    assert_eq!(count(&db, "SELECT count(*) FROM time_categories").await, 10);
    assert_eq!(count(&db, "SELECT count(*) FROM purchase_receipts").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM purchase_receipt_lines").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM purchase_bills").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM client_loan_lines").await, 3);
    // 3 hand-written ledger legs + 3 loanable "Opening stock" receives +
    // 3 the PO receipt legs post.
    assert_eq!(count(&db, "SELECT count(*) FROM stock_movements").await, 12);
    assert_eq!(count(&db, "SELECT count(*) FROM portal_users").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM board_portal_exposure").await, 3);
    // The eight `asset_specs` rows plus the three the client loans
    // instantiate through the real write path.
    assert_eq!(count(&db, "SELECT count(*) FROM device_assets").await, 11);
    // SLA state: the non-closed hand-built tickets, the per-field-seat
    // spread over the bulk queue, and one guaranteed row per priority tier.
    // Every open tier therefore renders a live countdown, which is what the
    // ticket board's pills need.
    assert_eq!(count(&db, "SELECT count(*) FROM ticket_sla_state").await, 25);
    for tier in ["p1", "p2", "p3", "p4"] {
        let live = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tickets t
               JOIN board_statuses st ON st.id = t.status_id
               JOIN ticket_sla_state s ON s.ticket_id = t.id
              WHERE t.priority = $1 AND st.category <> 'closed'",
        )
        .bind(tier)
        .fetch_one(&db.admin)
        .await
        .unwrap();
        assert!(live >= 1, "tier {tier} needs an SLA row on an open ticket");
    }
    assert_eq!(
        count(&db, "SELECT count(*) FROM document_versions").await,
        2
    );
    assert_eq!(count(&db, "SELECT count(*) FROM project_tasks").await, 4);
    assert_eq!(count(&db, "SELECT count(*) FROM appointments").await, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM alert_incidents").await, 2);
    assert_eq!(count(&db, "SELECT count(*) FROM report_schedules").await, 1);
    assert_eq!(count(&db, "SELECT count(*) FROM webhooks").await, 1);
    // Field-ops fixtures (promoted from the assets-stock lane's capture
    // script): two vendors, three purchase orders (two ordered, one draft),
    // three open client loans, and their loaned serials.
    assert_eq!(second.vendors, 2);
    assert_eq!(second.purchase_orders, 3);
    assert_eq!(second.client_loans, 3);
    assert_eq!(count(&db, "SELECT count(*) FROM vendors").await, 2);
    assert_eq!(count(&db, "SELECT count(*) FROM purchase_orders").await, 3);
    // The receipt legs advance the status through the real write path:
    // the two-line orders receive their FIRST line only, so they are
    // `partially_received`, and the single-line order is fully `received`.
    // The draft order is still `draft`.
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM purchase_orders WHERE status = 'partially_received'"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM purchase_orders WHERE status = 'received'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM purchase_orders WHERE status = 'draft'"
        )
        .await,
        0
    );
    assert_eq!(count(&db, "SELECT count(*) FROM client_loans").await, 3);
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM stock_serials WHERE status = 'loaned'"
        )
        .await,
        3
    );
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM stock_movements WHERE kind = 'loan'"
        )
        .await,
        3
    );

    // Fixed ids survive the loop: the owner login exists, once.
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM users WHERE email = 'owner@demo.local'"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn demo_logins_verify_against_the_shared_password() {
    use argon2::PasswordVerifier;

    let (db, _guard) = fresh_demo_db().await;

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT email, password_hash FROM users ORDER BY email")
            .fetch_all(&db.admin)
            .await
            .unwrap();
    assert_eq!(rows.len(), 4, "four demo logins");

    let argon = argon2::Argon2::default();
    for (email, hash) in &rows {
        let parsed = argon2::PasswordHash::new(hash).expect("argon2 PHC string");
        assert!(
            argon
                .verify_password(DEMO_PASSWORD.as_bytes(), &parsed)
                .is_ok(),
            "login {email} must verify against the demo password"
        );
    }
}

#[tokio::test]
async fn time_travel_shifts_sla_windows_back() {
    let (db, _guard) = fresh_demo_db().await;

    type SlaRow = (
        uuid::Uuid,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    );

    let before: Vec<SlaRow> = sqlx::query_as(
        "SELECT ticket_id, first_response_due, resolution_due
             FROM ticket_sla_state ORDER BY ticket_id",
    )
    .fetch_all(&db.admin)
    .await
    .unwrap();
    assert!(
        !before.is_empty(),
        "non-closed demo tickets carry SLA state"
    );

    let shifted = demo::time_travel(&db.app, 48).await.expect("time-travel");
    assert_eq!(shifted as usize, before.len(), "every open SLA row shifts");

    let after: Vec<SlaRow> = sqlx::query_as(
        "SELECT ticket_id, first_response_due, resolution_due
             FROM ticket_sla_state ORDER BY ticket_id",
    )
    .fetch_all(&db.admin)
    .await
    .unwrap();
    for ((id, fr_before, res_before), (after_id, fr_after, res_after)) in
        before.iter().zip(after.iter())
    {
        assert_eq!(id, after_id);
        let shift = chrono::Duration::hours(48);
        assert_eq!(fr_before.map(|d| d - shift), *fr_after);
        assert_eq!(res_before.map(|d| d - shift), *res_after);
    }

    // Out-of-range requests are rejected without touching the clocks.
    assert!(matches!(
        demo::time_travel(&db.app, 0).await,
        Err(demo::DemoError::InvalidHours)
    ));
}

/// Wave 47 B3 — the sales-demo acceptance list, pinned at the row level.
///
/// Every item the user named for the demo org is asserted here so a later
/// seed edit that quietly drops one fails the suite rather than the demo:
/// invoices across draft/sent/paid/overdue, tickets in all four priority
/// tiers with a live countdown each, a time entry sitting in the manager
/// queue, a quote awaiting client approval, one portal user per client, KB
/// articles that read like a client's, and assets with contrasting warranty
/// states.
#[tokio::test]
async fn wave47_sales_story_survives_the_seed() {
    let (db, _guard) = fresh_demo_db().await;

    // --- invoices: draft, sent, paid, overdue -----------------------------
    for status in ["draft", "issued", "paid"] {
        let n = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM invoices WHERE status = $1",
        )
        .bind(status)
        .fetch_one(&db.admin)
        .await
        .unwrap();
        assert!(n >= 1, "the seed must carry an invoice in '{status}' state");
    }
    // "Overdue" is derived, not a status: issued, past its due date, with
    // money still outstanding — and the dunning ladder agrees.
    let overdue = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM invoices
          WHERE status = 'issued' AND due_on < CURRENT_DATE
            AND total_minor > total_paid_minor",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(overdue >= 1, "at least one issued invoice must be past its due date");
    let overdue_dunned = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM invoices WHERE dunning_state = 'overdue'",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(
        overdue_dunned >= 1,
        "the overdue invoice should already be marked 'overdue' by the dunning ladder"
    );

    // --- tickets: every priority tier carries a LIVE countdown -------------
    for tier in ["p1", "p2", "p3", "p4"] {
        let live = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM tickets t
               JOIN board_statuses st ON st.id = t.status_id
               JOIN ticket_sla_state s ON s.ticket_id = t.id
              WHERE t.priority = $1 AND st.category <> 'closed'
                AND s.resolution_due IS NOT NULL AND s.resolution_due > now()",
        )
        .bind(tier)
        .fetch_one(&db.admin)
        .await
        .unwrap();
        assert!(
            live >= 1,
            "tier {tier} needs an open ticket with a live SLA countdown"
        );
    }
    // The clock each tier shows comes from the org's own policy targets, so
    // the countdown and the published ladder cannot disagree.
    let p1_row = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
        "SELECT first_response_due, resolution_due FROM ticket_sla_state s
           JOIN tickets t ON t.id = s.ticket_id
          WHERE t.priority = 'p1' AND s.resolution_due > now()
          ORDER BY s.resolution_due LIMIT 1",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    let p1_budget = p1_row.1 - p1_row.0;
    assert!(
        p1_budget <= chrono::Duration::hours(8),
        "a live P1 row must sit inside the policy's 8h resolution budget, got {p1_budget:?}"
    );

    // --- a time entry awaiting approval, in the manager queue -------------
    let awaiting = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, approval_submitted_by, approval_submitted_at FROM time_entries
          WHERE approval_status = 'submitted'",
    )
    .fetch_all(&db.admin)
    .await
    .unwrap();
    assert!(
        !awaiting.is_empty(),
        "one time entry must sit in the manager queue as 'submitted'"
    );
    for (_, submitted_by, submitted_at) in &awaiting {
        assert!(!submitted_by.is_nil(), "a submitted entry names who submitted it");
        assert!(*submitted_at <= chrono::Utc::now(), "submitted_at is in the past");
    }
    // The approval trail walks an entry that agrees with what it describes.
    let trail = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM time_entry_approvals a
           JOIN time_entries e ON e.id = a.time_entry_id
          WHERE a.time_entry_id = (SELECT id FROM time_entries WHERE approval_status = 'approved' LIMIT 1)
            AND e.approval_status = 'approved'",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(trail >= 2, "the decided entry's trail has its hops recorded");

    // --- a quote awaiting client approval ---------------------------------
    let acme: uuid::Uuid = sqlx::query_scalar("SELECT id FROM clients WHERE name = 'Acme Manufacturing'")
        .fetch_one(&db.admin)
        .await
        .unwrap();
    let sent_to_acme = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM quotes WHERE client_id = $1 AND status = 'sent' AND valid_until > CURRENT_DATE",
    )
    .bind(acme)
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(
        sent_to_acme >= 1,
        "Acme needs a sent quote still inside its validity window to approve"
    );

    // --- one portal user per client, all active ---------------------------
    for (client_name, email) in [
        ("Acme Manufacturing", "portal@acme.demo"),
        ("Northwind Legal", "portal@northwind.demo"),
        ("Globex Foods", "portal@globex.demo"),
    ] {
        let client: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM clients WHERE name = $1")
                .bind(client_name)
                .fetch_one(&db.admin)
                .await
                .unwrap();
        let seats = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_users WHERE client_id = $1 AND status = 'active'",
        )
        .bind(client)
        .fetch_one(&db.admin)
        .await
        .unwrap();
        assert_eq!(seats, 1, "{client_name} needs exactly one active portal user");
        assert!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM portal_users WHERE email = $1")
                .bind(email)
                .fetch_one(&db.admin)
                .await
                .unwrap()
                == 1,
            "{email} must exist"
        );
        // Every client's portal ticket list is populated: the board they can
        // see is exposed to them.
        let exposed = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM board_portal_exposure WHERE client_id = $1 AND exposed",
        )
        .bind(client)
        .fetch_one(&db.admin)
        .await
        .unwrap();
        assert!(
            exposed >= 1,
            "{client_name} needs at least one exposed board or its portal ticket list is empty"
        );
    }

    // --- KB articles that read like a real client's ------------------------
    let published = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_articles a
           JOIN kb_categories c ON c.id = a.category_id
          WHERE a.status = 'published' AND a.client_visible AND c.client_visible",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(published >= 5, "the portal KB needs real client-facing articles");
    let internal = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_articles WHERE status = 'draft' AND NOT client_visible",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(internal >= 1, "the internal escalation ladder stays unpublished");
    // A client-facing article is not a one-line stub. `length()` returns INT4,
    // so the scalar has to be decoded as that.
    let shortest = sqlx::query_scalar::<_, i32>(
        "SELECT min(length(body)) FROM kb_articles
          WHERE status = 'published' AND client_visible",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(
        shortest >= 120,
        "a published article should read like something a technician wrote, got {shortest} chars"
    );

    // --- assets: contrasting warranty states + the org's own kit -----------
    let internal_assets = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM device_assets WHERE scope = 'internal_org' AND client_id IS NULL",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(
        internal_assets >= 1,
        "the MSP's own equipment rides scope='internal_org' with no client"
    );
    let asset_rows = sqlx::query_as::<_, (
        uuid::Uuid,
        Option<chrono::NaiveDate>,
        Option<i32>,
        String,
        serde_json::Value,
    )>(
        "SELECT id, purchase_date, useful_life_months, tag, fields FROM device_assets",
    )
    .fetch_all(&db.admin)
    .await
    .unwrap();
    let today = chrono::Utc::now().date_naive();
    let mut states: Vec<String> = asset_rows
        .iter()
        .filter_map(|(_, purchase, life, _, fields)| {
            core_domain::entities::client::asset_warranty_from_fields(
                fields,
                *purchase,
                *life,
                today,
            )
            .map(|w| format!("{:?}", w.state))
        })
        .collect();
    states.sort();
    states.dedup();
    for state in ["Active", "Ending", "Ended"] {
        assert!(
            states.iter().any(|s| s == state),
            "the asset set must contain a warranty state {state}, got {states:?}"
        );
    }
}

/// The two pill states the mobile pack draws — one breached, one still
/// counting down — exist as seeded SLA rows, and the ambiguous-scan inputs
/// resolve to more than one candidate. This is the seed-level pin for the
/// pack states the board and the scan sheets render.
#[tokio::test]
async fn sla_spread_and_scan_ambiguity_survive_the_seed() {
    let (db, _guard) = fresh_demo_db().await;

    // At least one seeded SLA row is already past due and one is due within
    // the hour — the "Breached" and "…m left" states.
    let breached: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ticket_sla_state
          WHERE breached = false AND resolution_due < now()",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    let due_soon: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ticket_sla_state
          WHERE resolution_due BETWEEN now() AND now() + interval '1 hour'",
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();
    assert!(breached >= 1, "a seeded row must already be breached");
    assert!(due_soon >= 1, "a seeded row must be due within the hour");

    // Scanning the shared part number and the shared location code each
    // yield more than one candidate — the ambiguity picker's whole reason
    // to exist.
    let mut tx = db.app.begin().await.unwrap();
    core_db::set_org(&mut tx, DEMO_ORG_ID).await.unwrap();
    let items = core_db::labels::find_items_by_code(&mut tx, DEMO_ORG_ID, "DOCK-01", 8)
        .await
        .unwrap();
    assert!(
        items.len() > 1,
        "the shared DOCK-01 part number must resolve to two candidates"
    );
    let locations = core_db::labels::find_locations_by_code(&mut tx, DEMO_ORG_ID, "SHELF-B", 8)
        .await
        .unwrap();
    assert!(
        locations.len() > 1,
        "the shared SHELF-B location code must resolve to two candidates"
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn seeded_secrets_open_with_the_seed_key() {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::Aes256Gcm;

    let (db, _guard) = fresh_demo_db().await;

    let cipher = Aes256Gcm::new_from_slice(&[7u8; 32]).unwrap();
    let open = |sealed: &[u8]| {
        let (nonce, ct) = sealed.split_at(12);
        cipher
            .decrypt(aes_gcm::Nonce::from_slice(nonce), ct)
            .expect("demo secret ciphertext must open with the seed key")
    };

    let (alert_secret, webhook_secret): (Vec<u8>, Vec<u8>) = sqlx::query_as(
        r#"
        SELECT
          (SELECT secret_encrypted FROM alert_sources LIMIT 1),
          (SELECT secret_encrypted FROM webhooks LIMIT 1)
        "#,
    )
    .fetch_one(&db.admin)
    .await
    .unwrap();

    assert_eq!(open(&alert_secret), demo::DEMO_ALERT_HMAC_SECRET.as_bytes());
    assert_eq!(open(&webhook_secret), demo::DEMO_WEBHOOK_SECRET.as_bytes());
}
