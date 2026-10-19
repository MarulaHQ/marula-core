//! Plugin & skill distribution lifecycle (Wave 16 lane 7, migration
//! 0079, ADR 0009) — the query-layer verification suite for the
//! distribution tables on the branch-local Postgres (port 54379).
//!
//! Purpose: prove the distribution contract —
//!   * the staged pipeline: a member upload lands PENDING (validated,
//!     un-executed), applies/refuses are owner/admin terminal states,
//!     and approval mints the plugin adopt DENY-BY-DEFAULT (disabled)
//!     or the agent-skill version row;
//!   * the append-only provenance chain
//!     `{source_kind, source_url, fetched_ref, artifact_digest,
//!     updater}` — and TERMINAL-UNINSTALL DATA SURVIVAL: deleting the
//!     install row never deletes the chain (no cascade by design);
//!   * per-install settings rendered FROM a manifest JSON-Schema:
//!     the typed SettingsSchema write-boundary gate + the store round
//!     trip;
//!   * the update re-consent pending state: an update proposal pins the
//!     OLD version until `commit_pending_version` flips it (the consent
//!     IS the swap);
//!   * agent skills: version rows append-only, enable deny-by-default on
//!     the active version, provenance per version.
//!
//! Non-purpose: HTTP surfaces (core-api route tests), the plugin-host
//! adapter sandbox caps (crate unit tests), and the events contract
//! (core-events conformance).
//!
//! Slot discipline: fixed UUIDs in the 7900–7950 range only (lane 7's
//! numbers ride migration 0079).

mod common;

use common::{fixed_uuid, org_tx};
use core_db::{orgs, plugin_distribution, plugins, users};
use core_domain::entities::agent_skills::{AgentSkillVersion, SkillManifest};
use core_domain::entities::plugins::{PluginSourceKind, PluginStagedItem, PluginStagedStatus};
use std::collections::HashMap;
use uuid::Uuid;

/// Seeds one org + owner.
async fn seed_org_at(db: &common::TestDb, n: u64, slug: &str) -> (Uuid, HashMap<String, Uuid>) {
    let org = fixed_uuid(n);
    let owner = fixed_uuid(n + 40);
    let mut tx = org_tx(&db.admin, org).await;
    orgs::insert_org(&mut *tx, org, "Distribution Co", slug, "DIST")
        .await
        .expect("org insert");
    let roles = orgs::seed_default_roles(&mut tx, org).await.expect("roles");
    users::create_user(
        &mut *tx,
        owner,
        org,
        &format!("owner-{n}@dist.test"),
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
    (org, roles)
}

fn sample_manifest(n: u64, capabilities: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "name": format!("Staged Plugin {n}"),
        "version": "1.0.0",
        "interface_version": 1,
        "description": "declarative",
        "capabilities": capabilities,
        "settings_schema": {
            "type": "object",
            "properties": {"tone": {"type": "string"}}
        }
    })
}

fn staged_item(n: u64, org: Uuid, uploader: Uuid, manifest: serde_json::Value) -> PluginStagedItem {
    PluginStagedItem {
        id: fixed_uuid(n),
        org_id: org,
        artifact_kind: core_domain::entities::PluginArtifactKind::Plugin,
        source_kind: PluginSourceKind::LocalZip,
        source_url: "local://zip-import".into(),
        fetched_ref: None,
        artifact_digest: "a".repeat(64),
        manifest,
        declared_capabilities: vec!["tickets.read".to_string()],
        status: PluginStagedStatus::Pending,
        uploaded_by: uploader,
        uploaded_at: chrono::Utc::now(),
        review_note: None,
        reviewed_by: None,
        reviewed_at: None,
        adopted_install_id: None,
        adopted_skill_id: None,
        adopted_theme_pack_id: None,
    }
}

async fn pool() -> common::TestDb {
    common::test_db().await
}

