//! Event-outbox bounds — DB-layer contracts (Wave 52 lane P1,
//! `p52-event-stream-bounds`).
//!
//! Pins the two promises the retention sweep and the SSE resume query lean on:
//!
//! 1. `prune_published` removes published rows past the window and NOTHING
//!    else. The unpublished row is the one that matters — it is an event the
//!    relay still owes a consumer, so a purge that took it would drop a
//!    contract event silently, which is the single failure the outbox exists
//!    to make impossible.
//! 2. It honours `batch`, so a table that has grown for a year is trimmed by
//!    many short statements rather than one long one.
//!
//! Also asserts migration 0138's two indexes exist, because the sweep and the
//! resume query are both defined by them: the prune orders by `published_at`
//! under a partial index, and the resume query filters `(org_id, occurred_at)`
//! in the same order the index declares.
//!
//! Runs on the ADMIN (owner) pool for the delete, deliberately: the worker
//! connects as the owner role precisely so housekeeping can delete across
//! orgs, and a constrained role here deletes nothing while reporting success.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use sqlx::PgPool;
use uuid::Uuid;

/// Key for the suite's advisory lock; any constant works as long as this file
/// is the only holder.
const SUITE_LOCK: i64 = 0x5200_0000_0001;

/// These tests all purge GLOBALLY, because the sweep is cross-org by design —
/// which is the point of it, and also the reason two of them running at once
/// would delete each other's fixtures and turn every count into a coin toss.
/// Test binaries run their cases on parallel threads, so each case takes a
/// transaction-scoped advisory lock and holds the transaction for its duration;
/// the lock releases when the guard drops.
async fn suite_guard(admin: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = admin.begin().await.expect("begin suite lock");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SUITE_LOCK)
        .execute(&mut *tx)
        .await
        .expect("advisory lock");
    tx
}

/// Takes the suite lock and clears the table, so each case starts from an empty
/// `outbox_events`.
///
/// The lock alone is not enough. `prune_published` is global because the sweep
/// is, so a case that deliberately leaves published rows behind hands the next
/// case extra rows to delete: the first case keeps a one-day-old row and a
/// six-day-old row to prove they survive a seven-day window, and the second
/// case's one-day window then sweeps those two up along with its own. Its
/// `removed` reads 3 where the test says 1, which is exactly how this gate run
/// failed the first time.
///
/// The wipe is a separate, committed statement rather than part of the locked
/// transaction, because a delete inside that transaction stays invisible to the
/// pool connections the assertions read through.
async fn locked_empty_suite(admin: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let guard = suite_guard(admin).await;
    sqlx::query("DELETE FROM outbox_events")
        .execute(admin)
        .await
        .expect("clear outbox_events");
    guard
}

/// Seeds one outbox row and stamps `published_at` `days_ago` in the past when
/// given (it stays NULL otherwise). Both writes ride one org-scoped
/// transaction: `outbox_events` is RLS-gated, so an unscoped UPDATE on the app
/// pool matches no rows while reporting success — the silent failure the
/// worker's module note warns about, and one this test must not walk into.
/// Timestamps are set relative to the DATABASE clock, the clock the prune
/// compares against.
async fn seed_event(app: &PgPool, org: Uuid, n: u64, published_days_ago: Option<i32>) -> Uuid {
    let id = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    core_db::outbox::insert(
        &mut tx,
        id,
        org,
        "ticket.created",
        "ticket",
        None,
        &serde_json::json!({ "probe": n }),
    )
    .await
    .expect("outbox insert");
    if let Some(days) = published_days_ago {
        sqlx::query(
            "UPDATE outbox_events SET published_at = now() - make_interval(days => $2) WHERE id = $1",
        )
        .bind(id)
        .bind(days)
        .execute(&mut *tx)
        .await
        .expect("stamp published_at");
    }
    tx.commit().await.expect("commit seed");
    id
}

async fn remaining_ids(admin: &PgPool, org: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT id FROM outbox_events WHERE org_id = $1 ORDER BY id")
        .bind(org)
        .fetch_all(admin)
        .await
        .expect("read remaining")
}

