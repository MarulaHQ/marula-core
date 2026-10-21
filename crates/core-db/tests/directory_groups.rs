//! Directory-group storage and the effective-role union (Wave 52 lane R10).
//!
//! These cases pin the four promises migration 0148 makes:
//!
//! 1. **The union is additive.** A principal holds the roles assigned directly
//!    AND the roles their groups carry, with no duplicate when both paths reach
//!    the same role.
//! 2. **Removal revokes exactly the group's roles.** Leaving a group takes away
//!    what that group alone granted and nothing else; roles held directly
//!    survive, and so do the roles of another group.
//! 3. **Orgs are isolated.** A group, its members and its mappings are invisible
//!    from another org, and a cross-org membership cannot be written.
//! 4. **All three tables carry RLS.** An unscoped session sees nothing and a
//!    foreign-scope session sees nothing, which is what makes the union read
//!    safe on any executor.
//!
//! Non-purpose: the HTTP surface (core-api tests own request shapes) and the
//! SCIM protocol, which is not core.

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::directory_groups::{
    add_member, add_role, delete_group, get_group, insert_group, list_group_roles, list_groups,
    list_members, remove_member, remove_role, NewDirectoryGroup,
};
use core_db::{orgs, users};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

/// One seeded org: its id and the preset role ids keyed by builtin tag.
struct Seeded {
    org: Uuid,
    roles: HashMap<String, Uuid>,
}

async fn seed_org(app: &PgPool, n: u64) -> Seeded {
    let org = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    orgs::insert_org(&mut *tx, org, "Acme MSP", &format!("acme-{n}"), "MAR")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org)
        .await
        .expect("role seeds");
    tx.commit().await.expect("commit seed");
    Seeded { org, roles }
}

async fn seed_user(app: &PgPool, org: Uuid, n: u64) -> Uuid {
    let id = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    users::create_user(
        &mut *tx,
        id,
        org,
        &format!("user{n}@acme.test"),
        &format!("User {n}"),
        "argon2hash",
        "UTC",
    )
    .await
    .expect("create user");
    tx.commit().await.expect("commit user");
    id
}

async fn seed_group(app: &PgPool, org: Uuid, n: u64, name: &str) -> Uuid {
    let id = fixed_uuid(n);
    let mut tx = org_tx(app, org).await;
    insert_group(
        &mut tx,
        &NewDirectoryGroup {
            id,
            org_id: org,
            name,
            description: Some("seeded"),
            external_id: None,
        },
    )
    .await
    .expect("insert group");
    tx.commit().await.expect("commit group");
    id
}

async fn effective_role_names(app: &PgPool, org: Uuid, user: Uuid) -> Vec<String> {
    let mut tx = org_tx(app, org).await;
    let roles = users::effective_roles_for_user(&mut *tx, user, org)
        .await
        .expect("effective roles");
    tx.commit().await.expect("commit read");
    roles.into_iter().map(|r| r.name).collect()
}

// --- 1. the union ------------------------------------------------------------

#[tokio::test]
async fn effective_roles_union_direct_and_group_grants_without_duplicates() {
    let db = test_db().await;
    let s = seed_org(&db.app, 700).await;
    let user = seed_user(&db.app, s.org, 7001).await;
    let group = seed_group(&db.app, s.org, 7002, "Tier 2 Support").await;

    {
        let mut tx = org_tx(&db.app, s.org).await;
        users::assign_role(&mut *tx, user, s.roles["technician"])
            .await
            .expect("direct role");
        add_member(&mut tx, s.org, group, user, None)
            .await
            .expect("membership");
        add_role(&mut tx, s.org, group, s.roles["dispatcher"], None)
            .await
            .expect("group role");
        add_role(&mut tx, s.org, group, s.roles["bookkeeper"], None)
            .await
            .expect("group role");
        // The same role through both paths: it must appear once.
        add_role(&mut tx, s.org, group, s.roles["technician"], None)
            .await
            .expect("group role overlapping the direct grant");
        tx.commit().await.expect("commit");
    }

    let mut names = effective_role_names(&db.app, s.org, user).await;
    names.sort();
    assert_eq!(
        names,
        vec![
            "Bookkeeper".to_string(),
            "Dispatcher".to_string(),
            "Technician".to_string()
        ],
        "the union carries both sources, and the shared role appears once"
    );

    // The editorial read is untouched: exactly the direct grant.
    let mut tx = org_tx(&db.app, s.org).await;
    let direct = users::direct_roles_for_user(&mut *tx, user, s.org)
        .await
        .expect("direct roles");
    tx.commit().await.expect("commit");
    assert_eq!(direct.len(), 1);
    assert_eq!(direct[0].name, "Technician");
}

// --- 2. revocation is exact ---------------------------------------------------

