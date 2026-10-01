//! Pure seam tests: heuristic math, registry transitions, and both HTTP
//! adapters against local fixture servers (no live vendor, no keys).

use crate::contract::{ContractConfig, ContractRoutingProvider};
use crate::error::RoutingError;
use crate::heuristic::{HeuristicConfig, HeuristicRoutingProvider};
use crate::osrm::{OsrmConfig, OsrmDialect, OsrmRoutingProvider};
use crate::provider::{estimate_travel, RoutingProvider};
use crate::registry::{RoutingRegistry, RoutingStatus};
use crate::types::{
    GeoPoint, MatrixRequest, Place, RoutingCapability, RoutingQuality, TimeControl, TravelMode,
};
use axum::{
    extract::Path, extract::Query, http::HeaderMap, routing::get, routing::post, Json, Router,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fixture server");
    });
    format!("http://{addr}")
}

fn driving_matrix_request() -> MatrixRequest {
    MatrixRequest::new(
        vec![Place::at(GeoPoint::new(52.5, 13.4))],
        vec![Place::at(GeoPoint::new(52.6, 13.5))],
        TravelMode::Driving,
    )
}

#[test]
fn haversine_is_sane() {
    let a = GeoPoint::new(52.5, 13.4);
    let b = GeoPoint::new(53.5, 13.4);
    let meters = a.haversine_meters(&b);
    assert!(
        (meters - 111_195.0).abs() < 500.0,
        "one degree of latitude should be ~111 km, got {meters}"
    );
    assert_eq!(a.haversine_meters(&a), 0.0);
}

#[tokio::test]
async fn heuristic_labels_estimates_and_refuses_beyond_radius() {
    let provider = HeuristicRoutingProvider::default();
    assert_eq!(provider.info().quality, RoutingQuality::Heuristic);
    assert_eq!(provider.info().id, "none");
    let response = provider
        .matrix(&driving_matrix_request())
        .await
        .expect("matrix ok");
    assert_eq!(response.quality, RoutingQuality::Heuristic);
    let cell = response.cell(0, 0).expect("cell");
    assert!(cell.duration_seconds.unwrap() > 0);
    assert!(cell.distance_meters.unwrap() > 0);

    // A place without coordinates cannot be estimated — an honest hole.
    let unresolved = MatrixRequest::new(
        vec![Place::address("Unter den Linden 1, Berlin")],
        vec![Place::at(GeoPoint::new(52.6, 13.5))],
        TravelMode::Driving,
    );
    let response = provider.matrix(&unresolved).await.expect("matrix ok");
    assert_eq!(response.cell(0, 0).unwrap().duration_seconds, None);

    // Beyond the configured radius the cell is unreachable, not guessed.
    let far = MatrixRequest::new(
        vec![Place::at(GeoPoint::new(52.5, 13.4))],
        vec![Place::at(GeoPoint::new(40.7, -74.0))],
        TravelMode::Driving,
    );
    let response = provider.matrix(&far).await.expect("matrix ok");
    assert_eq!(response.cell(0, 0).unwrap().duration_seconds, None);
}

#[test]
fn heuristic_settings_fold_integer_units() {
    let config = HeuristicConfig::from_settings(2, 10, 130);
    assert_eq!(config.minutes_per_mile, 2.0);
    assert_eq!(config.overhead_minutes, 10.0);
    assert!((config.road_factor - 1.3).abs() < f64::EPSILON);
    // Percent below the floor clamps to the honest minimum (1.0).
    assert_eq!(HeuristicConfig::from_settings(2, 10, 90).road_factor, 1.0);
}

#[tokio::test]
async fn estimate_travel_marks_uncontrolled_time_honestly() {
    let provider = HeuristicRoutingProvider::default();
    let estimate = estimate_travel(
        &provider,
        &Place::at(GeoPoint::new(52.5, 13.4)),
        &Place::at(GeoPoint::new(52.6, 13.5)),
        TravelMode::Driving,
        Some(chrono::Utc::now()),
    )
    .await
    .expect("estimate");
    assert!(!estimate.time_controlled);
    assert_eq!(estimate.provider, "none");
    assert!(estimate.minutes > 0);
}

#[test]
fn registry_transitions_are_typed() {
    let registry = RoutingRegistry::new();
    assert_eq!(registry.status(), RoutingStatus::NoneMode);
    assert!(registry.active().is_none());
    let view = registry.status_view();
    assert_eq!(view.provider_id, "none");
    assert_eq!(view.reason.as_deref(), Some("no_provider_configured"));
    assert!(!view.suggestions_available);

    registry.set_configured(Some("osrm".into()));
    assert_eq!(
        registry.status(),
        RoutingStatus::Missing {
            configured: "osrm".into()
        }
    );
    assert_eq!(
        registry.status_view().reason.as_deref(),
        Some("provider_not_registered")
    );

    let heuristic: Arc<dyn RoutingProvider> = Arc::new(HeuristicRoutingProvider::default());
    registry.register(heuristic);
    registry.set_configured(Some("none".into()));
    assert_eq!(
        registry.status(),
        RoutingStatus::Configured(HeuristicRoutingProvider::default().info().clone())
    );
    assert!(registry.active().is_some());
    // Heuristic quality keeps suggestion-grade features silent.
    assert!(!registry.status_view().suggestions_available);
    assert!(!registry.status().measured());
}

