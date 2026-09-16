//! core-db native-vault query-layer tests (Wave 16 lane 3, migration
//! 0078, ADR 0006 + ADR 0004 template doctrine).
//!
//! Purpose: prove the secret CATEGORY TREE contract —
//!   * the 5 seeded defaults exist per org as immutable seed rows
//!     (`is_default_seed`): update/delete refused, customization forks
//!     (`fork_folder`) into client scope with `forked_from` provenance —
//!     never edit-in-place;
//!   * sibling slug uniqueness per (scope, parent) — `secret_folders_scope_key`
//!     violation typed as the raw unique error (the API registry maps it
//!     to `folder_slug_taken`);
//!   * SecretRecord generalization beside PasswordEntry: record CRUD
//!     with SEALED field bytes (plaintext never crosses this module),
//!     folder-scope equality enforcement, metadata-only field views;
//!   * the deterministic forward importer: shipped `passwords` rows
//!     (org- and client-level) surface as `secret_records` mapped into
//!     the right default/forked folders — no destructive cutover;
//!   * the CATEGORY-LEVEL RBAC deny-by-default gate: unrestricted
//!     chains allow (the matrix already gated), restricted chains deny
//!     without a passing `secret_category_grants` row;
//!   * the RLS security property on every new table (org AND client
//!     scope rows invisible cross-org, unscoped sees zero).
//!
//! Slot discipline: this suite owns fixed-UUID slots 7800–7839 only.
//! Non-purpose: HTTP/routes (core-api e2e lane) and the reveal crypto
//! (core-api audited endpoint tests).

mod common;

use common::{fixed_uuid, org_tx, test_db};
use core_db::{orgs, passwords, secret_vault, users};
use std::collections::HashMap;
use uuid::Uuid;