#[tokio::test]
async fn removing_a_membership_revokes_exactly_the_groups_roles() {
    let db = test_db().await;
    let s = seed_org(&db.app, 710).await;
    let user = seed_user(&db.app, s.org, 7101).await;
    let tier_two = seed_group(&db.app, s.org, 7102, "Tier 2 Support").await;
    let on_call = seed_group(&db.app, s.org, 7103, "On Call").await;

    {
        let mut tx = org_tx(&db.app, s.org).await;
        users::assign_role(&mut *tx, user, s.roles["technician"])
            .await
            .expect("direct role");
        add_member(&mut tx, s.org, tier_two, user, None)
            .await
            .expect("membership tier two");
        add_member(&mut tx, s.org, on_call, user, None)
            .await
            .expect("membership on call");
        add_role(&mut tx, s.org, tier_two, s.roles["dispatcher"], None)
            .await
            .expect("tier two role");
        add_role(&mut tx, s.org, tier_two, s.roles["bookkeeper"], None)
            .await
            .expect("tier two role");
        add_role(&mut tx, s.org, on_call, s.roles["auditor"], None)
            .await
            .expect("on call role");
        tx.commit().await.expect("commit");
    }

    {
        let mut tx = org_tx(&db.app, s.org).await;
        remove_member(&mut tx, s.org, tier_two, user)
            .await
            .expect("leave tier two");
        tx.commit().await.expect("commit");
    }

    // Leaving Tier 2 drops dispatcher + bookkeeper. Technician is direct, and
    // Auditor comes from the other group, so both survive.
    let mut names = effective_role_names(&db.app, s.org, user).await;
    names.sort();
    assert_eq!(
        names,
        vec!["Auditor".to_string(), "Technician".to_string()],
        "only the departed group's roles are gone"
    );

    // Re-adding restores them: the same union read in the other direction, with
    // nothing cached or flattened along the way.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        add_member(&mut tx, s.org, tier_two, user, None)
            .await
            .expect("rejoin");
        tx.commit().await.expect("commit");
    }
    let mut names = effective_role_names(&db.app, s.org, user).await;
    names.sort();
    assert_eq!(
        names,
        vec![
            "Auditor".to_string(),
            "Bookkeeper".to_string(),
            "Dispatcher".to_string(),
            "Technician".to_string()
        ]
    );
}

#[tokio::test]
async fn unmapping_or_deleting_a_group_revokes_without_touching_direct_roles() {
    let db = test_db().await;
    let s = seed_org(&db.app, 720).await;
    let user = seed_user(&db.app, s.org, 7201).await;
    let group = seed_group(&db.app, s.org, 7202, "Tier 3").await;

    {
        let mut tx = org_tx(&db.app, s.org).await;
        users::assign_role(&mut *tx, user, s.roles["technician"])
            .await
            .expect("direct role");
        add_member(&mut tx, s.org, group, user, None)
            .await
            .expect("membership");
        add_role(&mut tx, s.org, group, s.roles["admin"], None)
            .await
            .expect("group role");
        tx.commit().await.expect("commit");
    }

    // One role off the group: the direct grant is untouched.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        remove_role(&mut tx, s.org, group, s.roles["admin"])
            .await
            .expect("unmap");
        tx.commit().await.expect("commit");
    }
    assert_eq!(
        effective_role_names(&db.app, s.org, user).await,
        vec!["Technician".to_string()]
    );

    // Delete the group: membership rows cascade, and the direct grant survives.
    {
        let mut tx = org_tx(&db.app, s.org).await;
        delete_group(&mut tx, s.org, group).await.expect("delete");
        tx.commit().await.expect("commit");
    }
    assert_eq!(
        effective_role_names(&db.app, s.org, user).await,
        vec!["Technician".to_string()]
    );

    let mut tx = org_tx(&db.app, s.org).await;
    assert!(
        get_group(&mut *tx, s.org, group).await.is_err(),
        "the group row is gone"
    );
    assert!(
        list_members(&mut *tx, s.org, group)
            .await
            .expect("members read")
            .is_empty(),
        "membership rows cascaded"
    );
    tx.commit().await.expect("commit");
}

// --- 3. cross-org isolation ---------------------------------------------------

