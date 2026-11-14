//! UniFi connector conformance over contract-shaped stub servers:
//! `X-API-KEY` auth, offset/limit pagination with the totalCount
//! envelope, the device mapper's identity/observed fields, per-site
//! feeding + interface creation, none/dry modes, the alert receiver's
//! raw-bytes verification, and the signed ingest forwarder. No live
//! controller.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query};
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
use crate::{map_device, MarulaInterfaceSink, UnifiClient, UnifiSync};
use adapters_core::OrgMappingBuilder;
use ci_feeders::sink::{MarulaFeederConfig, MarulaFeederSink, NoneFeederSink};

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
    /// When set, a request carrying any other key gets 401 — the credential
    /// failure the per-client run must scope.
    expected_key: Option<String>,
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
        let expected = self.expected_key.as_deref().unwrap_or("unifi-key");
        let requests = self.requests.lock().unwrap();
        !requests.is_empty() && requests.iter().all(|(_, key, _)| key == expected)
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
    controller_stub_with_key(sites, devices_by_site, None)
}

fn controller_stub_with_key(
    sites: Vec<Value>,
    devices_by_site: HashMap<String, Vec<Value>>,
    expected_key: Option<&str>,
) -> (Router, Controller) {
    let controller = Controller {
        requests: Arc::new(Mutex::new(Vec::new())),
        sites: Arc::new(sites),
        devices: Arc::new(devices_by_site),
        expected_key: expected_key.map(str::to_string),
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
                        if state
                            .expected_key
                            .as_deref()
                            .is_some_and(|expected| expected != key)
                        {
                            return axum::http::StatusCode::UNAUTHORIZED.into_response();
                        }
                        Json(state.page(&state.sites, offset, limit)).into_response()
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
                        if state
                            .expected_key
                            .as_deref()
                            .is_some_and(|expected| expected != key)
                        {
                            return axum::http::StatusCode::UNAUTHORIZED.into_response();
                        }
                        let items = state.devices.get(&site_id).cloned().unwrap_or_default();
                        Json(state.page(&items, offset, limit)).into_response()
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
    let sink = MarulaFeederSink::new(MarulaFeederConfig {
        base_url: base.clone(),
        api_key: "mrl_test_key".into(),
    });
    let interfaces = MarulaInterfaceSink::new(base.clone(), "mrl_test_key");

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

    // No mapping at all: dry inventory — pulls happen, zero Marula I/O.
    let report = UnifiSync::run(&client, None, &NoneFeederSink, &crate::NoneInterfaceSink)
        .await
        .unwrap();
    assert_eq!(report.report.total, 1);
    assert_eq!(report.report.would_sync, 1);
    assert_eq!(report.report.synced, 0);
    assert!(report.is_clean());

    // Mapping present, None sinks (none mode): every device counts
    // "would have synced", zero Marula I/O.
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

/// What the loopback ingest route captured: (signature, content-type,
/// raw body). Named once so the assertion below reads as prose.
type CapturedPost = (String, String, Vec<u8>);

#[tokio::test]
async fn forwarder_signs_and_posts_the_exact_body() {
    let posted: Arc<Mutex<Option<CapturedPost>>> = Arc::new(Mutex::new(None));
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

// ---------------------------------------------------------------------------
// G11: per-client controllers.
// ---------------------------------------------------------------------------

#[test]
fn a_client_scoped_connection_wins_and_the_org_connection_is_the_fallback() {
    let client = Uuid::from_u128(1);
    let other = Uuid::from_u128(2);
    let org = crate::ControllerConnection {
        connection_id: Uuid::from_u128(10),
        scope_client_id: None,
        label: "org controller".into(),
        base_url: "https://org.example".into(),
        api_key: "org-key".into(),
    };
    let own = crate::ControllerConnection {
        connection_id: Uuid::from_u128(11),
        scope_client_id: Some(client),
        label: "client controller".into(),
        base_url: "https://client.example".into(),
        api_key: "client-key".into(),
    };
    let elsewhere = crate::ControllerConnection {
        connection_id: Uuid::from_u128(12),
        scope_client_id: Some(other),
        label: "another client".into(),
        base_url: "https://other.example".into(),
        api_key: "other-key".into(),
    };

    let rows = vec![org.clone(), own.clone(), elsewhere];
    assert_eq!(
        crate::connection_for_client(&rows, client).map(|c| c.connection_id),
        Some(own.connection_id),
        "the client's own controller wins"
    );
    assert_eq!(
        crate::connection_for_client(&rows, other).map(|c| c.connection_id),
        Some(Uuid::from_u128(12)),
        "a client-scoped row is never borrowed for another client"
    );
    assert_eq!(
        crate::connection_for_client(&rows, Uuid::from_u128(3)).map(|c| c.connection_id),
        Some(org.connection_id),
        "the org-scoped connection is the fallback"
    );
    assert!(crate::connection_for_client(&rows[1..], Uuid::from_u128(3)).is_none());

    // The row's own settings name the origin the manifest's base_url_key reads.
    let built = crate::ControllerConnection::from_settings(
        Uuid::from_u128(20),
        Some(client),
        "from a row",
        &json!({ "controller_url": "https://acme.hostifi.example/", "site": "acme" }),
        "sealed-key",
    )
    .expect("a row with an origin");
    assert_eq!(built.base_url, "https://acme.hostifi.example");
    assert!(crate::ControllerConnection::from_settings(
        Uuid::from_u128(21),
        Some(client),
        "no origin",
        &json!({ "site": "acme" }),
        "sealed-key",
    )
    .is_none());
}

#[tokio::test]
async fn two_clients_on_two_controllers_sync_with_their_own_credentials() {
    let client_a = Uuid::from_u128(0xA1);
    let client_b = Uuid::from_u128(0xB2);
    let feed: Arc<Mutex<Vec<(String, Uuid)>>> = Arc::new(Mutex::new(Vec::new()));
    let feed_state = feed.clone();

    let app = Router::new().route(
        "/api/v1/config-items/feed-upsert",
        post(move |Json(body): Json<Value>| {
            let feed = feed_state.clone();
            async move {
                feed.lock().unwrap().push((
                    body["external_id"].as_str().unwrap().to_string(),
                    body["client_id"].as_str().unwrap().parse().unwrap(),
                ));
                Json(json!({
                    "id": Uuid::now_v7(),
                    "source_link": {
                        "id": Uuid::now_v7(),
                        "source_system": "unifi",
                        "external_id": body["external_id"]
                    },
                    "created_item": false,
                    "resolved_existing": true,
                    "auto_merged": false,
                    "queued_candidates": [],
                    "contradictions": []
                }))
            }
        }),
    );
    let marula = spawn(app).await;

    // Client A: Hostifi-managed controller, key-a.
    let (a_app, a_stub) = controller_stub_with_key(
        vec![site("site-a", "Acme HQ")],
        HashMap::from([(
            "site-a".to_string(),
            vec![device("dev-a", "sw-a", "94:2a:6f:00:00:0a")],
        )]),
        Some("key-a"),
    );
    let a_base = spawn(a_app).await;
    // Client B: on-prem controller, key-b.
    let (b_app, b_stub) = controller_stub_with_key(
        vec![site("site-b", "Contoso DC")],
        HashMap::from([(
            "site-b".to_string(),
            vec![device("dev-b", "sw-b", "94:2a:6f:00:00:0b")],
        )]),
        Some("key-b"),
    );
    let b_base = spawn(b_app).await;

    let connections = vec![
        crate::ControllerConnection {
            connection_id: Uuid::from_u128(0xC1),
            scope_client_id: Some(client_a),
            label: "Hostifi — Acme".into(),
            base_url: a_base,
            api_key: "key-a".into(),
        },
        crate::ControllerConnection {
            connection_id: Uuid::from_u128(0xC2),
            scope_client_id: Some(client_b),
            label: "On-prem — Contoso".into(),
            base_url: b_base,
            api_key: "key-b".into(),
        },
    ];
    let clients = vec![
        crate::ClientSites {
            client_id: client_a,
            site_ids: vec!["site-a".into()],
        },
        crate::ClientSites {
            client_id: client_b,
            site_ids: vec!["site-b".into()],
        },
    ];

    let sink = MarulaFeederSink::new(MarulaFeederConfig {
        base_url: marula,
        api_key: "mrl_test_key".into(),
    });
    let report =
        crate::UnifiSync::run_scoped(&connections, &clients, &sink, &crate::NoneInterfaceSink)
            .await;

    assert_eq!(report.report.synced, 2, "report: {report:?}");
    assert_eq!(report.clients.len(), 2);
    assert!(report.clients.iter().all(|c| c.error.is_none()));
    assert_eq!(report.clients[0].devices, 1);
    assert_eq!(report.clients[1].devices, 1);

    // Each controller saw its own key, and each device fed its own client.
    assert!(a_stub.saw_api_key());
    assert!(b_stub.saw_api_key());
    let fed = feed.lock().unwrap().clone();
    assert!(fed.contains(&("dev-a".to_string(), client_a)));
    assert!(fed.contains(&("dev-b".to_string(), client_b)));
}

#[tokio::test]
async fn one_clients_credential_failure_does_not_stop_the_other() {
    let client_a = Uuid::from_u128(0xA1);
    let client_b = Uuid::from_u128(0xB2);
    let fed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fed_state = fed.clone();

    let app = Router::new().route(
        "/api/v1/config-items/feed-upsert",
        post(move |Json(body): Json<Value>| {
            let fed = fed_state.clone();
            async move {
                fed.lock()
                    .unwrap()
                    .push(body["external_id"].as_str().unwrap().to_string());
                Json(json!({
                    "id": Uuid::now_v7(),
                    "created_item": false,
                    "resolved_existing": true,
                    "queued_candidates": []
                }))
            }
        }),
    );
    let marula = spawn(app).await;

    let (a_app, _) = controller_stub_with_key(
        vec![site("site-a", "Acme HQ")],
        HashMap::from([(
            "site-a".to_string(),
            vec![device("dev-a", "sw-a", "94:2a:6f:00:00:0a")],
        )]),
        Some("key-a"),
    );
    let a_base = spawn(a_app).await;
    // Client B's connection carries a stale key: the controller refuses it.
    let (b_app, _) = controller_stub_with_key(
        vec![site("site-b", "Contoso DC")],
        HashMap::new(),
        Some("key-b"),
    );
    let b_base = spawn(b_app).await;

    let connections = vec![
        crate::ControllerConnection {
            connection_id: Uuid::from_u128(0xC1),
            scope_client_id: Some(client_a),
            label: "Hostifi — Acme".into(),
            base_url: a_base,
            api_key: "key-a".into(),
        },
        crate::ControllerConnection {
            connection_id: Uuid::from_u128(0xC2),
            scope_client_id: Some(client_b),
            label: "On-prem — Contoso".into(),
            base_url: b_base,
            api_key: "stale-key".into(),
        },
    ];
    let clients = vec![
        crate::ClientSites {
            client_id: client_a,
            site_ids: vec!["site-a".into()],
        },
        crate::ClientSites {
            client_id: client_b,
            site_ids: vec!["site-b".into()],
        },
    ];

    let sink = MarulaFeederSink::new(MarulaFeederConfig {
        base_url: marula,
        api_key: "mrl_test_key".into(),
    });
    let report =
        crate::UnifiSync::run_scoped(&connections, &clients, &sink, &crate::NoneInterfaceSink)
            .await;

    // The healthy client synced; the refused one is scoped and named.
    assert_eq!(fed.lock().unwrap().clone(), vec!["dev-a".to_string()]);
    assert_eq!(report.report.synced, 1);
    assert_eq!(report.clients[0].error, None);
    let failed = &report.clients[1];
    assert_eq!(failed.client_id, client_b);
    assert_eq!(failed.connection_id, Uuid::from_u128(0xC2));
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|e| e.contains("unauthorized")),
        "scoped error: {failed:?}"
    );
    assert!(report
        .report
        .errors
        .iter()
        .any(|e| e.external_id == format!("client:{client_b}")));
}

#[test]
fn the_shipped_manifest_declares_the_unifi_deep_link() {
    let manifest: Value =
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest parses");
    assert_eq!(manifest["product_kinds"], json!(["network"]));
    let surfaces = &manifest["connector_surfaces"];
    assert_eq!(surfaces["base_url_key"], json!("controller_url"));
    let block = &surfaces["external_refs"][0];
    assert_eq!(block["record"], json!("configuration"));
    assert_eq!(block["external_kind"], json!("device"));
    let open_url = block["open_url"].as_str().expect("open_url");
    assert!(
        open_url.starts_with("{base_url}"),
        "origin comes from the connection"
    );
    assert!(open_url.contains("{settings.site}"));
    assert!(open_url.contains("{external_id}"));
    // The key the template reads exists in the schema it ships with.
    let properties = &manifest["settings_schema"]["properties"];
    for key in ["controller_url", "site"] {
        assert!(properties.get(key).is_some(), "{key} must be declared");
    }
}
