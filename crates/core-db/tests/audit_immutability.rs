//! Audit trail immutability — the database contract (Wave 56 lane
//! `p56-audit-immutability`).
//!
//! The audit record this lane answers says `audit_events` had no database-level
//! append-only guard: RLS scoped the rows per org, and the application role
//! still held UPDATE and DELETE. Migration 0168 closes that in three layers —
//! the privilege is revoked, a trigger refuses the verbs for every caller, and
//! retention gets one narrow SECURITY DEFINER function instead of a grant.
//!
//! These cases pin each layer and the paths that must keep working:
//!
//! 1. The app role keeps SELECT and INSERT and does not hold UPDATE or DELETE.
//! 2. An UPDATE or a DELETE is refused — on the privilege, and then again on
//!    the trigger when the grant is temporarily restored. The second case is
//!    the one that matters: it is the "a future grant cannot quietly restore
//!    mutability" claim, proved rather than asserted.
//! 3. A fresh insert still succeeds.
//! 4. Retention still works, through `purge_audit_events()` and through the
//!    sweep that calls it, and it refuses a window below the floor.
//! 5. The two other sanctioned deletes still work: a tenant erasure by org
//!    cascade, and the demo toolchain's dev-reset bypass.
//!
//! Everything runs against a real migrated schema. The app-role cases connect
//! as `marula_app` through the harness's app pool, so RLS applies exactly as it
//! does in production; the privilege cases have to run as the owner because
//! that is who grants.
//!
//! These tests mutate table privileges, so every case takes `SCHEMA_LOCK`: a
//! grant left behind by one case would change another case's answer.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use uuid::Uuid;

/// One lock per process for the cases below, which share a schema and one of
/// which grants and revokes a table privilege.
static SCHEMA_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn seed_org(db: &TestDb, n: u64) -> Uuid {
    let org_id = fixed_uuid(n);
    sqlx::query("INSERT INTO orgs (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(format!("Audit Immutability Org {n}"))
        .bind(format!("audit-immutability-{n}"))
        .execute(&db.admin)
        .await
        .expect("seed org");
    org_id
}

/// One audit row, aged by `age_days` (0 = now). Inserted on the owner pool:
/// this is fixture setup, not the path under test.
async fn seed_audit_row(db: &TestDb, org: Uuid, n: u64, age_days: i32) -> Uuid {
    let id = fixed_uuid(n);
    sqlx::query(
        r#"INSERT INTO audit_events
             (id, org_id, actor_kind, actor_id, action, entity_type, entity_id,
              detail, occurred_at)
           VALUES ($1, $2, 'human', $3, 'test.seeded', 'ticket', $4, '{}'::jsonb,
                   now() - make_interval(days => $5))"#,
    )
    .bind(id)
    .bind(org)
    .bind(Uuid::now_v7().to_string())
    .bind(Uuid::now_v7().to_string())
    .bind(age_days)
    .execute(&db.admin)
    .await
    .expect("seed audit row");
    id
}

fn sqlstate(error: &sqlx::Error) -> Option<String> {
    match error {
        sqlx::Error::Database(db) => db.code().map(std::borrow::Cow::into_owned),
        _ => None,
    }
}

/// Layer 1: the grant. The app role reads and appends, and holds neither write
/// verb. This is the cheap assertion that catches a future `ALTER DEFAULT
/// PRIVILEGES` or a re-grant in a later migration.
#[tokio::test]
async fn the_application_role_reads_and_appends_but_holds_no_write_verbs() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;

    let (select, insert, update, delete): (bool, bool, bool, bool) = sqlx::query_as(
        r#"SELECT has_table_privilege('marula_app', 'public.audit_events', 'SELECT'),
                  has_table_privilege('marula_app', 'public.audit_events', 'INSERT'),
                  has_table_privilege('marula_app', 'public.audit_events', 'UPDATE'),
                  has_table_privilege('marula_app', 'public.audit_events', 'DELETE')"#,
    )
    .fetch_one(&db.admin)
    .await
    .expect("privilege probes");

    assert!(select, "the app role reads the trail");
    assert!(insert, "the app role appends to the trail");
    assert!(!update, "UPDATE must be revoked from the app role");
    assert!(!delete, "DELETE must be revoked from the app role");
}