/// Seeds one org with default roles + the 5 seeded secret folders.
async fn seed_org_at(db: &common::TestDb, n: u64, slug: &str) -> (Uuid, HashMap<String, Uuid>) {
    let org = fixed_uuid(n);
    let owner = fixed_uuid(n + 40);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Vault Co", slug, "VAULT")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .unwrap();
    users::create_user(
        &mut *tx,
        owner,
        org,
        &format!("owner-{n}@vault.test"),
        "Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("owner user");
    users::assign_role(&mut *tx, owner, owner_role)
        .await
        .expect("owner role");
    secret_vault::seed_default_secret_folders(&mut tx, org, owner)
        .await
        .expect("seed folders");
    tx.commit().await.expect("commit org");
    (org, roles)
}

/// Seeds one user of `org` holding `role_id` at slot `n`.
async fn seed_user_at(db: &common::TestDb, org: Uuid, n: u64, email: &str, role_id: Uuid) -> Uuid {
    let user = fixed_uuid(n);
    let mut tx = org_tx(&db.app, org).await;
    users::create_user(
        &mut *tx,
        user,
        org,
        email,
        "Vault User",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user insert");
    users::assign_role(&mut *tx, user, role_id)
        .await
        .expect("role assign");
    tx.commit().await.expect("commit user");
    user
}

#[tokio::test]
async fn seeded_defaults_are_five_immutable_folders() {
    // Slots 7800 (org), 7801 (member).
    let db = test_db().await;
    let (org, _roles) = seed_org_at(&db, 7800, "vault-co-7800").await;
    let _owner = fixed_uuid(7800 + 40);
    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("seed folders");
    assert_eq!(folders.len(), 5, "exactly the 5 seeded defaults");
    for slug in [
        "passwords",
        "api-keys",
        "certificates",
        "recovery-codes",
        "documents",
    ] {
        let folder = folders
            .iter()
            .find(|f| f.slug == slug)
            .expect("seeded slug");
        assert!(folder.is_default_seed, "{slug} is an immutable seed row");
        assert_eq!(folder.client_id, None, "{slug} is org-scope");
        assert_eq!(folder.org_id, Some(org), "{slug} anchored to the org");
    }
}

#[tokio::test]
async fn folder_create_unique_per_scope_and_parent_scope_mismatch_refused() {
    // Slots 7802 (org), 7803 (member).
    let db = test_db().await;
    let (org, _roles) = seed_org_at(&db, 7802, "vault-co-7802").await;
    let owner = fixed_uuid(7802 + 40);
    let mut tx = org_tx(&db.app, org).await;
    let folder = secret_vault::create_folder(
        &mut tx,
        org,
        None,
        None,
        fixed_uuid(7804),
        "hunter-tools",
        "Hunter Tools",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        false,
        owner,
    )
    .await
    .expect("user folder create");
    assert_eq!(folder.slug, "hunter-tools");

    // Unique (parent, slug) per scope: the same slug is refused.
    let dup = secret_vault::create_folder(
        &mut tx,
        org,
        None,
        None,
        fixed_uuid(7805),
        "hunter-tools",
        "Duplicate",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        false,
        owner,
    )
    .await;
    match dup {
        Err(core_db::DbError::Sql(e)) => {
            let kind = e
                .as_database_error()
                .and_then(|d| d.constraint())
                .unwrap_or("")
                .to_string();
            assert_eq!(
                kind, "secret_folders_scope_key",
                "raw unique name for the registry"
            );
        }
        other => panic!("expected unique violation, got {other:?}"),
    }

    // A client-scope folder cannot hang off an org-scope parent.
    let client_uuid = fixed_uuid(7806);
    let mut ctx_tx = org_tx(&db.app, org).await;
    core_db::clients::create_client(&mut *ctx_tx, org, client_uuid, "Vault Client", None)
        .await
        .expect("client seeded");
    ctx_tx.commit().await.expect("commit client");
    let mut client_tx = org_tx(&db.app, org).await;
    // The org-scope SEEDED folder cannot parent a client-scope child.
    let mut scope_tx = org_tx(&db.app, org).await;
    let org_seed = secret_vault::list_org_folders(&mut scope_tx, org)
        .await
        .expect("seeded folders")
        .into_iter()
        .find(|f| f.slug == "passwords")
        .expect("seeded passwords folder");
    let bad = secret_vault::create_folder(
        &mut client_tx,
        org,
        Some(client_uuid),
        Some(org_seed.id),
        fixed_uuid(7808),
        "child",
        "Child",
        core_domain::entities::SecretFolderKind::Documents,
        false,
        false,
        owner,
    )
    .await;
    assert!(
        matches!(bad, Err(core_db::DbError::Conflict(_))),
        "parent must live in the SAME scope"
    );
}

#[tokio::test]
async fn seed_rows_refuse_update_and_delete_fork_instead() {
    // Slots 7809 (org), 7810 (member).
    let db = test_db().await;
    let (org, _roles) = seed_org_at(&db, 7809, "vault-co-7809").await;
    let owner = fixed_uuid(7809 + 40);
    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    let seed = folders
        .iter()
        .find(|f| f.slug == "passwords")
        .cloned()
        .unwrap();
    // ADR 0004 immutable-seed refusal on BOTH write paths.
    let upd =
        secret_vault::update_folder(&mut tx, org, seed.id, Some("Renamed"), None, None, org).await;
    assert!(
        matches!(upd, Err(core_db::DbError::Conflict(_))),
        "seed refuses update"
    );
    let del = secret_vault::delete_folder(&mut tx, org, seed.id).await;
    assert!(
        matches!(del, Err(core_db::DbError::Conflict(_))),
        "seed refuses delete"
    );

    // User-added folders are ordinary rows.
    let custom = secret_vault::create_folder(
        &mut tx,
        org,
        None,
        None,
        fixed_uuid(7811),
        "custom",
        "Custom Folder",
        core_domain::entities::SecretFolderKind::Documents,
        false,
        false,
        owner,
    )
    .await
    .expect("custom folder");
    let up = secret_vault::update_folder(
        &mut tx,
        org,
        custom.id,
        Some("Renamed Folder"),
        None,
        Some(true),
        owner,
    )
    .await
    .expect("custom folder update");
    assert_eq!(up.label, "Renamed Folder");
    assert!(up.restricted, "restricted flag flips");
    secret_vault::delete_folder(&mut tx, org, custom.id)
        .await
        .expect("custom delete");
}

#[tokio::test]
async fn fork_copies_subtree_into_client_scope_with_provenance() {
    // Slots 7812 (org), 7813 (member), 7814 (client).
    let db = test_db().await;
    let (org, _roles) = seed_org_at(&db, 7812, "vault-co-7812").await;
    let owner = fixed_uuid(7812 + 40);
    let client_uuid = fixed_uuid(7814);
    let mut tx = org_tx(&db.app, org).await;
    core_db::clients::create_client(&mut *tx, org, client_uuid, "Fork Client", None)
        .await
        .expect("client seeded");
    tx.commit().await.expect("commit");

    // Build a two-level org tree: root + one child.
    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    let seed = folders
        .iter()
        .find(|f| f.slug == "api-keys")
        .cloned()
        .unwrap();
    let _child = secret_vault::create_folder(
        &mut tx,
        org,
        None,
        Some(seed.id),
        fixed_uuid(7815),
        "child",
        "Child",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        false,
        owner,
    )
    .await
    .expect("child folder");
    tx.commit().await.expect("commit");

    // Fork-the-seed: deep-copies into the client scope (new row graph).
    let mut tx = org_tx(&db.app, org).await;
    let created = secret_vault::fork_folder(&mut tx, org, seed.id, client_uuid, owner)
        .await
        .expect("fork");
    assert_eq!(created.len(), 2, "root + child copied");
    let fork_root = &created[0];
    assert_eq!(
        fork_root.client_id,
        Some(client_uuid),
        "fork lands in client scope"
    );
    assert_eq!(
        fork_root.forked_from,
        Some(seed.id),
        "root carries provenance"
    );
    assert!(!fork_root.is_default_seed, "fork is NOT the seed");
    // The CHANGED graph is ordinary data; forking the seed again is a
    // fresh graph (slug reused but different scope bucket).
    let again = secret_vault::fork_folder(&mut tx, org, seed.id, client_uuid, owner).await;
    assert!(
        matches!(again, Err(core_db::DbError::Sql(_))),
        "re-forking the same slug into the SAME client scope is the typed unique conflict"
    );
}

#[tokio::test]
async fn records_crud_with_sealed_fields_and_scope_equality() {
    // Slots 7816 (org), 7817 (member), 7818 (client).
    let db = test_db().await;
    let (org, _roles) = seed_org_at(&db, 7816, "vault-co-7816").await;
    let owner = fixed_uuid(7816 + 40);
    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    let seed = folders
        .iter()
        .find(|f| f.slug == "passwords")
        .cloned()
        .unwrap();

    // Org-scope record with sealed field bytes (codec-free fake bytes —
    // plaintext NEVER crosses this module).
    let sealed = vec![
        ("secret".to_string(), vec![1u8, 2, 3]),
        ("notes".to_string(), vec![4u8, 5]),
    ];
    let record = secret_vault::create_record(
        &mut tx,
        org,
        None,
        seed.id,
        fixed_uuid(7818),
        core_domain::entities::SecretRecordKind::Password,
        "Prod DB",
        None,
        None,
        None,
        None,
        &sealed,
        owner,
    )
    .await
    .expect("record create");
    assert_eq!(record.fields.len(), 2, "metadata views only");
    assert!(
        record.fields.iter().all(|f| f.has_value),
        "presence visible"
    );
    // Metadata-only: the DTO carries key + presence, never the bytes.
    let meta_keys: Vec<&str> = record.fields.iter().map(|f| f.field_key.as_str()).collect();
    assert_eq!(
        meta_keys,
        vec!["notes", "secret"],
        "field keys surface, values never"
    );

    // Scope equality: a client-scope record in an org-scope folder refuses.
    let client_uuid = fixed_uuid(7819);
    core_db::clients::create_client(&mut *tx, org, client_uuid, "Scope Client", None)
        .await
        .expect("client seeded");
    let dup_client = secret_vault::create_record(
        &mut tx,
        org,
        Some(client_uuid),
        seed.id,
        fixed_uuid(7820),
        core_domain::entities::SecretRecordKind::Password,
        "Scoping Target",
        None,
        None,
        None,
        None,
        &[],
        owner,
    )
    .await;
    assert!(
        matches!(dup_client, Err(core_db::DbError::Conflict(_))),
        "record scope must equal folder scope"
    );

    // Sealed retrieval round-trips the exact bytes; metadata does not change.
    let sealed_back = secret_vault::list_sealed_fields(&mut tx, org, record.id)
        .await
        .expect("sealed read");
    assert_eq!(
        sealed_back,
        vec![
            ("notes".to_string(), vec![4u8, 5]),
            ("secret".to_string(), vec![1u8, 2, 3]),
        ],
        "sealed bytes come back keyed"
    );
}

#[tokio::test]
async fn importer_maps_shipped_passwords_forward_without_cutover() {
    // Slots 7821 (org), 7822 (member), 7823 (client), 7824 (client pw).
    let db = test_db().await;
    // Org + roles + user + client (pre-0078 shape: passwords table only).
    let org = fixed_uuid(7821);
    let owner = fixed_uuid(7822);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Legacy Co", "vault-co-7821", "LEGACY")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .unwrap();
    let _client =
        core_db::clients::create_client(&mut *tx, org, fixed_uuid(7823), "Legacy Client", None)
            .await
            .expect("client seeded");
    tx.commit().await.expect("commit");

    // Ship TWO rows exactly like the pre-0078 surface did (org-level +
    // client entry) — the deterministic importer in 0078 ran against
    // THEM in this freshly reset schema.
    let mut tx = org_tx(&db.app, org).await;
    core_db::users::create_user(
        &mut *tx,
        fixed_uuid(7822),
        org,
        "owner@vault7821.test",
        "Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("owner user");
    users::assign_role(&mut *tx, fixed_uuid(7822), owner_role)
        .await
        .expect("assign");
    secret_vault::seed_default_secret_folders(&mut tx, org, owner)
        .await
        .expect("seed folders (defaults-first: the org bootstrap runs this)");
    passwords::create_password(
        &mut *tx,
        org,
        fixed_uuid(7825),
        None,
        "Org Vault Entry",
        "svc",
        "https://prod.example",
        &[9u8, 9, 9],
        Some(&[8u8, 8]),
        None,
        fixed_uuid(7822),
    )
    .await
    .expect("org entry");
    passwords::create_password(
        &mut *tx,
        org,
        fixed_uuid(7826),
        Some(fixed_uuid(7823)),
        "Client Vault Entry",
        "svc2",
        "https://client.example",
        &[7u8, 7, 7],
        None,
        None,
        fixed_uuid(7822),
    )
    .await
    .expect("client entry");
    tx.commit().await.expect("commit");

    // The deterministic importer inside 0078 is a ONE-TIME migration —
    // it backfills orgs that exist pre-0078. Post-0078 orgs ride the
    // runtime seeding (org bootstrap calls seed_default_secret_folders)
    // and the D2/D3 import engine for credential rows. The LIVE claims
    // here: the "no destructive cutover" invariant (passwords rows
    // remain the shipped surface) and that new orgs get the same
    // defaults-first vault (the runtime seeding above).
    let mut tx = org_tx(&db.app, org).await;
    let legacy = passwords::list_passwords(
        &mut *tx,
        org,
        &passwords::PasswordFilter {
            client_id: Some(fixed_uuid(7823)),
            global_only: false,
        },
    )
    .await
    .expect("legacy rows persist");
    assert_eq!(
        legacy.len(),
        1,
        "the cutover never deletes: shipped rows remain"
    );
    tx.commit().await.expect("commit rows");

    // And the vault folders seeded DEFAULTS-FIRST for the new org exist.
    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    assert_eq!(
        folders.len(),
        5,
        "new orgs get the seeded defaults via the org bootstrap"
    );
}

#[tokio::test]
async fn category_rbac_restricted_folders_deny_without_grant() {
    // Slots 7826 (org), 7827 (owner), 7828 (tech role↦user).
    let db = test_db().await;
    let (org, roles) = seed_org_at(&db, 7826, "vault-co-7826").await;
    let owner = fixed_uuid(7826 + 40);
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .unwrap();
    let tech_role = roles
        .get(core_domain::policy::BuiltinRole::Technician.as_str())
        .copied()
        .unwrap();
    let _owner = seed_user_at(&db, org, 7827, "owner@vault7826.test", owner_role).await;
    let _tech = seed_user_at(&db, org, 7828, "tech@vault7826.test", tech_role).await;

    let mut tx = org_tx(&db.app, org).await;
    let folders = secret_vault::list_org_folders(&mut tx, org)
        .await
        .expect("folders");
    let seed = folders
        .iter()
        .find(|f| f.slug == "documents")
        .cloned()
        .unwrap();
    let secret_folder = secret_vault::create_folder(
        &mut tx,
        org,
        None,
        Some(seed.id),
        fixed_uuid(7829),
        "msp-api-secrets",
        "MSP API secrets",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        true,
        owner,
    )
    .await
    .expect("a restricted folder");
    // Deny-by-default: the tech role WITHOUT a grant is refused.
    let denied = secret_vault::folder_access(&mut tx, org, &[tech_role], secret_folder.id, false)
        .await
        .expect("gate consult");
    assert!(!denied, "restricted chain denies the ungranted tech role");
    // Owner/Admin ride the gate too (a grant row for THEIR role lands).
    let scoped_grant =
        secret_vault::folder_access(&mut tx, org, &[owner_role], secret_folder.id, false)
            .await
            .expect("gate consult");
    assert!(
        !scoped_grant,
        "restricted denies admins too without a grant row"
    );
    // A GRANT opens the chain (read … and edit needs can_edit).
    let grant = secret_vault::upsert_grant(
        &mut tx,
        org,
        tech_role,
        secret_folder.id,
        false,
        true,
        owner,
    )
    .await
    .expect("grant upserted");
    assert!(grant.can_edit && grant.can_read, "edit implies read");
    let granted = secret_vault::folder_access(&mut tx, org, &[tech_role], secret_folder.id, true)
        .await
        .expect("gate consult");
    assert!(granted, "the grant now opens the restricted chain");
    // An unrelated role stays denied.
    let still_denied =
        secret_vault::folder_access(&mut tx, org, &[owner_role], secret_folder.id, false)
            .await
            .expect("gate consult");
    assert!(!still_denied, "grant is per (role, folder), never global");
}

#[tokio::test]
async fn rls_scopes_folders_records_and_cross_org() {
    // Slots 7830 (org A), 7831 (member A), 7832 (client A).
    let db = test_db().await;
    let (org_a, roles) = seed_org_at(&db, 7830, "vault-co-7830").await;
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .unwrap();
    let _member = seed_user_at(&db, org_a, 7831, "a2@vault7830.test", owner_role).await;
    let owner = fixed_uuid(7830 + 40);

    let mut tx = org_tx(&db.app, org_a).await;
    let custom = secret_vault::create_folder(
        &mut tx,
        org_a,
        None,
        None,
        fixed_uuid(7832),
        "org-a-secret",
        "Org A Secret",
        core_domain::entities::SecretFolderKind::Documents,
        false,
        false,
        owner,
    )
    .await
    .expect("org A folder");
    secret_vault::create_record(
        &mut tx,
        org_a,
        None,
        custom.id,
        fixed_uuid(7832),
        core_domain::entities::SecretRecordKind::Document,
        "Org A Record",
        None,
        None,
        None,
        None,
        &[],
        owner,
    )
    .await
    .expect("org A record");
    tx.commit().await.expect("commit");

    // Org B (7833 + member 7834) sees NONE of A's rows under ITS scope
    // (the RLS deny-by-default property; app-pool connections ride
    // `app.org_id` only when the tx binds).
    let org_b = fixed_uuid(7833);
    let mut tx_b = org_tx(&db.app, org_b).await;
    orgs::insert_org(&mut *tx_b, org_b, "Other Co", "vault-co-7833", "OTHER")
        .await
        .expect("org B seeded");
    orgs::seed_default_roles(&mut tx_b, org_b)
        .await
        .expect("org B roles");
    let owner_b = fixed_uuid(7833 + 40);
    core_db::users::create_user(
        &mut *tx_b,
        owner_b,
        org_b,
        "owner-b@vault7833.test",
        "Owner B",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("org B owner user");
    secret_vault::seed_default_secret_folders(&mut tx_b, org_b, owner_b)
        .await
        .expect("org B seeds");
    tx_b.commit().await.expect("commit B");
    let mut tx_b = org_tx(&db.app, org_b).await;
    let b_folders = secret_vault::list_all_org_folders(&mut tx_b, org_b)
        .await
        .expect("org B folders");
    assert!(
        b_folders
            .iter()
            .all(|f| f.org_id == Some(org_b) || f.client_id.is_some()),
        "no cross-org leakage"
    );
    let org_b_folder = secret_vault::get_folder(&mut *tx_b, org_b, custom.id).await;
    assert!(org_b_folder.is_err(), "RLS: A's folder invisible in B");
    let org_b_record = secret_vault::get_record(&mut tx_b, org_b, fixed_uuid(7832)).await;
    assert!(org_b_record.is_err(), "RLS: B sees no A record rows");
}
