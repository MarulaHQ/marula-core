//! Conformance tests for the Cloudflare connector against a deterministic
//! in-process mock of the Cloudflare REST API v4 (the lane-8 pattern:
//! contract-shaped stub servers, no network, no live credentials).
//!
//! The mock holds a small zone + record store in memory, records every call
//! path it saw, and can refuse requests with 403 to reproduce a token that
//! lacks the required scope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use adapters_core::dns::{
    candidate_zone_names, normalize_domain, verification_record_name, DnsProviderError,
    DnsRecordKind, DnsRecordProvider, EdgeRecordSpec, NoneDnsRecordProvider,
    VERIFICATION_TXT_PREFIX,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::api::CloudflareConfig;
use crate::dns::CloudflareDnsClient;
use crate::posture::{CloudflareEdgePosture, OriginCaCertificate, SslMode};

const ZONE_ID: &str = "zone-abc123";
const TOKEN: &str = "cf-token-scoped-to-dns-edit";
const DOMAIN: &str = "support.msp.example.com";

type ApiState = Arc<Mutex<MockState>>;

#[derive(Default, Clone)]
struct MockState {
    zones: Vec<Value>,
    records: Vec<Value>,
    ssl_mode: String,
    certificates: Vec<Value>,
    deny_all: bool,
    calls: Vec<String>,
}

fn zone_fixture(id: &str, name: &str, status: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "status": status,
        "paused": false,
        "type": "full",
        "name_servers": ["ns1.example.com", "ns2.example.com"],
    })
}

fn record_fixture(
    id: &str,
    kind: &str,
    name: &str,
    content: &str,
    ttl: u32,
    proxied: bool,
) -> Value {
    let proxiable = matches!(kind, "A" | "AAAA" | "CNAME");
    json!({
        "id": id,
        "zone_id": ZONE_ID,
        "type": kind,
        "name": name,
        "content": content,
        "ttl": ttl,
        "proxied": proxied,
        "proxiable": proxiable,
    })
}

fn ok(body: Value) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(body))
}

fn refused() -> (StatusCode, Json<Value>) {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "success": false,
            "errors": [{"code": 9109, "message": "Unauthorized to access requested resource"}],
            "messages": [],
            "result": null,
        })),
    )
}

fn envelope(result: Value) -> (StatusCode, Json<Value>) {
    ok(json!({
        "success": true,
        "errors": [],
        "messages": [],
        "result": result,
    }))
}

async fn zones_handler(
    State(state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let wanted = query.get("name").cloned().unwrap_or_default();
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard.calls.push("GET /zones DENIED".to_string());
        return refused();
    }
    guard.calls.push(format!("GET /zones name={wanted}"));
    let zones: Vec<Value> = guard
        .zones
        .iter()
        .filter(|zone| {
            zone.get("name")
                .and_then(Value::as_str)
                .map(|name| name.eq_ignore_ascii_case(&wanted))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    envelope(zones.into())
}

async fn list_records_handler(
    State(state): State<ApiState>,
    Path(zone_id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let kind_filter = query.get("type").cloned().unwrap_or_default();
    let name_filter = query.get("name").cloned().unwrap_or_default();
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard
            .calls
            .push(format!("GET /zones/{zone_id}/dns_records DENIED"));
        return refused();
    }
    guard.calls.push(format!(
        "GET /zones/{zone_id}/dns_records type={kind_filter} name={name_filter}"
    ));
    let records: Vec<Value> = guard
        .records
        .iter()
        .filter(|record| {
            let kind_ok = kind_filter.is_empty()
                || record
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|kind| kind.eq_ignore_ascii_case(&kind_filter))
                    .unwrap_or(false);
            let name_ok = name_filter.is_empty()
                || record
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| name.eq_ignore_ascii_case(&name_filter))
                    .unwrap_or(false);
            kind_ok && name_ok
        })
        .cloned()
        .collect();
    envelope(records.into())
}

