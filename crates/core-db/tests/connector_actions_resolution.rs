//! The external-actions resolver (ADR 0032 §"Surfaces are declared", ADR 0033
//! §"external_refs", `connector-surface.md` §6.7).
//!
//! Purpose: prove the four rules that decide whether a technician sees a chip,
//! because each one is a security or an honesty rule rather than formatting:
//!
//!   * a `disabled` connection is absent, and so is a client-scoped connection
//!     on another client's record — a per-client controller never leaks;
//!   * a connection that cannot act still offers `open` and loses `launch`,
//!     because a session must never start from a credential the vendor has
//!     already refused;
//!   * `capability_defaults` marks exactly one primary, and a client-scoped row
//!     beats the org row for that client;
//!   * providers group under the manifest's `product_kinds`, so a connector
//!     that declares none contributes nothing.
//!
//! Non-purpose: the HTTP shape (route tests), the manifest gate (its own unit
//! tests in `core-domain`), and share/bindings.
//!
//! Slot discipline: fixed-UUID slot 95000+ for the first fixture, 95100+ for
//! the second; each block is self-contained so parallel suites sharing one
//! database cannot collide.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use core_db::connector_connections::{
    resolve_external_actions, upsert_live_ref, ResolvedCapability,
};
use core_db::{clients, orgs, plugins};
use core_domain::connectors::ConnectionStatus;
use core_domain::entities::{
    ConnectorSurfaces, ExternalRefBlock, LaunchBlock, PluginCatalogEntry, PluginInstall,
    PluginInstallStatus, PluginManifestV2, PluginRuntimeKind, PluginVersion,
    PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    org: Uuid,
    client: Uuid,
    asset: Uuid,
}

/// An org, one client, and one configuration belonging to that client.
async fn seed_fixture(db: &TestDb, base: u64, slug: &str) -> Fixture {
    let org = fixed_uuid(base);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "Connector Co", slug, "CON")
        .await
        .expect("org");
    let client = fixed_uuid(base + 1);
    clients::create_client(&mut *tx, org, client, "Acme", None)
        .await
        .expect("client");
    let asset = fixed_uuid(base + 2);
    core_db::assets::create_asset(
        &mut *tx,
        org,
        asset,
        Some(client),
        "client",
        None,
        "LT-001",
        "Dell Latitude",
        "laptop",
        "active",
        None,
        &json!({}),
    )
    .await
    .expect("asset");
    tx.commit().await.expect("commit");
    Fixture { org, client, asset }
}

/// The manifest a connector author would ship: two product kinds, one record
/// block, both behaviours declared.
fn manifest(
    name: &str,
    product_kinds: &[&str],
    external_kind: &str,
    launch_capability: Option<&str>,
) -> PluginManifestV2 {
    PluginManifestV2 {
        name: name.into(),
        version: "1.0.0".into(),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        description: "test connector".into(),
        capabilities: Vec::new(),
        product_kinds: product_kinds.iter().map(|k| (*k).to_string()).collect(),
        connector_surfaces: Some(ConnectorSurfaces {
            base_url_key: None,
            external_refs: vec![ExternalRefBlock {
                record: "configuration".into(),
                external_kind: external_kind.into(),
                open_url: Some("{base_url}/device/{external_id}".into()),
                launch: Some(LaunchBlock {
                    url_template: "{base_url}/device/{external_id}/join".into(),
                    capability: launch_capability.map(str::to_string),
                }),
            }],
        }),
        settings_schema: Some(json!({
            "type": "object",
            "properties": { "base_url": { "type": "string" } }
        })),
        artifact_kind: Some(PluginRuntimeKind::HttpConnector),
        connector_profile: None,
        surface_profile: None,
    }
}

