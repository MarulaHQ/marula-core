//! Credentialed egress, phase 1 — the binding contract (Wave 53 lane 12,
//! migration 0157, ADR 0023 Decision 3's `secret_get` deferral).
//!
//! Purpose: prove the negative properties, which are the deliverable.
//!
//!   * a record from ANOTHER org cannot be bound, and neither can a
//!     client-scope record — the INSERT selects the record through the
//!     caller's org, so there is no row to bind;
//!   * the vault's CATEGORY grant decides twice: an operator whose roles
//!     cannot read a restricted folder cannot bind the record, and a resolve
//!     refuses once the recorded role's grant is withdrawn — consent is
//!     re-checked, never trusted because it was true at bind time;
//!   * a resolve returns CIPHERTEXT: the bytes are the column's bytes, they
//!     are not the plaintext, and they are the only bytes this crate hands out;
//!   * uninstalling the install removes the binding (the FK cascade);
//!   * a PIN MOVE clears every binding (the trigger), a status-only update does
//!     not, and a cleared move leaves an audit row behind;
//!   * RLS isolates the table like every other org-scoped plugin table.
//!
//! Non-purpose: the HTTP consent surface (core-api route tests — the
//! serialized `CredentialBindingView` key set is pinned there), the transport
//! that will consume a resolve (phase 2, `p53-host-callbridge`), and the
//! decrypt itself (core-api owns the key; `core-db` never sees one).
//!
//! Slot discipline: fixed-UUID slots 94800–94849 only.

mod common;

use common::{fixed_uuid, org_tx};
use core_db::{orgs, plugin_credentials, plugins, secret_vault, users};
use core_domain::entities::PluginInstallStatus;
use uuid::Uuid;

/// The bytes a test writes into `secret_fields.value_ciphertext`. They are NOT
/// a real seal — this crate has no key and neither does its test — they are a
/// distinguishable marker, so "what came back is the column's bytes" is a
/// statement a reader can check by eye.
fn fake_ciphertext(marker: &str) -> Vec<u8> {
    format!("sealed::{marker}").into_bytes()
}

struct Seeded {
    org: Uuid,
    owner: Uuid,
    owner_role: Uuid,
}

