//! In-crate conformance: the feeder mapping + wire mechanics, tested
//! against the NinjaOne fixture and a stub API server. The FULL CI gate
//! over the real core lives in `crates/core-api/tests/ci_feeder_gate.rs`;
//! this file keeps the connector's own surface honest without ever
//! importing a Marula crate. Marketplace feeder mappers (Auvik, IT
//! Glue, community adapters) carry their own conformance suites in their
//! repos, over the same public [`vendors`] surface.

use std::sync::{Arc, Mutex};

use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::identity::{fingerprint, IdentityKeysBuilder};
use crate::model::FeederRecord;
use crate::sink::{FeederSink, MarulaFeederConfig, MarulaFeederSink, NoneFeederSink, SinkError};
use crate::sync::{ApplyMode, FeederSync, SyncConfig};
use crate::vendors;

fn ninjaone_fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/ninjaone_device.json")).unwrap()
}

// --- vendor mapping ----------------------------------------------------------

#[test]
fn ninjaone_device_maps_identity_and_observed_fields() {
    let record = vendors::map_ninjaone_device(&ninjaone_fixture()).expect("maps");
    assert_eq!(record.source_system, "ninjaone");
    assert_eq!(record.external_id, "4242");
    assert_eq!(
        record.source_guid.as_deref(),
        Some("e7f1c2a4-9b3d-4c8e-a1f2-33b4c5d6e7f8")
    );
    assert_eq!(record.tag, "ACME-MAIL-01");
    let keys = record.identity_keys.as_object().unwrap();
    assert_eq!(keys["serial"], json!("SN8F3K2Q"));
    assert_eq!(keys["macs"], json!(["aa:bb:cc:dd:ee:ff"]));
    assert_eq!(keys["hostname"], json!("acme-mail-01"));
    assert_eq!(keys["domain"], json!("acme.local"));
    let observed = record.observed_fields.as_object().unwrap();
    assert_eq!(observed["externalIp"], json!("203.0.113.7"));
    assert_eq!(
        observed["osName"],
        json!("Microsoft Windows Server 2022 Standard")
    );
}

#[test]
fn malformed_payloads_are_rejected_not_guessed() {
    let err = vendors::map_ninjaone_device(&json!({"displayName": "no id"})).unwrap_err();
    assert!(matches!(err, vendors::NormalizeError::Unrecognised { .. }));
}

// --- identity determinism (the CI3 gate, connector side) ----------------------

#[test]
fn identity_keys_are_deterministic_under_payload_reformatting() {
    let base = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    // The same device with every formatting quirk flipped.
    let reformatted = vendors::map_ninjaone_device(&json!({
        "id": 4242,
        "uid": "e7f1c2a4-9b3d-4c8e-a1f2-33b4c5d6e7f8",
        "displayName": "acme-mail-01.",
        "systemName": "ACME-MAIL-01",
        "domain": "ACME.LOCAL.",
        "serialNumber": " sn8f3k2q ",
        "macAddresses": ["aabbccddeeff"],
        "lastSeen": "2026-09-13T01:00:00Z"
    }))
    .unwrap();
    assert_eq!(base.identity_keys, reformatted.identity_keys);
}

#[test]
fn fingerprint_changes_only_when_content_changes() {
    let base = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    let mut moved = base.clone();
    moved.observed_fields = json!({"externalIp": "203.0.113.8"});
    assert_ne!(fingerprint(&base), fingerprint(&moved));

    // Key-order-insensitive: rebuilt identity keys hash the same.
    let rebuilt = FeederRecord {
        identity_keys: IdentityKeysBuilder::new()
            .source_guid("e7f1c2a4-9b3d-4c8e-a1f2-33b4c5d6e7f8")
            .serial("sn-8f3k2-q")
            .macs(["AABBCCDDEEFF"])
            .hostname("acme-mail-01")
            .domain("acme.local")
            .build(),
        ..base.clone()
    };
    assert_eq!(base.identity_keys, rebuilt.identity_keys);
    assert_eq!(fingerprint(&base), fingerprint(&rebuilt));
}