/// Seed the org once with a fixed id so the harness's per-process schema reset
/// keeps the probe ids deterministic.
async fn seed_org(app: &PgPool, n: u64) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    // `&mut *tx` derefs the transaction to its connection: `insert_org` takes a
    // generic `Executor`, and sqlx implements that for `&mut PgConnection`, not
    // for `&mut Transaction`. The check gate caught this the first time.
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("outbox-{n}"), "OBX")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit seed");
    org
}

/// The window is the whole contract: published-and-old goes, published-and-new
/// stays, and unpublished NEVER goes however old it is.
#[tokio::test]
async fn prune_removes_published_rows_past_the_window_and_nothing_else() {
    let db = test_db().await;
    let _suite = locked_empty_suite(&db.admin).await;
    let org = seed_org(&db.app, 5201).await;

    let old_published = seed_event(&db.app, org, 520_111, Some(30)).await;
    let just_past = seed_event(&db.app, org, 520_112, Some(8)).await;
    let inside = seed_event(&db.app, org, 520_113, Some(1)).await;
    let boundary = seed_event(&db.app, org, 520_114, Some(6)).await;
    // Old, and still owed to the relay: the one row the sweep must not take.
    let unpublished = seed_event(&db.app, org, 520_115, None).await;

    let removed = core_db::outbox::prune_published(&db.admin, 7, 1_000)
        .await
        .expect("prune");
    assert_eq!(removed, 2, "only the two rows past seven days go");

    let mut left = remaining_ids(&db.admin, org).await;
    left.sort();
    let mut expected = vec![inside, boundary, unpublished];
    expected.sort();
    assert_eq!(left, expected);
    assert!(
        !left.contains(&old_published) && !left.contains(&just_past),
        "both rows past the window must be gone"
    );

    // Idempotent: a second pass over the same window finds nothing.
    let again = core_db::outbox::prune_published(&db.admin, 7, 1_000)
        .await
        .expect("second prune");
    assert_eq!(again, 0);
}

/// A short window is still a window: `0` days must not become "delete every
/// published row", because the relay stamps and the sweep ticks on different
/// schedules and a row published moments ago may still be inside a client's
/// resume window.
#[tokio::test]
async fn a_one_day_window_keeps_rows_published_today() {
    let db = test_db().await;
    let _suite = locked_empty_suite(&db.admin).await;
    let org = seed_org(&db.app, 5202).await;

    let today = seed_event(&db.app, org, 520_211, Some(0)).await;
    let three_days = seed_event(&db.app, org, 520_212, Some(3)).await;

    let removed = core_db::outbox::prune_published(&db.admin, 1, 1_000)
        .await
        .expect("prune");
    assert_eq!(removed, 1);

    let left = remaining_ids(&db.admin, org).await;
    assert_eq!(left, vec![today], "the 3-day-old row goes, today's stays");
    assert!(
        !left.contains(&three_days),
        "a row published three days ago is outside a one-day window"
    );
}

/// `batch` bounds one statement. This is what stops the first pass after the
/// upgrade from holding locks and generating one enormous burst of dead tuples
/// across a year of accumulated rows.
#[tokio::test]
async fn batch_bounds_one_statement_and_repeats_until_the_range_is_empty() {
    let db = test_db().await;
    let _suite = locked_empty_suite(&db.admin).await;
    let org = seed_org(&db.app, 5203).await;

    for n in 0..5 {
        seed_event(&db.app, org, 520_300 + n, Some(30)).await;
    }

    let mut total = 0_u64;
    let mut calls = 0;
    loop {
        let removed = core_db::outbox::prune_published(&db.admin, 7, 2)
            .await
            .expect("prune");
        assert!(removed <= 2, "one statement must honour the batch size");
        if removed == 0 {
            break;
        }
        total += removed;
        calls += 1;
    }
    assert_eq!(total, 5);
    assert_eq!(calls, 3, "5 rows at 2 per statement is three statements");
    assert!(remaining_ids(&db.admin, org).await.is_empty());
}

/// Migration 0138 is the other half of the lane, and a sweep without its index
/// is a sequential scan pretending to be housekeeping.
#[tokio::test]
async fn migration_0138_indexes_exist() {
    let db = test_db().await;
    for index in ["idx_outbox_org_occurred_id", "idx_outbox_published_at"] {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{index}"))
            .fetch_one(&db.admin)
            .await
            .expect("regclass probe");
        assert!(present, "{index} must exist after migrations apply");
    }
}
