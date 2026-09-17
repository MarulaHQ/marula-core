//! core-db restricted-category owner/admin grant tests (Wave 16 lane 10,
//! migration 0083 + the runtime seeding twin, ADR 0006 directive).
//!
//! Purpose: prove the resolved product decision — deny-by-default STAYS
//! for members, but the org owner/admin is never locked out of their own
//! vault:
//!   * runtime seeding: a restricted folder arising at the folder write
//!     boundary ⇒ the owner AND admin builtin roles hold `can_read`+
//!     `can_edit` grant rows on that folder (companion rows — the 0078
//!     folder/seed/fork rows themselves untouched);
//!   * idempotence (backfill determinism): rerunning the seeding AND the
//!     byte-identical migration-0083 statement is a no-op (0 new rows);
//!   * the deny-by-default posture for MEMBER roles survives: the
//!     technician role has no grant and `folder_access` denies, while
//!     unrestricted chains stay matrix-gated as before;
//!   * the 0078 immutable-seed discipline is untouched (seed rows remain
//!     immutable through the mutate path's refusal) and the backfill
//!     replays clean over a wholly empty restricted set.
//!
//! Slot discipline: this suite owns fixed-UUID slots 8600–8639 only, one
//! decade per test (org = decade, owner = decade+1, folders keep the rest).
//! Non-purpose: HTTP routes (core-api e2e covers admin-reads/tech-denied
//! over the real router) and crypto/reveal surfaces.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use core_db::{orgs, secret_vault, users};
use core_domain::entities::SecretFolderKind;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const BACKFILL_SQL: &str =
    include_str!("../migrations/0083_restricted_category_owner_admin_grants.sql");