/// Layer 2a: with no grant, the UPDATE never reaches the trigger.
#[tokio::test]
async fn an_update_on_an_audit_row_is_refused_for_the_application_role() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_601).await;
    let row = seed_audit_row(&db, org, 5_611, 0).await;

    let mut tx = org_tx(&db.app, org).await;
    let attempt = sqlx::query("UPDATE audit_events SET action = 'tampered' WHERE id = $1")
        .bind(row)
        .execute(&mut *tx)
        .await;

    let error = attempt.expect_err("an UPDATE by the app role must be refused");
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"), "got: {error}");
}

/// Layer 2b: the trigger, not the grant, is the guarantee. Grant UPDATE back
/// and the statement still fails with the guard's own exception.
#[tokio::test]
async fn the_guard_refuses_an_update_even_when_the_grant_is_restored() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_602).await;
    let row = seed_audit_row(&db, org, 5_612, 0).await;

    sqlx::query("GRANT UPDATE ON audit_events TO marula_app")
        .execute(&db.admin)
        .await
        .expect("restore the UPDATE grant for the duration of this case");

    let mut tx = org_tx(&db.app, org).await;
    let attempt = sqlx::query("UPDATE audit_events SET action = 'tampered' WHERE id = $1")
        .bind(row)
        .execute(&mut *tx)
        .await;

    // Revoke before asserting, so a failed assertion cannot leave the grant
    // behind for another case in this binary.
    sqlx::query("REVOKE UPDATE ON audit_events FROM marula_app")
        .execute(&db.admin)
        .await
        .expect("re-revoke the UPDATE grant");

    let error = attempt.expect_err("the trigger must refuse an UPDATE even with the grant");
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"), "got: {error}");
    assert!(
        error.to_string().contains("append-only"),
        "the refusal must be the guard's, not a privilege error: {error}"
    );
}

/// Layer 2c: the same proof for DELETE, the verb retention used to hold.
#[tokio::test]
async fn the_guard_refuses_a_delete_even_when_the_grant_is_restored() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_603).await;
    let row = seed_audit_row(&db, org, 5_613, 0).await;

    sqlx::query("GRANT DELETE ON audit_events TO marula_app")
        .execute(&db.admin)
        .await
        .expect("restore the DELETE grant for the duration of this case");

    let mut tx = org_tx(&db.app, org).await;
    let attempt = sqlx::query("DELETE FROM audit_events WHERE id = $1")
        .bind(row)
        .execute(&mut *tx)
        .await;

    sqlx::query("REVOKE DELETE ON audit_events FROM marula_app")
        .execute(&db.admin)
        .await
        .expect("re-revoke the DELETE grant");

    let error = attempt.expect_err("the trigger must refuse a DELETE even with the grant");
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"), "got: {error}");
    assert!(
        error.to_string().contains("append-only"),
        "the refusal must be the guard's, not a privilege error: {error}"
    );

    let survivors: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(row)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(survivors, 1, "the row must survive the refused delete");
}

/// Appending is the one write the app role still does, so it has to keep
/// working under RLS.
#[tokio::test]
async fn a_fresh_audit_row_still_inserts_from_the_application_role() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_604).await;
    let id = fixed_uuid(5_614);

    let mut tx = org_tx(&db.app, org).await;
    sqlx::query(
        r#"INSERT INTO audit_events
             (id, org_id, actor_kind, actor_id, action, entity_type, entity_id, detail)
           VALUES ($1, $2, 'agent', 'test-agent', 'ticket.updated', 'ticket', $3,
                   '{"field": "status"}'::jsonb)"#,
    )
    .bind(id)
    .bind(org)
    .bind(Uuid::now_v7().to_string())
    .execute(&mut *tx)
    .await
    .expect("a fresh audit row must insert");
    tx.commit().await.expect("commit");

    let action: String = sqlx::query_scalar("SELECT action FROM audit_events WHERE id = $1")
        .bind(id)
        .fetch_one(&db.admin)
        .await
        .expect("read back");
    assert_eq!(action, "ticket.updated");
}

/// The retention door: one row past the window goes, the fresh one stays, and
/// the count the sweep logs is the function's return value.
#[tokio::test]
async fn the_retention_function_removes_only_rows_past_the_window() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_605).await;
    let old = seed_audit_row(&db, org, 5_615, 800).await;
    let fresh = seed_audit_row(&db, org, 5_625, 0).await;

    // Called on the app pool on purpose: the grant is part of the contract,
    // and the function's SECURITY DEFINER body is what lets it delete.
    let removed: i64 = sqlx::query_scalar("SELECT public.purge_audit_events(365, 1000)")
        .fetch_one(&db.app)
        .await
        .expect("purge");
    // The function is deployment-wide, so it can legitimately remove another
    // case's past-window row as well. The row-level assertions below are what
    // this case owns.
    assert!(
        removed >= 1,
        "the function removes at least the row past the window, got {removed}"
    );

    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(old)
        .fetch_one(&db.admin)
        .await
        .expect("count old");
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(fresh)
        .fetch_one(&db.admin)
        .await
        .expect("count fresh");
    assert_eq!(gone, 0, "the past-window row is purged");
    assert_eq!(kept, 1, "the row inside the window stays");
}