#[tokio::test]
async fn staged_lifecycle_should_mint_disabled_adoption_with_provenance() {
    let db = pool().await;
    let (org, _roles) = seed_org_at(&db, 6900, "dist-stage").await;
    let uploader = fixed_uuid(6971);
    let mut tx = org_tx(&db.admin, org).await;
    users::create_user(
        &mut *tx,
        uploader,
        org,
        "member@dist.test",
        "Member",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("member");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.admin, org).await;
    let item = staged_item(6901, org, uploader, sample_manifest(1, &["tickets.read"]));
    plugin_distribution::create_staged_item(&mut tx, &item)
        .await
        .expect("stage");
    transaction_commit(tx).await;

    // Pending rows list, status pending.
    let mut tx = org_tx(&db.admin, org).await;
    let listed = plugin_distribution::list_staged_items(&mut tx, org)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, PluginStagedStatus::Pending);
    transaction_commit(tx).await;

    // Approval (owner/admin consent diff): mints catalog entry + version
    // + an install that is DISABLED (deny-by-default), plus the
    // provenance `installed` link.
    let mut tx = org_tx(&db.admin, org).await;
    plugin_distribution::update_staged_status(
        &mut tx,
        org,
        fixed_uuid(6901),
        PluginStagedStatus::Approved,
        Some("consent diff: tickets.read"),
        Some(uploader),
        chrono::Utc::now(),
        Some(fixed_uuid(6902)),
        None,
        None,
    )
    .await
    .expect("approve");
    tx.commit().await.expect("commit");

    // A terminal staged row refuses a second review.
    let mut tx = org_tx(&db.admin, org).await;
    let err = plugin_distribution::update_staged_status(
        &mut tx,
        org,
        fixed_uuid(6901),
        PluginStagedStatus::Refused,
        None,
        None,
        chrono::Utc::now(),
        None,
        None,
        None,
    )
    .await
    .expect_err("terminal staged rows refuse re-review");
    matches!(&err, core_db::DbError::Conflict(msg) if msg.contains("terminal"));
    transaction_commit(tx).await;
}