// --- wire mechanics (stub server) ---------------------------------------------

#[tokio::test]
async fn marula_sink_posts_feed_upsert_with_api_key_and_deterministic_key() {
    #[derive(Default)]
    struct Captured {
        path: Option<String>,
        api_key: Option<String>,
        body: Option<Value>,
    }
    let captured = Arc::new(Mutex::new(Captured::default()));
    let capture_route = captured.clone();
    let app = axum::Router::new().route(
        "/api/v1/config-items/feed-upsert",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                let captured = capture_route.clone();
                async move {
                    let mut guard = captured.lock().unwrap();
                    guard.path = Some("/api/v1/config-items/feed-upsert".into());
                    guard.api_key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    guard.body = Some(body);
                    Json(json!({
                        "id": Uuid::now_v7(),
                        "source_link": {
                            "id": Uuid::now_v7(),
                            "source_system": "ninjaone",
                            "external_id": "4242"
                        },
                        "created_item": true,
                        "resolved_existing": false,
                        "auto_merged": false,
                        "queued_candidates": [],
                        "contradictions": []
                    }))
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let record = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    let sink = MarulaFeederSink::new(MarulaFeederConfig {
        base_url: format!("http://{addr}"),
        api_key: "mrl_feeder_key".into(),
    });

    let first = sink.feed_upsert(&record, Uuid::now_v7()).await;
    let outcome = first.expect("first sync applied");
    assert!(outcome.created_item);
    assert_eq!(outcome.source_system, "ninjaone");
    assert_eq!(outcome.external_id, "4242");
    let key_one = captured.lock().unwrap().body.as_ref().and_then(|b| {
        b.get("idempotency_key")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    assert_eq!(
        key_one.as_deref(),
        Some(record.idempotency_key().as_str()),
        "the deterministic key rides the body"
    );
    assert_eq!(
        captured.lock().unwrap().api_key.as_deref(),
        Some("mrl_feeder_key")
    );

    // Same content ⇒ same deterministic key (a retried sync replays).
    sink.feed_upsert(&record, Uuid::now_v7())
        .await
        .expect("replay");
    let body = captured.lock().unwrap().body.clone().unwrap();
    assert_eq!(body["idempotency_key"], json!(record.idempotency_key()));
    assert_eq!(body["source_system"], json!("ninjaone"));
    assert_eq!(body["identity_keys"]["serial"], json!("SN8F3K2Q"));
}

#[tokio::test]
async fn sink_surfaces_typed_errors_for_auth_and_rejection() {
    let app = axum::Router::new().route(
        "/api/v1/config-items/feed-upsert",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::FORBIDDEN,
                Json(json!({"message": "denied"})),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let record = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    let sink = MarulaFeederSink::new(MarulaFeederConfig {
        base_url: format!("http://{addr}"),
        api_key: "mrl_scoped_wrong".into(),
    });
    let err = sink.feed_upsert(&record, Uuid::now_v7()).await.unwrap_err();
    assert!(matches!(err, SinkError::Unauthorized(_)));
}

// --- engine semantics -----------------------------------------------------------

#[tokio::test]
async fn none_mode_counts_everything_as_would_sync_without_io() {
    let record = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    let report =
        FeederSync::run(&SyncConfig::new(Uuid::now_v7()), &[record], &NoneFeederSink).await;
    assert_eq!(report.total, 1);
    assert_eq!(report.synced, 0);
    assert_eq!(report.would_sync, 1);
    assert!(report.is_clean());
}

#[tokio::test]
async fn dry_run_never_touches_the_sink() {
    struct ExplodingSink;
    #[async_trait::async_trait]
    impl FeederSink for ExplodingSink {
        async fn feed_upsert(
            &self,
            _: &FeederRecord,
            _: Uuid,
        ) -> Result<crate::sink::FeedUpsertOutcome, SinkError> {
            panic!("dry run must not touch the sink");
        }
    }
    let record = vendors::map_ninjaone_device(&ninjaone_fixture()).unwrap();
    let report = FeederSync::run(
        &SyncConfig {
            client_id: Uuid::now_v7(),
            apply: ApplyMode::DryRun,
        },
        &[record],
        &ExplodingSink,
    )
    .await;
    assert_eq!(report.would_sync, 1);
    assert_eq!(report.synced, 0);
}

#[tokio::test]
async fn per_record_failure_never_aborts_the_batch() {
    let failing =
        vendors::map_ninjaone_device(&json!({"id": 9001, "displayName": "Fails"})).unwrap();
    let passing =
        vendors::map_ninjaone_device(&json!({"id": 9002, "displayName": "Passes"})).unwrap();
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen_route = seen.clone();
    struct RouteSink(Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait::async_trait]
    impl FeederSink for RouteSink {
        async fn feed_upsert(
            &self,
            record: &FeederRecord,
            _: Uuid,
        ) -> Result<crate::sink::FeedUpsertOutcome, SinkError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if record.external_id == "9001" {
                Err(SinkError::Rejected("no".into()))
            } else {
                Ok(crate::sink::FeedUpsertOutcome {
                    config_item_id: Uuid::now_v7(),
                    source_link_id: Uuid::now_v7(),
                    source_system: record.source_system.clone(),
                    external_id: record.external_id.clone(),
                    created_item: true,
                    resolved_existing: false,
                    auto_merged: false,
                    queued_candidates: 0,
                    contradictions: vec![],
                })
            }
        }
    }
    let report = FeederSync::run(
        &SyncConfig::new(Uuid::now_v7()),
        &[failing, passing],
        &RouteSink(seen_route),
    )
    .await;
    assert_eq!(report.total, 2);
    assert_eq!(report.synced, 1);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].external_id, "9001");
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
}

// --- NinjaOne alerts (G4) ----------------------------------------------------

fn ninjaone_alert_fixture() -> Value {
    serde_json::from_str(include_str!("../tests/fixtures/ninjaone_alert.json")).unwrap()
}

#[test]
fn ninjaone_alert_maps_to_one_ingest_document() {
    let mapped = vendors::map_ninjaone_alert(&ninjaone_alert_fixture()).expect("maps");
    let vendors::NinjaOneEvent::Alert(alert) = mapped else {
        panic!("an active condition is an alert");
    };

    // Stable identity: the vendor's uid, unchanged across deliveries.
    assert_eq!(alert.dedup_key, "5f0c2a9e-6a7b-4c1d-9e2f-8a1b2c3d4e5f");
    // Storm identity: device + condition, not the alert id.
    assert_eq!(alert.correlation_key, "4242:DISK_SPACE");
    assert_eq!(alert.severity.as_str(), "critical");
    assert_eq!(alert.title, "Drive C: is 96% full (ACME-MAIL-01)");
    assert_eq!(alert.occurred_at.as_deref(), Some("2026-11-14T09:12:00Z"));
    assert_eq!(alert.attributes["condition"], json!("DISK_SPACE"));
    assert_eq!(alert.attributes["data"]["percentFree"], json!(4));

    // The body is the published ingest request, correlation_key included.
    let body = alert.ingest_body();
    assert_eq!(body["dedup_key"], json!(alert.dedup_key));
    assert_eq!(body["correlation_key"], json!("4242:DISK_SPACE"));
    assert_eq!(body["severity"], json!("critical"));
    assert_eq!(body["title"], json!("Drive C: is 96% full (ACME-MAIL-01)"));
}

#[test]
fn ninjaone_severity_is_total_and_never_guesses_critical() {
    use vendors::AlertSeverity;
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("CRITICAL")),
        AlertSeverity::Critical
    );
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("MAJOR")),
        AlertSeverity::Warning
    );
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("WARNING")),
        AlertSeverity::Warning
    );
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("MINOR")),
        AlertSeverity::Info
    );
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("NONE")),
        AlertSeverity::Info
    );
    // Unknown and absent default to warning: visible, and P2 rather than P1.
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(Some("BANANA")),
        AlertSeverity::Warning
    );
    assert_eq!(
        vendors::ninjaone_alerts::severity_of(None),
        AlertSeverity::Warning
    );
}