/// Seeds one org with its default roles and an owner at slot `n`.
async fn seed_org_at(db: &common::TestDb, n: u64, slug: &str) -> Seeded {
    let org = fixed_uuid(n);
    let owner = fixed_uuid(n + 40);
    let mut tx = org_tx(&db.admin, org).await;
    orgs::insert_org(&mut *tx, org, "Credential Co", slug, "CRED")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    users::create_user(
        &mut *tx,
        owner,
        org,
        &format!("owner-{n}@cred.test"),
        "Owner",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("owner user");
    let owner_role = roles
        .get(core_domain::policy::BuiltinRole::Owner.as_str())
        .copied()
        .unwrap();
    users::assign_role(&mut *tx, owner, owner_role)
        .await
        .expect("role");
    tx.commit().await.expect("commit");
    Seeded {
        org,
        owner,
        owner_role,
    }
}

/// A restricted folder inside the org's seeded tree, and one record in it with
/// a single `secret` field.
async fn restricted_folder_with_record(
    db: &common::TestDb,
    seeded: &Seeded,
    folder_slot: u64,
    record_slot: u64,
    marker: &str,
) -> (Uuid, Uuid) {
    let mut tx = org_tx(&db.admin, seeded.org).await;
    let seed = secret_vault::list_org_folders(&mut *tx, seeded.org)
        .await
        .expect("seeded folders")
        .into_iter()
        .find(|f| f.slug == "documents")
        .expect("the documents seed folder");
    let folder_id = fixed_uuid(folder_slot);
    secret_vault::create_folder(
        &mut tx,
        seeded.org,
        None,
        Some(seed.id),
        folder_id,
        &format!("vendor-keys-{folder_slot}"),
        "Vendor API keys",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        // restricted: a grant row is required, which is what the bind and the
        // resolve are supposed to consult.
        true,
        seeded.owner,
    )
    .await
    .expect("restricted folder");
    let record_id = fixed_uuid(record_slot);
    secret_vault::create_record(
        &mut tx,
        seeded.org,
        None,
        folder_id,
        record_id,
        core_domain::entities::SecretRecordKind::ApiKey,
        "Acme tenant token",
        None,
        None,
        None,
        None,
        None,
        &[("secret".to_string(), fake_ciphertext(marker))],
        seeded.owner,
    )
    .await
    .expect("record");
    tx.commit().await.expect("commit");
    (folder_id, record_id)
}

/// A catalog entry + an install at an empty module digest, so a pin move is a
/// version change and nothing has to exist in the artifact store.
async fn install_at(db: &common::TestDb, seeded: &Seeded, base: u64) -> Uuid {
    let mut tx = org_tx(&db.admin, seeded.org).await;
    let slug = format!("cred-plugin-{base}");
    let entry = core_domain::entities::PluginCatalogEntry {
        id: fixed_uuid(base),
        slug: slug.clone(),
        name: "Credential Plugin".into(),
        description: "declares one egress profile".into(),
        publisher: "marula".into(),
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: core_domain::entities::PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let manifest = core_domain::entities::PluginManifestV2 {
        name: "Credential Plugin".into(),
        version: "1.0.0".into(),
        interface_version: core_domain::entities::PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "d".into(),
        capabilities: vec!["automation.reads".into()],
        settings_schema: None,
        artifact_kind: None,
        connector_profile: Some(core_domain::entities::ConnectorProfile {
            webhook_endpoints: Vec::new(),
            scopes: Vec::new(),
            routing: None,
            outbound_scopes: [(
                "acme".to_string(),
                vec!["https://api.acme.test/v1".to_string()],
            )]
            .into_iter()
            .collect(),
        }),
        surface_profile: None,
    };
    let version = core_domain::entities::PluginVersion {
        id: fixed_uuid(base + 1),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");
    let install_id = fixed_uuid(base + 2);
    plugins::create_install(
        &mut tx,
        &core_domain::entities::PluginInstall {
            id: install_id,
            org_id: seeded.org,
            catalog_id: entry.id,
            slug,
            version: "1.0.0".into(),
            module_digest: String::new(),
            status: PluginInstallStatus::Enabled,
            granted_capabilities: vec!["automation.reads".into()],
            installed_by: Some(seeded.owner),
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            previous_version: None,
            previous_module_digest: None,
            previous_capabilities: Vec::new(),
        },
    )
    .await
    .expect("install");
    tx.commit().await.expect("commit");
    install_id
}

async fn grant_read(db: &common::TestDb, seeded: &Seeded, role: Uuid, folder: Uuid) {
    let mut tx = org_tx(&db.admin, seeded.org).await;
    secret_vault::upsert_grant(
        &mut *tx,
        seeded.org,
        role,
        folder,
        true,
        false,
        seeded.owner,
    )
    .await
    .expect("read grant");
    tx.commit().await.expect("commit");
}

async fn revoke_grant(db: &common::TestDb, seeded: &Seeded, role: Uuid, folder: Uuid) {
    let mut tx = org_tx(&db.admin, seeded.org).await;
    secret_vault::delete_grant(&mut *tx, seeded.org, role, folder)
        .await
        .expect("grant revoked");
    tx.commit().await.expect("commit");
}

// ---------------------------------------------------------------------------
// The grant decides, twice
// ---------------------------------------------------------------------------

/// Bind and resolve both walk `secret_vault::folder_access`. The bind refuses
/// a category the operator cannot read; the resolve refuses once that role's
/// read is withdrawn, and the ciphertext it returns before that is the
/// column's bytes and nothing else.
#[tokio::test]
async fn bind_and_resolve_follow_the_category_grant_and_only_return_ciphertext() {
    let db = common::test_db().await;
    let seeded = seed_org_at(&db, 94800, "cred-grant").await;
    let (folder, record) =
        restricted_folder_with_record(&db, &seeded, 94801, 94802, "acme-tenant-token-plaintext")
            .await;
    let install = install_at(&db, &seeded, 94810).await;

    let mut tx = org_tx(&db.app, seeded.org).await;
    // Restricted and ungranted: the owner's own roles cannot read it yet, so
    // the bind is refused before anything is written.
    let denied = plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind consult");
    assert!(
        matches!(denied, plugin_credentials::BindOutcome::CategoryDenied),
        "an operator who cannot read the category cannot hand it to a plugin"
    );
    assert!(
        plugin_credentials::list(&mut tx, seeded.org, install)
            .await
            .expect("list")
            .is_empty(),
        "a refused bind writes nothing"
    );
    tx.commit().await.expect("commit");

    grant_read(&db, &seeded, seeded.owner_role, folder).await;

    let mut tx = org_tx(&db.app, seeded.org).await;
    let bound = plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind");
    let plugin_credentials::BindOutcome::Bound(bound) = bound else {
        panic!("a granted read must allow the bind, got {bound:?}");
    };
    assert_eq!(bound.profile, "acme");
    assert_eq!(bound.header_name, "x-api-key");
    assert_eq!(bound.record_title, "Acme tenant token");
    tx.commit().await.expect("commit");

    // The consent view's read: one row, the record's title, no field.
    let mut tx = org_tx(&db.app, seeded.org).await;
    let listed = plugin_credentials::list(&mut tx, seeded.org, install)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].profile, "acme");

    // THE SEALED-ONLY RULE: what comes back is the ciphertext column, byte for
    // byte, and it is not the plaintext.
    let outcome = plugin_credentials::resolve(&mut tx, seeded.org, install, "acme")
        .await
        .expect("resolve");
    let plugin_credentials::ResolveOutcome::Granted(credential) = outcome else {
        panic!("a granted resolve must return the sealed bytes, got {outcome:?}");
    };
    assert_eq!(
        credential.sealed,
        fake_ciphertext("acme-tenant-token-plaintext")
    );
    assert_eq!(credential.field_key, "secret");
    assert_eq!(credential.header_name, "x-api-key");
    assert_eq!(credential.record_id, record);
    assert!(
        !credential
            .sealed
            .windows(b"acme-tenant-token-plaintext".len())
            .any(|w| w == b"acme-tenant-token-plaintext"),
        "the resolve must not return the plaintext"
    );
    tx.commit().await.expect("commit");

    // WITHDRAWN GRANT: the binding still exists, and the resolve refuses.
    revoke_grant(&db, &seeded, seeded.owner_role, folder).await;
    let mut tx = org_tx(&db.app, seeded.org).await;
    let after = plugin_credentials::resolve(&mut tx, seeded.org, install, "acme")
        .await
        .expect("resolve consult");
    assert!(
        matches!(after, plugin_credentials::ResolveOutcome::GrantRevoked),
        "consent is re-checked, not remembered: got {after:?}"
    );
    assert_eq!(
        plugin_credentials::list(&mut tx, seeded.org, install)
            .await
            .expect("list")
            .len(),
        1,
        "the refusal is the resolve's; the binding stays visible for the operator"
    );
    tx.commit().await.expect("commit");

    // A profile nobody bound resolves to NotBound, not to an error.
    let mut tx = org_tx(&db.app, seeded.org).await;
    assert!(matches!(
        plugin_credentials::resolve(&mut tx, seeded.org, install, "crm")
            .await
            .expect("resolve"),
        plugin_credentials::ResolveOutcome::NotBound
    ));
    // Unbind is a real withdrawal, and it is idempotent-refusing.
    plugin_credentials::unbind(&mut tx, seeded.org, install, "acme")
        .await
        .expect("unbind");
    assert!(
        plugin_credentials::unbind(&mut tx, seeded.org, install, "acme")
            .await
            .is_err()
    );
    tx.commit().await.expect("commit");
}

/// A record without the vault's `secret` field refuses instead of injecting
/// whatever field happens to be there (`notes`, `username`, `url` are
/// metadata, not credentials).
#[tokio::test]
async fn resolve_refuses_a_record_with_no_secret_field() {
    let db = common::test_db().await;
    let seeded = seed_org_at(&db, 94820, "cred-field").await;
    let mut tx = org_tx(&db.admin, seeded.org).await;
    let folder = secret_vault::list_org_folders(&mut *tx, seeded.org)
        .await
        .expect("folders")
        .into_iter()
        .find(|f| f.slug == "documents")
        .expect("seed folder");
    let record = fixed_uuid(94821);
    secret_vault::create_record(
        &mut tx,
        seeded.org,
        None,
        folder.id,
        record,
        core_domain::entities::SecretRecordKind::ApiKey,
        "Notes only",
        None,
        None,
        None,
        None,
        None,
        &[("notes".to_string(), fake_ciphertext("just-notes"))],
        seeded.owner,
    )
    .await
    .expect("record");
    tx.commit().await.expect("commit");
    let install = install_at(&db, &seeded, 94822).await;

    let mut tx = org_tx(&db.app, seeded.org).await;
    // The seeded tree is unrestricted, so the category gate passes.
    let bound = plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "authorization",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind");
    assert!(matches!(bound, plugin_credentials::BindOutcome::Bound(_)));
    let outcome = plugin_credentials::resolve(&mut tx, seeded.org, install, "acme")
        .await
        .expect("resolve");
    assert!(
        matches!(outcome, plugin_credentials::ResolveOutcome::FieldMissing),
        "a record with no material field must refuse, got {outcome:?}"
    );
    tx.commit().await.expect("commit");
}

// ---------------------------------------------------------------------------
// Cross-tenant refusal
// ---------------------------------------------------------------------------

/// Another org's record cannot be bound, and neither can a client-scope record
/// in the caller's own org. Both answer `NoSuchRecord`, so a probe cannot tell
/// "not yours" from "not there".
#[tokio::test]
async fn a_foreign_org_or_client_scope_record_cannot_be_bound() {
    let db = common::test_db().await;
    let ours = seed_org_at(&db, 94830, "cred-ours").await;
    let theirs = seed_org_at(&db, 94831, "cred-theirs").await;
    let (_, their_record) =
        restricted_folder_with_record(&db, &theirs, 94832, 94833, "their-secret").await;
    // Their grant is real and their folder is readable to them; the refusal
    // below is about tenancy, not about authority.
    grant_read(&db, &theirs, theirs.owner_role, fixed_uuid(94832)).await;

    let install = install_at(&db, &ours, 94834).await;
    let mut tx = org_tx(&db.app, ours.org).await;
    let outcome = plugin_credentials::bind(
        &mut tx,
        ours.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: their_record,
            header_name: "x-api-key",
            bound_by: Some(ours.owner),
            bound_role_ids: vec![ours.owner_role],
        },
    )
    .await
    .expect("bind consult");
    assert!(
        matches!(outcome, plugin_credentials::BindOutcome::NoSuchRecord),
        "another org's record is not bindable, got {outcome:?}"
    );
    assert!(plugin_credentials::list(&mut tx, ours.org, install)
        .await
        .expect("list")
        .is_empty());
    tx.commit().await.expect("commit");

    // A client-scope record in OUR org: the install is org-wide, so a client's
    // credential is out of reach even though the vault can see it.
    let client = fixed_uuid(94835);
    let mut tx = org_tx(&db.admin, ours.org).await;
    sqlx::query("INSERT INTO clients (id, org_id, name) VALUES ($1, $2, $3)")
        .bind(client)
        .bind(ours.org)
        .bind("Client Co")
        .execute(&mut *tx)
        .await
        .expect("client insert");
    let client_folder = fixed_uuid(94836);
    secret_vault::create_folder(
        &mut tx,
        ours.org,
        Some(client),
        None,
        client_folder,
        "client-vendor-keys",
        "Client vendor keys",
        core_domain::entities::SecretFolderKind::ApiKeys,
        false,
        false,
        ours.owner,
    )
    .await
    .expect("client folder");
    let client_record = fixed_uuid(94837);
    secret_vault::create_record(
        &mut tx,
        ours.org,
        Some(client),
        client_folder,
        client_record,
        core_domain::entities::SecretRecordKind::ApiKey,
        "Client token",
        None,
        None,
        None,
        None,
        None,
        &[("secret".to_string(), fake_ciphertext("client-secret"))],
        ours.owner,
    )
    .await
    .expect("client record");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, ours.org).await;
    let outcome = plugin_credentials::bind(
        &mut tx,
        ours.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: client_record,
            header_name: "x-api-key",
            bound_by: Some(ours.owner),
            bound_role_ids: vec![ours.owner_role],
        },
    )
    .await
    .expect("bind consult");
    assert!(
        matches!(outcome, plugin_credentials::BindOutcome::NoSuchRecord),
        "a client-scope record is not bindable by an org-wide install, got {outcome:?}"
    );
    tx.commit().await.expect("commit");
}