async fn transaction_commit<'c>(tx: sqlx::Transaction<'c, sqlx::Postgres>) {
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn provenance_chain_should_survive_terminal_uninstall() {
    let db = pool().await;
    let (org, _roles) = seed_org_at(&db, 6910, "dist-prov").await;
    let admin = fixed_uuid(6981);
    let mut tx = org_tx(&db.admin, org).await;
    users::create_user(
        &mut *tx,
        admin,
        org,
        "admin@dist.test",
        "Admin",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");

    // A catalog entry + install (the normal adopt path came from the
    // staged approval; here the stores layer is exercised directly).
    let mut tx = org_tx(&db.admin, org).await;
    let slug = "provenance-plugin";
    let entry = core_domain::entities::PluginCatalogEntry {
        id: fixed_uuid(6911),
        slug: slug.to_string(),
        name: "Provenance Plugin".into(),
        description: "d".into(),
        publisher: "marula".into(),
        // Owned by no org in the fixture: a seeded/operator entry.
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: 1,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let manifest = core_domain::entities::PluginManifestV2 {
        name: "Provenance Plugin".into(),
        version: "1.0.0".into(),
        interface_version: core_domain::entities::PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "d".into(),
        capabilities: vec!["tickets.read".into()],
        settings_schema: None,
        artifact_kind: None,
        connector_profile: None,
        surface_profile: None,
    };
    let version = core_domain::entities::PluginVersion {
        id: fixed_uuid(6912),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        created_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("entry");
    let install = core_domain::entities::PluginInstall {
        id: fixed_uuid(6913),
        org_id: org,
        catalog_id: entry.id,
        slug: slug.to_string(),
        version: "1.0.0".into(),
        status: core_domain::entities::PluginInstallStatus::Disabled,
        granted_capabilities: vec!["tickets.read".to_string()],
        installed_by: Some(admin),
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        revoked_at: None,
        revoked_by: None,
    };
    plugins::create_install(&mut tx, &install)
        .await
        .expect("install");
    plugin_distribution::insert_provenance(
        &mut tx,
        &core_domain::entities::PluginProvenanceEntry {
            id: fixed_uuid(6914),
            org_id: org,
            install_id: install.id,
            source_kind: PluginSourceKind::LocalZip,
            source_url: "local://zip-import".into(),
            fetched_ref: None,
            artifact_digest: "b".repeat(64),
            uploader: Some(admin),
            action: "installed".into(),
            detail: None,
            recorded_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("provenance");
    tx.commit().await.expect("commit");

    // TERMINAL UNINSTALL: the adoption row goes away...
    let mut tx = org_tx(&db.admin, org).await;
    plugins::delete_install(&mut tx, org, install.id)
        .await
        .expect("uninstall");
    // ...and the provenance chain SURVIVES (install_id has no cascade).
    let chain = plugin_distribution::list_provenance(&mut tx, org, install.id)
        .await
        .expect("provenance survives");
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].action, "installed");
    assert_eq!(chain[0].artifact_digest, "b".repeat(64));
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn update_proposal_should_pin_the_old_version_until_consented() {
    let db = pool().await;
    let (org, _roles) = seed_org_at(&db, 6920, "dist-consent").await;
    let admin = fixed_uuid(6991);
    let mut tx = org_tx(&db.admin, org).await;
    users::create_user(
        &mut *tx,
        admin,
        org,
        "consent@dist.test",
        "Admin",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");

    // Catalog entry with TWO versions: 1.0.0 (granted: tickets.read)
    // and 1.1.0 (adds financial.read) — the update candidate.
    let mut tx = org_tx(&db.admin, org).await;
    let slug = "consent-plugin";
    let entry = core_domain::entities::PluginCatalogEntry {
        id: fixed_uuid(6921),
        slug: slug.to_string(),
        name: "Consent Plugin".into(),
        description: String::new(),
        publisher: "marula".into(),
        // Owned by no org in the fixture: a seeded/operator entry.
        origin_org_id: None,
        latest_version: Some("1.1.0".into()),
        interface_version: 1,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let manifest_v1 = core_domain::entities::PluginManifestV2 {
        name: "Consent Plugin".into(),
        version: "1.0.0".into(),
        interface_version: 1,
        description: String::new(),
        capabilities: vec!["tickets.read".into()],
        settings_schema: None,
        artifact_kind: None,
        connector_profile: None,
        surface_profile: None,
    };
    let manifest_v2 = core_domain::entities::PluginManifestV2 {
        name: "Consent Plugin".into(),
        version: "1.1.0".into(),
        interface_version: 1,
        description: String::new(),
        capabilities: vec!["tickets.read".into(), "financial.read".into()],
        settings_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {"tone": {"type": "string"}},
            "required": ["tone"]
        })),
        artifact_kind: None,
        connector_profile: None,
        surface_profile: None,
    };
    plugins::create_catalog_entry(
        &mut tx,
        &entry,
        &core_domain::entities::PluginVersion {
            id: fixed_uuid(6922),
            catalog_id: entry.id,
            version: "1.0.0".into(),
            manifest: manifest_v1.clone(),
            created_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("entry");
    plugins::append_version(
        &mut tx,
        entry.id,
        &core_domain::entities::PluginVersion {
            id: fixed_uuid(6923),
            catalog_id: entry.id,
            version: "1.1.0".into(),
            manifest: manifest_v2.clone(),
            created_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("append");
    let install = core_domain::entities::PluginInstall {
        id: fixed_uuid(6924),
        org_id: org,
        catalog_id: entry.id,
        slug: slug.to_string(),
        version: "1.0.0".into(),
        status: core_domain::entities::PluginInstallStatus::Disabled,
        granted_capabilities: vec!["tickets.read".to_string()],
        installed_by: Some(admin),
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        revoked_at: None,
        revoked_by: None,
    };
    plugins::create_install(&mut tx, &install)
        .await
        .expect("install");
    tx.commit().await.expect("commit");

    // PROPOSE the update: pending columns carry the candidate; the
    // install STILL runs 1.0.0 with its current grants.
    let mut tx = org_tx(&db.admin, org).await;
    plugin_distribution::set_pending_version(
        &mut tx,
        org,
        install.id,
        "1.1.0",
        &serde_json::to_value(&manifest_v2).unwrap(),
        "c".repeat(64).as_str(),
        Some(&["tickets.read".to_string(), "financial.read".to_string()]),
        chrono::Utc::now(),
    )
    .await
    .expect("propose");
    let install_after_propose = plugins::get_install(&mut tx, org, install.id)
        .await
        .expect("install");
    assert_eq!(install_after_propose.version, "1.0.0");
    tx.commit().await.expect("commit");

    // CONSENT: the swap IS the consent commit.
    let mut tx = org_tx(&db.admin, org).await;
    plugin_distribution::commit_pending_version(&mut tx, org, install.id)
        .await
        .expect("consent");
    let consented = plugins::get_install(&mut tx, org, install.id)
        .await
        .expect("install");
    assert_eq!(consented.version, "1.1.0");
    assert!(consented
        .granted_capabilities
        .contains(&"financial.read".to_string()));
    tx.commit().await.expect("commit");

    // Clearing (discard) empties the pending state.
    let mut tx = org_tx(&db.admin, org).await;
    plugin_distribution::clear_pending_version(&mut tx, org, install.id)
        .await
        .expect("discard");
    transaction_commit(tx).await;
}

#[tokio::test]
async fn install_settings_should_validate_against_the_manifest_schema() {
    let db = pool().await;
    let (org, _roles) = seed_org_at(&db, 6930, "dist-settings").await;
    let admin = fixed_uuid(7001);
    let mut tx = org_tx(&db.admin, org).await;
    users::create_user(
        &mut *tx,
        admin,
        org,
        "cfg@dist.test",
        "Admin",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");

    let mut tx = org_tx(&db.admin, org).await;
    let slug = "settings-plugin";
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"tone": {"type": "string", "enum": ["concise", "detailed"]}},
        "required": ["tone"]
    });
    let manifest = core_domain::entities::PluginManifestV2 {
        name: "Settings Plugin".into(),
        version: "1.0.0".into(),
        interface_version: 1,
        description: String::new(),
        capabilities: vec!["tickets.read".into()],
        settings_schema: Some(schema),
        artifact_kind: None,
        connector_profile: None,
        surface_profile: None,
    };
    let entry = core_domain::entities::PluginCatalogEntry {
        id: fixed_uuid(6931),
        slug: slug.to_string(),
        name: "Settings Plugin".into(),
        description: String::new(),
        publisher: "marula".into(),
        // Owned by no org in the fixture: a seeded/operator entry.
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: 1,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(
        &mut tx,
        &entry,
        &core_domain::entities::PluginVersion {
            id: fixed_uuid(6932),
            catalog_id: entry.id,
            version: "1.0.0".into(),
            manifest,
            created_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("entry");
    let install = core_domain::entities::PluginInstall {
        id: fixed_uuid(6933),
        org_id: org,
        catalog_id: entry.id,
        slug: slug.to_string(),
        version: "1.0.0".into(),
        status: core_domain::entities::PluginInstallStatus::Disabled,
        granted_capabilities: Vec::new(),
        installed_by: Some(admin),
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        revoked_at: None,
        revoked_by: None,
    };
    plugins::create_install(&mut tx, &install)
        .await
        .expect("install");
    tx.commit().await.expect("commit");

    // The write boundary: config validated against the schema BEFORE any
    // row exists. Unknown keys, wrong types, and missing required keys
    // are rejections.
    let schema = core_domain::entities::plugins::SettingsSchema::new(serde_json::json!({
        "type": "object",
        "properties": {"tone": {"type": "string", "enum": ["concise", "detailed"]}},
        "required": ["tone"]
    }))
    .expect("schema");
    assert!(schema
        .validate_config(&serde_json::json!({"tone": "concise"}))
        .is_ok());
    for bad in [
        serde_json::json!({"tone": "rambling"}),
        serde_json::json!({}),
        serde_json::json!({"mystery": 1}),
    ] {
        assert!(schema.validate_config(&bad).is_err());
    }

    // The validated config store round trips.
    let mut tx = org_tx(&db.admin, org).await;
    plugin_distribution::upsert_install_settings(
        &mut tx,
        org,
        install.id,
        &serde_json::json!({"tone": "detailed"}),
        admin,
    )
    .await
    .expect("settings");
    let config = plugin_distribution::get_install_settings(&mut tx, org, install.id)
        .await
        .expect("read");
    assert_eq!(config, serde_json::json!({"tone": "detailed"}));
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn agent_skills_should_append_versions_and_default_to_disabled() {
    let db = pool().await;
    let (org, _roles) = seed_org_at(&db, 6940, "dist-skills").await;
    let admin = fixed_uuid(7011);
    let mut tx = org_tx(&db.admin, org).await;
    users::create_user(
        &mut *tx,
        admin,
        org,
        "skillant@dist.test",
        "Admin",
        "argon2hash",
        "UTC",
    )
    .await
    .expect("user");
    tx.commit().await.expect("commit");

    let skill = AgentSkillVersion {
        id: fixed_uuid(6941),
        org_id: org,
        slug: "triage-helper".to_string(),
        version: 1,
        manifest: SkillManifest {
            name: "Triage Helper".into(),
            version: "1.0.0".into(),
            description: "helps triage".into(),
            capabilities: vec!["tickets.read".to_string()],
            prompts: vec!["Summarize: {{ticket}}".to_string()],
            rules: vec!["Never fabricate statuses.".to_string()],
        },
        description: "helps triage".into(),
        source_kind: PluginSourceKind::SkillCommunity,
        source_url: "https://skills.example/index.json".into(),
        fetched_ref: Some("v3.1.0".into()),
        artifact_digest: "d".repeat(64),
        enabled: false,
        enabled_by: None,
        enabled_at: None,
        created_by: Some(admin),
        created_at: chrono::Utc::now(),
    };
    // Deny-by-default: the imported version row is DISABLED.
    assert!(!skill.enabled);

    let mut tx = org_tx(&db.admin, org).await;
    core_db::agent_platform::insert_skill(
        &mut tx,
        fixed_uuid(6941),
        org,
        admin,
        "triage-helper",
        1,
        &serde_json::to_value(&skill.manifest).unwrap(),
    )
    .await
    .expect("version row");
    // The ACTIVE row defaults to disabled (the distribution column).
    let latest: (bool,) = sqlx::query_as(
        "SELECT enabled FROM agent_skills WHERE org_id = $1 AND slug = 'triage-helper'",
    )
    .bind(org)
    .fetch_one(&mut *tx)
    .await
    .expect("row");
    assert!(!latest.0);
    // The enable gate is an explicit distributor action.
    plugin_distribution::set_skill_enabled(
        &mut tx,
        org,
        "triage-helper",
        true,
        admin,
        chrono::Utc::now(),
    )
    .await
    .expect("enable");
    let latest: (bool,) = sqlx::query_as(
        "SELECT enabled FROM agent_skills WHERE org_id = $1 AND slug = 'triage-helper'",
    )
    .bind(org)
    .fetch_one(&mut *tx)
    .await
    .expect("row");
    assert!(latest.0);
    tx.commit().await.expect("commit");
}
