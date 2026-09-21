//! Integration tests for the durable vault scale-out state (Wave 19
//! lane 1, migration 0094): cross-replica write-budget windows +
//! token-bucket bulk-ingestion pacer, both on the RLS-constrained pool
//! exactly as production rides them.

use core_db::vault_scale::{
    acquire_bulk_token, consume_write_budget_seat, BudgetSeatVerdict, PacerWait,
};
use sqlx::PgPool;

use common::{fixed_uuid, org_tx, test_db};

mod common;

/// Rolls the org's budget window into the far past (`seconds_back`) —
/// the DB-clock stand-in for "a minute elapsed between writes".
async fn roll_window_back(pool: &PgPool, org_id: uuid::Uuid, seconds_back: i64) {
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set org");
    sqlx::query(
        "UPDATE vault_write_budget_windows SET window_started = now() - \
         make_interval(secs => $2) WHERE org_id = $1",
    )
    .bind(org_id)
    .bind(seconds_back as f64)
    .execute(&mut *tx)
    .await
    .expect("roll window");
    tx.commit().await.expect("commit");
}

/// Same repositioning for the pacer bucket's refill anchor.
async fn roll_pacer_back(pool: &PgPool, org_id: uuid::Uuid, seconds_back: i64) {
    let mut tx = pool.begin().await.expect("begin");
    core_db::set_org(&mut tx, org_id).await.expect("set org");
    sqlx::query(
        "UPDATE vault_bulk_pacers SET last_refill = now() - \
         make_interval(secs => $2) WHERE org_id = $1",
    )
    .bind(org_id)
    .bind(seconds_back as f64)
    .execute(&mut *tx)
    .await
    .expect("roll pacer");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn budget_window_admits_up_to_limit_then_refuses() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9401);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9401", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    for seat in 1..=3 {
        tokio::task::yield_now().await;
        let mut conn = app.acquire().await.expect("conn");
        let verdict = consume_write_budget_seat(&mut conn, org, 3)
            .await
            .expect("seat consume");
        assert_eq!(verdict, BudgetSeatVerdict::Admitted, "seat {seat}");
        drop(conn);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let mut conn = app.acquire().await.expect("conn");
    match consume_write_budget_seat(&mut conn, org, 3).await {
        Ok(BudgetSeatVerdict::Exceeded { retry_after_secs }) => {
            assert!(
                (1..=core_db::vault_scale::BUDGET_WINDOW_SECS as i64).contains(&retry_after_secs)
            );
        }
        other => panic!("window over limit must refuse, got {other:?}"),
    }
}

#[tokio::test]
async fn budget_refused_seat_consumes_nothing() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9402);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9402", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    let mut conn = app.acquire().await.expect("conn");
    // Two admitted seats fill the limit-2 window…
    assert_eq!(
        consume_write_budget_seat(&mut conn, org, 2)
            .await
            .expect("seat 1"),
        BudgetSeatVerdict::Admitted
    );
    assert_eq!(
        consume_write_budget_seat(&mut conn, org, 2)
            .await
            .expect("seat 2"),
        BudgetSeatVerdict::Admitted
    );
    // …and further consumes are refused WITHOUT consuming anything.
    for _ in 0..30 {
        assert!(matches!(
            consume_write_budget_seat(&mut conn, org, 2)
                .await
                .expect("seat"),
            BudgetSeatVerdict::Exceeded { .. }
        ));
    }
    {
        let mut tx = org_tx(&app, org).await;
        let exhausted: i64 =
            sqlx::query_scalar("SELECT seats FROM vault_write_budget_windows WHERE org_id = $1")
                .bind(org)
                .fetch_one(&mut *tx)
                .await
                .expect("window row");
        tx.commit().await.expect("commit");
        assert_eq!(exhausted, 2, "refused writes must consume nothing");
    }
}

#[tokio::test]
async fn budget_window_rolls_by_db_clock() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9403);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9403", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    let mut conn = app.acquire().await.expect("conn");
    let _ = consume_write_budget_seat(&mut conn, org, 2)
        .await
        .expect("seat");
    roll_window_back(
        &app,
        org,
        core_db::vault_scale::BUDGET_WINDOW_SECS as i64 + 5,
    )
    .await;
    let verdict = consume_write_budget_seat(&mut conn, org, 2)
        .await
        .expect("post-roll seat");
    assert_eq!(verdict, BudgetSeatVerdict::Admitted, "rolled window admits");
    let mut tx = org_tx(&app, org).await;
    let (window_started, seats): (chrono::DateTime<chrono::Utc>, i64) = sqlx::query_as(
        "SELECT window_started, seats FROM vault_write_budget_windows WHERE org_id = $1",
    )
    .bind(org)
    .fetch_one(&mut *tx)
    .await
    .expect("window row");
    assert_eq!(seats, 1);
    let rolled_secs: f64 =
        sqlx::query_scalar("SELECT EXTRACT(EPOCH FROM (now() - $1))::double precision")
            .bind(window_started)
            .fetch_one(&mut *conn)
            .await
            .expect("elapsed");
    assert!(
        rolled_secs < core_db::vault_scale::BUDGET_WINDOW_SECS as f64,
        "window anchor moved to now (elapsed {rolled_secs})"
    );
}