/// The table is org-isolated like every other plugin-state table: another
/// org's transaction reads the row as absent.
#[tokio::test]
async fn bindings_are_invisible_to_another_org() {
    let db = common::test_db().await;
    let ours = seed_org_at(&db, 94840, "cred-rls-ours").await;
    let theirs = seed_org_at(&db, 94841, "cred-rls-theirs").await;
    let (folder, record) =
        restricted_folder_with_record(&db, &ours, 94842, 94843, "rls-secret").await;
    grant_read(&db, &ours, ours.owner_role, folder).await;
    let install = install_at(&db, &ours, 94844).await;

    let mut tx = org_tx(&db.app, ours.org).await;
    plugin_credentials::bind(
        &mut tx,
        ours.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(ours.owner),
            bound_role_ids: vec![ours.owner_role],
        },
    )
    .await
    .expect("bind");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.app, theirs.org).await;
    assert!(
        plugin_credentials::list(&mut tx, theirs.org, install)
            .await
            .expect("foreign list")
            .is_empty(),
        "the binding belongs to one org"
    );
    assert!(
        matches!(
            plugin_credentials::resolve(&mut tx, theirs.org, install, "acme")
                .await
                .expect("foreign resolve"),
            plugin_credentials::ResolveOutcome::NotBound
        ),
        "and no other org can resolve it"
    );
    tx.commit().await.expect("commit");
}

