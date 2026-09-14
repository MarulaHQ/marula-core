//! core-db scorecard-settings query-layer tests (Wave 14 lane 3, R5).
//!
//! Purpose: prove the opt-in gamification toggle upserts idempotently,
//! absent rows read as the private-by-default (`false`), and the RLS
//! policy scopes rows to the bound org — an UNscoped session sees zero
//! rows (default deny), the RLS security property.
//!
//! Slot discipline: this suite owns fixed-UUID slots 7500–7519 only; every
//! test claims a DISJOINT base so parallel tests sharing one migrated
//! database never collide.
//!
//! Non-purpose: the aggregation queries (read-model view semantics,
//! covered by `ops_read_model.rs` + the core-api e2e flow) and HTTP
//! behavior (core-api flow test). Database selection follows the
//! documented contract in `tests/common/mod.rs`.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{orgs, scorecards, users};
use uuid::Uuid;

/// Seeds one org (defaults-first roles) at slot `n`.
async fn seed_org_at(db: &common::TestDb, n: u64, slug: &str) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Scorecards Co", slug, "SCORE")
        .await
        .expect("org insert");
    orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    tx.commit().await.expect("commit org");
    org
}

/// Seeds one user of `org` at slot `n`.
async fn seed_user_at(db: &common::TestDb, org: Uuid, n: u64, email: &str) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    users::create_user(&mut *tx, user, org, email, "Test User", "argon2hash", "UTC")
        .await
        .expect("user insert");
    tx.commit().await.expect("commit user");
    user
}

#[tokio::test]
async fn settings_absent_reads_default_and_upsert_is_idempotent() {
    // Slots 7500 (org), 7501 (member).
    let db = test_db().await;
    let org = seed_org_at(&db, 7500, "score-co-7500").await;
    let member = seed_user_at(&db, org, 7501, "a@score7500.test").await;

    // Absent row reads as the default: gamification OFF.
    let mut tx = org_tx(&db.app, org).await;
    let settings = scorecards::settings::get(&mut *tx, org, member)
        .await
        .expect("default read");
    assert!(!settings.gamification_opt_in, "private-by-default");

    // Two upserts collapse onto one row; the last value wins.
    for opt_in in [true, false] {
        let stored = scorecards::settings::set_gamification(
            &mut *tx,
            org,
            member,
            opt_in,
            chrono::Utc::now(),
        )
        .await
        .expect("upsert");
        assert_eq!(stored.gamification_opt_in, opt_in);
    }
    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM scorecard_settings WHERE org_id = $1 AND user_id = $2",
    )
    .bind(org)
    .bind(member)
    .fetch_one(&mut *tx)
    .await
    .expect("count");
    assert_eq!(count.0, 1, "upsert must be idempotent");
    let settings = scorecards::settings::get(&mut *tx, org, member)
        .await
        .expect("read back");
    assert!(!settings.gamification_opt_in, "last write wins");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn settings_rls_scopes_to_org_and_defaults_deny_unscoped() {
    // Slots 7510 (org A), 7511 (member A), 7512 (org B).
    let db = test_db().await;
    let org_a = seed_org_at(&db, 7510, "score-co-7510").await;
    let member_a = seed_user_at(&db, org_a, 7511, "a@score7510.test").await;
    let org_b = seed_org_at(&db, 7512, "score-co-7512").await;

    {
        let mut tx = org_tx(&db.app, org_a).await;
        scorecards::settings::set_gamification(&mut *tx, org_a, member_a, true, chrono::Utc::now())
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");
    }

    // Org B's binding sees ZERO of org A's rows.
    let mut tx_b = org_tx(&db.app, org_b).await;
    let foreign = scorecards::settings::get(&mut *tx_b, org_a, member_a)
        .await
        .expect("cross-org read");
    assert!(
        !foreign.gamification_opt_in,
        "cross-org read must see the DEFAULT, not org A's row (RLS filtered it)"
    );
    tx_b.commit().await.expect("commit");

    // An UNscoped session default-denies: no app.org_id binding, no rows.
    // A VIRGIN pool is required because Postgres keeps custom GUCs on the
    // connection after COMMIT — a recycled connection would carry a stale
    // `app.org_id = ''` binding.
    let fresh = db.fresh_app_pool().await;
    let unscoped = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM scorecard_settings")
        .fetch_one(&fresh)
        .await
        .expect("unscoped count");
    assert_eq!(
        unscoped, 0,
        "unscoped sessions see zero rows (default deny)"
    );
}