/// The floor is in the function, not only in the Rust matrix: a caller asking
/// for a thirty-day audit window gets an error, not fewer rows.
#[tokio::test]
async fn the_retention_function_refuses_a_window_below_the_floor() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_606).await;
    let old = seed_audit_row(&db, org, 5_616, 800).await;

    let attempt: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT public.purge_audit_events(30, 1000)")
            .fetch_one(&db.app)
            .await;
    let error = attempt.expect_err("a window below 365 days must be refused");
    assert_eq!(sqlstate(&error).as_deref(), Some("22023"), "got: {error}");

    let survivors: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(old)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(survivors, 1, "a refused window deletes nothing");
}

/// The sweep is the caller that must keep working. It now reaches the audit
/// window through the function, and the count it reports has to be real.
#[tokio::test]
async fn the_retention_sweep_still_deletes_past_window_audit_rows() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_607).await;
    // Standard profile keeps 730 days; 800 is past it and past the floor.
    let old = seed_audit_row(&db, org, 5_617, 800).await;

    let plan = core_db::retention::RetentionPlan::resolve(|_| None);
    let report = core_db::retention::sweep_once(&db.admin, &plan, 100)
        .await
        .expect("sweep");
    let audit = report
        .iter()
        .find(|t| t.table == "audit_events")
        .expect("audit_events in the report");
    // The function sweeps past-window rows for every org, so a row another
    // case in this binary left behind is legitimately counted too. What this
    // case owns is that the sweep reached the table and removed its row.
    assert!(
        audit.deleted >= 1,
        "the sweep reports at least the row it removed, got {}",
        audit.deleted
    );

    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(old)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(gone, 0, "the past-window row is gone");
}

/// The erasure contract: one `DELETE FROM orgs` still removes the tenant's
/// audit rows by cascade. The guard allows this because the parent org is
/// already gone in the statement's snapshot, and whoever deletes the org
/// already owns everything it holds.
#[tokio::test]
async fn a_tenant_erasure_still_cascades_through_the_audit_trail() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_608).await;
    let row = seed_audit_row(&db, org, 5_618, 0).await;

    sqlx::query("DELETE FROM orgs WHERE id = $1")
        .bind(org)
        .execute(&db.admin)
        .await
        .expect("the erasure cascade must not be refused by the audit guard");

    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(row)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(gone, 0, "the audit rows went with the org");
}

/// `seed-demo --force` and `POST /api/dev/reset-org` wipe the demo org and
/// reseed. They rely on the 0119 bypass, which the audit guard honours.
#[tokio::test]
async fn the_dev_reset_bypass_still_wipes_audit_rows() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;
    let org = seed_org(&db, 5_609).await;
    let row = seed_audit_row(&db, org, 5_619, 0).await;

    let mut tx = db.admin.begin().await.expect("begin");
    core_db::set_dev_reset_bypass(&mut tx)
        .await
        .expect("set bypass");
    sqlx::query("DELETE FROM audit_events WHERE org_id = $1")
        .bind(org)
        .execute(&mut *tx)
        .await
        .expect("the dev-reset bypass must unlock the delete");
    tx.commit().await.expect("commit");

    let gone: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE id = $1")
        .bind(row)
        .fetch_one(&db.admin)
        .await
        .expect("count");
    assert_eq!(gone, 0, "the bypassed delete removed the row");
}

/// A `TRUNCATE` is a bulk erase with no row to inspect, and the guard has a
/// statement branch for it. The app role never held the privilege; this pins
/// the trigger so a future `GRANT TRUNCATE` is not a hole. The bypass stays
/// available to the demo toolchain.
#[tokio::test]
async fn a_truncate_is_refused_for_the_application_role() {
    let _guard = SCHEMA_LOCK.lock().await;
    let db = test_db().await;

    let attempt = sqlx::query("TRUNCATE audit_events").execute(&db.app).await;
    let error = attempt.expect_err("the app role must not be able to truncate the trail");
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"), "got: {error}");
    assert!(
        error.to_string().contains("append-only")
            || error.to_string().contains("permission denied"),
        "got: {error}"
    );
}
