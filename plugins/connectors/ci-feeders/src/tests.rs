//! In-crate conformance: the feeder mapping + wire mechanics, tested
//! against the NinjaOne fixture and a stub API server. The FULL CI gate
//! over the real core lives in `crates/core-api/tests/ci_feeder_gate.rs`;
//! this file keeps the connector's own surface honest without ever
//! importing a Meridian crate. Marketplace feeder mappers (Auvik, IT
//! Glue, community adapters) carry their own conformance suites in their
//! repos, over the same public [`vendors`] surface.

use std::sync::{Arc, Mutex};

use axum::Json;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::identity::{fingerprint, IdentityKeysBuilder};
use crate::model::FeederRecord;
use crate::sink::{
    FeederSink, MarulaFeederConfig, MarulaFeederSink, NoneFeederSink, SinkError,
};
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
async fn meridian_sink_posts_feed_upsert_with_api_key_and_deterministic_key() {
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
        api_key: "mrd_feeder_key".into(),
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
        Some("mrd_feeder_key")
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
        api_key: "mrd_scoped_wrong".into(),
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
    let failing = vendors::map_ninjaone_device(&json!({"id": 9001, "displayName": "Fails"})).unwrap();
    let passing = vendors::map_ninjaone_device(&json!({"id": 9002, "displayName": "Passes"})).unwrap();
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