#[test]
fn ninjaone_storm_alerts_share_a_key_and_keep_their_own_dedupe_keys() {
    let first = vendors::map_ninjaone_alert(&ninjaone_alert_fixture()).unwrap();
    // The same condition on the same device, reported again with a new uid.
    let mut second_payload = ninjaone_alert_fixture();
    second_payload["uid"] = json!("9b8c7d6e-5f4a-3b2c-1d0e-9f8a7b6c5d4e");
    second_payload["createTime"] = json!("2026-11-14T09:20:00Z");
    let second = vendors::map_ninjaone_alert(&second_payload).unwrap();

    let (vendors::NinjaOneEvent::Alert(a), vendors::NinjaOneEvent::Alert(b)) = (first, second)
    else {
        panic!("both are alerts");
    };
    assert_eq!(a.correlation_key, b.correlation_key, "one storm");
    assert_ne!(
        a.dedup_key, b.dedup_key,
        "two distinct alerts, so the count can rise"
    );
}

#[test]
fn a_repeated_delivery_is_one_event_and_the_count_moves() {
    use vendors::StormDecision;
    let mut coalescer = vendors::StormCoalescer::new(600);
    let payload = ninjaone_alert_fixture();

    // One payload from a batch of three: a repeat and a different alert.
    let mut repeated = payload.clone();
    repeated["status"] = json!("STARTED");
    let mut other_device = payload.clone();
    other_device["deviceId"] = json!(9001);
    other_device["uid"] = json!("11111111-2222-3333-4444-555555555555");

    let (decisions, refusals) = vendors::map_ninjaone_alerts(
        &[payload.clone(), repeated, other_device],
        &mut coalescer,
        1_700_000_000,
    );
    assert!(refusals.is_empty());
    assert_eq!(decisions.len(), 3);
    assert!(matches!(&decisions[0], StormDecision::Send(a) if a.dedup_key.starts_with("5f0c")));
    assert!(matches!(
        &decisions[1],
        StormDecision::Repeat { repeats: 2, .. }
    ));
    assert!(matches!(&decisions[2], StormDecision::Send(_)));

    // The same uid after the window is a new delivery again.
    let (later, _) = vendors::map_ninjaone_alerts(&[payload], &mut coalescer, 1_700_000_000 + 601);
    assert!(matches!(&later[0], StormDecision::Send(_)));
}