#[tokio::test]
async fn groups_cannot_cross_an_org_boundary() {
    let db = test_db().await;
    let a = seed_org(&db.app, 730).await;
    let b = seed_org(&db.app, 731).await;
    let user_a = seed_user(&db.app, a.org, 7301).await;
    let user_b = seed_user(&db.app, b.org, 7302).await;
    let group_a = seed_group(&db.app, a.org, 7303, "Tier 2").await;

    {
        let mut tx = org_tx(&db.app, a.org).await;
        add_member(&mut tx, a.org, group_a, user_a, None)
            .await
            .expect("own membership");
        add_role(&mut tx, a.org, group_a, b.roles["admin"], None)
            .await
            .expect("a foreign role id adds nothing");
        tx.commit().await.expect("commit");
    }

    // A foreign role mapping wrote nothing: the group carries no roles.
    {
        let mut tx = org_tx(&db.app, a.org).await;
        assert!(
            list_group_roles(&mut *tx, a.org, group_a)
                .await
                .expect("roles read")
                .is_empty(),
            "a role from another org is not mappable"
        );
        tx.commit().await.expect("commit");
    }

    // Org B cannot see org A's group, and cannot add its own user to it.
    {
        let mut tx = org_tx(&db.app, b.org).await;
        assert!(
            get_group(&mut *tx, b.org, group_a).await.is_err(),
            "org B cannot read org A's group"
        );
        assert!(
            list_groups(&mut *tx, b.org).await.expect("list").is_empty(),
            "org B's listing is empty"
        );
        add_member(&mut tx, b.org, group_a, user_b, None)
            .await
            .expect("cross-org add writes nothing");
        tx.commit().await.expect("commit");
    }
    {
        let mut tx = org_tx(&db.app, b.org).await;
        assert!(
            list_members(&mut *tx, b.org, group_a)
                .await
                .expect("members read")
                .is_empty(),
            "nothing was written across the boundary"
        );
        tx.commit().await.expect("commit");
    }

    assert!(
        effective_role_names(&db.app, b.org, user_b)
            .await
            .is_empty(),
        "org B's user gained nothing"
    );
}

// --- 4. RLS on all three tables ----------------------------------------------

#[tokio::test]
async fn rls_hides_all_three_tables_without_scope_and_across_orgs() {
    let db = test_db().await;
    let a = seed_org(&db.app, 740).await;
    let b = seed_org(&db.app, 741).await;
    let user_a = seed_user(&db.app, a.org, 7401).await;
    let group_a = seed_group(&db.app, a.org, 7402, "Tier 2").await;

    {
        let mut tx = org_tx(&db.app, a.org).await;
        add_member(&mut tx, a.org, group_a, user_a, None)
            .await
            .expect("membership");
        add_role(&mut tx, a.org, group_a, a.roles["senior_tech"], None)
            .await
            .expect("role mapping");
        tx.commit().await.expect("commit");
    }

    // A virgin app pool: `app.org_id` was never set on these sessions, which is
    // the true no-scope state the default-deny policies are written for.
    let virgin = db.fresh_app_pool().await;
    assert_eq!(
        unscoped_counts(&virgin).await,
        (0, 0, 0),
        "no scope: groups, memberships and mappings all default-deny"
    );

    // A foreign scope sees nothing either — including the child tables, whose
    // policy infers the org from the group rather than from a column.
    let mut tx = org_tx(&db.app, b.org).await;
    let foreign = scoped_counts(&mut tx, b.org).await;
    tx.commit().await.expect("commit");
    assert_eq!(foreign, (0, 0, 0), "foreign scope sees nothing");

    // The owning org sees its own rows, so the zeroes above are scope and not
    // an empty database.
    let mut tx = org_tx(&db.app, a.org).await;
    let own = scoped_counts(&mut tx, a.org).await;
    tx.commit().await.expect("commit");
    assert_eq!(own, (1, 1, 1));
}

async fn unscoped_counts(pool: &PgPool) -> (i64, i64, i64) {
    let groups: i64 = sqlx::query_scalar("SELECT count(*) FROM directory_groups")
        .fetch_one(pool)
        .await
        .expect("groups count");
    let members: i64 = sqlx::query_scalar("SELECT count(*) FROM directory_group_members")
        .fetch_one(pool)
        .await
        .expect("members count");
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM directory_group_roles")
        .fetch_one(pool)
        .await
        .expect("roles count");
    (groups, members, roles)
}

async fn scoped_counts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    org: Uuid,
) -> (i64, i64, i64) {
    let groups: i64 = sqlx::query_scalar("SELECT count(*) FROM directory_groups WHERE org_id = $1")
        .bind(org)
        .fetch_one(&mut **tx)
        .await
        .expect("groups count");
    let members: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM directory_group_members m \
                            JOIN directory_groups g ON g.id = m.group_id WHERE g.org_id = $1",
    )
    .bind(org)
    .fetch_one(&mut **tx)
    .await
    .expect("members count");
    let roles: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM directory_group_roles dgr \
                            JOIN directory_groups g ON g.id = dgr.group_id WHERE g.org_id = $1",
    )
    .bind(org)
    .fetch_one(&mut **tx)
    .await
    .expect("roles count");
    (groups, members, roles)
}
