//! UniFi connector conformance over contract-shaped stub servers:
//! `X-API-KEY` auth, offset/limit pagination with the totalCount
//! envelope, the device mapper's identity/observed fields, per-site
//! feeding + interface creation, none/dry modes, the alert receiver's
//! raw-bytes verification, and the signed ingest forwarder. No live
//! controller.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::alerts::{
    map_alert, sign_delivery, verify_delivery, AlertForwarder, ForwardError, REPLAY_TOLERANCE_SECS,
    SIGNATURE_HEADER,
};
use crate::{map_device, MeridianInterfaceSink, UnifiClient, UnifiSync};
use adapters_core::OrgMappingBuilder;
use ci_feeders::sink::{MeridianFeederConfig, MeridianFeederSink, NoneFeederSink};

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn site(id: &str, name: &str) -> Value {
    json!({ "id": id, "internalReference": name, "name": name })
}

fn device(id: &str, name: &str, mac: &str) -> Value {
    json!({
        "id": id,
        "macAddress": mac,
        "ipAddress": "192.168.1.10",
        "name": name,
        "model": "USW-Pro-24-PoE",
        "state": "ONLINE",
        "firmwareVersion": "7.0.50",
        "supported": true,
        "interfaces": {
            "ports": [
                { "idx": 1, "state": "UP", "connector": "RJ45", "maxSpeedMbps": 1000 },
                { "idx": 2, "state": "DOWN", "connector": "RJ45", "maxSpeedMbps": 1000 }
            ]
        }
    })
}

/// The controller stub's shared state: request log + site/device data.
#[derive(Clone)]
struct Controller {
    requests: Arc<Mutex<Vec<(String, String, String)>>>,
    sites: Arc<Vec<Value>>,
    devices: Arc<HashMap<String, Vec<Value>>>,
}

impl Controller {
    fn page(&self, items: &[Value], offset: usize, limit: usize) -> Value {
        let page: Vec<Value> = items.iter().skip(offset).take(limit).cloned().collect();
        json!({
            "offset": offset,
            "limit": limit,
            "count": page.len(),
            "totalCount": items.len(),
            "data": page,
        })
    }

    fn saw_api_key(&self) -> bool {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .all(|(_, key, _)| key == "unifi-key")
    }

    fn device_offsets(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _, _)| p.contains("/devices"))
            .map(|(_, _, o)| o.clone())
            .collect()
    }
}

/// UniFi controller stub: `X-API-KEY` enforced, totalCount-envelope
/// offset/limit pagination across sites + per-site devices.
fn controller_stub(
    sites: Vec<Value>,
    devices_by_site: HashMap<String, Vec<Value>>,
) -> (Router, Controller) {
    let controller = Controller {
        requests: Arc::new(Mutex::new(Vec::new())),
        sites: Arc::new(sites),
        devices: Arc::new(devices_by_site),
    };
    let state = controller.clone();
    let sites_state = state.clone();
    let devices_state = state.clone();
    let app = Router::new()
        .route(
            "/proxy/network/integration/v1/sites",
            get(
                move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| {
                    let state = sites_state.clone();
                    async move {
                        let key = headers
                            .get("x-api-key")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default();
                        let offset: usize = query
                            .get("offset")
                            .and_then(|o| o.parse().ok())
                            .unwrap_or(0);
                        let limit: usize = query
                            .get("limit")
                            .and_then(|o| o.parse().ok())
                            .unwrap_or(25);
                        state.requests.lock().unwrap().push((
                            "sites".into(),
                            key.into(),
                            offset.to_string(),
                        ));
                        Json(state.page(&state.sites, offset, limit))
                    }
                },
            ),
        )
        .route(
            "/proxy/network/integration/v1/sites/{site_id}/devices",
            get(
                move |headers: HeaderMap,
                      Path(site_id): Path<String>,
                      Query(query): Query<HashMap<String, String>>| {
                    let state = devices_state.clone();
                    async move {
                        let key = headers
                            .get("x-api-key")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default();
                        let offset: usize = query
                            .get("offset")
                            .and_then(|o| o.parse().ok())
                            .unwrap_or(0);
                        let limit: usize = query
                            .get("limit")
                            .and_then(|o| o.parse().ok())
                            .unwrap_or(25);
                        state.requests.lock().unwrap().push((
                            format!("sites/{site_id}/devices"),
                            key.into(),
                            offset.to_string(),
                        ));
                        let items = state.devices.get(&site_id).cloned().unwrap_or_default();
                        Json(state.page(&items, offset, limit))
                    }
                },
            ),
        );
    (app, controller)
}

