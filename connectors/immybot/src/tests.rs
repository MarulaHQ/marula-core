//! ImmyBot connector conformance over contract-shaped stub servers:
//! bearer auth, page/size pagination with short-page termination, the
//! device mapper's identity/observed fields, per-group feeding, and
//! none/dry modes. No live tenant.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{ImmybotClient, ImmybotSync};
use adapters_core::OrgMappingBuilder;
use ci_feeders::sink::{MeridianFeederConfig, MeridianFeederSink, NoneFeederSink};

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn device(id: u64, name: &str, group: u64, serial: &str) -> Value {
    json!({
        "id": id,
        "computerName": name,
        "hostname": format!("{name}.corp.example"),
        "clientId": group,
        "serialNumber": serial,
        "osName": "Windows 11",
        "lastPatchScan": "2026-09-12T08:00:00Z",
        "isOnline": true,
        "needsReboot": false
    })
}

#[tokio::test]
async fn pages_until_the_first_short_page_with_bearer_auth() {
    let requests: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let state = requests.clone();
    let app = Router::new().route(
        "/api/v1/devices",
        get(
            move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| {
                let state = state.clone();
                async move {
                    state.lock().unwrap().push((
                        query.get("page").cloned().unwrap_or_default(),
                        query.get("itemsPerPage").cloned().unwrap_or_default(),
                        headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string(),
                    ));
                    let page: u32 = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
                    if page == 1 {
                        // A FULL page (100 = the client's page size) → keep paging.
                        Json(Value::Array(
                            (1..=100)
                                .map(|i| device(i, &format!("ws-{i}"), 10, &format!("SN-{i}")))
                                .collect(),
                        ))
                    } else {
                        Json(json!([device(101, "ws-c", 10, "SN-101")])) // short page → stop
                    }
                }
            },
        ),
    );
    let base = spawn(app).await;

    let client = ImmybotClient::new(&base, "immy-key");
    let devices = client.devices().await.unwrap();

    let ids: Vec<u64> = devices.iter().filter_map(|d| d["id"].as_u64()).collect();
    assert_eq!(ids.len(), 101); // full page + 1-item short page
    assert_eq!(ids[0], 1);
    assert_eq!(ids[100], 101);
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (page, size, auth) in seen.iter() {
        assert_eq!(size, "100");
        assert_eq!(auth, "Bearer immy-key");
        assert!(page == "1" || page == "2");
    }
}

#[tokio::test]
async fn mapper_carries_patch_state_as_observed_fields() {
    let record = crate::map_device(&device(7, "WS-Zeta", 10, "sn 700")).unwrap();

    assert_eq!(record.source_system, "immybot");
    assert_eq!(record.external_id, "7");
    assert_eq!(record.identity_keys["serial"], "SN700");
    assert_eq!(record.identity_keys["hostname"], "ws-zeta.corp.example");
    // Patch-state signals are OBSERVED tier — they churn, never outrank.
    assert_eq!(
        record.observed_fields["lastPatchScan"],
        "2026-09-12T08:00:00Z"
    );
    assert_eq!(record.observed_fields["isOnline"], true);
    assert_eq!(record.observed_fields["needsReboot"], false);
}

#[tokio::test]
async fn mac_identity_joins_the_key_set_normalized_like_every_connector() {
    // ImmyBot's WMI/DMI NIC address under `MAC` (dash spelling accepted;
    // `macAddress`/`macAddresses` too) joins the normalized MAC set.
    let record = crate::map_device(&json!({
        "id": 8,
        "computerName": "WS-MacOnly",
        "clientId": 10,
        "MAC": "AA-BB-CC-DD-EE-FF"
    }))
    .unwrap();
    assert_eq!(record.identity_keys["macs"], json!(["aa:bb:cc:dd:ee:ff"]));
    // No MAC key → the key set stays honest (no empty macs array).
    let record = crate::map_device(&device(9, "WS-NoMac", 10, "SN-9")).unwrap();
    assert!(record.identity_keys.get("macs").is_none());
}