async fn osrm_table(
    Path((profile, coords)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    assert_eq!(profile, "driving");
    assert_eq!(coords, "13.4,52.5;13.5,52.6");
    assert_eq!(
        query.get("annotations").map(String::as_str),
        Some("duration,distance")
    );
    assert_eq!(query.get("sources").map(String::as_str), Some("0"));
    assert_eq!(query.get("destinations").map(String::as_str), Some("1"));
    Json(serde_json::json!({
        "code": "Ok",
        "durations": [[600.0]],
        "distances": [[5000.0]],
    }))
}

async fn osrm_route(Path((profile, coords)): Path<(String, String)>) -> Json<serde_json::Value> {
    assert_eq!(profile, "driving");
    assert_eq!(coords, "13.4,52.5;13.5,52.6");
    Json(serde_json::json!({
        "code": "Ok",
        "routes": [{
            "legs": [{ "duration": 600.0, "distance": 5000.0 }],
            "duration": 600.0,
            "distance": 5000.0
        }]
    }))
}

async fn osrm_rate_limited() -> (
    axum::http::StatusCode,
    [(&'static str, &'static str); 1],
    &'static str,
) {
    (
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", "7")],
        "slow down",
    )
}

#[tokio::test]
async fn osrm_adapter_maps_matrix_route_and_rate_limit() {
    let app = Router::new()
        .route("/table/v1/{profile}/{coords}", get(osrm_table))
        .route("/route/v1/{profile}/{coords}", get(osrm_route));
    let base = spawn(app).await;
    let provider = OsrmRoutingProvider::new(OsrmConfig::new(
        base,
        OsrmDialect::Osrm,
        TravelMode::Driving,
    ))
    .expect("provider");
    assert_eq!(provider.info().quality, RoutingQuality::Measured);
    assert_eq!(provider.info().time_control, TimeControl::None);

    let matrix = provider
        .matrix(&driving_matrix_request())
        .await
        .expect("matrix");
    let cell = matrix.cell(0, 0).expect("cell");
    assert_eq!(cell.duration_seconds, Some(600));
    assert_eq!(cell.distance_meters, Some(5000));

    let route = provider
        .route(&crate::types::RouteRequest::new(
            Place::at(GeoPoint::new(52.5, 13.4)),
            Place::at(GeoPoint::new(52.6, 13.5)),
            TravelMode::Driving,
        ))
        .await
        .expect("route");
    assert_eq!(route.total_duration_seconds, Some(600));
    assert_eq!(route.legs.len(), 1);

    let limited = Router::new().route("/table/v1/{profile}/{coords}", get(osrm_rate_limited));
    let limited_base = spawn(limited).await;
    let limited_provider = OsrmRoutingProvider::new(OsrmConfig::new(
        limited_base,
        OsrmDialect::Osrm,
        TravelMode::Driving,
    ))
    .expect("provider");
    match limited_provider.matrix(&driving_matrix_request()).await {
        Err(RoutingError::RateLimited { retry_after_secs }) => {
            assert_eq!(retry_after_secs, Some(7));
        }
        other => panic!("expected typed rate limit, got {other:?}"),
    }
}

/// One captured request: (Authorization header, JSON body).
type CapturedRequest = (Option<String>, serde_json::Value);

#[derive(Clone)]
struct Capture {
    seen: Arc<Mutex<Vec<CapturedRequest>>>,
}

async fn contract_matrix(
    axum::extract::State(capture): axum::extract::State<Capture>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    capture
        .seen
        .lock()
        .expect("capture lock")
        .push((auth, body));
    Json(serde_json::json!({
        "cells": [{ "source_index": 0, "target_index": 0, "duration_seconds": 900.0, "distance_meters": 12000.0 }]
    }))
}

#[tokio::test]
async fn contract_adapter_sends_bearer_and_maps_cells() {
    let capture = Capture {
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let app = Router::new()
        .route("/routing/matrix", post(contract_matrix))
        .with_state(capture.clone());
    let base = spawn(app).await;
    let mut config = ContractConfig::new("contract:traveltime", "TravelTime (relay pack)", base);
    config.token = Some("relay-secret".into());
    config.capabilities = vec![RoutingCapability::Matrix];
    let provider = ContractRoutingProvider::new(config).expect("provider");

    let response = provider
        .matrix(&driving_matrix_request())
        .await
        .expect("matrix");
    assert_eq!(response.provider, "contract:traveltime");
    assert_eq!(response.quality, RoutingQuality::Measured);
    assert_eq!(response.cell(0, 0).unwrap().duration_seconds, Some(900));

    {
        let seen = capture.seen.lock().expect("capture lock");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0.as_deref(), Some("Bearer relay-secret"));
        assert_eq!(seen[0].1["mode"], "driving");
        assert_eq!(seen[0].1["sources"][0]["point"]["latitude"], 52.5);
    }

    // An undeclared capability never leaves the process.
    match provider
        .geocode(&crate::types::GeocodeRequest::new("Berlin"))
        .await
    {
        Err(RoutingError::Unsupported { capability, .. }) => assert_eq!(capability, "geocode"),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn contract_upstream_error_is_typed_and_credential_free() {
    async fn failing() -> (axum::http::StatusCode, &'static str) {
        (axum::http::StatusCode::BAD_GATEWAY, "vendor exploded")
    }
    let app = Router::new().route("/routing/matrix", post(failing));
    let base = spawn(app).await;
    let mut config = ContractConfig::new("contract:x", "X", base);
    config.token = Some("super-secret".into());
    let provider = ContractRoutingProvider::new(config).expect("provider");
    match provider.matrix(&driving_matrix_request()).await {
        Err(RoutingError::Upstream { status, excerpt }) => {
            assert_eq!(status, 502);
            assert!(!excerpt.contains("super-secret"));
        }
        other => panic!("expected upstream error, got {other:?}"),
    }
}