async fn create_record_handler(
    State(state): State<ApiState>,
    Path(zone_id): Path<String>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard
            .calls
            .push(format!("POST /zones/{zone_id}/dns_records DENIED"));
        return refused();
    }
    guard
        .calls
        .push(format!("POST /zones/{zone_id}/dns_records"));
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("TXT")
        .to_string();
    let proxied = payload
        .get("proxied")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let record = json!({
        "id": format!("rec-{}", guard.records.len() + 1),
        "zone_id": zone_id,
        "type": kind,
        "name": payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase(),
        "content": payload.get("content").cloned().unwrap_or(Value::Null),
        "ttl": payload.get("ttl").and_then(Value::as_u64).unwrap_or(1),
        "proxied": proxied,
        "proxiable": proxied || matches!(kind.as_str(), "A" | "AAAA" | "CNAME"),
    });
    guard.records.push(record.clone());
    envelope(record)
}

async fn update_record_handler(
    State(state): State<ApiState>,
    Path((zone_id, record_id)): Path<(String, String)>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard.calls.push(format!(
            "PUT /zones/{zone_id}/dns_records/{record_id} DENIED"
        ));
        return refused();
    }
    let mut updated: Option<Value> = None;
    for slot in guard.records.iter_mut() {
        if slot.get("id").and_then(Value::as_str) == Some(record_id.as_str()) {
            if let Some(object) = slot.as_object_mut() {
                object.insert(
                    "type".to_string(),
                    json!(payload
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("TXT")
                        .to_string()),
                );
                object.insert(
                    "name".to_string(),
                    json!(payload
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_ascii_lowercase()),
                );
                object.insert(
                    "content".to_string(),
                    payload.get("content").cloned().unwrap_or(Value::Null),
                );
                object.insert(
                    "ttl".to_string(),
                    json!(payload.get("ttl").and_then(Value::as_u64).unwrap_or(1)),
                );
                object.insert(
                    "proxied".to_string(),
                    json!(payload
                        .get("proxied")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)),
                );
            }
            updated = Some(slot.clone());
            break;
        }
    }
    guard
        .calls
        .push(format!("PUT /zones/{zone_id}/dns_records/{record_id}"));
    match updated {
        Some(record) => envelope(record),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "errors": [{"code": 1010, "message": "record not found"}],
                "result": null,
            })),
        ),
    }
}

async fn delete_record_handler(
    State(state): State<ApiState>,
    Path((zone_id, record_id)): Path<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard.calls.push(format!(
            "DELETE /zones/{zone_id}/dns_records/{record_id} DENIED"
        ));
        return refused();
    }
    guard
        .calls
        .push(format!("DELETE /zones/{zone_id}/dns_records/{record_id}"));
    guard
        .records
        .retain(|record| record.get("id").and_then(Value::as_str) != Some(record_id.as_str()));
    envelope(json!({"id": record_id}))
}

async fn ssl_handler(
    State(state): State<ApiState>,
    Path(zone_id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard
            .calls
            .push(format!("GET /zones/{zone_id}/settings/ssl DENIED"));
        return refused();
    }
    guard
        .calls
        .push(format!("GET /zones/{zone_id}/settings/ssl"));
    let mode = guard.ssl_mode.clone();
    envelope(json!({"id": "ssl", "value": mode, "editable": true}))
}

async fn certificates_handler(
    State(state): State<ApiState>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let zone = query.get("zone_id").cloned().unwrap_or_default();
    let mut guard = state.lock().unwrap();
    if guard.deny_all {
        guard
            .calls
            .push(format!("GET /certificates zone_id={zone} DENIED"));
        return refused();
    }
    guard
        .calls
        .push(format!("GET /certificates zone_id={zone}"));
    let certificates = guard.certificates.clone();
    envelope(certificates.into())
}

fn router(state: ApiState) -> Router {
    Router::new()
        .route("/zones", get(zones_handler))
        .route(
            "/zones/{zone_id}/dns_records",
            get(list_records_handler).post(create_record_handler),
        )
        .route(
            "/zones/{zone_id}/dns_records/{record_id}",
            put(update_record_handler).delete(delete_record_handler),
        )
        .route("/zones/{zone_id}/settings/ssl", get(ssl_handler))
        .route("/certificates", get(certificates_handler))
        .with_state(state)
}