#[test]
fn ninjaone_resolved_conditions_do_not_open_a_ticket() {
    let mut resolved = ninjaone_alert_fixture();
    resolved["status"] = json!("RESOLVED");
    let event = vendors::map_ninjaone_alert(&resolved).unwrap();
    let vendors::NinjaOneEvent::Resolved {
        dedup_key,
        correlation_key,
    } = event
    else {
        panic!("a resolved condition is not an alert");
    };
    assert_eq!(dedup_key, "5f0c2a9e-6a7b-4c1d-9e2f-8a1b2c3d4e5f");
    assert_eq!(correlation_key, "4242:DISK_SPACE");

    // A payload with no identity at all is refused, never fabricated.
    assert!(vendors::map_ninjaone_alert(&json!({"message": "mystery"})).is_err());
}

#[test]
fn ninjaone_alert_without_uid_keys_on_device_and_condition() {
    let mut payload = ninjaone_alert_fixture();
    payload.as_object_mut().unwrap().remove("uid");
    let vendors::NinjaOneEvent::Alert(alert) = vendors::map_ninjaone_alert(&payload).unwrap()
    else {
        panic!("alert");
    };
    // Stable: derived from the condition's start time, never receipt time.
    assert_eq!(alert.dedup_key, "4242:DISK_SPACE:2026-11-14T09:12:00Z");
    assert_eq!(alert.correlation_key, "4242:DISK_SPACE");
}
