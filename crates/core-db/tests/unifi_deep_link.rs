//! G10 — the UniFi deep link, proved through the shipped resolver.
//!
//! The claim is "an asset opened from a UniFi-fed client links to that device
//! in the correct controller". Two clients, two controllers, two assets: the
//! manifest this crate ships is the one under test, and the URL comes from the
//! resolver the staff UI calls, not from a helper in this file.
//!
//! Slot discipline: fixed-UUID slots 95300–95399, clear of the resolver
//! suite's 95000s and 95100s.

mod common;

use common::{fixed_uuid, org_tx, test_db, TestDb};
use core_db::connector_connections::{resolve_external_actions, upsert_live_ref, ConnectionViewer};
use core_db::{assets, clients, orgs, plugins};
use core_domain::connectors::{ConnectionStatus, RecordType};
use core_domain::entities::{
    PluginCatalogEntry, PluginInstall, PluginInstallStatus, PluginManifestV2, PluginVersion,
    PLUGIN_RUNTIME_INTERFACE_VERSION,
};
use serde_json::json;
use uuid::Uuid;

/// The manifest the connector actually ships.
const MANIFEST: &str = include_str!("../../../plugins/connectors/unifi/manifest.json");

fn shipped_manifest() -> PluginManifestV2 {
    let manifest: PluginManifestV2 =
        serde_json::from_str(MANIFEST).expect("the shipped UniFi manifest parses");
    manifest
        .validate()
        .expect("the shipped manifest passes the gate");
    manifest
}

async fn seed_install(db: &TestDb, org: Uuid) -> Uuid {
    let manifest = shipped_manifest();
    let mut tx = org_tx(&db.app, org).await;
    let entry = PluginCatalogEntry {
        id: fixed_uuid(95_320),
        slug: "unifi".into(),
        name: "UniFi".into(),
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
        id: fixed_uuid(95_321),
        catalog_id: entry.id,
        version: "1.0.0".into(),
        manifest,
        module_digest: String::new(),
        created_at: chrono::Utc::now(),
    };
    plugins::create_catalog_entry(&mut tx, &entry, &version)
        .await
        .expect("catalog entry");
    let install = fixed_uuid(95_322);
    plugins::create_install(
        &mut tx,
        &PluginInstall {
            id: install,
            org_id: org,
            catalog_id: entry.id,
            slug: "unifi".into(),
            version: "1.0.0".into(),
            module_digest: String::new(),
            status: PluginInstallStatus::Enabled,
            granted_capabilities: vec!["clients.write".into()],
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

/// One client-scoped connection with its own controller origin and site.
#[allow(clippy::too_many_arguments)]
async fn seed_connection(
    db: &TestDb,
    org: Uuid,
    id: Uuid,
    install: Uuid,
    client: Uuid,
    label: &str,
    controller_url: &str,
    site: &str,
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
    .bind(client)
    .bind(label)
    .bind(json!({ "controller_url": controller_url, "site": site }))
    .bind(ConnectionStatus::Connected.as_str())
    .execute(&mut *tx)
    .await
    .expect("connection");
    tx.commit().await.expect("commit");
}

async fn seed_asset(db: &TestDb, org: Uuid, id: Uuid, client: Uuid, tag: &str) {
    let mut tx = org_tx(&db.app, org).await;
    assets::create_asset(
        &mut *tx,
        org,
        id,
        Some(client),
        "client",
        None,
        tag,
        "UniFi switch",
        "switch",
        "active",
        None,
        &json!({}),
    )
    .await
    .expect("asset");
    tx.commit().await.expect("commit");
}

async fn seed_ref(db: &TestDb, org: Uuid, asset: Uuid, connection: Uuid, external_id: &str) {
    let mut tx = org_tx(&db.app, org).await;
    upsert_live_ref(
        &mut tx,
        org,
        Uuid::now_v7(),
        RecordType::Configuration,
        asset,
        connection,
        "unifi",
        "device",
        external_id,
        None,
        None,
        None,
    )
    .await
    .expect("ref");
    tx.commit().await.expect("commit");
}

async fn open_url(db: &TestDb, org: Uuid, asset: Uuid) -> String {
    let mut tx = org_tx(&db.app, org).await;
    // An unrestricted viewer: this suite is about the ORIGIN each asset
    // resolves to, not about the connection's audience. Audience filtering has
    // its own suite; the sync paths that read this resolver have already
    // decided access above it (`ConnectionViewer::unrestricted`'s contract).
    let capabilities = resolve_external_actions(
        &mut tx,
        org,
        RecordType::Configuration,
        asset,
        &ConnectionViewer::unrestricted(),
    )
    .await
    .expect("resolve");
    tx.commit().await.expect("commit");
    capabilities
        .iter()
        .flat_map(|capability| capability.providers.iter())
        .flat_map(|provider| provider.actions.iter())
        .find(|action| action.kind == "open")
        .and_then(|action| action.url.clone())
        .unwrap_or_else(|| panic!("no open action resolved: {capabilities:?}"))
}

#[tokio::test]
async fn each_client_links_to_its_own_controller() {
    let db = test_db().await;
    let org = fixed_uuid(95_300);
    let mut tx = org_tx(&db.app, org).await;
    orgs::insert_org(&mut *tx, org, "UniFi Org", "unifi-deep-link", "UFI")
        .await
        .expect("org");
    let acme = fixed_uuid(95_301);
    let contoso = fixed_uuid(95_302);
    clients::create_client(&mut *tx, org, acme, "Acme", None)
        .await
        .expect("client");
    clients::create_client(&mut *tx, org, contoso, "Contoso", None)
        .await
        .expect("client");
    tx.commit().await.expect("commit");

    let install = seed_install(&db, org).await;
    let hostifi = fixed_uuid(95_330);
    let on_prem = fixed_uuid(95_331);
    seed_connection(
        &db,
        org,
        hostifi,
        install,
        acme,
        "Hostifi — Acme",
        "https://acme.hostifi.example",
        "acme",
    )
    .await;
    seed_connection(
        &db,
        org,
        on_prem,
        install,
        contoso,
        "On-prem — Contoso",
        "https://unifi.contoso.test",
        "contoso",
    )
    .await;

    let acme_asset = fixed_uuid(95_303);
    let contoso_asset = fixed_uuid(95_304);
    seed_asset(&db, org, acme_asset, acme, "ACME-SW-01").await;
    seed_asset(&db, org, contoso_asset, contoso, "CON-SW-01").await;
    seed_ref(&db, org, acme_asset, hostifi, "dev-acme-1").await;
    seed_ref(&db, org, contoso_asset, on_prem, "dev-contoso-9").await;

    assert_eq!(
        open_url(&db, org, acme_asset).await,
        "https://acme.hostifi.example/manage/acme/devices/dev-acme-1"
    );
    assert_eq!(
        open_url(&db, org, contoso_asset).await,
        "https://unifi.contoso.test/manage/contoso/devices/dev-contoso-9"
    );
}