async fn spawn_mock(state: MockState) -> (String, ApiState) {
    let shared = Arc::new(Mutex::new(state));
    let seen = shared.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(shared)).await.unwrap() });
    (format!("http://{addr}"), seen)
}

fn client_for(base: String) -> CloudflareDnsClient {
    CloudflareDnsClient::new(&CloudflareConfig::new(TOKEN).with_base_url(base))
}

fn posture_for(base: String) -> CloudflareEdgePosture {
    CloudflareEdgePosture::new(&CloudflareConfig::new(TOKEN).with_base_url(base))
}

fn calls_seen(seen: &ApiState) -> Vec<String> {
    seen.lock().unwrap().calls.clone()
}

fn records_seen(seen: &ApiState) -> Vec<Value> {
    seen.lock().unwrap().records.clone()
}

fn live_mock() -> MockState {
    MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "active")],
        ssl_mode: "strict".to_string(),
        certificates: vec![json!({
            "id": "cert-origin-001",
            "hostnames": ["msp.example.com", "*.example.com"],
            "expires_on": "2030-01-01T00:00:00Z",
            "request_type": "origin-rsa",
            "revoked_at": null,
        })],
        ..Default::default()
    }
}

// ------------------------------------------------------------- zone discovery

#[tokio::test]
async fn zone_discovery_walks_parent_labels_down_to_the_apex_zone() {
    let (base, seen) = spawn_mock(live_mock()).await;

    let zone = client_for(base).discover_zone(DOMAIN).await.unwrap();

    assert_eq!(zone.id, ZONE_ID);
    assert_eq!(zone.name, "example.com");
    assert_eq!(zone.status, "active");

    let queried: Vec<String> = calls_seen(&seen)
        .into_iter()
        .filter(|call| call.starts_with("GET /zones "))
        .collect();
    assert_eq!(
        queried.len(),
        3,
        "walk every parent label to the apex: {queried:?}"
    );
    assert!(queried[0].contains("name=support.msp.example.com"));
    assert!(queried[1].contains("name=msp.example.com"));
    assert!(queried[2].contains("name=example.com"));
}

#[tokio::test]
async fn zone_discovery_refuses_with_a_typed_error_when_no_zone_matches() {
    let (base, _seen) = spawn_mock(MockState::default()).await;

    let err = client_for(base).discover_zone(DOMAIN).await.unwrap_err();

    assert_eq!(
        err,
        DnsProviderError::ZoneNotFound {
            domain: DOMAIN.to_string(),
            tried: candidate_zone_names(DOMAIN).unwrap()
        }
    );
}

#[tokio::test]
async fn zone_discovery_reports_a_non_active_zone_readably() {
    let (base, _seen) = spawn_mock(MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "pending")],
        ..Default::default()
    })
    .await;

    let err = client_for(base).discover_zone(DOMAIN).await.unwrap_err();

    let message = err.to_string();
    assert!(message.contains("example.com"), "{message}");
    assert!(message.contains("pending"), "{message}");
}

// ---------------------------------------------------- verification TXT records

#[tokio::test]
async fn verification_txt_is_created_unproxied() {
    let (base, seen) = spawn_mock(live_mock()).await;

    let state = client_for(base)
        .upsert_txt(DOMAIN, "marula-verify-token")
        .await
        .unwrap();

    assert_eq!(
        state.name,
        format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}"),
        "the verification record is always _marula-verify.<domain>"
    );
    assert_eq!(state.kind, DnsRecordKind::Txt);
    assert_eq!(state.content, "marula-verify-token");
    assert_eq!(state.ttl, 300);
    assert!(!state.proxied, "TXT is never orange-cloud proxied");
    assert!(!state.proxiable);
    assert_eq!(state.zone_id, ZONE_ID);

    let stored = records_seen(&seen);
    assert_eq!(stored.len(), 1, "exactly one TXT row is created");
    assert_eq!(stored[0]["proxied"], false);
}