/// Seeds one org (roles + owner user) at a deterministic slot; the roles
/// map is keyed by builtin name ("owner" / "admin" / "technician").
#[allow(clippy::type_complexity)]
async fn seed_org_at(db: &TestDb, decade: u64) -> (Uuid, Uuid, HashMap<String, Uuid>) {
    let org = fixed_uuid(decade);
    let owner = fixed_uuid(decade + 1);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(
        &mut *tx,
        org,
        "Grant Co",
        &format!("grant-{decade}"),
        "GRNT",
    )
    .await
    .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    users::create_user(
        &mut *tx,
        owner,
        org,
        &format!("owner-{decade}@grant.test"),
        "Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("owner user");
    users::assign_role(&mut *tx, owner, roles["owner"])
        .await
        .expect("owner role");
    tx.commit().await.expect("commit");
    (org, owner, roles)
}

async fn make_restricted_folder(
    tx: &mut sqlx::PgConnection,
    org: Uuid,
    n: u64,
    slug: &str,
    actor: Uuid,
) -> Uuid {
    let folder_id = fixed_uuid(n);
    secret_vault::create_folder(
        tx,
        org,
        None,
        None,
        folder_id,
        slug,
        &slug.replace('-', " "),
        SecretFolderKind::ApiKeys,
        false,
        true,
        actor,
    )
    .await
    .expect("restricted folder create");
    folder_id
}

#[tokio::test]
async fn runtime_seeding_seats_owner_and_admin_grants_on_restricted_chains() {
    let db = test_db().await;
    let (org, owner, roles) = seed_org_at(&db, 8600).await;

    let mut tx = org_tx(&db.app, org).await;
    // A fresh org's plain tree carries ZERO grant rows (the 0078 defaults
    // are unrestricted — deny-by-default with no seeded grants until a
    // restricted chain exists).
    assert_eq!(
        core_db::secret_vault::list_grants(&mut tx, org)
            .await
            .expect("grants")
            .len(),
        0,
        "a fresh org ships with no category grant rows"
    );

    let folder_id = make_restricted_folder(&mut tx, org, 8604, "client-keys", owner).await;
    // The route write boundary seeds owner/admin access in the SAME tx;
    // the store helper is the seed primitive the routes call.
    assert_eq!(
        secret_vault::seed_admin_grants_for_restricted_folders(&mut tx, org, owner)
            .await
            .expect("seed"),
        2,
        "the write-boundary seed plants owner + admin grants"
    );
    let grants = secret_vault::list_grants(&mut tx, org)
        .await
        .expect("grants");
    assert_eq!(grants.len(), 2, "owner + admin grant rows only");
    for g in &grants {
        assert!(g.can_read && g.can_edit, "admin grants carry read AND edit");
    }
    let grantees: HashSet<Uuid> = grants.iter().map(|g| g.role_id).collect();
    assert!(
        grantees.contains(&roles["owner"]) && grantees.contains(&roles["admin"]),
        "exactly the owner/admin builtin roles are seated"
    );

    // The gate now ADMITS owner/admin on the restricted chain (read + edit).
    assert!(
        secret_vault::folder_access(&mut tx, org, &[roles["owner"]], folder_id, false)
            .await
            .expect("gate"),
        "owner reads the restricted chain"
    );
    assert!(
        secret_vault::folder_access(&mut tx, org, &[roles["admin"]], folder_id, true)
            .await
            .expect("gate"),
        "admin edits the restricted chain"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn member_roles_stay_denied_while_unrestricted_chains_ride_the_matrix() {
    let db = test_db().await;
    let (org, owner, roles) = seed_org_at(&db, 8610).await;
    let mut tx = org_tx(&db.app, org).await;
    let folder_id = make_restricted_folder(&mut tx, org, 8614, "client-keys", owner).await;
    // Technician: no grant seat ever for member roles.
    assert!(
        !secret_vault::folder_access(&mut tx, org, &[roles["technician"]], folder_id, false)
            .await
            .expect("gate"),
        "technician stays denied on the restricted chain"
    );
    assert!(
        !secret_vault::folder_access(&mut tx, org, &[roles["technician"]], folder_id, true)
            .await
            .expect("gate"),
        "technician stays denied on edit need"
    );
    // Unrestricted chains keep riding the existing matrix (no grants).
    let open_id = fixed_uuid(8615);
    secret_vault::create_folder(
        &mut tx,
        org,
        None,
        None,
        open_id,
        "general",
        "General",
        SecretFolderKind::Passwords,
        false,
        false,
        owner,
    )
    .await
    .expect("open folder create");
    assert!(
        secret_vault::folder_access(&mut tx, org, &[roles["technician"]], open_id, false)
            .await
            .expect("gate"),
        "unrestricted chains stay open through the matrix"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn backfill_is_idempotent_and_agrees_with_runtime_seeding() {
    let db = test_db().await;
    let (org, owner, roles) = seed_org_at(&db, 8620).await;
    let mut tx = org_tx(&db.app, org).await;
    let folder_id = make_restricted_folder(&mut tx, org, 8624, "client-keys", owner).await;
    // Runtime seeding first (org-creation twins the backfill): 2 rows.
    assert_eq!(
        secret_vault::seed_admin_grants_for_restricted_folders(&mut tx, org, owner)
            .await
            .expect("seed"),
        2,
        "runtime seeding plants owner + admin grants"
    );
    tx.commit().await.expect("commit");

    // The migration-0083 backfill then runs over the SAME state: row
    // count unchanged (it agrees with the runtime seeding on the action
    // set), and a bare rerun of the exact backfill statement inserts zero
    // rows from then on (backfill determinism).
    for _ in 0..2 {
        sqlx::query(BACKFILL_SQL)
            .execute(&db.admin)
            .await
            .expect("backfill rerun");
    }
    let mut tx = org_tx(&db.app, org).await;
    let grants = secret_vault::list_grants(&mut tx, org)
        .await
        .expect("grants");
    assert_eq!(grants.len(), 2, "backfill agrees and reruns no-op");
    assert!(
        secret_vault::folder_access(&mut tx, org, &[roles["admin"]], folder_id, false)
            .await
            .expect("gate"),
        "admin still reads after the backfill pass"
    );
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn seed_discipline_untouched_and_empty_backfill_clean() {
    let db = test_db().await;
    let (org, owner, _roles) = seed_org_at(&db, 8630).await;
    let mut tx = org_tx(&db.app, org).await;
    // Plant the immutable 0078 default tree the way org creation does
    // (then exercise the mutate path on a seeded default).
    secret_vault::seed_default_secret_folders(&mut tx, org, owner)
        .await
        .expect("default tree seeds");
    // The 0078 immutable-seed discipline is untouched by grant seeding:
    // mutating a seeded default is still refused (fork-only customization).
    let seeds = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    let seed_id = seeds
        .iter()
        .find(|f| f.is_default_seed && f.kind == SecretFolderKind::Passwords)
        .map(|f| f.id)
        .expect("seeded passwords folder");
    assert!(
        secret_vault::update_folder(
            &mut tx,
            org,
            seed_id,
            Some("Mutated"),
            Some("other"),
            Some(true),
            owner
        )
        .await
        .is_err(),
        "seed mutate path still refuses (ADR 0004 fork-only)"
    );
    tx.commit().await.expect("commit");

    // The backfill on a WHOLLY EMPTY restricted set (no restricted chains
    // at all): the SELECT finds nothing, ON CONFLICT stays silent — the
    // migration must replay clean on every fresh org tree.
    sqlx::query(BACKFILL_SQL)
        .execute(&db.admin)
        .await
        .expect("backfill on migrated schema");
}