#[tokio::test]
async fn pacer_grants_then_reports_exact_refill_wait() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9404);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9404", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    let mut conn = app.acquire().await.expect("conn");
    // Instant-grant on the first seeded call is NOT guaranteed — the
    // bucket starts empty and pays its refill price — so just drain
    // until a SleepMs shows up, then check the arithmetic.
    let mut saw_wait = None;
    for _ in 0..5 {
        match acquire_bulk_token(&mut conn, org, 2).await.expect("token") {
            PacerWait::Emit => continue,
            PacerWait::SleepMs(ms) => {
                saw_wait = Some(ms);
                break;
            }
        }
    }
    let wait_ms = saw_wait.expect("bucket must run dry under rate 2/s");
    assert!(
        (1..=1000).contains(&wait_ms),
        "wait hint should be ~500ms for rate 2/s, got {wait_ms}ms"
    );
    // Refund a full second: the next grant must arrive immediately.
    roll_pacer_back(&app, org, 1).await;
    let mut conn = app.acquire().await.expect("conn");
    match acquire_bulk_token(&mut conn, org, 2).await.expect("token") {
        PacerWait::Emit => {}
        PacerWait::SleepMs(ms) => {
            panic!("bucket refilled by DB clock yet still waited {ms}ms")
        }
    }
}

#[tokio::test]
async fn pacer_burst_capacity_never_exceeds_one_second() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9405);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9405", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    // A long-paused bucket (60s since last refill) holds exactly rate
    // tokens — not 60x rate: the burst cap closes the restart-rebound.
    let mut conn = app.acquire().await.expect("conn");
    let _ = acquire_bulk_token(&mut conn, org, 50).await.expect("token");
    roll_pacer_back(&app, org, 60).await;
    let mut conn = app.acquire().await.expect("conn");
    let mut tx_probe = org_tx(&app, org).await;
    let tokens: f64 = sqlx::query_scalar("SELECT tokens FROM vault_bulk_pacers WHERE org_id = $1")
        .bind(org)
        .fetch_one(&mut *tx_probe)
        .await
        .expect("pacer row");
    tx_probe.commit().await.expect("commit");
    let granted = acquire_bulk_token(&mut conn, org, 50).await.expect("token");
    assert_eq!(granted, PacerWait::Emit);
    let mut tx_probe = org_tx(&app, org).await;
    let after: f64 = sqlx::query_scalar("SELECT tokens FROM vault_bulk_pacers WHERE org_id = $1")
        .bind(org)
        .fetch_one(&mut *tx_probe)
        .await
        .expect("pacer row");
    tx_probe.commit().await.expect("commit");
    assert!(
        tokens <= 50.0,
        "refill must clamp to the burst capacity, got {tokens}"
    );
    assert!(
        (after - 49.0).abs() < f64::EPSILON,
        "grant must take one token off the capped bucket, got {after}"
    );
}

#[tokio::test]
async fn budget_and_pacer_short_circuit_on_limit_zero() {
    let db = test_db().await;
    let app = db.app;
    let org = fixed_uuid(9406);
    let mut tx = org_tx(&app, org).await;
    core_db::orgs::insert_org(&mut *tx, org, "Acme MSP", "acme-9406", "MER")
        .await
        .expect("org insert");
    tx.commit().await.expect("commit");

    let mut conn = app.acquire().await.expect("conn");
    let verdict = consume_write_budget_seat(&mut conn, org, 0)
        .await
        .expect("admitted");
    assert_eq!(verdict, BudgetSeatVerdict::Admitted);
    let wait = acquire_bulk_token(&mut conn, org, 0).await.expect("emit");
    assert_eq!(wait, PacerWait::Emit);
    // No organizational rows materialize for a disabled config.
    let budget_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vault_write_budget_windows WHERE org_id = $1")
            .bind(org)
            .fetch_one(&mut *conn)
            .await
            .expect("count");
    let pacer_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM vault_bulk_pacers WHERE org_id = $1")
            .bind(org)
            .fetch_one(&mut *conn)
            .await
            .expect("count");
    assert_eq!((budget_rows, pacer_rows), (0, 0));
}
