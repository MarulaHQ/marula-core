//! core-db settings-registry query-layer tests (item 2.5).
//!
//! Purpose: prove override upserts are total-key idempotent, effective
//! reads overlay defaults at the API layer correctly, user-scope rows are
//! isolated per owner by BOTH the explicit predicate and the RLS policy,
//! and the table's shape/JSONB backstops reject malformed rows.
//!
//! Slot discipline: this suite owns seed slots 70–79 only; every test
//! claims a DISJOINT base so parallel tests sharing one migrated database
//! never collide on fixed UUIDs.
//!
//! Non-purpose: registry validation rules (core-domain unit tests) and
//! HTTP behavior (core-api flow test). Database selection follows the
//! documented contract in `tests/common/mod.rs`.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{orgs, set_org_and_user, settings as settings_db, users};
use serde_json::json;
use uuid::Uuid;

/// Seeds one org (defaults-first roles) at slot `n`.
async fn seed_org_at(db: &common::TestDb, n: u64, slug: &str) -> Uuid {
    let org = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Settings Co", slug, "SET")
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
async fn org_override_upsert_is_idempotent_and_readable() {
    // Slots 70 (org), 71 (actor).
    let db = test_db().await;
    let org = seed_org_at(&db, 70, "set-co-70").await;
    let actor = seed_user_at(&db, org, 71, "a@set70.test").await;

    // Two writes to the same key collapse onto one row.
    for value in [json!(17), json!(20)] {
        let mut tx = org_tx(&db.app, org).await;
        settings_db::upsert_org_setting(&mut tx, org, "general.eod_hour", &value, actor)
            .await
            .expect("upsert");
        tx.commit().await.expect("commit");
    }
    // Reads ride an org-scoped transaction, exactly like production
    // requests; an UNscoped session default-denies (see the RLS test).
    let mut tx = org_tx(&db.app, org).await;
    let overrides = settings_db::list_org_overrides(&mut *tx, org)
        .await
        .expect("list");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].key, "general.eod_hour");
    assert_eq!(overrides[0].value, json!(20));
}

#[tokio::test]
async fn user_overrides_are_scoped_per_owner() {
    // Slots 72 (org), 73 (user A), 74 (user B).
    let db = test_db().await;
    let org = seed_org_at(&db, 72, "set-co-72").await;
    let user_a = seed_user_at(&db, org, 73, "a@set72.test").await;
    let user_b = seed_user_at(&db, org, 74, "b@set72.test").await;

    // Production writes user rows through transactions bound to BOTH the
    // org and the actor (set_org_and_user, as org_tx::begin does).
    let mut tx = org_tx(&db.app, org).await;
    set_org_and_user(&mut tx, org, user_a)
        .await
        .expect("scope A");
    settings_db::upsert_user_setting(&mut tx, org, user_a, "appearance.theme", &json!("dark"))
        .await
        .expect("upsert A");
    tx.commit().await.expect("commit");

    // B sees none of A's overrides…
    let mut tx = org_tx(&db.app, org).await;
    set_org_and_user(&mut tx, org, user_b)
        .await
        .expect("scope B");
    let bs = settings_db::list_user_overrides(&mut *tx, org, user_b)
        .await
        .expect("list B");
    assert!(bs.is_empty());
    tx.rollback().await.expect("rollback");

    // …and A sees exactly their own.
    let mut tx = org_tx(&db.app, org).await;
    set_org_and_user(&mut tx, org, user_a)
        .await
        .expect("scope A");
    let as_rows = settings_db::list_user_overrides(&mut *tx, org, user_a)
        .await
        .expect("list A");
    assert_eq!(as_rows.len(), 1);
    assert_eq!(as_rows[0].value, json!("dark"));
}

#[tokio::test]
async fn rls_hides_cross_org_and_cross_user_rows() {
    // Slots 75 (org), 76 (its user), 77 (second org), 78 (foreign user).
    let db = test_db().await;
    let org = seed_org_at(&db, 75, "set-co-75").await;
    let user_a = seed_user_at(&db, org, 76, "a@set75.test").await;
    let org2 = seed_org_at(&db, 77, "set-co-77").await;
    let _foreign = seed_user_at(&db, org2, 78, "x@set77.test").await;

    // Seed rows through the ADMIN pool (owner role bypasses RLS) so the
    // assertions below exercise policy filtering, not absence of data.
    sqlx::query(
        r#"INSERT INTO settings (id, org_id, scope, key, owner_id, value, updated_by)
           VALUES ($1,$2,'org','general.eod_hour',$2,'21',$3),
                  ($4,$2,'user','appearance.theme',$5,'"dark"',$5)"#,
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(user_a)
    .bind(Uuid::now_v7())
    .bind(user_a)
    .execute(&db.admin)
    .await
    .expect("admin seed");

    // Org-scoped transaction WITHOUT app.user_id: org rows visible,
    // sibling user rows NOT.
    let mut tx = org_tx(&db.app, org).await;
    let org_rows = settings_db::list_org_overrides(&mut *tx, org)
        .await
        .expect("org read");
    assert_eq!(org_rows.len(), 1);
    let user_rows = settings_db::list_user_overrides(&mut *tx, org, user_a)
        .await
        .expect("user read under org-only scope");
    assert!(
        user_rows.is_empty(),
        "user rows must require an app.user_id scope"
    );
    tx.rollback().await.expect("rollback");

    // With the actor bound, the same read succeeds.
    let mut tx = db.app.begin().await.expect("begin");
    set_org_and_user(&mut tx, org, user_a).await.expect("scope");
    let user_rows = settings_db::list_user_overrides(&mut *tx, org, user_a)
        .await
        .expect("scoped user read");
    assert_eq!(user_rows.len(), 1);

    // Cross-org reads return nothing even with the actor fully bound.
    let cross = settings_db::list_org_overrides(&mut *tx, org2)
        .await
        .expect("cross read");
    assert!(cross.is_empty());
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn shape_and_jsonb_backstops_reject_bad_rows() {
    // Slot 79 (org); updated_by stays NULL (column is nullable).
    let db = test_db().await;
    let org = seed_org_at(&db, 79, "set-co-79").await;

    // A user-scope row owned BY THE ORG violates settings_owner_shape.
    let bad = sqlx::query(
        r#"INSERT INTO settings (id, org_id, scope, key, owner_id, value)
           VALUES ($1,$2,'user','appearance.theme',$2,'"light"')"#,
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .execute(&db.admin)
    .await;
    match bad {
        Ok(_) => panic!("owner=org on a user row must violate CHECK"),
        Err(sqlx::Error::Database(db_err)) => {
            assert_eq!(
                db_err.constraint(),
                Some("settings_owner_shape"),
                "failure must be the shape CHECK, got {db_err}"
            );
        }
        Err(e) => panic!("unexpected error: {e}"),
    }

    // JSONB arrays are outside the registry's value vocabulary.
    let bad_json = sqlx::query(
        r#"INSERT INTO settings (id, org_id, scope, key, owner_id, value)
           VALUES ($1,$2,'org','general.eod_hour',$2,'[1,2]')"#,
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .execute(&db.admin)
    .await;
    match bad_json {
        Ok(_) => panic!("non-scalar JSONB must be rejected"),
        Err(sqlx::Error::Database(db_err)) => {
            assert_eq!(
                db_err.constraint(),
                Some("settings_value_check"),
                "failure must be the JSONB type backstop, got {db_err}"
            );
        }
        Err(e) => panic!("unexpected error: {e}"),
    }
}
