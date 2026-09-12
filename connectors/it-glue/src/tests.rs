//! IT Glue live-transport conformance over contract-shaped stub
//! servers: auth header, native JSON:API pagination, org→client
//! mapping, and the feed-upsert seam. No live credentials.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{ItGlueClient, ItGlueSync, NoneFeederSink};
use adapters_core::OrgMappingBuilder;
use ci_feeders::sink::{MeridianFeederConfig, MeridianFeederSink};

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn configuration(id: &str, name: &str, org_id: &str, serial: &str) -> Value {
    json!({
        "type": "configurations",
        "id": id,
        "attributes": {
            "name": name,
            "organization_id": {"id": org_id, "name": "Acme"},
            "hostname": format!("host-{name}"),
            "serial_number": serial,
            "mac_address": "AA-CC-DD-EE-FF-01",
            "primary_ip": "10.0.0.5",
            "operating_system_notes": "Windows 11"
        }
    })
}

#[tokio::test]
async fn pulls_all_pages_with_x_api_key_and_filters_by_org() {
    let requests: Arc<Mutex<Vec<(String, String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let state = requests.clone();
    let app = Router::new().route(
        "/configurations",
        get(
            move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| {
                let state = state.clone();
                async move {
                    state.lock().unwrap().push((
                        query
                            .get("filter[organization_id]")
                            .cloned()
                            .unwrap_or_default(),
                        query.get("page[number]").cloned().unwrap_or_default(),
                        headers
                            .get("x-api-key")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string(),
                    ));
                    let page: u32 = query
                        .get("page[number]")
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(1);
                    if page == 1 {
                        Json(json!({
                            "data": [
                                configuration("101", "srv-a", "77", "SN-8F 3K2 Q"),
                                configuration("102", "srv-b", "77", "SN-2222")
                            ],
                            "links": {"next": {"href": "/configurations?page[number]=2"}}
                        }))
                    } else {
                        Json(json!({
                            "data": [configuration("103", "srv-c", "77", "SN-3333")],
                            "links": {}
                        }))
                    }
                }
            },
        ),
    );
    let base = spawn(app).await;

    let client = ItGlueClient::new(&base, "itglue-key");
    let (records, errors) = client.configurations_for_org("77").await.unwrap();

    assert!(errors.is_empty());
    let ids: Vec<&str> = records.iter().map(|r| r.external_id.as_str()).collect();
    assert_eq!(ids, ["101", "102", "103"]);
    // Every page rode the org filter + the auth key.
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (org, _, key) in seen.iter() {
        assert_eq!(org, "77");
        assert_eq!(key, "itglue-key");
    }
}

#[tokio::test]
async fn identity_keys_are_deterministic_across_formatting() {
    let app = Router::new().route(
        "/configurations",
        get(|| async {
            Json(json!({
                "data": [
                    configuration("201", "srv-a", "77", "SN-8F/3K2-Q"),
                    configuration("202", "srv-a", "77", " sn 8f 3k2 q ")
                ],
                "links": {}
            }))
        }),
    );
    let base = spawn(app).await;

    let client = ItGlueClient::new(&base, "k");
    let (records, _) = client.configurations_for_org("77").await.unwrap();

    let a = &records[0].identity_keys;
    let b = &records[1].identity_keys;
    // The SAME serial, differently formatted by the vendor, normalizes
    // to the same key — determinism is a property of the KEYS.
    assert_eq!(a["serial"], b["serial"]);
    assert_eq!(a["serial"], "SN8F3K2Q");
    assert_eq!(a["macs"][0], "aa:cc:dd:ee:ff:01");
}

#[tokio::test]
async fn organizations_list_the_mapping_inventory() {
    let app = Router::new().route(
        "/organizations",
        get(|| async {
            Json(json!({
                "data": [
                    {"type": "organizations", "id": "77", "attributes": {"name": "Acme Corp"}},
                    {"type": "organizations", "id": "88", "attributes": {"name": "Beta LLC"}}
                ],
                "links": {}
            }))
        }),
    );
    let base = spawn(app).await;

    let client = ItGlueClient::new(&base, "k");
    let orgs = client.organizations().await.unwrap();
    assert_eq!(
        orgs,
        vec![
            ("77".into(), "Acme Corp".into()),
            ("88".into(), "Beta LLC".into())
        ]
    );
}

#[tokio::test]
async fn sync_feeds_mapped_orgs_and_reports_unmapped_never_guesses() {
    let feed_writes: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let writes = feed_writes.clone();
    let app = Router::new()
        .route(
            "/configurations",
            get(|| async {
                Json(json!({
                    "data": [configuration("301", "srv-a", "77", "SN-AAAA")],
                    "links": {}
                }))
            }),
        )
        .route(
            "/api/v1/config-items/feed-upsert",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let writes = writes.clone();
                async move {
                    // Machine principals ride x-api-key (the lane-8 seam).
                    assert_eq!(
                        headers.get("x-api-key").and_then(|v| v.to_str().ok()),
                        Some("mrd_test_key")
                    );
                    assert!(body["idempotency_key"]
                        .as_str()
                        .unwrap()
                        .starts_with("ci-feeder-it_glue-301-"));
                    writes.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": Uuid::now_v7(),
                        "source_link": {
                            "id": Uuid::now_v7(),
                            "source_system": "it_glue",
                            "external_id": "301"
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

    let client_id = Uuid::now_v7();
    let mapping = OrgMappingBuilder::new().map("77", client_id).build();
    let sink = MeridianFeederSink::new(MeridianFeederConfig {
        base_url: base.clone(),
        api_key: "mrd_test_key".into(),
    });

    let itglue = ItGlueClient::new(&base, "k");
    let outcomes = ItGlueSync::run(&itglue, &mapping, &["77".into(), "999".into()], &sink)
        .await
        .unwrap();

    assert_eq!(outcomes.len(), 2);
    assert_eq!(feed_writes.load(Ordering::SeqCst), 1); // only the mapped org fed
    let mapped = &outcomes[0];
    assert_eq!(mapped.total, 1);
    assert_eq!(mapped.report.as_ref().unwrap().created, 1);
    // Unmapped org: reported, records skipped, never a guess.
    let unmapped = &outcomes[1];
    assert_eq!(unmapped.total, 0);
    assert!(unmapped.errors[0].contains("999"));
    assert!(unmapped.errors[0].contains("not mapped"));
}

#[tokio::test]
async fn none_mode_counts_without_meridian_io() {
    let app = Router::new().route(
        "/configurations",
        get(|| async {
            Json(json!({
                "data": [
                    configuration("401", "srv-a", "77", "SN-1"),
                    configuration("402", "srv-b", "77", "SN-2")
                ],
                "links": {}
            }))
        }),
    );
    let base = spawn(app).await;

    let mapping = OrgMappingBuilder::new().map("77", Uuid::now_v7()).build();
    let itglue = ItGlueClient::new(&base, "k");
    let outcomes = ItGlueSync::run(&itglue, &mapping, &["77".into()], &NoneFeederSink)
        .await
        .unwrap();

    let report = outcomes[0].report.as_ref().unwrap();
    assert_eq!(report.total, 2);
    assert_eq!(report.would_sync, 2); // would have synced, zero I/O
    assert_eq!(report.synced, 0);
    assert!(report.is_clean());
}