#[tokio::test]
async fn verification_txt_upsert_is_idempotent_on_rerun() {
    let (base, seen) = spawn_mock(live_mock()).await;
    let dns = client_for(base);

    let first = dns.upsert_txt(DOMAIN, "marula-verify-token").await.unwrap();
    let second = dns.upsert_txt(DOMAIN, "marula-verify-token").await.unwrap();

    assert_eq!(
        first, second,
        "a re-run returns the same record, not a copy"
    );
    assert_eq!(records_seen(&seen).len(), 1, "the re-run writes nothing");
    assert_eq!(
        calls_seen(&seen)
            .iter()
            .filter(|call| call.starts_with("POST /zones/"))
            .count(),
        1
    );
}

#[tokio::test]
async fn verification_txt_upsert_updates_a_rotated_token_in_place() {
    let (base, seen) = spawn_mock(MockState {
        records: vec![record_fixture(
            "rec-existing",
            "TXT",
            &format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}"),
            "old-token",
            300,
            false,
        )],
        ..live_mock()
    })
    .await;

    let state = client_for(base)
        .upsert_txt(DOMAIN, "rotated-token")
        .await
        .unwrap();

    assert_eq!(
        state.id, "rec-existing",
        "the row is updated, not duplicated"
    );
    assert_eq!(state.content, "rotated-token");
    let stored = records_seen(&seen);
    assert_eq!(stored.len(), 1, "no duplicate _marula-verify row");
    assert!(calls_seen(&seen)
        .iter()
        .any(|call| call.starts_with("PUT /zones/zone-abc123/dns_records/rec-existing")));
}

#[tokio::test]
async fn verification_txt_removal_is_idempotent_when_absent() {
    let (base, seen) = spawn_mock(live_mock()).await;
    let dns = client_for(base);

    assert!(!dns.remove_txt(DOMAIN).await.unwrap(), "absent ⇒ no-op");

    dns.upsert_txt(DOMAIN, "marula-verify-token").await.unwrap();
    assert!(dns.remove_txt(DOMAIN).await.unwrap(), "present ⇒ removed");
    assert!(
        !dns.remove_txt(DOMAIN).await.unwrap(),
        "removed already ⇒ no-op"
    );
    assert_eq!(records_seen(&seen).len(), 0, "the row is gone");
}

#[tokio::test]
async fn verification_txt_ignores_presentation_quotes_in_the_token() {
    let (base, _seen) = spawn_mock(live_mock()).await;

    let state = client_for(base)
        .upsert_txt(DOMAIN, "\"marula-verify-token\"")
        .await
        .unwrap();

    assert_eq!(state.content, "marula-verify-token");
}

// ------------------------------------------------------------ edge A/AAAA/CNAME

#[tokio::test]
async fn edge_cname_upsert_reports_orange_cloud_proxied_and_ttl_one() {
    let (base, seen) = spawn_mock(live_mock()).await;

    let state = client_for(base)
        .upsert_edge(&EdgeRecordSpec::cname(DOMAIN, "edge.marula.example", true))
        .await
        .unwrap();

    assert!(
        state.proxied,
        "orange cloud is on: the state must report it"
    );
    assert_eq!(state.ttl, 1, "Cloudflare forces ttl 1 on proxied records");
    assert_eq!(state.content, "edge.marula.example");
    assert_eq!(state.kind, DnsRecordKind::Cname);

    let stored = records_seen(&seen);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["proxied"], true);
    assert_eq!(stored[0]["ttl"], 1);
}

#[tokio::test]
async fn edge_a_record_upsert_dns_only_reports_proxied_false() {
    let (base, seen) = spawn_mock(live_mock()).await;

    let state = client_for(base)
        .upsert_edge(&EdgeRecordSpec::a(DOMAIN, "203.0.113.10", false))
        .await
        .unwrap();

    assert!(!state.proxied, "DNS-only: the state must report it");
    assert_eq!(state.ttl, 300, "operator default TTL for DNS-only records");
    assert_eq!(state.kind, DnsRecordKind::A);

    let stored = records_seen(&seen);
    assert_eq!(stored[0]["proxied"], false);
}