#[tokio::test]
async fn pages_until_totalcount_with_api_key_auth() {
    let sites = vec![site("site-1", "HQ")];
    let mut devices = HashMap::new();
    devices.insert(
        "site-1".to_string(),
        (1..=150)
            .map(|i| {
                device(
                    &format!("dev-{i}"),
                    &format!("sw-{i}"),
                    &format!("94:2a:6f:{i:02x}:{i:02x}:{i:02x}"),
                )
            })
            .collect(),
    );
    let (app, controller) = controller_stub(sites, devices);
    let base = spawn(app).await;

    let client = UnifiClient::new(&base, "unifi-key");
    let sites = client.sites().await.unwrap();
    assert_eq!(sites.len(), 1);
    let devices = client.site_devices("site-1").await.unwrap();
    assert_eq!(devices.len(), 150); // page 1 (100) + page 2 (50) via totalCount

    assert!(controller.saw_api_key());
    assert_eq!(controller.device_offsets(), ["0", "100"]);
}

#[tokio::test]
async fn mapper_carries_mac_identity_and_verbatim_observed_fields() {
    let record = map_device(&device("dev-7", "HQ-Switch", "94:2A:6F:26:C6:CA")).unwrap();

    assert_eq!(record.source_system, "unifi");
    assert_eq!(record.external_id, "dev-7");
    assert_eq!(record.identity_keys["source_guid"], "dev-7");
    // MAC identity: normalized to lower colon form (ADR 0003 MAC class).
    assert_eq!(record.identity_keys["macs"], json!(["94:2a:6f:26:c6:ca"]));
    assert_eq!(record.identity_keys["hostname"], "hq-switch");
    // Observed tier: verbatim, only present keys.
    assert_eq!(record.observed_fields["state"], "ONLINE");
    assert_eq!(record.observed_fields["firmwareVersion"], "7.0.50");
    assert_eq!(record.observed_fields["model"], "USW-Pro-24-PoE");
    // No serial key in the payload → no serial identity (honest absence).
    assert!(record.identity_keys.get("serial").is_none());
}

#[tokio::test]
async fn mapper_normalizes_every_mac_format_deterministically() {
    // The same switch, three different MAC spellings — one identity.
    let records = ["94:2A:6F:26:C6:CA", "942a.6f26.c6ca", "942a6f26c6ca"]
        .map(|mac| map_device(&device("dev-7", "HQ-Switch", mac)).unwrap());
    let first = records[0].identity_keys["macs"].clone();
    for record in &records[1..] {
        assert_eq!(record.identity_keys["macs"], first);
    }
}

