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
    assert_eq!(report.invoices, 2);
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
    assert_eq!(count(&db, "SELECT count(*) FROM invoices").await, 2);
    assert_eq!(count(&db, "SELECT count(*) FROM invoice_payments").await, 1);
    assert_eq!(count(&db, "SELECT count(*) FROM kb_articles").await, 3);
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
    assert_eq!(
        count(
            &db,
            "SELECT count(*) FROM purchase_orders WHERE status = 'ordered'"
        )
        .await,
        2
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