#[tokio::test]
async fn edge_aaaa_upsert_and_edge_upsert_idempotency() {
    let (base, seen) = spawn_mock(live_mock()).await;
    let dns = client_for(base);

    let spec = EdgeRecordSpec::aaaa(DOMAIN, "2001:db8::10", true);
    let first = dns.upsert_edge(&spec).await.unwrap();
    let second = dns.upsert_edge(&spec).await.unwrap();

    assert_eq!(first, second, "same proxy mode ⇒ no write on the re-run");
    assert_eq!(records_seen(&seen).len(), 1);
    assert_eq!(
        calls_seen(&seen)
            .iter()
            .filter(|call| call.starts_with("POST /zones/"))
            .count(),
        1
    );

    // Switching the proxy mode rewrites the same row.
    let dns_only = dns
        .upsert_edge(&EdgeRecordSpec::aaaa(DOMAIN, "2001:db8::10", false))
        .await
        .unwrap();
    assert_eq!(dns_only.id, first.id);
    assert!(!dns_only.proxied);
    assert_eq!(dns_only.ttl, 300);
    assert_eq!(
        records_seen(&seen).len(),
        1,
        "the row flips, never duplicates"
    );
}

#[tokio::test]
async fn verification_txt_stays_unproxied_when_the_edge_record_is_proxied() {
    let (base, seen) = spawn_mock(live_mock()).await;
    let dns = client_for(base);

    dns.upsert_edge(&EdgeRecordSpec::a(DOMAIN, "203.0.113.10", true))
        .await
        .unwrap();
    let txt = dns.upsert_txt(DOMAIN, "marula-verify-token").await.unwrap();

    assert!(
        !txt.proxied,
        "TXT remains unproxied next to a proxied A record"
    );
    let stored = records_seen(&seen);
    let txt_row = stored
        .iter()
        .find(|record| record.get("type").and_then(Value::as_str) == Some("TXT"))
        .expect("TXT row present");
    assert_eq!(txt_row["proxied"], false);
}

// ------------------------------------------------------ credential / scope refusal

#[tokio::test]
async fn token_scope_refusal_maps_to_a_typed_authorization_error() {
    let (base, _seen) = spawn_mock(MockState {
        deny_all: true,
        ..live_mock()
    })
    .await;

    // The Cloudflare-native method surfaces the scope in its own message.
    let err = client_for(base)
        .discover_zone_for(DOMAIN)
        .await
        .unwrap_err();

    assert_eq!(
        err,
        crate::api::CloudflareError::Unauthorized {
            operation: "zone discovery".to_string(),
            detail: "Unauthorized to access requested resource".to_string()
        }
    );
    let message = err.to_string();
    assert!(message.contains("Zone:DNS:Edit"), "{message}");
}

#[tokio::test]
async fn token_scope_refusal_surfaces_through_the_provider_contract() {
    let (base, _seen) = spawn_mock(MockState {
        deny_all: true,
        ..live_mock()
    })
    .await;

    let dns = client_for(base);
    let provider: &dyn DnsRecordProvider = &dns;
    let err = provider
        .upsert_verification_txt(DOMAIN, "marula-verify-token")
        .await
        .unwrap_err();

    match err {
        DnsProviderError::UnauthorizedScope { operation, detail } => {
            assert_eq!(operation, "zone discovery");
            assert!(detail.contains("Unauthorized"), "{detail}");
        }
        other => panic!("expected UnauthorizedScope, got {other:?}"),
    }
}

// ------------------------------------------------------------- edge posture (V2)

#[tokio::test]
async fn edge_posture_reports_strict_as_v2_ready_without_warnings() {
    let (base, _seen) = spawn_mock(MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "active")],
        ssl_mode: "strict".to_string(),
        certificates: vec![json!({
            "id": "cert-origin-001",
            "hostnames": ["*.example.com"],
            "expires_on": "2030-01-01T00:00:00Z",
            "revoked_at": null,
        })],
        records: vec![record_fixture(
            "rec-edge",
            "A",
            DOMAIN,
            "203.0.113.10",
            1,
            true,
        )],
        ..Default::default()
    })
    .await;

    let report = posture_for(base)
        .inspect_edge_posture(DOMAIN)
        .await
        .unwrap();

    assert_eq!(report.zone.id, ZONE_ID);
    assert!(report.ssl.meets_v2_strict_requirement);
    assert!(report.hostname_proxied(), "the A record is orange-cloud");
    assert!(report.has_active_origin_ca_for_hostname());
    assert!(report.v2_edge_mode_ready());
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(report.checklist().iter().all(|(_, passed, _)| *passed));
    assert_eq!(report.checklist().len(), 3);
}