/// The cross-source determinism pin (leg C, Wave 22 lane 3): the SAME
/// physical machine through the NinjaOne mapper and the ImmyBot mapper
/// yields byte-identical normalized identity keys — serial, hostname,
/// AND the MAC set — so the feed-upsert seam's deterministic identity
/// matching converges them onto one canonical item. Divergence here
/// would silently break cross-source convergence (Wave 12 lane 9
/// shipped the ImmyBot mapper WITHOUT the MAC identity signal; the
/// NinjaOne/auvik feeders carried it all along).
#[tokio::test]
async fn same_physical_machine_converges_across_ninjaone_and_immybot() {
    // NinjaOne RMM view of the machine: serial with vendor separators,
    // FQDN system name, MAC in dash form.
    let ninja_record = ci_feeders::vendors::ninjaone::map_device(&json!({
        "id": 4021,
        "uid": "ninja-uuid-9",
        "displayName": "WS-Zeta",
        "systemName": "WS-Zeta.corp.example",
        "serialNumber": "sn 8f/3k2-q",
        "macAddresses": ["AA-BB-CC-DD-EE-FF"],
        "domain": "corp.example"
    }))
    .unwrap();
    // ImmyBot deployment view of the same machine: dotted MAC spelling,
    // hostname + computerName keys, same WMI serial.
    let immy_record = crate::map_device(&json!({
        "id": 991,
        "computerName": "WS-Zeta",
        "hostname": "WS-Zeta.corp.example.",
        "clientId": 10,
        "serialNumber": "SN-8F-3K2-Q",
        "MAC": "aabb.ccdd.eeff"
    }))
    .unwrap();

    assert_eq!(ninja_record.source_system, "ninjaone");
    assert_eq!(immy_record.source_system, "immybot");
    // The link ids differ (per-source), the IDENTITY is identical:
    // serial → SN8F3K2Q, hostname → ws-zeta.corp.example, MAC set →
    // the single normalized lower-colon address.
    assert_eq!(ninja_record.identity_keys["serial"], "SN8F3K2Q");
    assert_eq!(immy_record.identity_keys["serial"], "SN8F3K2Q");
    assert_eq!(
        ninja_record.identity_keys["hostname"],
        immy_record.identity_keys["hostname"]
    );
    assert_eq!(
        ninja_record.identity_keys["macs"],
        immy_record.identity_keys["macs"]
    );
    assert_eq!(
        immy_record.identity_keys["macs"],
        json!(["aa:bb:cc:dd:ee:ff"])
    );
}

#[tokio::test]
async fn sync_feeds_per_group_and_reports_unmapped_groups() {
    let feed_writes: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let writes = feed_writes.clone();
    let seen_clients: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = seen_clients.clone();

    let app = Router::new()
        .route(
            "/api/v1/devices",
            get(|| async {
                Json(json!([
                    device(1, "ws-a", 10, "SN-1"),
                    device(2, "ws-b", 10, "SN-2"),
                    device(3, "ws-c", 99, "SN-3")
                ]))
            }),
        )
        .route(
            "/api/v1/config-items/feed-upsert",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let writes = writes.clone();
                let seen = seen.clone();
                async move {
                    assert_eq!(
                        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
                        Some("mrd_test_key")
                    );
                    seen.lock().unwrap().push(body["client_id"].clone());
                    writes.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": Uuid::now_v7(),
                        "source_link": {
                            "id": Uuid::now_v7(),
                            "source_system": "immybot",
                            "external_id": body["external_id"]
                        },
                        "created_item": true,
                        "resolved_existing": false,
                        "auto_merged": false,
                        "queued_candidates": [],
                        "contradictions": []
                    }))
                }
            }),
        );
    let base = spawn(app).await;

    let client_a = Uuid::now_v7();
    let mapping = OrgMappingBuilder::new().map("10", client_a).build();
    let sink = MeridianFeederSink::new(MeridianFeederConfig {
        base_url: base.clone(),
        api_key: "mrd_test_key".into(),
    });

    let immy = ImmybotClient::new(&base, "k");
    let report = ImmybotSync::run(&immy, Some(&mapping), &sink)
        .await
        .unwrap();

    assert_eq!(report.total, 3);
    assert_eq!(report.synced, 2); // group 10 mapped; group 99 reported
    assert_eq!(report.created, 2);
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].error.contains("99"));
    assert!(report.errors[0].error.contains("not mapped"));
    // Both mapped devices rode the SAME mapped Meridian client.
    assert_eq!(seen_clients.lock().unwrap().len(), 2);
    assert!(seen_clients
        .lock()
        .unwrap()
        .iter()
        .all(|c| *c == json!(client_a)));
    assert_eq!(feed_writes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn no_mapping_is_a_dry_inventory_and_none_sink_counts() {
    let app = Router::new().route(
        "/api/v1/devices",
        get(|| async {
            Json(json!([
                device(1, "ws-a", 10, "SN-1"),
                device(2, "ws-b", 10, "SN-2")
            ]))
        }),
    );
    let base = spawn(app).await;

    let immy = ImmybotClient::new(&base, "k");

    // No mapping at all: dry inventory.
    let report = ImmybotSync::run(&immy, None, &NoneFeederSink)
        .await
        .unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.would_sync, 2);
    assert_eq!(report.synced, 0);

    // Mapping present, sink = NoneFeederSink: none mode counts
    // "would have synced" with zero Meridian I/O.
    let mapping = OrgMappingBuilder::new().map("10", Uuid::now_v7()).build();
    let report = ImmybotSync::run(&immy, Some(&mapping), &NoneFeederSink)
        .await
        .unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.would_sync, 2);
    assert_eq!(report.synced, 0);
    assert!(report.is_clean());
}
