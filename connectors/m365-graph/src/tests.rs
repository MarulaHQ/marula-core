//! M365 Graph connector conformance over contract-shaped stub servers:
//! OAuth2 token flow + bearer usage, @odata.nextLink pagination, both
//! inventory mappers, and the feed-upsert seam. No live tenant.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::{GraphClient, GraphSync};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn oauth2_token_fetched_once_and_bearer_applied_to_every_page() {
    let token_calls = Arc::new(AtomicUsize::new(0));
    let calls = token_calls.clone();
    let token_app = Router::new().route(
        "/token",
        post(move |headers: HeaderMap, body: String| {
            let calls = calls.clone();
            async move {
                assert_eq!(
                    headers.get("content-type").unwrap(),
                    "application/x-www-form-urlencoded"
                );
                assert!(body.contains("grant_type=client_credentials"));
                assert!(body.contains("client_id=app-1"));
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({"access_token": "tenant-token", "expires_in": 3600}))
            }
        }),
    );
    let token_base = spawn(token_app).await;

    let page = Arc::new(AtomicUsize::new(0));
    let page_state = page.clone();
    let graph_app = Router::new().route(
        "/deviceManagement/managedDevices",
        get(move |State(page): State<Arc<AtomicUsize>>, headers: HeaderMap| async move {
            assert_eq!(
                headers.get("authorization").unwrap(),
                "Bearer tenant-token"
            );
            match page.fetch_add(1, Ordering::SeqCst) {
                0 => Json(json!({
                    "value": [
                        {"id": "md-1", "deviceName": "ws-a", "serialNumber": "sn-100",
                         "operatingSystem": "Windows", "osVersion": "11", "complianceState": "compliant"},
                        {"id": "md-2", "deviceName": "ws-b", "serialNumber": "SN 200"}
                    ],
                    "@odata.nextLink": "/deviceManagement/managedDevices?$skip=2"
                })),
                _ => Json(json!({"value": [{"id": "md-3", "deviceName": "ws-c", "serialNumber": "sn-300"}]})),
            }
        })
        .with_state(page_state),
    );
    let graph_base = spawn(graph_app).await;

    let graph = GraphClient::new_with_token_url(
        format!("{token_base}/token"),
        "app-1",
        "secret-1",
        graph_base,
    );
    let devices = graph.managed_devices().await.unwrap();

    let ids: Vec<&str> = devices.iter().filter_map(|d| d["id"].as_str()).collect();
    assert_eq!(ids, ["md-1", "md-2", "md-3"]);
    assert_eq!(page.load(Ordering::SeqCst), 2);
    // ONE token grant for the whole pull (in-process cache).
    assert_eq!(token_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mapper_carries_deterministic_identity_and_verbatim_observed_fields() {
    let record = crate::map_managed_device(&json!({
        "id": "md-9",
        "azureAdDeviceId": "aad-77",
        "deviceName": "WS-Alpha.corp.example",
        "serialNumber": "sn-42/00",
        "operatingSystem": "Windows",
        "osVersion": "10.0.22631",
        "complianceState": "compliant",
        "lastSyncDateTime": "2026-09-12T10:00:00Z",
        "manufacturer": "Dell",
        "model": "OptiPlex"
    }))
    .unwrap();

    assert_eq!(record.source_system, "m365_graph");
    assert_eq!(record.external_id, "md-9");
    assert_eq!(record.identity_keys["source_guid"], "md-9");
    assert_eq!(record.identity_keys["serial"], "SN4200");
    assert_eq!(record.identity_keys["hostname"], "ws-alpha.corp.example");
    // Observed tier: verbatim, only present non-null keys.
    assert_eq!(record.observed_fields["osVersion"], "10.0.22631");
    assert_eq!(record.observed_fields["model"], "OptiPlex");
    assert!(record.observed_fields.get("deviceOwnership").is_none());

    let directory = crate::map_directory_device(&json!({
        "id": "dir-1",
        "deviceId": "device-guid-1",
        "displayName": "WS-Alpha.corp.example",
        "operatingSystem": "Windows",
        "operatingSystemVersion": "10.0.22631",
        "trustType": "AzureAd"
    }))
    .unwrap();
    // GUID + hostname identity for directory devices: the same physical
    // machine's directory object carries the SAME normalized hostname.
    assert_eq!(
        directory.identity_keys["hostname"],
        record.identity_keys["hostname"]
    );
    assert_eq!(directory.identity_keys["source_guid"], "device-guid-1");
    assert_eq!(directory.observed_fields["id"], "dir-1");
}

#[tokio::test]
async fn sync_feeds_through_the_seam_and_reports_both_inventories() {
    let feed_writes: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let writes = feed_writes.clone();

    let token_app = Router::new().route(
        "/token",
        post(|| async { Json(json!({"access_token": "tok", "expires_in": 3600})) }),
    );
    let token_base = spawn(token_app).await;

    let graph_app = Router::new()
        .route(
            "/deviceManagement/managedDevices",
            get(|| async {
                Json(json!({
                    "value": [
                        {"id": "md-1", "deviceName": "ws-a", "serialNumber": "sn-1",
                         "osVersion": "11", "complianceState": "compliant"}
                    ]
                }))
            }),
        )
        .route(
            "/devices",
            get(|| async {
                Json(json!({
                    "value": [
                        {"id": "dir-1", "deviceId": "dg-1", "displayName": "ws-a",
                         "operatingSystem": "Windows", "trustType": "AzureAd"}
                    ]
                }))
            }),
        )
        .route(
            "/api/v1/config-items/feed-upsert",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let writes = writes.clone();
                async move {
                    assert_eq!(
                        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
                        Some("mrd_test_key")
                    );
                    let external = body["external_id"].as_str().unwrap().to_string();
                    assert!(body["idempotency_key"]
                        .as_str()
                        .unwrap()
                        .starts_with(&format!("ci-feeder-m365_graph-{external}-")));
                    writes.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": Uuid::now_v7(),
                        "source_link": {
                            "id": Uuid::now_v7(),
                            "source_system": "m365_graph",
                            "external_id": external
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
    let graph_base = spawn(graph_app).await;

    // Full seam run: stub token endpoint → stub Graph → MeridianFeederSink.
    let graph = GraphClient::new_with_token_url(
        format!("{token_base}/token"),
        "app-1",
        "secret-1",
        graph_base.clone(),
    );
    let client_id = Uuid::now_v7();
    let sink = ci_feeders::sink::MeridianFeederSink::new(ci_feeders::sink::MeridianFeederConfig {
        base_url: graph_base,
        api_key: "mrd_test_key".into(),
    });
    let report = GraphSync::run(&graph, Some(client_id), &sink)
        .await
        .unwrap();

    assert!(report.mapping_errors.is_empty());
    let managed = report.managed_devices.as_ref().unwrap();
    let directory = report.directory_devices.as_ref().unwrap();
    assert_eq!(managed.total, 1);
    assert_eq!(managed.synced, 1);
    assert_eq!(managed.created, 1);
    assert_eq!(directory.total, 1);
    assert_eq!(directory.synced, 1);
    assert!(report.is_clean());
    // Two feed-upsert writes: one per inventory, through the seam only.
    assert_eq!(feed_writes.load(Ordering::SeqCst), 2);
}