#[tokio::test]
async fn edge_posture_warns_when_the_zone_is_flexible() {
    let (base, _seen) = spawn_mock(MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "active")],
        ssl_mode: "flexible".into(),
        certificates: vec![json!({
            "id": "cert-origin-001",
            "hostnames": [DOMAIN],
            "expires_on": "2030-01-01T00:00:00Z",
            "revoked_at": null,
        })],
        records: vec![record_fixture(
            "rec-edge",
            "CNAME",
            DOMAIN,
            "edge.marula.example",
            1,
            true,
        )],
        ..Default::default()
    })
    .await;

    let report = posture_for(base)
        .inspect_edge_posture(DOMAIN)
        .await
        .unwrap();

    assert_eq!(report.ssl.mode, SslMode::Flexible);
    assert!(!report.ssl.meets_v2_strict_requirement);
    assert!(!report.v2_edge_mode_ready());
    let joined = report.warnings.join(" | ");
    assert!(joined.contains("flexible"), "{joined}");
    assert!(joined.contains("reverse-proxy-tls.md"), "{joined}");
    let first = report.checklist().first().cloned().expect("ssl row");
    assert!(!first.1);
}

#[tokio::test]
async fn edge_posture_parses_the_origin_ca_inventory_and_skips_revoked_certificates() {
    let (base, _seen) = spawn_mock(MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "active")],
        ssl_mode: "strict".into(),
        certificates: vec![
            json!({
                "id": "cert-revoked",
                "hostnames": [DOMAIN],
                "expires_on": "2031-01-01T00:00:00Z",
                "revoked_at": "2029-06-01T00:00:00Z",
            }),
            json!({
                "id": "cert-live",
                "hostnames": ["example.com"],
                "expires_on": "2030-01-01T00:00:00Z",
                "request_type": "origin-rsa",
                "revoked_at": null,
            }),
        ],
        ..Default::default()
    })
    .await;

    let certs = posture_for(base.clone())
        .list_origin_ca_certificates(ZONE_ID)
        .await
        .unwrap();

    assert_eq!(certs.len(), 2);
    assert!(certs[0].is_revoked());
    assert!(!certs[1].is_revoked());
    assert_eq!(certs[1].expires_on.as_deref(), Some("2030-01-01T00:00:00Z"));
    assert!(
        !certs[0].covers_hostname(DOMAIN),
        "revoked certs do not count"
    );
    assert!(
        !certs[1].covers_hostname(DOMAIN),
        "the hostname is a subdomain"
    );

    let report = posture_for(base)
        .inspect_edge_posture(DOMAIN)
        .await
        .unwrap();
    assert!(!report.has_active_origin_ca_for_hostname());
    assert!(!report.v2_edge_mode_ready());
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("Origin CA")));
}

#[tokio::test]
async fn edge_posture_sees_a_dns_only_hostname_as_unproxied() {
    let (base, _seen) = spawn_mock(MockState {
        zones: vec![zone_fixture(ZONE_ID, "example.com", "active")],
        ssl_mode: "strict".into(),
        records: vec![record_fixture(
            "rec-edge",
            "A",
            DOMAIN,
            "203.0.113.10",
            300,
            false,
        )],
        ..Default::default()
    })
    .await;

    let report = posture_for(base)
        .inspect_edge_posture(DOMAIN)
        .await
        .unwrap();

    assert!(!report.hostname_proxied());
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("orange-cloud")));
}