// ---------------------------------------------------------------------------
// Lifecycle: uninstall and pin moves
// ---------------------------------------------------------------------------

/// Uninstalling the install removes the binding through the FK cascade — no
/// code path has to remember.
#[tokio::test]
async fn uninstalling_the_install_removes_the_binding() {
    let db = common::test_db().await;
    let seeded = seed_org_at(&db, 94850, "cred-uninstall").await;
    let (folder, record) = restricted_folder_with_record(&db, &seeded, 94851, 94852, "gone").await;
    grant_read(&db, &seeded, seeded.owner_role, folder).await;
    let install = install_at(&db, &seeded, 94853).await;

    let mut tx = org_tx(&db.admin, seeded.org).await;
    plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind");
    plugins::delete_install(&mut tx, seeded.org, install)
        .await
        .expect("uninstall");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM plugin_credential_bindings WHERE install_id = $1")
            .bind(install)
            .fetch_one(&mut *tx)
            .await
            .expect("count");
    tx.commit().await.expect("commit");
    assert_eq!(count, 0, "the uninstall takes the consent with it");
}

/// A binding does not survive a pin move, and the trigger is what guarantees
/// it: a hand-written UPDATE that moves the version clears the row and leaves
/// an audit trail. A status-only update leaves the binding alone.
#[tokio::test]
async fn a_pin_move_clears_bindings_and_a_status_change_does_not() {
    let db = common::test_db().await;
    let seeded = seed_org_at(&db, 94860, "cred-pin").await;
    let (folder, record) = restricted_folder_with_record(&db, &seeded, 94861, 94862, "pin").await;
    grant_read(&db, &seeded, seeded.owner_role, folder).await;
    let install = install_at(&db, &seeded, 94863).await;

    let mut tx = org_tx(&db.admin, seeded.org).await;
    plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind");
    tx.commit().await.expect("commit");

    // A status flip goes through `update_install`, which restates `version`.
    // Same value, so the trigger's IS DISTINCT FROM guard leaves the row. Run
    // under the APP role so the RLS-scoped path is the one proven.
    let mut tx = org_tx(&db.app, seeded.org).await;
    plugins::update_install(
        &mut tx,
        seeded.org,
        install,
        Some(PluginInstallStatus::Disabled),
        None,
        None,
    )
    .await
    .expect("status update");
    assert_eq!(
        plugin_credentials::list(&mut tx, seeded.org, install)
            .await
            .expect("list")
            .len(),
        1,
        "disabling an install is not a pin move and must not drop the consent"
    );
    tx.commit().await.expect("commit");

    // The hand-written pin move: no route, no helper, just the statement a
    // future refactor might write — under the app role, so the trigger's DELETE
    // and its audit INSERT both run inside the tenant's RLS scope.
    let mut tx = org_tx(&db.app, seeded.org).await;
    sqlx::query(
        "UPDATE plugin_installs SET version = $3, module_digest = $4 WHERE org_id = $1 AND id = $2",
    )
    .bind(seeded.org)
    .bind(install)
    .bind("2.0.0")
    .bind("c".repeat(64))
    .execute(&mut *tx)
    .await
    .expect("pin move");
    assert!(
        plugin_credentials::list(&mut tx, seeded.org, install)
            .await
            .expect("list")
            .is_empty(),
        "a pin move clears the bindings, whichever code performed it"
    );
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT action, detail FROM audit_events
          WHERE org_id = $1 AND entity_id = $2
            AND action = 'plugin.credentials_cleared_on_pin_move'
          ORDER BY occurred_at",
    )
    .bind(seeded.org)
    .bind(install.to_string())
    .fetch_all(&mut *tx)
    .await
    .expect("audit rows");
    tx.commit().await.expect("commit");
    assert_eq!(rows.len(), 1, "the clear is audited even without a route");
    assert_eq!(rows[0].0, "plugin.credentials_cleared_on_pin_move");
    assert_eq!(rows[0].1["cleared_bindings"], 1);
    assert_eq!(rows[0].1["from_version"], "1.0.0");
    assert_eq!(rows[0].1["to_version"], "2.0.0");
}