/// Installs one connector definition and returns the install id.
async fn seed_install(
    db: &TestDb,
    org: Uuid,
    base: u64,
    slug: &str,
    manifest: PluginManifestV2,
) -> Uuid {
    let mut tx = org_tx(&db.app, org).await;
    let entry = PluginCatalogEntry {
        id: fixed_uuid(base),
        slug: slug.into(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        publisher: "marula".into(),
        origin_org_id: None,
        latest_version: Some("1.0.0".into()),
        interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
        revoked_at: None,
        revoked_reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let version = PluginVersion {
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
    let install = fixed_uuid(base + 2);
    plugins::create_install(
        &mut tx,
        &PluginInstall {
            id: install,
            org_id: org,
            catalog_id: entry.id,
            slug: slug.into(),
            version: "1.0.0".into(),
            module_digest: String::new(),
            status: PluginInstallStatus::Enabled,
            granted_capabilities: Vec::new(),
            installed_by: None,
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
    install
}

/// Inserts one connection. There is no create function in the repository yet —
/// the write route is a later lane — so the fixture states the row directly.
#[allow(clippy::too_many_arguments)]
async fn seed_connection(
    db: &TestDb,
    org: Uuid,
    id: Uuid,
    install: Uuid,
    scope_client: Option<Uuid>,
    label: &str,
    status: ConnectionStatus,
) {
    let mut tx = org_tx(&db.app, org).await;
    sqlx::query(
        "INSERT INTO connector_connections
           (id, org_id, installation_id, scope_client_id, label, settings, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(org)
    .bind(install)
    .bind(scope_client)
    .bind(label)
    .bind(json!({ "base_url": "https://rmm.example.test/" }))
    .bind(status.as_str())
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");
}

async fn seed_ref(
    db: &TestDb,
    org: Uuid,
    fix: &Fixture,
    connection: Uuid,
    external_kind: &str,
    external_id: &str,
) {
    let mut tx = org_tx(&db.app, org).await;
    upsert_live_ref(
        &mut tx,
        org,
        Uuid::now_v7(),
        core_domain::connectors::RecordType::Configuration,
        fix.asset,
        connection,
        "vendor",
        external_kind,
        external_id,
        None,
        None,
        None,
    )
    .await
    .expect("ref");
    tx.commit().await.expect("commit");
}

async fn seed_default(
    db: &TestDb,
    org: Uuid,
    scope_client: Option<Uuid>,
    capability: &str,
    connection: Uuid,
) {
    let mut tx = org_tx(&db.app, org).await;
    sqlx::query(
        "INSERT INTO capability_defaults (id, org_id, scope_client_id, capability, connection_id)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(org)
    .bind(scope_client)
    .bind(capability)
    .bind(connection)
    .execute(&mut *tx)
    .await
    .expect("capability default");
    tx.commit().await.expect("commit");
}

async fn resolve(db: &TestDb, fix: &Fixture) -> Vec<ResolvedCapability> {
    let mut tx = org_tx(&db.app, fix.org).await;
    let resolved = resolve_external_actions(
        &mut tx,
        fix.org,
        core_domain::connectors::RecordType::Configuration,
        fix.asset,
    )
    .await
    .expect("resolve");
    tx.commit().await.expect("commit");
    resolved
}

fn connection_ids(capability: &ResolvedCapability) -> Vec<Uuid> {
    capability
        .providers
        .iter()
        .map(|p| p.connection_id)
        .collect()
}

fn action_kinds(provider: &core_db::connector_connections::ResolvedProvider) -> Vec<&str> {
    provider.actions.iter().map(|a| a.kind.as_str()).collect()
}

/// Two vendors, four connections: the rules the chip row depends on.
#[tokio::test]
async fn capabilities_group_by_product_kind_and_skip_what_cannot_act() {
    let db = test_db().await;
    let fix = seed_fixture(&db, 95000, "connector-resolve-a").await;

    // RMM vendor, two kinds declared so grouping is observable.
    let rmm_install = seed_install(
        &db,
        fix.org,
        95020,
        "ninjaone-95020",
        manifest("ninjaone", &["rmm", "remote_access"], "device", None),
    )
    .await;
    // A second definition contributes only `remote_access`, so the two
    // connectors meet in one capability and stay apart in the other.
    let screen_install = seed_install(
        &db,
        fix.org,
        95030,
        "screenconnect-95030",
        manifest("screenconnect", &["remote_access"], "machine", None),
    )
    .await;

    let primary = fixed_uuid(95040);
    let auth_failed = fixed_uuid(95041);
    let disabled = fixed_uuid(95042);
    let other_client = fixed_uuid(95043);
    let screen = fixed_uuid(95044);
    let client_b = fixed_uuid(95045);

    {
        let mut tx = org_tx(&db.app, fix.org).await;
        clients::create_client(&mut *tx, fix.org, client_b, "Contoso", None)
            .await
            .expect("second client");
        tx.commit().await.expect("commit");
    }

    seed_connection(
        &db,
        fix.org,
        primary,
        rmm_install,
        None,
        "NinjaOne — Acme",
        ConnectionStatus::Connected,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        auth_failed,
        rmm_install,
        Some(fix.client),
        "NinjaOne — Contoso",
        ConnectionStatus::AuthFailed,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        disabled,
        rmm_install,
        None,
        "NinjaOne — retired",
        ConnectionStatus::Disabled,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        other_client,
        rmm_install,
        Some(client_b),
        "NinjaOne — elsewhere",
        ConnectionStatus::Connected,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        screen,
        screen_install,
        None,
        "ScreenConnect",
        ConnectionStatus::Connected,
    )
    .await;

    seed_ref(&db, fix.org, &fix, primary, "device", "NIN-1").await;
    seed_ref(&db, fix.org, &fix, auth_failed, "device", "NIN-2").await;
    seed_ref(&db, fix.org, &fix, disabled, "device", "NIN-3").await;
    seed_ref(&db, fix.org, &fix, other_client, "device", "NIN-4").await;
    seed_ref(&db, fix.org, &fix, screen, "machine", "SC-9").await;
    seed_default(&db, fix.org, None, "rmm", primary).await;

    let capabilities = resolve(&db, &fix).await;
    let kinds: Vec<&str> = capabilities.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec!["rmm", "remote_access"],
        "grouped by the manifest's product_kinds"
    );

    let rmm = &capabilities[0];
    assert_eq!(
        connection_ids(rmm),
        vec![primary, auth_failed],
        "the disabled connection and another client's controller are absent"
    );
    assert!(
        rmm.providers[0].is_primary,
        "the org default is primary, and it sorts first"
    );
    assert!(!rmm.providers[1].is_primary, "only one provider is primary");

    let primary_provider = &rmm.providers[0];
    assert_eq!(primary_provider.connector, "ninjaone");
    assert_eq!(primary_provider.status, ConnectionStatus::Connected);
    assert_eq!(action_kinds(primary_provider), vec!["open", "launch"]);
    let open = &primary_provider.actions[0];
    assert_eq!(
        open.url.as_deref(),
        Some("https://rmm.example.test/device/NIN-1")
    );
    let launch = &primary_provider.actions[1];
    assert_eq!(
        launch.url.as_deref(),
        Some("https://rmm.example.test/device/NIN-1/join")
    );

    // auth_failed: visible, marked, open only.
    let degraded = &rmm.providers[1];
    assert_eq!(degraded.status, ConnectionStatus::AuthFailed);
    assert_eq!(
        action_kinds(degraded),
        vec!["open"],
        "a connection that cannot act never offers a session"
    );
    assert!(degraded.actions[0]
        .url
        .as_deref()
        .is_some_and(|u| u.ends_with("/device/NIN-2")));

    // remote_access carries both vendors. Two providers and no default: none
    // is primary on purpose, so the chip body opens a menu instead of guessing.
    let remote = &capabilities[1];
    assert_eq!(
        connection_ids(remote),
        vec![primary, auth_failed, screen],
        "the RMM connector's own remote_access kind groups with ScreenConnect's"
    );
    assert_eq!(
        remote.providers.iter().filter(|p| p.is_primary).count(),
        0,
        "several providers and no default means no primary"
    );
}

/// A launch attributed to one capability appears only under that chip.
///
/// This is the vendor that is both an RMM and a remote-access tool: the device
/// page opens under either capability, but "Start session" is a remote-access
/// intent and must not sit under the RMM chip.
#[tokio::test]
async fn an_attributed_launch_appears_only_under_its_capability() {
    let db = test_db().await;
    let fix = seed_fixture(&db, 95400, "connector-resolve-e").await;
    let install = seed_install(
        &db,
        fix.org,
        95420,
        "ninjaone-95420",
        manifest(
            "ninjaone",
            &["rmm", "remote_access"],
            "device",
            Some("remote_access"),
        ),
    )
    .await;
    let connection = fixed_uuid(95440);
    seed_connection(
        &db,
        fix.org,
        connection,
        install,
        None,
        "NinjaOne — Acme",
        ConnectionStatus::Connected,
    )
    .await;
    seed_ref(&db, fix.org, &fix, connection, "device", "NIN-20").await;

    let capabilities = resolve(&db, &fix).await;
    let kinds: Vec<&str> = capabilities.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, vec!["rmm", "remote_access"]);

    let rmm = &capabilities[0];
    assert_eq!(
        action_kinds(&rmm.providers[0]),
        vec!["open"],
        "the RMM chip opens the device page and never starts a session"
    );

    let remote = &capabilities[1];
    assert_eq!(
        action_kinds(&remote.providers[0]),
        vec!["open", "launch"],
        "the attributed capability gets both actions"
    );
    assert!(
        remote.providers[0].is_primary,
        "one provider, no default needed"
    );
}

/// A client-scoped default wins over the org default for that client's record.
#[tokio::test]
async fn client_scoped_default_beats_the_org_default() {
    let db = test_db().await;
    let fix = seed_fixture(&db, 95100, "connector-resolve-b").await;
    let install = seed_install(
        &db,
        fix.org,
        95120,
        "ninjaone-95120",
        manifest("ninjaone", &["rmm"], "device", None),
    )
    .await;

    let org_default = fixed_uuid(95140);
    let client_default = fixed_uuid(95141);
    seed_connection(
        &db,
        fix.org,
        org_default,
        install,
        None,
        "NinjaOne — Acme",
        ConnectionStatus::Connected,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        client_default,
        install,
        Some(fix.client),
        "NinjaOne — Contoso",
        ConnectionStatus::Connected,
    )
    .await;
    seed_ref(&db, fix.org, &fix, org_default, "device", "NIN-10").await;
    seed_ref(&db, fix.org, &fix, client_default, "device", "NIN-11").await;
    seed_default(&db, fix.org, None, "rmm", org_default).await;
    seed_default(&db, fix.org, Some(fix.client), "rmm", client_default).await;

    let capabilities = resolve(&db, &fix).await;
    assert_eq!(capabilities.len(), 1);
    let rmm = &capabilities[0];
    assert_eq!(
        connection_ids(rmm),
        vec![client_default, org_default],
        "the client's row is primary and sorts first"
    );
    assert!(rmm.providers[0].is_primary, "the client row wins");
    assert!(
        !rmm.providers[1].is_primary,
        "the org row is no longer primary"
    );
}

/// A connector that declares no product kinds contributes no capability even
/// though its reference resolves — the "complete connector with settings and a
/// canary" case.
#[tokio::test]
async fn a_connector_with_no_product_kinds_contributes_nothing() {
    let db = test_db().await;
    let fix = seed_fixture(&db, 95200, "connector-resolve-c").await;
    let install = seed_install(
        &db,
        fix.org,
        95220,
        "quiet-95220",
        manifest("quiet", &[], "device", None),
    )
    .await;
    let connection = fixed_uuid(95240);
    seed_connection(
        &db,
        fix.org,
        connection,
        install,
        None,
        "Quiet",
        ConnectionStatus::Connected,
    )
    .await;
    seed_ref(&db, fix.org, &fix, connection, "device", "Q-1").await;

    let capabilities = resolve(&db, &fix).await;
    assert!(capabilities.is_empty(), "no product kinds, no chip");
}

/// The roster is installation-first: an unconfigured install still appears,
/// connections nest under their connector, and a disabled connection stays
/// visible because fixing it is the roster's job.
#[tokio::test]
async fn the_roster_lists_installs_with_nested_connections_and_keeps_disabled_rows() {
    let db = test_db().await;
    let fix = seed_fixture(&db, 95300, "connector-resolve-d").await;
    let install = seed_install(
        &db,
        fix.org,
        95320,
        "ninjaone-95320",
        manifest("ninjaone", &["rmm"], "device", None),
    )
    .await;
    // Installed, never configured — the first-run card whose action is
    // `Connect`. It must render with an empty list, not vanish.
    let unconfigured = seed_install(
        &db,
        fix.org,
        95325,
        "slack-95325",
        manifest("slack", &["notifications"], "channel", None),
    )
    .await;
    // A WASM plugin is not a connector and has no roster entry.
    let mut wasm_manifest = manifest("aaa-wasm", &[], "device", None);
    wasm_manifest.artifact_kind = Some(PluginRuntimeKind::Wasm);
    let _wasm = seed_install(&db, fix.org, 95329, "aaa-wasm-95329", wasm_manifest).await;
    let healthy = fixed_uuid(95340);
    let disabled = fixed_uuid(95341);
    let scoped = fixed_uuid(95342);
    seed_connection(
        &db,
        fix.org,
        healthy,
        install,
        None,
        "NinjaOne — Acme",
        ConnectionStatus::Connected,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        disabled,
        install,
        None,
        "NinjaOne — retired",
        ConnectionStatus::Disabled,
    )
    .await;
    seed_connection(
        &db,
        fix.org,
        scoped,
        install,
        Some(fix.client),
        "NinjaOne — Acme controller",
        ConnectionStatus::Connected,
    )
    .await;
    let mut tx = org_tx(&db.app, fix.org).await;
    sqlx::query("UPDATE connector_connections SET last_failure_at = now() WHERE id = $1")
        .bind(healthy)
        .execute(&mut *tx)
        .await
        .expect("failure stamp");
    for (id, enabled) in [(Uuid::now_v7(), true), (Uuid::now_v7(), false)] {
        sqlx::query(
            "INSERT INTO connector_bindings
               (id, org_id, connection_id, binding_kind, label, external_id, enabled)
             VALUES ($1, $2, $3, 'slack_channel', '#helpdesk', $4, $5)",
        )
        .bind(id)
        .bind(fix.org)
        .bind(healthy)
        .bind(id.to_string())
        .bind(enabled)
        .execute(&mut *tx)
        .await
        .expect("binding");
    }
    let rows = core_db::connector_connections::list_connector_roster(&mut tx, fix.org)
        .await
        .expect("roster");
    tx.commit().await.expect("commit");

    let names: Vec<&str> = rows.iter().map(|r| r.connector.as_str()).collect();
    assert_eq!(
        names,
        vec!["ninjaone", "slack"],
        "connector ascending; the WASM install is not a roster entry"
    );

    let ninjaone = rows
        .iter()
        .find(|r| r.connector == "ninjaone")
        .expect("ninjaone");
    assert_eq!(ninjaone.installation_id, install);
    assert_eq!(
        ninjaone.description, "test connector",
        "the manifest's own line"
    );
    assert_eq!(ninjaone.product_kinds, vec!["rmm".to_string()]);
    let labels: Vec<&str> = ninjaone
        .connections
        .iter()
        .map(|c| c.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "NinjaOne — Acme",
            "NinjaOne — retired",
            "NinjaOne — Acme controller",
        ],
        "org-wide connections first, each group by label"
    );

    let healthy_row = ninjaone
        .connections
        .iter()
        .find(|c| c.id == healthy)
        .expect("healthy row");
    assert_eq!(
        healthy_row.connector, "ninjaone",
        "the connection names it too"
    );
    assert_eq!(
        healthy_row.scope_client_name, None,
        "an org-wide connection names no client"
    );
    assert!(
        healthy_row.last_failure_at.is_some(),
        "the failure timestamp is part of the connection contract"
    );
    assert_eq!(
        healthy_row.bindings_count, 1,
        "a disabled binding is not offered"
    );
    let disabled_row = ninjaone
        .connections
        .iter()
        .find(|c| c.id == disabled)
        .expect("disabled row");
    assert_eq!(disabled_row.status, ConnectionStatus::Disabled);
    let scoped_row = ninjaone
        .connections
        .iter()
        .find(|c| c.id == scoped)
        .expect("scoped row");
    assert_eq!(
        scoped_row.scope_client_name.as_deref(),
        Some("Acme"),
        "the roster says which client a scoped connection serves, by name"
    );

    let slack = rows.iter().find(|r| r.connector == "slack").expect("slack");
    assert_eq!(slack.installation_id, unconfigured);
    assert!(
        slack.connections.is_empty(),
        "an installed but unconfigured connector appears with an empty list"
    );
}