#[tokio::test]
async fn edge_posture_ssl_read_is_refused_for_a_token_without_the_read_scope() {
    let (base, _seen) = spawn_mock(MockState {
        deny_all: true,
        ..live_mock()
    })
    .await;

    let err = posture_for(base)
        .inspect_zone_ssl(ZONE_ID)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        crate::api::CloudflareError::Unauthorized { .. }
    ));
    let message = err.to_string();
    assert!(message.contains("Zone:Zone Settings:Read"), "{message}");
}

// ---------------------------------------------------------------- none-mode (D)

#[tokio::test]
async fn none_mode_instructions_round_trip_through_the_generic_verifier() {
    let instructions = NoneDnsRecordProvider::manual_instructions(
        DOMAIN,
        "marula-verify-token",
        Some(&EdgeRecordSpec::cname(DOMAIN, "edge.marula.example", true)),
    )
    .unwrap();

    assert_eq!(
        instructions.verification_txt.name,
        format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}")
    );
    assert_eq!(instructions.verification_txt.kind, DnsRecordKind::Txt);
    assert_eq!(instructions.verification_txt.value, "marula-verify-token");
    assert!(instructions.verification_txt.notes.contains("unproxied"));

    let edge = instructions.edge_record.expect("edge instructions");
    assert_eq!(edge.kind, DnsRecordKind::Cname);
    assert_eq!(edge.value, "edge.marula.example");
    assert_eq!(edge.ttl, 1, "proxied edge records carry the automatic TTL");

    // What Core's system-resolver verifier sees when the operator pastes the
    // records at any DNS host.
    let resolved: Vec<String> = vec![instructions.verification_txt.value.clone()];
    assert!(NoneDnsRecordProvider::verify_txt_records(
        &resolved,
        "marula-verify-token"
    ));
    assert!(!NoneDnsRecordProvider::verify_txt_records(
        &resolved,
        "wrong-token"
    ));

    // Resolvers that hand back presentation-quoted TXT strings verify too.
    let quoted: Vec<String> = vec![
        "\"marula-verify-token\"".to_string(),
        "unrelated-other-record".to_string(),
    ];
    assert!(NoneDnsRecordProvider::verify_txt_records(
        &quoted,
        "marula-verify-token"
    ));

    // An empty token never verifies: a challenge-less answer is a failure.
    assert!(!NoneDnsRecordProvider::verify_txt_records(&resolved, "   "));
}

#[tokio::test]
async fn none_mode_provider_satisfies_the_contract_without_any_network() {
    let provider = NoneDnsRecordProvider::new();
    let contract: &dyn DnsRecordProvider = &provider;

    assert_eq!(contract.provider_kind(), "none");
    assert_eq!(provider.provider_kind(), "none");

    let zone = contract.discover_zone(DOMAIN).await.unwrap();
    assert_eq!(zone.name, "example.com");

    let txt = contract
        .upsert_verification_txt(DOMAIN, "marula-verify-token")
        .await
        .unwrap();
    assert!(!txt.proxied, "the manual posture is never proxied");
    assert_eq!(txt.kind, DnsRecordKind::Txt);

    assert!(!contract
        .remove_verification_txt(DOMAIN)
        .await
        .expect("a manual record is the operator's to remove"));

    let proxied = contract
        .upsert_edge_record(&EdgeRecordSpec::cname(DOMAIN, "edge.marula.example", true))
        .await
        .unwrap();
    assert!(proxied.proxied);
    assert_eq!(proxied.ttl, 1);

    let dns_only = contract
        .upsert_edge_record(&EdgeRecordSpec::a(DOMAIN, "203.0.113.10", false))
        .await
        .unwrap();
    assert!(!dns_only.proxied);
    assert_eq!(dns_only.ttl, 300);
}

#[tokio::test]
async fn none_mode_instructions_overwrite_an_existing_prefix() {
    let named = format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}");
    let instructions = NoneDnsRecordProvider::manual_instructions(&named, "t", None).unwrap();

    assert_eq!(
        instructions.verification_txt.name,
        format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}")
    );
    assert_eq!(instructions.domain, DOMAIN);
    assert!(instructions.edge_record.is_none());
}