/// A deleted vault record takes its binding with it: a consent that points at
/// nothing must not keep looking live on the consent screen.
#[tokio::test]
async fn deleting_the_record_removes_the_binding() {
    let db = common::test_db().await;
    let seeded = seed_org_at(&db, 94870, "cred-record-gone").await;
    let (folder, record) = restricted_folder_with_record(&db, &seeded, 94871, 94872, "bye").await;
    grant_read(&db, &seeded, seeded.owner_role, folder).await;
    let install = install_at(&db, &seeded, 94873).await;

    let mut tx = org_tx(&db.admin, seeded.org).await;
    plugin_credentials::bind(
        &mut tx,
        seeded.org,
        install,
        &plugin_credentials::NewBinding {
            profile: "acme",
            secret_record_id: record,
            header_name: "x-api-key",
            bound_by: Some(seeded.owner),
            bound_role_ids: vec![seeded.owner_role],
        },
    )
    .await
    .expect("bind");
    secret_vault::delete_record(&mut tx, seeded.org, record)
        .await
        .expect("record deleted");
    assert!(
        plugin_credentials::list(&mut tx, seeded.org, install)
            .await
            .expect("list")
            .is_empty(),
        "a deleted credential leaves no binding"
    );
    tx.commit().await.expect("commit");
}