#[tokio::test]
async fn sync_feeds_per_site_and_records_interfaces_once() {
    let feed_writes = Arc::new(AtomicUsize::new(0));
    let writes = feed_writes.clone();
    let interface_writes: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
    let ifaces = interface_writes.clone();

    let app = Router::new()
        .route(
            "/api/v1/config-items/feed-upsert",
            post(move |Json(body): Json<Value>| {
                let writes = writes.clone();
                async move {
                    assert_eq!(body["identity_keys"]["macs"], json!(["94:2a:6f:26:c6:ca"]));
                    writes.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": Uuid::now_v7(),
                        "source_link": {
                            "id": Uuid::now_v7(),
                            "source_system": "unifi",
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
        )
        .route(
            "/api/v1/config-items/{id}/interfaces",
            post(move |Path(id): Path<Uuid>, Json(body): Json<Value>| {
                let ifaces = ifaces.clone();
                async move {
                    ifaces.lock().unwrap().push((id.to_string(), body.clone()));
                    Json(json!([body]))
                }
            }),
        );
    let base = spawn(app).await;

    let mapping = OrgMappingBuilder::new()
        .map("site-1", Uuid::now_v7())
        .build();
    let sink = MeridianFeederSink::new(MeridianFeederConfig {
        base_url: base.clone(),
        api_key: "mrd_test_key".into(),
    });
    let interfaces = MeridianInterfaceSink::new(base.clone(), "mrd_test_key");

    let (stub, _) = controller_stub(
        vec![site("site-1", "HQ"), site("site-2", "Branch")],
        HashMap::from([(
            "site-1".to_string(),
            vec![device("dev-1", "sw-1", "94:2a:6f:26:c6:ca")],
        )]),
    );
    let controller = spawn(stub).await;
    let client = UnifiClient::new(&controller, "k");
    let report = UnifiSync::run(&client, Some(&mapping), &sink, &interfaces)
        .await
        .unwrap();

    assert_eq!(report.report.total, 1);
    assert_eq!(report.report.synced, 1);
    assert_eq!(report.report.created, 1);
    assert_eq!(feed_writes.load(Ordering::SeqCst), 1);
    // Interfaces: main NIC (is_primary) + 2 ports, recorded exactly once.
    assert_eq!(report.interfaces_pushed, 3);
    let bodies = interface_writes.lock().unwrap();
    let names: Vec<&str> = bodies
        .iter()
        .map(|(_, b)| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["main", "Port 1", "Port 2"]);
    let main = &bodies[0].1;
    assert_eq!(main["mac"], "94:2a:6f:26:c6:ca");
    assert_eq!(main["ips"], json!(["192.168.1.10"]));
    assert_eq!(main["is_primary"], true);
}

#[tokio::test]
async fn unmapped_sites_are_reported_and_none_mode_counts() {
    let (stub, _) = controller_stub(
        vec![site("site-1", "HQ")],
        HashMap::from([(
            "site-1".to_string(),
            vec![device("dev-1", "sw-1", "94:2a:6f:26:c6:ca")],
        )]),
    );
    let controller = spawn(stub).await;
    let client = UnifiClient::new(&controller, "k");

    // No mapping at all: dry inventory — pulls happen, zero Meridian I/O.
    let report = UnifiSync::run(&client, None, &NoneFeederSink, &crate::NoneInterfaceSink)
        .await
        .unwrap();
    assert_eq!(report.report.total, 1);
    assert_eq!(report.report.would_sync, 1);
    assert_eq!(report.report.synced, 0);
    assert!(report.is_clean());

    // Mapping present, None sinks (none mode): every device counts
    // "would have synced", zero Meridian I/O.
    let mapping = OrgMappingBuilder::new()
        .map("site-1", Uuid::now_v7())
        .build();
    let report = UnifiSync::run(
        &client,
        Some(&mapping),
        &NoneFeederSink,
        &crate::NoneInterfaceSink,
    )
    .await
    .unwrap();
    assert_eq!(report.report.would_sync, 1);
    assert_eq!(report.report.synced, 0);

    // Unmapped site: REPORTED, never fed to a guessed client.
    let wrong_site = OrgMappingBuilder::new()
        .map("site-X", Uuid::now_v7())
        .build();
    let report = UnifiSync::run(
        &client,
        Some(&wrong_site),
        &NoneFeederSink,
        &crate::NoneInterfaceSink,
    )
    .await
    .unwrap();
    assert_eq!(report.report.errors.len(), 1);
    assert!(report.report.errors[0].error.contains("not mapped"));
}

// ---------------------------------------------------------------------------
// Alert leg: receiver verification + forwarder.
// ---------------------------------------------------------------------------

fn alert_payload() -> Value {
    json!({
        "_id": "alert-42",
        "key": "EVT_Lan_DeviceDisconnected",
        "msg": "Gateway disconnected from the LAN",
        "datetime": "2026-09-22T10:00:00Z",
        "meta": { "deviceName": "gw-hq", "site": "HQ" }
    })
}

#[test]
fn hop_signature_verifies_raw_bytes_before_parse() {
    let secret = b"hop-secret";
    let body = serde_json::to_vec(&alert_payload()).unwrap();
    let now = 1_700_000_000i64;

    // Valid fresh delivery.
    let sig = sign_delivery(secret, &body, now);
    assert!(sig.starts_with(&format!("t={now},v1=")));
    assert_eq!(verify_delivery(secret, &body, Some(&sig), now + 30), Ok(()));

    // Tampered bytes: same signature, different body → refused. The
    // signature covers bytes, not objects.
    let mut tampered = body.clone();
    tampered[0] ^= 0xff;
    assert_eq!(
        verify_delivery(secret, &tampered, Some(&sig), now + 30),
        Err("bad-signature")
    );

    // Outside the replay window.
    assert_eq!(
        verify_delivery(secret, &body, Some(&sig), now + REPLAY_TOLERANCE_SECS + 1),
        Err("outside-window")
    );

    // Malformed headers.
    assert_eq!(
        verify_delivery(secret, &body, None, now),
        Err("malformed-header")
    );
    assert_eq!(
        verify_delivery(secret, &body, Some("not-a-signature"), now),
        Err("malformed-header")
    );
    assert_eq!(
        verify_delivery(secret, &body, Some("t=abc,v1=zz"), now),
        Err("malformed-header")
    );

    // Wrong secret.
    assert_eq!(
        verify_delivery(b"other", &body, Some(&sig), now),
        Err("bad-signature")
    );
}

#[test]
fn unifi_alerts_map_onto_the_ingest_contract() {
    let alert = map_alert(&alert_payload()).unwrap();
    assert_eq!(alert.dedup_key, "alert-42");
    assert_eq!(alert.title, "Gateway disconnected from the LAN");
    // UniFi's alert stream carries no severity → warning default.
    assert_eq!(alert.severity, "warning");
    assert_eq!(alert.occurred_at.as_deref(), Some("2026-09-22T10:00:00Z"));
    assert_eq!(
        alert.attributes,
        Some(json!({ "deviceName": "gw-hq", "site": "HQ" }))
    );

    // Explicit level keys win.
    let mut critical = alert_payload();
    critical["level"] = json!("critical");
    assert_eq!(map_alert(&critical).unwrap().severity, "critical");

    // No id → key:datetime dedup key (deterministic).
    let mut no_id = alert_payload();
    no_id.as_object_mut().unwrap().remove("_id");
    let alert = map_alert(&no_id).unwrap();
    assert_eq!(
        alert.dedup_key,
        "EVT_Lan_DeviceDisconnected:2026-09-22T10:00:00Z"
    );

    // Unkeyable payload → None, never a fabricated alert.
    assert!(map_alert(&json!({ "unrelated": true })).is_none());
}

#[tokio::test]
async fn forwarder_signs_and_posts_the_exact_body() {
    let posted: Arc<Mutex<Option<(String, String, Vec<u8>)>>> = Arc::new(Mutex::new(None));
    let slot = posted.clone();
    let app = Router::new().route(
        "/api/v1/alerts/ingest/{source_ref}",
        post(
            move |Path(source_ref): Path<String>, headers: HeaderMap, body: axum::body::Bytes| {
                let slot = slot.clone();
                async move {
                    *slot.lock().unwrap() = Some((
                        source_ref,
                        headers
                            .get(SIGNATURE_HEADER)
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .to_string(),
                        body.to_vec(),
                    ));
                    Json(json!({
                        "status": "incident_created",
                        "alert": { "id": "alert-uuid-1" },
                        "incident": { "id": "ticket-uuid-1" }
                    }))
                    .into_response()
                }
            },
        ),
    );
    let base = spawn(app).await;

    let forwarder = AlertForwarder::new(&base, "unifi-ops", b"source-secret");
    let alert = map_alert(&alert_payload()).unwrap();
    let now = 1_700_000_000i64;
    let outcome = forwarder.forward(&alert, now).await.unwrap();
    assert_eq!(outcome.status, "incident_created");
    assert_eq!(outcome.alert_id, "alert-uuid-1");

    // The posted bytes are signable: the signature verifies against the
    // delivered body exactly the way the core's ingest endpoint verifies.
    let (source_ref, signature, body) = posted.lock().unwrap().take().unwrap();
    assert_eq!(source_ref, "unifi-ops");
    assert_eq!(
        verify_delivery(b"source-secret", &body, Some(&signature), now),
        Ok(())
    );
    // The wire body carries the ingest contract's required fields.
    let wire: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(wire["dedup_key"], "alert-42");
    assert_eq!(wire["title"], "Gateway disconnected from the LAN");
    assert_eq!(wire["severity"], "warning");
    assert_eq!(wire["occurred_at"], "2026-09-22T10:00:00Z");
}

#[tokio::test]
async fn rejected_ingests_map_to_typed_forward_errors() {
    async fn stub(status: axum::http::StatusCode, body: Value) -> String {
        let app = Router::new().route(
            "/api/v1/alerts/ingest/{source_ref}",
            post(move || async move { (status, Json(body.clone())).into_response() }),
        );
        spawn(app).await
    }

    let forwarder_at = |base: String| AlertForwarder::new(&base, "unifi-ops", b"secret");
    let alert = map_alert(&alert_payload()).unwrap();

    let base = stub(
        axum::http::StatusCode::UNAUTHORIZED,
        json!({"message": "invalid"}),
    )
    .await;
    assert!(matches!(
        forwarder_at(base).forward(&alert, 0).await,
        Err(ForwardError::Unauthorized(_))
    ));

    let base = stub(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        json!({"message": "bad shape"}),
    )
    .await;
    assert!(matches!(
        forwarder_at(base).forward(&alert, 0).await,
        Err(ForwardError::Rejected(_))
    ));
}