#[tokio::test]
async fn none_mode_rejects_an_empty_token_and_an_empty_target() {
    let empty_token = NoneDnsRecordProvider::manual_instructions(DOMAIN, "  ", None).unwrap_err();
    assert!(matches!(empty_token, DnsProviderError::InvalidRecord(_)));

    let empty_target = NoneDnsRecordProvider::manual_instructions(
        DOMAIN,
        "t",
        Some(&EdgeRecordSpec::cname(DOMAIN, "  ", false)),
    )
    .unwrap_err();
    assert!(matches!(empty_target, DnsProviderError::InvalidRecord(_)));
}

// ------------------------------------------------------- contract unit coverage

#[test]
fn candidate_zone_names_walk_stops_at_the_two_label_apex() {
    assert_eq!(
        candidate_zone_names("support.msp.example.com").unwrap(),
        vec!["support.msp.example.com", "msp.example.com", "example.com"]
    );
    assert_eq!(
        candidate_zone_names("example.com").unwrap(),
        vec!["example.com"]
    );
}

#[test]
fn normalize_domain_rejects_malformed_hostnames() {
    assert_eq!(
        normalize_domain("  Support.MSP.Example.COM. ").unwrap(),
        DOMAIN
    );
    assert_eq!(
        normalize_domain(&format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}")).unwrap(),
        DOMAIN,
        "the caller may pass the verification record name itself"
    );
    for bad in [
        "",
        "localhost",
        "-bad.example.com",
        "bad-.example.com",
        "exa mple.com",
        "example..com",
    ] {
        assert!(
            matches!(
                normalize_domain(bad),
                Err(DnsProviderError::InvalidDomain(_))
            ),
            "`{bad}` must be refused"
        );
    }
}

#[test]
fn verification_record_name_is_stable() {
    assert_eq!(
        verification_record_name(DOMAIN).unwrap(),
        format!("{VERIFICATION_TXT_PREFIX}.{DOMAIN}")
    );
}

#[test]
fn cloudflare_origin_ca_certificates_match_exact_and_wildcard_hostnames() {
    let wildcard = OriginCaCertificate {
        id: "c1".into(),
        hostnames: vec!["*.example.com".into(), "msp.example.com".into()],
        expires_on: None,
        request_type: None,
        revoked_at: None,
    };
    // The wildcard parent covers any hostname below it, and the exact entry matches.
    assert!(wildcard.covers_hostname(DOMAIN));
    assert!(wildcard.covers_hostname("msp.example.com"));
    assert!(wildcard.covers_hostname("other.sub.example.com"));
    assert!(
        !wildcard.covers_hostname("example.com"),
        "the parent is not itself covered"
    );
    assert!(!wildcard.covers_hostname("other.test"));

    let revoked = OriginCaCertificate {
        revoked_at: Some("2029-01-01T00:00:00Z".into()),
        ..wildcard
    };
    assert!(
        !revoked.covers_hostname(DOMAIN),
        "revoked certs never cover"
    );
    assert!(revoked.is_revoked());
}

#[test]
fn ssl_mode_parsing_covers_every_cloudflare_value() {
    assert_eq!(SslMode::parse("strict"), SslMode::Strict);
    assert_eq!(SslMode::parse("full_strict"), SslMode::Strict);
    assert_eq!(SslMode::parse("FULL"), SslMode::Full);
    assert_eq!(SslMode::parse(" flexible "), SslMode::Flexible);
    assert_eq!(SslMode::parse("off"), SslMode::Off);
    assert_eq!(SslMode::parse("weird"), SslMode::Unknown("weird".into()));
    assert!(SslMode::Strict.meets_v2_edge_requirement());
    assert!(!SslMode::Full.meets_v2_edge_requirement());
    assert_eq!(SslMode::Strict.as_str(), "strict");
}

#[test]
fn cloudflare_config_debug_redacts_the_api_token() {
    let config = CloudflareConfig::new("super-secret-token");
    let debug = format!("{config:?}");
    assert!(!debug.contains("super-secret-token"), "{debug}");
    assert!(debug.contains("api_token"), "{debug}");
}
